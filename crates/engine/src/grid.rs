// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! T01 grid admission against a complete pinned project snapshot.
use crate::footprint::Footprint;
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::grid::{CreateGrid, Grid, MAX_GRIDS};
use beamcraft_model::id::ElementId;
use std::collections::BTreeSet;

/// Decide grid creation without mutating the snapshot. The caller supplies every existing element
/// identity and every grid, including invisible grids. Publication still checks the base revision.
pub fn create(
    command: &CreateGrid,
    grids: &[Grid],
    elements: &BTreeSet<ElementId>,
) -> Result<Footprint, CommandError> {
    command.validate_arguments()?;
    let error = |code, field, message| CommandError::new(code, field, message);
    let mut ids = elements.clone();
    for grid in grids {
        ids.insert(grid.id);
        ids.extend(grid.axis_ids());
    }
    if !ids.insert(command.grid.id) {
        return Err(error(
            ErrorCode::DuplicateId,
            "new_id",
            "grid identity is already in use",
        ));
    }
    for id in command.grid.axis_ids() {
        if !ids.insert(id) {
            return Err(error(
                ErrorCode::DuplicateId,
                "axis_ids",
                "grid axis identity is already in use",
            ));
        }
    }
    if grids.len() >= MAX_GRIDS {
        return Err(error(
            ErrorCode::LimitExceeded,
            "grid",
            "project exceeds the grid cap",
        ));
    }
    let base = Footprint::for_create(command.grid.id, None)?;
    let mut reads = base.reads().iter().copied().collect::<Vec<_>>();
    reads.extend(command.grid.axis_ids().map(|id| crate::footprint::Entry {
        target: crate::footprint::Target::Element(id),
        group: crate::footprint::Group::Lifecycle,
    }));
    Footprint::from_entries(&reads, &base.writes().iter().copied().collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use beamcraft_model::grid::{GridFamily, RegularGridFamily};
    fn id(n: usize) -> ElementId {
        ElementId::parse(&format!("01990000-0000-7000-8000-{n:012x}")).unwrap()
    }
    fn grid(n: usize) -> Grid {
        Grid {
            id: id(n),
            name: "Name may repeat".to_owned(),
            translation_mm: [0.0; 2],
            orientation_zw: [0.0, 1.0],
            u: GridFamily::Regular(RegularGridFamily {
                expression: "0".to_owned(),
                labels: vec!["1".to_owned()],
                axis_ids: vec![id(n + 1000)],
                span_mm: [-2000.0, 2000.0],
                positions_mm: vec![0.0],
            }),
            v: GridFamily::Drawn(Vec::new()),
        }
    }
    #[test]
    fn accepts_the_sixty_fourth_grid_and_refuses_the_sixty_fifth() {
        let mut existing = (1..64).map(grid).collect::<Vec<_>>();
        let request = CreateGrid { grid: grid(64) };
        let before = existing.clone();
        assert!(create(&request, &existing, &BTreeSet::new()).is_ok());
        assert_eq!(existing, before);
        existing.push(request.grid);
        assert_eq!(
            create(&CreateGrid { grid: grid(65) }, &existing, &BTreeSet::new())
                .unwrap_err()
                .code,
            ErrorCode::LimitExceeded
        );
    }
    #[test]
    fn axis_absence_is_a_declared_read_and_all_identities_are_checked() {
        let existing = vec![grid(1)];
        let mut request = CreateGrid { grid: grid(2) };
        let footprint = create(&request, &existing, &BTreeSet::new()).unwrap();
        for axis in request.grid.axis_ids() {
            assert!(footprint.reads().contains(&crate::footprint::Entry {
                target: crate::footprint::Target::Element(axis),
                group: crate::footprint::Group::Lifecycle
            }));
        }
        if let GridFamily::Regular(f) = &mut request.grid.u {
            f.axis_ids = existing[0].axis_ids().collect();
        }
        assert_eq!(
            create(&request, &existing, &BTreeSet::new())
                .unwrap_err()
                .code,
            ErrorCode::DuplicateId
        );
    }
}
