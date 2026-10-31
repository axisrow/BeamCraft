// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D01 closed, canonical USDA intent. Writing requires admitted engine intent; reading goes
//! through the bounded layer engine and the same engine admission, never through a raw setter.

use crate::plate::{PlateError, integrity};
use beamcraft_engine::drawing::{
    self, CutSide, DetailSpec, DimensionSpec, DraftingGeometry, DraftingSpec, Drawing, DrawingSpec,
    LayerSpec, LineType, MAX_DETAILS, MAX_DRAFTING_ENTITIES, MAX_INTENT_PAGE_ITEMS, MAX_LAYERS,
    MAX_SECTIONS, MAX_VIEWS, Plane, SectionSpec, SheetFrame, SheetSpec, SourceModelPin, TitleBlock,
    ViewSpec,
};

use beamcraft_engine::query::feature::FeatureId;
use beamcraft_layer_engine::{
    Asset, BACKEND, Budgets, Control, Identity, OpenRequest, Prim, Profile, Reader, Value,
};
use beamcraft_model::digest::{ContentDigest, Sha256};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{
    DimensionId, DraftingEntityId, DrawingId, DrawingViewId, ElementId, ProjectId, RevisionId,
};
use beamcraft_model::sheet::Sheet;
use std::collections::{BTreeMap, BTreeSet};

/// Sole authored drawing asset, distinct from any engineering-model root.
pub const INTENT_LAYER: &str = "drawing.usda";
/// Cap checked before copying or parsing a document intent. It must hold the densest intent
/// admission accepts: fifty-eight sixteen-point hatches whose coordinates all take the longest
/// bounded canonical form fill roughly seventy-six kilobytes, so the cap sits above any
/// page-maximal document instead of refusing admitted state at encode time.
pub const MAX_INTENT_BYTES: usize = 131_072;
/// Closed descriptor whose SHA-256 is the drawing schema identity.
pub const SCHEMA_DESCRIPTOR: &str = "beamcraft-drawing-usd/4;intent-bytes=131072;number-text-bytes=24;root=/Drawing;view=/Drawing/View;views=/Drawing/Views;views-cap=16;sections=/Drawing/Sections;sections-cap=16;details=/Drawing/Details;details-cap=16;page-items=64;dimension=/Drawing/Dimension;sheet=/Drawing/Sheet;sheet-sizes=a4,a3,a2,a1,a0;drafting=/Drawing/Drafting;drafting-entities=line,polyline,arc,circle,rectangle,text,hatch,symbol,free_dimension;drafting-cap=60;drafting-attrs=layer,linetype;linetypes=continuous,dashed,dotted,dashdot;layers=/Drawing/Layers;layers-cap=16;intent=beamcraft-drawing/1;source=project,revision,manifest,schema,generation;anchors=qualified-member-vertices;units=mm;projection=front,top,right;canonical-usda";

/// Independent drawing schema digest; never interchangeable with the engineering-model schema.
pub fn schema_identity() -> Result<ContentDigest, PlateError> {
    Sha256::digest(SCHEMA_DESCRIPTOR.as_bytes())
        .map_err(|_| integrity("drawing schema digest failed"))
}

fn limit() -> PlateError {
    PlateError {
        rule_id: None,
        code: ErrorCode::LimitExceeded,
        message: "drawing intent byte cap exceeded",
    }
}

fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// Encodes bounded immutable intent as ordinary USD custom attributes. It authors no engineering
/// prim, geometry or approved mark and grants no source verification/publication authority.
pub fn encode(drawing: &Drawing) -> Result<Asset, PlateError> {
    let s = drawing.spec();
    let source = s.source;
    let [a, b] = &s.dimension.anchors;
    let [x, y] = s.view.paper_origin_mm;
    let mut text = String::with_capacity(4096);
    text.push_str("#usda 1.0\n(\n    defaultPrim = \"Drawing\"\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n)\ndef Xform \"Drawing\"\n{\n");
    for (name, value) in [
        ("contract", drawing::CONTRACT_VERSION.to_owned()),
        ("schema", schema_identity()?.to_string()),
        ("id", s.id.to_string()),
        ("name", s.name.clone()),
        ("member", s.member.to_string()),
        ("sourceProject", source.project.to_string()),
        ("sourceRevision", source.revision.to_string()),
        ("sourceManifest", source.manifest.to_string()),
        ("sourceSchema", source.schema.to_string()),
    ] {
        text.push_str(&format!(
            "    custom string drawing:{name} = {}\n",
            quote(&value)
        ));
    }
    text.push_str(&format!("    custom uint64 drawing:sourceGeneration = {}\n    def Scope \"View\"\n    {{\n        custom string drawing:id = {}\n        custom token drawing:plane = {}\n        custom double drawing:scale = {}\n        custom double[] drawing:origin = [{x}, {y}]\n    }}\n", source.generation, quote(&s.view.id.to_string()), quote(s.view.plane.as_str()), s.view.scale_denominator));
    text.push_str(&format!("    def Scope \"Dimension\"\n    {{\n        custom string drawing:id = {}\n        custom string[] drawing:anchors = [{}, {}]\n        custom double drawing:offset = {}\n        custom uint drawing:decimals = {}\n    }}\n", quote(&s.dimension.id.to_string()), quote(&a.to_string()), quote(&b.to_string()), s.dimension.paper_offset_mm, s.dimension.decimal_places));
    if let Some(sheet) = &s.sheet {
        text.push_str(&format!("    def Scope \"Sheet\"\n    {{\n        custom token drawing:size = {}\n        custom double drawing:frameInset = {}\n        custom string drawing:titleProject = {}\n        custom string drawing:titleAuthor = {}\n    }}\n", quote(sheet.size.as_str()), sheet.frame.inset_mm, quote(&sheet.title.project), quote(&sheet.title.author)));
    }
    if !s.views.is_empty() {
        text.push_str("    def Scope \"Views\"\n    {\n");
        for (ordinal, view) in s.views.iter().enumerate() {
            // Canonical ordinal names, not view UUIDs: the prim name is derived state, the
            // identity lives in the drawing:id attribute.
            let [x, y] = view.paper_origin_mm;
            text.push_str(&format!("        def Scope \"V{}\"\n        {{\n            custom string drawing:id = {}\n            custom token drawing:plane = {}\n            custom double drawing:scale = {}\n            custom double[] drawing:origin = [{x}, {y}]\n        }}\n", ordinal + 1, quote(&view.id.to_string()), quote(view.plane.as_str()), view.scale_denominator));
        }
        text.push_str("    }\n");
    }
    if !s.sections.is_empty() {
        text.push_str("    def Scope \"Sections\"\n    {\n");
        for (ordinal, section) in s.sections.iter().enumerate() {
            // Canonical ordinal names, not view UUIDs: the prim name is derived state, the
            // identity lives in the drawing:id attribute.
            let [x, y] = section.paper_origin_mm;
            text.push_str(&format!("        def Scope \"S{}\"\n        {{\n            custom string drawing:id = {}\n            custom token drawing:plane = {}\n            custom double drawing:cut = {}\n            custom token drawing:side = {}\n            custom double drawing:scale = {}\n            custom double[] drawing:origin = [{x}, {y}]\n        }}\n", ordinal + 1, quote(&section.id.to_string()), quote(section.plane.as_str()), section.cut_mm, quote(section.side.as_str()), section.scale_denominator));
        }
        text.push_str("    }\n");
    }
    if !s.details.is_empty() {
        text.push_str("    def Scope \"Details\"\n    {\n");
        for (ordinal, detail) in s.details.iter().enumerate() {
            let [cx, cy] = detail.center_paper_mm;
            let [x, y] = detail.paper_origin_mm;
            text.push_str(&format!("        def Scope \"D{}\"\n        {{\n            custom string drawing:id = {}\n            custom string drawing:parent = {}\n            custom double[] drawing:center = [{cx}, {cy}]\n            custom double drawing:radius = {}\n            custom double drawing:scale = {}\n            custom double[] drawing:origin = [{x}, {y}]\n        }}\n", ordinal + 1, quote(&detail.id.to_string()), quote(&detail.parent.to_string()), detail.radius_mm, detail.scale_denominator));
        }
        text.push_str("    }\n");
    }
    if !s.layers.is_empty() {
        text.push_str("    def Scope \"Layers\"\n    {\n");
        for (ordinal, layer) in s.layers.iter().enumerate() {
            // Canonical ordinal names, not layer UUIDs: the prim name is derived state, the
            // identity lives in the drawing:id attribute.
            text.push_str(&format!(
                "        def Scope \"L{}\"\n        {{\n            custom string drawing:id = {}\n            custom string drawing:name = {}\n            custom bool drawing:visible = {}\n        }}\n",
                ordinal + 1,
                quote(&layer.id.to_string()),
                quote(&layer.name),
                layer.visible
            ));
        }
        text.push_str("    }\n");
    }
    if !s.drafting.is_empty() {
        text.push_str("    def Scope \"Drafting\"\n    {\n");
        for (ordinal, entity) in s.drafting.iter().enumerate() {
            // Canonical ordinal names, not entity UUIDs: the prim name is derived state, the
            // identity lives in the drawing:id attribute.
            let numbers = entity
                .geometry
                .clone()
                .coordinates()
                .iter()
                .map(f64::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            let payload = match entity.geometry.string_payload() {
                Some((name, value)) => {
                    format!("            custom string {name} = {}\n", quote(value))
                }
                None => String::new(),
            };
            text.push_str(&format!(
                "        def Scope \"E{}\"\n        {{\n            custom string drawing:id = {}\n            custom token drawing:kind = {}\n            custom string drawing:layer = {}\n            custom token drawing:linetype = {}\n            custom double[] drawing:geometry = [{}]\n{}        }}\n",
                ordinal + 1,
                quote(&entity.id.to_string()),
                quote(entity.geometry.kind()),
                quote(&entity.layer.to_string()),
                quote(entity.line_type.as_str()),
                numbers,
                payload
            ));
        }
        text.push_str("    }\n");
    }
    text.push_str("}\n");
    if text.len() > MAX_INTENT_BYTES {
        return Err(limit());
    }
    let bytes = text.into_bytes();
    let digest = Sha256::digest(&bytes).map_err(|_| limit())?;
    Ok(Asset {
        name: INTENT_LAYER.to_owned(),
        digest,
        bytes,
    })
}

fn value<'a>(prim: &'a Prim, name: &str, type_name: &str) -> Result<&'a Value, PlateError> {
    let property = prim
        .properties
        .iter()
        .find(|p| p.name == name && p.type_name == type_name)
        .ok_or_else(|| integrity("drawing attribute is absent or has the wrong type"))?;
    property
        .value
        .as_ref()
        .ok_or_else(|| integrity("drawing attribute has no value"))
}

