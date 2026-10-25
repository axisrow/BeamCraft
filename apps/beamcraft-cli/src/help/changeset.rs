// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the changeset command family.

pub(super) const COMMANDS: &[&str] = &["create-changeset", "inspect-changeset", "apply-changeset"];

pub(super) const HELP: &str = concat!(
    "Changeset (C01, the publication unit of a workspace): create-changeset --store PUBLISHED ",
    "--workspace-dir DIR [--changeset-id UUID] [--revision-id UUID] [--actor-id UUID] (base, ",
    "unsent operations, complete layer set with base digests, conservative footprint; stored ",
    "as a blob in the workspace, addressed by digest; same ids on an unchanged draft give the ",
    "same digest); inspect-changeset --workspace-dir DIR --changeset SHA256; apply-changeset ",
    "--store PUBLISHED --workspace-dir DIR --changeset SHA256 --actor-id UUID (the trusted ",
    "caller: PERMISSION_DENIED unless it is the changeset's actor; the changeset id is bound ",
    "to its payload before publication, so the same payload replays and another payload is ",
    "OP_ID_REUSE; fast-forward only, BASE_REVISION_MISMATCH otherwise)",
);
