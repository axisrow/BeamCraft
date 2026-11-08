// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

use super::*;
use openusd::{
    gf::Vec3f,
    sdf::{self, AbstractData},
    usda::TextWriter,
};
use std::io::{self, Write};

/// The optional `double` of a definition the platform authors itself; absent stores nothing.
fn optional_number(
    data: &mut sdf::Data,
    p: &str,
    name: &str,
    value: Option<f64>,
) -> Result<(), Diagnostic> {
    match value {
        Some(v) => attr(data, p, name, "double", sdf::Value::Double(v)),
        None => Ok(()),
    }
}
/// The optional `string` of a definition the platform authors itself; absent stores nothing.
fn optional_text(
    data: &mut sdf::Data,
    p: &str,
    name: &str,
    value: &Option<String>,
) -> Result<(), Diagnostic> {
    match value {
        Some(v) => attr(data, p, name, "string", sdf::Value::String(v.clone())),
        None => Ok(()),
    }
}

/// Parses an authored prim or property path; the seam a discipline hook builds target paths
/// with.
pub fn path(text: &str) -> Result<sdf::Path, Diagnostic> {
    sdf::Path::new(text).map_err(|_| corrupt())
}
pub(super) fn data(asset: &Asset) -> Result<sdf::Data, Diagnostic> {
    if !asset.name.ends_with(".usda") {
        return Err(unsupported());
    }
    parse_bytes(&asset.bytes)
}
fn parse_bytes(bytes: &[u8]) -> Result<sdf::Data, Diagnostic> {
    let text = std::str::from_utf8(bytes).map_err(|_| corrupt())?;
    openusd::usda::parse(text).map_err(|_| corrupt())
}
/// Reads the `subLayers` list of a USDA layer's bytes. U19 publication compares the root
/// composition of the base and the candidate to prove no layer was added, dropped or reordered
/// by the workspace's work (`DV-SCOPE-003`).
pub(super) fn layer_sublayers(bytes: &[u8]) -> Result<Vec<String>, Diagnostic> {
    let layer = parse_bytes(bytes)?;
    let Some(value) = layer
        .try_field(&sdf::Path::abs_root(), "subLayers")
        .map_err(|_| corrupt())?
    else {
        return Ok(Vec::new());
    };
    match value.as_ref() {
        sdf::Value::StringVec(layers) => Ok(layers.clone()),
        _ => Err(invalid()),
    }
}
/// Inspect each owned USDA marker after the reader has verified the manifest and bytes.
pub(super) fn schema_pins(request: &OpenRequest) -> Result<(), Diagnostic> {
    for asset in request
        .assets
        .iter()
        .filter(|asset| request.owned_layers.contains(&asset.name) && asset.name.ends_with(".usda"))
    {
        let layer = data(asset)?;
        if let Some(value) = layer
            .try_field(&sdf::Path::abs_root(), "customLayerData")
            .map_err(|_| corrupt())?
            && let sdf::Value::Dictionary(fields) = value.as_ref()
            && let Some(pin) = fields.get("bimSchemaDigest")
            && !matches!(pin, sdf::Value::String(value) if pin_accepted(value)?)
        {
            return Err(fail(
                ErrorCode::UnsupportedVersion,
                "layer schema digest mismatch",
            ));
        }
    }
    Ok(())
}
pub(super) fn targets(request: &OpenRequest, targets: &EditTargets) -> Result<(), Diagnostic> {
    if targets.authored == targets.derived
        || targets.authored == request.root
        || targets.derived == request.root
    {
        return Err(invalid());
    }
    for name in [&request.root, &targets.authored, &targets.derived] {
        if !request.owned_layers.contains(name) {
            return Err(fail(ErrorCode::ForeignReadOnly, "edit layer is not owned"));
        }
        if !name.ends_with(".usda") {
            return Err(unsupported());
        }
        if !request.assets.iter().any(|asset| asset.name == *name) {
            return Err(corrupt());
        }
    }
    schema_pins(request)?;
    let root = data(
        request
            .assets
            .iter()
            .find(|a| a.name == request.root)
            .ok_or_else(corrupt)?,
    )?;
    let value = root
        .get_field(&sdf::Path::abs_root(), "subLayers")
        .map_err(|_| invalid())?;
    let sdf::Value::StringVec(layers) = value.as_ref() else {
        return Err(invalid());
    };
    let names = layers
        .iter()
        .map(|n| preflight::anchored(n, &request.root))
        .collect::<Result<BTreeSet<_>, _>>()?;
    if !names.contains(&targets.authored) || !names.contains(&targets.derived) {
        return Err(fail(
            ErrorCode::DependencyUnavailable,
            "edit layers must participate in root stack",
        ));
    }
    // U15: the derived stack lists exactly the per-element derived layers of the manifest. An
    // element-file name outside the fixed shape, an unlisted file or a stack entry without its
    // file is a broken stage, not a layer the engine may edit. The stack root's own name
    // ("derived.usda") is not 32 hex digits, so it never classifies as an element file.
    let derived_asset = request
        .assets
        .iter()
        .find(|a| a.name == targets.derived)
        .ok_or_else(corrupt)?;
    let derived_data = data(derived_asset)?;
    let stack = match derived_data
        .try_field(&sdf::Path::abs_root(), "subLayers")
        .map_err(|_| corrupt())?
    {
        None => BTreeSet::new(),
        Some(value) => match value.as_ref() {
            sdf::Value::StringVec(entries) => entries
                .iter()
                .map(|n| preflight::anchored(n, &targets.derived))
                .collect::<Result<BTreeSet<_>, _>>()?,
            _ => return Err(invalid()),
        },
    };
    let mut element_files = BTreeSet::new();
    for asset in &request.assets {
        if element_layer_id(&asset.name).is_some() {
            let Some(id) = element_layer_id(&asset.name) else {
                return Err(corrupt());
            };
            if !uuid7(&id) || !request.owned_layers.contains(&asset.name) {
                return Err(corrupt());
            }
            if !stack.contains(&asset.name) {
                return Err(corrupt());
            }
            element_files.insert(asset.name.clone());
        }
    }
    // The referenced asset layer (U15) hangs beside the stack as a plain member.
    let geometry_present = request
        .assets
        .iter()
        .any(|a| a.name == GEOMETRY_ASSET_LAYER);
    let stack_without_elements = stack
        .iter()
        .filter(|n| element_layer_id(n).is_none())
        .cloned()
        .collect::<BTreeSet<_>>();
    let expected = if geometry_present {
        let mut expected = BTreeSet::new();
        expected.insert(GEOMETRY_ASSET_LAYER.to_owned());
        expected
    } else {
        BTreeSet::new()
    };
    if stack_without_elements != expected {
        return Err(corrupt());
    }
    if geometry_present
        && (!request.owned_layers.contains(GEOMETRY_ASSET_LAYER)
            || !stack.contains(GEOMETRY_ASSET_LAYER))
    {
        return Err(corrupt());
    }
    Ok(())
}
pub(super) fn field(
    data: &mut sdf::Data,
    p: &str,
    key: &str,
    value: sdf::Value,
) -> Result<(), Diagnostic> {
    let p = path(p)?;
    if !data.has_spec(&p) {
        return Err(corrupt());
    }
    data.set_field(&p, key, value);
    Ok(())
}
pub(super) fn child(
    data: &mut sdf::Data,
    parent: &str,
    key: &str,
    name: &str,
    remove: bool,
) -> Result<(), Diagnostic> {
    let p = path(parent)?;
    let mut names = match data.try_field(&p, key).map_err(|_| corrupt())? {
        Some(v) => match v.as_ref() {
            sdf::Value::TokenVec(v) => v.clone(),
            _ => return Err(unsupported()),
        },
        None => Vec::new(),
    };
    if remove {
        names.retain(|n| n.as_str() != name);
    } else if !names.iter().any(|n| n.as_str() == name) {
        names.push(name.into());
    }
    field(data, parent, key, sdf::Value::TokenVec(names))
}
pub(super) fn prim(
    data: &mut sdf::Data,
    p: &str,
    ty: Option<&str>,
    defining: bool,
) -> Result<(), Diagnostic> {
    if p == "/" {
        if !data.has_spec(&sdf::Path::abs_root()) {
            data.create_spec(sdf::Path::abs_root(), sdf::SpecType::PseudoRoot);
        }
        return Ok(());
    }
    if p.split('/').count() > 17 {
        return Err(limit());
    }
    let (parent, name) = p.rsplit_once('/').ok_or_else(corrupt)?;
    let parent = if parent.is_empty() { "/" } else { parent };
    let parsed = path(p)?;
    if !data.has_spec(&parsed) {
        prim(data, parent, None, false)?;
        data.create_spec(parsed, sdf::SpecType::Prim);
        field(
            data,
            p,
            "specifier",
            sdf::Value::Specifier(if defining {
                sdf::Specifier::Def
            } else {
                sdf::Specifier::Over
            }),
        )?;
        child(data, parent, "primChildren", name, false)?;
    }
    if let Some(ty) = ty {
        field(data, p, "typeName", sdf::Value::token(ty))?;
    }
    Ok(())
}
pub(super) fn container(
    data: &mut sdf::Data,
    p: &str,
    ty: &str,
    kind: Option<&str>,
) -> Result<(), Diagnostic> {
    if !data.has_spec(&path(p)?) {
        prim(data, p, Some(ty), true)?;
    }
    if let Some(kind) = kind {
        field(data, p, "kind", sdf::Value::token(kind))?;
    }
    Ok(())
}
/// Authors one attribute opinion on a prim; the seam a discipline hook writes its attributes
/// with.
pub fn attribute(
    data: &mut sdf::Data,
    p: &str,
    name: &str,
    ty: &str,
    opinion: Opinion<sdf::Value>,
    custom: bool,
) -> Result<(), Diagnostic> {
    prim(data, p, None, false)?;
    let full = path(&format!("{p}.{name}"))?;
    if matches!(opinion, Opinion::Remove) {
        data.erase_spec(&full);
        child(data, p, "propertyChildren", name, true)?;
        return Ok(());
    }
    if !data.has_spec(&full) {
        data.create_spec(full.clone(), sdf::SpecType::Attribute);
        child(data, p, "propertyChildren", name, false)?;
    } else if data.spec_type(&full) != Some(sdf::SpecType::Attribute) {
        return Err(unsupported());
    }
    data.set_field(&full, "typeName", sdf::Value::token(ty));
    data.set_field(&full, "custom", sdf::Value::Bool(custom));
    data.set_field(
        &full,
        "variability",
        sdf::Value::Variability(sdf::Variability::Varying),
    );
    match opinion {
        Opinion::Value(v) => data.set_field(&full, "default", v),
        Opinion::Block => data.set_field(&full, "default", sdf::Value::ValueBlock),
        Opinion::Absent => data.erase_field(&full, "default"),
        Opinion::Remove => (),
    }
    Ok(())
}
/// Authors one custom attribute value; the seam's value-writing shorthand.
pub fn attr(
    data: &mut sdf::Data,
    p: &str,
    name: &str,
    ty: &str,
    v: sdf::Value,
) -> Result<(), Diagnostic> {
    attribute(data, p, name, ty, Opinion::Value(v), true)
}
/// Authors a definition-target relationship (`bim:material`, `bim:profile`); the seam a
/// discipline hook links immutable definitions with.
pub fn relationship(
    data: &mut sdf::Data,
    p: &str,
    name: &str,
    opinion: MaterialOpinion,
) -> Result<(), Diagnostic> {
    let targets = match opinion {
        MaterialOpinion::Target(d) => Some(vec![path(&definition_path(
            d,
            if name == "bim:material" {
                DefinitionCategory::Material
            } else {
                DefinitionCategory::Profile
            },
        ))?]),
        MaterialOpinion::Empty => Some(Vec::new()),
        MaterialOpinion::Remove => None,
    };
    relationship_targets(data, p, name, targets)
}
/// Authors an explicit relationship target list, or removes the relationship spec (`None`).
/// The seam a discipline hook writes its element relationships with.
pub fn relationship_targets(
    data: &mut sdf::Data,
    p: &str,
    name: &str,
    targets: Option<Vec<sdf::Path>>,
) -> Result<(), Diagnostic> {
    prim(data, p, None, false)?;
    let full = path(&format!("{p}.{name}"))?;
    let Some(targets) = targets else {
        // Refuse erasing unknown relationship-target specs/metadata that this profile cannot model.
        if data
            .spec_paths()
            .iter()
            .any(|s| s.to_string().starts_with(&format!("{p}.{name}[")))
        {
            return Err(unsupported());
        }
        data.erase_spec(&full);
        child(data, p, "propertyChildren", name, true)?;
        return Ok(());
    };
    if !data.has_spec(&full) {
        data.create_spec(full.clone(), sdf::SpecType::Relationship);
        child(data, p, "propertyChildren", name, false)?;
    } else if data.spec_type(&full) != Some(sdf::SpecType::Relationship) {
        return Err(unsupported());
    }
    data.set_field(&full, "custom", sdf::Value::Bool(true));
    data.set_field(
        &full,
        "variability",
        sdf::Value::Variability(sdf::Variability::Uniform),
    );
    data.set_field(
        &full,
        "targetPaths",
        sdf::Value::PathListOp(sdf::ListOp::explicit(targets)),
    );
    Ok(())
}
/// Builds the `double3[]` value of a point list; the seam's point-array encoding.
pub fn points(values: &[[f64; 3]]) -> sdf::Value {
    sdf::Value::Vec3dVec(values.iter().map(|p| (*p).into()).collect())
}
pub(super) fn pose(data: &mut sdf::Data, p: &str, value: &Placement) -> Result<(), Diagnostic> {
    let [x, y, z] = value.translation_mm;
    let [qx, qy, qz, qw] = value.orientation_xyzw;
    for (name, ty, v) in [
        ("xformOp:translate", "double3", sdf::Value::vec3d(x, y, z)),
        ("xformOp:orient", "quatd", sdf::Value::quatd(qw, qx, qy, qz)),
        (
            "xformOpOrder",
            "token[]",
            sdf::Value::token_vec(["xformOp:translate", "xformOp:orient"]),
        ),
    ] {
        attribute(data, p, name, ty, Opinion::Value(v), false)?;
    }
    Ok(())
}
fn units(data: &mut sdf::Data, schema: Option<SchemaIdentity>) -> Result<(), Diagnostic> {
    prim(data, "/", None, false)?;
    field(data, "/", "metersPerUnit", sdf::Value::Double(0.001))?;
    field(data, "/", "upAxis", sdf::Value::token("Z"))?;
    if let Some(schema) = schema {
        let root = sdf::Path::abs_root();
        let mut dictionary = match data
            .try_field(&root, "customLayerData")
            .map_err(|_| corrupt())?
        {
            Some(v) => match v.as_ref() {
                sdf::Value::Dictionary(v) => v.clone(),
                _ => return Err(corrupt()),
            },
            None => Default::default(),
        };
        dictionary.insert(
            "bimSchemaDigest".into(),
            sdf::Value::String(schema.digest.to_string()),
        );
        field(
            data,
            "/",
            "customLayerData",
            sdf::Value::Dictionary(dictionary),
        )?;
    }
    Ok(())
}
fn definition(data: &mut sdf::Data, p: &str, value: &Definition) -> Result<(), Diagnostic> {
    prim(data, p, Some("Scope"), true)?;
    attr(
        data,
        p,
        "bim:schemaVersion",
        "string",
        sdf::Value::String("0.1.0".into()),
    )?;
    attr(data, p, "bim:units", "token", sdf::Value::token("mm"))?;
    match value {
        Definition::Material {
            density_kg_m3,
            grade,
            standard,
            ..
        } => {
            attr(
                data,
                p,
                "bim:definitionKind",
                "token",
                sdf::Value::token("material"),
            )?;
            optional_number(data, p, "bim:densityKgM3", *density_kg_m3)?;
            optional_text(data, p, "bim:materialGrade", grade)?;
            optional_text(data, p, "bim:materialStandard", standard)?;
        }
        Definition::Discipline(hook) => hook.0.author(data, p)?,
    }
    let display_name = match value {
        Definition::Material { display_name, .. } => display_name.as_deref(),
        Definition::Discipline(hook) => hook.0.display_name(),
    };
    if let Some(name) = display_name {
        field(data, p, "displayName", sdf::Value::String(name.to_owned()))?;
    }
    Ok(())
}
pub(super) fn definition_bytes(d: &Definition) -> Result<Vec<u8>, Diagnostic> {
    validate::definition(d)?;
    let mut data = sdf::Data::new();
    units(&mut data, None)?;
    definition(&mut data, "/Definition", d)?;
    serialize(&data, Budgets::default().layer_bytes)
}
struct BoundedOutput {
    bytes: Vec<u8>,
    max: usize,
}
impl Write for BoundedOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.bytes.len().saturating_add(bytes.len()) > self.max {
            return Err(io::ErrorKind::FileTooLarge.into());
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub(super) fn serialize(data: &sdf::Data, max: usize) -> Result<Vec<u8>, Diagnostic> {
    let mut output = BoundedOutput {
        bytes: Vec::new(),
        max,
    };
    TextWriter::write(data, &mut output).map_err(|_| limit())?;
    Ok(output.bytes)
}
fn create(
    data: &mut sdf::Data,
    e: &Element,
    actor: &str,
    input: &Prepare,
    prims: &[Prim],
) -> Result<(), Diagnostic> {
    let p = element_path(&e.id, shape_category(&e.shape))?;
    container(data, "/World", "Xform", Some("assembly"))?;
    container(data, "/World/Steel", "Scope", Some("group"))?;
    let folder = p
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .ok_or_else(corrupt)?;
    container(data, folder, "Scope", Some("group"))?;
    prim(data, &p, Some("Xform"), true)?;
    let model_kind = match e.shape {
        Shape::Assembly => "assembly",
        // An instance is a component; its part kind lives only in bim:elementKind.
        _ => "component",
    };
    field(data, &p, "kind", sdf::Value::token(model_kind))?;
    field(data, &p, "displayName", sdf::Value::String(e.name.clone()))?;
    let kind = match &e.shape {
        Shape::Assembly => "assembly",
        Shape::Discipline(hook) => hook.0.kind_token(),
        Shape::Level { .. } => "level",
        Shape::Grid(_) => "grid",
        Shape::Construction(_) => "construction",
        Shape::Annotation(_) => "annotation",
    };
    for (name, ty, v) in [
        ("bim:id", "string", sdf::Value::String(e.id.clone())),
        ("bim:name", "string", sdf::Value::String(e.name.clone())),
        ("bim:elementKind", "token", sdf::Value::token(kind)),
        ("bim:geometryMode", "token", sdf::Value::token("parametric")),
        (
            "bim:schemaVersion",
            "string",
            sdf::Value::String("0.1.0".into()),
        ),
        ("bim:createdBy", "string", sdf::Value::String(actor.into())),
    ] {
        attr(data, &p, name, ty, v)?;
    }
    if matches!(&e.shape, Shape::Discipline(hook) if hook.0.classed()) {
        attr(data, &p, "steel:class", "int", sdf::Value::Int(0))?;
        appearance::author_element(data, &p, 0)?;
    }
    match &e.shape {
        Shape::Assembly => (),
        Shape::Discipline(hook) => hook.0.author(&mut hook::AuthorCx {
            data,
            path: &p,
            element: e,
            input,
            prims,
        })?,
        Shape::Grid(g) => grid::author(data, &p, g)?,
        Shape::Construction(o) => construction::author(data, &p, o)?,
        Shape::Annotation(a) => annotation::author(data, &p, a)?,
        Shape::Level { elevation_mm } => attr(
            data,
            &p,
            "bim:elevationMm",
            "double",
            sdf::Value::Double(*elevation_mm),
        )?,
    }
    if let Some(d) = e.material {
        relationship(data, &p, "bim:material", MaterialOpinion::Target(d))?;
    }
    // The owned plane-cut records of a plate, beam or column (A03, amendment A-2), as five
    // parallel attributes in ascending ordinal order; the hook writes them, or nothing.
    if let Shape::Discipline(hook) = &e.shape {
        hook.0.author_cuts(data, &p)?;
    }
    pose(data, &p, &e.placement)
}

/// Authors the persistent ordinal and retired-identity bookkeeping of one operation-record
/// family; the seam a discipline hook keeps its record history through.
pub fn author_record_history(
    data: &mut sdf::Data,
    p: &str,
    source: Option<&Prim>,
    records: &[(beamcraft_model::id::OpId, usize)],
    family: cut_history::Family,
) -> Result<(), Diagnostic> {
    let (next_name, retired_name, id_name, ordinal_name) = cut_history::fields(family);
    let old = source
        .map(|p| match family {
            cut_history::Family::Cut => cut_history::read(p),
            cut_history::Family::PlateHole => cut_history::read_holes(p),
            cut_history::Family::MemberHole => cut_history::read_member_holes(p),
        })
        .transpose()?
        .unwrap_or_default();
    let mut next = usize::from(old.next_ordinal);
    let mut retired = old
        .retired
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    let old_pairs = if let Some(source) = source {
        cut_history::ids(source, id_name)?
            .into_iter()
            .zip(cut_history::ordinals(source, ordinal_name)?)
            .collect::<BTreeMap<_, _>>()
    } else {
        BTreeMap::new()
    };
    for id in old_pairs.keys() {
        if !records.iter().any(|(new, _)| new == id) {
            retired.insert(*id);
        }
    }
    for (id, ordinal) in records {
        if *ordinal > 999 || retired.contains(id) {
            return Err(invalid());
        }
        match old_pairs.get(id) {
            Some(old) if usize::from(*old) != *ordinal => return Err(invalid()),
            None if *ordinal < usize::from(old.next_ordinal) => return Err(invalid()),
            _ => {}
        }
        next = next.max(ordinal + 1);
    }
    if next > 1000 || retired.len() > 1000 {
        return Err(limit());
    }
    if next > 0 {
        attr(
            data,
            p,
            next_name,
            "int",
            sdf::Value::Int(i32::try_from(next).map_err(|_| limit())?),
        )?;
    }
    if !retired.is_empty() {
        attr(
            data,
            p,
            retired_name,
            "string[]",
            sdf::Value::StringVec(retired.into_iter().map(|id| id.to_string()).collect()),
        )?;
    }
    Ok(())
}

/// The identity of the immutable geometry asset (U11) that a derived body pins: the full
/// SHA-256 of the canonical asset content — the engine shape key followed by the exact stored
/// float/int buffers. Equal bodies therefore share one asset, and the name always recomputes
/// from the content it labels.
pub fn mesh_asset_digest(m: &Mesh) -> Result<ContentDigest, Diagnostic> {
    let mut hash = Sha256::new();
    hash.update(b"beamcraft-geometry-asset-v1")
        .map_err(|_| limit())?;
    hash.update(m.shape_key.as_bytes()).map_err(|_| limit())?;
    let mut chunk = |bytes: &[u8]| {
        hash.update(&(bytes.len() as u64).to_le_bytes())
            .map_err(|_| limit())?;
        hash.update(bytes).map_err(|_| limit())
    };
    for p in &m.points_mm {
        chunk(&p[0].to_le_bytes())?;
        chunk(&p[1].to_le_bytes())?;
        chunk(&p[2].to_le_bytes())?;
    }
    let mut numbers = Vec::new();
    for n in m.face_counts.iter().chain(m.indices.iter()) {
        numbers.extend_from_slice(&n.to_le_bytes());
    }
    chunk(&numbers)?;
    Ok(hash.finalize())
}

/// The canonical path of the immutable geometry asset a derived body pins (U16 naming: the
/// digest lives in the prim name, as for `MAT_`/`PRF_` definitions).
fn mesh_asset_path(m: &Mesh) -> Result<(String, String), Diagnostic> {
    let digest = mesh_asset_digest(m)?;
    let name = format!("GEO_{digest}");
    Ok((format!("{GEOMETRY_ASSETS}/{name}"), name))
}

/// The authored-layer scope holding the immutable geometry assets (U11).
pub const GEOMETRY_ASSETS: &str = "/World/Definitions/Geometry";

fn mesh(
    assets: &mut sdf::Data,
    data: &mut sdf::Data,
    m: &Mesh,
    parent: &str,
    purpose: &str,
) -> Result<(), Diagnostic> {
    let extent = derive::extent(&m.points_mm).ok_or_else(invalid)?;
    let normals =
        derive::face_normals(&m.points_mm, &m.face_counts, &m.indices).ok_or_else(invalid)?;
    // The asset prim is written once per distinct body into the shared derived stack root
    // (U15): an existing spec with the same content-addressed name is left untouched, so equal
    // bodies across a batch, across elements and across revisions share one immutable
    // definition, and an edit of another element never re-serializes it.
    let (asset, _) = mesh_asset_path(m)?;
    container(assets, GEOMETRY_ASSETS, "Scope", None)?;
    if !assets.has_spec(&path(&asset)?) {
        prim(assets, &asset, Some("Mesh"), true)?;
        for (name, ty, v) in [
            (
                "points",
                "point3f[]",
                sdf::Value::Vec3fVec(m.points_mm.iter().map(|p| (*p).into()).collect()),
            ),
            (
                "faceVertexCounts",
                "int[]",
                sdf::Value::IntVec(m.face_counts.clone()),
            ),
            (
                "faceVertexIndices",
                "int[]",
                sdf::Value::IntVec(m.indices.clone()),
            ),
            ("subdivisionScheme", "token", sdf::Value::token("none")),
            ("orientation", "token", sdf::Value::token("rightHanded")),
            (
                "extent",
                "float3[]",
                sdf::Value::Vec3fVec(extent.iter().map(|p| (*p).into()).collect()),
            ),
            (
                "normals",
                "normal3f[]",
                sdf::Value::Vec3fVec(normals.iter().map(|n| (*n).into()).collect()),
            ),
        ] {
            attribute(assets, &asset, name, ty, Opinion::Value(v), false)?;
        }
        let normals_spec = path(&format!("{asset}.normals"))?;
        assets.set_field(&normals_spec, "interpolation", sdf::Value::token("uniform"));
        attr(
            assets,
            &asset,
            "bim:shapeKey",
            "string",
            sdf::Value::String(m.shape_key.to_string()),
        )?;
    }
    // The element's own prim is an instanceable reference (U11): the composed Mesh comes from
    // the shared asset, and only the U12 purpose and the per-element quantity record stay
    // local. The reference targets the asset layer explicitly (U15): the body lives in the
    // sibling `derived/geometry.usda`, so the anchored external target composes the shared
    // body in every reader without a layer-stack cycle. The exactness marker is absent
    // (`true` being the default), as U12 stores it.
    let p = format!("{parent}/Geometry");
    prim(data, &p, Some("Mesh"), true)?;
    field(data, &p, "kind", sdf::Value::token("subcomponent"))?;
    field(data, &p, "instanceable", sdf::Value::Bool(true))?;
    field(
        data,
        &p,
        "references",
        sdf::Value::ReferenceListOp(sdf::ListOp::explicit([sdf::Reference {
            asset_path: super::GEOMETRY_ASSET_LAYER
                .rsplit_once('/')
                .map(|(_, file)| file.to_owned())
                .unwrap_or_else(|| super::GEOMETRY_ASSET_LAYER.to_owned()),
            prim_path: path(&asset)?,
            layer_offset: sdf::LayerOffset::IDENTITY,
            custom_data: Default::default(),
        }])),
    )?;
    attr(data, &p, "purpose", "token", sdf::Value::token(purpose))?;
    if let Some(q) = &m.quantities {
        let mut fields = vec![
            ("bim:volumeMm3", "double", sdf::Value::Double(q.volume_mm3)),
            (
                "bim:volumeStatus",
                "token",
                sdf::Value::token(&q.volume_status),
            ),
            ("bim:massStatus", "token", sdf::Value::token(&q.mass_status)),
            (
                "bim:shapeSignature",
                "string",
                sdf::Value::String(q.shape_signature.to_string()),
            ),
            (
                "bim:massSignature",
                "string",
                sdf::Value::String(q.mass_signature.to_string()),
            ),
            (
                "bim:quantityAlgorithm",
                "string",
                sdf::Value::String(q.algorithm.clone()),
            ),
        ];
        if let Some(kg) = q.mass_kg {
            fields.push(("bim:massKg", "double", sdf::Value::Double(kg)));
        }
        for (name, ty, value) in fields {
            attr(data, &p, name, ty, value)?;
        }
    }
    // The U12 representation levels exist only where their body differs from the full one, so
    // every authored prim carries content no other level repeats. A level is a display result:
    // it never carries a quantity record, and each carries `bim:exact = false` (the exactness
    // marker the acceptance reads; the full mesh stores none, `true` being the default).
    // Levels stay inline, per-element prims: unlike the shared full body they are authored
    // only where they differ, so they take no part in the U11 asset sharing.
    if let Some(r) = &m.reduced {
        mesh_prim(data, r, &format!("{parent}/Reduced"), "default", false)?;
    }
    if let Some(r) = &m.proxy {
        mesh_prim(data, r, &format!("{parent}/Proxy"), "proxy", false)?;
    }
    if let Some(points) = &m.axis {
        axis_prim(data, &format!("{parent}/Axis"), points)?;
    }
    Ok(())
}

/// One inline mesh body at `path` (a U12 representation level): the standard buffer, the
/// level's own shape key, the purpose and exactness marks. `exact = false` is authored, `true`
/// is stored as absence.
fn mesh_prim(
    data: &mut sdf::Data,
    m: &Mesh,
    p: &str,
    purpose: &str,
    exact: bool,
) -> Result<(), Diagnostic> {
    prim(data, p, Some("Mesh"), true)?;
    field(data, p, "kind", sdf::Value::token("subcomponent"))?;
    let extent = derive::extent(&m.points_mm).ok_or_else(invalid)?;
    let normals =
        derive::face_normals(&m.points_mm, &m.face_counts, &m.indices).ok_or_else(invalid)?;
    for (name, ty, v) in [
        (
            "points",
            "point3f[]",
            sdf::Value::Vec3fVec(m.points_mm.iter().map(|p| (*p).into()).collect()),
        ),
        (
            "faceVertexCounts",
            "int[]",
            sdf::Value::IntVec(m.face_counts.clone()),
        ),
        (
            "faceVertexIndices",
            "int[]",
            sdf::Value::IntVec(m.indices.clone()),
        ),
        ("subdivisionScheme", "token", sdf::Value::token("none")),
        ("orientation", "token", sdf::Value::token("rightHanded")),
        (
            "extent",
            "float3[]",
            sdf::Value::Vec3fVec(extent.iter().map(|p| (*p).into()).collect()),
        ),
        (
            "normals",
            "normal3f[]",
            sdf::Value::Vec3fVec(normals.iter().map(|n| (*n).into()).collect()),
        ),
    ] {
        attribute(data, p, name, ty, Opinion::Value(v), false)?;
    }
    let normals_spec = path(&format!("{p}.normals"))?;
    data.set_field(&normals_spec, "interpolation", sdf::Value::token("uniform"));
    attr(data, p, "purpose", "token", sdf::Value::token(purpose))?;
    attr(
        data,
        p,
        "bim:shapeKey",
        "string",
        sdf::Value::String(m.shape_key.to_string()),
    )?;
    if !exact {
        attr(data, p, "bim:exact", "bool", sdf::Value::Bool(false))?;
    }
    if let Some(q) = &m.quantities {
        let mut fields = vec![
            ("bim:volumeMm3", "double", sdf::Value::Double(q.volume_mm3)),
            (
                "bim:volumeStatus",
                "token",
                sdf::Value::token(&q.volume_status),
            ),
            ("bim:massStatus", "token", sdf::Value::token(&q.mass_status)),
            (
                "bim:shapeSignature",
                "string",
                sdf::Value::String(q.shape_signature.to_string()),
            ),
            (
                "bim:massSignature",
                "string",
                sdf::Value::String(q.mass_signature.to_string()),
            ),
            (
                "bim:quantityAlgorithm",
                "string",
                sdf::Value::String(q.algorithm.clone()),
            ),
        ];
        if let Some(kg) = q.mass_kg {
            fields.push(("bim:massKg", "double", sdf::Value::Double(kg)));
        }
        for (name, ty, value) in fields {
            attr(data, p, name, ty, value)?;
        }
    }
    Ok(())
}

/// The U12 axis level: one linear, nonperiodic `BasisCurves` polyline through the element-local
/// axis points (`purpose = guide`, `bim:exact = false`). A member's display axis, never an
/// engineering body: it carries no quantity record and no shape key.
fn axis_prim(data: &mut sdf::Data, p: &str, points: &[[f64; 3]]) -> Result<(), Diagnostic> {
    prim(data, p, Some("BasisCurves"), true)?;
    field(data, p, "kind", sdf::Value::token("subcomponent"))?;
    let count = i32::try_from(points.len()).map_err(|_| invalid())?;
    for (name, ty, v, uniform) in [
        (
            "points",
            "point3f[]",
            sdf::Value::Vec3fVec(
                points
                    .iter()
                    .map(|p| Vec3f::from([p[0] as f32, p[1] as f32, p[2] as f32]))
                    .collect(),
            ),
            false,
        ),
        (
            "curveVertexCounts",
            "int[]",
            sdf::Value::IntVec(vec![count]),
            false,
        ),
        ("type", "token", sdf::Value::token("linear"), true),
        ("wrap", "token", sdf::Value::token("nonperiodic"), true),
    ] {
        attribute(data, p, name, ty, Opinion::Value(v), false)?;
        if uniform {
            let spec = path(&format!("{p}.{name}"))?;
            data.set_field(&spec, "interpolation", sdf::Value::token("uniform"));
        }
    }
    attr(data, p, "purpose", "token", sdf::Value::token("guide"))?;
    attr(data, p, "bim:exact", "bool", sdf::Value::Bool(false))?;
    Ok(())
}
/// The stored quantity attributes of a mesh prim; a replacement writes them as a whole.
const QUANTITY_FIELDS: [(&str, &str); 7] = [
    ("bim:volumeMm3", "double"),
    ("bim:volumeStatus", "token"),
    ("bim:massStatus", "token"),
    ("bim:shapeSignature", "string"),
    ("bim:massSignature", "string"),
    ("bim:quantityAlgorithm", "string"),
    ("bim:massKg", "double"),
];
/// Drops the quantity attributes of an existing mesh prim and nothing else, so a replacement
/// never keeps a stale record while unknown content under the mesh is preserved.
fn clear_quantities(data: &mut sdf::Data, parent: &str) -> Result<(), Diagnostic> {
    let p = format!("{parent}/Geometry");
    if !data.has_spec(&path(&p)?) {
        return Ok(());
    }
    for (name, ty) in QUANTITY_FIELDS {
        attribute(data, &p, name, ty, Opinion::Remove, true)?;
    }
    Ok(())
}
/// The derived child prims a mesh change owns, beside the properties of `Geometry` itself.
const MESH_CHILDREN: [&str; 4] = ["Geometry", "Reduced", "Proxy", "Axis"];
fn erase_level(data: &mut sdf::Data, parent: &str, name: &str) -> Result<(), Diagnostic> {
    let p = format!("{parent}/{name}");
    let paths = data.spec_paths();
    for path in paths {
        let s = path.to_string();
        if s == p || s.starts_with(&format!("{p}.")) || s.starts_with(&format!("{p}/")) {
            data.erase_spec(&path);
        }
    }
    if data.has_spec(&path(parent)?) {
        child(data, parent, "primChildren", name, true)?;
    }
    Ok(())
}
fn erase_mesh(data: &mut sdf::Data, parent: &str) -> Result<(), Diagnostic> {
    for name in &MESH_CHILDREN {
        erase_level(data, parent, name)?;
    }
    Ok(())
}

/// Authors the complete definition prim of one user attribute (I04). A definition carries no
/// opinion beyond its admitted fields: an absent unit, empty options or an absent set remove
/// their properties, so a redefinition leaves no stale opinion behind.
fn author_attr_define(
    data: &mut sdf::Data,
    def: &beamcraft_model::attribute::AttrDef,
) -> Result<(), Diagnostic> {
    let name = validate::schema_path_name(&def.name)?;
    container(data, "/World", "Xform", Some("assembly"))?;
    container(data, "/World/Definitions", "Scope", None)?;
    container(data, super::ATTR_SCHEMA_PATH, "Scope", None)?;
    let p = format!("{}/{name}", super::ATTR_SCHEMA_PATH);
    prim(data, &p, Some("Scope"), true)?;
    attr(
        data,
        &p,
        "bim:attr:type",
        "token",
        sdf::Value::token(def.ty.as_str()),
    )?;
    let text_opinion = |v: &Option<String>| match v {
        Some(text) => Opinion::Value(sdf::Value::String(text.clone())),
        None => Opinion::Remove,
    };
    attribute(
        data,
        &p,
        "bim:attr:unit",
        "string",
        text_opinion(&def.unit),
        true,
    )?;
    attribute(
        data,
        &p,
        "bim:attr:options",
        "string[]",
        match def.ty {
            beamcraft_model::attribute::AttrType::Choice => {
                Opinion::Value(sdf::Value::StringVec(def.options.clone()))
            }
            _ => Opinion::Remove,
        },
        true,
    )?;
    attribute(
        data,
        &p,
        "bim:attr:set",
        "string",
        text_opinion(&def.set),
        true,
    )?;
    Ok(())
}
fn erase_element(data: &mut sdf::Data, parent: &str) -> Result<(), Diagnostic> {
    let name = parent.rsplit_once('/').map_or(parent, |(_, n)| n);
    for path in data.spec_paths() {
        let s = path.to_string();
        if s == parent
            || s.starts_with(&format!("{parent}."))
            || s.starts_with(&format!("{parent}/"))
        {
            data.erase_spec(&path);
        }
    }
    let folder = parent.rsplit_once('/').map_or("/", |(folder, _)| folder);
    if data.has_spec(&path(folder)?) {
        child(data, folder, "primChildren", name, true)?;
    }
    Ok(())
}

/// Appends one entry to the derived stack's `subLayers` (U15). The stack is a string list,
/// exactly the form the parser and the read profile produce and consume.
fn stack_append(derived: &mut sdf::Data, entry: &str) -> Result<(), Diagnostic> {
    let root = sdf::Path::abs_root();
    let mut names = match derived
        .try_field(&root, "subLayers")
        .map_err(|_| corrupt())?
    {
        Some(value) => match value.as_ref() {
            sdf::Value::StringVec(names) => names.clone(),
            _ => return Err(invalid()),
        },
        None => Vec::new(),
    };
    if !names.iter().any(|n| n == entry) {
        names.push(entry.to_owned());
        field(derived, "/", "subLayers", sdf::Value::StringVec(names))?;
    }
    Ok(())
}

/// Removes one entry from the derived stack's `subLayers` (U15).
fn stack_remove(derived: &mut sdf::Data, entry: &str) -> Result<(), Diagnostic> {
    let root = sdf::Path::abs_root();
    let Some(value) = derived
        .try_field(&root, "subLayers")
        .map_err(|_| corrupt())?
    else {
        return Ok(());
    };
    let sdf::Value::StringVec(names) = value.as_ref() else {
        return Err(invalid());
    };
    let kept: Vec<String> = names
        .iter()
        .filter(|n| n.as_str() != entry)
        .cloned()
        .collect();
    if kept.len() != names.len() {
        field(derived, "/", "subLayers", sdf::Value::StringVec(kept))?;
    }
    Ok(())
}

/// Whether one contributing prim site of a composed result prim is a U11 body-asset
/// contribution: the shared immutable body every instanceable reference composes, never a
/// per-element result location of its own. The body lives in the referenced asset layer (U15)
/// or inline in the derived stack root of a pre-U15 store; the result-location checks of
/// `apply` skip it.
fn asset_contribution(s: &Provenance, targets: &EditTargets) -> bool {
    s.path.starts_with(GEOMETRY_ASSETS)
        && (s.layer == GEOMETRY_ASSET_LAYER || s.layer == targets.derived)
}

/// Loads, or creates on the first mesh write, the per-element derived layer of one element
/// (U15): an owned asset beside the derived stack, listed once in the stack's `subLayers`.
/// A new element file rewrites alone on every later edit of the element, so unchanged elements
/// are never re-serialized into a common layer.
fn element_layer_data<'a>(
    layers: &'a mut BTreeMap<String, sdf::Data>,
    request: &mut OpenRequest,
    derived: &mut sdf::Data,
    id: &str,
    schema: SchemaIdentity,
) -> Result<&'a mut sdf::Data, Diagnostic> {
    if !layers.contains_key(id) {
        let name = element_layer(id)?;
        let loaded = match request.assets.iter().find(|a| a.name == name) {
            Some(asset) => data(asset)?,
            None => {
                let mut fresh = sdf::Data::new();
                units(&mut fresh, Some(schema))?;
                request.assets.push(Asset {
                    name: name.clone(),
                    digest: Sha256::digest(b"").map_err(|_| limit())?,
                    bytes: Vec::new(),
                });
                request.owned_layers.insert(name.clone());
                stack_append(derived, &name)?;
                fresh
            }
        };
        layers.insert(id.to_owned(), loaded);
    }
    layers.get_mut(id).ok_or_else(corrupt)
}

