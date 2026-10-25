// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the view command family.

pub(super) const COMMANDS: &[&str] = &["snapshot-view", "inspect-view-changes"];

pub(super) const HELP: &str = concat!(
    "View (synthetic kernel plate scene, no store): snapshot-view --plates 1..500 [--hide ",
    "KIND,...] [--representation full|simplified] [--stand-in-simplified true|false] ",
    "[--select ID,...] [--revision SHA256] [--generation U64]; inspect-view-changes --plates ",
    "1..500 --edit move-first|resize-first|add-one|remove-last,... [--since TOKEN] ",
    "[--stand-in-simplified true|false]",
);
