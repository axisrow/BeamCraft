# F04 amendment draft: plate position, corner treatment and contour editing

Status: **accepted amendment (2026-10-27), current MVP rules for M5 CLI delivery.**
The draft date below records its origin; accepted amendments are versioned alongside the frozen contracts, not a rewrite of them.

Historical proposal status (superseded): **draft for owner review (2026-10-23).** F04 0.1.0 stays frozen and unchanged until the
owner accepts this text; on acceptance it is recorded as an F04 amendment (name to be confirmed)
and the sections it replaces in [F04](f04-model-and-commands-v0.1.md) and
[F06](f06-geometry-contract-v0.1.md) get a dated pointer. It extends the plate of A02 and is the
contract for the slices A12.1 to A12.3 and V18 in the [GitHub task catalogue](https://github.com/Apibim-com/BeamCraft/issues/474). The behaviours
come from the owner's description of 2026-10-23 ([decisions](../plan/decisions.md)); the wording
and the model are our own.

Nothing here changes an existing store: a plate without the new fields reads and evaluates exactly
as before.

## 1. Thickness side

A plate has the new input `thickness_side`: `top`, `middle` or `bottom`. The contour lies in the
element-local plane `z = 0`; the side says where the thickness `t` is laid off from that plane.

| Value | Material between | Note |
|-------|------------------|------|
| `top` | `z = 0` and `z = +t` | Default. Identical to every plate of A02 |
| `middle` | `z = -t/2` and `z = +t/2` | Half on each side of the contour plane |
| `bottom` | `z = -t` and `z = 0` | Thickness laid off in `-Z` |

Rules:

* `CreatePlate` takes an optional `thickness_side`; absent means `top`. `UpdateElement` may patch it.
* The contour, hole polygons and plane-cut coordinates stay in the same element-local frame
  whatever the side. A hole is a through hole, so it is unaffected. A plane cut is replayed on the
  new body, so changing the side changes what a cut with a fixed `z` removes.
* Volume, area and mass do not depend on the side. The mesh and the shape key do.
* The side is the axis of the element-local frame, not a world direction: a rotated placement
  (P10) turns it with the plate.
* The numbering key follows the geometry, not the authored side (owner decision 2026-10-23). Two
  plates whose geometry is the same, differing only in `thickness_side`, get the same part number.
  The key is the plate body normalised to the `top` frame: the material occupies `z = 0` to
  `z = +t`, and hole and cut coordinates are shifted by the same offset (`0` for `top`, `t/2` for
  `middle`, `t` for `bottom`). It is compared as A08 compares every fact, within the tolerance
  policy, not by the raw mesh digest, because translating a mesh can change its last bits. The
  placement of the plate in the model never enters the key.

## 2. Corner treatment

Every vertex of the authored contour may carry a corner treatment. The authored contour keeps the
sharp corner point; the treatment is an authored parameter, so a rounded corner stays editable by
its radius and is not a stored polygon.

| `corner` | Parameters | Result |
|----------|-----------|--------|
| `sharp` | none | Default. The vertex as authored |
| `chamfer` | `d1`, optional `d2` (default `d1`) | The corner is replaced by a straight cut. `d1` is measured from the vertex along the edge towards the **previous** vertex, `d2` along the edge towards the **next** vertex. One value gives a symmetric cut |
| `round` | `r` | The corner is replaced by a circular arc of radius `r`, tangent to both edges |

Rules:

* `d1`, `d2` and `r` are finite, greater than zero and within the F04 coordinate cap.
* A treatment applies to a corner that really turns: a vertex whose two edges are collinear or
  fold back onto each other is refused (`INVALID_ARGUMENT`, field `corner`). Convex and concave
  (reflex) corners are both allowed.
* Fit: the length each treatment consumes from an edge is `d1` or `d2` for a chamfer and
  `r / tan(phi / 2)` for a round (`phi` is the angle between the two edges at the vertex). The
  consumed lengths at the two ends of one edge must add up to **less than** the edge length, so no
  edge collapses to a point. Otherwise `INVALID_ARGUMENT`, field `corner`.
* The arc is faceted for the mesh with a chord deviation of at most 0.05 mm (the rule of the round
  hole, F06 A-2 D-2), at least two segments, the end points exactly on the tangent points. The
  evaluator derives the polygon in f64 and the kernel then validates the derived polygon with its
  exact predicates, as for any contour.
* Quantities are those of the faceted body. The result reports the largest chord deviation
  (`arc_deviation_mm`), as the A12 acceptance requires; an analytical area is never claimed for a
  rounded plate.
* Operation records (holes, cuts) are replayed on the evaluated outline. A treatment that leaves a
  hole outside the plate follows the existing rule: the plate is refused, or degenerate with a
  diagnostic naming the record when `keep_degenerate` is used.
* Corner fillets of holes are not part of this amendment.

## 3. Contour editing (engine API and CLI)

All edits happen in the plate plane: only the local `x` and `y` of a vertex change, `z` stays `0`,
so the plate is planar by construction. Each command is one atomic command and one undo unit, is
equivalent to an `UpdateElement` replacing the contour, replays every operation record and takes
`on_invalid_shape` like `UpdateElement` (interactive tools pass `keep_degenerate`).

| Command (CLI) | Arguments | Behaviour |
|---------------|-----------|-----------|
| `MovePlateVertex` (`move-plate-vertex`) | target_id, vertex, `to_mm` (x, y) | Moves one vertex. Its treatment moves with it |
| `MovePlateEdge` (`move-plate-edge`) | target_id, edge, `delta_mm` (x, y) | Moves both end vertices of an edge by the same in-plane vector; the two neighbouring edges follow |
| `InsertPlateVertex` (`insert-plate-vertex`) | target_id, edge, `point_mm` (x, y) | Splits an edge with a new `sharp` vertex at the given point (a UI inserts at the middle and then moves it). The neighbouring treatments stay |
| `DeletePlateVertex` (`delete-plate-vertex`) | target_id, vertex | Removes a vertex with its treatment. **A plate with three vertices is never reduced**: `INVALID_ARGUMENT`, field `contour_points_mm`, nothing changes, whatever `on_invalid_shape` says |
| `SetPlateCorner` (`set-plate-corner`) | target_id, vertex, `corner` | Sets or clears the treatment of one vertex |

A vertex and an edge are named by their position in the authored contour at the generation the
caller saw; a stale generation is refused by the existing base-revision rule. An edge `i` joins
vertex `i` and vertex `i + 1` (the last joins back to the first).

* **Decided (O-1, 2026-10-23): no stable vertex ids for now.** F06 forbids index addressing of
  FeatureIds, but contour vertices are addressed by position, as above. Revisit when welds,
  dimensions or marks must attach to a plate edge (A15, D06).
* A result that is not a valid simple counter-clockwise polygon (a self-crossing after an edge was
  dragged, for example) follows the validity rules of F04 section 4: refused by default, kept as a
  degenerate element with `keep_degenerate`.

## 4. What the UI does with these commands (V18, not engine contract)

Selecting a plate highlights its contour with a grip on every vertex and a grip in the middle of
every edge. Dragging a vertex grip calls `MovePlateVertex`; dragging an edge, `MovePlateEdge`;
dragging a middle grip inserts a vertex (`InsertPlateVertex`) and then moves it; `Delete` on a
selected vertex calls `DeletePlateVertex` and does nothing on a triangle; choosing a vertex opens
its corner treatment (`chamfer` with one or two distances, or `round` with a radius). The plate
stays in its plane. A drag is one undo unit and publishes nothing until the workspace is published.

## 5. Acceptance cases (to be bound to fixtures)

Positive:

1. `top`, `middle` and `bottom` give the `z` ranges of section 1; equal volume; an older plate
   reads as `top`.
2. A 1000 x 1000 square with one chamfer `d1 = d2 = 100` has area 995,000 mm2 (the cut triangle
   is 5,000 mm2); with only `d1 = 100` it is the same.
3. A chamfer `d1 = 100`, `d2 = 50` removes 2,500 mm2.
4. A square with one `round` of radius 100: area within the reported deviation of
   `1,000,000 - (1 - pi/4) * 100^2`; `arc_deviation_mm <= 0.05`.
5. A treatment on a concave corner of an L-shaped plate is accepted.
6. Delete on a quadrilateral gives a triangle; insert then move gives a pentagon; moving an edge
   changes both end vertices and nothing else.
7. Corner treatments survive copy, undo and redo exactly.
8. Two plates with the same contour, thickness, holes and cuts, one `top` and one `bottom` (the
   cuts and holes given so that the material is the same after the shift), are one numbering
   group; a plate with a different outline, thickness or hole is not.

Negative (each changes nothing, or is kept degenerate with `keep_degenerate`):

1. `r` or `d` zero, negative, non-finite or over the cap.
2. A treatment whose consumed lengths reach the edge length.
3. A treatment on a collinear vertex.
4. `DeletePlateVertex` on a triangle.
5. An edge dragged so that the contour crosses itself.
6. A vertex move that leaves a hole outside the plate.

## 6. Provenance

The corner geometry is elementary plane geometry (tangent length from the angle between two
edges); the arc faceting reuses the chord-deviation rule of F06 A-2. No algorithm, text or asset of
another product is used; the examples are our own.
