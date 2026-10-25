// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the member command family.

pub(super) const COMMANDS: &[&str] = &["create-member"];

pub(super) const HELP: &str = concat!(
    "Member (P03): create-member --store ROOT --start X,Y,Z --end X,Y,Z (--profile-digest ",
    "SHA256 | --profile NAME) [--kind beam|column] [--roll RAD] [--material-digest SHA256 | ",
    "--material NAME] [--name TEXT] [--translation X,Y,Z] [--orientation-xyzw X,Y,Z,W] ",
    "[--new-id UUID] [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id ",
    "UUID] [--workspace-id UUID] (straight 3D member: start and end may differ in X, Y and Z ",
    "(millimetres, Z up); for example --start 100,200,300 --end 3100,4200,12300; with a ",
    "pinned rectangle or A07.1 section profile, built from the detailed outline; ",
    "--orientation-xyzw is the placement quaternion in x,y,z,w order, normalized and ",
    "validated: a non-normalized, zero or non-finite quaternion is INVALID_ARGUMENT; the ",
    "section frame is rolled by --roll about the axis first and then mapped by the ",
    "quaternion, so both compose; rigid placement leaves the local mesh and quantities ",
    "unchanged; inspect-elements lists plates and members)",
);
