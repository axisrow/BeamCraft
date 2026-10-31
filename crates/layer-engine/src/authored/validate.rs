// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

use super::*;
use beamcraft_model::fastener::{FastenerKind, HeadKind, HoleMode, WasherSpec};
use beamcraft_model::id::OpId;
use beamcraft_model::limits::{
    MAX_FASTENER_LENGTHS, MAX_MEMBER_PATH_POINTS, MAX_RELATIONSHIP_TARGETS, MIN_WELD_SEGMENT_MM,
};
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
        // A schema change names no element: its bounded definition or name is the whole payload.
        match change {
            Change::AttrDefine { def } => {
                def.validate().map_err(|_| invalid())?;
                let size = def.name.len()
                    + def.unit.as_ref().map_or(0, |s| s.len())
                    + def.set.as_ref().map_or(0, |s| s.len())
                    + def.options.iter().map(|o| o.len()).sum::<usize>();
                preflight::charge(&mut bytes, size, 1024 * 1024)?;
                continue;
            }
            Change::AttrUndefine { name } => {
                schema_path_name(name)?;
                preflight::charge(&mut bytes, name.len(), 1024 * 1024)?;
                continue;
            }
            _ => {}
        }
        if !uuid7(change.key().0) {
            return Err(fail(ErrorCode::InvalidId, "invalid full element identity")
                .with_rule("DV-BASE-001"));
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
                    return Err(limit().with_rule("DV-BASE-003"));
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
                if let Shape::LoftedPlate { start_mm, end_mm } = &e.shape {
                    count(
                        start_mm
                            .len()
                            .checked_add(end_mm.len())
                            .ok_or_else(limit)?
                            .checked_mul(3)
                            .ok_or_else(limit)?,
                        8,
                    )?;
                }
            }
            Change::Label { name, .. } if name.len() > 256 => {
                return Err(limit().with_rule("DV-BASE-003"));
            }
            Change::AttrSet { name, value, .. } => {
                // The value's own bounds are model admission; charge its worst stored size.
                let size = name.len()
                    + 8
                    + match value {
                        Some(v) => v.as_text().map_or(8, str::len),
                        None => 0,
                    };
                preflight::charge(&mut bytes, size, 1024 * 1024)?;
            }
            Change::Mark { mark: m, .. } => mark(m)?,
            Change::AttachReference(r) if r.name.len() > 256 => {
                return Err(limit().with_rule("DV-BASE-003"));
            }
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
        return Err(
            fail(ErrorCode::NonFiniteNumber, "non-finite authored value").with_rule("DV-ATTR-003"),
        );
    }
    Ok(())
}
pub(super) fn cap(values: impl IntoIterator<Item = f64>, max: f64) -> Result<(), Diagnostic> {
    if values.into_iter().any(|n| n.abs() > max) {
        return Err(limit().with_rule("DV-LIM-001"));
    }
    Ok(())
}
pub(super) fn label(s: &str) -> Result<(), Diagnostic> {
    if s.len() > 256 {
        return Err(limit().with_rule("DV-BASE-003"));
    }
    if s.is_empty() || s.chars().any(label_char_forbidden) {
        return Err(invalid().with_rule("DV-BASE-003"));
    }
    Ok(())
}
/// A user-attribute name, checked on the candidate and at every write (I04).
pub(super) fn schema_path_name(name: &str) -> Result<&str, Diagnostic> {
    if beamcraft_model::attribute::attr_name_is_valid(name) {
        Ok(name)
    } else {
        Err(invalid())
    }
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
    let refused = |e: beamcraft_model::command::CommandError| Diagnostic {
        code: e.code,
        message: "definition value refused by the F04 admission rules",
        rule_id: e.rule_id,
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
        Definition::Fastener { spec } => spec.validate().map_err(refused),
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
/// The user-attribute schema scope (I04): every owned definition prim must be one bounded
/// segment below the scope and a complete, well-formed definition; foreign or unknown content
/// stays inert and is never interpreted.
pub(super) fn attribute_schema(prims: &[Prim]) -> Result<(), Diagnostic> {
    let scope = format!("{}/", super::ATTR_SCHEMA_PATH);
    for p in prims.iter().filter(|p| p.path.starts_with(&scope)) {
        if p.sources.iter().any(|s| !s.owned) {
            continue;
        }
        let name = p.path.strip_prefix(&scope).ok_or_else(invalid)?;
        schema_path_name(name)?;
        if name.contains('/') {
            return Err(unsupported());
        }
        let ty = beamcraft_model::attribute::AttrType::parse(text(p, "bim:attr:type", "token")?)
            .ok_or_else(unsupported)?;
        let options = match optional(p, "bim:attr:options", "string[]")? {
            None => Vec::new(),
            Some(Value::Array(items)) => {
                let mut options = Vec::with_capacity(items.len());
                for item in items {
                    match item {
                        Value::Text(option) => options.push(option.clone()),
                        _ => return Err(invalid()),
                    }
                }
                options
            }
            Some(_) => return Err(invalid()),
        };
        let def = beamcraft_model::attribute::AttrDef {
            name: name.to_owned(),
            ty,
            unit: optional_text(p, "bim:attr:unit")?,
            options,
            set: optional_text(p, "bim:attr:set")?,
        };
        def.validate().map_err(|_| invalid())?;
    }
    Ok(())
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
        "fastener" => Ok(Definition::Fastener {
            spec: read_fastener(p)?,
        }),
        _ => Err(unsupported()),
    }
}

/// The logical fastener definition stored in a fastener definition prim (A14.1): every value
/// re-read and re-admitted, because a committed layer is untrusted input.
fn read_fastener(p: &Prim) -> Result<FastenerSpec, Diagnostic> {
    let token = |name: &str| text(p, name, "token").map(str::to_owned);
    let kind = FastenerKind::from_token(&token("steel:fastener:kind")?).ok_or_else(invalid)?;
    let head_kind = HeadKind::from_token(&token("steel:fastener:headKind")?).ok_or_else(invalid)?;
    let nut_across_flats_mm = optional_number(p, "steel:fastener:nutAcrossFlatsMm")?;
    let nut_height_mm = optional_number(p, "steel:fastener:nutHeightMm")?;
    let washer = match (
        optional_number(p, "steel:fastener:washerOuterDiameterMm")?,
        optional_number(p, "steel:fastener:washerInnerDiameterMm")?,
        optional_number(p, "steel:fastener:washerThicknessMm")?,
    ) {
        (None, None, None) => None,
        (outer, inner, thickness) => Some(WasherSpec {
            outer_diameter_mm: outer.ok_or_else(invalid)?,
            inner_diameter_mm: inner.ok_or_else(invalid)?,
            thickness_mm: thickness.ok_or_else(invalid)?,
        }),
    };
    let lengths = match required(p, "steel:fastener:lengthsMm", "double[]")? {
        Value::Array(values) => values
            .iter()
            .map(|v| match v {
                Value::Number(n) => Ok(*n),
                _ => Err(invalid()),
            })
            .collect::<Result<Vec<_>, Diagnostic>>()?,
        _ => return Err(invalid()),
    };
    if lengths.len() > MAX_FASTENER_LENGTHS {
        return Err(limit());
    }
    let hole_mode = HoleMode::from_token(&token("steel:fastener:holeMode")?).ok_or_else(invalid)?;
    Ok(FastenerSpec {
        designation: optional_text(p, "steel:fastener:designation")?,
        standard: optional_text(p, "steel:fastener:standard")?,
        grade: optional_text(p, "steel:fastener:grade")?,
        display_name: p.display_name.clone(),
        kind,
        head_kind,
        nominal_diameter_mm: number(p, "steel:fastener:nominalDiameterMm")?,
        head_across_flats_mm: number(p, "steel:fastener:headAcrossFlatsMm")?,
        head_height_mm: number(p, "steel:fastener:headHeightMm")?,
        nut_across_flats_mm,
        nut_height_mm,
        washer,
        hole_clearance_mm: number(p, "steel:fastener:holeClearanceMm")?,
        lengths_mm: lengths,
        thread_protrusion_mm: number(p, "steel:fastener:threadProtrusionMm")?,
        mass_each_kg: optional_number(p, "steel:fastener:massEachKg")?,
        washer_under_head: match required(p, "steel:fastener:washerUnderHead", "bool")? {
            Value::Bool(v) => *v,
            _ => return Err(invalid()),
        },
        washer_under_nut: match required(p, "steel:fastener:washerUnderNut", "bool")? {
            Value::Bool(v) => *v,
            _ => return Err(invalid()),
        },
        hole_mode,
    })
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
    let p = prim
        .properties
        .iter()
        .find(|p| p.name == name)
        .ok_or_else(|| invalid().with_rule("DV-ATTR-001"))?;
    if p.type_name != ty {
        return Err(invalid().with_rule("DV-ATTR-002"));
    }
    p.value
        .as_ref()
        .filter(|v| !matches!(v, Value::Block))
        .ok_or_else(|| invalid().with_rule("DV-ATTR-001"))
}
pub(super) fn text<'a>(prim: &'a Prim, name: &str, ty: &str) -> Result<&'a str, Diagnostic> {
    match required(prim, name, ty)? {
        Value::Text(s) => Ok(s),
        _ => Err(invalid().with_rule("DV-ATTR-002")),
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
/// The stored face contours of a lofted plate (A11): the shared coordinate cap and the
/// vertex-count bounds of the schema descriptor. Winding, planarity, simplicity and the
/// equal vertex count of the loft correspondence are the engine's evaluation decision.
fn lofted_plate(start: &[[f64; 3]], end: &[[f64; 3]]) -> Result<(), Diagnostic> {
    cap(start.iter().flatten().copied(), 1e9)?;
    cap(end.iter().flatten().copied(), 1e9)?;
    let span = |points: &[[f64; 3]]| -> f64 {
        let Some(&first) = points.first() else {
            return 0.0;
        };
        let mut size = 0.0;
        for &p in points {
            let n = norm(difference(p, first));
            if n > size {
                size = n;
            }
        }
        size
    };
    if start.len() < 3 || start.len() != end.len() || span(start).max(span(end)) > 2e6 {
        return Err(invalid());
    }
    Ok(())
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

/// The optional through-hole operation records of a loaded plate (A04): all four attributes or
/// none, the counts add up to the points, every polygon has at least three in-cap points with
/// every `z` zero, there is one full UUIDv7 and one strictly increasing non-negative ordinal
/// per polygon, and the totals are capped. Winding, placement and clearance are the engine's
/// decision on replay, never re-derived here.
fn plate_hole_records(prim: &Prim) -> Result<(), Diagnostic> {
    let present = |name: &str| prim.properties.iter().any(|p| p.name == name);
    let names = [
        "steel:hole:pointsMm",
        "steel:hole:vertexCounts",
        "steel:hole:operationIds",
        "steel:hole:ordinals",
        "steel:hole:kinds",
        "steel:hole:generatedBy",
    ];
    let declared: Vec<bool> = names.iter().map(|name| present(name)).collect();
    if declared.iter().all(|seen| !seen) {
        return Ok(());
    }
    if !declared.iter().all(|seen| *seen) {
        return Err(invalid());
    }
    let Value::Array(counts) = required(prim, "steel:hole:vertexCounts", "int[]")? else {
        return Err(invalid());
    };
    let Value::Array(ids) = required(prim, "steel:hole:operationIds", "string[]")? else {
        return Err(invalid());
    };
    let Value::Array(ordinals) = required(prim, "steel:hole:ordinals", "int[]")? else {
        return Err(invalid());
    };
    let Value::Array(kinds) = required(prim, "steel:hole:kinds", "token[]")? else {
        return Err(invalid());
    };
    let Value::Array(owners) = required(prim, "steel:hole:generatedBy", "string[]")? else {
        return Err(invalid());
    };
    if counts.len() > MAX_HOLES
        || counts.len() != ids.len()
        || counts.len() != ordinals.len()
        || counts.len() != kinds.len()
        || counts.len() != owners.len()
    {
        return Err(invalid());
    }
    // Every generated record names its full generating bolt group; a hand-placed hole stores
    // the empty string (A14.4).
    for owner in owners {
        let Value::Text(owner) = owner else {
            return Err(invalid());
        };
        if !owner.is_empty() && !uuid7(owner) {
            return Err(invalid());
        }
    }
    let points = points(prim, "steel:hole:pointsMm")?;
    if points.len() > MAX_HOLES * MAX_HOLE_POINTS {
        return Err(limit());
    }
    cap(points.iter().flatten().copied(), 1e9)?;
    let mut at = 0usize;
    let mut last: Option<i64> = None;
    for count in counts {
        let n = match count {
            Value::Int(n) => usize::try_from(*n).map_err(|_| invalid())?,
            _ => return Err(invalid()),
        };
        if !(3..=MAX_HOLE_POINTS).contains(&n) {
            return Err(invalid());
        }
        let end = at.checked_add(n).ok_or_else(invalid)?;
        let hole = points.get(at..end).ok_or_else(invalid)?;
        if hole.iter().any(|p| p[2] != 0.0) {
            return Err(invalid());
        }
        at = end;
    }
    if at != points.len() {
        return Err(invalid());
    }
    for (id, ordinal) in ids.iter().zip(ordinals.iter()) {
        let Value::Text(id) = id else {
            return Err(invalid());
        };
        if !uuid7(id) {
            return Err(invalid());
        }
        let Value::Int(n) = ordinal else {
            return Err(invalid());
        };
        if *n < 0 || last.is_some_and(|seen| *n <= seen) {
            return Err(invalid());
        }
        last = Some(*n);
    }
    for kind in kinds {
        let Value::Text(kind) = kind else {
            return Err(invalid());
        };
        if kind != "hole" && kind != "polygon_cut" {
            return Err(invalid());
        }
    }
    Ok(())
}
/// Most through holes of one plate and most points of one hole.
const MAX_HOLES: usize = 64;
const MAX_HOLE_POINTS: usize = 1024;

/// The optional member-hole operation records of a loaded beam or column (A04.2): all nine
/// attributes or none, the counts add up to the points, every polygon has at least three
/// in-cap points with every `z` zero, there is one full UUIDv7 and one strictly increasing
/// non-negative ordinal per polygon, each axis point is finite and in cap, each axis direction
/// is a signed unit token, and each extent is `through` (zero range) or a `from < to` range.
/// Winding, placement and fit are the engine's decision on replay, never re-derived here.
fn member_hole_records(prim: &Prim) -> Result<(), Diagnostic> {
    let present = |name: &str| prim.properties.iter().any(|p| p.name == name);
    let names = [
        "steel:mhole:pointsMm",
        "steel:mhole:vertexCounts",
        "steel:mhole:operationIds",
        "steel:mhole:ordinals",
        "steel:mhole:axisPointsMm",
        "steel:mhole:axisDirections",
        "steel:mhole:extents",
        "steel:mhole:extentFromMm",
        "steel:mhole:extentToMm",
    ];
    let declared: Vec<bool> = names.iter().map(|name| present(name)).collect();
    if declared.iter().all(|seen| !seen) {
        return Ok(());
    }
    if !declared.iter().all(|seen| *seen) {
        return Err(invalid());
    }
    let Value::Array(counts) = required(prim, "steel:mhole:vertexCounts", "int[]")? else {
        return Err(invalid());
    };
    let Value::Array(ids) = required(prim, "steel:mhole:operationIds", "string[]")? else {
        return Err(invalid());
    };
    let Value::Array(ordinals) = required(prim, "steel:mhole:ordinals", "int[]")? else {
        return Err(invalid());
    };
    let Value::Array(extents) = required(prim, "steel:mhole:extents", "token[]")? else {
        return Err(invalid());
    };
    let Value::Array(from) = required(prim, "steel:mhole:extentFromMm", "double[]")? else {
        return Err(invalid());
    };
    let Value::Array(to) = required(prim, "steel:mhole:extentToMm", "double[]")? else {
        return Err(invalid());
    };
    if counts.len() > MAX_HOLES
        || counts.len() != ids.len()
        || counts.len() != ordinals.len()
        || counts.len() != extents.len()
        || counts.len() != from.len()
        || counts.len() != to.len()
    {
        return Err(invalid());
    }
    let polygons = points(prim, "steel:mhole:pointsMm")?;
    let axis = points(prim, "steel:mhole:axisPointsMm")?;
    let directions = points(prim, "steel:mhole:axisDirections")?;
    if axis.len() != counts.len() || directions.len() != counts.len() {
        return Err(invalid());
    }
    if polygons.len() > MAX_HOLES * MAX_HOLE_POINTS {
        return Err(limit());
    }
    cap(polygons.iter().flatten().copied(), 1e9)?;
    cap(axis.iter().flatten().copied(), 1e9)?;
    for direction in &directions {
        let unit = direction.iter().filter(|v| **v != 0.0).count() == 1
            && direction.iter().all(|v| matches!(v, 1.0 | -1.0 | 0.0));
        if !unit {
            return Err(invalid());
        }
    }
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
        let hole = polygons.get(at..end).ok_or_else(invalid)?;
        if hole.iter().any(|p| p[2] != 0.0) {
            return Err(invalid());
        }
        at = end;
    }
    if at != polygons.len() {
        return Err(invalid());
    }
    let double = |value: &&Value| match value {
        Value::Number(v) => Some(*v),
        _ => None,
    };
    let mut last: Option<i64> = None;
    for (((id, ordinal), extent), (start, end)) in ids
        .iter()
        .zip(ordinals.iter())
        .zip(extents.iter())
        .zip(from.iter().zip(to.iter()))
    {
        let Value::Text(id) = id else {
            return Err(invalid());
        };
        if !uuid7(id) {
            return Err(invalid());
        }
        let Value::Int(n) = ordinal else {
            return Err(invalid());
        };
        if *n < 0 || last.is_some_and(|seen| *n <= seen) {
            return Err(invalid());
        }
        last = Some(*n);
        let Value::Text(extent) = extent else {
            return Err(invalid());
        };
        let (Some(start), Some(end)) = (double(&start), double(&end)) else {
            return Err(invalid());
        };
        if extent == "through" {
            if start != 0.0 || end != 0.0 {
                return Err(invalid());
            }
        } else if extent == "range" {
            if start >= end {
                return Err(invalid());
            }
            cap([start, end], 1e9)?;
        } else {
            return Err(invalid());
        }
    }
    Ok(())
}

/// The owned plane-cut records of a loaded plate, beam or column (A03, amendment A-2): five
/// parallel attributes, all five or none, of one common length, in ascending ordinals at most
/// 999; each point is finite and within the coordinate cap and each normal is not the zero
/// vector. Whether a plane crosses its element is the engine's decision on replay, never
/// re-derived here. A damaged or half-authored list is invalid, never a default.
pub(super) fn element_cut(prim: &Prim) -> Result<Vec<CutRecord>, Diagnostic> {
    const FIELDS: [&str; 5] = [
        "steel:cut:operationId",
        "steel:cut:ordinal",
        "steel:cut:pointMm",
        "steel:cut:normal",
        "steel:cut:keep",
    ];
    let present = |name: &str| prim.properties.iter().any(|p| p.name == name);
    let count = FIELDS.iter().filter(|name| present(name)).count();
    if count == 0 {
        return Ok(Vec::new());
    }
    if count != FIELDS.len() {
        return Err(invalid());
    }
    let strings = |name: &str| -> Result<Vec<String>, Diagnostic> {
        let Value::Array(values) = required(prim, name, "string[]")? else {
            return Err(invalid());
        };
        values
            .iter()
            .map(|v| match v {
                Value::Text(s) => Ok(s.clone()),
                _ => Err(invalid()),
            })
            .collect()
    };
    let ids = strings(FIELDS[0])?;
    let Value::Array(ordinal_values) = required(prim, FIELDS[1], "int[]")? else {
        return Err(invalid());
    };
    let mut ordinals = Vec::with_capacity(ordinal_values.len());
    for value in ordinal_values {
        let Value::Int(n) = value else {
            return Err(invalid());
        };
        let ordinal = u16::try_from(*n).map_err(|_| invalid())?;
        if ordinal > 999 {
            return Err(invalid());
        }
        if ordinals.last().is_some_and(|last: &u16| *last >= ordinal) {
            return Err(invalid());
        }
        ordinals.push(ordinal);
    }
    let cut_points = points(prim, FIELDS[2])?;
    let normals = points(prim, FIELDS[3])?;
    let Value::Array(keeps) = required(prim, FIELDS[4], "token[]")? else {
        return Err(invalid());
    };
    let keep = |value: &Value| match value {
        Value::Text(s) => KeepSide::parse(s),
        _ => None,
    };
    let keeps = keeps
        .iter()
        .map(|value| keep(value).ok_or_else(invalid))
        .collect::<Result<Vec<_>, Diagnostic>>()?;
    if ids.len() != ordinals.len()
        || cut_points.len() != ordinals.len()
        || normals.len() != ordinals.len()
        || keeps.len() != ordinals.len()
    {
        return Err(invalid());
    }
    let records = ids
        .iter()
        .zip(
            ordinals
                .iter()
                .zip(cut_points.iter().zip(normals.iter().zip(keeps.iter()))),
        )
        .map(|(id, (ordinal, (point, (normal, keep))))| {
            let record = CutRecord {
                operation_id: OpId::parse(id).map_err(|_| invalid())?,
                ordinal: *ordinal,
                point_mm: *point,
                normal: *normal,
                keep: *keep,
            };
            record.validate().map_err(|_| invalid())?;
            Ok(record)
        })
        .collect::<Result<Vec<_>, Diagnostic>>()?;
    Ok(records)
}
fn resource<'a>(
    prims: &'a [Prim],
    digest: ContentDigest,
    kinds: &[&str],
    category: DefinitionCategory,
) -> Result<&'a Prim, Diagnostic> {
    let path = definition_path(digest, category);
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
            let Some((category, digest)) = DefinitionCategory::parse_path(target) else {
                return Err(invalid());
            };
            if target != &definition_path(digest, category)
                || category
                    != if name == "bim:material" {
                        DefinitionCategory::Material
                    } else {
                        DefinitionCategory::Profile
                    }
            {
                return Err(invalid());
            }
            Ok(Some(resource(prims, digest, kinds, category)?))
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
    for (path, kind) in [
        ("/World", "assembly"),
        ("/World/Steel", "group"),
        ("/World/Steel/Parts", "group"),
        ("/World/Steel/Assemblies", "group"),
        ("/World/Steel/Connections", "group"),
        ("/World/Levels", "group"),
        ("/World/Grids", "group"),
        ("/World/References", "group"),
    ] {
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
    let mut operation_ids = std::collections::BTreeSet::new();
    attribute_schema(prims)?;
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
        crate::is_deactivated(p)?;
        if text(p, "bim:geometryMode", "token")? == "reference" {
            if text(p, "bim:elementKind", "token")? != "reference" {
                return Err(unsupported());
            }
            // A foreign wrapper is no storey member: a level record on it is outside D2.
            if relationship(p, "bim:level").is_some_and(|r| !r.targets.is_empty()) {
                return Err(fail(
                    ErrorCode::InvalidArgument,
                    "only a plate, beam or column is assigned to a level",
                )
                .with_rule("DV-REL-003"));
            }
            reference::element(p, prims)?;
            continue;
        }
        if text(p, "bim:geometryMode", "token")? != "parametric" {
            continue;
        }
        if !matches!(
            text(p, "bim:elementKind", "token")?,
            "plate" | "beam" | "column" | "assembly" | "connection" | "level" | "grid"
        ) {
            continue;
        }
        if p.path
            != element_path(
                id,
                ElementCategory::from_kind(text(p, "bim:elementKind", "token")?)
                    .ok_or_else(invalid)?,
            )?
        {
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
        if !matches!(kind, "plate" | "beam" | "column")
            && p.properties.iter().any(|p| p.name == "steel:class")
        {
            return Err(unsupported());
        }
        if matches!(kind, "plate" | "beam" | "column")
            && let Some(prop) = p.properties.iter().find(|a| a.name == "steel:class")
            && (prop.type_name != "int"
                || !matches!(prop.value, Some(Value::Int(v)) if i32::try_from(v).is_ok()))
        {
            return Err(invalid());
        }
        if instance::definition(p, prims)?.is_none() {
            if matches!(kind, "plate" | "beam" | "column") {
                let hole_history = cut_history::read_holes(p)?;
                let member_history = cut_history::read_member_holes(p)?;
                let history = cut_history::read(p)?;
                for id in history
                    .retired
                    .into_iter()
                    .chain(hole_history.retired)
                    .chain(member_history.retired)
                    .chain(cut_history::ids(p, "steel:cut:operationId")?)
                    .chain(cut_history::ids(p, "steel:hole:operationIds")?)
                    .chain(cut_history::ids(p, "steel:mhole:operationIds")?)
                {
                    if !operation_ids.insert(id) {
                        return Err(invalid());
                    }
                }
            }
            match kind {
                "plate" => {
                    // The form token discriminates the stored record: a lofted plate (A11)
                    // carries the three loft attributes and no plate extrusion inputs; an
                    // extruded plate stores neither the token nor the loft attributes.
                    let lofted = match p.properties.iter().find(|a| a.name == "steel:plate:form") {
                        None => {
                            if p.properties.iter().any(|a| {
                                matches!(
                                    a.name.as_str(),
                                    "steel:plate:loftStartMm" | "steel:plate:loftEndMm"
                                )
                            }) {
                                return Err(invalid());
                            }
                            false
                        }
                        Some(_) => {
                            if text(p, "steel:plate:form", "token")? != "lofted" {
                                return Err(invalid());
                            }
                            if p.properties.iter().any(|a| {
                                matches!(
                                    a.name.as_str(),
                                    "steel:plate:contourPointsMm" | "steel:plate:thicknessMm"
                                )
                            }) {
                                return Err(invalid());
                            }
                            true
                        }
                    };
                    if lofted {
                        lofted_plate(
                            &points(p, "steel:plate:loftStartMm")?,
                            &points(p, "steel:plate:loftEndMm")?,
                        )?;
                    } else {
                        plate(
                            &points(p, "steel:plate:contourPointsMm")?,
                            number(p, "steel:plate:thicknessMm")?,
                        )?;
                    }
                    plate_corner_records(p)?;
                    plate_holes(p)?;
                    plate_hole_records(p)?;
                    element_cut(p)?;
                }
                "beam" | "column" => {
                    let axis = points(p, "bim:axisPointsMm")?;
                    if let Some(prop) = p
                        .properties
                        .iter()
                        .find(|v| v.name == "steel:member:pathType")
                    {
                        if prop.type_name != "token"
                            || text(p, "steel:member:pathType", "token")? != "arc"
                            || axis.len() != 3
                        {
                            return Err(invalid());
                        }
                        if p.properties
                            .iter()
                            .any(|v| v.name == "steel:member:rollRad")
                        {
                            return Err(invalid());
                        }
                    }
                    if axis.len() < 2 || axis.len() > MAX_MEMBER_PATH_POINTS {
                        return Err(invalid());
                    }
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
                        // A member pins a section profile, never a material or a fastener.
                        Definition::Material { .. } | Definition::Fastener { .. } => {
                            return Err(invalid());
                        }
                    };
                    // The size reference is the total path length for a polybeam and the one
                    // chord for a straight member; both sum the same first chord.
                    let mut length = 0.0;
                    for (a, b) in axis.iter().zip(axis.iter().skip(1)) {
                        length += norm(difference(*b, *a));
                    }
                    let half_width = width / 2.0;
                    let half_height = height / 2.0;
                    let size =
                        length.max((half_width * half_width + half_height * half_height).sqrt());
                    if size > 1e6 {
                        return Err(limit());
                    }
                    let eps = 0.001f64.max(size / 4194304.0);
                    // Every chord, not only the total, must clear the geometric floor.
                    let mut short = length < eps;
                    for (a, b) in axis.iter().zip(axis.iter().skip(1)) {
                        short |= norm(difference(*b, *a)) < eps;
                    }
                    if short || width < eps || height < eps {
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
                    element_cut(p)?;
                    member_hole_records(p)?;
                }
                // An assembly has no shape inputs and no material of its own; a connection is a weld
                // (A06) or a bolt group (A05) with its own properties and no material either.
                "grid" => {
                    grid::read(p)?;
                    if relationship(p, "bim:material").is_some_and(|r| !r.targets.is_empty()) {
                        return Err(invalid());
                    }
                }
                "assembly" | "connection" | "level" => {
                    if relationship(p, "bim:material").is_some_and(|r| !r.targets.is_empty()) {
                        return Err(invalid());
                    }
                    if kind == "connection" {
                        let connection_type = text(p, "steel:connection:type", "token")?;
                        let has =
                            |prefix: &str| p.properties.iter().any(|v| v.name.starts_with(prefix));
                        let placed = has("steel:bolt:fastener");
                        match connection_type {
                            "weld" if !has("steel:bolt:") => weld_properties(p)?,
                            // An A05 annotation group and an A14.3 group are discriminated by
                            // the definition digest; a mixed property set is never guessed.
                            "bolt_group" if !has("steel:weld:") && placed => {
                                bolt_group_properties(p, prims)?
                            }
                            "bolt_group" if !has("steel:weld:") && !has("steel:bolt:point1Mm") => {
                                bolt_properties(p)?
                            }
                            _ => return Err(invalid().with_rule("DV-ATTR-002")),
                        }
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
    grid::read_grids(prims)?;
    instance::links(prims)?;
    part_of(prims)?;
    main_part(prims)?;
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
        // An assembly, a level or a weld without a path has no body: a Geometry mesh under one
        // is refused, authored or loaded. A grid carries only derived guide meshes (T01.7);
        // only the A15 weld with an authored path carries a derived solid.
        let kind = supported_kind(element);
        let bodyless = matches!(kind, Some("assembly" | "level"))
            || (kind == Some("connection")
                && !element
                    .properties
                    .iter()
                    .any(|v| v.name == "steel:weld:pathMm"));
        if bodyless {
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
            "A16-V01:main-part-on-an-assembly-one-per-assembly-direct-part",
            "A06-V01:weld-properties-and-connects-two-distinct-parts",
            "A05-V01:bolt-properties-and-connects-two-distinct-parts",
            "D2-V01:level-elevation-and-level-kinds-cardinality-and-dangling",
            "T01-V01:grid-arrays-expression-frame-and-project-axis-identities",
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
            "plate" | "beam" | "column" | "assembly" | "connection" | "level" | "grid"
        )
    })
}

/// The values of an A09 mark: a series of at most 16 letters, digits, `_` and `-`, a number in
/// `0..=i32::MAX` and a non-empty printable-ASCII approval record of at most 512 bytes.
fn mark_values(series: &str, number: i64, basis: &str) -> Result<(), Diagnostic> {
    if series.chars().count() > MARK_MAX_SERIES
        || !series
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        || !(0..=i64::from(i32::MAX)).contains(&number)
        || basis.is_empty()
        || basis.len() > MARK_MAX_BASIS
        || !basis.bytes().all(|b| (0x20..=0x7e).contains(&b))
    {
        return Err(invalid());
    }
    Ok(())
}

/// Checks a mark about to be authored.
pub(super) fn mark(m: &Mark) -> Result<(), Diagnostic> {
    mark_values(&m.series, i64::from(m.number), &m.basis)
}

/// A09 marks on loaded or authored elements: only a plate, beam, column or assembly carries one,
/// and it carries all three properties with their exact types, or none. A partial or malformed
/// record is refused, never read as "unnumbered".
fn marks(prims: &[Prim]) -> Result<(), Diagnostic> {
    for p in prims {
        let Some(kind) = supported_kind(p) else {
            continue;
        };
        let series = optional(p, MARK_SERIES, "string")?;
        let number = optional(p, MARK_NUMBER, "int")?;
        let basis = optional(p, MARK_BASIS, "string")?;
        match (series, number, basis) {
            (None, None, None) => {}
            (Some(Value::Text(series)), Some(number), Some(Value::Text(basis))) => {
                if !matches!(kind, "plate" | "beam" | "column" | "assembly") {
                    return Err(unsupported());
                }
                let number = match number {
                    Value::Int(n) => *n,
                    Value::Uint(n) => i64::try_from(*n).map_err(|_| invalid())?,
                    _ => return Err(invalid()),
                };
                mark_values(series, number, basis)?;
            }
            _ => return Err(invalid()),
        }
    }
    Ok(())
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
                    )
                    .with_rule("DV-REL-003"));
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
                )
                .with_rule("DV-REL-004"));
            }
        };
        if !matches!(kind, "plate" | "beam" | "column") {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "only a plate, beam or column is assigned to a level",
            )
            .with_rule("DV-REL-003"));
        }
        match elements.get(target) {
            Some((t, "level")) if t.active => {}
            Some((t, _)) if !t.active => {
                return Err(fail(
                    ErrorCode::DanglingRelationship,
                    "level target is not active",
                )
                .with_rule("DV-REL-001"));
            }
            Some(_) => {
                return Err(
                    fail(ErrorCode::InvalidArgument, "level target is not a level")
                        .with_rule("DV-REL-003"),
                );
            }
            None if prims
                .iter()
                .any(|p| p.path == target && p.element_id.is_some()) =>
            {
                return Err(fail(
                    ErrorCode::InvalidArgument,
                    "level target is not a supported owned level",
                )
                .with_rule("DV-REL-003"));
            }
            None => {
                return Err(
                    fail(ErrorCode::DanglingRelationship, "level target unavailable")
                        .with_rule("DV-REL-001"),
                );
            }
        }
    }
    Ok(())
}

