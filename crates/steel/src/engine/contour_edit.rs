// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Plate contour editing (A12.3, the F04 plate amendment section 3): the five edits move one
//! vertex, move one edge, insert one vertex, delete one vertex and set one corner treatment.
//!
//! Each edit is a pure function from the authored contour and its sparse corner list to the
//! edited pair; the adapter commits the result as one shape-changing `UpdateElement` patch, so
//! the kernel revalidates the new contour, every hole and cut replays, and the revision is one
//! undo unit. All edits happen in the plate plane: only the local `x` and `y` of a point change,
//! its authored `z` is kept. Vertices and edges are named by position (decision O-1: no stable
//! vertex ids); edge `i` joins vertex `i` and vertex `i + 1`, the last joins back to the first.

use crate::model::command::{CornerTreatment, PlateCorner};
use beamcraft_geom::tolerance::COORDINATE_CAP_MM;
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;

fn refused(field: &'static str, message: &'static str) -> CommandError {
    CommandError::new(ErrorCode::InvalidArgument, field, message)
}

fn out_of_range(field: &'static str) -> CommandError {
    refused(field, "the index names no vertex or edge of the contour")
}

/// Admits one in-plane point the same way a created contour point is admitted: finite
/// (`NON_FINITE_NUMBER`) and within the coordinate cap (`LIMIT_EXCEEDED`).
fn admit_point(field: &'static str, xy: [f64; 2]) -> Result<(), CommandError> {
    if xy.iter().any(|v| !v.is_finite()) {
        return Err(
            CommandError::new(ErrorCode::NonFiniteNumber, field, "value is not finite")
                .with_rule("DV-ATTR-003"),
        );
    }
    if xy.iter().any(|v| v.abs() > COORDINATE_CAP_MM) {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            field,
            "coordinate exceeds the coordinate cap",
        )
        .with_rule("DV-LIM-001"));
    }
    Ok(())
}

/// Moves one vertex to the given in-plane point. Its corner treatment moves with it: the
/// position indices are unchanged, so the sparse list carries over untouched.
pub fn move_vertex(
    points: &[[f64; 3]],
    corners: &[PlateCorner],
    vertex: usize,
    to_mm: [f64; 2],
) -> Result<(Vec<[f64; 3]>, Vec<PlateCorner>), CommandError> {
    admit_point("to_mm", to_mm)?;
    if vertex >= points.len() {
        return Err(out_of_range("vertex"));
    }
    let mut next = points.to_vec();
    if let Some(point) = next.get_mut(vertex) {
        point[0] = to_mm[0];
        point[1] = to_mm[1];
    }
    Ok((next, corners.to_vec()))
}

/// Moves both end vertices of edge `edge` by the same in-plane vector; the two neighbouring
/// edges follow. The treatments of the two end vertices move with their vertices.
pub fn move_edge(
    points: &[[f64; 3]],
    corners: &[PlateCorner],
    edge: usize,
    delta_mm: [f64; 2],
) -> Result<(Vec<[f64; 3]>, Vec<PlateCorner>), CommandError> {
    admit_point("delta_mm", delta_mm)?;
    if edge >= points.len() {
        return Err(out_of_range("edge"));
    }
    let count = points.len();
    let mut next = points.to_vec();
    for index in [edge, (edge + 1) % count] {
        if let Some(point) = next.get_mut(index) {
            point[0] += delta_mm[0];
            point[1] += delta_mm[1];
        }
    }
    Ok((next, corners.to_vec()))
}

/// Splits edge `edge` with a new sharp vertex at the given point. The treatments of the
/// neighbouring vertices stay; positions after the split shift by one.
pub fn insert_vertex(
    points: &[[f64; 3]],
    corners: &[PlateCorner],
    edge: usize,
    point_mm: [f64; 2],
) -> Result<(Vec<[f64; 3]>, Vec<PlateCorner>), CommandError> {
    admit_point("point_mm", point_mm)?;
    if edge >= points.len() {
        return Err(out_of_range("edge"));
    }
    let after = edge + 1;
    let z = points
        .get(edge)
        .and_then(|p| p.get(2))
        .copied()
        .ok_or_else(|| out_of_range("edge"))?;
    let mut next = points.to_vec();
    next.insert(after.min(next.len()), [point_mm[0], point_mm[1], z]);
    Ok((next, shift_corners(corners, after, 1)))
}