fn text<'a>(prim: &'a Prim, name: &str, type_name: &str) -> Result<&'a str, PlateError> {
    match value(prim, name, type_name)? {
        Value::Text(text) => Ok(text),
        _ => Err(integrity("drawing attribute is not text")),
    }
}

fn number(prim: &Prim, name: &str) -> Result<f64, PlateError> {
    match value(prim, name, "double")? {
        Value::Number(n) => Ok(*n),
        _ => Err(integrity("drawing attribute is not a double")),
    }
}

fn unsigned(prim: &Prim, name: &str, kind: &str) -> Result<u64, PlateError> {
    match value(prim, name, kind)? {
        Value::Uint(n) => Ok(*n),
        Value::Int(n) => u64::try_from(*n).map_err(|_| integrity("negative drawing integer")),
        _ => Err(integrity("drawing attribute is not an integer")),
    }
}

/// Reads one view's exactly-two-component paper origin.
fn paper_origin(prim: &Prim) -> Result<[f64; 2], PlateError> {
    match value(prim, "drawing:origin", "double[]")? {
        Value::Array(values) => match values.as_slice() {
            [Value::Number(x), Value::Number(y)] => Ok([*x, *y]),
            _ => Err(integrity("invalid drawing origin")),
        },
        _ => Err(integrity("invalid drawing origin")),
    }
}

