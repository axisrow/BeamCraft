// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the checks command family (Q03).

pub(super) const HELP: &str = concat!(
    "Checks (Q03): check-model --store ROOT [--element UUID] [--offset N] [--limit N] (checks ",
    "one identified revision: the committed head at the moment the check starts, named in the ",
    "reply; --element narrows the scope to that element's subtree. The supported set is the ",
    "geometric PART_CLASH rule with the 0.5 mm contact tolerance, the completeness rules ",
    "PART_WITHOUT_MATERIAL, PART_DEGENERATE and QUANTITY_STALE, the connectivity rules ",
    "PART_UNCONNECTED, WELD_WITHOUT_CONTACT and CONTACT_WITHOUT_CONNECTION under recorded ",
    "joint intent, and the corner-cut rules CORNER_NOT_CUT and CORNER_OVERCUT under recorded ",
    "corner-cut requirements: each requirement is judged at the corner re-derived from the ",
    "final evaluated geometry against its two recorded neighbours, the measured cut leg goes ",
    "through the recorded acceptance band, and an unambiguous CORNER_NOT_CUT carries a ",
    "set-plate-corner fix hint (apply one fix, then recheck). With no recorded requirement ",
    "a rule group is not applicable for any element, and the coverage paragraph says so ",
    "(contract docs/contracts/q03-model-checks-draft-v0.1.md). Every element of the scope is ",
    "reported in exactly one coverage bucket — checked, not_applicable(reason) or ",
    "unknown(reason) — so partial evaluation is explicit and never silently green; findings ",
    "pages do not hide the whole-scope status, counts and coverage. Exit 0 for a green or ",
    "unknown (not fully checked) model and 2 for a red one; an operational refusal keeps the ",
    "ordinary rejected reply with its own exit 2, so branch on the reply shape, not on the ",
    "code alone. The checks detect; they do not block export or publication, and a green ",
    "report is not a structural design approval)",
);
