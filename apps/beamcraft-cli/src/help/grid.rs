// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help for project grids.
pub(super) const COMMANDS: &[&str] = &[
    "create-grid",
    "update-grid",
    "list-grids",
    "resolve-grid-point",
    "resolve-grid-plane",
    "draw-grid-line",
    "reshape-grid-line",
    "remove-grid-line",
    "relabel-grid-line",
];
pub(super) const HELP: &str = "Grids (T01.1): create-grid --store ROOT --name TEXT [--new-id UUID] [--origin X,Y] [--rotation DEGREES] [--u-expression TEXT] [--v-expression TEXT] [--u-labels TEXT] [--v-labels TEXT] [--u-scheme numeric|alpha] [--v-scheme numeric|alpha] [--u-axis-ids UUIDS] [--v-axis-ids UUIDS] [--u-span START,END] [--v-span START,END] and the usual command-context identity options. At least one expression is required; an omitted family is empty and drawn. Default labels: U numeric, V alpha; default spans extend 2000 mm beyond the other family. list-grids --store ROOT returns verified grids, source expressions, line identities and world endpoints. copy-element --id GRID [--new-axis-ids UUIDS] creates independent axes (U then V); --offset X,Y,0 is supported. delete-element --id GRID refuses incoming references. Supply all creation and operation IDs explicitly for exact retries.";

pub(super) const LINES: &str = "Drawn grids (T01.2): create-grid can start a drawn family with --u-from X,Y --u-to X,Y --u-label TEXT [--u-axis-id UUID] (and V equivalents), in world XY millimetres. draw-grid-line --store ROOT --id GRID --family u|v --label TEXT --from X,Y --to X,Y [--new-axis-id UUID]; reshape-grid-line --store ROOT --id GRID --axis-id UUID --from X,Y --to X,Y; remove-grid-line --store ROOT --id GRID --axis-id UUID; relabel-grid-line --store ROOT --id GRID (--axis-id UUID --label TEXT | --family u|v --labels TEXT). The usual context IDs are accepted. Regular and drawn modes never convert implicitly; the last line and referenced lines cannot be removed. Each edit is one revision and one undo unit.";

pub(super) const UPDATE: &str = "Update a regular grid (T01.3): update-grid --store ROOT --id GRID [--name TEXT] [--origin X,Y] [--rotation DEGREES] [--u-expression TEXT --u-labels TEXT] [--u-new-axis-ids UUIDS] [--u-span START,END] and equivalent V options. A changed expression carries its full label list; new labels consume new AxisIds in line order, and kept labels retain their identities. A span-only change needs no expression. Family mode is immutable; grid edits move no placed part. The usual context identity options apply; supply explicit new-axis-ids, including an empty list when none are created, for exact family-edit retries after later changes.";

pub(super) const QUERIES: &str = "Grid queries (T01.4): resolve-grid-point --store ROOT --grid ID_OR_NAME --labels U,V; resolve-grid-plane --store ROOT --grid ID_OR_NAME --label TEXT. Names must identify exactly one live grid. snap accepts grid_intersection,grid_end,grid_line and optional --grids UUIDS (empty means none); distance is XY and hits retain query Z. create-member accepts --start-at-grid GRID:LABEL,LABEL and --end-at-grid GRID:LABEL,LABEL in place of the respective point; create-plate accepts --origin-at-grid GRID:LABEL,LABEL in place of --translation. Give --z MM or --level UUID [--offset MM]; grid placement cannot combine with translation or orientation. Stored points are ordinary coordinates and never follow later grid edits. Supply explicit --op-id and --new-id for exact retries.";
