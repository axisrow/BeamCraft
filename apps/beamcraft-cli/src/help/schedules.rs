// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the schedule command family.

pub(super) const HELP: &str = concat!(
    "Schedules (I05): schedule-export --store ROOT --template TEMPLATE.json --out ",
    "ELEMENTS_CSV [--summary GROUPS_CSV] [--html PAGE.html] (the verified head rendered ",
    "through a schedule template: columns from standard report fields and typed user ",
    "attributes, one optional V14 filter rule, grouping and totals, as CSV and HTML; a ",
    "missing, blocked or stale value stays an empty cell and is counted next to its total, ",
    "never zero, and a column naming an unknown attribute stays in the schedule and is ",
    "reported in unknown_columns; same head and template render byte-identical files)",
);
