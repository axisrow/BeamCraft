// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

use super::*;
use beamcraft_model::section::{
    CustomSection, MAX_CUSTOM_CAVITIES, MAX_CUSTOM_POINTS, ParametricSection, SectionFamily,
};

pub(super) fn schema(pin: SchemaIdentity) -> Result<(), Diagnostic> {
    if pin != schema_identity()? {
        return Err(fail(
            ErrorCode::UnsupportedVersion,
            "authored schema identity mismatch",
        ));
    }
    Ok(())
}
pub(super) fn input_bounds(input: &Prepare, b: Budgets) -> Result<(), Diagnostic> {
    let mut bytes = 0usize;
    for change in &input.changes {
        if !uuid7(change.key().0) {
            return Err(fail(ErrorCode::InvalidId, "invalid full element identity"));
        }
        preflight::charge(&mut bytes, 256, 1024 * 1024)?;
        let mut count = |n: usize, size: usize| -> Result<(), Diagnostic> {
            if n > b.array_items {
                return Err(limit());
            }
            preflight::charge(
                &mut bytes,
                n.checked_mul(size).ok_or_else(limit)?,
                1024 * 1024,
            )
        };
        match change {
            Change::Create(e) => {
                if e.name.len() > 256 {
                    return Err(limit());
                }
                if let Shape::Plate {
                    contour_mm, holes, ..
                } = &e.shape
                {
                    count(contour_mm.len().checked_mul(3).ok_or_else(limit)?, 8)?;
                    let hole_points = holes.iter().map(Vec::len).sum::<usize>();
                    count(hole_points.checked_mul(3).ok_or_else(limit)?, 8)?;
                    count(holes.len(), 4)?;
                }
            }
            Change::Label { name, .. } if name.len() > 256 => return Err(limit()),
            Change::AttachReference(r) if r.name.len() > 256 => return Err(limit()),
            Change::Mark { value: Some(v), .. } if v.len() > 64 => return Err(limit()),
            Change::Contour { points_mm, .. } => {
                count(points_mm.len().checked_mul(3).ok_or_else(limit)?, 8)?
            }
            Change::Mesh(m) => {
                count(m.points_mm.len().checked_mul(3).ok_or_else(limit)?, 4)?;
                count(m.face_counts.len(), 4)?;
                count(m.indices.len(), 4)?;
            }
            _ => (),
        }
    }
    Ok(())
}
pub(super) fn finite(values: impl IntoIterator<Item = f64>) -> Result<(), Diagnostic> {
    if values.into_iter().any(|n| !n.is_finite()) {
        return Err(fail(
            ErrorCode::NonFiniteNumber,
            "non-finite authored value",
        ));
    }
    Ok(())
}
pub(super) fn cap(values: impl IntoIterator<Item = f64>, max: f64) -> Result<(), Diagnostic> {
    if values.into_iter().any(|n| n.abs() > max) {
        return Err(limit());
    }
    Ok(())
}
pub(super) fn label(s: &str) -> Result<(), Diagnostic> {
    if s.len() > 256 {
        return Err(limit());
    }
    if s.is_empty() || s.chars().any(label_char_forbidden) {
        return Err(invalid());
    }
    Ok(())
}
pub(super) fn pose(p: &Placement) -> Result<(), Diagnostic> {
    cap(p.translation_mm, 1e9)?;
    cap(p.orientation_xyzw, 1.0)?;
    let [x, y, z, w] = p.orientation_xyzw;
    let norm = ((x * x + y * y) + z * z) + w * w;
    if (norm - 1.0).abs() > 1e-6 {
        return Err(invalid());
    }
    Ok(())
}
pub(super) fn definition(d: &Definition) -> Result<(), Diagnostic> {
    let refused = |e: beamcraft_model::command::CommandError| {
        fail(
            e.code,
            "definition value refused by the F04 admission rules",
        )
    };
    match d {
        Definition::RectangleProfile {
            width_mm,
            height_mm,
            section,
        } => beamcraft_model::definition::RectangleProfileSpec {
            width_mm: *width_mm,
            height_mm: *height_mm,
            section: section.clone(),
        }
        .validate()
        .map_err(refused),
        Definition::SectionProfile { shape, section } => {
            beamcraft_model::definition::SectionProfileSpec {
                shape: shape.clone(),
                section: section.clone(),
            }
            .validate()
            .map_err(refused)
        }
        Definition::Material {
            density_kg_m3,
            grade,
            standard,
            display_name,
        } => beamcraft_model::definition::MaterialSpec {
            density_kg_m3: *density_kg_m3,
            grade: grade.clone(),
            standard: standard.clone(),
            display_name: display_name.clone(),
        }
        .validate()
        .map_err(refused),
    }
}
/// An optional property of an exact USD type. A property declared with another type, blocked or
/// without a value is invalid; only an absent one is unknown.
pub(super) fn optional<'a>(
    prim: &'a Prim,
    name: &str,
    ty: &str,
) -> Result<Option<&'a Value>, Diagnostic> {
    match prim.properties.iter().find(|p| p.name == name) {
        None => Ok(None),
        Some(p) if p.type_name == ty => match &p.value {
            Some(v) if !matches!(v, Value::Block) => Ok(Some(v)),
            _ => Err(invalid()),
        },
        Some(_) => Err(invalid()),
    }
}
pub(super) fn optional_number(prim: &Prim, name: &str) -> Result<Option<f64>, Diagnostic> {
    match optional(prim, name, "double")? {
        None => Ok(None),
        Some(Value::Number(n)) => Ok(Some(*n)),
        Some(_) => Err(invalid()),
    }
}
pub(super) fn optional_text(prim: &Prim, name: &str) -> Result<Option<String>, Diagnostic> {
    match optional(prim, name, "string")? {
        None => Ok(None),
        Some(Value::Text(s)) => Ok(Some(s.clone())),
        Some(_) => Err(invalid()),
    }
}
/// Every definition kind a member's `bim:profile` may target.
pub(super) const PROFILE_KINDS: [&str; 11] = [
    "rectangle",
    "i_shape",
    "channel",
    "angle",
    "rhs",
    "chs",
    "lipped_c",
    "plain_c",
    "lipped_z",
    "omega",
    "custom",
];
fn section_properties(p: &Prim) -> Result<SectionProperties, Diagnostic> {
    Ok(SectionProperties {
        designation: optional_text(p, "steel:profile:designation")?,
        standard: optional_text(p, "steel:profile:standard")?,
        area_mm2: optional_number(p, "steel:profile:areaMm2")?,
        mass_per_meter_kg: optional_number(p, "steel:profile:massPerMeterKg")?,
        surface_per_meter_m2: optional_number(p, "steel:profile:surfacePerMeterM2")?,
        display_name: p.display_name.clone(),
    })
}
/// The authored outline of a custom profile: the outer contour and the optional cavities (both
/// cavity attributes or neither, counts adding up to the points), all at z = 0 and capped.
fn custom_section(p: &Prim) -> Result<CustomSection, Diagnostic> {
    let flat = |points: Vec<[f64; 3]>| -> Result<Vec<[f64; 2]>, Diagnostic> {
        if points.len() > MAX_CUSTOM_POINTS {
            return Err(limit());
        }
        if points.iter().any(|q| q[2] != 0.0) {
            return Err(invalid());
        }
        Ok(points.into_iter().map(|[x, y, _]| [x, y]).collect())
    };
    let outer = flat(points(p, "steel:profile:outerPointsMm")?)?;
    let present = |name: &str| p.properties.iter().any(|q| q.name == name);
    let (has_points, has_counts) = (
        present("steel:profile:cavityPointsMm"),
        present("steel:profile:cavityVertexCounts"),
    );
    if has_points != has_counts {
        return Err(invalid());
    }
    let mut cavities = Vec::new();
    if has_points {
        let all = flat(points(p, "steel:profile:cavityPointsMm")?)?;
        let Value::Array(counts) = required(p, "steel:profile:cavityVertexCounts", "int[]")? else {
            return Err(invalid());
        };
        if counts.len() > MAX_CUSTOM_CAVITIES {
            return Err(limit());
        }
        let mut at = 0usize;
        for count in counts {
            let n = match count {
                Value::Int(n) => usize::try_from(*n).map_err(|_| invalid())?,
                _ => return Err(invalid()),
            };
            if n < 3 {
                return Err(invalid());
            }
            let end = at.checked_add(n).ok_or_else(invalid)?;
            cavities.push(all.get(at..end).ok_or_else(invalid)?.to_vec());
            at = end;
        }
        if at != all.len() {
            return Err(invalid());
        }
    }
    Ok(CustomSection { outer, cavities })
}
/// The logical definition stored in a definition prim of this kind.
fn read_definition(p: &Prim, kind: &str) -> Result<Definition, Diagnostic> {
    if let Some(family) = SectionFamily::from_token(kind) {
        let values = family
            .parameters()
            .iter()
            .map(|name| number(p, &format!("steel:profile:{name}Mm")))
            .collect::<Result<Vec<_>, _>>()?;
        let parametric = ParametricSection::new(family, values).map_err(|_| invalid())?;
        return Ok(Definition::SectionProfile {
            shape: SectionShape::Parametric(parametric),
            section: section_properties(p)?,
        });
    }
    match kind {
        "rectangle" => Ok(Definition::RectangleProfile {
            width_mm: number(p, "steel:profile:widthMm")?,
            height_mm: number(p, "steel:profile:heightMm")?,
            section: section_properties(p)?,
        }),
        "custom" => Ok(Definition::SectionProfile {
            shape: SectionShape::Custom(custom_section(p)?),
            section: section_properties(p)?,
        }),
        "material" => Ok(Definition::Material {
            density_kg_m3: optional_number(p, "bim:densityKgM3")?,
            grade: optional_text(p, "bim:materialGrade")?,
            standard: optional_text(p, "bim:materialStandard")?,
            display_name: p.display_name.clone(),
        }),
        _ => Err(unsupported()),
    }
}
pub(super) fn owned(prims: &[Prim], path: &str, descendants: bool) -> Result<(), Diagnostic> {
    let prefix = format!("{path}/");
    // Ancestors and descendants matter: defining a child/over cannot bypass foreign provenance.
    for prim in prims.iter().filter(|p| {
        p.path == path
            || path.starts_with(&format!("{}/", p.path))
            || (descendants && p.path.starts_with(&prefix))
    }) {
        if prim.sources.is_empty() || prim.sources.iter().any(|s| !s.owned) {
            return Err(fail(
                ErrorCode::ForeignReadOnly,
                "foreign composition is read-only",
            ));
        }
    }
    Ok(())
}
pub(super) fn required<'a>(prim: &'a Prim, name: &str, ty: &str) -> Result<&'a Value, Diagnostic> {
    prim.properties
        .iter()
        .find(|p| p.name == name && p.type_name == ty)
        .and_then(|p| p.value.as_ref())
        .filter(|v| !matches!(v, Value::Block))
        .ok_or_else(invalid)
}
pub(super) fn text<'a>(prim: &'a Prim, name: &str, ty: &str) -> Result<&'a str, Diagnostic> {
    match required(prim, name, ty)? {
        Value::Text(s) => Ok(s),
        _ => Err(invalid()),
    }
}
pub(super) fn number(prim: &Prim, name: &str) -> Result<f64, Diagnostic> {
    match required(prim, name, "double")? {
        Value::Number(n) => Ok(*n),
        _ => Err(invalid()),
    }
}
pub(super) fn points(prim: &Prim, name: &str) -> Result<Vec<[f64; 3]>, Diagnostic> {
    let Value::Array(values) = required(prim, name, "double3[]")? else {
        return Err(invalid());
    };
    values
        .iter()
        .map(|v| {
            let Value::Array(items) = v else {
                return Err(invalid());
            };
            match items.as_slice() {
                [Value::Number(x), Value::Number(y), Value::Number(z)] => Ok([*x, *y, *z]),
                _ => Err(invalid()),
            }
        })
        .collect()
}
pub(super) fn vec3s(value: &Value) -> Result<Vec<[f32; 3]>, Diagnostic> {
    let Value::Array(values) = value else {
        return Err(invalid());
    };
    values
        .iter()
        .map(|v| {
            let Value::Array(items) = v else {
                return Err(invalid());
            };
            match items.as_slice() {
                [Value::Number(x), Value::Number(y), Value::Number(z)] => {
                    Ok([*x as f32, *y as f32, *z as f32])
                }
                _ => Err(invalid()),
            }
        })
        .collect()
}
fn norm([x, y, z]: [f64; 3]) -> f64 {
    ((x * x + y * y) + z * z).sqrt()
}
fn difference([x, y, z]: [f64; 3], [a, b, c]: [f64; 3]) -> [f64; 3] {
    [x - a, y - b, z - c]
}
fn plate(points: &[[f64; 3]], thickness: f64) -> Result<(), Diagnostic> {
    cap(points.iter().flatten().copied(), 1e9)?;
    cap([thickness], 1e6)?;
    let Some(&first) = points.first() else {
        return Err(invalid());
    };
    let mut size = 0.0;
    for &p in points {
        let n = norm(difference(p, first));
        if n > size {
            size = n;
        }
    }
    if size > 2e6 {
        return Err(limit());
    }
    let eps = 0.001f64.max(size / 4194304.0);
    let mut too_small = points.len() < 3 || thickness < eps;
    for (&p, &q) in points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
    {
        let length = norm(difference(q, p));
        if length > 1e6 {
            return Err(limit());
        }
        if length < eps {
            too_small = true;
        }
    }
    if too_small {
        return Err(invalid());
    }
    // Planarity, exact topology and rectangle support are deliberately the engine's F06 gate.
    Ok(())
}
pub(super) fn relationship<'a>(prim: &'a Prim, name: &str) -> Option<&'a Relationship> {
    prim.relationships.iter().find(|r| r.name == name)
}
/// The optional through-hole polygons of a loaded plate: either both attributes or neither, the
/// counts add up to the points, every hole has at least three points, and the totals are capped.
fn plate_holes(prim: &Prim) -> Result<Vec<Vec<[f64; 3]>>, Diagnostic> {
    let present = |name: &str| prim.properties.iter().any(|p| p.name == name);
    let (has_points, has_counts) = (
        present("steel:plate:holePointsMm"),
        present("steel:plate:holeVertexCounts"),
    );
    if !has_points && !has_counts {
        return Ok(Vec::new());
    }
    if has_points != has_counts {
        return Err(invalid());
    }
    let points = points(prim, "steel:plate:holePointsMm")?;
    let Value::Array(counts) = required(prim, "steel:plate:holeVertexCounts", "int[]")? else {
        return Err(invalid());
    };
    if counts.len() > MAX_HOLES || points.len() > MAX_HOLES * MAX_HOLE_POINTS {
        return Err(limit());
    }
    cap(points.iter().flatten().copied(), 1e9)?;
    let mut holes = Vec::with_capacity(counts.len());
    let mut at = 0usize;
    for count in counts {
        let n = match count {
            Value::Int(n) => usize::try_from(*n).map_err(|_| invalid())?,
            _ => return Err(invalid()),
        };
        if !(3..=MAX_HOLE_POINTS).contains(&n) {
            return Err(invalid());
        }
        let end = at.checked_add(n).ok_or_else(invalid)?;
        holes.push(points.get(at..end).ok_or_else(invalid)?.to_vec());
        at = end;
    }
    if at != points.len() || holes.iter().flatten().any(|p| p[2] != 0.0) {
        return Err(invalid());
    }
    Ok(holes)
}
/// Most through holes of one plate and most points of one hole.
const MAX_HOLES: usize = 64;
const MAX_HOLE_POINTS: usize = 1024;
fn resource<'a>(
    prims: &'a [Prim],
    digest: ContentDigest,
    kinds: &[&str],
) -> Result<&'a Prim, Diagnostic> {
    let path = definition_path(digest);
    let p = prims
        .iter()
        .find(|p| p.path == path && p.active)
        .ok_or_else(|| {
            fail(
                ErrorCode::DanglingRelationship,
                "definition target unavailable",
            )
        })?;
    let kind = text(p, "bim:definitionKind", "token")?;
    if text(p, "bim:contentDigest", "string")? != digest.to_string() || !kinds.contains(&kind) {
        return Err(corrupt());
    }
    if text(p, "bim:schemaVersion", "string")? != "0.1.0" || text(p, "bim:units", "token")? != "mm"
    {
        return Err(corrupt());
    }
    let definition = read_definition(p, kind)?;
    if Resource::new(definition)?.digest != digest {
        return Err(corrupt());
    }
    Ok(p)
}
fn link<'a>(
    p: &Prim,
    prims: &'a [Prim],
    name: &str,
    kinds: &[&str],
    required: bool,
) -> Result<Option<&'a Prim>, Diagnostic> {
    let targets = relationship(p, name)
        .map(|r| r.targets.as_slice())
        .unwrap_or_default();
    match targets {
        [] if !required => Ok(None),
        [target] => {
            let Some(hex) = target.strip_prefix("/World/Definitions/D_") else {
                return Err(invalid());
            };
            let digest = hex.parse::<ContentDigest>().map_err(|_| invalid())?;
            Ok(Some(resource(prims, digest, kinds)?))
        }
        _ => Err(fail(
            ErrorCode::DanglingRelationship,
            "missing or ambiguous definition target",
        )),
    }
}