/// Collects one scope's canonical ordinal children in order, refusing foreign prims, duplicate
/// or non-contiguous ordinals and counts over the cap.
fn ordinal_children<'a>(
    items: &'a [Prim],
    prefix: &str,
    cap: usize,
) -> Result<Vec<&'a Prim>, PlateError> {
    let mut children: BTreeMap<usize, &Prim> = BTreeMap::new();
    for item in items {
        let Some(name) = item.path.strip_prefix(prefix) else {
            continue;
        };
        if item.type_name.as_deref() != Some("Scope") || !item.active {
            return Err(integrity("drawing scope holds a foreign prim"));
        }
        let ordinal: usize = name
            .parse()
            .map_err(|_| integrity("drawing prim name is not a canonical ordinal"))?;
        if ordinal == 0 || children.insert(ordinal, item).is_some() {
            return Err(integrity("drawing ordinals must be distinct from one"));
        }
    }
    if children.len() > cap {
        return Err(limit());
    }
    let mut ordered = Vec::with_capacity(children.len());
    for (expected, (ordinal, item)) in children.iter().enumerate() {
        if *ordinal != expected + 1 {
            return Err(integrity("drawing ordinals must be contiguous"));
        }
        ordered.push(*item);
    }
    Ok(ordered)
}

/// Verifies an externally pinned intent asset, then parses and re-admits it. `identity` must come
/// from the authorized document manifest: project is the source project and workspace is DrawingId.
/// Revision/generation belong to the document; they are not substituted for its source pin.
/// This first schema admits exactly canonical bytes produced by [`encode`], no composition,
/// unknown fields or alternate topology. Decoding alone does not verify the archived source.
pub fn decode(asset: Asset, identity: Identity, control: &Control) -> Result<Drawing, PlateError> {
    if asset.bytes.len() > MAX_INTENT_BYTES {
        return Err(limit());
    }
    if asset.name != INTENT_LAYER {
        return Err(integrity("wrong drawing intent asset name"));
    }
    let mut request = OpenRequest {
        identity: identity.clone(),
        root: INTENT_LAYER.to_owned(),
        assets: vec![asset.clone()],
        owned_layers: BTreeSet::from([INTENT_LAYER.to_owned()]),
        manifest: asset.digest,
        backend: BACKEND,
        profile: Profile::StaticLocal,
        budgets: Budgets {
            layer_bytes: MAX_INTENT_BYTES,
            total_bytes: MAX_INTENT_BYTES,
            layers: 1,
            depth: 8,
            // Root, view, dimension, the optional sheet/views/sections/details/layers/drafting
            // scopes and every object — admission caps the whole intent page (engine
            // MAX_INTENT_PAGE_ITEMS), so the reader budget is that exact bound (plus the stage's
            // pseudo-root entry the page walk delivers); the semantic reader below still
            // re-checks everything.
            prims: MAX_INTENT_PAGE_ITEMS + 1,
            // The densest admitted page holds ~2,300 semantic values but far more parser
            // tokens: the text preflight charges one token per numeric character, and a
            // page of twenty-four-byte coordinates alone is tens of thousands of them.
            // The closed semantic reader below still requires exactly two anchors/two origins.
            values: 16_384,
            // The text preflight charges every array's leaf bytes against this bound, not its
            // element count. The largest array the contract can hold is one drawn entity's
            // `drawing:geometry`: thirty-two coordinates at the twenty-four-byte canonical
            // maximum, so 768 is exactly the closed ceiling.
            array_items: 768,
            // The densest page holds ~6,400 string/token bytes (every identity, kind, layer,
            // line type and pattern of fifty-eight entities plus the document's own strings).
            text_bytes: 16_384,
            decoded_bytes: 131_072,
            // The typed snapshot charges every prim's provenance paths and every attribute's
            // key plus value size; a page of sixty-four prims with thirty-two-coordinate
            // entities crosses a bare byte-parity budget, so this sits well above it.
            snapshot_bytes: 524_288,
            page_items: MAX_INTENT_PAGE_ITEMS,
        },
    };
    // Asset digest and identity come from the caller's manifest. This is the wrapper's distinct
    // commitment over that already authorized input, not a new assertion of byte ownership.
    request.manifest = request.manifest_digest()?;
    let mut reader = Reader::default();
    let opened = reader.open(request, control)?;
    if opened.meters_per_unit != 0.001 || opened.up_axis != "Z" {
        return Err(integrity("drawing stage profile disagrees"));
    }
    // Walk the frozen snapshot to the end: admission bounds the intent to MAX_INTENT_PAGE_ITEMS
    // semantic prims, and the stage delivers them in page-sized slices behind a cursor.
    let mut items: Vec<Prim> = Vec::new();
    let mut cursor = None;
    loop {
        let page = reader.read(&opened.context, cursor.as_ref(), control)?;
        items.extend(page.items);
        match page.next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    let prim = |path: &str, kind: &str| {
        items
            .iter()
            .find(|p| p.path == path && p.type_name.as_deref() == Some(kind) && p.active)
            .ok_or_else(|| integrity("drawing prim profile disagrees"))
    };
    let root = prim("/Drawing", "Xform")?;
    let view = prim("/Drawing/View", "Scope")?;
    let dim = prim("/Drawing/Dimension", "Scope")?;
    if text(root, "drawing:contract", "string")? != drawing::CONTRACT_VERSION
        || text(root, "drawing:schema", "string")? != schema_identity()?.to_string()
    {
        return Err(PlateError {
            rule_id: None,
            code: ErrorCode::UnsupportedVersion,
            message: "unsupported drawing schema or contract",
        });
    }
    let bad_id = |_| integrity("drawing identity is not a full UUIDv7");
    let digest = |name| {
        text(root, name, "string")?
            .parse::<ContentDigest>()
            .map_err(|_| integrity("drawing source digest is malformed"))
    };
    let id = DrawingId::parse(text(root, "drawing:id", "string")?).map_err(bad_id)?;
    let source = SourceModelPin {
        project: ProjectId::parse(text(root, "drawing:sourceProject", "string")?)
            .map_err(bad_id)?,
        revision: RevisionId::parse(text(root, "drawing:sourceRevision", "string")?)
            .map_err(bad_id)?,
        manifest: digest("drawing:sourceManifest")?,
        schema: digest("drawing:sourceSchema")?,
        generation: unsigned(root, "drawing:sourceGeneration", "uint64")?,
    };
    if source.project.to_string() != identity.project || id.to_string() != identity.workspace {
        return Err(integrity(
            "drawing identity disagrees with document manifest",
        ));
    }
    let origin = paper_origin(view)?;
    let anchors = match value(dim, "drawing:anchors", "string[]")? {
        Value::Array(values) => match values.as_slice() {
            [Value::Text(a), Value::Text(b)] => [FeatureId::parse(a), FeatureId::parse(b)]
                .into_iter()
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| integrity("invalid drawing anchors"))?,
            _ => return Err(integrity("drawing requires exactly two anchors")),
        },
        _ => return Err(integrity("invalid drawing anchors")),
    };
    let anchors: [FeatureId; 2] = anchors
        .try_into()
        .map_err(|_| integrity("drawing requires exactly two anchors"))?;
    let sheet: Option<SheetSpec> = match items.iter().find(|p| p.path == "/Drawing/Sheet") {
        None => None,
        Some(scope) if scope.type_name.as_deref() == Some("Scope") && scope.active => {
            // Admission re-validates every bound; the canonical re-encode comparison below
            // refuses any other topology or stray attribute.
            Some(SheetSpec {
                size: Sheet::parse(text(scope, "drawing:size", "token")?)
                    .ok_or_else(|| integrity("unsupported drawing sheet size"))?,
                frame: SheetFrame {
                    inset_mm: number(scope, "drawing:frameInset")?,
                },
                title: TitleBlock {
                    project: text(scope, "drawing:titleProject", "string")?.to_owned(),
                    author: text(scope, "drawing:titleAuthor", "string")?.to_owned(),
                },
            })
        }
        Some(_) => return Err(integrity("drawing prim profile disagrees")),
    };
    let drafting: Vec<DraftingSpec> = match items.iter().find(|p| p.path == "/Drawing/Drafting") {
        None => Vec::new(),
        Some(scope) if scope.type_name.as_deref() == Some("Scope") && scope.active => {
            // Canonical ordinal children E1..En with no gaps; the final canonical re-encode
            // comparison refuses any other topology or stray attribute.
            let mut entities = Vec::new();
            for item in ordinal_children(&items, "/Drawing/Drafting/E", MAX_DRAFTING_ENTITIES)? {
                let id = DraftingEntityId::parse(text(item, "drawing:id", "string")?)
                    .map_err(|_| integrity("drawing identity is not a full UUIDv7"))?;
                let kind = text(item, "drawing:kind", "token")?;
                let layer = DraftingEntityId::parse(text(item, "drawing:layer", "string")?)
                    .map_err(|_| integrity("drawing identity is not a full UUIDv7"))?;
                let line_type = LineType::parse(text(item, "drawing:linetype", "token")?)
                    .ok_or_else(|| integrity("invalid drawing line type"))?;
                let payload = match DraftingGeometry::payload_attribute(kind) {
                    Some(name) => Some(text(item, name, "string")?),
                    None => None,
                };
                let values = match value(item, "drawing:geometry", "double[]")? {
                    Value::Array(values) => values
                        .iter()
                        .map(|value| match value {
                            Value::Number(number) => Some(*number),
                            _ => None,
                        })
                        .collect::<Option<Vec<f64>>>()
                        .ok_or_else(|| integrity("drafting geometry is not an exact double set"))?,
                    _ => return Err(integrity("drafting geometry is not an exact double set")),
                };
                let geometry = DraftingGeometry::from_parts(kind, &values, payload)
                    .ok_or_else(|| integrity("drafting geometry disagrees with its kind"))?;
                entities.push(DraftingSpec {
                    id,
                    geometry,
                    layer,
                    line_type,
                });
            }
            entities
        }
        Some(_) => return Err(integrity("drawing prim profile disagrees")),
    };
    let layers: Vec<LayerSpec> = match items.iter().find(|p| p.path == "/Drawing/Layers") {
        None => Vec::new(),
        Some(scope) if scope.type_name.as_deref() == Some("Scope") && scope.active => {
            // Canonical ordinal children L1..Ln with no gaps; the final canonical re-encode
            // comparison refuses any other topology or stray attribute.
            let mut layers = Vec::new();
            for item in ordinal_children(&items, "/Drawing/Layers/L", MAX_LAYERS)? {
                layers.push(LayerSpec {
                    id: DraftingEntityId::parse(text(item, "drawing:id", "string")?)
                        .map_err(|_| integrity("drawing identity is not a full UUIDv7"))?,
                    name: text(item, "drawing:name", "string")?.to_owned(),
                    visible: match value(item, "drawing:visible", "bool")? {
                        Value::Bool(flag) => *flag,
                        _ => return Err(integrity("layer visibility is not a boolean")),
                    },
                });
            }
            layers
        }
        Some(_) => return Err(integrity("drawing prim profile disagrees")),
    };
    let views: Vec<ViewSpec> = match items.iter().find(|p| p.path == "/Drawing/Views") {
        None => Vec::new(),
        Some(scope) if scope.type_name.as_deref() == Some("Scope") && scope.active => {
            // Canonical ordinal children V1..Vn with no gaps; the final canonical re-encode
            // comparison refuses any other topology or stray attribute.
            let mut views = Vec::new();
            for item in ordinal_children(&items, "/Drawing/Views/V", MAX_VIEWS)? {
                views.push(ViewSpec {
                    id: DrawingViewId::parse(text(item, "drawing:id", "string")?)
                        .map_err(|_| integrity("drawing identity is not a full UUIDv7"))?,
                    plane: Plane::parse(text(item, "drawing:plane", "token")?)
                        .ok_or_else(|| integrity("invalid drawing plane"))?,
                    scale_denominator: number(item, "drawing:scale")?,
                    paper_origin_mm: paper_origin(item)?,
                });
            }
            views
        }
        Some(_) => return Err(integrity("drawing prim profile disagrees")),
    };
    let sections: Vec<SectionSpec> = match items.iter().find(|p| p.path == "/Drawing/Sections") {
        None => Vec::new(),
        Some(scope) if scope.type_name.as_deref() == Some("Scope") && scope.active => {
            let mut sections = Vec::new();
            for item in ordinal_children(&items, "/Drawing/Sections/S", MAX_SECTIONS)? {
                sections.push(SectionSpec {
                    id: DrawingViewId::parse(text(item, "drawing:id", "string")?)
                        .map_err(|_| integrity("drawing identity is not a full UUIDv7"))?,
                    plane: Plane::parse(text(item, "drawing:plane", "token")?)
                        .ok_or_else(|| integrity("invalid drawing plane"))?,
                    cut_mm: number(item, "drawing:cut")?,
                    side: CutSide::parse(text(item, "drawing:side", "token")?)
                        .ok_or_else(|| integrity("invalid drawing side"))?,
                    scale_denominator: number(item, "drawing:scale")?,
                    paper_origin_mm: paper_origin(item)?,
                });
            }
            sections
        }
        Some(_) => return Err(integrity("drawing prim profile disagrees")),
    };
    let details: Vec<DetailSpec> = match items.iter().find(|p| p.path == "/Drawing/Details") {
        None => Vec::new(),
        Some(scope) if scope.type_name.as_deref() == Some("Scope") && scope.active => {
            let mut details = Vec::new();
            for item in ordinal_children(&items, "/Drawing/Details/D", MAX_DETAILS)? {
                let [cx, cy] = match value(item, "drawing:center", "double[]")? {
                    Value::Array(values) => match values.as_slice() {
                        [Value::Number(x), Value::Number(y)] => [*x, *y],
                        _ => return Err(integrity("invalid drawing center")),
                    },
                    _ => return Err(integrity("invalid drawing center")),
                };
                details.push(DetailSpec {
                    id: DrawingViewId::parse(text(item, "drawing:id", "string")?)
                        .map_err(|_| integrity("drawing identity is not a full UUIDv7"))?,
                    parent: DrawingViewId::parse(text(item, "drawing:parent", "string")?)
                        .map_err(|_| integrity("drawing identity is not a full UUIDv7"))?,
                    center_paper_mm: [cx, cy],
                    radius_mm: number(item, "drawing:radius")?,
                    scale_denominator: number(item, "drawing:scale")?,
                    paper_origin_mm: paper_origin(item)?,
                });
            }
            details
        }
        Some(_) => return Err(integrity("drawing prim profile disagrees")),
    };
    let drawing = Drawing::admit(DrawingSpec {
        id,
        name: text(root, "drawing:name", "string")?.to_owned(),
        source,
        member: ElementId::parse(text(root, "drawing:member", "string")?).map_err(bad_id)?,
        view: ViewSpec {
            id: DrawingViewId::parse(text(view, "drawing:id", "string")?).map_err(bad_id)?,
            plane: Plane::parse(text(view, "drawing:plane", "token")?)
                .ok_or_else(|| integrity("invalid drawing plane"))?,
            scale_denominator: number(view, "drawing:scale")?,
            paper_origin_mm: origin,
        },
        dimension: DimensionSpec {
            id: DimensionId::parse(text(dim, "drawing:id", "string")?).map_err(bad_id)?,
            anchors,
            paper_offset_mm: number(dim, "drawing:offset")?,
            decimal_places: u8::try_from(unsigned(dim, "drawing:decimals", "uint")?)
                .map_err(|_| integrity("drawing numeric format is oversized"))?,
        },
        drafting,
        views,
        sections,
        details,
        sheet,
        layers,
    })?;
    if encode(&drawing)?.bytes != asset.bytes {
        return Err(integrity(
            "drawing intent is outside the canonical closed schema",
        ));
    }
    Ok(drawing)
}
