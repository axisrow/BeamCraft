// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the member command family.

pub(super) const HELP: &str = concat!(
    "Member (P03): create-member --store ROOT --start X,Y,Z --end X,Y,Z (--profile-digest ",
    "SHA256 | --profile NAME) [--kind beam|column] [--roll RAD] [--material-digest SHA256 | ",
    "--material NAME] [--name TEXT] [--class INT32] [--plane OX,OY,OZ,XX,XY,XZ,YX,YY,YZ] [--translation X,Y,Z] [--orientation-xyzw X,Y,Z,W] ",
    "[--axis-lock none|x|y|z] ",
    "(T04 coordinates: X,Y,Z absolute; @DX,DY,DZ relative; @DISTANCE<ANGLE[,DZ] polar, ",
    "millimetres/radians; start is relative to zero, end to start; lock constrains end) ",
    "[--section-plane left|centre|right] [--section-plane-offset MM] [--section-depth front|centre|back] ",
    "[--section-depth-offset MM] [--start-offset MM] [--end-offset MM] ",
    "(P14: position of the section about the member axis, element-local and turned by the roll ",
    "and the placement; left/front are the negative halves of the section frame, right/back the ",
    "positive ones; the offsets add to the position; start/end offsets move the solid's faces ",
    "along the axis, so its length follows and must stay positive; absent flags are the centred ",
    "default; the rotation about the axis is --roll) ",
    "[--new-id UUID] [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id ",
    "UUID] [--workspace-id UUID] (straight 3D member: start and end may differ in X, Y and Z ",
    "(millimetres, Z up); for example --start 100,200,300 --end 3100,4200,12300; with a ",
    "pinned rectangle or A07.1 section profile, built from the detailed outline; ",
    "--orientation-xyzw is the placement quaternion in x,y,z,w order, normalized and ",
    "validated: a non-normalized, zero or non-finite quaternion is INVALID_ARGUMENT; the ",
    "section frame is rolled by --roll about the axis first and then mapped by the ",
    "quaternion, so both compose; rigid placement leaves the local mesh and quantities ",
    "unchanged; inspect-elements lists plates and members)\n",
    "Curved beam (P13): create-curved-beam with --points \"START;ON;END\" or --start X,Y,Z --end X,Y,Z --radius MM --normal X,Y,Z; shared profile/material/identity/placement options as create-polybeam. Surface chord bound is 0.05 mm; lengths are true arcs, uncut volumes use effective section area times true arc length. ",
    "Polybeam (P12): create-polybeam --store ROOT --points \"X,Y,Z;X,Y,Z[;...]\" ",
    "(--profile-digest SHA256 | --profile NAME) [--kind beam|column] [--material-digest ",
    "SHA256 | --material NAME] [--name TEXT] [--class INT32] [--translation X,Y,Z] ",
    "[--orientation-xyzw X,Y,Z,W] [--new-id UUID] [--op-id UUID] [--revision-id UUID] ",
    "[--actor-id UUID] [--project-id UUID] [--workspace-id UUID] (a member along a polyline ",
    "path of 3 to 1024 absolute three-component points, one pinned section along the whole ",
    "path, every interior corner mitred; the volume is the section area times the path ",
    "length; points are absolute element-local millimetres, coordinate expressions and grid ",
    "placement are not supported here; update-element --axis-point replaces the path)",
);