/// The rigid placement an owned element carries: translation, orientation and the exact
/// `[translate, orient]` operation order. Shared by plates, members and reference wrappers.
pub(super) fn owned_pose(p: &Prim) -> Result<(), Diagnostic> {
    let translation = match required(p, "xformOp:translate", "double3")? {
        Value::Array(v) => match v.as_slice() {
            [Value::Number(x), Value::Number(y), Value::Number(z)] => [*x, *y, *z],
            _ => return Err(invalid()),
        },
        _ => return Err(invalid()),
    };
    let orientation = match required(p, "xformOp:orient", "quatd")? {
        Value::Array(v) => match v.as_slice() {
            [
                Value::Number(w),
                Value::Number(x),
                Value::Number(y),
                Value::Number(z),
            ] => [*x, *y, *z, *w],
            _ => return Err(invalid()),
        },
        _ => return Err(invalid()),
    };
    pose(&Placement {
        translation_mm: translation,
        orientation_xyzw: orientation,
    })?;
    if required(p, "xformOpOrder", "token[]")?
        != &Value::Array(vec![
            Value::Text("xformOp:translate".into()),
            Value::Text("xformOp:orient".into()),
        ])
    {
        return Err(unsupported());
    }
    Ok(())
}

pub(super) fn snapshot(
    prims: &[Prim],
    metadata: &BTreeMap<String, Value>,
) -> Result<Validation, Diagnostic> {
    reference::stray_holders(prims)?;
    for (path, kind) in [("/World", "assembly"), ("/World/Steel", "group")] {
        if let Some(p) = prims
            .iter()
            .find(|p| p.path == path && p.sources.iter().all(|s| s.owned))
            && p.kind.as_deref().is_some_and(|k| k != kind)
        {
            return Err(invalid());
        }
    }
    if let Some(Value::Dictionary(data)) = metadata.get("customLayerData")
        && let Some(pin) = data.get("bimSchemaDigest")
        && !matches!(pin, Value::Text(s) if pin_accepted(s)?)
    {
        return Err(fail(
            ErrorCode::UnsupportedVersion,
            "layer schema digest mismatch",
        ));
    }
    project_record(metadata)?;
    for p in prims.iter().filter(|p| p.element_id.is_some()) {
        let id = p.element_id.as_deref().ok_or_else(invalid)?;
        // Foreign content is preserved inertly, not interpreted as engineering input.
        if p.sources.iter().any(|s| !s.owned) {
            continue;
        }
        // Loaded newer schemas, unknown kinds and geometry modes stay inert/read-only.
        // The mutation guard independently refuses editing them.
        if text(p, "bim:schemaVersion", "string")? != "0.1.0" {
            continue;
        }
        if text(p, "bim:geometryMode", "token")? == "reference" {
            if text(p, "bim:elementKind", "token")? != "reference" {
                return Err(unsupported());
            }
            // A foreign wrapper is no storey member: a level record on it is outside D2.
            if relationship(p, "bim:level").is_some_and(|r| !r.targets.is_empty()) {
                return Err(fail(
                    ErrorCode::InvalidArgument,
                    "only a plate, beam or column is assigned to a level",
                ));
            }
            reference::element(p, prims)?;
            continue;
        }
        if text(p, "bim:geometryMode", "token")? != "parametric" {
            continue;
        }
        if !matches!(
            text(p, "bim:elementKind", "token")?,
            "plate" | "beam" | "column" | "assembly" | "connection" | "level"
        ) {
            continue;
        }
        if p.path != element_path(id)? {
            return Err(fail(
                ErrorCode::InvalidId,
                "invalid canonical owned element path",
            ));
        }
        if !uuid7(text(p, "bim:createdBy", "string")?) {
            return Err(fail(ErrorCode::InvalidId, "invalid actor identity"));
        }
        let name = text(p, "bim:name", "string")?;
        label(name)?;
        let kind = text(p, "bim:elementKind", "token")?;
        let model_kind = if kind == "assembly" {
            "assembly"
        } else {
            "component"
        };
        if p.kind.as_deref() != Some(model_kind) || p.display_name.as_deref() != Some(name) {
            return Err(invalid());
        }
        if p.type_name.as_deref() != Some("Xform") {
            return Err(unsupported());
        }
        match kind {
            "plate" => {
                plate(
                    &points(p, "steel:plate:contourPointsMm")?,
                    number(p, "steel:plate:thicknessMm")?,
                )?;
                plate_holes(p)?;
            }
            "beam" | "column" => {
                let axis = points(p, "bim:axisPointsMm")?;
                let [a, b] = axis.as_slice() else {
                    return Err(invalid());
                };
                let profile =
                    link(p, prims, "bim:profile", &PROFILE_KINDS, true)?.ok_or_else(invalid)?;
                let profile =
                    read_definition(profile, text(profile, "bim:definitionKind", "token")?)?;
                definition(&profile)?;
                let (width, height) = match &profile {
                    Definition::RectangleProfile {
                        width_mm,
                        height_mm,
                        ..
                    } => (*width_mm, *height_mm),
                    Definition::SectionProfile { shape, .. } => shape.extent_mm(),
                    Definition::Material { .. } => return Err(invalid()),
                };
                let length = norm(difference(*b, *a));
                let half_width = width / 2.0;
                let half_height = height / 2.0;
                let size = length.max((half_width * half_width + half_height * half_height).sqrt());
                if size > 1e6 {
                    return Err(limit());
                }
                let eps = 0.001f64.max(size / 4194304.0);
                if length < eps || width < eps || height < eps {
                    return Err(invalid());
                }
                if let Some(Value::Number(n)) = p
                    .properties
                    .iter()
                    .find(|v| v.name == "steel:member:rollRad")
                    .and_then(|v| v.value.as_ref())
                {
                    cap([*n], std::f64::consts::TAU)?;
                }
            }
            // An assembly has no shape inputs and no material of its own; a connection is a weld
            // with its own properties (A06) and no material either.
            "assembly" | "connection" | "level" => {
                if relationship(p, "bim:material").is_some_and(|r| !r.targets.is_empty()) {
                    return Err(invalid());
                }
                if kind == "connection" {
                    weld_properties(p)?;
                }
                if kind == "level" {
                    elevation(number(p, "bim:elevationMm")?)?;
                }
            }
            _ => continue, // Unknown vocabulary remains inert; edits below refuse it.
        }
        if let Some(material) = link(p, prims, "bim:material", &["material"], false)? {
            definition(&read_definition(material, "material")?)?;
        }
        owned_pose(p)?;
        // The placement of an assembly, a weld and a level is the identity in this slice.
        if matches!(kind, "assembly" | "connection" | "level")
            && (required(p, "xformOp:translate", "double3")?
                != &Value::Array(vec![Value::Number(0.0); 3])
                || required(p, "xformOp:orient", "quatd")?
                    != &Value::Array(vec![
                        Value::Number(1.0),
                        Value::Number(0.0),
                        Value::Number(0.0),
                        Value::Number(0.0),
                    ]))
        {
            return Err(unsupported());
        }
    }
    part_of(prims)?;
    connects(prims)?;
    level_links(prims)?;
    marks(prims)?;
    for p in prims
        .iter()
        .filter(|p| p.type_name.as_deref() == Some("Mesh") && p.sources.iter().all(|s| s.owned))
    {
        let Some((parent, "Geometry")) = p.path.rsplit_once('/') else {
            continue;
        };
        let Some(element) = prims
            .iter()
            .find(|p| p.path == parent && p.element_id.is_some())
        else {
            continue;
        };
        // An assembly, a weld or a level has no body: a Geometry mesh under one is refused,
        // authored or loaded.
        if matches!(
            supported_kind(element),
            Some("assembly" | "connection" | "level")
        ) {
            return Err(unsupported());
        }
        let id = element.element_id.clone().ok_or_else(invalid)?;
        let key = text(p, "bim:shapeKey", "string")?
            .parse::<ContentDigest>()
            .map_err(|_| invalid())?;
        let Value::Array(values) = required(p, "points", "point3f[]")? else {
            return Err(invalid());
        };
        let points = values
            .iter()
            .map(|v| {
                let Value::Array(v) = v else {
                    return Err(invalid());
                };
                match v.as_slice() {
                    [Value::Number(x), Value::Number(y), Value::Number(z)] => {
                        Ok([*x as f32, *y as f32, *z as f32])
                    }
                    _ => Err(invalid()),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        let integers = |name| -> Result<Vec<i32>, Diagnostic> {
            let Value::Array(values) = required(p, name, "int[]")? else {
                return Err(invalid());
            };
            values
                .iter()
                .map(|v| match v {
                    Value::Int(n) => i32::try_from(*n).map_err(|_| invalid()),
                    _ => Err(invalid()),
                })
                .collect()
        };
        let result = Mesh {
            id,
            shape_key: key,
            points_mm: points,
            face_counts: integers("faceVertexCounts")?,
            indices: integers("faceVertexIndices")?,
            quantities: None,
        };
        mesh(&result, Budgets::default())?;
        if p.kind.as_deref() != Some("subcomponent") {
            return Err(invalid());
        }
        let expected = derive::extent(&result.points_mm).ok_or_else(invalid)?;
        let stored = vec3s(required(p, "extent", "float3[]")?)?;
        if stored.len() != 2 || stored.iter().zip(expected).any(|(s, e)| *s != e) {
            return Err(invalid());
        }
        let expected =
            derive::face_normals(&result.points_mm, &result.face_counts, &result.indices)
                .ok_or_else(invalid)?;
        let stored = vec3s(required(p, "normals", "normal3f[]")?)?;
        // Per-face data read as per-vertex data (the USD default) would mis-shade every reader.
        let interpolation = p
            .properties
            .iter()
            .find(|v| v.name == "normals")
            .and_then(|v| v.interpolation.as_deref());
        if interpolation != Some("uniform") {
            return Err(invalid());
        }
        if stored.len() != expected.len()
            || stored.iter().zip(&expected).any(|(s, e)| {
                s.iter()
                    .zip(e)
                    .any(|(s, e)| (s - e).abs() > derive::NORMAL_TOLERANCE)
            })
        {
            return Err(invalid());
        }
        if text(p, "subdivisionScheme", "token")? != "none"
            || text(p, "orientation", "token")? != "rightHanded"
        {
            return Err(unsupported());
        }
    }
    Ok(Validation {
        profile: "u02-structural-v1",
        rules: &[
            "U02-V01:U01-integrity-budgets-units-provenance-finite",
            "U02-V02:schema-pin-and-supported-field-types",
            "U02-V03:full-id-path-and-required-inputs",
            "U02-V04:definition-target-kind-and-digest-label",
            "U02-V05:scalar-caps-dimensions-scaled-minima-pose",
            "U02-V06:standard-mesh-buffer-structure",
            "A01-V01:part-of-kinds-cardinality-self-link-and-cycles",
            "A06-V01:weld-properties-and-connects-two-distinct-parts",
            "D2-V01:level-elevation-and-level-kinds-cardinality-and-dangling",
            "A08-V01:mark-value-status-pairing-kinds-and-uniqueness",
        ],
        prims: prims.len(),
    })
}

/// The kind token of an owned, supported 0.1.0 element prim, or `None` for content that stays inert.
fn supported_kind(p: &Prim) -> Option<&str> {
    if p.element_id.is_none() || p.sources.is_empty() || p.sources.iter().any(|s| !s.owned) {
        return None;
    }
    let token = |name: &str| match required(p, name, "token").ok()? {
        Value::Text(s) => Some(s.as_str()),
        _ => None,
    };
    let version = match required(p, "bim:schemaVersion", "string").ok()? {
        Value::Text(s) => s.as_str(),
        _ => return None,
    };
    if version != "0.1.0" || token("bim:geometryMode")? != "parametric" {
        return None;
    }
    token("bim:elementKind").filter(|k| {
        matches!(
            *k,
            "plate" | "beam" | "column" | "assembly" | "connection" | "level"
        )
    })
}

/// The elevation of a loaded or authored level: finite and within the coordinate cap.
fn elevation(mm: f64) -> Result<(), Diagnostic> {
    finite([mm])?;
    cap([mm], 1e9)
}

/// D2 `level` records (F10 relationships, domain vocabulary section 5.1 pattern): only a plate,
/// beam or column carries a `bim:level` record, with at most one target (`DV-REL-004`,
/// `DEPENDENCY_CONFLICT`), which must be an active owned level (`DV-REL-001` dangling,
/// `DV-REL-003` kind). A level itself carries no `bim:level`, `bim:partOf`, `bim:connects` or
/// `bim:profile` record.
fn level_links(prims: &[Prim]) -> Result<(), Diagnostic> {
    let elements: BTreeMap<&str, (&Prim, &str)> = prims
        .iter()
        .filter_map(|p| supported_kind(p).map(|k| (p.path.as_str(), (p, k))))
        .collect();
    for &(p, kind) in elements.values() {
        if kind == "level" {
            for name in ["bim:level", "bim:partOf", "bim:profile"] {
                if relationship(p, name).is_some_and(|r| !r.targets.is_empty()) {
                    return Err(fail(
                        ErrorCode::InvalidArgument,
                        "a level carries no element or definition links",
                    ));
                }
            }
        }
        let Some(r) = relationship(p, "bim:level") else {
            continue;
        };
        let target = match r.targets.as_slice() {
            [] => continue,
            [target] => target.as_str(),
            _ => {
                return Err(fail(
                    ErrorCode::DependencyConflict,
                    "an element is assigned to at most one level",
                ));
            }
        };
        if !matches!(kind, "plate" | "beam" | "column") {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "only a plate, beam or column is assigned to a level",
            ));
        }
        match elements.get(target) {
            Some((t, "level")) if t.active => {}
            Some((t, _)) if !t.active => {
                return Err(fail(
                    ErrorCode::DanglingRelationship,
                    "level target is not active",
                ));
            }
            Some(_) => {
                return Err(fail(
                    ErrorCode::InvalidArgument,
                    "level target is not a level",
                ));
            }
            None if prims
                .iter()
                .any(|p| p.path == target && p.element_id.is_some()) =>
            {
                return Err(fail(
                    ErrorCode::InvalidArgument,
                    "level target is not a supported owned level",
                ));
            }
            None => {
                return Err(fail(
                    ErrorCode::DanglingRelationship,
                    "level target unavailable",
                ));
            }
        }
    }
    Ok(())
}

/// The value of a mark (A08): 1 to 64 UTF-8 bytes without control characters.
fn mark_value(s: &str) -> Result<(), Diagnostic> {
    if s.is_empty() || s.len() > 64 || s.chars().any(char::is_control) {
        return Err(invalid());
    }
    Ok(())
}

/// A08 marks (`A08-V01`): `bim:mark` and its `bim:markStatus` are authored together on one
/// supported fabricable element (a level is not fabricated), the value is bounded readable text
/// and no two elements of a model carry the same value. Foreign, newer and unknown content stays
/// inert, as everywhere: only owned 0.1.0 parametric elements are checked.
fn marks(prims: &[Prim]) -> Result<(), Diagnostic> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for p in prims
        .iter()
        .filter_map(|p| supported_kind(p).map(|k| (p, k)))
    {
        let value = optional_text(p.0, "bim:mark")?;
        let status = optional(p.0, "bim:markStatus", "token")?;
        match (&value, status) {
            (None, None) => continue,
            (None, Some(_)) | (Some(_), None) => return Err(invalid()),
            (Some(v), Some(s)) => {
                mark_value(v)?;
                if !matches!(s, Value::Text(t) if MarkStatus::parse(t).is_some()) {
                    return Err(invalid());
                }
            }
        }
        if p.1 == "level" {
            return Err(fail(ErrorCode::InvalidArgument, "a level carries no mark"));
        }
        if !seen.insert(value.as_deref().ok_or_else(invalid)?.to_owned()) {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "the mark is already in use",
            ));
        }
    }
    Ok(())
}

