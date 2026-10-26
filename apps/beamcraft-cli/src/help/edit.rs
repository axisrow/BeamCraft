// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the edit command family.

pub(super) const COMMANDS: &[&str] = &["update-element", "cut-by-plane"];

pub(super) const HELP: &str = concat!(
    "Update (P04, P08): update-element --store ROOT --id UUID [--name TEXT] [--class INT32] ",
    "[--material-digest SHA256|none] [--thickness MM] [--translation X,Y,Z] ",
    "[--orientation-xyzw X,Y,Z,W|none] [--level UUID|none] [--op-id UUID] [--revision-id ",
    "UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID] (typed patch; a rename ",
    "rewrites only the authored layer, a material change only the quantity record, a ",
    "thickness change the mesh, a move (--translation, absolute, rotation kept) only the ",
    "placement without rebuilding the body; --orientation-xyzw replaces the stored placement ",
    "quaternion (absolute; none resets to the identity) and is validated like on ",
    "creation: a plate keeps its local mesh; a member rebuilds its rotated body; inspect-elements reports stored ",
    "quantities as current, stale or missing)\n",
    "Cut (A03, A-2 record): cut-by-plane --store ROOT --id UUID --operation-id UUID ",
    "--point X,Y,Z --normal X,Y,Z --keep positive|negative [--op-id UUID] [--revision-id ",
    "UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID] (appends one owned ",
    "plane_cut operation record to a plate, beam or column; coordinates are element-local ",
    "millimetres, the plane must strictly cross the element once, the kept half-space is ",
    "--keep; the mesh, volume and mass are rebuilt in the same transaction, the cap face is ",
    "cut:f.p<ordinal>, features the plane removes stop resolving, and one command is one ",
    "undo unit)",
);
