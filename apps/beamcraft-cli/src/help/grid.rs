// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help for project grids.
pub(super) const COMMANDS: &[&str] = &[
    "create-grid",
    "list-grids",
    "draw-grid-line",
    "reshape-grid-line",
    "remove-grid-line",
    "relabel-grid-line",
];
pub(super) const HELP: &str = "Grids (T01.1): create-grid --store ROOT --name TEXT [--new-id UUID] [--origin X,Y] [--rotation DEGREES] [--u-expression TEXT] [--v-expression TEXT] [--u-labels TEXT] [--v-labels TEXT] [--u-scheme numeric|alpha] [--v-scheme numeric|alpha] [--u-axis-ids UUIDS] [--v-axis-ids UUIDS] [--u-span START,END] [--v-span START,END] and the usual command-context identity options. At least one expression is required; an omitted family is empty and drawn. Default labels: U numeric, V alpha; default spans extend 2000 mm beyond the other family. list-grids --store ROOT returns verified grids, source expressions, line identities and world endpoints. copy-element --id GRID [--new-axis-ids UUIDS] creates independent axes (U then V); --offset X,Y,0 is supported. delete-element --id GRID refuses incoming references. Supply all creation and operation IDs explicitly for exact retries.";

pub(super) const LINES: &str = "Drawn grids (T01.2): create-grid can start a drawn family with --u-from X,Y --u-to X,Y --u-label TEXT [--u-axis-id UUID] (and V equivalents), in world XY millimetres. draw-grid-line --store ROOT --id GRID --family u|v --label TEXT --from X,Y --to X,Y [--new-axis-id UUID]; reshape-grid-line --store ROOT --id GRID --axis-id UUID --from X,Y --to X,Y; remove-grid-line --store ROOT --id GRID --axis-id UUID; relabel-grid-line --store ROOT --id GRID (--axis-id UUID --label TEXT | --family u|v --labels TEXT). The usual context IDs are accepted. Regular and drawn modes never convert implicitly; the last line and referenced lines cannot be removed. Each edit is one revision and one undo unit.";
