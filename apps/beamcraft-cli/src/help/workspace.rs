// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the workspace command family.

pub(super) const COMMANDS: &[&str] = &[
    "create-workspace",
    "inspect-workspace",
    "publish-workspace",
    "resolve-workspace",
    "undo-workspace",
    "redo-workspace",
    "inspect-workspace-history",
];

pub(super) const HELP: &str = concat!(
    "Workspace (P06, a personal draft is a separate store directory; edit it with the usual ",
    "commands and --store DIR): create-workspace --store PUBLISHED --workspace-dir DIR ",
    "[--workspace-id UUID] [--user-id UUID] [--revision-id UUID] [--op-id UUID] (forks the ",
    "published head); inspect-workspace --store PUBLISHED --workspace-dir DIR (base, unsent ",
    "revisions, whether the published head moved, and pending_conflicts); publish-workspace ",
    "--store PUBLISHED --workspace-dir DIR (the published revision id is the draft head's id, ",
    "so a retry is the same request; fast-forward only: BASE_REVISION_MISMATCH when the ",
    "published head moved, no merge; DEPENDENCY_CONFLICT while unresolved semantic conflicts ",
    "stand, naming them in diagnostics); resolve-workspace --workspace-dir DIR --op-id UUID ",
    "--discard UUID --expected-draft UUID (explicitly renounce one named own unsent operation ",
    "and every unsent operation after it: the resolution restores the state before it and ",
    "records a durable receipt with its own identity; later rebase and publication exclude the ",
    "renounced work and keep the accepted side untouched; repeating the same identity replays ",
    "the recorded outcome, another payload under it is OP_ID_REUSE)",
);

pub(super) const UNDO: &str = concat!(
    "Undo (P07, durable undo and redo of the draft; each is one new verified revision with ",
    "the restored layers): undo-workspace --workspace-dir DIR [--revision-id UUID] [--op-id ",
    "UUID]; redo-workspace --workspace-dir DIR [same]; inspect-workspace-history ",
    "--workspace-dir DIR (undo and redo units; an edit after an undo discards redo; ",
    "INVALID_ARGUMENT when there is nothing to undo or redo)",
);
