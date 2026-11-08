// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the schedule command family.

pub(super) const HELP: &str = concat!(
    "Schedules (I05): schedule-export --store ROOT (--template TEMPLATE.json | --schedule ",
    "NAME) --out ELEMENTS_CSV [--summary GROUPS_CSV] [--html PAGE.html] (the verified head ",
    "rendered through a schedule template: columns from standard report fields and typed ",
    "user attributes, one optional V14 filter rule, grouping and totals, as CSV and HTML; a ",
    "missing, blocked or stale value stays an empty cell and is counted next to its total, ",
    "never zero, and a column naming an unknown attribute stays in the schedule and is ",
    "reported in unknown_columns; same head and template render byte-identical files; the ",
    "template comes from a file or a stored definition, exactly one). schedule-define ",
    "--store ROOT --name NAME --template TEMPLATE.json saves the template in the model as a ",
    "shared owned definition under /World/Definitions/Schedules, admitted against the ",
    "pinned attribute state (a duplicate name refuses). schedules --store ROOT lists the ",
    "stored templates in name order with their documents. schedule-undefine --store ROOT ",
    "--name NAME removes one; an export naming it refuses. Definition commands carry ",
    "revisions, footprints, commit and undo like any authored entity."
);
