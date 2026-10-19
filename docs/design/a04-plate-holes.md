# A04: plate through holes

Date: 2026-10-17 (pilot). Acceptance record: 2026-10-19, verified on `origin/master` `11a2139`.
Status: **implemented and verified as one result with `create-plate`; the contract deviation of
decision 1 and the cross-target question of decision 2 still need owner review.** The pilot
landed on `master` through the stacked P02/P03 branch line; this record describes the state on
`master` at `11a2139`.

Plan: the G09 kernel hole is the headless API (`through_hole`); its command and end-to-end gate
belong to A04. This slice authors holes together with the plate (`create-plate`), because there
is no hole edit command yet (P04's `UpdateElement` does not carry holes) and no operation-record
vocabulary in the frozen F04 contract. The full A04 result: hole creation rules, validation,
quantities effect, CLI and engine parity, and end-to-end tests.

## The result

```text
beamcraft create-plate ... [--hole-rect "X,Y,W,H;..."] [--hole-round "X,Y,D;..."] [--hole-segments N]
  CLI generators --> clockwise polygons in plate-local XY (the only stored form)
  model::CreatePlate.holes_mm (caps) --> engine: kernel Body::from_plate + one through_hole per hole
  --> U02 authored plate with steel:plate:holePointsMm / steel:plate:holeVertexCounts + derived mesh
  --> host commit; the verifier re-cuts every plate and compares the committed mesh
```

| Acceptance item | Where it stands |
|---|---|
| Creation rules | The engine's hole application in `beamcraft_engine::command`: one kernel cutter per hole, in order, overshoot 1 mm past both faces. The kernel decides, with exact predicates, that a hole is a clockwise simple polygon strictly inside the outline and clear of the others; a refusal is `INVALID_ARGUMENT` for a bad polygon and the kernel's own code otherwise, and nothing is applied. |
| Validation | `beamcraft_model::command`: at most 64 holes of 3 to 1024 points, finite, within the coordinate cap, part of the content fingerprint (other holes under one `op_id` are `OP_ID_REUSE`). The layer engine validates both attributes on write and on every load; inconsistent ones are `INTEGRITY_ERROR`. |
| Quantities effect | `SolidMesh` (the certified mesh of the holed body); the volume is the midpoint of the kernel's certified enclosure, status `current`, algorithm `kernel-g09-through-hole-volume/1;tolerance-policy-3`; mass uses it like any other volume. The P04 stored records carry a shape signature over the holes, publishing is strict (a stored record that disagrees with the kernel cannot become a revision), and update, copy, move and undo keep them current. |
| CLI and engine parity | The CLI generators are the input edge only: `--hole-rect`, `--hole-round`, `--hole-segments` produce the polygons and everything else runs the engine's `CreatePlate`. `plate_json` reports `hole_count` and `holes_mm`; the plate decoder reads the attributes. The engine and the CLI evaluate the same command and the same verifier re-cuts both. |
| End to end | `cargo test -p beamcraft-cli --test holes` (real binary, 5 tests), plus holes running through the later slices' suites: copy, update (thickness rebuild), move and delete, workspace undo and pick. |

Integration with the slices that landed after the pilot: an independent copy keeps the body and
mesh digest and its own stored record; a thickness update re-cuts the holes and both records
follow; a holed plate has no FeatureIds and is never a pick hit (F06 amendment A-1 leaves hole
FeatureIds undefined); a holed plate and a beam share one pinned snapshot.

## Decisions (owner review needed)

1. **Contract deviation, recorded.** F04 section 4 says the initial create has no cuts, holes or
   welds and that engineering operations are explicit owned operation records. This slice stores
   the hole polygons as two plate attributes instead of operation records, authored with the
   plate, and amends the U02 schema descriptor (the schema digest changes; stores written before
   this change are not readable by the new schema). A logical `AddHole`/`UpdateElement` command
   for holes and operation records need an F04 amendment and remain open work.
2. **A "round" hole is a polygon.** The CLI generates a regular N-gon (default 32) inscribed in the
   circle; the model stores only the polygon, its volume is exact for that polygon, and the kernel's
   curve approximations (flagged `approximated`, which F06 0.1.0 does not cover) are not used. The
   vertices use the platform sin and cos at the input edge and are authored as the doubles they
   round to; the cross-target reproducibility question stays open for the same reason as for roll.
3. **Volume is an enclosure midpoint.** The kernel certifies the closed shell and gives an
   enclosure of the exact signed-volume sum whose width is rounding only; the quantity is its
   midpoint with status `current`. The mass uses it like any other volume.
4. **The kernel decides geometry.** Strictly inside the outline, clear of other holes, clockwise,
   simple: a refusal is `INVALID_ARGUMENT` for a bad polygon (the kernel's own code otherwise) and
   nothing is authored.

## Still open (not part of this result)

* No `AddHole`/hole edit/remove command, no operation records: authoring is create-time only
  (decision 1). Hole features stay out of the FeatureId table by F06 amendment A-1.
* Holes only in the origin rectangle plate (the P01 contour rule shared with plain plates); no
  member holes, bolts, slots as curves or plane cuts (A03/A05).
* Placement rotation is `UNSUPPORTED_GEOMETRY` pipeline-wide; the layer engine runs in the CLI
  process. Cross-target byte evidence for the authored polygon vertices (decision 2).

## Evidence

* `cargo test -p beamcraft-cli --test holes` (real binary, 5 tests): exact volume
  `(1000*500 - 100*100 - 50*50)*10 = 4,875,000 mm3` and mass 38.26875 kg at 7850 kg/m3, a 64-gon with
  the polygon-area volume and clockwise signed area, saved layer text with the polygon arrays, plate
  with a hole next to a beam in one snapshot, every rejection code, replay and `OP_ID_REUSE`.
* `cargo test -p beamcraft-cli --test update` and `--test copy`: a thickness update of a holed
  plate gives the exact new volume and mass with both stored records current; an independent copy
  shares the body and mesh digest. `--test undo`, `--test relocate` and `--test pick` run holed
  plates through workspace undo, move, delete and picking (a holed plate is never a hit).
* Engine tests: divergence-theorem volume of the returned mesh, refusals (counter-clockwise,
  outside, crossing, touching, overlapping). Layer-engine test: authored arrays, validator and
  invalid input.
* pxr oracle (local, opt-in, usd-core 25.11): reads the hole arrays and the cut mesh, derives the
  volume from its own mesh and matches the closed form (rectangle plus 32-gon), and
  `ComplianceChecker` reports nothing.

## Roadmap entry (proposed, not applied)

- 2026-10-19: A04 accepted: `create-plate` authors through holes (`--hole-rect`, `--hole-round`) as
  clockwise polygons cut by the G09 kernel, stored as two plate attributes under an amended U02
  schema (a recorded deviation from the F04 operation-record rule, owner review pending). Exact
  polygon volume and mass, stored quantity records that stay current through update, copy, move and
  undo, verified at publish and reopen; no hole edit command, operation records or hole FeatureIds
  (F06 A-1 leaves them undefined).
