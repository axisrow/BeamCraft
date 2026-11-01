// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The steel write helpers (#388 slice): the profile section properties and the cut, corner,
//! bend and bolt-group authors of the authored layer. Extracted from the platform's `edit.rs`
//! unchanged; the discipline reversal moved this file to `crates/steel`, where the hooks write
//! their attributes through it.

use super::BoltGroup;
use crate::model::command::{BendRecord, CutRecord, PlateCorner};
use crate::model::definition::SectionProperties;
use beamcraft_layer_engine::authored::seam::author::{
    attr, attribute, author_record_history, points,
};
use beamcraft_layer_engine::authored::{Opinion, cut_history};
use beamcraft_layer_engine::{Diagnostic, Prim, limit};
use openusd::sdf;

/// The optional declared designation, standard and section numbers of any profile definition.
pub(super) fn section_properties(
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
pub(super) fn optional_text(
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
pub(super) fn optional_number(
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
/// Authors the connection type and the seventeen bolt-group attributes of an A14.3 group
/// (bolt amendment sections 4 and 5), replaced as a whole on every write. The `connects`
/// record is the caller's: it maps identities through the batch.
pub(super) fn author_bolt_group(
    data: &mut sdf::Data,
    p: &str,
    b: &BoltGroup,
) -> Result<(), Diagnostic> {
    attr(
        data,
        p,
        "steel:connection:type",
        "token",
        sdf::Value::Token("bolt_group".into()),
    )?;
    attr(
        data,
        p,
        "steel:bolt:fastener",
        "string",
        sdf::Value::String(b.fastener.to_string()),
    )?;
    let [x1, y1, z1] = b.point1_mm;
    let [x2, y2, z2] = b.point2_mm;
    let [nx, ny, nz] = b.plane_normal_mm;
    for (name, ty, v) in [
        (
            "steel:bolt:point1Mm",
            "double3",
            sdf::Value::vec3d(x1, y1, z1),
        ),
        (
            "steel:bolt:point2Mm",
            "double3",
            sdf::Value::vec3d(x2, y2, z2),
        ),
        (
            "steel:bolt:planeNormalMm",
            "double3",
            sdf::Value::vec3d(nx, ny, nz),
        ),
        (
            "steel:bolt:rotationRad",
            "double",
            sdf::Value::Double(b.rotation_rad),
        ),
        (
            "steel:bolt:onPlaneMm",
            "double",
            sdf::Value::Double(b.on_plane_mm),
        ),
        (
            "steel:bolt:depthMm",
            "double",
            sdf::Value::Double(b.depth_mm),
        ),
        (
            "steel:bolt:offsetStartMm",
            "double",
            sdf::Value::Double(b.offset_start_mm),
        ),
        (
            "steel:bolt:spacingMode",
            "token",
            sdf::Value::Token(b.spacing_mode.clone().into()),
        ),
        (
            "steel:bolt:spacingValueMm",
            "double",
            sdf::Value::Double(b.spacing_value_mm),
        ),
        (
            "steel:bolt:spacingsMm",
            "double[]",
            sdf::Value::DoubleVec(b.spacings_mm.clone()),
        ),
        (
            "steel:bolt:lengthMode",
            "token",
            sdf::Value::Token(b.length_mode.clone().into()),
        ),
        (
            "steel:bolt:lengthMm",
            "double",
            sdf::Value::Double(b.length_mm),
        ),
        (
            "steel:bolt:washerUnderHead",
            "bool",
            sdf::Value::Bool(b.washer_under_head),
        ),
        (
            "steel:bolt:washerUnderNut",
            "bool",
            sdf::Value::Bool(b.washer_under_nut),
        ),
        (
            "steel:bolt:holeMode",
            "token",
            sdf::Value::Token(b.hole_mode.clone().into()),
        ),
    ] {
        attr(data, p, name, ty, v)?;
    }
    // The group's own clearance travels only with a generating group; a quiet group removes
    // the attribute, so an edit from generate to none never leaves a stale value behind.
    match b.hole_clearance_mm {
        Some(clearance) => attr(
            data,
            p,
            "steel:bolt:holeClearanceMm",
            "double",
            sdf::Value::Double(clearance),
        )?,
        None => attribute(
            data,
            p,
            "steel:bolt:holeClearanceMm",
            "double",
            Opinion::Remove,
            true,
        )?,
    }
    Ok(())
}

/// Writes the five parallel cut-record attributes of one element (A03). An empty list removes
/// typed empty arrays, masking weaker opinions; fresh uncut elements still author no cuts.
pub(super) fn author_cuts(
    data: &mut sdf::Data,
    p: &str,
    cuts: &[CutRecord],
    source: Option<&Prim>,
) -> Result<(), Diagnostic> {
    author_record_history(
        data,
        p,
        source,
        &cuts
            .iter()
            .map(|r| (r.operation_id, usize::from(r.ordinal)))
            .collect::<Vec<_>>(),
        cut_history::Family::Cut,
    )?;
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

pub(super) fn author_corners(
    data: &mut sdf::Data,
    p: &str,
    corners: &[PlateCorner],
) -> Result<(), Diagnostic> {
    // The five parallel corner attributes (A12.2), always together; a value that
    // its treatment does not use is written as zero.
    let mut vertices = Vec::with_capacity(corners.len());
    let mut kinds: Vec<&str> = Vec::with_capacity(corners.len());
    let mut d1s = Vec::with_capacity(corners.len());
    let mut d2s = Vec::with_capacity(corners.len());
    let mut radii = Vec::with_capacity(corners.len());
    for corner in corners {
        vertices.push(i32::try_from(corner.vertex).map_err(|_| limit())?);
        kinds.push(corner.treatment.kind());
        let (d1, d2, radius) = match corner.treatment {
            crate::model::command::CornerTreatment::Chamfer { d1_mm, d2_mm } => (d1_mm, d2_mm, 0.0),
            crate::model::command::CornerTreatment::Round { radius_mm } => (0.0, 0.0, radius_mm),
        };
        d1s.push(d1);
        d2s.push(d2);
        radii.push(radius);
    }
    attr(
        data,
        p,
        "steel:plate:cornerVertex",
        "int[]",
        sdf::Value::IntVec(vertices),
    )?;
    attr(
        data,
        p,
        "steel:plate:cornerKind",
        "token[]",
        sdf::Value::token_vec(kinds),
    )?;
    attr(
        data,
        p,
        "steel:plate:cornerD1Mm",
        "double[]",
        sdf::Value::DoubleVec(d1s),
    )?;
    attr(
        data,
        p,
        "steel:plate:cornerD2Mm",
        "double[]",
        sdf::Value::DoubleVec(d2s),
    )?;
    attr(
        data,
        p,
        "steel:plate:cornerRadiusMm",
        "double[]",
        sdf::Value::DoubleVec(radii),
    )?;

    Ok(())
}

/// The seven parallel bend attributes (A10), always together; a value its kind does not use is
/// written as the neutral fill (the line's first point twice, the radius twice, angle zero).
pub(super) fn author_bends(
    data: &mut sdf::Data,
    p: &str,
    bends: &[BendRecord],
) -> Result<(), Diagnostic> {
    let mut ids = Vec::with_capacity(bends.len());
    let mut kinds: Vec<&str> = Vec::with_capacity(bends.len());
    let mut lines_a: Vec<[f64; 3]> = Vec::with_capacity(bends.len());
    let mut lines_b: Vec<[f64; 3]> = Vec::with_capacity(bends.len());
    let mut radii = Vec::with_capacity(bends.len());
    let mut radii_b = Vec::with_capacity(bends.len());
    let mut angles = Vec::with_capacity(bends.len());
    for bend in bends {
        ids.push(bend.operation_id.to_string());
        kinds.push(bend.spec.kind());
        let (a, b, radius, radius_b, angle) = match bend.spec {
            crate::model::command::BendSpec::Fold {
                a_mm,
                b_mm,
                radius_mm,
                angle_rad,
            } => (a_mm, b_mm, radius_mm, radius_mm, angle_rad),
            crate::model::command::BendSpec::Roll {
                a_mm,
                b_mm,
                radius_mm,
                radius_b_mm,
            } => (a_mm, b_mm, radius_mm, radius_b_mm.unwrap_or(radius_mm), 0.0),
        };
        lines_a.push(a);
        lines_b.push(b);
        radii.push(radius);
        radii_b.push(radius_b);
        angles.push(angle);
    }
    attr(
        data,
        p,
        "steel:plate:bendOperationId",
        "string[]",
        sdf::Value::StringVec(ids),
    )?;
    attr(
        data,
        p,
        "steel:plate:bendKind",
        "token[]",
        sdf::Value::token_vec(kinds),
    )?;
    attr(
        data,
        p,
        "steel:plate:bendLineAMm",
        "double3[]",
        points(&lines_a),
    )?;
    attr(
        data,
        p,
        "steel:plate:bendLineBMm",
        "double3[]",
        points(&lines_b),
    )?;
    attr(
        data,
        p,
        "steel:plate:bendRadiusMm",
        "double[]",
        sdf::Value::DoubleVec(radii),
    )?;
    attr(
        data,
        p,
        "steel:plate:bendRadiusBMm",
        "double[]",
        sdf::Value::DoubleVec(radii_b),
    )?;
    attr(
        data,
        p,
        "steel:plate:bendAngleRad",
        "double[]",
        sdf::Value::DoubleVec(angles),
    )?;
    Ok(())
}
pub(super) const CUT_FIELDS: &[(&str, &str)] = &[
    ("steel:cut:operationId", "string[]"),
    ("steel:cut:ordinal", "int[]"),
    ("steel:cut:pointMm", "double3[]"),
    ("steel:cut:normal", "double3[]"),
    ("steel:cut:keep", "token[]"),
    (cut_history::NEXT, "int"),
    (cut_history::RETIRED, "string[]"),
];
pub(super) const MEMBER_PLACEMENT: &[(&str, &str)] = &[
    ("steel:member:planePosition", "token"),
    ("steel:member:planeOffsetMm", "double"),
    ("steel:member:depthPosition", "token"),
    ("steel:member:depthOffsetMm", "double"),
    ("steel:member:startOffsetMm", "double"),
    ("steel:member:endOffsetMm", "double"),
];
pub(super) const MHOLE_FIELDS: &[(&str, &str)] = &[
    ("steel:mhole:pointsMm", "double3[]"),
    ("steel:mhole:vertexCounts", "int[]"),
    ("steel:mhole:operationIds", "string[]"),
    ("steel:mhole:ordinals", "int[]"),
    ("steel:mhole:axisPointsMm", "double3[]"),
    ("steel:mhole:axisDirections", "double3[]"),
    ("steel:mhole:extents", "token[]"),
    ("steel:mhole:extentFromMm", "double[]"),
    ("steel:mhole:extentToMm", "double[]"),
    (cut_history::MHOLE_NEXT, "int"),
    (cut_history::MHOLE_RETIRED, "string[]"),
];
