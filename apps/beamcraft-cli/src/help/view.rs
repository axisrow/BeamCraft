// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the view command family.

pub(super) const COMMANDS: &[&str] = &[
    "snapshot-view",
    "inspect-view-changes",
    "save-view",
    "list-views",
    "inspect-view",
    "update-view",
    "delete-view",
    "restore-view",
];

pub(super) const HELP: &str = concat!(
    "Personal views (W01): save-view --workspace-dir DIR --name TEXT [--id UUID] [--position X,Y,Z] [--orientation-xyzw X,Y,Z,W] [--projection orthographic|perspective] [--width MM] [--aspect F] [--fov RADIANS] [--plane ORIGIN,XDIR,YDIR] [--comment TEXT]; list-views --workspace-dir DIR; inspect-view|restore-view|delete-view --workspace-dir DIR --id UUID; update-view --workspace-dir DIR --id UUID [same camera fields]. All accept --expected-views DIGEST|none for optimistic concurrency. Views never change the model revision.\n",
    "View (synthetic kernel plate scene, no store): snapshot-view --plates 1..500 [--hide ",
    "KIND,...] [--representation full|simplified] [--stand-in-simplified true|false] ",
    "[--select ID,...] [--revision SHA256] [--generation U64]; inspect-view-changes --plates ",
    "1..500 --edit move-first|resize-first|add-one|remove-last,... [--since TOKEN] ",
    "[--stand-in-simplified true|false]",
);
