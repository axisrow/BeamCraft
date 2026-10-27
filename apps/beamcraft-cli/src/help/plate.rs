// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the plate command family.

pub(super) const COMMANDS: &[&str] = &["create-plate", "add-hole", "inspect-elements"];

pub(super) const HELP: &str = concat!(
    "Plate (P01, committed USD revision): create-plate --store ROOT (--width MM --length MM | ",
    "--contour X,Y[,Z];...) --thickness MM [--name TEXT] [--class INT32] [--plane OX,OY,OZ,XX,XY,XZ,YX,YY,YZ] [--translation X,Y,Z] [--orientation-xyzw X,Y,Z,W] ",
    "[--material-digest SHA256] [--hole-rect X,Y,W,H[;...]] [--hole-round X,Y,D[;...]] ",
    "[--hole-segments N] [--new-id UUID] [--op-id UUID] [--revision-id UUID] [--actor-id ",
    "UUID] [--project-id UUID] [--workspace-id UUID] (--width and --length make the origin ",
    "rectangle; --contour (A02.1) takes ordered X,Y or X,Y,Z points separated by semicolons: ",
    "simple counter-clockwise polygons in z = 0 and exact rectangles in other planes are ",
    "evaluated, extruded along their ordered normal; holes on framed rectangles are ",
    "unsupported; clockwise, self-crossing or touching outlines are INVALID_ARGUMENT, nothing ",
    "is reordered or repaired; the origin rectangle written as a contour is the same box; ",
    "inspect-elements lists contour_points_mm and outline rectangle|contour, and width and ",
    "length are the bounding-box extents of a contour; a retry of the same --op-id needs the ",
    "same --new-id and arguments and is replayed); add-hole --store ROOT --id UUID ",
    "(--polygon X,Y;... | --hole-rect X,Y,W,H | --hole-round X,Y,D) [--operation-id UUID] ",
    "[--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID] ",
    "(A04: one through-hole operation record on a plate, replayed with the plate's other ",
    "holes; a round hole is the regular inscribed N-gon of the contract's own 0.05 mm chord ",
    "tolerance, at most 256 sides, so a very large diameter is unsupported; the operation id ",
    "names the record and is the logical operation: a retry with the same arguments is ",
    "replayed, another use of a recorded id is DUPLICATE_ID; a member target is ",
    "UNSUPPORTED_GEOMETRY; one undo unit with the revision it created); inspect-elements ",
    "--store ROOT [--element UUID] (verified head revision, plates, meshes, snapshot; ",
    "--element lists only that element, none when the head does not hold it, names it as ",
    "element_filter and keeps the counts of the whole revision)",
);
