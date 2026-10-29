# A02.1 contour plates

Date: 2026-10-19. Status: **implemented as an experimental slice; no gate acceptance is claimed.**
Branch `feat/a02-contour-plates`, based on `origin/master` `fd6fde9`.

Plan row: A02, "contour plates", MVP scope item 1 of `ROADMAP.md`. Until now the engine evaluated
one plate shape, the origin rectangle of F04-POS-01; every other valid contour was refused as
`UNSUPPORTED_GEOMETRY` (P01, [design note](p01-first-plate.md)), and F06 kept
`GEO_UNSUPPORTED_CONTOUR` "until A02". The kernel could already extrude any simple polygon (G05)
and cut holes into the result (G09); this slice wires that into the engine, the CLI and the
consumers of a plate.

## 1. Scope

A plate is created from a contour, a thickness and an optional list of through holes. A02.1
accepts **any simple counter-clockwise polygon in the plane `z = 0`**, up to the G05 caps (1024
points, the F04 coordinate caps), used exactly as authored.

| Contour | Result |
|---|---|
| The origin rectangle `(0,0,0) (w,0,0) (w,l,0) (0,l,0)` | The G02 box, unchanged: same mesh bytes, digest, face ids and quantity label as in P01. |
| Any other simple counter-clockwise polygon with every `z = 0` | G05 extrusion along `+Z` by the thickness (new). |
| Clockwise, self-crossing, touching itself, doubling back, too few points | `INVALID_ARGUMENT` (the kernel decides with exact predicates, F04 P-1). Nothing is reordered, merged or repaired. |
| Any `z` other than `0`, or a `z` that varies | `UNSUPPORTED_GEOMETRY`, as before. |
| A placement rotation other than the identity | `UNSUPPORTED_GEOMETRY`, as before (the view contract carries translations). |

The authored contour is stored exactly as for the rectangle (`steel:plate:contourPointsMm`), so
**no USD schema change** was needed and every older store reads unchanged. The publish-time
verifier already re-evaluated each plate from its authored contour with `evaluate_plate`; it now
does so for contour plates too, so a hand-edited layer still cannot be published.

## 2. What changed

| Area | Change |
|---|---|
| `beamcraft_engine::command` | `evaluate_plate` sends a contour that is not the origin rectangle to `evaluate_contour`: `extrude(outer, [], thickness)`, then the holes through `Body::from_extrusion` and `through_hole`, as for the box. `PlateBody.result` is now `Option<PlateResult>` (`Some` only for the box), `gross_volume_mm3` is new, and `is_origin_rectangle()` tells the two apart. `width_mm` and `length_mm` are the extents of the outline (for a contour, its bounding box). New volume label `CONTOUR_VOLUME_ALGORITHM` (`kernel-g05-contour-plate-volume/1;tolerance-policy-3`) for a contour with no hole; a plate with holes keeps the hole label. |
| `beamcraft_engine::solid` | `SolidMesh::from_extrusion`: the sealed G05 mesh, its SHA-256 and the analytic `net_area * thickness`. |
| `beamcraft_engine::numbering` | `PartFact.outline`: the outline of a contour plate relative to its bounding-box minimum, compared point by point within the tolerance. The CLI also gives the through holes relative to the same minimum, so a contour that sits elsewhere in the element frame is the same part only if its holes keep their distance from its edges. Without it two plates with the same bounding box and thickness but different outlines would share a number. |
| CLI | `create-plate --contour "X,Y;X,Y;..."` instead of `--width` and `--length` (both together are `INVALID_ARGUMENT`). `inspect-elements` and `create-plate` report `contour_points_mm` and `outline` (`rectangle` or `contour`). |

Everything else a plate can do needed no change, because it already works from the authored state
or the sealed mesh: `update-element` (thickness, move, rename), `copy-element`, `delete-element`,
workspaces and undo, welds, assemblies, levels, `export-ifc` (an `IfcPlate` with an
`IfcTriangulatedFaceSet`) and the view snapshot.

## 3. Quantities

