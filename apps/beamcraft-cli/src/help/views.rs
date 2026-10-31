// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Named-view help (V12).

pub(super) const HELP: &str = "Named views (V12): saved display state in a session JSON file; the model store is never written. list-views --file FILE returns every saved view with its resolved pose. save-view --file FILE --name NAME saves exactly one kind: an orbit view --target X,Y,Z --yaw RAD --pitch RAD --distance MM [--ortho true|false], or a plane view --distance MM resolved once at save time: --plane-grid SELECTOR --plane-line LABEL [--plane-reverse true] --store ROOT (the grid lookup is read-only), --plane-level true --centre X,Y,Z (near top-down), or --from X,Y,Z --to X,Y,Z (look from the first point toward the second). Every view may carry --work-area XMIN,YMIN,ZMIN,XMAX,YMAX,ZMAX. remove-view --file FILE --name NAME deletes one view. apply-view --file FILE --name NAME returns the pose, work area and plane definition a window restores, without touching any store.";
