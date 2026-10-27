// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Corner treatments of authored plate contours (A12.2, the F04 plate amendment section 2).
//!
//! The authored contour keeps its sharp corner points; a treatment is an authored parameter, so
//! a rounded corner stays editable by its radius and is never a stored polygon. This module
//! derives the treated outline in `f64` the kernel then validates with its exact predicates, as
//! for any contour. A chamfer replaces the corner with a straight cut, a rounding with a
//! circular arc tangent to both edges, faceted to a chord deviation of at most
//! [`ARC_CHORD_DEVIATION_MM`] (the rule of the round hole, F06 A-2 D-2), with at least two
//! segments and the end points exactly on the tangent points.

use beamcraft_model::command::{CommandError, CornerTreatment, PlateCorner};
use beamcraft_model::error::ErrorCode;

/// The largest chord deviation a faceted arc may keep (the round-hole rule of F06 A-2 D-2).
pub const ARC_CHORD_DEVIATION_MM: f64 = 0.05;

/// Below this turn magnitude (per unit edge direction) the two edges count as collinear or
/// folding back onto each other: the corner does not really turn, so a treatment on it is
/// refused (the amendment refuses exactly-collinear corners; this near-exact guard keeps the
/// decision deterministic in `f64` while every real corner passes).
const TURN_EPSILON: f64 = 1e-12;

fn refused(message: &'static str) -> CommandError {
    CommandError::new(ErrorCode::InvalidArgument, "corner", message)
}

