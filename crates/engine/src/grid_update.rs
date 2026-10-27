// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! T01.3 regular-family replacement, preserving AxisIds by exact label matching.
use crate::{
    footprint::{Entry, Footprint, Group, Target},
    grid_edit::EditedGrid,
};
use beamcraft_model::{
    command::CommandError,
    error::ErrorCode,
    grid::{Grid, GridFamily, parse_grid_expression},
    grid_update::{RegularGridPatch, UpdateGrid},
    id::ElementId,
};
use std::collections::{BTreeMap, BTreeSet};
fn bad(code: ErrorCode, msg: &'static str) -> CommandError {
    CommandError::new(code, "grid.update", msg).with_rule("T01-V01")
}
fn family(
    patch: &RegularGridPatch,
    family: &mut GridFamily,
    used: &mut BTreeSet<ElementId>,
) -> Result<(), CommandError> {
    let GridFamily::Regular(f) = family else {
        return Err(bad(ErrorCode::UnsupportedProperty, "GRID_FAMILY_MODE"));
    };
    if let (Some(expression), Some(labels)) = (&patch.expression, &patch.labels) {
        let (expression, positions) = parse_grid_expression(expression)
            .map_err(|_| bad(ErrorCode::InvalidArgument, "GRID_EXPRESSION_INVALID"))?;
        let old = f
            .labels
            .iter()
            .cloned()
            .zip(f.axis_ids.iter().copied())
            .collect::<BTreeMap<_, _>>();
        let mut fresh = patch.new_axis_ids.iter().copied();
        let mut ids = Vec::with_capacity(labels.len());
        for label in labels {
            if let Some(id) = old.get(label) {
                ids.push(*id);
            } else {
                let id = fresh
                    .next()
                    .ok_or_else(|| bad(ErrorCode::InvalidArgument, "GRID_AXIS_ID_COUNT"))?;
                if !used.insert(id) {
                    return Err(bad(
                        ErrorCode::DuplicateId,
                        "grid axis identity is already in use",
                    ));
                }
                ids.push(id);
            }
        }
        if fresh.next().is_some() {
            return Err(bad(ErrorCode::InvalidArgument, "GRID_AXIS_ID_COUNT"));
        }
        f.expression = expression;
        f.positions_mm = positions;
        f.labels = labels.clone();
        f.axis_ids = ids;
    }
    if let Some(span) = patch.span_mm {
        f.span_mm = span;
    }
    Ok(())
}

fn fitted_span(other: &GridFamily, u: bool) -> [f64; 2] {
    let mut bounds: Option<[f64; 2]> = None;
    let mut include = |value: f64| {
        bounds = Some(match bounds {
            Some([low, high]) => [low.min(value), high.max(value)],
            None => [value, value],
        });
    };
    match other {
        GridFamily::Regular(f) => {
            for position in &f.positions_mm {
                include(*position);
            }
        }
        GridFamily::Drawn(lines) => {
            for line in lines {
                for point in [line.start_mm, line.end_mm] {
                    if let Some(value) = point.get(if u { 1 } else { 0 }) {
                        include(*value);
                    }
                }
            }
        }
    }
    let [low, high] = bounds.unwrap_or([0.0, 0.0]);
    [low - 2000.0, high + 2000.0]
}

/// Decide a full grid update without mutating the source or moving any placed element.
pub fn update(
    request: &UpdateGrid,
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
    let mut used = used_ids.clone();
    used.insert(source.id);
    used.extend(source.axis_ids());
    if let Some(p) = &request.u {
        family(p, &mut grid.u, &mut used)?;
    }
    if let Some(p) = &request.v {
        family(p, &mut grid.v, &mut used)?;
    }
    if let Some(name) = &request.name {
        grid.name = name.clone();
    }
    if let Some(origin) = request.origin_mm {
        grid.translation_mm = origin;
    }
    if let Some(orientation) = request.orientation_zw {
        grid.orientation_zw = orientation;
    }
    if request.fit_spans {
        let u_span = fitted_span(&grid.v, true);
        let v_span = fitted_span(&grid.u, false);
        if request
            .u
            .as_ref()
            .is_none_or(|patch| patch.span_mm.is_none())
            && let GridFamily::Regular(family) = &mut grid.u
        {
            family.span_mm = u_span;
        }
        if request
            .v
            .as_ref()
            .is_none_or(|patch| patch.span_mm.is_none())
            && let GridFamily::Regular(family) = &mut grid.v
        {
            family.span_mm = v_span;
        }
    }
    grid.validate()?;
    let retained = grid.axis_ids().collect::<BTreeSet<_>>();
    if source
        .axis_ids()
        .any(|id| !retained.contains(&id) && referenced_axes.contains(&id))
    {
        return Err(bad(ErrorCode::DependencyConflict, "GRID_IN_USE"));
    }
    let target = Target::Element(source.id);
    let mut reads = [
        Group::Lifecycle,
        Group::ShapeInputs,
        Group::Placement,
        Group::Label,
    ]
    .into_iter()
    .map(|group| Entry { target, group })
    .collect::<Vec<_>>();
    reads.extend(
        grid.axis_ids()
            .filter(|id| !source.axis_ids().any(|old| old == *id))
            .map(|id| Entry {
                target: Target::Element(id),
                group: Group::Lifecycle,
            }),
    );
    let mut writes = vec![Entry {
        target,
        group: Group::ShapeInputs,
    }];
    writes.push(Entry {
        target,
        group: Group::Derived,
    });
    if request.name.is_some() {
        writes.push(Entry {
            target,
            group: Group::Label,
        });
    }
    if request.origin_mm.is_some() || request.orientation_zw.is_some() {
        writes.extend(
            [Group::Placement, Group::Derived]
                .into_iter()
                .map(|group| Entry { target, group }),
        );
    }
    Ok(EditedGrid {
        grid,
        footprint: Footprint::from_entries(&reads, &writes)?,
    })
}