The gross volume is the analytic `net_area * thickness` of the polygon prism (shoelace area,
exact for straight edges). With holes, the net volume is the midpoint of the enclosure of the
certified hole operation, as for the box, and the removed volume is the difference. Mass is
`volume * density` for a pinned material and `unknown` without one, never guessed.

## 4. Divergences and limits, recorded rather than hidden

* **Plane `z = 0` only.** F04 accepts a contour in any plane and derives a plate frame (`X_p`, `Y_p`,
  `Z_p`) and local bounds from it (F04-POS-18, F04-POS-19, F04-POS-28). That needs the body-frame
  adapter and the exact planarity test, which are not implemented. F04-POS-18 (a rectangle at
  `z = 30`), F04-POS-28 and the non-planar cases F04-NEG-29 and F04-NEG-38 therefore stay declared
  deviations in `apps/beamcraft-cli/tests/f04_cases.rs`.
* **F04-POS-20 (a pentagon).** F04 would accept it with `geometry_status: unsupported`. Since F06 names
  A02 as the task that lifts `GEO_UNSUPPORTED_CONTOUR`, the pentagon is now evaluated and
  the case's `accepted_draft` outcome holds; the `unsupported` status is no longer produced for a
  planar polygon. F04-POS-19 (a rectangle listed from another corner) and the large-coordinate
  cases F04-POS-25 and F04-POS-26 are accepted too. F04-NEG-30 (a bow tie) is now `INVALID_ARGUMENT`
  as F04 expects.
* **No plate frame.** The mesh is expressed in the element-local frame in which the contour was
  authored; there is no derived frame, so a contour that does not start at the origin is not
  re-centred. `width_mm` and `length_mm` are bounding-box extents, not the sides of a rectangle.
* **No feature ids.** Face, edge and vertex ids (V04 `fid1`) exist for box bodies only. A contour
  plate, like a holed plate, is listed as skipped by `pick` and `snap` features; defining
  their topology is a later task (the V04 note names A02 for it).
* **Straight edges only.** There are no arcs or other curves in a plate outline; a rounded corner
  is written as a polygon.
* **Numbering** treats two outlines as identical only when their vertex lists match point by point
  in the same order from the same start; the same shape written from another corner is a separate
  group. This errs towards not merging.
* **The label of the hole quantity is shared** with box plates (`kernel-g09-through-hole-volume/1`).
  Stored quantity signatures of existing plates are unchanged; a contour plate without holes
  carries the new label.

## 5. Evidence

* Engine unit tests (`cargo test -p beamcraft-engine command::`): the L-shape volume equals
  420,000 mm2 x 10 mm and the divergence-theorem volume of the stored triangles agrees; the same
  contour gives the same mesh digest and fingerprint; a pentagon, a rectangle started from another
  corner and a shifted rectangle are evaluated; clockwise, bow tie, touching, collinear and
  zero-thickness contours are `INVALID_ARGUMENT`; a contour in another plane is
  `UNSUPPORTED_GEOMETRY`; a rectangular hole is cut from an L-shaped plate (4,100,000 mm3, removed
  100,000 mm3) and a hole through the notch is refused with nothing applied.
* `cargo test -p beamcraft-engine numbering`: equal bounding boxes with different outlines are not
  one group.
* `cargo test -p beamcraft-cli --test cli -- contour_plates::` (real binary): create, save, reopen in a fresh
  process, closed-form volume and mass, 12 vertices and 20 triangles, deterministic digest across
  stores, the origin rectangle by size or by contour is the same box, a moved rectangle is a
  contour, holes, thickness edit, move, copy and delete, numbering by outline, IFC export of four
  plates, and every refusal leaving the store empty.
* `cargo test -p beamcraft-cli --test cli -- f04_cases::`: the declared deviations shrink from nine to four.

## Provenance

The contour handling is the G05 extrusion (ear clipping with exact predicates, described in
`crates/kernel/src/profile`) and G09 hole operation; this slice adds no algorithm. The L-shaped
and mirrored-L contours in the tests are invented for these tests and follow no standard or
catalogue.