/// The treated outline of an authored contour and the largest chord deviation any faceted arc
/// keeps (`0.0` when no corner is rounded). Without corners the contour is returned unchanged,
/// exactly as A02 extruded it.
///
/// The caller has admitted the corners against this contour (one per existing vertex, bounded
/// values); the geometry rules are decided here: a treatment on a corner that does not really
/// turn, and treatments whose consumed lengths reach an edge length, are `INVALID_ARGUMENT`.
/// A treated outline that needs more points than the F04 contour cap is `LIMIT_EXCEEDED`.
pub fn treated_outline(
    points: &[[f64; 3]],
    corners: &[PlateCorner],
) -> Result<(Vec<[f64; 2]>, f64), CommandError> {
    if corners.is_empty() {
        return Ok((points.iter().map(|[x, y, _]| [*x, *y]).collect(), 0.0));
    }
    let count = points.len();
    if count < 3 {
        return Err(refused("a treated contour needs at least three vertices"));
    }
    // One treatment slot per vertex; the admission already refused duplicates and out-of-range
    // vertices, so an unexpected entry here is an input rejection, never a panic.
    let mut treatments = vec![None; count];
    for corner in corners {
        let Some(slot) = treatments.get_mut(corner.vertex) else {
            return Err(refused("corner names no vertex of the contour"));
        };
        *slot = Some(corner.treatment);
    }
    let xy = |i: usize| -> Option<[f64; 2]> { points.get(i).map(|p| [p[0], p[1]]) };
    // The vertex positions come from indices below `count`; a miss is an input rejection.
    let refused_index = || refused("corner names no vertex of the contour");
    let unit = |from: [f64; 2], to: [f64; 2]| -> Option<[f64; 2]> {
        let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
        let len = (dx * dx + dy * dy).sqrt();
        if len > 0.0 && len.is_finite() {
            Some([dx / len, dy / len])
        } else {
            None
        }
    };
    // The replacement of one vertex: zero points for a sharp corner, the two cut points of a
    // chamfer, or the tangent points and interior arc points of a rounding.
    let mut replacements: Vec<Vec<[f64; 2]>> = Vec::with_capacity(count);
    let mut consumed = vec![[0.0; 2]; count];
    let mut max_deviation = 0.0_f64;
    for (i, treatment) in treatments.iter().enumerate() {
        let Some(treatment) = *treatment else {
            replacements.push(Vec::new());
            continue;
        };
        let vertex = xy(i).ok_or_else(refused_index)?;
        let previous = xy((i + count - 1) % count).ok_or_else(refused_index)?;
        let next = xy((i + 1) % count).ok_or_else(refused_index)?;
        // The directions of travel through the vertex: `arriving` along the previous edge,
        // `leaving` along the next. Collinear or folding-back edges make them parallel.
        let Some(arriving) = unit(previous, vertex) else {
            return Err(refused("the corner sits on a zero-length edge"));
        };
        let Some(leaving) = unit(vertex, next) else {
            return Err(refused("the corner sits on a zero-length edge"));
        };
        let cross = arriving[0] * leaving[1] - arriving[1] * leaving[0];
        if cross.abs() <= TURN_EPSILON {
            return Err(refused(
                "a corner treatment applies only to a corner that really turns",
            ));
        }
        // The interior angle between the two edges at the vertex, in (0, pi).
        let dot = arriving[0] * leaving[0] + arriving[1] * leaving[1];
        let phi = core::f64::consts::PI - cross.abs().atan2(dot);
        match treatment {
            CornerTreatment::Chamfer { d1_mm, d2_mm } => {
                if let Some(slot) = consumed.get_mut(i) {
                    *slot = [d1_mm, d2_mm];
                } else {
                    return Err(refused_index());
                }
                replacements.push(vec![
                    [
                        vertex[0] - arriving[0] * d1_mm,
                        vertex[1] - arriving[1] * d1_mm,
                    ],
                    [
                        vertex[0] + leaving[0] * d2_mm,
                        vertex[1] + leaving[1] * d2_mm,
                    ],
                ]);
            }
            CornerTreatment::Round { radius_mm } => {
                // Elementary plane geometry: the tangent length from the angle between the two
                // edges, the centre on the bisector at the radius' offset distance.
                let tangent = radius_mm / (phi / 2.0).tan();
                let towards_previous = [-arriving[0], -arriving[1]];
                let start = [
                    vertex[0] + towards_previous[0] * tangent,
                    vertex[1] + towards_previous[1] * tangent,
                ];
                let end = [
                    vertex[0] + leaving[0] * tangent,
                    vertex[1] + leaving[1] * tangent,
                ];
                let bisector = unit(
                    [0.0, 0.0],
                    [
                        towards_previous[0] + leaving[0],
                        towards_previous[1] + leaving[1],
                    ],
                )
                .ok_or_else(|| refused("the rounding of this corner has no angle bisector"))?;
                let offset = radius_mm / (phi / 2.0).sin();
                let centre = [
                    vertex[0] + bisector[0] * offset,
                    vertex[1] + bisector[1] * offset,
                ];
                // The arc takes the short way round, so it always cuts the corner off; the
                // segments follow the deviation rule, and at least two are used.
                let sweep = core::f64::consts::PI - phi;
                let segments = if radius_mm > ARC_CHORD_DEVIATION_MM {
                    let max_step = 2.0 * (1.0 - ARC_CHORD_DEVIATION_MM / radius_mm).acos();
                    ((sweep / max_step).ceil() as usize).max(2)
                } else {
                    2
                };
                if !tangent.is_finite() || !offset.is_finite() {
                    return Err(refused("the rounding of this corner is not representable"));
                }
                // One arc can never fit the derived contour cap, so refuse before it allocates
                // (the arc holds segments + 1 points); a radius near the coordinate cap on a
                // sharp corner would otherwise expand to hundreds of thousands of points only
                // to be refused by the same cap below.
                if segments + 1 > beamcraft_geom::tolerance::CONTOUR_POINT_CAP {
                    return Err(CommandError::new(
                        ErrorCode::LimitExceeded,
                        "corner",
                        "the rounding of this corner needs more points than the contour cap",
                    )
                    .with_rule("DV-LIM-001"));
                }
                if let Some(slot) = consumed.get_mut(i) {
                    *slot = [tangent, tangent];
                } else {
                    return Err(refused_index());
                }
                let start_angle = (start[1] - centre[1]).atan2(start[0] - centre[0]);
                let end_angle = (end[1] - centre[1]).atan2(end[0] - centre[0]);
                let mut delta = end_angle - start_angle;
                if delta > core::f64::consts::PI {
                    delta -= core::f64::consts::TAU;
                } else if delta <= -core::f64::consts::PI {
                    delta += core::f64::consts::TAU;
                }
                let mut arc = Vec::with_capacity(segments + 1);
                arc.push(start);
                for step in 1..segments {
                    let angle = start_angle + delta * (step as f64) / (segments as f64);
                    arc.push([
                        centre[0] + radius_mm * angle.cos(),
                        centre[1] + radius_mm * angle.sin(),
                    ]);
                }
                arc.push(end);
                replacements.push(arc);
                let deviation = radius_mm * (1.0 - (sweep / (2.0 * segments as f64)).cos());
                max_deviation = max_deviation.max(deviation);
            }
        }
    }
    // The consumed lengths at the two ends of one edge must add up to less than the edge
    // length, so no edge collapses to a point (the amendment's fit rule).
    for i in 0..count {
        let j = (i + 1) % count;
        let edge = {
            let a = xy(i).ok_or_else(refused_index)?;
            let b = xy(j).ok_or_else(refused_index)?;
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            (dx * dx + dy * dy).sqrt()
        };
        let towards_next = consumed.get(i).map(|c| c[1]).unwrap_or(0.0);
        let towards_previous = consumed.get(j).map(|c| c[0]).unwrap_or(0.0);
        if towards_next + towards_previous >= edge {
            return Err(refused(
                "the corner treatments of one edge reach the edge length",
            ));
        }
    }
    let mut outline = Vec::with_capacity(count);
    for (i, replacement) in replacements.into_iter().enumerate() {
        if replacement.is_empty() {
            outline.push(xy(i).ok_or_else(refused_index)?);
        } else {
            outline.extend_from_slice(&replacement);
        }
    }
    if outline.len() > beamcraft_geom::tolerance::CONTOUR_POINT_CAP {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "corner",
            "the treated contour needs more points than the contour cap",
        )
        .with_rule("DV-LIM-001"));
    }
    Ok((outline, max_deviation))
}

