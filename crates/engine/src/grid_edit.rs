// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Pure T01.2 line-edit decisions against one complete project-grid snapshot.
use crate::footprint::{Entry, Footprint, Group, Target};
use beamcraft_model::{
    command::CommandError,
    error::ErrorCode,
    grid::{DrawnGridLine, Grid, GridFamily},
    grid_edit::{GridFamilyId, GridLineEdit, GridLineEditRequest},
    id::ElementId,
};
use std::collections::BTreeSet;

/// Validated replacement record and its semantic dependencies.
#[derive(Debug, Clone, PartialEq)]
pub struct EditedGrid {
    /// Complete replacement grid. The caller authors the grid attributes atomically.
    pub grid: Grid,
    /// Declared reads and writes.
    pub footprint: Footprint,
}
fn bad(code: ErrorCode, msg: &'static str) -> CommandError {
    CommandError::new(code, "grid.line", msg).with_rule("T01-V01")
}
fn family_mut(grid: &mut Grid, id: GridFamilyId) -> &mut GridFamily {
    match id {
        GridFamilyId::U => &mut grid.u,
        GridFamilyId::V => &mut grid.v,
    }
}
/// Convert a world XY point to the grid frame after frame admission.
pub fn world_to_local(grid: &Grid, p: [f64; 2]) -> [f64; 2] {
    let [x, y] = p;
    let [tx, ty] = grid.translation_mm;
    let [z, w] = grid.orientation_zw;
    let n = z * z + w * w;
    let c = (w * w - z * z) / n;
    let s = 2.0 * z * w / n;
    [c * (x - tx) + s * (y - ty), -s * (x - tx) + c * (y - ty)]
}
/// Apply one line operation without changing the input. New identities are checked against the
/// complete project's live identity set; referenced AxisIds cannot be removed.
pub fn edit(
    request: &GridLineEditRequest,
    source: &Grid,
    used_ids: &BTreeSet<ElementId>,
    referenced_axes: &BTreeSet<ElementId>,
) -> Result<EditedGrid, CommandError> {
    request.validate_arguments()?;
    source.validate()?;
    if source.id != request.target_id {
        return Err(bad(ErrorCode::NotFound, "grid not found"));
    }
    let mut grid = source.clone();
    let find = |id: ElementId| -> Result<GridFamilyId, CommandError> {
        for (family, f) in [(GridFamilyId::U, &source.u), (GridFamilyId::V, &source.v)] {
            let found = match f {
                GridFamily::Regular(f) => f.axis_ids.contains(&id),
                GridFamily::Drawn(lines) => lines.iter().any(|l| l.axis_id == id),
            };
            if found {
                return Ok(family);
            }
        }
        Err(bad(ErrorCode::NotFound, "GRID_AXIS_UNKNOWN"))
    };
    match &request.edit {
        GridLineEdit::Draw {
            family,
            new_axis_id,
            label,
            from_mm,
            to_mm,
        } => {
            if used_ids.contains(new_axis_id)
                || *new_axis_id == source.id
                || source.axis_ids().any(|id| id == *new_axis_id)
            {
                return Err(bad(
                    ErrorCode::DuplicateId,
                    "grid axis identity is already in use",
                ));
            }
            let a = world_to_local(source, *from_mm);
            let b = world_to_local(source, *to_mm);
            let GridFamily::Drawn(lines) = family_mut(&mut grid, *family) else {
                return Err(bad(ErrorCode::UnsupportedProperty, "GRID_FAMILY_MODE"));
            };
            if lines.len() >= beamcraft_model::grid::MAX_GRID_LINES_PER_FAMILY {
                return Err(bad(
                    ErrorCode::LimitExceeded,
                    "grid family exceeds the line cap",
                ));
            }
            lines.push(DrawnGridLine {
                axis_id: *new_axis_id,
                label: label.clone(),
                start_mm: a,
                end_mm: b,
            });
        }
        GridLineEdit::Reshape {
            axis_id,
            from_mm,
            to_mm,
        } => {
            let GridFamily::Drawn(lines) = family_mut(&mut grid, find(*axis_id)?) else {
                return Err(bad(ErrorCode::UnsupportedProperty, "GRID_FAMILY_MODE"));
            };
            let line = lines
                .iter_mut()
                .find(|l| l.axis_id == *axis_id)
                .ok_or_else(|| bad(ErrorCode::NotFound, "GRID_AXIS_UNKNOWN"))?;
            line.start_mm = world_to_local(source, *from_mm);
            line.end_mm = world_to_local(source, *to_mm);
        }
        GridLineEdit::Remove { axis_id } => {
            if referenced_axes.contains(axis_id) {
                return Err(bad(ErrorCode::DependencyConflict, "GRID_IN_USE"));
            }
            let GridFamily::Drawn(lines) = family_mut(&mut grid, find(*axis_id)?) else {
                return Err(bad(ErrorCode::UnsupportedProperty, "GRID_FAMILY_MODE"));
            };
            lines.retain(|l| l.axis_id != *axis_id);
        }
        GridLineEdit::Relabel { axis_id, label } => match family_mut(&mut grid, find(*axis_id)?) {
            GridFamily::Regular(f) => {
                let slot = f
                    .axis_ids
                    .iter()
                    .position(|id| id == axis_id)
                    .ok_or_else(|| bad(ErrorCode::NotFound, "GRID_AXIS_UNKNOWN"))?;
                *f.labels
                    .get_mut(slot)
                    .ok_or_else(|| bad(ErrorCode::InvalidArgument, "GRID_LABEL_COUNT"))? =
                    label.clone();
            }
            GridFamily::Drawn(lines) => {
                lines
                    .iter_mut()
                    .find(|l| l.axis_id == *axis_id)
                    .ok_or_else(|| bad(ErrorCode::NotFound, "GRID_AXIS_UNKNOWN"))?
                    .label = label.clone();
            }
        },
        GridLineEdit::RelabelFamily { family, labels } => match family_mut(&mut grid, *family) {
            GridFamily::Regular(f) => {
                if labels.len() != f.axis_ids.len() {
                    return Err(bad(ErrorCode::InvalidArgument, "GRID_LABEL_COUNT"));
                }
                f.labels = labels.clone();
            }
            GridFamily::Drawn(lines) => {
                if labels.len() != lines.len() {
                    return Err(bad(ErrorCode::InvalidArgument, "GRID_LABEL_COUNT"));
                }
                for (line, label) in lines.iter_mut().zip(labels) {
                    line.label = label.clone();
                }
            }
        },
    }
    grid.validate()?;
    let target = Target::Element(grid.id);
    let mut reads = [Group::Lifecycle, Group::ShapeInputs, Group::Placement]
        .into_iter()
        .map(|group| Entry { target, group })
        .collect::<Vec<_>>();
    if let GridLineEdit::Draw { new_axis_id, .. } = request.edit {
        reads.push(Entry {
            target: Target::Element(new_axis_id),
            group: Group::Lifecycle,
        });
    }
    let footprint = Footprint::from_entries(
        &reads,
        &[
            Entry {
                target,
                group: Group::ShapeInputs,
            },
            Entry {
                target,
                group: Group::Derived,
            },
        ],
    )?;
    Ok(EditedGrid { grid, footprint })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn id(n: u8) -> ElementId {
        ElementId::parse(&format!("01990000-0000-7000-8000-{n:012x}")).unwrap()
    }
    fn source() -> Grid {
        Grid {
            id: id(1),
            name: "Drawn".into(),
            translation_mm: [0.0; 2],
            orientation_zw: [0.0, 1.0],
            u: GridFamily::Drawn(vec![
                DrawnGridLine {
                    axis_id: id(2),
                    label: "A".into(),
                    start_mm: [0.0, 0.0],
                    end_mm: [0.0, 1000.0],
                },
                DrawnGridLine {
                    axis_id: id(3),
                    label: "B".into(),
                    start_mm: [1000.0, 0.0],
                    end_mm: [1000.0, 1000.0],
                },
            ]),
            v: GridFamily::Drawn(Vec::new()),
        }
    }
    #[test]
    fn referenced_lines_cannot_be_removed_and_refusals_leave_the_input_unchanged() {
        let source = source();
        let before = source.clone();
        let request = GridLineEditRequest {
            target_id: source.id,
            edit: GridLineEdit::Remove { axis_id: id(2) },
        };
        assert_eq!(
            edit(
                &request,
                &source,
                &BTreeSet::new(),
                &BTreeSet::from([id(2)])
            )
            .unwrap_err()
            .code,
            ErrorCode::DependencyConflict
        );
        assert_eq!(source, before);
        let result = edit(&request, &source, &BTreeSet::new(), &BTreeSet::new()).unwrap();
        assert_eq!(result.grid.axis_ids().collect::<Vec<_>>(), [id(3)]);
    }
    #[test]
    fn world_points_are_converted_using_the_pinned_frame_and_each_edit_has_one_target() {
        let mut source = source();
        source.translation_mm = [1000.0, 2000.0];
        let theta = std::f64::consts::FRAC_PI_4;
        source.orientation_zw = [theta.sin(), theta.cos()];
        let request = GridLineEditRequest {
            target_id: source.id,
            edit: GridLineEdit::Reshape {
                axis_id: id(2),
                from_mm: [1000.0, 2000.0],
                to_mm: [0.0, 2000.0],
            },
        };
        let result = edit(&request, &source, &BTreeSet::new(), &BTreeSet::new()).unwrap();
        let GridFamily::Drawn(lines) = result.grid.u else {
            panic!()
        };
        assert!((lines[0].end_mm[1] - 1000.0).abs() < 1e-8);
        assert_eq!(lines[1].axis_id, id(3));
        assert!(
            result
                .footprint
                .writes()
                .iter()
                .all(|e| e.target == Target::Element(source.id))
        );
    }
}