/// The A15 weld path rules shared by the loaded-content and candidate checks: 2 to 1024 points,
/// finite and within the coordinate cap, consecutive points at least the minimum apart.
fn weld_path_points(points: &[[f64; 3]]) -> Result<(), Diagnostic> {
    if !(2..=1024).contains(&points.len()) {
        return Err(invalid());
    }
    for point in points {
        finite(*point)?;
        cap(*point, 1e9)?;
    }
    for pair in points.windows(2) {
        let [from, to] = pair else {
            continue;
        };
        let [fx, fy, fz] = from;
        let [tx, ty, tz] = to;
        let (dx, dy, dz) = (tx - fx, ty - fy, tz - fz);
        if (dx * dx + dy * dy + dz * dz) < MIN_WELD_SEGMENT_MM * MIN_WELD_SEGMENT_MM {
            return Err(invalid());
        }
    }
    Ok(())
}

/// The weld properties of a loaded `connection` (A06): every one is authored, with a supported
/// token, and the size is finite in `(0, 1000]`. A connection without them is not a weld this
/// slice can read, so it is refused rather than guessed. The optional geometry attributes (A15)
/// are validated as a whole when the path is present and refused in part.
fn weld_properties(p: &Prim) -> Result<(), Diagnostic> {
    weld_tokens(
        text(p, "steel:weld:type", "token")?,
        text(p, "steel:weld:location", "token")?,
        text(p, "steel:weld:side", "token")?,
    )?;
    weld_size(number(p, "steel:weld:sizeMm")?)?;
    let path = p.properties.iter().any(|v| v.name == "steel:weld:pathMm");
    let all_around = p
        .properties
        .iter()
        .any(|v| v.name == "steel:weld:allAround");
    let contour = p.properties.iter().any(|v| v.name == "steel:weld:contour");
    if path {
        weld_path_points(&points(p, "steel:weld:pathMm")?)?;
        if all_around && !matches!(text(p, "steel:weld:allAround", "token")?, "true" | "false") {
            return Err(invalid());
        }
        if contour
            && !matches!(
                text(p, "steel:weld:contour", "token")?,
                "none" | "flush" | "concave" | "convex"
            )
        {
            return Err(invalid());
        }
    } else if all_around || contour {
        // Symbol data without the path it belongs to has no supported meaning.
        return Err(invalid());
    }
    Ok(())
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

/// A05 bolt properties of a loaded `connection`: every one is authored, the diameter is finite in
/// `(0, 1000]`, the count is 1 to 512, the pitch is finite in `(0, 1e6]`, the origin is finite
/// within the coordinate cap and the direction is a finite unit vector. A connection without them
/// is not a bolt group this slice can read, so it is refused rather than guessed.
fn bolt_properties(p: &Prim) -> Result<(), Diagnostic> {
    // A stray A14.3 attribute on an A05 group is mixed content this version cannot read.
    if p.properties.iter().any(|v| {
        matches!(
            v.name.as_str(),
            "steel:bolt:fastener"
                | "steel:bolt:point1Mm"
                | "steel:bolt:point2Mm"
                | "steel:bolt:planeNormalMm"
                | "steel:bolt:rotationRad"
                | "steel:bolt:onPlaneMm"
                | "steel:bolt:depthMm"
                | "steel:bolt:offsetStartMm"
                | "steel:bolt:spacingMode"
                | "steel:bolt:spacingValueMm"
                | "steel:bolt:spacingsMm"
                | "steel:bolt:lengthMode"
                | "steel:bolt:lengthMm"
                | "steel:bolt:washerUnderHead"
                | "steel:bolt:washerUnderNut"
        )
    }) {
        return Err(invalid());
    }
    let diameter = number(p, "steel:bolt:diameterMm")?;
    finite([diameter])?;
    if diameter <= 0.0 {
        return Err(invalid());
    }
    cap([diameter], 1000.0)?;
    let count = match required(p, "steel:bolt:count", "int")? {
        Value::Int(n) => *n,
        _ => return Err(invalid()),
    };
    if !(1..=512).contains(&count) {
        return Err(invalid());
    }
    let pitch = number(p, "steel:bolt:pitchMm")?;
    finite([pitch])?;
    if pitch <= 0.0 {
        return Err(invalid());
    }
    cap([pitch], 1e6)?;
    let origin = bolt_vec3(p, "steel:bolt:originMm")?;
    finite(origin)?;
    cap(origin, 1e9)?;
    let direction = bolt_vec3(p, "steel:bolt:directionMm")?;
    finite(direction)?;
    let squared = direction.iter().map(|v| v * v).sum::<f64>();
    if (squared - 1.0).abs() > 1e-6 {
        return Err(invalid());
    }
    Ok(())
}

/// One scalar `double3` property of a prim.
fn bolt_vec3(p: &Prim, name: &str) -> Result<[f64; 3], Diagnostic> {
    let Value::Array(values) = required(p, name, "double3")? else {
        return Err(invalid());
    };
    match values.as_slice() {
        [Value::Number(x), Value::Number(y), Value::Number(z)] => Ok([*x, *y, *z]),
        _ => Err(invalid()),
    }
}

/// One `double[]` property of a prim.
fn bolt_f64s(p: &Prim, name: &str) -> Result<Vec<f64>, Diagnostic> {
    let Value::Array(values) = required(p, name, "double[]")? else {
        return Err(invalid());
    };
    values
        .iter()
        .map(|v| match v {
            Value::Number(n) => Ok(*n),
            _ => Err(invalid()),
        })
        .collect()
}

/// The `connects` identities of an A14.3 group: 2 to 1024 full UUIDv7s, distinct, never the
/// group itself (`DV-REL-004`, `DV-REL-006`, `DV-REL-007`).
fn bolt_group_connects(connects: &[String], self_id: &str) -> Result<(), Diagnostic> {
    if connects.len() < 2 || connects.len() > MAX_RELATIONSHIP_TARGETS {
        return Err(fail(
            ErrorCode::DependencyConflict,
            "a bolt group connects 2 to 1024 parts",
        )
        .with_rule("DV-REL-004"));
    }
    for part in connects {
        if !uuid7(part) {
            return Err(fail(ErrorCode::InvalidId, "invalid full element identity")
                .with_rule("DV-BASE-001"));
        }
        if part == self_id {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "a bolt group cannot connect itself",
            )
            .with_rule("DV-REL-006"));
        }
    }
    for (index, part) in connects.iter().enumerate() {
        if connects.iter().skip(index + 1).any(|other| other == part) {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "a bolt group connects distinct parts",
            )
            .with_rule("DV-REL-007"));
        }
    }
    Ok(())
}