/// Sorts the `subLayers` of the derived stack: the element-layer list is a set, so its byte
/// form never depends on the edit order.
fn sort_sublayers(derived: &mut sdf::Data) -> Result<(), Diagnostic> {
    let Some(value) = derived
        .try_field(&sdf::Path::abs_root(), "subLayers")
        .map_err(|_| corrupt())?
    else {
        return Ok(());
    };
    let sdf::Value::StringVec(names) = value.as_ref() else {
        return Err(invalid());
    };
    let mut sorted = names.clone();
    sorted.sort();
    if sorted != *names {
        field(derived, "/", "subLayers", sdf::Value::StringVec(sorted))?;
    }
    Ok(())
}

pub(super) fn apply(
    original: &OpenRequest,
    targets: &EditTargets,
    input: &Prepare,
    prims: &[Prim],
    control: &Control,
) -> Result<(OpenRequest, Vec<AuthoredItem>), Diagnostic> {
    self::targets(original, targets)?;
    let mut request = original.clone();
    let mut authored = data(
        original
            .assets
            .iter()
            .find(|a| a.name == targets.authored)
            .ok_or_else(corrupt)?,
    )?;
    let mut derived = data(
        original
            .assets
            .iter()
            .find(|a| a.name == targets.derived)
            .ok_or_else(corrupt)?,
    )?;
    let mut root = data(
        original
            .assets
            .iter()
            .find(|a| a.name == original.root)
            .ok_or_else(corrupt)?,
    )?;
    units(&mut root, Some(input.schema))?;
    units(&mut authored, Some(input.schema))?;
    units(&mut derived, Some(input.schema))?;
    // The per-element derived layers (U15) touched by this batch: element id, parsed bytes.
    let mut element_layers: BTreeMap<String, sdf::Data> = BTreeMap::new();
    // The shared U11 geometry assets live in their own referenced layer (U15): loaded when the
    // batch or the base carries it, created with the first body asset.
    let mut geometry_assets = match original
        .assets
        .iter()
        .find(|a| a.name == GEOMETRY_ASSET_LAYER)
    {
        Some(asset) => Some(data(asset)?),
        None => None,
    };
    let mut resources = input.resources.iter().collect::<Vec<_>>();
    resources.sort_by_key(|r| r.digest.to_string());
    let mut seen = BTreeSet::new();
    for r in resources {
        control.check()?;
        if !seen.insert(r.digest) {
            return Err(fail(
                ErrorCode::MalformedCommand,
                "duplicate resource identity",
            ));
        }
        let p = definition_path(r.digest, definition_category(&r.definition));
        validate::owned(prims, &p, true)?;
        if prims.iter().any(|v| v.path == p) {
            continue;
        } // Existing immutable bytes are never rewritten.
        container(&mut authored, "/World", "Xform", Some("assembly"))?;
        container(&mut authored, "/World/Definitions", "Scope", None)?;
        let folder = p
            .rsplit_once('/')
            .map(|(parent, _)| parent)
            .ok_or_else(corrupt)?;
        container(&mut authored, folder, "Scope", None)?;
        definition(&mut authored, &p, &r.definition)?;
        attr(
            &mut authored,
            &p,
            "bim:contentDigest",
            "string",
            sdf::Value::String(r.digest.to_string()),
        )?;
    }
    // Lifecycle first, then all scalar/relationship opinions, then engine results.
    for c in &input.changes {
        if let Change::Create(e) = c {
            control.check()?;
            create(&mut authored, e, &input.actor, input, prims)?;
        }
    }
    for c in &input.changes {
        if let Change::AttachReference(r) = c {
            control.check()?;
            reference::check_source_asset(original, targets, &r.source)?;
            reference::attach(&mut authored, r, &input.actor)?;
        }
    }
    for c in &input.changes {
        control.check()?;
        match c {
            // The schema scope is not an element: its changes resolve no element path and pass
            // no element kind checks; the candidate snapshot re-validates the scope as a whole.
            Change::AttrDefine { def } => {
                author_attr_define(&mut authored, def)?;
                continue;
            }
            Change::AttrUndefine { name } => {
                erase_element(
                    &mut authored,
                    &format!("{}/{name}", super::ATTR_SCHEMA_PATH),
                )?;
                continue;
            }
            _ => {}
        }
        let (id, _) = c.key();
        let p = change_path(id, input, prims)?;
        match c {
            Change::Grid { grid: g, .. } => grid::author(&mut authored, &p, g)?,
            Change::PartClass { class, .. } => {
                attr(
                    &mut authored,
                    &p,
                    "steel:class",
                    "int",
                    sdf::Value::Int(*class),
                )?;
                appearance::author_element(&mut authored, &p, *class)?;
            }
            Change::Label { name, .. } => {
                attr(
                    &mut authored,
                    &p,
                    "bim:name",
                    "string",
                    sdf::Value::String(name.clone()),
                )?;
                field(
                    &mut authored,
                    &p,
                    "displayName",
                    sdf::Value::String(name.clone()),
                )?;
            }
            Change::Discipline(hook) => hook.0.author(&mut hook::ChangeAuthorCx {
                data: &mut authored,
                path: &p,
                input,
                prims,
            })?,
            Change::Material { opinion, .. } => {
                relationship(&mut authored, &p, "bim:material", opinion.clone())?
            }
            Change::Placement { placement, .. } => pose(&mut authored, &p, placement)?,
            Change::PartOf { assembly, .. } => {
                // A detach authors an explicit empty target list, which also masks a weaker
                // layer's record; removing the spec could reveal it again.
                let targets = match assembly.as_deref() {
                    Some(a) => vec![path(&target_path(
                        a,
                        ElementCategory::Assembly,
                        input,
                        prims,
                    )?)?],
                    None => Vec::new(),
                };
                relationship_targets(&mut authored, &p, "bim:partOf", Some(targets))?;
            }
            Change::MainPart { part, .. } => {
                let targets = vec![path(&target_path(
                    part,
                    ElementCategory::Part,
                    input,
                    prims,
                )?)?];
                relationship_targets(&mut authored, &p, "bim:mainPart", Some(targets))?;
            }
            Change::Level { level, .. } => {
                // An unassignment authors an explicit empty target list, as a detach does.
                let targets = match level.as_deref() {
                    Some(l) => vec![path(&target_path(
                        l,
                        ElementCategory::Level,
                        input,
                        prims,
                    )?)?],
                    None => Vec::new(),
                };
                relationship_targets(&mut authored, &p, "bim:level", Some(targets))?;
            }
            Change::Follow { binding, .. } => match binding {
                FollowBinding::Bind(record) => {
                    // While bound the derived world geometry (local points + current frame)
                    // is authoritative; the stored world axis stays as the bind-time frozen
                    // snapshot the member storage schema requires, and the release overwrites
                    // it with the current world.
                    let targets = vec![path(&target_path(
                        &record.grid,
                        ElementCategory::Grid,
                        input,
                        prims,
                    )?)?];
                    relationship_targets(&mut authored, &p, "bim:followsGrid", Some(targets))?;
                    attr(
                        &mut authored,
                        &p,
                        "steel:grid:localAxisMm",
                        "double3[]",
                        points(&record.local_axis_mm),
                    )?;
                }
                FollowBinding::Release(world) => {
                    // An explicit empty target list masks a weaker layer's record, as a
                    // part-of detach does; the world axis points are restored as a whole.
                    relationship_targets(&mut authored, &p, "bim:followsGrid", Some(Vec::new()))?;
                    attribute(
                        &mut authored,
                        &p,
                        "steel:grid:localAxisMm",
                        "double3[]",
                        Opinion::Remove,
                        true,
                    )?;
                    attr(
                        &mut authored,
                        &p,
                        "bim:axisPointsMm",
                        "double3[]",
                        points(world),
                    )?;
                }
            },
            Change::Mark { mark, .. } => {
                let number = i32::try_from(mark.number).map_err(|_| invalid())?;
                for (name, ty, value) in [
                    (
                        MARK_SERIES,
                        "string",
                        sdf::Value::String(mark.series.clone()),
                    ),
                    (MARK_NUMBER, "int", sdf::Value::Int(number)),
                    (MARK_BASIS, "string", sdf::Value::String(mark.basis.clone())),
                ] {
                    attr(&mut authored, &p, name, ty, value)?;
                }
            }
            Change::AttrSet { name, value, .. } => {
                let property = format!("bim:attr:{}", validate::schema_path_name(name)?);
                match value {
                    Some(v) => {
                        let (ty, stored) = match v {
                            beamcraft_model::attribute::AttrValue::Text(t) => {
                                ("string", sdf::Value::String(t.clone()))
                            }
                            beamcraft_model::attribute::AttrValue::Choice(t) => {
                                ("string", sdf::Value::String(t.clone()))
                            }
                            beamcraft_model::attribute::AttrValue::Number(n) => {
                                ("double", sdf::Value::Double(*n))
                            }
                            beamcraft_model::attribute::AttrValue::Integer(i) => {
                                ("int", sdf::Value::Int(*i))
                            }
                            beamcraft_model::attribute::AttrValue::Boolean(b) => {
                                ("bool", sdf::Value::Bool(*b))
                            }
                        };
                        attr(&mut authored, &p, &property, ty, stored)?;
                    }
                    // A clear erases the property alone; the rest of the prim is untouched.
                    None => attribute(
                        &mut authored,
                        &p,
                        &property,
                        "string",
                        Opinion::Remove,
                        true,
                    )?,
                }
            }
            Change::ReferenceUnits { interpretation, .. } => {
                let wrapper = prims
                    .iter()
                    .find(|v| v.path == p)
                    .ok_or_else(|| fail(ErrorCode::NotFound, "element not found"))?;
                reference::units(
                    &mut authored,
                    id,
                    interpretation,
                    reference::recorded_digest(wrapper)?,
                )?;
            }
            Change::Delete { .. } => {
                let inside = format!("{p}/");
                let layer_name = element_layer(id)?;
                // The element and its results must live in the authored layer and in exactly
                // one owned derived location: its own U15 layer, or the shared derived layer
                // of a pre-U15 store.
                let mut location: Option<bool> = None;
                for v in prims
                    .iter()
                    .filter(|v| v.path == p || v.path.starts_with(&inside))
                {
                    for s in &v.sources {
                        if s.layer == targets.authored {
                            continue;
                        }
                        if asset_contribution(s, targets) {
                            continue;
                        }
                        let shared = s.layer == targets.derived;
                        if !shared && s.layer.as_str() != layer_name {
                            return Err(fail(
                                ErrorCode::DependencyUnavailable,
                                "delete requires the element to live in the authored and derived layers only",
                            ));
                        }
                        match location {
                            None => location = Some(shared),
                            Some(prev) if prev != shared => {
                                return Err(fail(
                                    ErrorCode::DependencyUnavailable,
                                    "delete requires one owned derived layer",
                                ));
                            }
                            _ => {}
                        }
                    }
                }
                match location {
                    Some(true) => erase_element(&mut derived, &p)?,
                    Some(false) => {
                        let element = element_layer_data(
                            &mut element_layers,
                            &mut request,
                            &mut derived,
                            id,
                            input.schema,
                        )?;
                        erase_element(element, &p)?;
                    }
                    None => (),
                }
                erase_element(&mut authored, &p)?;
                // The element's derived file goes away with the element (U15).
                if request.assets.iter().any(|a| a.name == layer_name) {
                    request.assets.retain(|a| a.name != layer_name);
                    request.owned_layers.remove(layer_name.as_str());
                    stack_remove(&mut derived, &layer_name)?;
                }
            }
            _ => (),
        }
        if matches!(c, Change::Discipline(hook) if hook.0.invalidates_derived()) {
            // Never reveal a weaker stale result after removing the derived opinion. The
            // result prims must live in exactly one owned location: the element's own U15
            // layer or the shared derived layer of a pre-U15 store. The composed `/Geometry`
            // also cites the shared U11 body asset from the derived stack root; that
            // contribution is bookkeeping of the immutable body, not a result location.
            let mesh_path = format!("{p}/Geometry");
            let layer_name = element_layer(id)?;
            let mut location: Option<bool> = None;
            for v in prims.iter().filter(|v| v.path == mesh_path) {
                for s in &v.sources {
                    if asset_contribution(s, targets) {
                        continue;
                    }
                    let shared = s.layer == targets.derived;
                    if !shared && s.layer.as_str() != layer_name {
                        return Err(fail(
                            ErrorCode::DependencyUnavailable,
                            "derived invalidation requires exclusive result layer",
                        ));
                    }
                    match location {
                        None => location = Some(shared),
                        Some(prev) if prev != shared => {
                            return Err(fail(
                                ErrorCode::DependencyUnavailable,
                                "derived invalidation requires one result layer",
                            ));
                        }
                        _ => {}
                    }
                }
            }
            match location {
                Some(true) => erase_mesh(&mut derived, &p)?,
                Some(false) => {
                    let element = element_layer_data(
                        &mut element_layers,
                        &mut request,
                        &mut derived,
                        id,
                        input.schema,
                    )?;
                    erase_mesh(element, &p)?;
                }
                None => (),
            }
        }
    }
    for c in &input.changes {
        if let Change::Mesh(m) = c {
            control.check()?;
            let parent = change_path(&m.id, input, prims)?;
            let mesh_path = format!("{parent}/Geometry");
            let layer_name = element_layer(&m.id)?;
            // The result prims must live in exactly one owned location: the element's own U15
            // layer, or the shared derived layer of a pre-U15 store, which this write empties.
            // The composed `/Geometry` also cites the shared U11 body asset; see
            // `asset_contribution`.
            let mut legacy = false;
            for v in prims
                .iter()
                .filter(|v| v.path == mesh_path || v.path.starts_with(&format!("{mesh_path}/")))
            {
                for s in &v.sources {
                    if asset_contribution(s, targets) {
                        continue;
                    }
                    let shared = s.layer == targets.derived;
                    if !shared && s.layer.as_str() != layer_name {
                        return Err(fail(
                            ErrorCode::DependencyUnavailable,
                            "mesh writes require exclusive derived layer",
                        ));
                    }
                    match legacy {
                        false if shared => legacy = true,
                        true if !shared => {
                            return Err(fail(
                                ErrorCode::DependencyUnavailable,
                                "mesh writes require one result layer",
                            ));
                        }
                        _ => {}
                    }
                }
            }
            let element = element_layer_data(
                &mut element_layers,
                &mut request,
                &mut derived,
                &m.id,
                input.schema,
            )?;
            // The body asset composes from the referenced asset layer (U15): created with the
            // first asset this batch writes, owned, and sublayered from the derived stack so
            // the asset prims compose at their own paths.
            let created_layer = geometry_assets.is_none();
            if created_layer {
                let mut fresh = sdf::Data::new();
                units(&mut fresh, Some(input.schema))?;
                request.assets.push(Asset {
                    name: GEOMETRY_ASSET_LAYER.to_owned(),
                    digest: Sha256::digest(b"").map_err(|_| limit())?,
                    bytes: Vec::new(),
                });
                request.owned_layers.insert(GEOMETRY_ASSET_LAYER.to_owned());
                geometry_assets = Some(fresh);
                stack_append(&mut derived, GEOMETRY_ASSET_LAYER)?;
            }
            let assets_data = geometry_assets.as_mut().ok_or_else(corrupt)?;
            if legacy {
                erase_mesh(&mut derived, &parent)?;
            }
            // A mesh change replaces the supported fields and the quantity record as a whole:
            // stale quantity attributes never survive it, unknown content stays. The U12 level
            // set is replaced as a whole too: a level the new body does not carry is erased,
            // never left stale beside the fresh full mesh.
            clear_quantities(element, &parent)?;
            for (carried, name) in [
                (m.reduced.is_some(), "Reduced"),
                (m.proxy.is_some(), "Proxy"),
                (m.axis.is_some(), "Axis"),
            ] {
                if !carried {
                    erase_level(element, &parent, name)?;
                }
            }
            // The U12 purpose of the full body: a grid's only geometry is its display guide
            // (T01.7); an element body is the exact render level.
            let purpose = if grid::guide_target(input, prims, &m.id) {
                "guide"
            } else {
                "render"
            };
            mesh(assets_data, element, m, &parent, purpose)?;
        }
    }
    // The element-layer stack is a canonical set: sorted, whatever the edit order.
    sort_sublayers(&mut derived)?;
    let opinions = capture(input, prims, &authored, &element_layers, original.budgets)?;
    for (name, data) in [
        (&original.root, &root),
        (&targets.authored, &authored),
        (&targets.derived, &derived),
    ] {
        let a = request
            .assets
            .iter_mut()
            .find(|a| a.name == *name)
            .ok_or_else(corrupt)?;
        a.bytes = serialize(data, request.budgets.layer_bytes)?;
        a.digest = Sha256::digest(&a.bytes).map_err(|_| limit())?;
    }
    for (id, element) in &element_layers {
        let name = element_layer(id)?;
        // An element deleted in this batch is gone from the candidate: its parsed bytes only
        // served the removal opinions above.
        let Some(a) = request.assets.iter_mut().find(|a| a.name == name) else {
            continue;
        };
        a.bytes = serialize(element, request.budgets.layer_bytes)?;
        a.digest = Sha256::digest(&a.bytes).map_err(|_| limit())?;
    }
    if let Some(assets_data) = &geometry_assets {
        let a = request
            .assets
            .iter_mut()
            .find(|a| a.name == GEOMETRY_ASSET_LAYER)
            .ok_or_else(corrupt)?;
        a.bytes = serialize(assets_data, request.budgets.layer_bytes)?;
        a.digest = Sha256::digest(&a.bytes).map_err(|_| limit())?;
    }
    Ok((request, opinions))
}

