// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the coordinator command family.

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
    "--address 127.0.0.1:PORT --token-file FILE; coordinator-compensate --address ",
    "127.0.0.1:PORT --token-file FILE --workspace NAME --op-id UUID --target-revision ",
    "REVISION_UUID --expected-head REVISION_UUID (compensates NAME's own accepted operation ",
    "with a new forward draft operation using the token actor; preconditions are checked ",
    "against the current head and every intersecting later accepted operation is named in a ",
    "DEPENDENCY_CONFLICT report; the published head never moves: package and publish ",
    "explicitly afterwards; the same request identity replays across this wire and the local ",
    "CLI); coordinator-rebase --address 127.0.0.1:PORT --token-file FILE --workspace NAME --op-id UUID ",
    "--expected-head REVISION_UUID --expected-draft DRAFT_UUID (rebases NAME's unsent draft ",
    "onto the published head with the token actor, the local rebase-workspace semantics; the ",
    "answer is the typed result plus the current head, and the published head never moves: ",
    "publish explicitly afterwards; DEPENDENCY_CONFLICT carries the bounded report until an ",
    "explicit resolution; the same request identity replays across this wire and the local ",
    "CLI)",
);
