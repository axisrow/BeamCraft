// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the coordinator command family.

pub(super) const COMMANDS: &[&str] = &[
    "serve-coordinator",
    "publish-changeset",
    "rebase-coordinator",
    "inspect-coordinator-head",
    "coordinator-compensate",
];

pub(super) const HELP: &str = concat!(
    "Coordinator (C02, off unless started; loopback only; one publish queue; tokens from ",
    "files, at least 32 printable bytes, not group/world-readable on Unix): serve-coordinator ",
    "--store PUBLISHED --workspaces-root DIR --client ACTOR_UUID:TOKEN_FILE [--client ...] ",
    "[--port N] [--max-requests N] (prints a listening line with the address, then serves; ",
    "the token decides the actor, so a client publishes only its own changesets); ",
    "publish-changeset --address 127.0.0.1:PORT --token-file FILE --workspace NAME ",
    "--changeset SHA256 --expected-head REVISION_UUID (NAME is a directory under the ",
    "workspaces root; the expected head must be the changeset's base; BASE_REVISION_MISMATCH ",
    "with the current head when it moved; the same payload replays); coordinator-compensate ",
    "--address 127.0.0.1:PORT --token-file FILE --workspace NAME --op-id UUID ",
    "--target-revision REVISION_UUID --expected-head REVISION_UUID (compensates NAME's own ",
    "accepted operation with a new forward draft operation using the token actor; ",
    "preconditions are checked against the current head and every intersecting later ",
    "accepted operation is named in a DEPENDENCY_CONFLICT report; the published head never ",
    "moves: package and publish explicitly afterwards; the same request identity replays ",
    "across this wire and the local CLI); rebase-coordinator --address 127.0.0.1:PORT ",
    "--token-file FILE --workspace NAME --op-id OP_UUID --expected-head REVISION_UUID ",
    "--expected-draft REVISION_UUID (the C03 rebase of the workspace's unsent draft onto ",
    "the published head; the token's actor must own the workspace, a rebase never ",
    "publishes, a conflict is answered with semantic diagnostics, a moved head is ",
    "BASE_REVISION_MISMATCH with the current head, and the same identity replays its ",
    "recorded outcome); inspect-coordinator-head --address 127.0.0.1:PORT --token-file FILE",
);