fn capture(
    input: &Prepare,
    prims: &[Prim],
    authored: &sdf::Data,
    element_layers: &BTreeMap<String, sdf::Data>,
    b: Budgets,
) -> Result<Vec<AuthoredItem>, Diagnostic> {
    const POSE: &[(&str, &str)] = &[
        ("xformOp:translate", "double3"),
        ("xformOp:orient", "quatd"),
        ("xformOpOrder", "token[]"),
    ];
    // The standard mesh-buffer attributes of a U12 representation level, in storage order.
    // The full body's buffers are shared-asset content (U11): only a level prim carries them
    // as its own authored fields.
    const MESH: &[(&str, &str)] = &[
        ("points", "point3f[]"),
        ("faceVertexCounts", "int[]"),
        ("faceVertexIndices", "int[]"),
        ("orientation", "token"),
        ("subdivisionScheme", "token"),
        ("extent", "float3[]"),
        ("normals", "normal3f[]"),
        ("bim:shapeKey", "string"),
    ];
    // Place of an attribute: 0 the element, 1 the derived mesh, 2 the wrapper's units level.
    let mut fields = BTreeMap::new();
    for change in &input.changes {
        let (id, _) = change.key();
        let mut add = |name: &'static str, ty: &'static str, place: u8| {
            fields.insert((id.to_owned(), name, place), ty);
        };
        match change {
            Change::Create(e) => {
                for (name, ty) in [
                    ("bim:id", "string"),
                    ("bim:name", "string"),
                    ("steel:class", "int"),
                    ("primvars:displayColor", "color3f[]"),
                    ("material:binding", "rel"),
                    ("bim:elementKind", "token"),
                    ("bim:geometryMode", "token"),
                    ("bim:schemaVersion", "string"),
                    ("bim:createdBy", "string"),
                    ("bim:material", "rel"),
                ] {
                    add(name, ty, 0);
                }
                for &(name, ty) in POSE {
                    add(name, ty, 0);
                }
                match e.shape {
                    Shape::Assembly => (),
                    Shape::Discipline(ref hook) => {
                        for (name, ty) in hook.0.attributes() {
                            add(name, ty, 0);
                        }
                    }
                    Shape::Level { .. } => add("bim:elevationMm", "double", 0),
                    Shape::Grid(_) => {
                        for &(name, ty) in grid::FIELDS {
                            add(name, ty, 0);
                        }
                    }
                    Shape::Construction(_) => {
                        for &(name, ty) in construction::FIELDS {
                            add(name, ty, 0);
                        }
                    }
                    Shape::Annotation(_) => {
                        for &(name, ty) in annotation::FIELDS {
                            add(name, ty, 0);
                        }
                    }
                }
            }
            Change::Grid { .. } => {
                for &(name, ty) in grid::FIELDS {
                    add(name, ty, 0);
                }
            }
            Change::AttachReference(r) => {
                for &(name, ty) in reference::ATTACH_FIELDS {
                    let absent = (name == "bim:reference:declaredMetersPerUnit"
                        && r.source.declared_meters_per_unit.is_none())
                        || (name == "bim:reference:declaredUpAxis"
                            && r.source.declared_up_axis.is_none());
                    if !absent {
                        add(name, ty, 0);
                    }
                }
                for &(name, ty) in POSE {
                    add(name, ty, 0);
                }
            }
            Change::ReferenceUnits { .. } => {
                for &(name, ty) in reference::UNITS_FIELDS {
                    add(name, ty, 2);
                }
            }
            Change::PartClass { .. } => {
                add("steel:class", "int", 0);
                add("primvars:displayColor", "color3f[]", 0);
                add("material:binding", "rel", 0);
            }
            Change::Label { .. } => add("bim:name", "string", 0),
            Change::Discipline(hook) => {
                for (name, ty) in hook.0.attributes() {
                    add(name, ty, 0);
                }
            }
            Change::Delete { .. } => {
                for (name, ty) in [
                    ("bim:id", "string"),
                    ("bim:name", "string"),
                    ("steel:class", "int"),
                    ("primvars:displayColor", "color3f[]"),
                    ("material:binding", "rel"),
                    ("bim:elementKind", "token"),
                    ("bim:geometryMode", "token"),
                    ("bim:schemaVersion", "string"),
                    ("bim:createdBy", "string"),
                    ("bim:material", "rel"),
                    ("bim:connects", "rel"),
                    ("bim:partOf", "rel"),
                    (instance::RELATIONSHIP, "rel"),
                    ("bim:mainPart", "rel"),
                    ("bim:level", "rel"),
                    ("bim:elevationMm", "double"),
                    (MARK_SERIES, "string"),
                    (MARK_NUMBER, "int"),
                    (MARK_BASIS, "string"),
                ] {
                    add(name, ty, 0);
                }
                // The attribute names of the registered disciplines: a deleted part or
                // connection reports their removal opinions too.
                for discipline in disciplines::registered() {
                    for &(name, ty) in discipline.delete_attributes() {
                        add(name, ty, 0);
                    }
                }
                for &(name, ty) in POSE {
                    add(name, ty, 0);
                }
                for (name, ty) in QUANTITY_FIELDS {
                    add(name, ty, 1);
                }
            }
            Change::Material { .. } => add("bim:material", "rel", 0),
            Change::PartOf { .. } => add("bim:partOf", "rel", 0),
            Change::MainPart { .. } => add("bim:mainPart", "rel", 0),
            Change::Level { .. } => add("bim:level", "rel", 0),
            Change::Follow { .. } => {
                add("bim:axisPointsMm", "double3[]", 0);
                add("bim:followsGrid", "rel", 0);
                add("steel:grid:localAxisMm", "double3[]", 0);
            }
            Change::Mark { .. } => {
                add(MARK_SERIES, "string", 0);
                add(MARK_NUMBER, "int", 0);
                add(MARK_BASIS, "string", 0);
            }
            Change::Placement { .. } => {
                for &(name, ty) in POSE {
                    add(name, ty, 0);
                }
            }
            Change::Mesh(m) => {
                // The wrapper authors its U12 purpose opinion; the mesh buffers themselves are
                // shared-asset content (U11), never an element's own opinion.
                add("purpose", "token", 1);
                // Every quantity field is reported: absent ones as removals.
                for (name, ty) in QUANTITY_FIELDS {
                    add(name, ty, 1);
                }
                // The U12 level prims report the fields they author, and a level the change
                // erases from the pre-state reports its removals: the level set is replaced
                // as a whole, never partially.
                let parent = change_path(&m.id, input, prims)?;
                for (carried, name, place) in [
                    (m.reduced.is_some(), "Reduced", 3u8),
                    (m.proxy.is_some(), "Proxy", 4),
                ] {
                    let stale = prims.iter().any(|p| p.path == format!("{parent}/{name}"));
                    if carried || stale {
                        add("purpose", "token", place);
                        add("bim:exact", "bool", place);
                        for &(name, ty) in MESH {
                            add(name, ty, place);
                        }
                    }
                }
                if m.axis.is_some() || prims.iter().any(|p| p.path == format!("{parent}/Axis")) {
                    for (name, ty) in [
                        ("points", "point3f[]"),
                        ("curveVertexCounts", "int[]"),
                        ("type", "token"),
                        ("wrap", "token"),
                        ("purpose", "token"),
                        ("bim:exact", "bool"),
                    ] {
                        add(name, ty, 5);
                    }
                }
            }
            // User attributes (I04) are dynamic-named values outside the fixed supported
            // mapping: they contribute no opinion inventory yet, while their footprint writes
            // and the candidate bytes carry them as usual.
            Change::AttrDefine { .. } | Change::AttrUndefine { .. } | Change::AttrSet { .. } => {}
        }
        if fields.len() > b.values {
            return Err(limit());
        }
    }
    let mut result = Vec::new();
    for ((id, name, place), ty) in fields {
        let p = change_path(&id, input, prims)?;
        // Place of a property: 0 the element, 2 the wrapper's units level, the rest the derived
        // bodies of the element (1 the full mesh, 3 reduced, 4 proxy, 5 the axis guide).
        let (p, derived_data) = match place {
            1 => (format!("{p}/Geometry"), true),
            3 => (format!("{p}/Reduced"), true),
            4 => (format!("{p}/Proxy"), true),
            5 => (format!("{p}/Axis"), true),
            2 => (format!("{p}/{}", reference::CORRECTION), false),
            _ => (p, false),
        };
        let spec = path(&format!("{p}.{name}"))?;
        // The derived result places read the element's own layer; an element with no such layer
        // in the candidate has every result field removed.
        let opinion = match derived_data {
            true => match element_layers.get(&id) {
                None => Opinion::Remove,
                Some(data) => field_opinion(data, &spec, ty, b, name)?,
            },
            false => field_opinion(authored, &spec, ty, b, name)?,
        };
        result.push(AuthoredItem {
            element_id: id,
            attribute: name,
            type_name: ty,
            opinion,
        });
    }
    Ok(result)
}