/// The weld properties of a loaded `connection` (A06): every one is authored, with a supported
/// token, and the size is finite in `(0, 1000]`. A connection without them is not a weld this
/// slice can read, so it is refused rather than guessed.
fn weld_properties(p: &Prim) -> Result<(), Diagnostic> {
    weld_tokens(
        text(p, "steel:weld:type", "token")?,
        text(p, "steel:weld:location", "token")?,
        text(p, "steel:weld:side", "token")?,
    )?;
    weld_size(number(p, "steel:weld:sizeMm")?)
}
fn weld_tokens(weld_type: &str, location: &str, side: &str) -> Result<(), Diagnostic> {
    if weld_type != "fillet" {
        return Err(unsupported());
    }
    if !matches!(location, "workshop" | "site") || !matches!(side, "arrow" | "other" | "both") {
        return Err(invalid());
    }
    Ok(())
}
fn weld_size(mm: f64) -> Result<(), Diagnostic> {
    finite([mm])?;
    if mm <= 0.0 {
        return Err(invalid());
    }
    cap([mm], 1000.0)
}

/// A06 `connects` records (domain vocabulary section 5.1): only a weld carries one, with exactly
/// two distinct targets (`DV-REL-004`, `DV-REL-007`), never itself (`DV-REL-006`), each an active
/// owned plate, beam or column (`DV-REL-001` dangling, `DV-REL-003` kind). An assembly is a valid
/// target in the vocabulary but not for a weld in this slice.
fn connects(prims: &[Prim]) -> Result<(), Diagnostic> {
    let elements: BTreeMap<&str, (&Prim, &str)> = prims
        .iter()
        .filter_map(|p| supported_kind(p).map(|k| (p.path.as_str(), (p, k))))
        .collect();
    for (&path, &(p, kind)) in &elements {
        let Some(r) = relationship(p, "bim:connects") else {
            if kind == "connection" {
                return Err(fail(
                    ErrorCode::DependencyConflict,
                    "a weld connects exactly two parts",
                ));
            }
            continue;
        };
        if kind != "connection" {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "only a connection carries a connects record",
            ));
        }
        let [first, second] = r.targets.as_slice() else {
            return Err(fail(
                ErrorCode::DependencyConflict,
                "a weld connects exactly two parts",
            ));
        };
        if first == second {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "a weld connects two distinct parts",
            ));
        }
        for target in [first, second] {
            if target == path {
                return Err(fail(
                    ErrorCode::InvalidArgument,
                    "a weld cannot connect itself",
                ));
            }
            match elements.get(target.as_str()) {
                Some((t, _)) if !t.active => {
                    return Err(fail(
                        ErrorCode::DanglingRelationship,
                        "connects target is not active",
                    ));
                }
                Some((_, "plate" | "beam" | "column")) => {}
                Some((_, "assembly")) => return Err(unsupported()),
                Some(_) => {
                    return Err(fail(
                        ErrorCode::InvalidArgument,
                        "connects target is not a plate, beam or column",
                    ));
                }
                None if prims
                    .iter()
                    .any(|p| p.path == *target && p.element_id.is_some()) =>
                {
                    return Err(fail(
                        ErrorCode::InvalidArgument,
                        "connects target is not a supported owned part",
                    ));
                }
                None => {
                    return Err(fail(
                        ErrorCode::DanglingRelationship,
                        "connects target unavailable",
                    ));
                }
            }
        }
    }
    Ok(())
}

