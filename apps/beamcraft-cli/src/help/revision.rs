// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the revision command family.

pub(super) const COMMANDS: &[&str] = &[
    "commit-revision",
    "open-revision",
    "recover-revision",
    "inspect-revision-outcome",
];

pub(super) const CONTROL: &str = concat!(
    "Control (blob and revision commands): [--timeout-ms 1..120000] [--cancel-after-ms ",
    "0..120000]",
);

pub(super) const HELP: &str = concat!(
    "Revision: commit-revision --store ROOT --project-id UUID --workspace-id UUID ",
    "--revision-id UUID --op-id UUID --generation U64 --base none|REVISION_UUID --schema ",
    "SHA256 --root-layer NAME --layers ROLE:NAME:SHA256,... (ROLE authored|derived|external; ",
    "blobs must be stored first); open-revision --store ROOT",
);

pub(super) const RECOVERY: &str = concat!(
    "Recovery: recover-revision --store ROOT --min-age-secs U64 (0 asserts that no commit is ",
    "running; closes unresolved intents from the root, removes abandoned temporary files, ",
    "reports unreferenced blobs, deletes no blob); inspect-revision-outcome --store ROOT ",
    "--op-id UUID (host-ledger QueryOutcome: committed | aborted | not recorded | ",
    "OUTCOME_UNKNOWN)",
);