/// The stored values of an A14.3 group (bolt amendment section 4): two distinct points within
/// the coordinate cap, a unit plane normal, finite position values in range, and a spacing and
/// length record consistent with its mode. The engine's crossing, grip and length decisions
/// have already run; this re-checks the stored form so a damaged or forged record is refused,
/// never guessed.
fn bolt_group_values(b: &BoltGroup) -> Result<(), Diagnostic> {
    finite(b.point1_mm)?;
    finite(b.point2_mm)?;
    cap(b.point1_mm, 1e9)?;
    cap(b.point2_mm, 1e9)?;
    let squared = b
        .point2_mm
        .iter()
        .zip(b.point1_mm)
        .map(|(q, p)| {
            let d = q - p;
            d * d
        })
        .sum::<f64>();
    if squared < 1e-6 {
        return Err(invalid());
    }
    finite(b.plane_normal_mm)?;
    let unit = b.plane_normal_mm.iter().map(|v| v * v).sum::<f64>();
    if (unit - 1.0).abs() > 1e-6 {
        return Err(invalid());
    }
    finite([b.rotation_rad, b.on_plane_mm, b.depth_mm, b.offset_start_mm])?;
    cap([b.on_plane_mm, b.depth_mm], 1e6)?;
    if !(0.0..=1e6).contains(&b.offset_start_mm) || !(0.0..=1e6).contains(&b.length_mm) {
        return Err(invalid());
    }
    // The authoritative spacings give 2 to 512 fasteners, positive and capped.
    if b.spacings_mm.is_empty() || b.spacings_mm.len() >= 512 {
        return Err(invalid());
    }
    finite(b.spacings_mm.iter().copied())?;
    cap(b.spacings_mm.iter().copied(), 1e6)?;
    if b.spacings_mm.iter().any(|s| *s <= 0.0) {
        return Err(invalid());
    }
    // The stored pattern agrees with the authoritative list it expanded to: an explicit list
    // carries no value, an equal pitch repeats its value, and an offset-end pattern re-expands
    // from the stored points within a micron.
    match b.spacing_mode.as_str() {
        "spacings" if b.spacing_value_mm == 0.0 => {}
        "pitch" => {
            if b.spacing_value_mm <= 0.0 || b.spacing_value_mm > 1e6 {
                return Err(invalid());
            }
            if b.spacings_mm.iter().any(|s| *s != b.spacing_value_mm) {
                return Err(invalid());
            }
        }
        "offset_end" => {
            if !(0.0..=1e6).contains(&b.spacing_value_mm) {
                return Err(invalid());
            }
            let pitch = (squared.sqrt() - b.offset_start_mm - b.spacing_value_mm)
                / b.spacings_mm.len() as f64;
            let agrees = |stored: f64| (stored - pitch).abs() <= stored.abs() * 1e-9 + 1e-9;
            if pitch <= 0.0 || !b.spacings_mm.iter().all(|s| agrees(*s)) {
                return Err(invalid());
            }
        }
        _ => return Err(invalid()),
    }
    if !matches!(b.length_mode.as_str(), "auto" | "fixed") {
        return Err(invalid());
    }
    if !matches!(b.hole_mode.as_str(), "generate" | "none") {
        return Err(invalid());
    }
    // A generating group may carry its own clearance; a quiet one carries none.
    if matches!(b.hole_mode.as_str(), "none") && b.hole_clearance_mm.is_some() {
        return Err(invalid());
    }
    if let Some(clearance) = b.hole_clearance_mm {
        finite([clearance])?;
        cap([clearance], 1e6)?;
    }
    Ok(())
}

