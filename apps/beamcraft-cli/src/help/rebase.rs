// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the rebase and compensation command families.

pub(super) const HELP: &str = concat!(
    "Rebase (C03): rebase-workspace --store PUBLISHED --workspace-dir DRAFT --op-id UUID ",
    "--expected-head UUID --expected-draft UUID [--actor-id UUID] (keeps independent edits in ",
    "a new personal draft, never publishes automatically; explicit semantic conflicts keep ",
    "both sides, are recorded, and block publish and packaging until the draft is resolved; ",
    "retry uses the same operation and expected pins). Compensation (C06): ",
    "compensate-workspace --store PUBLISHED --workspace-dir DRAFT --target-revision UUID ",
    "--op-id UUID --expected-head UUID [--actor-id UUID] (appends a new forward draft ",
    "operation restoring the target's own accepted effect; the workspace must sit on the ",
    "current accepted head, and any accepted operation after the target that intersects the ",
    "target's footprint is refused by name; the same operation identity replays, and the ",
    "compensated draft publishes through the ordinary path)",
);
