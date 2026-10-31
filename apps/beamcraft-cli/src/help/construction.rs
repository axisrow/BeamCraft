// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help for the T02 construction object commands.

pub(super) const HELP: &str = "Construction objects (T02): create-construction --store ROOT --name TEXT --kind point|line|circle|plane --a X,Y,Z [--b X,Y,Z] [--radius MM] [--new-id UUID] and the usual command-context identity options. A point takes only --a; a line takes --b as its second end; a circle takes --b as its plane axis and --radius; a plane takes --b as its normal. The geometry is authored in world millimetres and the element has the identity placement. Helpers are snap targets only: schedules, numbering and exports never see them. list-construction --store ROOT returns the verified helpers in identity order; delete-element --id HELPER removes one.";
