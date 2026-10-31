// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the delete command family.

pub(super) const HELP: &str = concat!(
    "Delete (P08): delete-element --store ROOT --id UUID [--op-id UUID] [--revision-id UUID] ",
    "[--actor-id UUID] [--project-id UUID] [--workspace-id UUID] (removes the whole element; ",
    "DEPENDENCY_CONFLICT while another element links to it, no cascade; the definitions it ",
    "pinned stay)",
);