/// Consistency of a stored A14.3 group with its pinned definition (bolt amendment sections 2
/// and 4): the resolved length is one of the definition's standard lengths, a washer switch is
/// true only for a washer the definition carries, and under a nut only for a bolt.
fn bolt_group_against(spec: &FastenerSpec, b: &BoltGroup) -> Result<(), Diagnostic> {
    if !spec.lengths_mm.contains(&b.length_mm) {
        return Err(invalid());
    }
    if b.washer_under_head && spec.washer.is_none() {
        return Err(invalid());
    }
    if b.washer_under_nut && (spec.washer.is_none() || spec.kind != FastenerKind::Bolt) {
        return Err(invalid());
    }
    Ok(())
}

/// The fastener definition a group places: one this batch pins as a resource, or the pinned
/// definition the model already holds. Anything else is a dangling reference.
fn fastener_definition(
    input: &Prepare,
    prims: &[Prim],
    digest: ContentDigest,
) -> Result<FastenerSpec, Diagnostic> {
    if let Some(found) = input.resources.iter().find(|r| r.digest == digest) {
        return match &found.definition {
            Definition::Fastener { spec } => Ok(spec.clone()),
            _ => Err(invalid()),
        };
    }
    pinned_fastener(prims, digest)
}

/// The pinned fastener definition of a stored digest, re-admitted from its canonical bytes.
fn pinned_fastener(prims: &[Prim], digest: ContentDigest) -> Result<FastenerSpec, Diagnostic> {
    let p = resource(prims, digest, &["fastener"], DefinitionCategory::Fastener)?;
    match read_definition(p, "fastener")? {
        Definition::Fastener { spec } => Ok(spec),
        _ => Err(corrupt()),
    }
}