/// A01 `part-of` records (domain vocabulary section 5.1): a supported element has at most one
/// target (`DV-REL-004`, `DEPENDENCY_CONFLICT`), never itself (`DV-REL-006`), which must be an
/// active owned assembly (`DV-REL-001` dangling, `DV-REL-003` kind), and the records form no cycle
/// of any length (`DV-REL-005`, `DEPENDENCY_CONFLICT`). The walk is linear in the element count.
fn part_of(prims: &[Prim]) -> Result<(), Diagnostic> {
    let elements: BTreeMap<&str, (&Prim, &str)> = prims
        .iter()
        .filter_map(|p| supported_kind(p).map(|k| (p.path.as_str(), (p, k))))
        .collect();
    let mut parent: BTreeMap<&str, &str> = BTreeMap::new();
    for (&path, &(p, _)) in &elements {
        let Some(r) = relationship(p, "bim:partOf") else {
            continue;
        };
        let target = match r.targets.as_slice() {
            [] => continue,
            [target] => target.as_str(),
            _ => {
                return Err(fail(
                    ErrorCode::DependencyConflict,
                    "an element is a part of at most one assembly",
                ));
            }
        };
        if target == path {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "an element cannot be a part of itself",
            ));
        }
        match elements.get(target) {
            Some((t, "assembly")) if t.active => {}
            Some((t, _)) if !t.active => {
                return Err(fail(
                    ErrorCode::DanglingRelationship,
                    "part-of target is not active",
                ));
            }
            Some(_) => {
                return Err(fail(
                    ErrorCode::InvalidArgument,
                    "part-of target is not an assembly",
                ));
            }
            None if prims
                .iter()
                .any(|p| p.path == target && p.element_id.is_some()) =>
            {
                return Err(fail(
                    ErrorCode::InvalidArgument,
                    "part-of target is not a supported owned assembly",
                ));
            }
            None => {
                return Err(fail(
                    ErrorCode::DanglingRelationship,
                    "part-of target unavailable",
                ));
            }
        }
        parent.insert(path, target);
    }
    // 0 unvisited, 1 on the current chain, 2 proven acyclic.
    let mut state: BTreeMap<&str, u8> = BTreeMap::new();
    for &start in parent.keys() {
        let mut chain = Vec::new();
        let mut at = Some(start);
        while let Some(node) = at {
            match state.get(node).copied().unwrap_or(0) {
                2 => break,
                1 => {
                    return Err(fail(
                        ErrorCode::DependencyConflict,
                        "part-of records form a cycle",
                    ));
                }
                _ => {
                    state.insert(node, 1);
                    chain.push(node);
                    at = parent.get(node).copied();
                }
            }
        }
        for node in chain {
            state.insert(node, 2);
        }
    }
    Ok(())
}

