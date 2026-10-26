// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D01 closed, canonical USDA intent. Writing requires admitted engine intent; reading goes
//! through the bounded layer engine and the same engine admission, never through a raw setter.

use crate::plate::{PlateError, integrity};
use beamcraft_engine::drawing::{
    self, DimensionSpec, Drawing, DrawingSpec, Plane, SourceModelPin, ViewSpec,
};
use beamcraft_engine::query::feature::FeatureId;
use beamcraft_layer_engine::{
    Asset, BACKEND, Budgets, Control, Identity, OpenRequest, Prim, Profile, Reader, Value,
};
use beamcraft_model::digest::{ContentDigest, Sha256};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{
    DimensionId, DrawingId, DrawingViewId, ElementId, ProjectId, RevisionId,
};
use std::collections::BTreeSet;

/// Sole authored drawing asset, distinct from any engineering-model root.
pub const INTENT_LAYER: &str = "drawing.usda";
/// Tight cap checked before copying or parsing a document intent.
pub const MAX_INTENT_BYTES: usize = 16_384;
/// Closed descriptor whose SHA-256 is the drawing schema identity.
pub const SCHEMA_DESCRIPTOR: &str = "beamcraft-drawing-usd/1;root=/Drawing;view=/Drawing/View;dimension=/Drawing/Dimension;intent=beamcraft-drawing/1;source=project,revision,manifest,schema,generation;anchors=qualified-member-vertices;units=mm;projection=front,top,right;canonical-usda";

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
    text.push_str(&format!("    def Scope \"Dimension\"\n    {{\n        custom string drawing:id = {}\n        custom string[] drawing:anchors = [{}, {}]\n        custom double drawing:offset = {}\n        custom uint drawing:decimals = {}\n    }}\n}}\n", quote(&s.dimension.id.to_string()), quote(&a.to_string()), quote(&b.to_string()), s.dimension.paper_offset_mm, s.dimension.decimal_places));
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
            prims: 3,
            // Account for Sdf property-child lists and per-attribute metadata as well as values.
            // The closed semantic reader below still requires exactly two anchors/two origins.
            values: 512,
            array_items: 32,
            decoded_bytes: 65_536,
            snapshot_bytes: 65_536,
            page_items: 3,
        },
    };
    // Asset digest and identity come from the caller's manifest. This is the wrapper's distinct
    // commitment over that already authorized input, not a new assertion of byte ownership.
    request.manifest = request.manifest_digest()?;
    let mut reader = Reader::default();
    let opened = reader.open(request, control)?;
    if opened.prim_count != 3 || opened.meters_per_unit != 0.001 || opened.up_axis != "Z" {
        return Err(integrity("drawing stage profile disagrees"));
    }
    let page = reader.read(&opened.context, None, control)?;
    if page.next.is_some() {
        return Err(limit());
    }
    let prim = |path: &str, kind: &str| {
        page.items
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
    let origin = match value(view, "drawing:origin", "double[]")? {
        Value::Array(values) => match values.as_slice() {
            [Value::Number(x), Value::Number(y)] => [*x, *y],
            _ => return Err(integrity("invalid drawing origin")),
        },
        _ => return Err(integrity("invalid drawing origin")),
    };
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
    })?;
    if encode(&drawing)?.bytes != asset.bytes {
        return Err(integrity(
            "drawing intent is outside the canonical closed schema",
        ));
    }
    Ok(drawing)
}