#[cfg(test)]
mod tests {
    use super::*;
    use beamcraft_model::command::CornerTreatment::{Chamfer, Round};

    fn chamfer(vertex: usize, d1: f64, d2: f64) -> PlateCorner {
        PlateCorner {
            vertex,
            treatment: Chamfer {
                d1_mm: d1,
                d2_mm: d2,
            },
        }
    }

    fn round(vertex: usize, radius: f64) -> PlateCorner {
        PlateCorner {
            vertex,
            treatment: Round { radius_mm: radius },
        }
    }

    fn square() -> Vec<[f64; 3]> {
        vec![
            [0.0, 0.0, 0.0],
            [1000.0, 0.0, 0.0],
            [1000.0, 1000.0, 0.0],
            [0.0, 1000.0, 0.0],
        ]
    }

    /// Shoelace area of an outline, the independent closed form the assertions compare.
    fn area(outline: &[[f64; 2]]) -> f64 {
        let mut total = 0.0;
        for i in 0..outline.len() {
            let j = (i + 1) % outline.len();
            total += outline[i][0] * outline[j][1] - outline[j][0] * outline[i][1];
        }
        total / 2.0
    }

    #[test]
    fn a_symmetric_chamfer_cuts_the_corner_triangle_once() {
        // Amendment case 2: the cut triangle is 100 * 100 / 2 = 5,000 mm2.
        let (outline, deviation) = treated_outline(&square(), &[chamfer(2, 100.0, 100.0)]).unwrap();
        assert_eq!(deviation, 0.0);
        assert!(
            (area(&outline) - 995_000.0).abs() <= 1e-9,
            "{}",
            area(&outline)
        );
        // A single authored distance means the same symmetric cut: d2 defaults to d1, which
        // the typed command already carries as two equal values.
        let (same, _) = treated_outline(&square(), &[chamfer(2, 100.0, 100.0)]).unwrap();
        assert_eq!(outline, same);
        assert_eq!(outline.len(), 5);
    }

    #[test]
    fn an_asymmetric_chamfer_removes_half_the_product_of_the_distances() {
        // Amendment case 3: 100 x 50 cut removes 2,500 mm2.
        let (outline, _) = treated_outline(&square(), &[chamfer(1, 50.0, 100.0)]).unwrap();
        assert!(
            (area(&outline) - 997_500.0).abs() <= 1e-9,
            "{}",
            area(&outline)
        );
    }

    #[test]
    fn a_rounded_corner_facets_within_the_stated_deviation() {
        // Amendment case 4: the faceted area stays within the reported deviation of the
        // analytic value, and the deviation itself is at most 0.05 mm.
        let (outline, deviation) = treated_outline(&square(), &[round(2, 100.0)]).unwrap();
        assert!(
            deviation > 0.0 && deviation <= ARC_CHORD_DEVIATION_MM,
            "{deviation}"
        );
        let analytic = 1_000_000.0 - (1.0 - core::f64::consts::FRAC_PI_4) * 100.0 * 100.0;
        let faceted = area(&outline);
        // The chords lie inside the true arc, so the faceted corner cuts a little more than
        // the analytic fillet: strictly less area, and the loss is bounded by the chord
        // sagittas of the segments the rule chose (about 0.2 mm2 each here).
        let lost = analytic - faceted;
        assert!(
            lost > 0.0 && lost <= 10.0,
            "faceted {faceted} vs analytic {analytic}"
        );
        // The tangent points sit exactly on both edges of the corner.
        let on_x_edge = outline
            .iter()
            .any(|p| p[1] == 1000.0 && (p[0] - 900.0).abs() <= 1e-9);
        let on_y_edge = outline
            .iter()
            .any(|p| p[0] == 1000.0 && (p[1] - 900.0).abs() <= 1e-9);
        assert!(on_x_edge && on_y_edge, "{outline:?}");
    }