/// The fifteen A14.3 bolt-group attributes of a loaded `connection` (bolt amendment section 4):
/// every one is authored and never mixed with the five A05 attributes; the points, the normal
/// and the spacing and length records follow their stored rules; and the digest resolves to a
/// pinned fastener definition the stored record agrees with. A connection without them is not a
/// group this slice can read, so it is refused rather than guessed.
fn bolt_group_properties(p: &Prim, prims: &[Prim]) -> Result<(), Diagnostic> {
    // A stray A05 attribute on an A14.3 group is mixed content this version cannot read.
    if p.properties.iter().any(|v| {
        matches!(
            v.name.as_str(),
            "steel:bolt:diameterMm"
                | "steel:bolt:count"
                | "steel:bolt:pitchMm"
                | "steel:bolt:originMm"
                | "steel:bolt:directionMm"
        )
    }) {
        return Err(invalid());
    }
    let digest = text(p, "steel:bolt:fastener", "string")?
        .parse::<ContentDigest>()
        .map_err(|_| invalid())?;
    let point1 = bolt_vec3(p, "steel:bolt:point1Mm")?;
    let point2 = bolt_vec3(p, "steel:bolt:point2Mm")?;
    let normal = bolt_vec3(p, "steel:bolt:planeNormalMm")?;
    let rotation = number(p, "steel:bolt:rotationRad")?;
    let on_plane = number(p, "steel:bolt:onPlaneMm")?;
    let depth = number(p, "steel:bolt:depthMm")?;
    let offset_start = number(p, "steel:bolt:offsetStartMm")?;
    let spacing_mode = text(p, "steel:bolt:spacingMode", "token")?.to_owned();
    let spacing_value = number(p, "steel:bolt:spacingValueMm")?;
    let spacings = bolt_f64s(p, "steel:bolt:spacingsMm")?;
    let length_mode = text(p, "steel:bolt:lengthMode", "token")?.to_owned();
    let length = number(p, "steel:bolt:lengthMm")?;
    let washer_under_head = matches!(
        required(p, "steel:bolt:washerUnderHead", "bool")?,
        Value::Bool(true)
    );
    let washer_under_nut = matches!(
        required(p, "steel:bolt:washerUnderNut", "bool")?,
        Value::Bool(true)
    );
    let hole_mode = text(p, "steel:bolt:holeMode", "token")?.to_owned();
    let hole_clearance_mm = optional_number(p, "steel:bolt:holeClearanceMm")?;
    let record = BoltGroup {
        connects: Vec::new(),
        fastener: digest,
        point1_mm: point1,
        point2_mm: point2,
        plane_normal_mm: normal,
        rotation_rad: rotation,
        on_plane_mm: on_plane,
        depth_mm: depth,
        offset_start_mm: offset_start,
        spacing_mode,
        spacing_value_mm: spacing_value,
        spacings_mm: spacings,
        length_mode,
        length_mm: length,
        washer_under_head,
        washer_under_nut,
        hole_mode,
        hole_clearance_mm,
    };
    bolt_group_values(&record)?;
    bolt_group_against(&pinned_fastener(prims, digest)?, &record)
}

/// A06 and A05 `connects` records (domain vocabulary section 5.1): only a connection — a weld or
/// a bolt group — carries one, with exactly two distinct targets (`DV-REL-004`, `DV-REL-007`),
/// never itself (`DV-REL-006`), each an active owned plate, beam or column (`DV-REL-001`
/// dangling, `DV-REL-003` kind). An assembly is a valid target in the vocabulary but not for a
/// weld or a bolt group in this slice.
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
                    "a connection connects exactly two parts",
                )
                .with_rule("DV-REL-004"));
            }
            continue;
        };
        if kind != "connection" {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "only a connection carries a connects record",
            )
            .with_rule("DV-REL-003"));
        }
        // An A14.3 bolt group carries 2 to 1024 targets (the vocabulary's own range); a weld
        // and an A05 group carry exactly two.
        let limit = if p.properties.iter().any(|v| v.name == "steel:bolt:fastener") {
            MAX_RELATIONSHIP_TARGETS
        } else {
            2
        };
        if r.targets.len() < 2 || r.targets.len() > limit {
            return Err(fail(
                ErrorCode::DependencyConflict,
                "a connection connects exactly two parts",
            )
            .with_rule("DV-REL-004"));
        }
        for (index, first) in r.targets.iter().enumerate() {
            if r.targets.iter().skip(index + 1).any(|other| other == first) {
                return Err(fail(
                    ErrorCode::InvalidArgument,
                    "a connection connects two distinct parts",
                )
                .with_rule("DV-REL-007"));
            }
        }
        for target in &r.targets {
            if target == path {
                return Err(fail(
                    ErrorCode::InvalidArgument,
                    "a connection cannot connect itself",
                ));
            }
            match elements.get(target.as_str()) {
                Some((t, _)) if !t.active => {
                    return Err(fail(
                        ErrorCode::DanglingRelationship,
                        "connects target is not active",
                    )
                    .with_rule("DV-REL-001"));
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
                    )
                    .with_rule("DV-REL-003"));
                }
                None => {
                    return Err(fail(
                        ErrorCode::DanglingRelationship,
                        "connects target unavailable",
                    )
                    .with_rule("DV-REL-001"));
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
                )
                .with_rule("DV-REL-004"));
            }
        };
        if target == path {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "an element cannot be a part of itself",
            )
            .with_rule("DV-REL-006"));
        }
        match elements.get(target) {
            Some((t, "assembly")) if t.active => {}
            Some((t, _)) if !t.active => {
                return Err(fail(
                    ErrorCode::DanglingRelationship,
                    "part-of target is not active",
                )
                .with_rule("DV-REL-001"));
            }
            Some(_) => {
                return Err(fail(
                    ErrorCode::InvalidArgument,
                    "part-of target is not an assembly",
                )
                .with_rule("DV-REL-003"));
            }
            None if prims
                .iter()
                .any(|p| p.path == target && p.element_id.is_some()) =>
            {
                return Err(fail(
                    ErrorCode::InvalidArgument,
                    "part-of target is not a supported owned assembly",
                )
                .with_rule("DV-REL-003"));
            }
            None => {
                return Err(fail(
                    ErrorCode::DanglingRelationship,
                    "part-of target unavailable",
                )
                .with_rule("DV-REL-001"));
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
                    )
                    .with_rule("DV-REL-005"));
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

