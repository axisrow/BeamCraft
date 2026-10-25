// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the edit command family.

pub(super) const COMMANDS: &[&str] = &["update-element"];

pub(super) const HELP: &str = concat!(
    "Update (P04, P08): update-element --store ROOT --id UUID [--name TEXT] ",
    "[--material-digest SHA256|none] [--thickness MM] [--translation X,Y,Z] ",
    "[--orientation-xyzw X,Y,Z,W|none] [--level UUID|none] [--op-id UUID] [--revision-id ",
    "UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID] (typed patch; a rename ",
    "rewrites only the authored layer, a material change only the quantity record, a ",
    "thickness change the mesh, a move (--translation, absolute, rotation kept) only the ",
    "placement without rebuilding the body; --orientation-xyzw replaces the stored placement ",
    "quaternion (absolute; none resets to the identity) and is validated like on ",
    "create-member, rebuilding a member's rotated body; inspect-elements reports stored ",
    "quantities as current, stale or missing)",
);