/// The `part-of` chain from `start` upwards: `start`, its assembly, that one's assembly, and so on.
/// The batch's own `PartOf` changes override the composed records. The walk stops at a repeated
/// element (a cycle, refused later by the candidate check) and is bounded by the prim count.
fn chain(prims: &[Prim], changes: &[Change], start: &str) -> Vec<String> {
    let id_of_path: BTreeMap<&str, &str> = prims
        .iter()
        .filter_map(|p| Some((p.path.as_str(), p.element_id.as_deref()?)))
        .collect();
    let mut parent: BTreeMap<String, Option<String>> = BTreeMap::new();
    for p in prims {
        let (Some(id), Some(r)) = (p.element_id.as_deref(), relationship(p, "bim:partOf")) else {
            continue;
        };
        let target = match r.targets.as_slice() {
            [target] => id_of_path.get(target.as_str()).map(|t| (*t).to_owned()),
            _ => None,
        };
        parent.insert(id.to_owned(), target);
    }
    for change in changes {
        if let Change::PartOf { id, assembly } = change {
            parent.insert(id.clone(), assembly.clone());
        }
    }
    let mut out: Vec<String> = Vec::new();
    let mut at = Some(start.to_owned());
    while let Some(node) = at {
        if out.contains(&node) || out.len() > prims.len().saturating_add(changes.len()) {
            break;
        }
        at = parent.get(&node).cloned().flatten();
        out.push(node);
    }
    out
}