    #[test]
    fn a_concave_corner_takes_a_treatment_too() {
        // Amendment case 5: the reflex corner of an L-shaped plate is treated like any other.
        let l_shape = vec![
            [0.0, 0.0, 0.0],
            [1000.0, 0.0, 0.0],
            [1000.0, 300.0, 0.0],
            [400.0, 300.0, 0.0],
            [400.0, 600.0, 0.0],
            [0.0, 600.0, 0.0],
        ];
        // Vertex 3 is the reflex corner; a rounding of 100 mm leaves every edge long enough.
        let (outline, deviation) = treated_outline(&l_shape, &[round(3, 100.0)]).unwrap();
        assert!(deviation <= ARC_CHORD_DEVIATION_MM);
        let faceted = area(&outline);
        // The reflex corner gains material: more than the sharp 420,000 mm2, and the arc
        // segments are all inside the outline's bounding box.
        assert!(faceted > 420_000.0, "{faceted}");
        assert!(outline.len() > 6);
    }

    #[test]
    fn treatments_that_do_not_turn_or_do_not_fit_are_refused() {
        // Negative case 3: a collinear vertex (the contour doubles straight through).
        let straight_through = vec![
            [0.0, 0.0, 0.0],
            [500.0, 0.0, 0.0],
            [1000.0, 0.0, 0.0],
            [1000.0, 500.0, 0.0],
            [0.0, 500.0, 0.0],
        ];
        let error = treated_outline(&straight_through, &[chamfer(1, 100.0, 100.0)]).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidArgument);
        assert_eq!(error.field, "corner");
        // A fold-back vertex is parallel too.
        let fold_back = vec![
            [0.0, 0.0, 0.0],
            [1000.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            [0.0, 500.0, 0.0],
        ];
        assert_eq!(
            treated_outline(&fold_back, &[round(1, 50.0)])
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        // Negative case 2: the consumed lengths of one edge reach the edge length. Two
        // 500 mm chamfers on the adjacent vertices of the 1000 mm bottom edge consume
        // exactly the edge, which is refused; more than the edge length as well.
        assert!(
            treated_outline(
                &square(),
                &[chamfer(0, 500.0, 500.0), chamfer(1, 500.0, 500.0)]
            )
            .unwrap_err()
            .code
                == ErrorCode::InvalidArgument
        );
        assert!(
            treated_outline(
                &square(),
                &[chamfer(0, 900.0, 900.0), chamfer(1, 900.0, 900.0)]
            )
            .unwrap_err()
            .code
                == ErrorCode::InvalidArgument
        );
        // A single over-consuming chamfer on one short edge is refused: d2 = 350 mm exceeds
        // the 300 mm edge towards the next vertex.
        let small = vec![
            [0.0, 0.0, 0.0],
            [300.0, 0.0, 0.0],
            [300.0, 1000.0, 0.0],
            [0.0, 1000.0, 0.0],
        ];
        assert_eq!(
            treated_outline(&small, &[chamfer(0, 100.0, 350.0)])
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn an_out_of_range_corner_is_refused_and_no_corner_leaves_the_contour_unchanged() {
        // The admission refuses this at the command level; the evaluator stays total.
        assert!(treated_outline(&square(), &[chamfer(9, 100.0, 100.0)]).is_err());
        let (outline, deviation) = treated_outline(&square(), &[]).unwrap();
        assert_eq!(
            outline,
            [[0.0, 0.0], [1000.0, 0.0], [1000.0, 1000.0], [0.0, 1000.0]]
        );
        assert_eq!(deviation, 0.0);
    }

    #[test]
    fn arc_segment_counts_follow_the_deviation_rule_and_the_point_cap_binds() {
        // A large radius needs many segments for the same deviation; on a kilometre square
        // the treatments still fit the edges, and the derived outline is capped at the F04
        // contour cap and refused beyond it.
        let big = vec![
            [0.0, 0.0, 0.0],
            [1_000_000.0, 0.0, 0.0],
            [1_000_000.0, 1_000_000.0, 0.0],
            [0.0, 1_000_000.0, 0.0],
        ];
        let corners: Vec<PlateCorner> = (0..4).map(|v| round(v, 400_000.0)).collect();
        let error = treated_outline(&big, &corners).unwrap_err();
        assert_eq!(error.code, ErrorCode::LimitExceeded);
        // A practical radius stays far below the cap.
        let (outline, deviation) = treated_outline(&big, &[round(0, 50.0)]).unwrap();
        assert!(outline.len() < 40, "{}", outline.len());
        assert!(deviation <= ARC_CHORD_DEVIATION_MM);
    }
}