/// A16.1 `mainPart` records: only an assembly carries `bim:mainPart`, with at most one target
/// (`DV-REL-004`), which must be an active owned element whose own `part-of` record names this
/// assembly — a direct part, whatever its supported kind (`DV-REL-001` dangling, `DV-REL-003`
/// kind). A record whose part is not a member is refused, never read as "no main part".
fn main_part(prims: &[Prim]) -> Result<(), Diagnostic> {
    let elements: BTreeMap<&str, (&Prim, &str)> = prims
        .iter()
        .filter_map(|p| supported_kind(p).map(|k| (p.path.as_str(), (p, k))))
        .collect();
    for &(p, kind) in elements.values() {
        let Some(r) = relationship(p, "bim:mainPart") else {
            continue;
        };
        let target = match r.targets.as_slice() {
            [] => continue,
            [target] => target.as_str(),
            _ => {
                return Err(fail(
                    ErrorCode::DependencyConflict,
                    "an assembly has at most one main part",
                )
                .with_rule("DV-REL-004"));
            }
        };
        if kind != "assembly" {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "only an assembly carries a main-part record",
            )
            .with_rule("DV-REL-003"));
        }
        match elements.get(target) {
            // The main part must be a direct member: its own part-of record names this assembly.
            Some((t, _)) if t.active && part_of_target(t, p.path.as_str()) => {}
            Some((t, _)) if !t.active => {
                return Err(fail(
                    ErrorCode::DanglingRelationship,
                    "main-part target is not active",
                )
                .with_rule("DV-REL-001"));
            }
            Some(_) => {
                return Err(fail(
                    ErrorCode::InvalidArgument,
                    "main-part target is not a direct part of the assembly",
                )
                .with_rule("DV-REL-003"));
            }
            None if prims
                .iter()
                .any(|p| p.path == target && p.element_id.is_some()) =>
            {
                return Err(fail(
                    ErrorCode::InvalidArgument,
                    "main-part target is not a supported owned part",
                )
                .with_rule("DV-REL-003"));
            }
            None => {
                return Err(fail(
                    ErrorCode::DanglingRelationship,
                    "main-part target unavailable",
                )
                .with_rule("DV-REL-001"));
            }
        }
    }
    Ok(())
}

