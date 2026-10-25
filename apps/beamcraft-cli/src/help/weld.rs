// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the weld command family.

pub(super) const COMMANDS: &[&str] = &["create-weld", "welds"];

pub(super) const HELP: &str = concat!(
    "Weld (A06, a connection element with no body): create-weld --store ROOT --name TEXT ",
    "--main PART_UUID --secondary PART_UUID --size MM --location workshop|site --side ",
    "arrow|other|both [--type fillet] [--new-id UUID] [--op-id UUID] [--revision-id UUID] ",
    "[--actor-id UUID] [--project-id UUID] [--workspace-id UUID] (authors the weld's ",
    "properties and its one bim:connects record towards two distinct plates, beams or ",
    "columns, main part first; the parts are only read; no default for size, location or ",
    "side); welds --store ROOT (verified head: welds, parts, properties). delete-element ",
    "refuses a welded part while the weld connects it and deletes a weld",
);
