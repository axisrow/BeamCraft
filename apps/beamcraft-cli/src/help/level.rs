// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the level command family.

pub(super) const HELP: &str = concat!(
    "Level (D2, a storey element with no body): create-level --store ROOT --name TEXT ",
    "--elevation MM [--new-id UUID] [--op-id UUID] [--revision-id UUID] [--actor-id UUID] ",
    "[--project-id UUID] [--workspace-id UUID] (a level with its own UUID and a finite ",
    "elevation within 1e9 mm; moves no element); levels --store ROOT (verified head: levels ",
    "in ascending id order, their elevations and the elements assigned); assign with ",
    "update-element --level UUID (one level per plate, beam or column; DANGLING_RELATIONSHIP ",
    "for an unknown level) or --level none; delete-element refuses a level that elements use ",
    "(DEPENDENCY_CONFLICT); copy-element keeps the source's level. Project (D1): the first ",
    "revision records its --project-id (or a generated one) in the root layer; ",
    "inspect-elements reports it as project_id_record; a later command naming another ",
    "--project-id is INVALID_ARGUMENT",
);
