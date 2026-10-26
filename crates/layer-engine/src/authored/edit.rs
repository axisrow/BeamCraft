// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

use super::*;
use openusd::{
    sdf::{self, AbstractData},
    usda::TextWriter,
};
use std::io::{self, Write};

pub(super) fn path(text: &str) -> Result<sdf::Path, Diagnostic> {
    sdf::Path::new(text).map_err(|_| corrupt())
}
pub(super) fn data(asset: &Asset) -> Result<sdf::Data, Diagnostic> {
    if !asset.name.ends_with(".usda") {
        return Err(unsupported());
    }
    openusd::usda::parse(std::str::from_utf8(&asset.bytes).map_err(|_| corrupt())?)
        .map_err(|_| corrupt())
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
pub(super) fn attribute(
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
pub(super) fn attr(
    data: &mut sdf::Data,
    p: &str,
    name: &str,
    ty: &str,
    v: sdf::Value,
) -> Result<(), Diagnostic> {
    attribute(data, p, name, ty, Opinion::Value(v), true)
}
fn relationship(
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
fn relationship_targets(
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
fn points(values: &[[f64; 3]]) -> sdf::Value {
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
/// The optional declared designation, standard and section numbers of any profile definition.
fn section_properties(
    data: &mut sdf::Data,
    p: &str,
    section: &SectionProperties,
) -> Result<(), Diagnostic> {
    optional_text(data, p, "steel:profile:designation", &section.designation)?;
    optional_text(data, p, "steel:profile:standard", &section.standard)?;
    optional_number(data, p, "steel:profile:areaMm2", section.area_mm2)?;
    optional_number(
        data,
        p,
        "steel:profile:massPerMeterKg",
        section.mass_per_meter_kg,
    )?;
    optional_number(
        data,
        p,
        "steel:profile:surfacePerMeterM2",
        section.surface_per_meter_m2,
    )
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
        Definition::RectangleProfile {
            width_mm,
            height_mm,
            section,
        } => {
            attr(
                data,
                p,
                "bim:definitionKind",
                "token",
                sdf::Value::token("rectangle"),
            )?;
            attr(
                data,
                p,
                "steel:profile:widthMm",
                "double",
                sdf::Value::Double(*width_mm),
            )?;
            attr(
                data,
                p,
                "steel:profile:heightMm",
                "double",
                sdf::Value::Double(*height_mm),
            )?;
            section_properties(data, p, section)?;
        }
        Definition::SectionProfile { shape, section } => {
            attr(
                data,
                p,
                "bim:definitionKind",
                "token",
                sdf::Value::token(shape.token()),
            )?;
            match shape {
                SectionShape::Parametric(parametric) => {
                    let names = parametric.family().parameters();
                    for (name, value) in names.iter().zip(parametric.values()) {
                        attr(
                            data,
                            p,
                            &format!("steel:profile:{name}Mm"),
                            "double",
                            sdf::Value::Double(*value),
                        )?;
                    }
                }
                SectionShape::Custom(custom) => {
                    let flat = |c: &[[f64; 2]]| c.iter().map(|[x, y]| [*x, *y, 0.0]).collect();
                    let outer: Vec<[f64; 3]> = flat(&custom.outer);
                    attr(
                        data,
                        p,
                        "steel:profile:outerPointsMm",
                        "double3[]",
                        points(&outer),
                    )?;
                    if !custom.cavities.is_empty() {
                        let cavities: Vec<[f64; 3]> =
                            custom.cavities.iter().flat_map(|c| flat(c)).collect();
                        let counts = custom
                            .cavities
                            .iter()
                            .map(|c| i32::try_from(c.len()).map_err(|_| limit()))
                            .collect::<Result<Vec<_>, _>>()?;
                        attr(
                            data,
                            p,
                            "steel:profile:cavityPointsMm",
                            "double3[]",
                            points(&cavities),
                        )?;
                        attr(
                            data,
                            p,
                            "steel:profile:cavityVertexCounts",
                            "int[]",
                            sdf::Value::IntVec(counts),
                        )?;
                    }
                }
            }
            section_properties(data, p, section)?;
        }
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
    }
    let display_name = match value {
        Definition::RectangleProfile { section, .. }
        | Definition::SectionProfile { section, .. } => &section.display_name,
        Definition::Material { display_name, .. } => display_name,
    };
    if let Some(name) = display_name {
        field(data, p, "displayName", sdf::Value::String(name.clone()))?;
    }
    Ok(())
}
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
        _ => "component",
    };
    field(data, &p, "kind", sdf::Value::token(model_kind))?;
    field(data, &p, "displayName", sdf::Value::String(e.name.clone()))?;
    let kind = match e.shape {
        Shape::Assembly => "assembly",
        Shape::Weld(_) | Shape::Bolt(_) => "connection",
        Shape::Level { .. } => "level",
        Shape::Grid(_) => "grid",
        Shape::Plate { .. } => "plate",
        Shape::Member {
            kind: MemberKind::Beam,
            ..
        } => "beam",
        Shape::Member {
            kind: MemberKind::Column,
            ..
        } => "column",
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
    if matches!(e.shape, Shape::Plate { .. } | Shape::Member { .. }) {
        attr(data, &p, "steel:class", "int", sdf::Value::Int(0))?;
    }
    match &e.shape {
        Shape::Plate {
            contour_mm,
            thickness_mm,
            holes,
            hole_records,
            ..
        } => {
            attr(
                data,
                &p,
                "steel:plate:contourPointsMm",
                "double3[]",
                points(contour_mm),
            )?;
            attr(
                data,
                &p,
                "steel:plate:thicknessMm",
                "double",
                sdf::Value::Double(*thickness_mm),
            )?;
            if !holes.is_empty() {
                let flat: Vec<[f64; 3]> = holes.iter().flatten().copied().collect();
                let counts = holes
                    .iter()
                    .map(|h| i32::try_from(h.len()).map_err(|_| limit()))
                    .collect::<Result<Vec<_>, _>>()?;
                attr(
                    data,
                    &p,
                    "steel:plate:holePointsMm",
                    "double3[]",
                    points(&flat),
                )?;
                attr(
                    data,
                    &p,
                    "steel:plate:holeVertexCounts",
                    "int[]",
                    sdf::Value::IntVec(counts),
                )?;
            }
            if !hole_records.is_empty() {
                let mut flat: Vec<[f64; 3]> = Vec::new();
                let mut counts = Vec::new();
                let mut ids = Vec::new();
                let mut ordinals = Vec::new();
                for record in hole_records {
                    flat.extend(record.polygon_mm.iter().map(|[x, y]| [*x, *y, 0.0]));
                    counts.push(i32::try_from(record.polygon_mm.len()).map_err(|_| limit())?);
                    ids.push(record.operation_id.to_string());
                    ordinals.push(i32::try_from(record.ordinal).map_err(|_| limit())?);
                }
                attr(data, &p, "steel:hole:pointsMm", "double3[]", points(&flat))?;
                attr(
                    data,
                    &p,
                    "steel:hole:vertexCounts",
                    "int[]",
                    sdf::Value::IntVec(counts),
                )?;
                attr(
                    data,
                    &p,
                    "steel:hole:operationIds",
                    "string[]",
                    sdf::Value::StringVec(ids),
                )?;
                attr(
                    data,
                    &p,
                    "steel:hole:ordinals",
                    "int[]",
                    sdf::Value::IntVec(ordinals),
                )?;
            }
        }
        Shape::Member {
            axis_mm,
            profile,
            roll_rad,
            ..
        } => {
            attr(data, &p, "bim:axisPointsMm", "double3[]", points(axis_mm))?;
            relationship(data, &p, "bim:profile", MaterialOpinion::Target(*profile))?;
            if let Some(n) = roll_rad {
                attr(
                    data,
                    &p,
                    "steel:member:rollRad",
                    "double",
                    sdf::Value::Double(*n),
                )?;
            }
        }
        Shape::Assembly => (),
        Shape::Weld(w) => {
            attr(
                data,
                &p,
                "steel:connection:type",
                "token",
                sdf::Value::Token("weld".into()),
            )?;
            for (name, value) in [
                ("steel:weld:type", &w.weld_type),
                ("steel:weld:location", &w.location),
                ("steel:weld:side", &w.side),
            ] {
                attr(data, &p, name, "token", sdf::Value::token(value.as_str()))?;
            }
            attr(
                data,
                &p,
                "steel:weld:sizeMm",
                "double",
                sdf::Value::Double(w.size_mm),
            )?;
            if let Some(geometry) = &w.geometry {
                attr(
                    data,
                    &p,
                    "steel:weld:pathMm",
                    "double3[]",
                    points(&geometry.points_mm),
                )?;
                attr(
                    data,
                    &p,
                    "steel:weld:allAround",
                    "token",
                    sdf::Value::token(if geometry.all_around { "true" } else { "false" }),
                )?;
                attr(
                    data,
                    &p,
                    "steel:weld:contour",
                    "token",
                    sdf::Value::token(geometry.contour.as_str()),
                )?;
            }
            let targets = w
                .connects
                .iter()
                .map(|id| path(&target_path(id, ElementCategory::Part, input, prims)?))
                .collect::<Result<Vec<_>, _>>()?;
            relationship_targets(data, &p, "bim:connects", Some(targets))?;
        }
        Shape::Bolt(b) => {
            attr(
                data,
                &p,
                "steel:connection:type",
                "token",
                sdf::Value::Token("bolt_group".into()),
            )?;
            attr(
                data,
                &p,
                "steel:bolt:diameterMm",
                "double",
                sdf::Value::Double(b.diameter_mm),
            )?;
            // The engine admits 1 to 512 bolts, so the count always fits an int.
            attr(
                data,
                &p,
                "steel:bolt:count",
                "int",
                sdf::Value::Int(i32::try_from(b.count).map_err(|_| limit())?),
            )?;
            attr(
                data,
                &p,
                "steel:bolt:pitchMm",
                "double",
                sdf::Value::Double(b.pitch_mm),
            )?;
            let [ox, oy, oz] = b.origin_mm;
            attr(
                data,
                &p,
                "steel:bolt:originMm",
                "double3",
                sdf::Value::vec3d(ox, oy, oz),
            )?;
            let [dx, dy, dz] = b.direction_mm;
            attr(
                data,
                &p,
                "steel:bolt:directionMm",
                "double3",
                sdf::Value::vec3d(dx, dy, dz),
            )?;
            let targets = b
                .connects
                .iter()
                .map(|id| path(&target_path(id, ElementCategory::Part, input, prims)?))
                .collect::<Result<Vec<_>, _>>()?;
            relationship_targets(data, &p, "bim:connects", Some(targets))?;
        }
        Shape::Grid(g) => grid::author(data, &p, g)?,
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
    // parallel attributes in ascending ordinal order.
    if !e.shape.cuts().is_empty() {
        author_cuts(data, &p, e.shape.cuts())?;
    }
    pose(data, &p, &e.placement)
}

/// Writes the five parallel cut-record attributes of one element (A03). An empty list removes
/// the attributes instead, so a "no records" element authors nothing about cuts.
pub(super) fn author_cuts(
    data: &mut sdf::Data,
    p: &str,
    cuts: &[CutRecord],
) -> Result<(), Diagnostic> {
    if cuts.is_empty() {
        for name in [
            "steel:cut:operationId",
            "steel:cut:ordinal",
            "steel:cut:pointMm",
            "steel:cut:normal",
            "steel:cut:keep",
        ] {
            let full = path(&format!("{p}.{name}"))?;
            if data.has_spec(&full) {
                data.erase_spec(&full);
            }
        }
        return Ok(());
    }
    let ids: Vec<String> = cuts.iter().map(|c| c.operation_id.to_string()).collect();
    let ordinals = cuts.iter().map(|c| i32::from(c.ordinal)).collect();
    let cut_points: Vec<[f64; 3]> = cuts.iter().map(|c| c.point_mm).collect();
    let normals: Vec<[f64; 3]> = cuts.iter().map(|c| c.normal).collect();
    let keeps = cuts.iter().map(|c| c.keep.as_str()).collect::<Vec<_>>();
    for (name, ty, v) in [
        (
            "steel:cut:operationId",
            "string[]",
            sdf::Value::StringVec(ids),
        ),
        ("steel:cut:ordinal", "int[]", sdf::Value::IntVec(ordinals)),
        ("steel:cut:pointMm", "double3[]", points(&cut_points)),
        ("steel:cut:normal", "double3[]", points(&normals)),
        ("steel:cut:keep", "token[]", sdf::Value::token_vec(keeps)),
    ] {
        attr(data, p, name, ty, v)?;
    }
    Ok(())
}
fn mesh(data: &mut sdf::Data, m: &Mesh, parent: &str) -> Result<(), Diagnostic> {
    let p = format!("{parent}/Geometry");
    prim(data, &p, Some("Mesh"), true)?;
    field(data, &p, "kind", sdf::Value::token("subcomponent"))?;
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
        attribute(data, &p, name, ty, Opinion::Value(v), false)?;
    }
    let normals_spec = path(&format!("{p}.normals"))?;
    data.set_field(&normals_spec, "interpolation", sdf::Value::token("uniform"));
    attr(
        data,
        &p,
        "bim:shapeKey",
        "string",
        sdf::Value::String(m.shape_key.to_string()),
    )?;
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
fn erase_mesh(data: &mut sdf::Data, parent: &str) -> Result<(), Diagnostic> {
    let p = format!("{parent}/Geometry");
    let paths = data.spec_paths();
    for path in paths {
        let s = path.to_string();
        if s == p || s.starts_with(&format!("{p}.")) || s.starts_with(&format!("{p}/")) {
            data.erase_spec(&path);
        }
    }
    if data.has_spec(&path(parent)?) {
        child(data, parent, "primChildren", "Geometry", true)?;
    }
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
        let (id, _) = c.key();
        let p = change_path(id, input, prims)?;
        match c {
            Change::Grid { grid: g, .. } => grid::author(&mut authored, &p, g)?,
            Change::PartClass { class, .. } => attr(
                &mut authored,
                &p,
                "steel:class",
                "int",
                sdf::Value::Int(*class),
            )?,
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
            Change::Contour { points_mm, .. } => attr(
                &mut authored,
                &p,
                "steel:plate:contourPointsMm",
                "double3[]",
                points(points_mm),
            )?,
            Change::Thickness { mm, .. } => attr(
                &mut authored,
                &p,
                "steel:plate:thicknessMm",
                "double",
                sdf::Value::Double(*mm),
            )?,
            Change::Cuts { cuts, .. } => author_cuts(&mut authored, &p, cuts)?,
            Change::HoleRecords { records, .. } => {
                // The four record attributes, always together: the authoritative polygons of
                // every record, replayed from the base shape in ordinal order, with the id and
                // the engine-assigned ordinal of each.
                let mut flat = Vec::new();
                let mut counts = Vec::new();
                let mut ids = Vec::new();
                let mut ordinals = Vec::new();
                for record in records {
                    flat.extend(record.polygon_mm.iter().map(|[x, y]| [*x, *y, 0.0]));
                    counts.push(i32::try_from(record.polygon_mm.len()).map_err(|_| limit())?);
                    ids.push(record.operation_id.to_string());
                    ordinals.push(i32::try_from(record.ordinal).map_err(|_| limit())?);
                }
                attr(
                    &mut authored,
                    &p,
                    "steel:hole:pointsMm",
                    "double3[]",
                    points(&flat),
                )?;
                attr(
                    &mut authored,
                    &p,
                    "steel:hole:vertexCounts",
                    "int[]",
                    sdf::Value::IntVec(counts),
                )?;
                attr(
                    &mut authored,
                    &p,
                    "steel:hole:operationIds",
                    "string[]",
                    sdf::Value::StringVec(ids),
                )?;
                attr(
                    &mut authored,
                    &p,
                    "steel:hole:ordinals",
                    "int[]",
                    sdf::Value::IntVec(ordinals),
                )?;
            }
            Change::Axis { points_mm, .. } => attr(
                &mut authored,
                &p,
                "bim:axisPointsMm",
                "double3[]",
                points(points_mm),
            )?,
            Change::Roll { opinion, .. } => {
                let opinion = match opinion {
                    Opinion::Value(n) => Opinion::Value(sdf::Value::Double(*n)),
                    Opinion::Absent => Opinion::Absent,
                    Opinion::Block => Opinion::Block,
                    Opinion::Remove => Opinion::Remove,
                };
                attribute(
                    &mut authored,
                    &p,
                    "steel:member:rollRad",
                    "double",
                    opinion,
                    true,
                )?;
            }
            Change::Material { opinion, .. } => {
                relationship(&mut authored, &p, "bim:material", opinion.clone())?
            }
            Change::Profile { digest, .. } => relationship(
                &mut authored,
                &p,
                "bim:profile",
                MaterialOpinion::Target(*digest),
            )?,
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
                if prims
                    .iter()
                    .filter(|v| v.path == p || v.path.starts_with(&inside))
                    .flat_map(|v| &v.sources)
                    .any(|s| s.layer != targets.authored && s.layer != targets.derived)
                {
                    return Err(fail(
                        ErrorCode::DependencyUnavailable,
                        "delete requires the element to live in the authored and derived layers only",
                    ));
                }
                erase_element(&mut authored, &p)?;
                erase_element(&mut derived, &p)?;
            }
            _ => (),
        }
        if matches!(
            c,
            Change::Contour { .. }
                | Change::Thickness { .. }
                | Change::Cuts { .. }
                | Change::Axis { .. }
                | Change::Roll { .. }
                | Change::Profile { .. }
                | Change::HoleRecords { .. }
        ) {
            // Never reveal a weaker stale result after removing the derived opinion.
            let mesh_path = format!("{p}/Geometry");
            if prims
                .iter()
                .filter(|v| v.path == mesh_path)
                .flat_map(|v| &v.sources)
                .any(|s| s.layer != targets.derived)
            {
                return Err(fail(
                    ErrorCode::DependencyUnavailable,
                    "derived invalidation requires exclusive result layer",
                ));
            }
            erase_mesh(&mut derived, &p)?;
        }
    }
    for c in &input.changes {
        if let Change::Mesh(m) = c {
            control.check()?;
            let parent = change_path(&m.id, input, prims)?;
            let mesh_path = format!("{parent}/Geometry");
            if prims
                .iter()
                .filter(|v| v.path == mesh_path || v.path.starts_with(&format!("{mesh_path}/")))
                .flat_map(|v| &v.sources)
                .any(|s| s.layer != targets.derived)
            {
                return Err(fail(
                    ErrorCode::DependencyUnavailable,
                    "mesh writes require exclusive derived layer",
                ));
            }
            // A mesh change replaces the supported fields and the quantity record as a whole:
            // stale quantity attributes never survive it, unknown content stays.
            clear_quantities(&mut derived, &parent)?;
            mesh(&mut derived, m, &parent)?;
        }
    }
    let opinions = capture(input, prims, &authored, &derived, original.budgets)?;
    for (name, data) in [
        (&original.root, root),
        (&targets.authored, authored),
        (&targets.derived, derived),
    ] {
        let a = request
            .assets
            .iter_mut()
            .find(|a| a.name == *name)
            .ok_or_else(corrupt)?;
        a.bytes = serialize(&data, request.budgets.layer_bytes)?;
        a.digest = Sha256::digest(&a.bytes).map_err(|_| limit())?;
    }
    Ok((request, opinions))
}

fn capture(
    input: &Prepare,
    prims: &[Prim],
    authored: &sdf::Data,
    derived: &sdf::Data,
    b: Budgets,
) -> Result<Vec<AuthoredItem>, Diagnostic> {
    const POSE: &[(&str, &str)] = &[
        ("xformOp:translate", "double3"),
        ("xformOp:orient", "quatd"),
        ("xformOpOrder", "token[]"),
    ];
    const WELD: &[(&str, &str)] = &[
        ("steel:connection:type", "token"),
        ("steel:weld:type", "token"),
        ("steel:weld:sizeMm", "double"),
        ("steel:weld:location", "token"),
        ("steel:weld:side", "token"),
        ("bim:connects", "rel"),
    ];
    const BOLT: &[(&str, &str)] = &[
        ("steel:connection:type", "token"),
        ("steel:bolt:diameterMm", "double"),
        ("steel:bolt:count", "int"),
        ("steel:bolt:pitchMm", "double"),
        ("steel:bolt:originMm", "double3"),
        ("steel:bolt:directionMm", "double3"),
        ("bim:connects", "rel"),
    ];
    const CUT_FIELDS: &[(&str, &str)] = &[
        ("steel:cut:operationId", "string[]"),
        ("steel:cut:ordinal", "int[]"),
        ("steel:cut:pointMm", "double3[]"),
        ("steel:cut:normal", "double3[]"),
        ("steel:cut:keep", "token[]"),
    ];
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
                    Shape::Plate {
                        ref holes,
                        ref cuts,
                        ref hole_records,
                        ..
                    } => {
                        add("steel:plate:contourPointsMm", "double3[]", 0);
                        add("steel:plate:thicknessMm", "double", 0);
                        if !holes.is_empty() {
                            add("steel:plate:holePointsMm", "double3[]", 0);
                            add("steel:plate:holeVertexCounts", "int[]", 0);
                        }
                        if !cuts.is_empty() {
                            for &(name, ty) in CUT_FIELDS {
                                add(name, ty, 0);
                            }
                        }
                        if !hole_records.is_empty() {
                            add("steel:hole:pointsMm", "double3[]", 0);
                            add("steel:hole:vertexCounts", "int[]", 0);
                            add("steel:hole:operationIds", "string[]", 0);
                            add("steel:hole:ordinals", "int[]", 0);
                        }
                    }
                    Shape::Member { ref cuts, .. } => {
                        add("bim:axisPointsMm", "double3[]", 0);
                        add("steel:member:rollRad", "double", 0);
                        add("bim:profile", "rel", 0);
                        if !cuts.is_empty() {
                            for &(name, ty) in CUT_FIELDS {
                                add(name, ty, 0);
                            }
                        }
                    }
                    Shape::Assembly => (),
                    Shape::Weld(ref w) => {
                        for &(name, ty) in WELD {
                            add(name, ty, 0);
                        }
                        if w.geometry.is_some() {
                            add("steel:weld:pathMm", "double3[]", 0);
                            add("steel:weld:allAround", "token", 0);
                            add("steel:weld:contour", "token", 0);
                        }
                    }
                    Shape::Bolt(_) => {
                        for &(name, ty) in BOLT {
                            add(name, ty, 0);
                        }
                    }
                    Shape::Level { .. } => add("bim:elevationMm", "double", 0),
                    Shape::Grid(_) => {
                        for &(name, ty) in grid::FIELDS {
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
            Change::PartClass { .. } => add("steel:class", "int", 0),
            Change::Label { .. } => add("bim:name", "string", 0),
            Change::HoleRecords { .. } => {
                add("steel:hole:pointsMm", "double3[]", 0);
                add("steel:hole:vertexCounts", "int[]", 0);
                add("steel:hole:operationIds", "string[]", 0);
                add("steel:hole:ordinals", "int[]", 0);
            }
            Change::Delete { .. } => {
                for (name, ty) in [
                    ("bim:id", "string"),
                    ("bim:name", "string"),
                    ("steel:class", "int"),
                    ("bim:elementKind", "token"),
                    ("bim:geometryMode", "token"),
                    ("bim:schemaVersion", "string"),
                    ("bim:createdBy", "string"),
                    ("bim:material", "rel"),
                    ("steel:plate:contourPointsMm", "double3[]"),
                    ("steel:plate:thicknessMm", "double"),
                    ("steel:plate:holePointsMm", "double3[]"),
                    ("steel:plate:holeVertexCounts", "int[]"),
                    ("steel:cut:operationId", "string[]"),
                    ("steel:cut:ordinal", "int[]"),
                    ("steel:cut:pointMm", "double3[]"),
                    ("steel:cut:normal", "double3[]"),
                    ("steel:cut:keep", "token[]"),
                    ("steel:hole:pointsMm", "double3[]"),
                    ("steel:hole:vertexCounts", "int[]"),
                    ("steel:hole:operationIds", "string[]"),
                    ("steel:hole:ordinals", "int[]"),
                    ("bim:axisPointsMm", "double3[]"),
                    ("steel:member:rollRad", "double"),
                    ("bim:profile", "rel"),
                    ("bim:partOf", "rel"),
                    ("bim:mainPart", "rel"),
                    ("bim:level", "rel"),
                    ("bim:elevationMm", "double"),
                    (MARK_SERIES, "string"),
                    (MARK_NUMBER, "int"),
                    (MARK_BASIS, "string"),
                ] {
                    add(name, ty, 0);
                }
                for &(name, ty) in WELD {
                    add(name, ty, 0);
                }
                for &(name, ty) in BOLT {
                    add(name, ty, 0);
                }
                for &(name, ty) in POSE {
                    add(name, ty, 0);
                }
                for &(name, ty) in MESH {
                    add(name, ty, 1);
                }
                for (name, ty) in QUANTITY_FIELDS {
                    add(name, ty, 1);
                }
            }
            Change::Contour { .. } => add("steel:plate:contourPointsMm", "double3[]", 0),
            Change::Thickness { .. } => add("steel:plate:thicknessMm", "double", 0),
            Change::Cuts { cuts, .. } if !cuts.is_empty() => {
                for &(name, ty) in CUT_FIELDS {
                    add(name, ty, 0);
                }
            }
            Change::Axis { .. } => add("bim:axisPointsMm", "double3[]", 0),
            Change::Roll { .. } => add("steel:member:rollRad", "double", 0),
            Change::Profile { .. } => add("bim:profile", "rel", 0),
            Change::Material { .. } => add("bim:material", "rel", 0),
            Change::PartOf { .. } => add("bim:partOf", "rel", 0),
            Change::MainPart { .. } => add("bim:mainPart", "rel", 0),
            Change::Level { .. } => add("bim:level", "rel", 0),
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
            Change::Mesh(_) => {
                for &(name, ty) in MESH {
                    add(name, ty, 1);
                }
                // Every quantity field is reported: absent ones as removals.
                for (name, ty) in QUANTITY_FIELDS {
                    add(name, ty, 1);
                }
            }
            // An empty replacement removes the attributes, so there is nothing to report.
            Change::Cuts { .. } => (),
        }
        if matches!(
            change,
            Change::Contour { .. }
                | Change::Thickness { .. }
                | Change::Cuts { .. }
                | Change::Axis { .. }
                | Change::Roll { .. }
                | Change::Profile { .. }
        ) {
            for &(name, ty) in MESH {
                add(name, ty, 1);
            }
        }
        if fields.len() > b.values {
            return Err(limit());
        }
    }
    let mut result = Vec::new();
    for ((id, name, place), ty) in fields {
        let p = change_path(&id, input, prims)?;
        let p = match place {
            1 => format!("{p}/Geometry"),
            2 => format!("{p}/{}", reference::CORRECTION),
            _ => p,
        };
        let spec = path(&format!("{p}.{name}"))?;
        let data = if place == 1 { derived } else { authored };
        let opinion = if !data.has_spec(&spec) {
            Opinion::Remove
        } else if ty == "rel" {
            match data
                .try_field(&spec, "targetPaths")
                .map_err(|_| corrupt())?
            {
                None => Opinion::Absent,
                Some(v) => match v.as_ref() {
                    sdf::Value::PathListOp(list) if list.explicit => Opinion::Value(Value::Array(
                        list.explicit_items
                            .iter()
                            .map(|p| Value::Text(p.to_string()))
                            .collect(),
                    )),
                    _ => return Err(unsupported()),
                },
            }
        } else {
            match data.try_field(&spec, "default").map_err(|_| corrupt())? {
                None => Opinion::Absent,
                Some(v) if matches!(v.as_ref(), sdf::Value::ValueBlock) => Opinion::Block,
                Some(v) => Opinion::Value(backend::typed_attribute(v.as_ref(), b, name)?),
            }
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
