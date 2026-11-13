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
  view plane (a `WorkPlane`; local +Z points towards the viewer, and the plane of a plan or an
  elevation is also the input frame while the view is active), the projection, the visible
  height at the plane, the depth
  `[above, below]` the plane (`None` shows everything), the render mode, grid visibility, the
  browser folder, a favourite mark, a temporary mark (session only), the hidden element kinds,
  the work area (V15's `view_clipping::WorkArea`), the camera rotation about the view's own Z
  and X (the plane never turns with it), a colour override, an opacity and the default mark.
  A model has exactly one default view (the one it opens on, saved with the model): inserting or
  updating a view with the mark moves it, the default cannot be removed or unmarked, and it is
  never temporary. A window that finds none (a new model, or a file from before the default)
  makes `3D View` from its camera. A view without a name in the window is temporary.
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

`beamcraft_host::views` maps the book to the session JSON (format version 2; version 1 files,
which have no filters, overrides or presets, are still read). `beamcraft_ui::views` persists it
as a sibling `<store>.views.json` file, never inside the store, through a `.tmp` rename;
temporary views are never written. A missing file is an empty book and an unreadable one only shows a note; opening
the model never fails over views.

Headlessly the same file is served by `list-views`, `save-view`, `update-view`, `remove-view`
and `apply-view` (`--file`). `save-view` builds a view from exactly one plane option and takes
every display property as an option; grids, levels, parts, assemblies and faces are read from a
read-only `--store`. Without `--name` a single view is named `View N` (the lowest free number)
and a batch by its labels alone; the window's new-view form starts with the same name. Batches
suffix repeated names. `--default true` marks the one default view (it moves the mark; `remove-view`
and `--default false` on the default are refused). `apply-view` returns the view, its camera
frame and its clip planes and touches no store. Filters and presets have their own commands
(below). Folders, favourites in the browser, tabs and previews need no agent and have no command.

## Filters, overrides and view presets

`beamcraft_engine::view_filter` (re-exported by `views`):

* `ViewFilter`: a name (1 to 64 characters, unique case-insensitively, no `;`, `|` or `:`), all
  or any, and at most 16 rows `field op value` over the V14 fields (`type`, `name`, `class`,
  `attribute:<name>`) and operators. A row is admitted by its field (a known type token for
  `type eq|ne`, a whole number for `class`, text operators for text fields); the V14 evaluator
  stays the only matching semantics: `beamcraft_host::views::filter_rule` turns the rows into the
  rule JSON `query-elements` evaluates, so a filter the evaluator refuses is reported, never
  matched by a second path. An attribute value is a number, `true`/`false`, a number with its
  unit (`250 mm`) or text.
* `FilterOverride`: one filter bound to one view, whether what it matches is visible, an
  optional colour and a transparency. A view lists at most 16, each filter once; the first
  override whose filter matches an element decides how it is drawn, as Revit's view filters do.
  The book admits a view only when its overrides name filters it has; renaming a filter follows
  into every view, removing it drops its overrides. At most 64 filters per book.
* Presets are the V07 discipline presets (`beamcraft_engine::presets::ViewPreset`), one
  mechanism for the CLI and the window: hidden kinds, depth slab, representation, colour set,
  grid toggle and the filter overrides, never the camera. Applying one copies its values and
  keeps no link; its overrides must name filters of the views file it is applied in.

The filters and the overrides are stored in the W01 views layer beside the camera prims:
`bim:view:filterOverrides` (`string[]` of override specs) on each camera and `bim:view:filters`
(`string[]`, one compact filter JSON each, in book order) on the views scope. A legacy JSON
sidecar may carry an optional `filters` list and per-view `overrides`; the next save migrates
them into the layer.

The environment's presets file may declare filters (`steel:preset:kind = "filter"`,
`steel:preset:conditions` as `string[]` rows, optional `steel:preset:match`) and view presets
(`"view"`, the display properties as `steel:preset:*` attributes and the overrides as
`filter|show|r,g,b|transparency` strings) beside the part presets; `definitions` lists them as
`environment_view_filters` and `environment_view_presets`; a view preset entry carries the V07 bundle (`depthMm`,
`representation`, `colourSet`, `grids`, `hiddenKinds`, `overrides`). The blank environment ships Beams,
Columns, Plates and Steel parts, and the presets 3D by part type, Plan, Plan with columns faded
and Section. An environment filter is copied into the model's views the first time a view uses
it. Headlessly: `list-view-filters`, `save-view-filter` (`--conditions "type eq beam;class ge
2"`, `--match`, `--rename-from`) and `remove-view-filter`; `save-view` and `update-view` take
`--overrides`; `define-preset` takes `--overrides`, and `apply-preset` fills a view from a preset.
The window keeps the user's own presets in `view-presets.json` in the configuration directory.

In the window (`beamcraft_ui::view_overlay`) the filters of the open views are evaluated on a
worker by the read-only `query-elements`, kept per store head, and turned into each view's
overlay over the colour set shown (V17): matched elements take the override's colour, which
replaces the model colour, and its transparency (seen in the transparent display); hidden ones
leave the view's scene and cannot be picked (`views::pick_visible_except`).

## Window

