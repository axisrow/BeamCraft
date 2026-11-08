# T01.8: magnetic grid mode

Contributor-original MPL-2.0. Delivers T01.8 slice 1 (members) per the O-G5 answers
(`docs/plan/decisions.md` 2026-11-08) and the T01 contract §7.5; builds on the U19 scopes,
the T01.5 pinned grids and the GR-N14 guard (#539). This slice does not claim the T01 parent
gate.

## Storage and derivation

A bound member carries one `bim:followsGrid` relationship (single target: the grid prim) and a
`steel:grid:localAxisMm` `double3[]` with its axis in the grid's local frame. The stored
`bim:axisPointsMm` stays in place as the frozen world snapshot of the last binding event — the
member storage schema requires the property, and the release overwrites it with the current
world. The world geometry of a bound member is **derived at read time**: the decode pass in
`crates/commands/src/plate.rs` maps the local points through the grid's current frame
(`steel::engine::member::grid_world_axis`, exact inverses of the grid's own
`world_to_local`/`local_to_world` in `beamcraft-engine`), so every consumer — bodies,
quantities, snapshots, exports — sees the moved member while the grid edit itself writes only
the grid.

A release (`unbind-grid`, or a rebind naming another grid) erases the local points attr,
authors the explicit empty target list (the PartOf detach pattern), restores the current world
axis and refreshes the derived mesh and quantities from the re-evaluated record, so the stored
picture stops lagging the frame the moment the member stops following it.

## Guards

- **Line edits stay free** (the GR-N14 reconciliation): `referenced_axes` in
  `crates/commands/src/grid.rs` ignores `bim:followsGrid` by kind — a follower references the
  frame, not a line — while any other relationship targeting the grid still pins every axis.
  A binding relationship therefore never blocks a line removal or a line-dropping expression
  edit.
- **Deletion**: `delete-element` of a grid with any incoming relationship — bindings included —
  refuses `DEPENDENCY_CONFLICT`, `GRID_IN_USE` (the delete-side check in
  `crates/commands/src/delete.rs`); this is the symmetric delete guard the GR-N14 note
  deferred until a real producer existed.
- **Hand edits (GR-N22)**: any shape-input rewrite of a bound element refuses
  `DEPENDENCY_CONFLICT`, `GRID_IN_USE` until the binding is released (the layer validation in
  `crates/layer-engine/src/authored/validate.rs`); the binding changes themselves are exempt.
  Nothing detaches implicitly, so the local record and the world snapshot can never diverge
  silently.
- **Pinned grids**: `bind-grid` refuses `FOREIGN_READ_ONLY` (`PC-OWN-002`) for a pinned
  coordination grid before any local work.

## Evidence

`apps/beamcraft-cli/tests/t018_magnetic.rs` runs the real binary: bind → frame move (translate
+ rotate) → derived world verified through `inspect-elements`; the stored truth stays local;
unbind materializes the world and re-arms the strict derived checks; a grid with a follower
refuses deletion until the release; rebinding recomputes the local axis against the new frame
and unblocks the old grid's deletion. The frame-follows commands-level regression for the
blocker-kind distinction lives in `crates/commands/src/grid.rs` (`a_frame_following_binding_
never_pins_a_line`), next to the GR-N14 chain test from #539.

## Limits

- Plates bind in a later slice; the layer accepts part kinds, the command restricts to beams
  and columns today.
- Line-level following (a follower pinned to one grid line) and automatic session-wide
  magnetism stay post-MVP (O-G5).
- GR-N20's refusal is implemented in `bind-grid`, but its end-to-end case needs the T01.5
  coordination harness (a pinned project in the store); it rides the next coordination test
  pass.
- A grid frame rotation carries the double-precision residual of the quaternion roundtrip
  (≈1e-13 mm at 90 degrees on a 6 m member) — the same arithmetic the grid's own placement
  uses.
- The stored world axis of a bound member is intentionally a frozen bind-time snapshot; it is
  never read while the binding is present.

Contributor-original MPL-2.0. No dependencies or assets added.
