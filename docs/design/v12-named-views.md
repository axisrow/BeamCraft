# V12 model views, several windows, work area

Model views of the model window, extra view windows and the work area, the V12 slice, extended
2026-11-01 with the view construction, view properties and browser work of the
`feat/model-views-properties` branch (#547 view properties, #511 views by points and planes).
Not the parent card's review, the K10 gate or the W01 shared-view gate: views stay local
display state beside one store.

## Result

A view is a view plane with its own display properties. The engine owns the semantics
(`crates/engine/src/views.rs`):

* `ModelView`: a UUIDv7 identity, a name (1 to 128 characters, unique case-insensitively), the
  view plane (a `WorkPlane`; local +Z points towards the viewer, and the plane is also the input
  frame while the view is active), the projection, the visible height at the plane, the depth
  `[above, below]` the plane (`None` shows everything), the render mode, grid visibility, the
  browser folder, a favourite mark, a temporary mark (session only), the hidden element kinds,
  the work area (V15's `view_clipping::WorkArea`), the camera rotation about the view's own Z
  and X (the plane never turns with it), a colour override and an opacity.
* `ViewBook`: at most 256 views and 512 folders (eight path segments each), names and
  identities unique, folders created with their ancestors, `insert_all` adds a batch whole or
  not at all, an empty folder can be deleted and one that holds views cannot.
* Constructors: `coordinate_plane` (XY, XZ or YZ at an offset), `two_points` (vertical) and
  `two_points_in_plane` (across the input plane), `three_points`, `orbit_plane` (a 3D camera),
  `grid_axis` and `grid_views` (U and V families each none, first, last or all, by distance from
  the grid origin), `level_plane` and `level_batch`, `part_plane` and `group_plane` (front, back,
  top, bottom, end or 3D in the part's frame, framed by its evaluated bounds; four views are
  front, top, end and 3D), and `surface_plane` (the first planar face a ray hits, its X along a
  given edge or the face's longest boundary edge).
* `ModelView::contains` and `pick_visible`: what a view shows is inside its depth slab and work
  area and of a visible kind; a clipped or hidden part in front never masks a visible one behind
  it. `ModelView::clipping` turns the slab and the work area into the V15 `Clipping`, so the
  renderer has one clipping path for the cut and the view.

## Session sidecar

`beamcraft_host::views` maps the book to the session JSON (format version 1, rewritten for this
model; the earlier pose-based shape is not read). `beamcraft_ui::views` persists it as a sibling
`<store>.views.json` file, never inside the store, through a `.tmp` rename; temporary views are
never written. A missing file is an empty book and an unreadable one only shows a note; opening
the model never fails over views.

Headlessly the same file is served by `list-views`, `save-view`, `update-view`, `remove-view`
and `apply-view` (`--file`). `save-view` builds a view from exactly one plane option and takes
every display property as an option; grids, levels, parts, assemblies and faces are read from a
read-only `--store`. Batches suffix repeated names. `apply-view` returns the view, its camera
frame and its clip planes and touches no store. Folders, favourites in the browser, windows and
previews need no agent and have no command.

## Window

* The project browser lists the views by folder, with search, quick access (favourites,
  temporary and open views), favourites only, previews, Ctrl and Shift selection, a context
  menu (open, open in a new window, properties, favourite, duplicate, cut, copy, close, delete
  with confirmation), new and empty-folder deletion, and drag to a folder. `{3D}` is the free
  view.
* The View tab: Create view (every construction), View properties, 3D / plane (Ctrl+P),
  Direction (top, bottom, front, back, left, right: the camera turns, the input plane stays),
  New view window, Tile windows, and Redraw views. The command palette lists the same.
* Creating by picking: two points, three points, a face, or a face and an edge are picked in
  the model with the creation snaps; the last pick builds the view. A pick that makes no plane
  is dropped and the earlier ones stay; Esc cancels and Backspace removes the last pick.
* The properties window edits the name, folder, marks, projection, visible height, rotation,
  depth, work area (model or selection bounds), render mode, grids, colour, opacity and object
  visibility; Create or Apply is the only action that changes the book. "Take the current
  camera" saves the camera as the view plane.
* Activating a view builds a plane-aligned camera (`Camera::from_view`), takes its render mode,
  grids and plane, hides its kinds from the scene, tints the model (`View::tint`, a uniform the
  shader reads: a colour replaces the model colour, zero opacity draws nothing, opacity blends in
  the transparent mode), and adds its slab and work area to the cut. Picking and snapping see
  only what it shows. Each view keeps its session camera; Ctrl+Tab cycles the open views; the
  free 3D view and its work plane come back as they were.
* Up to eight windows besides the main viewport show a view or the free 3D view with their own
  camera, cut, tint, hidden kinds, render mode and grid guides; they tile and dock to an edge.
  Selection and tools stay in the main viewport.

## Not in this slice

Shared views and their publication, author, time and revision metadata (W01), drawings from
views, an undeformed member view (the branch's version only checked a straight axis), curved
grid axes, and per-view snapping inside the engine query (the window filters the candidates).
