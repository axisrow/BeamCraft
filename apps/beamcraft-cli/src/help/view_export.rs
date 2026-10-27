// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the view export command family.

pub(super) const COMMANDS: &[&str] = &["export-view"];

pub(super) const HELP: &str = concat!(
    "Display options (V16/V17): --render-mode wireframe|transparent|grey|shaded|selected-only [--select UUID,... | --select-file FILE.json] [--representation SET_UUID] [--overlay FILE.json]. Display options emit BCVXFRM3 JSON v3 plus the unchanged engineering bundle. Empty selection hides all in selected-only. Overlay is an array of {id:UUID,color:[R,G,B],transparency:0..1}, has final priority and is never saved.\n",
    "View export (V03, input of the native viewport): export-view --store ROOT --out FILE|- ",
    "[--with-definitions true|false] [--with-element UUID] [--with-grids true|false] [--view UUID] (the verified head's pinned view ",
    "snapshot as one engine bundle; --view UUID requires a personal workspace store and emits BCVXFRM3 with four length-prefixed JSON sections (definitions, element, grids, presentation), then the unchanged engineering bundle. Presentation supplies the resolved camera, effective clip planes and visible-ID mask. FILE output also retains that frame. Legacy exports keep their existing protocol. ",
    " - writes the raw bundle to standard output, a FILE is ",
    "created new and a JSON summary printed; --with-definitions true and --with-element, only ",
    "with --out -, frame replies of the same inspection before the bundle: BCVXFRM1, then for ",
    "the definitions and then the inspect-elements --element reply a little-endian u32 length ",
    "and the reply as that command prints it, 0 for one not asked, then the bundle; ",
    "--with-grids true uses BCVXFRM2 and adds a third section with pinned grid display data)",
);
