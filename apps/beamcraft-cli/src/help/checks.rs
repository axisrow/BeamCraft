// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the checks command family (Q03).

pub(super) const HELP: &str = concat!(
    "Checks (Q03): check-model --store ROOT [--element UUID] [--offset N] [--limit N] (checks ",
    "one identified revision: the committed head at the moment the check starts, named in the ",
    "reply; --element narrows the scope to that element's subtree. The supported set is the ",
    "geometric PART_CLASH rule with the 0.5 mm contact tolerance and the completeness rules ",
    "PART_WITHOUT_MATERIAL, PART_DEGENERATE and QUANTITY_STALE (contract ",
    "docs/contracts/q03-model-checks-draft-v0.1.md; connectivity, weld and corner-cut rules ",
    "are not supported yet). Every element of the scope is reported in exactly one coverage ",
    "bucket — checked, not_applicable(reason) or unknown(reason) — so partial evaluation is ",
    "explicit and never silently green; findings pages do not hide the whole-scope status, ",
    "counts and coverage. Exit 0 for a green or unknown (not fully checked) model and 2 for ",
    "a red one; an operational refusal keeps the ordinary rejected reply with its own exit 2, ",
    "so branch on the reply shape, not on the code alone. The checks detect; they do not ",
    "block export or publication, and a green report is not a structural design approval)",
);
