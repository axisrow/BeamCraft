// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the assembly command family.

pub(super) const COMMANDS: &[&str] = &[
    "create-assembly",
    "attach-to-assembly",
    "detach-from-assembly",
    "set-main-part",
    "explode-assembly",
    "assemblies",
];

pub(super) const HELP: &str = concat!(
    "Assembly (A01/A16): create-assembly --store ROOT --name TEXT [--new-id UUID] [--op-id UUID] ",
    "[--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID] (an ",
    "element of kind assembly with its own UUID and no body); attach-to-assembly --store ROOT ",
    "--id PART_UUID --assembly-id UUID [same ids] (authors the part's one bim:partOf record; ",
    "plate, beam, column or nested assembly; DEPENDENCY_CONFLICT for a part of another ",
    "assembly or a cycle); detach-from-assembly --store ROOT --id PART_UUID [same ids]; ",
    "set-main-part --store ROOT --id PART_UUID --assembly-id UUID [same ids] (A16.1: names the ",
    "assembly's one main part, stored on the assembly; the part must be a direct member, and ",
    "detaching it is DEPENDENCY_CONFLICT until another main part is named); explode-assembly ",
    "--store ROOT --id ASSEMBLY_UUID [same ids] (A16: dissolves one level; the assembly is ",
    "deleted, its direct parts become loose and do not change, sub-assemblies stay assemblies); ",
    "assemblies --store ROOT (verified head: assemblies, their parts, parent and main part). ",
    "delete-element refuses an assembly that still has parts and a main part",
);