/// The authored opinion of one spec in one candidate layer: Remove when the spec is gone,
/// the typed value (or a relationship's explicit targets) otherwise.
fn field_opinion(
    data: &sdf::Data,
    spec: &sdf::Path,
    ty: &str,
    b: Budgets,
    name: &str,
) -> Result<Opinion<Value>, Diagnostic> {
    if !data.has_spec(spec) {
        return Ok(Opinion::Remove);
    }
    if ty == "rel" {
        return match data.try_field(spec, "targetPaths").map_err(|_| corrupt())? {
            None => Ok(Opinion::Absent),
            Some(v) => match v.as_ref() {
                sdf::Value::PathListOp(list) if list.explicit => Ok(Opinion::Value(Value::Array(
                    list.explicit_items
                        .iter()
                        .map(|p| Value::Text(p.to_string()))
                        .collect(),
                ))),
                _ => Err(unsupported()),
            },
        };
    }
    match data.try_field(spec, "default").map_err(|_| corrupt())? {
        None => Ok(Opinion::Absent),
        Some(v) if matches!(v.as_ref(), sdf::Value::ValueBlock) => Ok(Opinion::Block),
        Some(v) => Ok(Opinion::Value(backend::typed_attribute(
            v.as_ref(),
            b,
            name,
        )?)),
    }
}
