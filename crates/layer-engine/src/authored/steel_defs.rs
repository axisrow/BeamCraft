// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The steel definition readers (#388 slice): the profile and fastener definitions of a
//! stored prim, re-admitted because a committed layer is untrusted input. Extracted from
//! `validate.rs` unchanged; the discipline reversal moves this file to `crates/steel`.

use super::validate::{number, optional_number, optional_text, points, required, text};
use super::*;
use beamcraft_model::limits::MAX_FASTENER_LENGTHS;
use steel::model::fastener::{FastenerKind, HeadKind, HoleMode, WasherSpec};
use steel::model::section::{
    CustomSection, MAX_CUSTOM_CAVITIES, MAX_CUSTOM_POINTS, ParametricSection, SectionFamily,
};
use steel::model::{definition::SectionProperties, fastener::FastenerSpec, section::SectionShape};

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
pub(super) fn read_definition(p: &Prim, kind: &str) -> Result<Definition, Diagnostic> {
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