/// Removes one vertex with its treatment; positions after it shift down by one. A plate with
/// three vertices is never reduced, whatever an interactive caller would prefer.
pub fn delete_vertex(
    points: &[[f64; 3]],
    corners: &[PlateCorner],
    vertex: usize,
) -> Result<(Vec<[f64; 3]>, Vec<PlateCorner>), CommandError> {
    if points.len() <= 3 {
        return Err(refused(
            "contour_points_mm",
            "a plate with three vertices is never reduced",
        ));
    }
    if vertex >= points.len() {
        return Err(out_of_range("vertex"));
    }
    let mut next = points.to_vec();
    next.remove(vertex);
    // The deleted vertex's treatment goes with it; later positions shift down by one.
    let treated = corners
        .iter()
        .filter(|corner| corner.vertex != vertex)
        .map(|corner| match corner.vertex > vertex {
            true => PlateCorner {
                vertex: corner.vertex - 1,
                treatment: corner.treatment,
            },
            false => *corner,
        })
        .collect();
    Ok((next, treated))
}

/// Sets or clears the treatment of one vertex: a treatment replaces any existing one in its
/// authored-list position, `None` removes it, and a new treatment joins the list in vertex
/// order. The contour is untouched.
pub fn set_corner(
    points: &[[f64; 3]],
    corners: &[PlateCorner],
    vertex: usize,
    treatment: Option<CornerTreatment>,
) -> Result<(Vec<[f64; 3]>, Vec<PlateCorner>), CommandError> {
    if vertex >= points.len() {
        return Err(out_of_range("vertex"));
    }
    let mut next = corners.to_vec();
    if let Some(existing) = next.iter_mut().find(|c| c.vertex == vertex) {
        match treatment {
            Some(treatment) => existing.treatment = treatment,
            None => next.retain(|c| c.vertex != vertex),
        }
        return Ok((points.to_vec(), next));
    }
    if let Some(treatment) = treatment {
        let corner = PlateCorner { vertex, treatment };
        corner.validate()?;
        let position = next.partition_point(|c| c.vertex < vertex);
        next.insert(position.min(next.len()), corner);
    }
    Ok((points.to_vec(), next))
}

