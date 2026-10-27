// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the view export command family.

pub(super) const COMMANDS: &[&str] = &["export-view"];

pub(super) const HELP: &str = concat!(
    "View export (V03, input of the native viewport): export-view --store ROOT --out FILE|- ",
    "[--with-definitions true|false] [--with-element UUID] [--with-grids true|false] (the verified head's pinned view ",
    "snapshot as one engine bundle; - writes the raw bundle to standard output, a FILE is ",
    "created new and a JSON summary printed; --with-definitions true and --with-element, only ",
    "with --out -, frame replies of the same inspection before the bundle: BCVXFRM1, then for ",
    "the definitions and then the inspect-elements --element reply a little-endian u32 length ",
    "and the reply as that command prints it, 0 for one not asked, then the bundle; ",
    "--with-grids true uses BCVXFRM2 and adds a third section with pinned grid display data)",
);
