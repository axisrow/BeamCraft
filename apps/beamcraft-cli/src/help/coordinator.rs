// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the coordinator command family.

pub(super) const COMMANDS: &[&str] = &[
    "serve-coordinator",
    "publish-changeset",
    "inspect-coordinator-head",
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
    "with the current head when it moved; the same payload replays); inspect-coordinator-head ",
    "--address 127.0.0.1:PORT --token-file FILE",
);