* The project browser is a tree of folders, as Revit's browser organised by folder: the
  default view heads it on its own row with a house icon, then the folders and the views, each
  by name. A view's kind is the orientation of its plane, shown by its icon: plan
  (horizontal), elevation or section (vertical; a section has a limited depth) or inclined.
  Every view can be orbited in 3D, so 3D is a way of looking, not a kind. A new folder (a
  folder's menu, the free space's menu, or the toolbar inside the selected view's folder) is
  stored as `<parent>/<name>`. A click selects a view and shows its properties, a double click
  opens it as a tab, Ctrl and Shift extend the selection, a view dragged onto a folder moves
  into it and onto the free space under the list moves out to the top level; dragged into the
  work area it opens
  where it is dropped. A view's menu: Open, Open beside, Properties, Duplicate, Apply view
  preset, Save as view preset, favourite, Make default, Close, Delete with confirmation; a
  folder's: New folder…, Open all, Close all, Delete empty folder. Search, quick access,
  favourites only and previews (the filter menu) filter the tree.
* The View tab: Create view (an icon dropdown of every construction),
  3D / plane (Ctrl+P, shown in its tooltip),
  Direction (top, bottom, front, back, left, right: the camera turns, the input plane stays),
  Windows (Open beside, Close inactive, Tab views, Tile views, Arrange: Tabs, Tile, Tile
  horizontal, Tile vertical, Maximize / restore), Display, Filters (the filter editor), Colour,
  Cut and Redraw views. The command palette lists the same, and "View properties".
* Creating by picking: two points, three points, a face, or a face and an edge are picked in
  the model with the creation snaps; the last pick fills the view plane and Apply creates the
  view. A pick that makes no plane is dropped and the earlier ones stay; Esc cancels the pick
  and Backspace removes the last pick. Apply refuses while a pick runs or after a cancelled one.
* The view properties live in the right panel, laid out as a properties grid: a type box (the
  view's kind and name), the name with the favourite and default marks, the folder, View preset
  (fills the form from the model's or the environment's presets; + saves the form as a preset),
  then groups: Filters (the view's overrides in order with visible, colour and transparency,
  raise and remove; Add filter…, New filter…; a click on a filter's name edits it), Graphics
  (colour and opacity), Object visibility, Camera (projection, height, rotation) and Extents
  (depth, work area). Clicking empty space in the viewport shows them, a click in the browser
  too; picking a part shows the part's own properties instead. Create view fills a "New view"
  form there, which stays while a part is picked. Apply is the only action that changes the
  book, and after a create the panel shows the new view's properties. An empty click keeps
  unsaved edits of the view already shown; Esc, Cancel or the Select tool close the panel.
  "Take the current camera" saves the camera as the view plane.
* The filter editor takes the panel's place: pick or start a filter, its name, match all or
  any, and one row per condition (field, operator, value; the type value is a list of element
  types), Add condition, Save filter, Delete and Back. A new filter saved from a view's form
  joins that form, and Apply adds it to the view.
* Activating a view builds a plane-aligned camera (`Camera::from_view`), takes its render mode,
  grids and plane, hides its kinds from the scene, tints the model (`View::tint`, a uniform the
  shader reads: a colour replaces the model colour, zero opacity draws nothing, opacity blends in
  the transparent mode), and adds its slab and work area to the cut. Picking and snapping see
  only what it shows. Each view keeps its session camera; Ctrl+Tab cycles the open views; the
  default view fits the model like a free camera (one saved with its own target opens on that
  target, so its saved height is not replaced by the fit) and keeps the model's own work plane, which
  comes back as it was when another view is left. An inclined view keeps the model's work
  plane too: an orbit camera's plane is only the direction it looks along, and a member drawn
  on it floated above the grid and dived under the slab (the 584 recheck on master). A plan
  and an elevation or section install their plane; an inclined plane to work on is installed
  through the Work plane section (three points, a picked face).
* The work area holds up to 16 open views as tabs in at most eight tab groups
  (`beamcraft_ui::dock`), drawn inside it and never over the side panels. Each tab shows a view
  with its own camera, cut, tint, hidden kinds, filter overlay, render mode and grid guides. A
  click on a tab shows it and makes it the active view; its cross or a middle click closes it;
  a double click maximizes its group and back. A tab dragged onto the middle or strip of a group
  moves into it, onto a group's edge splits that group and takes the new side; the place it
  would go is shaded while it is dragged, and a group whose last tab leaves closes. Dividers
  between groups resize them (each side keeps at least a tenth). The active group's frame and
  tab are highlighted, and selection, picking, snapping and every tool work in its shown view.
  A primary press focuses a group's view (a press on a popup or dialog over the work area does
  not); with a tool armed that click only focuses, and the next one places. Scroll zoom and
  middle-drag pan act on the view under the pointer without focusing it. New views open as tabs
  of the active group; Open beside opens one in a new group on the right. The last tab may be
  closed, which leaves "No view is open." and "Open default view". Arrange: Tabs puts every view
  in one group; Tile lays one group per view in a square-ish grid (four make 2 x 2); Tile
  horizontal stacks them and Tile vertical sets them side by side, the active view first; views
  past eight join the last group. The layout is not saved.

## Not in this slice

Shared views and their publication, author, time and revision metadata (W01), drawings from
views, an undeformed member view (the branch's version only checked a straight axis), curved
grid axes, and per-view snapping inside the engine query (the window filters the candidates).
