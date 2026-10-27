// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the copy command family.

pub(super) const COMMANDS: &[&str] = &["copy-element", "copy-instance"];

pub(super) const HELP: &str = concat!(
    "Copy (P05): copy-element --store ROOT --id UUID [--name TEXT] [--offset X,Y,Z] [--new-id ",
    "UUID] [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID] ",
    "[--workspace-id UUID] (new element with the source's authored inputs, the same body and ",
    "mesh digest; the source is only read; editing the copy later never changes the source)\n",
    "Instance (P11): copy-instance takes the same part options as copy-element. Shape, profile, ",
    "material and operations share the canonical source definition; edits affect the whole group. ",
    "Identity, placement, name, level and other relationships stay per instance. Deleting a ",
    "definition owner with linked instances is refused; delete its instances first.\n",
);
