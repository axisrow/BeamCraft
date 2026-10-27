// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the weld command family.

pub(super) const COMMANDS: &[&str] = &["create-weld", "welds"];

pub(super) const HELP: &str = concat!(
    "Weld (A06, a connection element; A15 adds an optional solid along a path): create-weld ",
    "--store ROOT --name TEXT --main PART_UUID --secondary PART_UUID --size MM --location ",
    "workshop|site --side arrow|other|both [--type fillet] [--path X,Y,Z;X,Y,Z;...] ",
    "[--all-around true|false] [--contour none|flush|concave|convex] [--new-id UUID] ",
    "[--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID] ",
    "[--workspace-id UUID] (authors the weld's properties and its one bim:connects record ",
    "towards two distinct plates, beams or columns, main part first; the parts are only ",
    "read; no default for size, location or side; --path derives the fillet solid along the ",
    "polyline and stores the symbol data, --all-around and --contour need --path); welds ",
    "--store ROOT (verified head: welds, parts, properties, path length and volume). ",
    "delete-element refuses a welded part while the weld connects it and deletes a weld",
);
