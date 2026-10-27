// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the copy command family.

pub(super) const COMMANDS: &[&str] = &[
    "copy-element",
    "copy-instance",
    "copy-elements",
    "move-elements",
    "rotate-elements",
    "mirror-elements",
    "array-linear",
    "array-polar",
];

pub(super) const HELP: &str = concat!(
    "Arrays (T03): array-linear --store ROOT --ids UUID,... --step X,Y,Z --count NEW_COPIES; array-polar --store ROOT --ids UUID,... --center X,Y,Z --axis X,Y,Z --angle STEP_RADIANS --count NEW_COPIES. Both accept --instance true|false, --plane and context options; originals remain and one array is one undo unit.\n",
    "Mirror (T03): mirror-elements --store ROOT --ids UUID,... --point X,Y,Z --normal X,Y,Z [--plane ORIGIN,XDIR,YDIR] creates independent reflected copies in one revision.\n",
    "Group transforms (T03): move-elements|copy-elements --store ROOT --ids UUID,... (--vector X,Y,Z | --from X,Y,Z --to X,Y,Z) [--plane ORIGIN,XDIR,YDIR]; copy-elements also accepts --instance true|false. rotate-elements --store ROOT --ids UUID,... --center X,Y,Z --axis X,Y,Z --angle RADIANS [--plane ORIGIN,XDIR,YDIR]. All accept common operation identity options. One atomic revision per group.\n",
    "Copy (P05): copy-element --store ROOT --id UUID [--name TEXT] [--offset X,Y,Z] [--new-id ",
    "UUID] [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID] ",
    "[--workspace-id UUID] (new element with the source's authored inputs, the same body and ",
    "mesh digest; the source is only read; editing the copy later never changes the source)\n",
    "Instance (P11): copy-instance takes the same part options as copy-element. Shape, profile, ",
    "material and operations share the canonical source definition; edits affect the whole group. ",
    "Identity, placement, name, level and other relationships stay per instance. Deleting a ",
    "definition owner with linked instances is refused; delete its instances first.\n",
);
