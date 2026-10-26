# T01 contract v0.1: project grids (axes)

Status: **proposed additive amendment to the F04 0.1.0 model and F10 0.2.0 kind registry; revised 2026-10-26 after the user delegated remaining MVP defaults to project best practices and conventions.**
[F04](f04-model-and-commands-v0.1.md) and the current [F10](f10-domain-vocabulary-v0.2.md) baseline remain unchanged. On acceptance, this versioned T01 contract adds the shared `grid` kind, its stored attributes, commands, limits and diagnostic reasons without changing existing element rules. Dated pointers to this extension are then added to F04 and F10. It extends the draft delivered for [issue #192](https://github.com/Apibim-com/BeamCraft/issues/192), and governs T01.1 to T01.7 in [`tasks.md`](../plan/tasks.md). Development starts with grids ([decisions](../plan/decisions.md)).

The MVP defaults below settle O-G7 and O-G11 to O-G16. Magnetic mode (O-G5/T01.8) remains deferred behind its scope and ownership dependencies. Nothing here is implemented.

Today the repository only says that the `Grids` layer holds "axes and levels"
([architecture](../architecture.md)), that levels are elements ([D2](../design/d1-d2-project-level.md))
and that shared grids change only by an explicit authorized command (F10 0.2 `DV-SCOPE-004`,
F12 `PC-OWN-002`). This document says what a grid is, how it is stored, built, edited and used.

## 1. Owner decisions that shape this draft

| # | Decision (owner, 2026-10-23) | Where |
|---|------------------------------|-------|
| D1 | A grid is a **set of curves in two families, U and V**, taken from the IFC schema (`IfcGrid`: `UAxes`, `VAxes`; each `IfcGridAxis` has a tag and a curve). Each line is a **GridLine** with a name such as `A` or `1` | 2 |
| D2 | In the MVP a GridLine is a **straight line**. Arcs and other curves come after the MVP | 2 |
| D3 | A point on an intersection is written as **one parameter** | 8.2 |
| D4 | Axes are **records inside one grid element**, not elements of their own | 5.1 |
| D5 | A family keeps its **source expression** (`0 5*6000`) as well as the lines, so a step can be edited fast | 4 |
| D6 | Per-line extents of regular grids and partial arcs come **after the MVP** | 2 |
| D7 | A section project may **use a grid of a pinned coordination project**, read-only, so a grid is not created again for each section | 9 |
| D8 | There is a **magnetic mode**: elements placed on a grid may follow it. Details are open | 7.5, O-G5 |
| D9 | The height of a placed element is taken **from a level plus an offset**, as is usual in BIM tools; there is no stored link between a grid and a level | 6, 7.2 |
| D10 | Grids are **visible in 3D to other USD viewers** as 50 mm diameter tubes at `z = 0`, in derived USD geometry with `purpose = guide`; labels remain native UI text | 11 |
| D11 | Limits of 64 grids and 200 lines per family are accepted | 10 |
| D12 | Labels are **not skipped** (no letters left out); default label schemes need no skip set | 3 |
| D13 | Grid **names need not be unique** | 5.2, 10 |
| D14 | Labels are a **list of words separated by spaces**: `А Б В Г Г-1 Г-2 Д`; the character `/` is allowed in a label | 3 |

### MVP defaults selected under the user's delegation (2026-10-26)

| # | Default | Rule |
|---|---------|------|
| M1 | Explicit labels and helpers | Any Unicode letters and digits allowed by section 3 may be entered explicitly, including Cyrillic. The only automatic schemes are `numeric` and Latin `alpha`; there is no Cyrillic auto-labeling scheme in the MVP. |
| M2 | Regular-line drag | A drag rewrites the authoritative family expression. Labels match lines and preserve their AxisIds. A drag that breaks line order or the minimum spacing is refused without changing the grid. |
| M3 | Family mode | `regular` or `drawn` is selected at grid creation and does not change in the MVP. A line cannot be drawn into a regular family; mode conversion is deferred. |
| M4 | Orientation and span | `u` lines are parallel to local Y, `v` lines to local X, and the grid's Z rotation applies to the complete frame. A default span extends 2000 mm beyond the other family in the span direction; section 2 defines the projection rule for drawn lines. |
| M5 | Intersections and placement height | The one-parameter syntax is `GRID:LABEL,LABEL`, with comma as the separator. Placement height is level elevation plus `--offset` (default zero), or explicit `--z`; a grid has no elevation. |
| M6 | Viewer geometry | T01.7 uses 50 mm diameter tubes at `z = 0` in derived USD geometry with `purpose = guide`. Labels stay in BeamCraft's native UI; USD text labels are not authored. |
| M7 | Later features | Radial grids remain post-MVP with curved lines (T01.9). Magnetic placement remains T01.8 and waits for its existing ownership/scope dependencies (O-G5). |

## 2. What a grid is

A **grid** is a named, plan-only construction object: two families of **GridLines** (`u` and `v`) in
the horizontal plane, each line standing for a vertical plane at every height. It has no thickness,
mass, material or engineering geometry and no effect on any quantity. Units are millimetres and
radians, right-handed, Z up (F04).

The model follows the public IFC4 schema (`IfcGrid` with its two axis lists and `IfcGridAxis` with
a tag and a curve), so that a later IFC export is a direct mapping. The optional third list of that
schema (`WAxes`) is not used: a plan grid has no vertical axes. U and V are two ordered families;
they need not be perpendicular, and a drawn line may take any direction.

| Part | Rule |
|------|------|
| **Frame** | An origin `(x, y)` and a rotation about +Z. Local coordinates `(u, v)` are measured in that frame; the grid lies at `z = 0` and has no elevation |
| **GridLine** | An AxisId, a **label** (the name the user sees, section 3) and a geometry. In the MVP the geometry is a straight segment between two points in grid-local coordinates. Its `shape` token is `line`; any other shape is `UNSUPPORTED_GEOMETRY` (arcs, polylines, splines: after the MVP, D2) |
| **Family `u`, `v`** | An ordered list of at most 200 GridLines. A family is **regular** or **drawn** |
| **Regular family** | Its lines are parallel and are generated from one expression (section 4). Lines of a family `u` run parallel to local Y at `u = position`; lines of a family `v` run parallel to local X at `v = position`. The family also has a **span**: the interval along its lines over which they are drawn |
| **Drawn family** | Its lines are drawn one by one, each a segment by two points, in any position and at any angle (section 2.1) |
| **Intersection** | A point where a line of `u` crosses a line of `v` over their stored segments. Lines of one family are not intersected with each other. Two lines cross in at most one point; parallel, collinear or non-crossing lines have none. Intersections are computed, never stored |

A grid needs at least one line in total. A family may be empty. A grid may mix a regular `u` family
with a drawn `v` family. A rectangular grid is therefore two regular families; a free layout is two
drawn ones.

**Span of a regular family.** Stored explicitly as two numbers, the start and end along the line
direction, in local millimetres. At creation, its default is the minimum and maximum coordinates
of the other family in that span direction, extended by 2000 mm on each side. For `u`, use the
other family's local-Y coordinates: regular `v` positions, or the local-Y coordinates of all
endpoints when `v` is drawn. For `v`, use local-X coordinates: regular `u` positions, or the
local-X coordinates of all endpoints when `u` is drawn. If the other family is empty, use `-2000`
to `+2000`. Changing the other family afterwards does not change the span; an edit sets it
explicitly. A regular line is therefore `(position, span.start)` to `(position, span.end)` for `u`,
and `(span.start, position)` to `(span.end, position)` for `v`. Individual lengths per line are
drawn lines (below) or after the MVP (D6).

### 2.1 Drawing lines

Drawing is a UI feature on engine commands (T01.2, T01.6): the user picks a family, clicks two
points with the usual snaps (including those of other grids), and gives or accepts a label. The UI
turns the clicks into the numbers of `DrawGridLine`; the engine never sees a mouse event. A drawn
line can be reshaped (new end points), relabelled and removed. Drawing into a regular family is
refused (`UNSUPPORTED_PROPERTY`); family mode is selected when the grid is created and cannot be
converted in the MVP. The engine never changes a family mode implicitly.

The following are not supported in this version (`UNSUPPORTED_GEOMETRY` unless noted): arcs,
circles, rings, splines and any curved line, radial grids (they need rings, so they come with
curves, T01.9), grids inclined from the vertical or rotated about any axis but Z, a grid with its
own elevation, and a third family.

## 3. Labels

Every GridLine has one **label**, the name the user sees and types.

| Rule | Value |
|------|-------|
| `GR-LBL-001` Characters | 1 to 16 Unicode characters and at most 64 UTF-8 bytes: Unicode letters and digits, `-`, `_`, `.`, `+` and **`/`** (so `1/2` and `А/1` are labels). Letters of any script are allowed (`А Б В Г Д`). No spaces and none of `: , #`, which the input syntax uses as separators |
| `GR-LBL-002` Uniqueness | Unique **in the grid**, across both families, so one label names one line. Compared exactly (code point by code point, no case folding, no normalisation). A clash is `INVALID_ARGUMENT`, reason `GRID_LABEL_DUPLICATE`. A different grid may reuse a label |
| `GR-LBL-003` Stored | Labels are stored on the lines. Moving a line, or adding or removing another, never renumbers or relabels. A gap in the sequence is allowed (`Г`, `Г-1`, `Г-2`, `Д`) |
| `GR-LBL-004` Not an identity | A label can change; the identity is the AxisId (section 5.3). Nothing in the model depends on the text of a label |

**How labels are given.** The user types a **list of words separated by spaces** (a comma is also
accepted), one word per line, in line order: `1 2 3 4`, `A B C`, `А Б В Г Г-1 Г-2 Д`, `1/2 2/3`. The
number of words must equal the number of lines (`INVALID_ARGUMENT`, `GRID_LABEL_COUNT`). Two
helpers write the list for the user: `numeric` (`start` default 1, `step` default 1) and `alpha`
(the Latin letters A to Z, then AA, AB, ...; `start` default `A`), each with `order` `ascending`
(the first label goes to the lowest position, default) or `descending` (the first goes to the
highest). Nothing is skipped (D12). Helpers expand to the explicit list before the command is built,
so a command always carries the list. No Cyrillic helper is provided in the MVP; users supply Cyrillic labels in the explicit list.
Defaults when the user says nothing: `u` numeric, `v` alpha.

**Labels per view.** Which labels a view shows, at which end of a line and how large, and whether a
grid is shown at all, is **view state** (W01, V12 and D04), not model data. Until the views layer
exists the UI keeps it in the session. Hiding a grid or a label never changes the model, a revision,
a signature or an export (`GR-USE-005`).

## 4. Coordinate expressions and editing

A **regular family** is created from one **expression**, the text the user types:

* words separated by spaces or commas;
* the **first number is the position of the first line**, in millimetres (any value, at most
  1,000,000 in absolute value);
* every following word is the **distance to the next line**, a number greater than zero, or
  `N*D`, meaning `N` equal distances `D` (`N` is a whole number from 1 to 200).

| Expression | Lines (positions) |
|------------|-------------------|
| `0 5*6000` | 6 lines: `0, 6000, 12000, 18000, 24000, 30000` (5 bays of 6 m) |
| `0 3*6000 7500` | 5 lines: `0, 6000, 12000, 18000, 25500` |
| `1000 2*4500 6000` | 4 lines: `1000, 5500, 10000, 16000` |
| `0` | one line at 0 |

Positions follow by summation: `p[0]` is the first number, then each distance is added. The
expression is stored (normalised: single spaces, repetitions kept as written) together with the
generated lines, and shown back to the user as typed. **The expression is the authoritative form of
a regular family** (D5): the stored lines must equal what the expression and the span generate, and
a stored family where they differ is refused by the structural validator (`T01-V01`). Editing the
step is editing the text: `0 5*6000` to `0 5*6500` moves every line of the family.

**Editing a regular family** is a command that gives the new expression and the **full label list**
for the new line count (section 3). Lines are matched by **label**: a label that remains keeps its
AxisId and takes its new position; a new label creates a line with a new AxisId; a label that
disappears removes its line. Moving one line alone or inserting a bay is writing the equivalent
expression; the UI does this when a line is dragged. For a one-line drag, the UI rewrites the
expression distances needed to place that line while retaining the family order. The engine matches
by label, so the dragged line and every retained line keep its AxisId. If the resulting positions
would reorder lines or violate the 1 mm spacing floor, the whole command is refused. Removing a
line that a stored reference names is `DEPENDENCY_CONFLICT`.

A drawn family has no expression.

## 5. Storage and identity

### 5.1 One element, lines as records

A grid is **one element** of kind `grid`. Its lines are **records inside it**, not elements (D4).
The reasons, against one element per line:

* Label uniqueness, the expression and the limits are properties of the whole set. One element
  keeps them inside one footprint and one validator rule.
* A family of up to 200 lines stays one prim, not 200 elements with their own identity,
  lifecycle, relationship and copy rules.
* Deleting, copying and permission checks act on one object.

The price is that a line cannot be the target of a USD relationship. A reference to a line is the
pair `(GridId, AxisId)`: a relationship to the grid plus the AxisId (section 5.4).

### 5.2 Where it is stored

| Item | Rule |
|------|------|
| Path | `/World/Grids/GRD_<uuid>`, the category folder `Grids` (USD `kind = group`) beside `/World/Levels`; prefix `GRD`; the full 32 hex digits of the UUIDv7 (F04 `A-3`, [architecture](../architecture.md#prim-hierarchy-and-naming-rule)). A grid is shared by every discipline and sits outside `/Steel` |
| Layer | Today the authored model, as levels are. When U19 lands, grid **and** level elements move to the `Grids` layer (`<code>[_<zone>]_Grids.usda`) by task T01.5; paths and identities do not change, and no migration of earlier stores is promised while the project is pre-alpha |
| Kind | `bim:elementKind = grid`. USD `kind = component`. No `Geometry` child in the authored layer, no material, no mass (viewer geometry is derived data, section 12) |
| Placement | `xformOp:translate` = `(x, y, 0)` and `xformOp:orient` = a rotation about +Z only. `z != 0`, or an orientation with an X or Y component, is `UNSUPPORTED_GEOMETRY`, reason `GRID_PLACEMENT_UNSUPPORTED` |
| Common | `bim:id`, `bim:name` (F04 label rule, 1 to 256 bytes; **not required to be unique**, D13), `bim:discipline` (the authoring module token; immutable), `bim:schemaVersion`, `bim:createdBy` |

Attributes, for each family `<f>` = `u` or `v` (explicit, readable without a schema):

| Attribute | Meaning |
|-----------|---------|
| `bim:grid:<f>:mode` | token `regular` or `drawn` |
| `bim:grid:<f>:expression` | string, `regular` only: the coordinate expression of section 4 |
| `bim:grid:<f>:spanMm` | double array of 2, `regular` only: the span |
| `bim:grid:<f>:ids` | string array: the AxisIds, in order |
| `bim:grid:<f>:labels` | string array: the labels, same order |
| `bim:grid:<f>:shapes` | token array: `line` (the only value in the MVP), same order |
| `bim:grid:<f>:pointsMm` | double array, four numbers per line `(x1, y1, x2, y2)` in grid-local coordinates, same order |

The proposed F10 registry extension adds `grid` beside `level` with owner `any` and family
`project structure`. Under F10, `bim:discipline` records the authoring module and is not write
authority; shared `Grids` scope under F10 0.2 `DV-SCOPE-004` authorizes grid writes. Grid-specific
attributes use the `bim:grid:` namespace.

The arrays of a family have lengths that agree (`pointsMm` has four times as many entries as
`ids`). A stored grid that breaks this, a duplicate AxisId or label, a segment shorter than 1 mm, a
regular family that does not match its expression, or any array above the limits, is refused by the
structural validator: `INVALID_ARGUMENT`, rule `T01-V01`, and `LIMIT_EXCEEDED` for sizes, checked
before allocation. The reader never indexes with a stored value.

### 5.3 Identity

* **GridId** is a UUIDv7 and never changes: not on rename, move, rotation, relabelling or edit of any
  line.
* **AxisId** is a UUIDv7 unique **in the project**, never reused (also not after the line is
  removed), so `(GridId, AxisId)` and the AxisId alone each name one line for ever. Moving a line,
  changing its label or its position keeps the AxisId. Removing a line ends it; a new line with the
  same label gets a new AxisId.
* A command carries the ids of everything it creates (`new_id`, `axis_ids`), so replay under the same
  `op_id` is exact (F04 section 6). The CLI generates them when they are omitted. A duplicate GridId
  or AxisId, inside the command or against the project, is `DUPLICATE_ID`.

### 5.4 Edits, forks, copies and references

| Event | Behaviour |
|-------|-----------|
| Edit | Any grid command keeps the GridId and every unaffected AxisId |
| Workspace fork, publication, rebase | A fork is the same project history, so all ids are the same. A later merge matches grids and lines by id, never by label or position |
| `CopyElement` of a grid | A new independent grid: a fresh GridId and **fresh AxisIds**, all supplied by the command. Nothing is shared |
| Reference to a line | `(GridId, AxisId)`. Saved views (W01) and, later, drawing dimensions (D03) store the pair plus a relationship to the grid. A line that disappears leaves the reference **unresolved** and visible, never silently dropped |
| Delete or remove | Refused with `DEPENDENCY_CONFLICT` (reason `GRID_IN_USE`) while a stored reference names the grid or the line; the diagnostic names the referrers |

## 6. Grids, levels and zones

* **Levels stay as D2 made them:** elements of kind `level` with `bim:elevationMm`, shared by every
  discipline. A grid has no height and a level has no plan position, and **no link between them is
  stored** (D9). The height of a placed element is taken from a level plus an offset when the
  element is placed (section 7.2).
* **Any number of grids**, at most 64 per project (D11), which may overlap. A "common" grid is one in
  the unzoned `Grids` layer. Once U19 lands, a zone's grids and levels live in that zone's `Grids`
  layer with the zone's own frame; the zone of a grid is the layer it is stored in.
* **Sections use a shared grid, they do not copy it** (D7): a section project reads a grid of the
  pinned coordination project (F12), so one grid serves KM, KMD, AR and the others. Using it for
  snapping and placement is read-only (section 9).

## 7. How grids are used

### 7.1 Snapping

`snap` (V04) gains three kinds beside `vertex`, `axis_end` and `axis`:

| Kind | Candidate |
|------|-----------|
| `grid_intersection` | An intersection of section 2 |
| `grid_line` | The nearest point of a GridLine over its segment (perpendicular foot clamped to the ends) |
| `grid_end` | An end point of a GridLine |

* **Plan distance.** The distance is measured in the XY plane. A hit keeps the query point's `z`
  (a grid has no height; the caller's work plane or level supplies it).
* **Ordering and bands** are the V04 rules: quantized distance, kind, then ids. The new kinds rank
  after the existing ones, `grid_intersection`, `grid_end`, `grid_line`; then GridId, AxisId (the
  pair of AxisIds `u` then `v` for an intersection). A hit carries `(GridId, AxisId[, AxisId])` and
  the labels, never a FeatureId. A UI may prefer intersections by its own rule.
* **Work bounds.** At most 64 grids and 400 lines per grid are read. Only lines whose distance to
  the query point is within the radius are candidates, and intersections are computed among those
  candidates only, so a `200 x 200` grid costs the work of the nearby lines, not 40,000 pairs. The
  band test is that of V04 (`distance <= radius + 0.001`).
* **Visibility is the caller's.** `snap` takes an optional grid filter (`--grids ID,ID`, default
  all); a grid a view hides is not passed.
* The query is read-only and never changes the head.

### 7.2 Placing an element on an intersection

The engine API `grid_point(grid, label, label)` returns the world point `(x, y)` where the two
named lines cross: one label from each family, in either order. Two labels of one family, an
unknown label, a non-crossing pair or a grid that does not exist are refused (section 10).

A command that takes points accepts **one parameter** for it (D3): `GRID:LABEL,LABEL`, where `GRID`
is a GridId or a grid name carried by exactly one live grid (`GRID_NAME_AMBIGUOUS` otherwise), for
example `--start-at-grid 2401:Б,3`. A name containing `:` must be addressed by GridId. Comma is
the fixed label separator. Height is given as
`--z MM`, or as **`--level LEVEL [--offset MM]`**, which takes `z = elevation of the level + offset`
(offset default 0; D9). Without either the command is refused (`INVALID_ARGUMENT`). T01.4 adds the
syntax to `create-member` (`--start-at-grid`, `--end-at-grid`) and to the placement origin of
`create-plate`.

In the normal (non-magnetic) mode the CLI resolves the parameter **before** it builds the command,
so the stored command and the stored element hold plain coordinates: moving a grid or a line
afterwards moves no element, changes no signature, and replaying an old `op_id` gives the same
result; the part has no relationship to the grid.

### 7.3 View by grid line

`grid_line_plane(grid, line)` returns the vertical plane of a GridLine as a world point and a unit
normal (horizontal, perpendicular to the line direction). A saved view (V12) stores `(GridId,
AxisId)`, the side and the depth and **re-resolves** the plane each time it opens, so moving the
line moves the view; a missing line leaves the view unresolved (`GR-USE-004`). Plan views by level are
unchanged.

### 7.4 Visibility and selection

Showing or hiding grids, labels and ends is view state (section 3). Selecting a grid is the V11
switch `grids`, which changes picking only. Hiding a grid never changes the model, the revision, the
snapshot token or any export (`GR-USE-005`).

### 7.5 Magnetic mode (open)

The owner wants a **magnetic** mode in which elements placed on a grid follow it (D8). Where the
switch lives, what follows, what happens when a line is deleted, and how a section element is kept
authoritative when a shared grid changes are not decided; they are the questions of O-G5. This
document specifies the non-magnetic placement of section 7.2 only; the magnetic mode is slice T01.8
and is not scheduled until O-G5 is answered. The non-magnetic mode never becomes magnetic by itself.

## 8. Commands (engine API first, then CLI)

Each command is one atomic edit and one undo unit, takes `op_id`, validates before it writes and, on
any refusal, changes nothing. The CLI command and option names in this contract are the planned public interface.

### 8.1 Edits

| Command (CLI) | Arguments | Behaviour |
|---------------|-----------|-----------|
| `CreateGrid` (`create-grid`) | `new_id`, `name`, `origin`, `rotation` (degrees at the CLI); per family `u`, `v`: either an `expression`, its `labels` list or helper, a `span` and `axis_ids`, or `drawn` with an optional explicit `endpoints` list (empty by default) | Validates, expands the expression and the labels, authors the grid; a drawn family given at creation feeds its endpoint projections into the other family's default span (section 2) |
| `UpdateGrid` (`update-grid`) | `target_id`; any of `name`, `origin`, `rotation`; per regular family a new `expression` with the full `labels` list and `axis_ids` for new labels, and `span` | Rewrites a regular family by section 4; moves or renames the grid; ids of kept lines unchanged |
| `GridLineDraw` (`draw-grid-line`) | `target_id`, `family`, `new_axis_id`, `label`, `from`, `to` (world points) | Adds one straight line to a **drawn** family |
| `GridLineReshape` (`reshape-grid-line`) | `target_id`, `axis_id`, `from`, `to` | New end points of a line of a drawn family; id and label kept |
| `GridLineRemove` (`remove-grid-line`) | `target_id`, `axis_id` | Removes a line of a drawn family. The only line of a grid is refused (delete the grid) |
| `GridRelabel` (`relabel-grid-line`) | `target_id`, `axis_id` + `label`, or `family` + the full `labels` list | Changes labels only; ids and geometry untouched |
| `DeleteElement` | `target_id` | Deletes the grid with its lines; refused while a stored reference exists |
| `CopyElement` | `source_id`, `new_id`, `name`, `new_axis_ids` | An independent copy (section 5.4) |

**Queries** (read-only, no revision): `list-grids` (every grid with its frame and each line: id, label,
and world end points, and each family's expression), `resolve-grid-point` (section 7.2),
`resolve-grid-plane` (section 7.3) and `snap` with the new kinds. `inspect-elements` lists grids beside
levels.

**Footprints.** Create: read the absence of the GridId and of every AxisId, write the new lifecycle
and the complete authored fields. Any other grid command: read the lifecycle and the complete
`shape_inputs` group of the grid, write that group (and `label` for a rename, `placement` for a
move). A grid command reads and writes no other element. AxisIds are checked against an index of all
AxisIds of the project, which a stored-graph validator also rebuilds.

## 9. Rights

Grids and levels are **shared data** with their own shared scope (F12 `PC-OWN-002`, F10 0.2
`DV-SCOPE-004`).

| Operation | Section user (ordinary edit) | Shared `Grids` scope |
|-----------|------------------------------|----------------------|
| Read grids and levels, `snap`, `resolve-grid-point`, `resolve-grid-plane`, `--start-at-grid` placement | allowed | allowed |
| Create, update, copy or delete a grid; draw, reshape, remove or relabel a line | `PERMISSION_DENIED` | allowed |
| Create or change a level (D2) | `PERMISSION_DENIED` | allowed |
| Ordinary `UpdateElement`, create or copy of a part | cannot touch a grid or a level: a part holds coordinates and a `bim:level` link | n/a |

* An ordinary section edit therefore **never changes a grid or a level**; a KMD edit preserves the
  grids exactly (`PC-OWN-001`).
* A grid of a pinned coordination project or a read-only reference is usable for reading and for
  placement by the section (D7) and `FOREIGN_READ_ONLY` for every write. Its identity there is
  `(project, GridId)`; the exact addressing is part of F12 coordination (T01.5).
* **Transition.** Before U19 a workspace has one scope and grid commands run without an
  authorization check, as `create-level` does today. T01.5 puts grids and levels under the shared
  scope together with U19; the commands do not change, only the refusal.
* Authority comes from the trusted execution context, never from a file name, a layer name or a
  client-provided scope (`DV-SCOPE-001`, `DV-SCOPE-002`).

## 10. Limits, codes and what is unsupported

Limits are original operating limits, checked before allocation and never widened.

| Item | Limit | Above or outside |
|------|-------|------------------|
| Grids in a project | 64 | `LIMIT_EXCEEDED`, `GRID_COUNT_LIMIT` |
| Lines in one family | 200 | `LIMIT_EXCEEDED`, `GRID_COUNT_LIMIT` |
| Expression | at most 4096 UTF-8 bytes; `N` of `N*D` from 1 to 200; at most 200 lines after expansion | `INVALID_ARGUMENT` (`GRID_EXPRESSION_INVALID`) or `LIMIT_EXCEEDED` |
| One distance, `abs(position)`, span ends | at most 1,000,000 mm | `LIMIT_EXCEEDED` |
| Origin and world end points | the coordinate cap of F04, 1,000,000,000 mm | `LIMIT_EXCEEDED` |
| Distance between neighbouring lines of a regular family | at least 1 mm; every distance greater than zero | `INVALID_ARGUMENT`, `GRID_SPACING_INVALID` |
| Length of a line (drawn, or the span of a regular family) | at least 1 mm | `INVALID_ARGUMENT`, `GRID_LINE_DEGENERATE` |
| `abs(rotation)` | at most `2π` | `INVALID_ARGUMENT` |
| Label | section 3 | `INVALID_ARGUMENT`, `GRID_LABEL_INVALID` or `GRID_LABEL_COUNT` |
| Grid name | F04 label rule (1 to 256 bytes) | `INVALID_ARGUMENT` |
| Non-finite number | any numeric input | `NON_FINITE_NUMBER` |

**Codes.** No new top-level code: F04's codes with a diagnostic `reason` token.

| Case | Code | Reason |
|------|------|--------|
| Two lines with one label in a grid | `INVALID_ARGUMENT` | `GRID_LABEL_DUPLICATE` |
| Unknown grid, line or label | `NOT_FOUND` | `GRID_AXIS_UNKNOWN` |
| Two labels of one family, or two lines that do not cross, for an intersection | `INVALID_ARGUMENT` | `GRID_INTERSECTION_UNDEFINED` |
| A grid name that more than one live grid carries | `INVALID_ARGUMENT` | `GRID_NAME_AMBIGUOUS` |
| Drawing into a regular family; an expression for a drawn family | `UNSUPPORTED_PROPERTY` | `GRID_FAMILY_MODE` |
| A shape other than `line`; a placement outside section 5.2 | `UNSUPPORTED_GEOMETRY` | `GRID_SHAPE_UNSUPPORTED`, `GRID_PLACEMENT_UNSUPPORTED` |
| Delete or remove while a reference exists | `DEPENDENCY_CONFLICT` | `GRID_IN_USE` |
| Used GridId or AxisId | `DUPLICATE_ID` | `GRID_ID_USED` |
| No authority; foreign grid write | `PERMISSION_DENIED`; `FOREIGN_READ_ONLY` | |
| Same `op_id`, other content | `OP_ID_REUSE` | |

## 11. Interchange

* **IFC** (E01, deferred): not in the MVP. A grid element is not exported and is not counted as an
  element, as a `level` is a storey and not an exported element. The model follows `IfcGrid`, so
  the later export is a direct mapping of the families, tags and straight curves. E01 limitation
  L-12 reads "the export records no grids".
* **BOM, numbering, schedules, quantities:** a grid has no part number, mass or volume and is
  skipped by every one; adding a grid never makes numbering stale or changes a signature.
* **DXF, DSTV, STEP, FBX, DWG:** not part of the MVP. A drawing view (D04) may draw the grid lines
  its view shows as view content; that is drawing data, not an exchange of grids.
* **Other USD viewers:** the grid is an ordinary `Xform` with attributes, readable by any USD tool,
  and carries one 50 mm diameter tube per line at `z = 0` in a derived layer with `purpose = guide`.
  USD text labels are omitted; native BeamCraft UI renders the labels. Viewer geometry never enters
  the authored model or engineering queries (T01.7).

## 12. Deferred owner decision

The MVP rules and acceptance cases are settled in this contract. The only remaining question is
magnetic placement, which is explicitly outside T01.1 to T01.7 and waits for T01.8.

| # | Question | Blocks |
|---|----------|--------|
| O-G5 | **Magnetic mode.** Decide where the switch lives, which placed parts follow a line, how shared-grid changes affect followers owned by a section, whether a line with followers may be deleted, whether hand edits detach a follower, and whether a pinned foreign grid may be followed. A shared-grid edit must never rewrite section elements outside their scope. | T01.8 |

`T01.7` has exact tube geometry and purpose rules in section 11 and acceptance case GR-P16.
Questions O-G7 and O-G11 to O-G16 from the original #192 draft are resolved by the defaults in
section 1 and the rules below; they do not block the MVP.

## 13. Acceptance cases (to be bound to fixtures)

Tolerance for coordinates is 1e-6 mm unless stated. Each negative case changes nothing.

Positive:

| # | Case | Expected |
|---|------|----------|
| GR-P01 | Grid with origin `(0, 0)`, rotation 0; `u`: expression `0 3*6000`, labels `1 2 3 4`; `v`: expression `0 7500 4500`, labels `A B C`; default span | `u` positions `[0, 6000, 12000, 18000]`; `v` positions `[0, 7500, 12000]`; line `3` runs `(12000, -2000)` to `(12000, 14000)`; line `B` runs `(-2000, 7500)` to `(20000, 7500)`; 12 intersections; `B,3` and `3,B` are `(12000, 7500)` |
| GR-P02 | Expressions `0 5*6000`; `0,3*6000,7500`; `1000 2*4500 6000`; `0` | `[0, 6000, ..., 30000]` (6 lines); `[0, 6000, 12000, 18000, 25500]`; `[1000, 5500, 10000, 16000]`; one line at 0. The family `0 3*6000 7500` is shown back as typed |
| GR-P03 | The grid of GR-P01 with origin `(1000, 2000)` and rotation 90° | `B,3` is `(-6500, 14000)` (`(-7500 + 1000, 12000 + 2000)`) |
| GR-P04 | Labels: the list `А Б В Г Г-1 Г-2 Д` on a family `0 6*6000`; the list `1/2 2/3`; `alpha` for 28 lines; `numeric` descending on `0 4*1000` | the seven labels as given, in order; labels `1/2`, `2/3`; `A` to `Z`, `AA`, `AB`; labels `5, 4, 3, 2, 1` at positions `0, 1000, 2000, 3000, 4000` |
| GR-P05 | Drawn families: draw `D1` from `(0, 0)` to `(3000, 4000)` into `u` and `D2` from `(3000, 0)` to `(0, 4000)` into `v` | One intersection `D1,D2` at `(1500, 2000)`. Reshape `D2` to `(3000, 0)`-`(3000, 4000)`: `D1,D2` at `(3000, 4000)` (end point included) |
| GR-P06 | Edit GR-P01 `u` to `0 3*6500` with labels `1 2 3 4`; to `0 4*6000` with `1 2 3 4 5`; to `0 2*6000` with `1 2 3` | positions `[0, 6500, 13000, 19500]` with every AxisId kept; lines `1`-`4` keep their AxisId and `5` is new; line `4` removed, the rest kept. GridId unchanged; each command one undo unit |
| GR-P07 | `snap` at `(12003, 7502, 3000)`, radius 10, kinds `grid_intersection,grid_end,grid_line` on GR-P01 | Hits in order: line `B` at `(12003, 7500, 3000)` (2), line `3` at `(12000, 7502, 3000)` (3), intersection `B,3` at `(12000, 7500, 3000)` (3.6056); no `grid_end`. With `--grids` omitting the grid: no hit; the head is unchanged |
| GR-P08 | A level at `3000`; `create-member` from `--start-at-grid G:B,3` to `--end-at-grid G:B,4`, `--level L --offset 500`, on GR-P01; then edit `u` of the grid | Axis `(12000, 7500, 3500)` to `(18000, 7500, 3500)`; after the edit the member, its shape key and its mass signature are unchanged; its stored command holds coordinates |
| GR-P09 | `CopyElement` of a grid | A new GridId and new AxisIds, same labels and geometry; both snap independently; deleting one leaves the other |
| GR-P10 | Create a grid in a project with a plate, a member and a level; export the BOM and run numbering | Element list, quantities and part numbers equal those without the grid; numbering is not stale; the grid is not in the IFC element list |
| GR-P11 | A KMD edit (rename a part, move a plate) under the rules of T01.5 | Every grid and level is byte-equal before and after; with no `Grids` authority a grid command is `PERMISSION_DENIED` |
| GR-P12 | A section project places a member at an intersection of a grid of the pinned coordination project | Coordinates as GR-P08; the grid is unchanged; a write to that grid is `FOREIGN_READ_ONLY` |
| GR-P13 | `resolve-grid-plane` of line `3` of GR-P01 | A vertical plane through `(12000, y)` with normal `(±1, 0, 0)`; the same after reopening |
| GR-P14 | Grid A has regular `u` at 0 and drawn `v` endpoints `(1000, 8000)` and `(5000, 12000)`; Grid B has regular `v` at 0 and drawn `u` endpoints `(1000, 3000)` and `(5000, 4000)` | Grid A's `u` segment runs `(0, 6000)` to `(0, 14000)`; Grid B's `v` segment runs `(-1000, 0)` to `(7000, 0)` |
| GR-P15 | Drag regular line `2` of positions `[0, 6000, 12000]` to `6500` | Expression becomes `0 6500 5500`; all three labels and AxisIds remain; one undo restores the previous expression and positions |
| GR-P16 | Open a derived USD grid in another viewer | One 50 mm diameter tube per line at `z = 0`, `purpose = guide`; no USD text labels and no grid geometry in the authored layer |

Negative:

| # | Case | Expected |
|---|------|----------|
| GR-N01 | Label `1` in `u` and again in `v`; relabel to an existing label | `INVALID_ARGUMENT`, `GRID_LABEL_DUPLICATE` |
| GR-N02 | A label with a space, `:`, `,` or `#`; an empty label; 17 characters | `INVALID_ARGUMENT`, `GRID_LABEL_INVALID` |
| GR-N03 | Three lines and a label list of two words | `INVALID_ARGUMENT`, `GRID_LABEL_COUNT` |
| GR-N04 | Expressions `0 0*6000`, `0 -6000`, `0 6000 x`, empty text | `INVALID_ARGUMENT`, `GRID_EXPRESSION_INVALID` |
| GR-N05 | Two neighbouring lines 0.5 mm apart; a distance of 0 | `INVALID_ARGUMENT`, `GRID_SPACING_INVALID` |
| GR-N06 | 201 lines in a family; the 65th grid; a distance of 1,000,001 mm; origin 1,000,000,001 mm | `LIMIT_EXCEEDED` |
| GR-N07 | NaN or infinite number | `NON_FINITE_NUMBER` |
| GR-N08 | `resolve-grid-point` with two labels of one family; an unknown label; two lines that do not cross (parallel lines, or crossing outside both segments) | `INVALID_ARGUMENT` (`GRID_INTERSECTION_UNDEFINED`) or `NOT_FOUND` (`GRID_AXIS_UNKNOWN`) |
| GR-N09 | Draw a line into a regular family; give an expression to a drawn family | `UNSUPPORTED_PROPERTY`, `GRID_FAMILY_MODE` |
| GR-N10 | A drawn line of length 0.5 mm or with equal end points | `INVALID_ARGUMENT`, `GRID_LINE_DEGENERATE` |
| GR-N11 | A shape `arc`; a placement rotated about X, or `z != 0` | `UNSUPPORTED_GEOMETRY` |
| GR-N12 | Duplicate GridId; an AxisId repeated in the command or used by another grid | `DUPLICATE_ID` |
| GR-N13 | The same `op_id` with other content | `OP_ID_REUSE` |
| GR-N14 | Remove a line, or delete a grid, named by a saved view (after W01) | `DEPENDENCY_CONFLICT`, `GRID_IN_USE`; the view stays |
| GR-N15 | A stored grid whose arrays differ in length, whose AxisId repeats, whose regular lines differ from their expression, or whose array exceeds the limit | `INVALID_ARGUMENT` or `LIMIT_EXCEEDED`; no panic, no allocation sized by the file |
| GR-N16 | A write to a grid of a foreign reference or a pinned project | `FOREIGN_READ_ONLY` |
| GR-N17 | `--start-at-grid` with neither `--z` nor `--level` | `INVALID_ARGUMENT` |
| GR-N18 | A grid name carried by two grids used as a grid reference | `INVALID_ARGUMENT`, `GRID_NAME_AMBIGUOUS` |
| GR-N19 | Drag a regular line past its neighbour or to less than 1 mm spacing | `INVALID_ARGUMENT`, `GRID_SPACING_INVALID`; the head, expression and AxisIds are unchanged |

Viewer geometry is covered by GR-P16. Magnetic-mode cases are added only after O-G5 is resolved for T01.8.

## 14. Slices and dependencies

Recorded in [`tasks.md`](../plan/tasks.md): T01.1 grid model with regular families, labels and
expressions (model, layer engine, engine, CLI); T01.2 drawn lines; T01.3 editing a regular family;
T01.4 queries, snaps and placement; T01.5 shared-scope rights, pinned grids and the move to the
`Grids` layer with U19; T01.6 the UI; T01.7 viewer geometry; T01.8 magnetic mode (after O-G5);
T01.9 curved lines and radial grids (after the MVP). Every slice has its own end-to-end test
(`AGENTS.md` section 3).

## 15. Provenance

The request, the structure (two families of named lines, as in the IFC schema), the expression form,
the label list, the magnetic mode, the level-and-offset height and the viewer geometry are the
owner's decisions of 2026-10-23. The model, the rules, the limits and the acceptance numbers are
our own and use only what this repository defines (the element and prim rules of F04, the level of
D2, the snap of V04, the F10 0.2 and F12 scope rules) and the public buildingSMART IFC4
specification for `IfcGrid` and `IfcGridAxis` (attribute names and meaning only; no text is
copied). The behaviour is described in our own words; nothing was taken from Tekla Structures or
another commercial product, and no screenshot or asset is part of this document (`AGENTS.md`
sections 1 and 2).