pub(super) fn changes(
    input: &Prepare,
    prims: &[Prim],
    b: Budgets,
) -> Result<(BTreeSet<Footprint>, BTreeSet<Footprint>), Diagnostic> {
    let mut reads = BTreeSet::new();
    let mut writes = BTreeSet::new();
    for change in &input.changes {
        let (id, _) = change.key();
        let path = element_path(id)?;
        let existing = prims.iter().find(|p| p.path == path);
        // A wrapper is owned although its `Source` descendant is foreign: only the wrapper's own
        // levels are checked here, and the loaded shape (`reference::element`) refuses every
        // owned opinion below `Source`. Every other element must be wholly owned.
        let wrapper = existing
            .is_some_and(|p| matches!(text(p, "bim:geometryMode", "token"), Ok("reference")));
        owned(prims, &path, !wrapper)?;
        if matches!(change, Change::Delete { .. })
            && input.changes.iter().filter(|c| c.key().0 == id).count() > 1
        {
            return Err(fail(
                ErrorCode::MalformedCommand,
                "a delete cannot be combined with other changes of the element",
            ));
        }
        match change {
            Change::Create(_) | Change::AttachReference(_) if existing.is_some() => {
                return Err(fail(ErrorCode::DuplicateId, "element already exists"));
            }
            Change::Create(_) | Change::AttachReference(_) => (),
            // A mesh or part-of record of an element created in the same batch; the candidate
            // snapshot re-validates it as a whole.
            Change::Mesh(_) | Change::PartOf { .. } | Change::Level { .. }
                if existing.is_none()
                    && input
                        .changes
                        .iter()
                        .any(|c| matches!(c,Change::Create(e) if e.id == id)) => {}
            _ => {
                let p = existing.ok_or_else(|| fail(ErrorCode::NotFound, "element not found"))?;
                if p.element_id.as_deref() != Some(id) {
                    return Err(fail(ErrorCode::InvalidId, "element identity mismatch"));
                }
                if text(p, "bim:schemaVersion", "string")? != "0.1.0" {
                    return Err(fail(
                        ErrorCode::UnsupportedVersion,
                        "element schema version mismatch",
                    ));
                }
                let kind = text(p, "bim:elementKind", "token")?;
                let mode = text(p, "bim:geometryMode", "token")?;
                if mode != if wrapper { "reference" } else { "parametric" }
                    || (wrapper && kind != "reference")
                {
                    return Err(unsupported());
                }
                let valid = match change {
                    Change::Contour { .. } | Change::Thickness { .. } => kind == "plate",
                    Change::Axis { .. } | Change::Roll { .. } | Change::Profile { .. } => {
                        matches!(kind, "beam" | "column")
                    }
                    Change::ReferenceUnits { .. } => wrapper,
                    // A rename and a move are the only edits a wrapper takes as an element; an
                    // assembly can be renamed, deleted and be a part, never moved (identity pose).
                    Change::Placement { .. } => {
                        matches!(kind, "plate" | "beam" | "column" | "reference")
                    }
                    Change::Label { .. } => {
                        matches!(
                            kind,
                            "plate"
                                | "beam"
                                | "column"
                                | "reference"
                                | "assembly"
                                | "connection"
                                | "level"
                        )
                    }
                    Change::Delete { .. } => {
                        matches!(
                            kind,
                            "plate" | "beam" | "column" | "assembly" | "connection" | "level"
                        )
                    }
                    Change::PartOf { .. } => {
                        matches!(kind, "plate" | "beam" | "column" | "assembly")
                    }
                    Change::Level { .. } => matches!(kind, "plate" | "beam" | "column"),
                    // A mark lives on the fabricable elements a bill can list; a level is not
                    // fabricated and a foreign wrapper is never marked (A08).
                    Change::Mark { value, status, .. } => {
                        if value.is_none() && status.is_none() {
                            return Err(fail(
                                ErrorCode::MalformedCommand,
                                "a mark change authors nothing",
                            ));
                        }
                        if let Some(v) = value {
                            mark_value(v)?;
                        }
                        matches!(
                            kind,
                            "plate" | "beam" | "column" | "assembly" | "connection"
                        )
                    }
                    _ => matches!(kind, "plate" | "beam" | "column"),
                };
                if !valid {
                    return Err(unsupported());
                }
            }
        }
        let (r, w): (&[Group], &[Group]) = match change {
            Change::Create(_) => (
                &[Group::Lifecycle],
                &[
                    Group::Lifecycle,
                    Group::Label,
                    Group::ShapeInputs,
                    Group::Placement,
                    Group::Material,
                    Group::Relationship,
                    Group::Derived,
                ],
            ),
            Change::AttachReference(_) => (
                &[Group::Lifecycle],
                &[
                    Group::Lifecycle,
                    Group::Label,
                    Group::ShapeInputs,
                    Group::Placement,
                ],
            ),
            Change::Label { .. } => (&[Group::Lifecycle, Group::Label], &[Group::Label]),
            Change::Material { .. } => (
                &[Group::Lifecycle, Group::Derived],
                &[Group::Material, Group::Relationship, Group::Derived],
            ),
            Change::Placement { .. } => (
                &[Group::Lifecycle, Group::ShapeInputs, Group::Placement],
                &[Group::Placement, Group::Derived],
            ),
            Change::Mesh(_) => (&[Group::Lifecycle, Group::ShapeInputs], &[Group::Derived]),
            Change::PartOf { .. } | Change::Level { .. } => (
                &[Group::Lifecycle, Group::Relationship],
                &[Group::Relationship],
            ),
            // A mark decision reads the element's existence and its current mark; it writes only
            // the mark group (A08), mirroring the engine's `Footprint::for_mark`.
            Change::Mark { .. } => (&[Group::Lifecycle, Group::Mark], &[Group::Mark]),
            Change::Delete { .. } => (
                &[Group::Lifecycle, Group::Relationship],
                &[
                    Group::Lifecycle,
                    Group::Label,
                    Group::ShapeInputs,
                    Group::Placement,
                    Group::Material,
                    Group::Relationship,
                    Group::Derived,
                ],
            ),
            _ => (
                &[Group::Lifecycle, Group::ShapeInputs],
                &[Group::ShapeInputs, Group::Derived],
            ),
        };
        for &group in r {
            reads.insert(Footprint {
                identity: id.into(),
                group,
            });
        }
        for &group in w {
            writes.insert(Footprint {
                identity: id.into(),
                group,
            });
        }
        if let Change::PartOf {
            assembly: Some(target),
            ..
        } = change
        {
            if !uuid7(target) {
                return Err(fail(ErrorCode::InvalidId, "invalid full element identity"));
            }
            // The target's existence and kind and its whole chain decide acceptance (cycle check),
            // so every assembly on the chain is read, with this batch's own records applied.
            for ancestor in chain(prims, &input.changes, target) {
                for group in [Group::Lifecycle, Group::Relationship] {
                    reads.insert(Footprint {
                        identity: ancestor.clone(),
                        group,
                    });
                }
            }
        }
        if let Change::Level {
            level: Some(target),
            ..
        } = change
        {
            if !uuid7(target) {
                return Err(fail(ErrorCode::InvalidId, "invalid full element identity"));
            }
            // The level's existence and kind decide acceptance; it is only read.
            reads.insert(Footprint {
                identity: target.clone(),
                group: Group::Lifecycle,
            });
        }
        if !matches!(
            change,
            Change::Label { .. }
                | Change::Create(_)
                | Change::Delete { .. }
                | Change::PartOf { .. }
                | Change::Level { .. }
        ) && let Some(p) = existing
        {
            for target in p
                .relationships
                .iter()
                .filter(|r| matches!(r.name.as_str(), "bim:profile" | "bim:material"))
                .flat_map(|r| &r.targets)
            {
                let hex = target
                    .strip_prefix("/World/Definitions/D_")
                    .ok_or_else(invalid)?;
                let digest = hex.parse::<ContentDigest>().map_err(|_| invalid())?;
                reads.insert(Footprint {
                    identity: digest.to_string(),
                    group: Group::Relationship,
                });
            }
        }
        if matches!(change, Change::Mesh(_)) {
            let mesh_path = format!("{path}/Geometry");
            owned(prims, &mesh_path, true)?;
        }
    }
    // Whole-batch finiteness precedes numeric caps/ranges, independent of insertion order.
    for change in &input.changes {
        finite_change(change)?;
    }
    for resource in &input.resources {
        resource.bytes()?;
        reads.insert(Footprint {
            identity: resource.digest.to_string(),
            group: Group::Relationship,
        });
        // A definition that is not in the stage yet is created; an existing one is immutable and
        // must already be exactly this content, active and owned (kind, fields, types, label and
        // the recomputed digest), or the batch is refused instead of treating it as a no-op.
        if prims
            .iter()
            .any(|p| p.path == definition_path(resource.digest))
        {
            let kind = match &resource.definition {
                Definition::RectangleProfile { .. } => "rectangle",
                Definition::SectionProfile { shape, .. } => shape.token(),
                Definition::Material { .. } => "material",
            };
            self::resource(prims, resource.digest, &[kind])?;
        } else {
            writes.insert(Footprint {
                identity: resource.digest.to_string(),
                group: Group::Relationship,
            });
        }
    }
    if reads.len().saturating_add(writes.len()) > 4096 {
        return Err(limit());
    }
    for change in &input.changes {
        match change {
            Change::Create(e) => {
                label(&e.name)?;
                pose(&e.placement)?;
                match &e.shape {
                    Shape::Plate {
                        contour_mm,
                        thickness_mm,
                        holes,
                    } => {
                        count_points(contour_mm, b)?;
                        cap(contour_mm.iter().flatten().copied(), 1e9)?;
                        cap([*thickness_mm], 1e6)?;
                        if holes.len() > MAX_HOLES {
                            return Err(limit());
                        }
                        for hole in holes {
                            if hole.len() > MAX_HOLE_POINTS {
                                return Err(limit());
                            }
                            if hole.len() < 3 || hole.iter().any(|p| p[2] != 0.0) {
                                return Err(invalid());
                            }
                            cap(hole.iter().flatten().copied(), 1e9)?;
                        }
                    }
                    Shape::Member {
                        axis_mm,
                        roll_rad,
                        profile,
                        ..
                    } => {
                        cap(axis_mm.iter().flatten().copied(), 1e9)?;
                        if let Some(n) = roll_rad {
                            cap([*n], std::f64::consts::TAU)?;
                        }
                        reads.insert(Footprint {
                            identity: profile.to_string(),
                            group: Group::Relationship,
                        });
                    }
                    Shape::Assembly => {
                        if e.material.is_some()
                            || e.placement.translation_mm != [0.0; 3]
                            || e.placement.orientation_xyzw != [0.0, 0.0, 0.0, 1.0]
                        {
                            return Err(unsupported());
                        }
                    }
                    Shape::Weld(w) => {
                        if e.material.is_some()
                            || e.placement.translation_mm != [0.0; 3]
                            || e.placement.orientation_xyzw != [0.0, 0.0, 0.0, 1.0]
                        {
                            return Err(unsupported());
                        }
                        weld_tokens(&w.weld_type, &w.location, &w.side)?;
                        weld_size(w.size_mm)?;
                        let [main, secondary] = &w.connects;
                        if !uuid7(main) || !uuid7(secondary) {
                            return Err(fail(
                                ErrorCode::InvalidId,
                                "invalid full element identity",
                            ));
                        }
                        if main == secondary || main == &e.id || secondary == &e.id {
                            return Err(fail(
                                ErrorCode::InvalidArgument,
                                "a weld connects two distinct other parts",
                            ));
                        }
                        // The parts' existence and kinds decide acceptance; they are only read.
                        for part in [main, secondary] {
                            reads.insert(Footprint {
                                identity: part.clone(),
                                group: Group::Lifecycle,
                            });
                        }
                    }
                    Shape::Level { elevation_mm } => {
                        if e.material.is_some()
                            || e.placement.translation_mm != [0.0; 3]
                            || e.placement.orientation_xyzw != [0.0, 0.0, 0.0, 1.0]
                        {
                            return Err(unsupported());
                        }
                        elevation(*elevation_mm)?;
                    }
                }
                if let Some(d) = e.material {
                    reads.insert(Footprint {
                        identity: d.to_string(),
                        group: Group::Relationship,
                    });
                }
            }
            Change::Label { name, .. } => label(name)?,
            Change::AttachReference(r) => reference::check_new(r)?,
            Change::ReferenceUnits { interpretation, .. } => {
                reference::check_units(interpretation)?
            }
            Change::Contour { points_mm, .. } => {
                count_points(points_mm, b)?;
                cap(points_mm.iter().flatten().copied(), 1e9)?;
            }
            Change::Thickness { mm, .. } => cap([*mm], 1e6)?,
            Change::Axis { points_mm, .. } => cap(points_mm.iter().flatten().copied(), 1e9)?,
            Change::Roll {
                opinion: Opinion::Value(n),
                ..
            } => cap([*n], std::f64::consts::TAU)?,
            Change::Placement { placement, .. } => pose(placement)?,
            Change::Material {
                opinion: MaterialOpinion::Target(d),
                ..
            } => {
                reads.insert(Footprint {
                    identity: d.to_string(),
                    group: Group::Relationship,
                });
            }
            Change::Profile { digest, .. } => {
                reads.insert(Footprint {
                    identity: digest.to_string(),
                    group: Group::Relationship,
                });
            }
            Change::Mesh(m) => {
                mesh(m, b)?;
                if let Some(q) = &m.quantities {
                    quantities(q)?;
                }
            }
            _ => (),
        }
    }
    if reads.len().saturating_add(writes.len()) > 4096 {
        return Err(limit());
    }
    Ok((reads, writes))
}
fn count_points(points: &[[f64; 3]], b: Budgets) -> Result<(), Diagnostic> {
    if points.len() > 1024 || points.len().saturating_mul(3) > b.array_items {
        return Err(limit());
    }
    if points.len() < 3 {
        return Err(invalid());
    }
    Ok(())
}
fn finite_change(c: &Change) -> Result<(), Diagnostic> {
    match c {
        Change::Create(e) => {
            finite(
                e.placement
                    .translation_mm
                    .into_iter()
                    .chain(e.placement.orientation_xyzw),
            )?;
            match &e.shape {
                Shape::Plate {
                    contour_mm,
                    thickness_mm,
                    holes,
                } => finite(
                    contour_mm
                        .iter()
                        .flatten()
                        .copied()
                        .chain(holes.iter().flatten().flatten().copied())
                        .chain([*thickness_mm]),
                ),
                Shape::Member {
                    axis_mm, roll_rad, ..
                } => finite(
                    axis_mm
                        .iter()
                        .flatten()
                        .copied()
                        .chain(roll_rad.iter().copied()),
                ),
                Shape::Assembly => Ok(()),
                Shape::Weld(w) => finite([w.size_mm]),
                Shape::Level { elevation_mm } => finite([*elevation_mm]),
            }
        }
        Change::AttachReference(r) => finite(
            r.placement
                .translation_mm
                .into_iter()
                .chain(r.placement.orientation_xyzw)
                .chain(r.source.declared_meters_per_unit),
        ),
        Change::ReferenceUnits { interpretation, .. } => {
            finite(interpretation.iter().map(|i| i.meters_per_unit))
        }
        Change::Contour { points_mm, .. } => finite(points_mm.iter().flatten().copied()),
        Change::Axis { points_mm, .. } => finite(points_mm.iter().flatten().copied()),
        Change::Thickness { mm, .. } => finite([*mm]),
        Change::Roll {
            opinion: Opinion::Value(n),
            ..
        } => finite([*n]),
        Change::Placement { placement, .. } => finite(
            placement
                .translation_mm
                .into_iter()
                .chain(placement.orientation_xyzw),
        ),
        Change::Mesh(m) => finite(
            m.points_mm.iter().flatten().map(|n| f64::from(*n)).chain(
                m.quantities
                    .iter()
                    .flat_map(|q| [q.volume_mm3].into_iter().chain(q.mass_kg)),
            ),
        ),
        _ => Ok(()),
    }
}
fn quantities(q: &StoredQuantities) -> Result<(), Diagnostic> {
    let status = |s: &str| matches!(s, "current" | "stale" | "unknown");
    if !status(&q.volume_status)
        || !status(&q.mass_status)
        || q.algorithm.len() > 128
        || q.algorithm.is_empty()
        || q.volume_mm3 < 0.0
        || q.mass_kg.is_some_and(|kg| kg < 0.0)
        // A mass value exists exactly when its status is current.
        || q.mass_kg.is_some() != (q.mass_status == "current")
    {
        return Err(invalid());
    }
    cap([q.volume_mm3], 1e30)?;
    cap(q.mass_kg, 1e30)
}
fn mesh(m: &Mesh, b: Budgets) -> Result<(), Diagnostic> {
    if m.points_mm.len().saturating_mul(3) > b.array_items
        || m.face_counts.len() > b.array_items
        || m.indices.len() > b.array_items
    {
        return Err(limit());
    }
    cap(m.points_mm.iter().flatten().map(|n| f64::from(*n)), 1e9)?;
    if m.points_mm.is_empty() || m.face_counts.is_empty() {
        return Err(invalid());
    }
    let mut count = 0usize;
    for &n in &m.face_counts {
        if n < 3 {
            return Err(invalid());
        }
        count = count
            .checked_add(usize::try_from(n).map_err(|_| invalid())?)
            .ok_or_else(limit)?;
    }
    if count != m.indices.len()
        || m.indices
            .iter()
            .any(|&n| usize::try_from(n).map_or(true, |i| i >= m.points_mm.len()))
    {
        return Err(invalid());
    }
    Ok(())
}
