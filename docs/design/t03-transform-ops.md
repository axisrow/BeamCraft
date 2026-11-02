# T03 transform operations

Date: 2026-10-31. Status: **implemented; the profile-handedness honesty limit below is the one
open detail.** Branch `feat/t03-transform-ops` (fork delivery for
axisrow/BeamCraft#202), based on `origin/master` `e8c58f87`.

Plan row: T03 / M, depends on V05 (merged), P10, P11. Result: copy, move, rotate, mirror and
arrays, by vector or two points, linear and polar. Acceptance: one undo step per operation;
links and quantities remapped.

## What is implemented

```text
beamcraft update-element --store ROOT --id UUID --move-from P --move-to P
beamcraft update-element --store ROOT --id UUID --rotate-about P --rotate-axis V --rotate-angle RAD
beamcraft update-element --store ROOT --id UUID --mirror-point P --mirror-normal V
beamcraft copy-element   --store ROOT --id UUID --copy-from P --copy-to P
beamcraft array-linear   --store ROOT --id UUID --count N --direction V --spacing MM
beamcraft array-polar    --store ROOT --id UUID --count N --point P --axis V --angle RAD
```

| Area | What exists |
|---|---|
| `beamcraft_model::command` | `ElementPatch` gains the relative fields `rotate` (`ElementRotate`: pivot, axis, angle), `mirror` (`ElementMirror`: point, normal) and `move_mm`; each refuses any absolute placement field and any second relative transform in the same patch, and appends its own versioned fingerprint bytes only when present. `ArrayElements { source_id, name, new_ids, step: ArrayStep }` with `Linear { direction, spacing_mm }` and `Polar { point_mm, axis, angle_rad }`, one to `MAX_ARRAY_COPIES` (100) distinct identities. |
| `beamcraft_engine::transform` | `rotate_placement` (world-axis rotation composed on the placement side), `mirror_placement` (`R' = A·R·L`, `t' = A·t + c` through a local body symmetry `L`), `mirror_plate_local` (side flip + reflected cuts), `mirror_member_local` (reflected two-point path + cuts + traded section offsets), `array_placements` and the array copy builders over the P05 copy rules. |
| `beamcraft_engine::update` | The patch applies the relative fields: a rotate or a move stays placement-only; a mirror is a shape effect (the side, the cuts and a straight member's path are re-encoded, so the body is rebuilt). |
| `beamcraft_commands::edit` | A mirror authors `Change::ThicknessSide` (plates) and `Change::Cuts` (either kind) beside the mesh. A relative transform replays exactly when the head is still the revision the operation itself committed; any later edit is `OP_ID_REUSE`. |
| `beamcraft_commands::array` | One batch: every copy is an independent P05-style duplicate with fresh identities, freshly minted records and the source's level, authored and verified in a single revision (one undo step). The declared footprint is the union of the per-copy footprints and is checked against the layer engine's write set; unsupported source links refuse with `DEPENDENCY_CONFLICT` (DV-REL-011). |

## Decisions

- A move stays the placement patch (P08); the two-point form is a relative `move_mm` the
  engine composes with the current placement, so a retried command is decidable: the head must
  still be the revision the operation committed, otherwise `OP_ID_REUSE`. The same history
  criterion covers `rotate` and `mirror`, whose would-be result cannot be recomputed from a
  head it already transformed.
- A mirror is not rigid, so it is re-encoded through a symmetry `L` of the element's local
  body: the world result is the exact mirror image of `A·R·L·(L⁻¹·local) = A·R·local`. For a
  plate `L` is the local `z = 0` reflection: the thickness side flips (top and bottom trade
  places, middle is its own image, the absent side reads as top and comes back explicit) and
  the plane cuts reflect; contour, holes and corner treatments are through-thickness or
  in-plane and stay. For a straight member `L` is the reflection across the plane
  perpendicular to the axis: the path reflects, the cuts reflect, and the section placement is
  re-encoded for the reversed sweep direction — the end offsets stay at their ends (the
  kernel's net length is asymmetric in them and the reflected path keeps the start anchor),
  the across-width position and offset negate as a pair (the sweep frame's `X_m` flips when
  the path direction reverses) and the roll negates. Record identities are untouched. A
  mirror never shares a patch with a path replacement: one path, one symmetry.
- A polybeam or a curved member refuses a mirror with `UNSUPPORTED_GEOMETRY`: a bent path has
  no expressible mirror symmetry in the authored fields. A plate with bend records (A10)
  refuses for the same reason: the curl of a fold or a roll about the mid-plane reflection has
  no authored form; negating a fold's angle alone would cover folds, but a roll carries no
  direction field, and one honest rule for the whole bent body was preferred. A member with
  hole records (A04.2) refuses too: the hole polygons live in a face's own frame, which has no
  face-aware mirrored encoding yet.
- **Honesty limit:** the profile definition is not mirrored. A member mirror is exact for
  mirror-symmetric sections and keeps the profile handedness of a chiral one (L, Z). A
  mirrored-profile definition (or a mirror flag on the part) belongs to a later task.
- Arrays create independent duplicates, never instances: a shared-definition array is a
  follow-up to P11, not part of the T03 acceptance. A weld or a bolt group refuses an array
  (its element links do not carry over, DV-REL-011), as for a copy.
- An array is one revision: the batch authors every copy at once and the strict verifier
  checks the whole write set, so one undo step takes every copy back (tested).
- The quantity records of a copy are the source's body remapped to the copy's identity; the
  level link follows every copy (tested end to end through the `levels` listing).

## Evidence

`apps/beamcraft-cli/tests/transform.rs` (6 end-to-end tests): a rotation turns the placement,
replays from the head revision and is one undo step, and refuses an absolute placement field
in the same patch; a plate mirror re-encodes the side and the cut, rebuilds the body, and
leaves the published base untouched by the draft; a member mirror reflects the path and a
polybeam refuses; a linear array of a level-assigned plate creates the copies at the stepped
placements with the level link on each, shares the body, and one undo removes them all; a
polar array rotates each copy about the axis and refuses a zero angle or direction; the
two-point move and copy equal the vector forms and are exclusive with them. Engine unit tests
cover the quaternion, mirror and array math and the patch effects; `update.rs` covers the
patch-level side flip and the polybeam refusal.

## Not done

- Mirrored-profile definitions for chiral sections (the honesty limit above).
- Arrays that instance a shared definition (P11 follow-up).
- A GUI entry point: the UI calls the same commands (V05.2 covers select and move today).
