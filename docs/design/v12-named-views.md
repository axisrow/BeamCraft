# V12 named views, several windows, work area

Named views of the model window, extra view windows and the work area, the V12 slice. Not the
parent card's review or the K10 gate; the cut plane and its renderer clipping stay with V15.

## Result

The window saves, restores and deletes named views; a view is display state only — nothing in
this slice reads or writes a store, and a saved view restores exactly the pose, projection and
work area it captured. The engine owns the semantics (`crates/engine/src/views.rs`):
`ViewPose` (target, yaw, pitch, distance, orthographic — the orbit camera's own bounds),
`PlaneSpec` (grid line, level or two points), `NamedView` and the bounded `ViewBook` (128
views, names trimmed to 64 bytes, ordered by name). The work area is V15's
`view_clipping::WorkArea`, re-exported here: one type is admitted once and serves both the
saved view and the cut record.

A plane view resolves once at save time, so applying it needs no store:
`resolve_plane` walks `grid_query` for the grid line's plane (eye along the horizontal normal,
target at the line's world midpoint at the frame centre's height), centres a near-top-down
level view on the frame centre (pitch `PITCH_MAX_RAD`, yaw 0 — the orbit basis has no exact
nadir), or looks from the first of two points toward the second (a view from below flattens to
horizontal, because the orbit pitch never goes negative).

## Session sidecar

`beamcraft_host::views` maps the book to the session JSON (the F08 table allows JSON here, not
in the engine); `beamcraft_ui::views` persists it as a sibling `<store>.views.json` file —
never inside the store — written through a `.tmp` rename. A missing file is an empty book and
an unreadable one only shows a note; opening the model never fails over views. Headlessly the
same file is served by `list-views`, `save-view`, `remove-view` and `apply-view` (`--file`).
`save-view` resolves a grid-line view through a read-only `--store`; `apply-view` returns the
saved pose and touches no store, which is the acceptance proof that a view never changes the
model.

## Windows and panels

The saved-views window (View tab's VIEWS group) lists the book with apply and delete, saves the
current camera, builds a plane view from the displayed grids or typed points, and edits the
work area and projection — all through explicit buttons; typing never moves the camera. The
project browser lists the same views. Up to three extra view windows (`View 2` to `View 4`)
draw the loaded scene through their own camera and offscreen targets, with the same navigation
presets; selection and tools stay in the main viewport, and in them `selected only` falls back
to shaded. `Camera::pose`/`set_pose` (`apps/beamcraft-ui/src/camera.rs`) are the exact bridge,
so a restore is bit-equal; the orbit clamp's lower pitch bound widened to 0 so level views
stay level until the user orbits.

The work area is data in this slice: carried per view, editable, restored. V15 renders the box
and clips with it through the same `view_clipping::WorkArea`.
