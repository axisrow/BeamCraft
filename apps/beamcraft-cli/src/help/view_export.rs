// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the view export command family.

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
pub(super) const SCREENSHOT_HELP: &str = concat!(
    "View screenshot: screenshot-view --store ROOT --out FILE [--width U32] [--height U32] ",
    "[--samples 1|4] [--yaw RAD] [--pitch RAD] [--projection perspective|ortho] [--zoom F32] ",
    "[--pan X,Y] [--colour-set TEXT] [--background light|dark] (one viewport render of the ",
    "verified head through the native renderer in the beamcraft-ui binary, which this command ",
    "finds next to its own executable; the camera is fitted over the whole scene, --yaw and ",
    "--pitch, radians, point it from another direction, pitch above the horizon, --projection ",
    "ortho switches to the orthographic projection, --zoom is the logarithmic zoom and --pan ",
    "X,Y shifts the fitted target in screen-plane millimetres; --colour-set names a built-in ",
    "set, Catalogue (the default) or Part classes, or a colour-sets file holding exactly one ",
    "set of its own, and --background light renders over a light paper tone instead of the ",
    "dark clear colour; FILE is created new, never overwritten, and one JSON ",
    "summary is printed; default size is 1280x800; needs a native graphics adapter, so exit ",
    "code 3 reports that the render backend is unavailable)",
);