/// Whether the loaded `part-of` record of `part` names exactly `assembly` (an explicit empty
/// record is not a membership).
fn part_of_target(part: &Prim, assembly: &str) -> bool {
    relationship(part, "bim:partOf")
        .is_some_and(|r| matches!(r.targets.as_slice(), [target] if target.as_str() == assembly))
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
        // A schema change targets no element: it reads and writes the schema as a whole, so two
        // schema edits conflict and neither conflicts with an element edit (I04). The candidate
        // snapshot re-validates the whole scope.
        if matches!(
            change,
            Change::AttrDefine { .. } | Change::AttrUndefine { .. }
        ) {
            writes.insert(Footprint {
                identity: super::ATTR_SCHEMA_IDENTITY.to_owned(),
                group: Group::Lifecycle,
            });
            continue;
        }
        let (id, _) = change.key();
        let path = change_path(id, input, prims)?;
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
                return Err(
                    fail(ErrorCode::DuplicateId, "element already exists").with_rule("DV-BASE-002")
                );
            }
            Change::Create(_) | Change::AttachReference(_) => (),
            // A mesh or part-of record of an element created in the same batch; the candidate
            // snapshot re-validates it as a whole.
            Change::Mesh(_)
            | Change::PartClass { .. }
            | Change::PartOf { .. }
            | Change::Level { .. }
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
                    Change::Contour { .. }
                    | Change::Thickness { .. }
                    | Change::Corners { .. }
                    | Change::Bends { .. } => kind == "plate",
                    Change::HoleRecords { .. } => kind == "plate",
                    Change::MemberHoles { .. } => matches!(kind, "beam" | "column"),
                    Change::Axis { .. } | Change::Roll { .. } | Change::Profile { .. } => {
                        matches!(kind, "beam" | "column")
                    }
                    Change::PartClass { .. } => matches!(kind, "plate" | "beam" | "column"),
                    Change::Grid { .. } => kind == "grid",
                    Change::BoltGroup { .. } => kind == "connection",
                    Change::Mesh(m) if kind == "grid" => m.quantities.is_none(),
                    Change::ReferenceUnits { .. } => wrapper,
                    // A rename and a move are the only edits a wrapper takes as an element; an
                    // assembly can be renamed, deleted and be a part, never moved (identity pose).
                    Change::Placement { .. } => {
                        matches!(kind, "plate" | "beam" | "column" | "reference" | "grid")
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
                                | "grid"
                        )
                    }
                    Change::Delete { .. } => {
                        matches!(
                            kind,
                            "plate"
                                | "beam"
                                | "column"
                                | "assembly"
                                | "connection"
                                | "level"
                                | "grid"
                                | "construction"
                                | "annotation"
                        )
                    }
                    Change::PartOf { .. } => {
                        matches!(kind, "plate" | "beam" | "column" | "assembly")
                    }
                    // The main-part record lives on the assembly; only it is a source.
                    Change::MainPart { .. } => kind == "assembly",
                    Change::Level { .. } => matches!(kind, "plate" | "beam" | "column"),
                    Change::Mark { .. } => {
                        matches!(kind, "plate" | "beam" | "column" | "assembly")
                    }
                    // A user attribute is carried by any element kind, including a wrapper.
                    Change::AttrSet { .. } => true,
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
            // A mark is stored with the label's group: it conflicts with a rename or another
            // mark of the element and with nothing derived.
            Change::Mark { .. } => (&[Group::Lifecycle, Group::Label], &[Group::Label]),
            // A value lives with its element and conflicts only with another change of the
            // element's attributes; no derived result depends on a user attribute.
            Change::AttrSet { .. } => {
                (&[Group::Lifecycle, Group::Attributes], &[Group::Attributes])
            }
            Change::PartClass { .. } | Change::Label { .. } => {
                (&[Group::Lifecycle, Group::Label], &[Group::Label])
            }
            Change::Grid { .. } => (
                &[Group::Lifecycle, Group::ShapeInputs, Group::Placement],
                &[Group::ShapeInputs],
            ),
            // The complete authored inputs and the `connects` record are replaced as a whole
            // (A14.3); a connection derives no body, so no derived group is written.
            Change::BoltGroup { .. } => (
                &[Group::Lifecycle, Group::ShapeInputs, Group::Relationship],
                &[Group::ShapeInputs, Group::Relationship],
            ),
            Change::Material { .. } => (
                &[Group::Lifecycle, Group::Derived],
                &[Group::Material, Group::Relationship, Group::Derived],
            ),
            Change::Placement { .. } => (
                &[Group::Lifecycle, Group::ShapeInputs, Group::Placement],
                &[Group::Placement, Group::Derived],
            ),
            Change::Mesh(_) => (&[Group::Lifecycle, Group::ShapeInputs], &[Group::Derived]),
            Change::PartOf { .. } | Change::MainPart { .. } | Change::Level { .. } => (
                &[Group::Lifecycle, Group::Relationship],
                &[Group::Relationship],
            ),
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
                return Err(fail(ErrorCode::InvalidId, "invalid full element identity")
                    .with_rule("DV-BASE-001"));
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
                return Err(fail(ErrorCode::InvalidId, "invalid full element identity")
                    .with_rule("DV-BASE-001"));
            }
            // The level's existence and kind decide acceptance; it is only read.
            reads.insert(Footprint {
                identity: target.clone(),
                group: Group::Lifecycle,
            });
        }
        if let Change::MainPart { part, .. } = change {
            if !uuid7(part) {
                return Err(fail(ErrorCode::InvalidId, "invalid full element identity")
                    .with_rule("DV-BASE-001"));
            }
            // The part's existence and its membership decide acceptance: only a direct part of
            // the assembly can be its main part, so its relationships are read too.
            for group in [Group::Lifecycle, Group::Relationship] {
                reads.insert(Footprint {
                    identity: part.clone(),
                    group,
                });
            }
        }
        if !matches!(
            change,
            Change::PartClass { .. }
                | Change::Label { .. }
                | Change::Create(_)
                | Change::Delete { .. }
                | Change::PartOf { .. }
                | Change::MainPart { .. }
                | Change::Level { .. }
                | Change::Mark { .. }
        ) && let Some(p) = existing
        {
            for target in p
                .relationships
                .iter()
                .filter(|r| matches!(r.name.as_str(), "bim:profile" | "bim:material"))
                .flat_map(|r| &r.targets)
            {
                let (_, digest) = DefinitionCategory::parse_path(target).ok_or_else(invalid)?;
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
        if prims.iter().any(|p| {
            p.path == definition_path(resource.digest, definition_category(&resource.definition))
        }) {
            let (kind, category) = match &resource.definition {
                Definition::RectangleProfile { .. } => ("rectangle", DefinitionCategory::Profile),
                Definition::SectionProfile { shape, .. } => {
                    (shape.token(), DefinitionCategory::Profile)
                }
                Definition::Material { .. } => ("material", DefinitionCategory::Material),
                Definition::Fastener { .. } => ("fastener", DefinitionCategory::Fastener),
            };
            self::resource(prims, resource.digest, &[kind], category)?;
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
                        // A parsed side is one of three closed tokens; nothing to bound.
                        thickness_side: _,
                        corners,
                        bends,
                        holes,
                        hole_records,
                        cuts,
                    } => {
                        count_points(contour_mm, b)?;
                        cap(contour_mm.iter().flatten().copied(), 1e9)?;
                        cap([*thickness_mm], 1e6)?;
                        // Corner treatments (A12.2): at most one per vertex, each naming a
                        // vertex of this contour with bounded values; whether the corner
                        // really turns and the treatments fit the edges are the engine's.
                        if corners.len() > contour_mm.len() {
                            return Err(limit());
                        }
                        let mut seen = Vec::new();
                        for corner in corners {
                            if corner.vertex >= contour_mm.len() || seen.contains(&corner.vertex) {
                                return Err(invalid());
                            }
                            seen.push(corner.vertex);
                            let values = match corner.treatment {
                                CornerTreatment::Chamfer { d1_mm, d2_mm } => [d1_mm, d2_mm],
                                CornerTreatment::Round { radius_mm } => [radius_mm, radius_mm],
                            };
                            if values.iter().any(|v| *v <= 0.0) {
                                return Err(invalid());
                            }
                            cap(values, 1e9)?;
                        }
                        if holes.len() + hole_records.len() > MAX_HOLES {
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
                        for record in hole_records {
                            // Record polygons are planar XY pairs by type; winding, placement
                            // and clearance are the engine's replay decision.
                            if record.polygon_mm.len() > MAX_HOLE_POINTS
                                || record.polygon_mm.len() < 3
                            {
                                return Err(invalid());
                            }
                            cap(record.polygon_mm.iter().flatten().copied(), 1e9)?;
                        }
                        cut_values(cuts)?;
                        // Bend records (A10): structural bounds only — finiteness and caps for
                        // the line points and radii, a finite angle inside (-pi, pi); whether
                        // the records form one supported bent body is the engine's replay
                        // decision.
                        if bends.len() > MAX_BENDS {
                            return Err(limit());
                        }
                        for bend in bends {
                            let (lines, radii, angle): ([[f64; 3]; 2], &[f64], Option<f64>) =
                                match bend.spec {
                                    BendSpec::Fold {
                                        a_mm,
                                        b_mm,
                                        radius_mm,
                                        angle_rad,
                                    } => ([a_mm, b_mm], &[radius_mm][..], Some(angle_rad)),
                                    BendSpec::Roll {
                                        a_mm,
                                        b_mm,
                                        radius_mm,
                                        radius_b_mm,
                                    } => (
                                        [a_mm, b_mm],
                                        match radius_b_mm {
                                            Some(rb) => &[radius_mm, rb][..],
                                            None => &[radius_mm][..],
                                        },
                                        None,
                                    ),
                                };
                            cap(lines.iter().flatten().copied(), 1e9)?;
                            if radii.iter().any(|v| *v <= 0.0) {
                                return Err(invalid());
                            }
                            cap(radii.iter().copied(), 1e9)?;
                            // A cone with equal radii is a cylinder, and the stored encoding
                            // keeps one radius for a cylinder: equal radii would read back as
                            // a cylinder and refuse their own sector blank.
                            if let BendSpec::Roll {
                                radius_mm,
                                radius_b_mm: Some(radius_b),
                                ..
                            } = bend.spec
                                && radius_b == radius_mm
                            {
                                return Err(invalid());
                            }
                            if let Some(angle) = angle
                                && (!angle.is_finite()
                                    || angle == 0.0
                                    || angle.abs() >= core::f64::consts::PI)
                            {
                                return Err(invalid());
                            }
                        }
                    }
                    Shape::LoftedPlate { start_mm, end_mm } => {
                        // Both contours of a lofted plate (A11): the shared contour bounds;
                        // the equal vertex count, floor, winding and planarity are the
                        // engine's, and the certification refuses a twisted correspondence.
                        for contour in [start_mm, end_mm] {
                            count_points(contour, b)?;
                            cap(contour.iter().flatten().copied(), 1e9)?;
                        }
                    }
                    Shape::Member {
                        curved,
                        axis_mm,
                        roll_rad,
                        profile,
                        placement,
                        cuts,
                        ..
                    } => {
                        if axis_mm.len() < 2 || axis_mm.len() > MAX_MEMBER_PATH_POINTS {
                            return Err(invalid());
                        }
                        if *curved && (axis_mm.len() != 3 || roll_rad.is_some()) {
                            return Err(invalid());
                        }
                        cap(axis_mm.iter().flatten().copied(), 1e9)?;
                        if let Some(n) = roll_rad {
                            cap([*n], std::f64::consts::TAU)?;
                        }
                        if let Some(p) = placement {
                            cap(
                                [
                                    p.plane_offset_mm,
                                    p.depth_offset_mm,
                                    p.start_offset_mm,
                                    p.end_offset_mm,
                                ],
                                1e6,
                            )?;
                        }
                        cut_values(cuts)?;
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
                        if let Some(geometry) = &w.geometry {
                            weld_path_points(&geometry.points_mm)?;
                            if !matches!(
                                geometry.contour.as_str(),
                                "none" | "flush" | "concave" | "convex"
                            ) {
                                return Err(invalid());
                            }
                        }
                        let [main, secondary] = &w.connects;
                        if !uuid7(main) || !uuid7(secondary) {
                            return Err(fail(
                                ErrorCode::InvalidId,
                                "invalid full element identity",
                            )
                            .with_rule("DV-BASE-001"));
                        }
                        if main == secondary || main == &e.id || secondary == &e.id {
                            return Err(fail(
                                ErrorCode::InvalidArgument,
                                "a weld connects two distinct other parts",
                            )
                            .with_rule(if main == secondary {
                                "DV-REL-007"
                            } else {
                                "DV-REL-006"
                            }));
                        }
                        // The parts' existence and kinds decide acceptance; they are only read.
                        for part in [main, secondary] {
                            reads.insert(Footprint {
                                identity: part.clone(),
                                group: Group::Lifecycle,
                            });
                        }
                    }
                    Shape::Bolt(b) => {
                        if e.material.is_some()
                            || e.placement.translation_mm != [0.0; 3]
                            || e.placement.orientation_xyzw != [0.0, 0.0, 0.0, 1.0]
                        {
                            return Err(unsupported());
                        }
                        finite([b.diameter_mm])?;
                        if b.diameter_mm <= 0.0 {
                            return Err(invalid());
                        }
                        cap([b.diameter_mm], 1000.0)?;
                        if b.count == 0 || b.count > 512 {
                            return Err(invalid());
                        }
                        finite([b.pitch_mm])?;
                        if b.pitch_mm <= 0.0 {
                            return Err(invalid());
                        }
                        cap([b.pitch_mm], 1e6)?;
                        finite(b.origin_mm)?;
                        cap(b.origin_mm, 1e9)?;
                        finite(b.direction_mm)?;
                        let squared = b.direction_mm.iter().map(|v| v * v).sum::<f64>();
                        if (squared - 1.0).abs() > 1e-6 {
                            return Err(invalid());
                        }
                        let [main, secondary] = &b.connects;
                        if !uuid7(main) || !uuid7(secondary) {
                            return Err(fail(
                                ErrorCode::InvalidId,
                                "invalid full element identity",
                            )
                            .with_rule("DV-BASE-001"));
                        }
                        if main == secondary || main == &e.id || secondary == &e.id {
                            return Err(fail(
                                ErrorCode::InvalidArgument,
                                "a bolt group connects two distinct other parts",
                            )
                            .with_rule(if main == secondary {
                                "DV-REL-007"
                            } else {
                                "DV-REL-006"
                            }));
                        }
                        // The parts' existence and kinds decide acceptance; they are only read.
                        for part in [main, secondary] {
                            reads.insert(Footprint {
                                identity: part.clone(),
                                group: Group::Lifecycle,
                            });
                        }
                    }
                    Shape::BoltGroup(b) => {
                        bolt_group_connects(&b.connects, &e.id)?;
                        bolt_group_values(b)?;
                        // The pinned definition travels by digest: either this batch pins it as
                        // a resource or the model already holds it. Either way the stored
                        // record must agree with the definition's own values.
                        let spec = fastener_definition(input, prims, b.fastener)?;
                        bolt_group_against(&spec, b)?;
                        for part in &b.connects {
                            reads.insert(Footprint {
                                identity: part.clone(),
                                group: Group::Lifecycle,
                            });
                        }
                        reads.insert(Footprint {
                            identity: b.fastener.to_string(),
                            group: Group::Relationship,
                        });
                    }
                    Shape::Instance { kind, definition } => {
                        if e.id == *definition || e.material.is_some() {
                            return Err(invalid());
                        }
                        instance::validate_source(definition, *kind, prims)?;
                        for group in [
                            Group::Lifecycle,
                            Group::Label,
                            Group::ShapeInputs,
                            Group::Placement,
                            Group::Material,
                            Group::Relationship,
                        ] {
                            reads.insert(Footprint {
                                identity: definition.clone(),
                                group,
                            });
                        }
                    }
                    Shape::Grid(g) => {
                        grid::validate_record(g)?;
                        if e.id != g.id.to_string()
                            || e.name != g.name
                            || e.material.is_some()
                            || e.placement != grid::placement(g)
                        {
                            return Err(invalid());
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
                    Shape::Construction(o) => {
                        construction::validate_record(o)?;
                        if e.id != o.id.to_string()
                            || e.name != o.name
                            || e.material.is_some()
                            || e.placement.translation_mm != [0.0; 3]
                            || e.placement.orientation_xyzw != [0.0, 0.0, 0.0, 1.0]
                        {
                            return Err(invalid());
                        }
                    }
                    Shape::Annotation(a) => {
                        annotation::validate_record(a)?;
                        if e.id != a.id.to_string()
                            || e.name != a.name
                            || e.material.is_some()
                            || e.placement.translation_mm != [0.0; 3]
                            || e.placement.orientation_xyzw != [0.0, 0.0, 0.0, 1.0]
                        {
                            return Err(invalid());
                        }
                    }
                }
                if let Some(d) = e.material {
                    reads.insert(Footprint {
                        identity: d.to_string(),
                        group: Group::Relationship,
                    });
                }
            }
            Change::Grid { id, grid: g } => {
                grid::validate_record(g)?;
                let existing = prims
                    .iter()
                    .find(|p| p.element_id.as_deref() == Some(id))
                    .ok_or_else(invalid)?;
                let before = grid::read(existing)?;
                let mode = |f: &beamcraft_model::grid::GridFamily| {
                    matches!(f, beamcraft_model::grid::GridFamily::Regular(_))
                };
                if g.id != before.id
                    || g.name != before.name
                    || g.translation_mm != before.translation_mm
                    || g.orientation_zw != before.orientation_zw
                    || mode(&g.u) != mode(&before.u)
                    || mode(&g.v) != mode(&before.v)
                {
                    return Err(unsupported());
                }
            }
            Change::PartClass { .. } => {}
            Change::BoltGroup { id, group } => {
                bolt_group_connects(&group.connects, id)?;
                bolt_group_values(group)?;
                // A repin in the same batch or the pinned definition the model already holds.
                let spec = fastener_definition(input, prims, group.fastener)?;
                bolt_group_against(&spec, group)?;
                for part in &group.connects {
                    reads.insert(Footprint {
                        identity: part.clone(),
                        group: Group::Lifecycle,
                    });
                }
                reads.insert(Footprint {
                    identity: group.fastener.to_string(),
                    group: Group::Relationship,
                });
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
            Change::Corners { corners, .. } => {
                if corners.len() > 1024 {
                    return Err(limit());
                }
                let mut seen = Vec::new();
                for corner in corners {
                    if corner.vertex >= 1024 || seen.contains(&corner.vertex) {
                        return Err(invalid());
                    }
                    seen.push(corner.vertex);
                    corner.validate().map_err(|_| invalid())?;
                }
            }
            Change::Thickness { mm, .. } => cap([*mm], 1e6)?,
            Change::Bends { bends, .. } => {
                // Structural bounds only: finiteness, caps, positive radii and a finite angle
                // inside (-pi, pi); whether the records form one supported bent body is the
                // engine's replay decision.
                if bends.len() > MAX_BENDS {
                    return Err(limit());
                }
                for bend in bends {
                    let (lines, radii, angle): ([[f64; 3]; 2], &[f64], Option<f64>) =
                        match bend.spec {
                            BendSpec::Fold {
                                a_mm,
                                b_mm,
                                radius_mm,
                                angle_rad,
                            } => ([a_mm, b_mm], &[radius_mm][..], Some(angle_rad)),
                            BendSpec::Roll {
                                a_mm,
                                b_mm,
                                radius_mm,
                                radius_b_mm,
                            } => (
                                [a_mm, b_mm],
                                match radius_b_mm {
                                    Some(rb) => &[radius_mm, rb][..],
                                    None => &[radius_mm][..],
                                },
                                None,
                            ),
                        };
                    cap(lines.iter().flatten().copied(), 1e9)?;
                    if radii.iter().any(|v| *v <= 0.0) {
                        return Err(invalid());
                    }
                    cap(radii.iter().copied(), 1e9)?;
                    // A cone with equal radii is a cylinder, and the stored encoding keeps one
                    // radius for a cylinder: equal radii would read back as a cylinder and
                    // refuse their own sector blank.
                    if let BendSpec::Roll {
                        radius_mm,
                        radius_b_mm: Some(radius_b),
                        ..
                    } = bend.spec
                        && radius_b == radius_mm
                    {
                        return Err(invalid());
                    }
                    if let Some(angle) = angle
                        && (!angle.is_finite()
                            || angle == 0.0
                            || angle.abs() >= core::f64::consts::PI)
                    {
                        return Err(invalid());
                    }
                }
            }
            Change::Cuts { cuts, .. } => cut_values(cuts)?,
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
                if grid::guide_target(input, prims, &m.id) && m.quantities.is_some() {
                    return Err(unsupported());
                }
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
/// The authored plane-cut records of a created plate or member (A03): finite and within the
/// coordinate cap, with a non-zero normal, in ascending ordinals at most 999. Whether a plane
/// crosses the element is the engine's gate.
fn cut_values(cuts: &[CutRecord]) -> Result<(), Diagnostic> {
    let mut last_ordinal = None;
    for cut in cuts {
        // The record admission keeps the model's codes: non-finite and a zero normal are
        // invalid, the coordinate cap is a limit.
        match cut.validate() {
            Ok(()) => (),
            Err(error) if error.code == ErrorCode::LimitExceeded => return Err(limit()),
            Err(_) => return Err(invalid()),
        }
        if cut.ordinal > 999 {
            return Err(limit());
        }
        if last_ordinal.is_some_and(|last| last >= cut.ordinal) {
            return Err(invalid());
        }
        last_ordinal = Some(cut.ordinal);
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
                    thickness_side: _,
                    corners,
                    bends,
                    holes,
                    hole_records,
                    cuts,
                } => finite(
                    contour_mm
                        .iter()
                        .flatten()
                        .copied()
                        .chain(holes.iter().flatten().flatten().copied())
                        .chain(
                            hole_records
                                .iter()
                                .flat_map(|r| r.polygon_mm.iter().flatten().copied()),
                        )
                        .chain([*thickness_mm])
                        .chain(
                            cuts.iter()
                                .flat_map(|c| c.point_mm.into_iter().chain(c.normal)),
                        )
                        .chain(corners.iter().flat_map(|c| match c.treatment {
                            CornerTreatment::Chamfer { d1_mm, d2_mm } => [d1_mm, d2_mm],
                            CornerTreatment::Round { radius_mm } => [radius_mm, radius_mm],
                        }))
                        .chain(bends.iter().flat_map(|b| {
                            match b.spec {
                                BendSpec::Fold {
                                    a_mm,
                                    b_mm,
                                    radius_mm,
                                    angle_rad,
                                } => a_mm
                                    .into_iter()
                                    .chain(b_mm)
                                    .chain([radius_mm, angle_rad])
                                    .collect::<Vec<_>>(),
                                BendSpec::Roll {
                                    a_mm,
                                    b_mm,
                                    radius_mm,
                                    radius_b_mm,
                                } => a_mm
                                    .into_iter()
                                    .chain(b_mm)
                                    .chain([radius_mm, radius_b_mm.unwrap_or(radius_mm)])
                                    .collect::<Vec<_>>(),
                            }
                        })),
                ),
                Shape::Member {
                    axis_mm,
                    roll_rad,
                    placement,
                    cuts,
                    ..
                } => finite(
                    axis_mm
                        .iter()
                        .flatten()
                        .copied()
                        .chain(roll_rad.iter().copied())
                        .chain(placement.iter().flat_map(|p| {
                            [
                                p.plane_offset_mm,
                                p.depth_offset_mm,
                                p.start_offset_mm,
                                p.end_offset_mm,
                            ]
                        }))
                        .chain(
                            cuts.iter()
                                .flat_map(|c| c.point_mm.into_iter().chain(c.normal)),
                        ),
                ),
                Shape::Assembly => Ok(()),
                Shape::LoftedPlate { start_mm, end_mm } => finite(
                    start_mm
                        .iter()
                        .flatten()
                        .copied()
                        .chain(end_mm.iter().flatten().copied()),
                ),
                Shape::Weld(w) => finite([w.size_mm]),
                Shape::Bolt(b) => finite(
                    [b.diameter_mm, b.pitch_mm]
                        .into_iter()
                        .chain(b.origin_mm)
                        .chain(b.direction_mm),
                ),
                Shape::BoltGroup(b) => finite(
                    b.point1_mm
                        .into_iter()
                        .chain(b.point2_mm)
                        .chain(b.plane_normal_mm)
                        .chain([
                            b.rotation_rad,
                            b.on_plane_mm,
                            b.depth_mm,
                            b.offset_start_mm,
                            b.spacing_value_mm,
                            b.length_mm,
                        ])
                        .chain(b.spacings_mm.iter().copied()),
                ),
                Shape::Level { elevation_mm } => finite([*elevation_mm]),
                Shape::Instance { .. } => Ok(()),
                Shape::Grid(g) => grid::validate_record(g),
                Shape::Construction(o) => construction::validate_record(o),
                Shape::Annotation(a) => annotation::validate_record(a),
            }
        }
        Change::Grid { grid: g, .. } => grid::validate_record(g),
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
        Change::BoltGroup { group, .. } => finite(
            group
                .point1_mm
                .into_iter()
                .chain(group.point2_mm)
                .chain(group.plane_normal_mm)
                .chain([
                    group.rotation_rad,
                    group.on_plane_mm,
                    group.depth_mm,
                    group.offset_start_mm,
                    group.spacing_value_mm,
                    group.length_mm,
                ])
                .chain(group.spacings_mm.iter().copied()),
        ),
        Change::Cuts { cuts, .. } => finite(
            cuts.iter()
                .flat_map(|c| c.point_mm.into_iter().chain(c.normal)),
        ),
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

fn plate_corner_records(p: &Prim) -> Result<(), Diagnostic> {
    let fields = [
        ("steel:plate:cornerVertex", "int[]"),
        ("steel:plate:cornerKind", "token[]"),
        ("steel:plate:cornerD1Mm", "double[]"),
        ("steel:plate:cornerD2Mm", "double[]"),
        ("steel:plate:cornerRadiusMm", "double[]"),
    ];
    let mut arrays = Vec::new();
    let mut absent = 0usize;
    for (name, ty) in fields {
        match optional(p, name, ty)? {
            None => absent += 1,
            Some(Value::Array(a)) => arrays.push(a),
            _ => return Err(invalid()),
        }
    }
    if absent == 5 {
        return Ok(());
    }
    if absent != 0 {
        return Err(invalid());
    }
    let [vertices, kinds, d1s, d2s, radii] = arrays.as_slice() else {
        return Err(invalid());
    };
    if vertices.len() > 1024 {
        return Err(limit());
    }
    if [kinds.len(), d1s.len(), d2s.len(), radii.len()]
        .iter()
        .any(|n| *n != vertices.len())
    {
        return Err(invalid());
    }
    let count = points(p, "steel:plate:contourPointsMm")?.len();
    let mut seen = Vec::new();
    for ((((v, k), d1), d2), r) in vertices
        .iter()
        .zip(kinds.iter())
        .zip(d1s.iter())
        .zip(d2s.iter())
        .zip(radii.iter())
    {
        let Value::Int(v) = v else {
            return Err(invalid());
        };
        let v = usize::try_from(*v).map_err(|_| invalid())?;
        if v >= count || seen.contains(&v) {
            return Err(invalid());
        }
        seen.push(v);
        let number = |n: &Value| match n {
            Value::Number(n) => Ok(*n),
            Value::Int(n) => Ok(*n as f64),
            _ => Err(invalid()),
        };
        let (d1, d2, r) = (number(d1)?, number(d2)?, number(r)?);
        let treatment = match k {
            Value::Text(k) if k == "chamfer" && r == 0. => {
                beamcraft_model::command::CornerTreatment::Chamfer {
                    d1_mm: d1,
                    d2_mm: d2,
                }
            }
            Value::Text(k) if k == "round" && d1 == 0. && d2 == 0. => {
                beamcraft_model::command::CornerTreatment::Round { radius_mm: r }
            }
            _ => return Err(invalid()),
        };
        PlateCorner {
            vertex: v,
            treatment,
        }
        .validate()
        .map_err(|_| invalid())?;
    }
    Ok(())
}
