// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the rebase command family.

pub(super) const COMMANDS: &[&str] = &["rebase-workspace"];

pub(super) const HELP: &str = concat!(
    "Rebase (C03): rebase-workspace --store PUBLISHED --workspace-dir DRAFT --op-id UUID ",
    "--expected-head UUID --expected-draft UUID [--actor-id UUID] (keeps independent edits in ",
    "a new personal draft, never publishes automatically; explicit semantic conflicts keep ",
    "both sides, are recorded, and block publish and packaging until the draft is resolved; ",
    "retry uses the same operation and expected pins)",
);
