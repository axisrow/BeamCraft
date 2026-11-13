// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the view export command family.

pub(super) const HELP: &str = concat!(
    "View export (V03, input of the native viewport): export-view (--store ROOT | --source ",
    "FILE) --out FILE|- [--with-definitions true|false] [--with-element UUID] [--with-grids ",
    "true|false] (the verified head's pinned view snapshot as one engine bundle; - writes the ",
    "raw bundle to standard output, a FILE is created new and a JSON summary printed; ",
    "--with-definitions true and --with-element, only with --out -, frame replies of the same ",
    "inspection before the bundle: BCVXFRM1, then for the definitions and then the ",
    "inspect-elements --element reply a little-endian u32 length and the reply as that ",
    "command prints it, 0 for one not asked, then the bundle; --with-grids true uses ",
    "BCVXFRM2 and adds a third section with pinned grid display data; a plate kept without ",
    "geometry, such as a degenerate contour, is left out of the bundle and the JSON summary ",
    "lists it in skipped_plates; --source FILE reads a received IFC file instead of a store ",
    "and exports the same bundle from its placed tessellated bodies, pinned to the file's own ",
    "SHA-256, so the same bytes give the same bundle; the framed extras need a store and are ",
    "refused with a source)",
);
pub(super) const SCREENSHOT_HELP: &str = concat!(
    "View screenshot: screenshot-view (--store ROOT | --source FILE) --out FILE [--width U32] ",
    "[--height U32] [--samples 1|4] [--yaw RAD] [--pitch RAD] [--projection perspective|ortho] ",
    "[--zoom F32] [--pan X,Y] [--colour-set TEXT] [--background light|dark] [--grids ",
    "true|false] (one viewport render of the verified head through the native renderer in the ",
    "beamcraft-ui binary, which this command finds next to its own executable; the project ",
    "grids are drawn with their labels as the window draws them and the camera is fitted over ",
    "the whole scene and its grids, --grids false leaves the grids out of both, --yaw and ",
    "--pitch, radians, point it from another direction (the direction from the target to the ",
    "camera is (cos yaw cos pitch, sin yaw cos pitch, sin pitch), so yaw turns about +Z from +X ",
    "and pitch lifts above the horizon), --projection ",
    "ortho switches to the orthographic projection, --zoom EXP is the logarithmic zoom: ",
    "lengths in the frame scale by e to the power of EXP, so 1 is about a 2.7x close-up, 2 ",
    "about 7.4x, a negative value zooms out, and the accepted range is -10 to 10; --pan ",
    "MM,MM shifts the fitted target in screen-plane millimetres, +X to the right and +Y up; ",
    "--colour-set names a built-in ",
    "set, Catalogue (the default) or Part classes, or a colour-sets file holding exactly one ",
    "set of its own, and --background light renders over a light paper tone instead of the ",
    "dark clear colour; FILE is created new, never overwritten, and one JSON ",
    "summary is printed; default size is 1280x800; needs a native graphics adapter, so exit ",
    "code 3 reports that the render backend is unavailable; a refusal of the store by the CLI ",
    "export, such as UNSUPPORTED_VERSION, is relayed with its own code, message and ",
    "diagnostics; --source FILE renders a received IFC file the same way, read-only: the CLI ",
    "exports its display bundle, no store is opened, no project grids or annotations are ",
    "drawn, and only the Catalogue colour set applies)",
);