/// Shifts the position of every corner whose vertex is at or after `from` by one (after an
/// insertion). Earlier vertices keep their positions.
fn shift_corners(corners: &[PlateCorner], from: usize, delta: i64) -> Vec<PlateCorner> {
    corners
        .iter()
        .map(|corner| {
            if corner.vertex >= from {
                PlateCorner {
                    vertex: usize::try_from(corner.vertex as i64 + delta).unwrap_or(corner.vertex),
                    treatment: corner.treatment,
                }
            } else {
                *corner
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use CornerTreatment::{Chamfer, Round};

    fn square() -> (Vec<[f64; 3]>, Vec<PlateCorner>) {
        (
            vec![
                [0.0, 0.0, 0.0],
                [1000.0, 0.0, 0.0],
                [1000.0, 1000.0, 0.0],
                [0.0, 1000.0, 0.0],
            ],
            Vec::new(),
        )
    }

    fn treated() -> (Vec<[f64; 3]>, Vec<PlateCorner>) {
        let (points, _) = square();
        let corners = vec![
            PlateCorner {
                vertex: 1,
                treatment: Chamfer {
                    d1_mm: 100.0,
                    d2_mm: 50.0,
                },
            },
            PlateCorner {
                vertex: 3,
                treatment: Round { radius_mm: 40.0 },
            },
        ];
        (points, corners)
    }

    #[test]
    fn a_vertex_move_rewrites_one_point_and_keeps_the_treatments() {
        let (points, corners) = treated();
        let (next, treated) = move_vertex(&points, &corners, 2, [700.0, 1200.0]).unwrap();
        assert_eq!(next[2], [700.0, 1200.0, 0.0]);
        assert_eq!(next[0], points[0]);
        assert_eq!(treated, corners);
        // An unknown vertex and a non-finite point are refused before anything is written.
        assert_eq!(
            move_vertex(&points, &corners, 9, [0.0; 2])
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            move_vertex(&points, &corners, 0, [f64::NAN, 0.0])
                .unwrap_err()
                .code,
            ErrorCode::NonFiniteNumber
        );
    }

    #[test]
    fn an_edge_move_shifts_exactly_its_two_end_vertices() {
        let (points, corners) = square();
        let (next, treated) = move_edge(&points, &corners, 1, [100.0, -50.0]).unwrap();
        assert_eq!(next[1], [1100.0, -50.0, 0.0]);
        assert_eq!(next[2], [1100.0, 950.0, 0.0]);
        assert_eq!(next[0], points[0]);
        assert_eq!(next[3], points[3]);
        assert_eq!(treated, corners);
        // The closing edge joins the last vertex back to the first.
        let (wrapped, _) = move_edge(&points, &corners, 3, [10.0, 20.0]).unwrap();
        assert_eq!(wrapped[3], [10.0, 1020.0, 0.0]);
        assert_eq!(wrapped[0], [10.0, 20.0, 0.0]);
        assert_eq!(
            move_edge(&points, &corners, 4, [0.0; 2]).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn an_inserted_vertex_is_sharp_and_shifts_the_later_treatments() {
        let (points, corners) = treated();
        let (next, treated) = insert_vertex(&points, &corners, 0, [500.0, 0.0]).unwrap();
        assert_eq!(next.len(), 5);
        assert_eq!(next[1], [500.0, 0.0, 0.0]);
        // The chamfer was on the old vertex 1, now 2; the rounding moves from 3 to 4.
        assert_eq!(treated.iter().map(|c| c.vertex).collect::<Vec<_>>(), [2, 4]);
        assert_eq!(treated[0].treatment, corners[0].treatment);
        // An insertion between the treated vertices shifts only the rounding.
        let (_, treated) = insert_vertex(&points, &corners, 1, [1000.0, 500.0]).unwrap();
        assert_eq!(treated.iter().map(|c| c.vertex).collect::<Vec<_>>(), [1, 4]);
        assert_eq!(
            insert_vertex(&points, &corners, 4, [0.0; 2])
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn a_deleted_vertex_takes_its_treatment_and_a_triangle_is_never_reduced() {
        let (points, corners) = treated();
        let (next, treated) = delete_vertex(&points, &corners, 1).unwrap();
        assert_eq!(next.len(), 3);
        // The deleted chamfer is gone; the rounding moves from 3 to 2.
        assert_eq!(treated.len(), 1);
        assert_eq!(treated[0].vertex, 2);
        assert_eq!(treated[0].treatment, corners[1].treatment);
        assert_eq!(
            delete_vertex(&next, &treated, 0).unwrap_err(),
            refused(
                "contour_points_mm",
                "a plate with three vertices is never reduced"
            )
        );
        assert_eq!(
            delete_vertex(&points, &corners, 7).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn set_corner_replaces_clears_and_inserts_in_vertex_order() {
        let (points, corners) = treated();
        // Replace in the authored-list position.
        let (same, treated) =
            set_corner(&points, &corners, 1, Some(Round { radius_mm: 5.0 })).unwrap();
        assert_eq!(same, points);
        assert_eq!(treated[0].vertex, 1);
        assert_eq!(treated[0].treatment, Round { radius_mm: 5.0 });
        // Clear removes the entry and keeps the other.
        let (_, cleared) = set_corner(&points, &treated, 1, None).unwrap();
        assert_eq!(cleared.len(), 1);
        assert_eq!(cleared[0].vertex, 3);
        // A treatment on a fresh vertex joins in vertex order and is admitted as a command
        // argument (whether it really turns is the body evaluation's decision).
        let (_, added) = set_corner(
            &points,
            &cleared,
            0,
            Some(Chamfer {
                d1_mm: 10.0,
                d2_mm: 10.0,
            }),
        )
        .unwrap();
        assert_eq!(added.iter().map(|c| c.vertex).collect::<Vec<_>>(), [0, 3]);
        // An inadmissible value is refused before the list grows.
        assert!(set_corner(&points, &cleared, 2, Some(Round { radius_mm: 0.0 })).is_err());
        assert_eq!(
            set_corner(&points, &corners, 4, None).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }
}
