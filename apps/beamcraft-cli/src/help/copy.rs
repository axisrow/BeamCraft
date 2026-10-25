// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the copy command family.

pub(super) const COMMANDS: &[&str] = &["copy-element"];

pub(super) const HELP: &str = concat!(
    "Copy (P05): copy-element --store ROOT --id UUID [--name TEXT] [--offset X,Y,Z] [--new-id ",
    "UUID] [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID] ",
    "[--workspace-id UUID] (new element with the source's authored inputs, the same body and ",
    "mesh digest; the source is only read; editing the copy later never changes the source)",
);
