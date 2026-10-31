// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the history command family.

pub(super) const HELP: &str = concat!(
    "History (U14): inspect-history --store ROOT [--workspaces-root DIR] [--workspace-dir ",
    "DIR] (revisions, blob bytes in total, reachable from the head and from the workspaces, ",
    "unreachable blobs, revision-log and ledger size; read-only); compact-history --store ",
    "ROOT (--keep-head | --keep N) --dry-run (changes nothing; reports bytes freed, revisions ",
    "removed and the undo points that would be lost); compact-history ... --yes (--backup DIR ",
    "| --no-backup) (--workspaces-root DIR | --workspace-dir DIR | --no-workspaces) ",
    "[--ledger-tail N] [--orphan-grace-secs N] (deletes history for good: refused without ",
    "--yes, without a backup decision, with a live workspace based on a removed revision, ",
    "while a coordinator holds the store or an unresolved intent exists)",
);
