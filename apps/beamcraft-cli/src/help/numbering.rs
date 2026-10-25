// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the numbering command family.

pub(super) const COMMANDS: &[&str] = &[
    "preview-numbering",
    "approve-numbering",
    "inspect-numbering",
];

pub(super) const PREVIEW: &str = concat!(
    "Numbering (A08, read-only preview; approving numbers is A09 below): preview-numbering ",
    "--store ROOT [--plate-series PREFIX:START] [--beam-series PREFIX:START] [--column-series ",
    "PREFIX:START] [--assembly-series PREFIX:START] [--tolerance MM] [--compare-name ",
    "true|false] [--previous TOKEN] [--offset N] [--limit 1..256] (groups plates, beams and ",
    "columns of the verified head that have the same kind, pinned profile, pinned material, ",
    "lengths within the tolerance (default 0.001 mm), roll and holes; the oldest part is the ",
    "reference and groups are numbered in the order of their references; defaults PL:1, B:1, ",
    "C:1 and A:1 (assembly marks are PREFIX-NUMBER, grouped by the groups of all parts they ",
    "hold); a position that an earlier group already takes is overlapping and its parts stay ",
    "unnumbered; preview_token covers everything that changes the proposal, so --previous ",
    "TOKEN reports stale true after an edit that would change the numbering (thickness, ",
    "material, holes, a new or deleted part) and false after a move or a rename; nothing is ",
    "written by the preview, which also reports the stored approval and, per group, whether ",
    "the stored marks of its members are the proposed position)",
);

pub(super) const APPROVAL: &str = concat!(
    "Numbering approval (A09, experimental): approve-numbering --store ROOT --preview-token ",
    "TOKEN [the settings options of preview-numbering] [--op-id UUID] [--revision-id UUID] ",
    "[--actor-id UUID] [--project-id UUID] [--workspace-id UUID] (evaluates the preview again ",
    "against the head and refuses a TOKEN that is no longer the proposal with ",
    "STALE_WORKSPACE, overlapping series and an empty model with INVALID_ARGUMENT; otherwise ",
    "stores the mark of every part and assembly (series, number and the approval record) as ",
    "authored data in the next revision, so equivalent parts share a mark; approving what is ",
    "already stored writes nothing (numbering_approved_unchanged); a retry of the same ",
    "--op-id is a replay; undo-workspace restores the earlier marks); inspect-numbering ",
    "--store ROOT [--offset N] [--limit 1..256] (the approval stored in the head: ",
    "not_approved, current or stale with the reason, and every stored mark; an edit of ",
    "profile, material, dimensions, holes or outline, or a new, copied or deleted part makes ",
    "it stale, a move or a view change does not)",
);
