# P08 move and delete

Date: 2026-10-18. Status: **experimental vertical slice; P08 acceptance is limited as listed below.**
Branch `feat/p08-move-delete`, stacked on `feat/p07-undo-redo` (PR #69). Merge order:
#63 to #69, then this PR.

Plan row: P08 / M, depends on P04 and U03. Result: move and delete with relationship preconditions.
Acceptance: a move updates the placement and invalidates only world measurements, with no shape
copy; a delete is refused while a dangling link would remain; footprints are complete.

## What is implemented

```text
beamcraft update-element --store ROOT --id UUID --translation X,Y,Z [--op-id UUID ...]
beamcraft delete-element --store ROOT --id UUID [--op-id UUID ...]
```

| Area | What exists |
|---|---|
| `beamcraft_model::command` | `ElementPatch::translation_mm` (absolute translation, finite, within the coordinate cap; part of the fingerprint only when present, so earlier fingerprints are unchanged). `DeleteElement { target_id }` and its fingerprint. |
| `beamcraft_engine` | `update::{effect, patch_plate, patch_member}` carry the move (translation replaced, rotation kept, result validated); `Footprint::for_update(.., placement, ..)` and `Footprint::for_delete`. |
| `beamcraft_layer_engine::authored` | `Change::Delete { id }`: removes the element prim from the authored layer and its result prims from the derived layer, drops it from `/World/Steel`, writes every group, refuses elements whose prims live in other layers, and cannot share a batch with another change of the element. |
| `beamcraft_commands::edit` | A placement patch becomes `Change::Placement` only: no mesh change, no quantity change. |
| `beamcraft_commands::delete` | `delete_element`: replay by `op_id`, `OP_ID_REUSE`, `NOT_FOUND`; the layer engine's element index decides the links (`Author::check_delete`) before anything is authored; the observed writes are checked against the footprint; commit through U06/U07 with the strict verifier. |

## Decisions

- Contract F04 section 7 lists placement as a field of `UpdateElement`, so a move is the placement
  patch and not a new command. It is absolute (not an offset): that keeps a retry of the same
  `op_id` decidable from the head state. Rotation is kept; a rotated placement stays
  `UNSUPPORTED_GEOMETRY` everywhere, so no rotation can be set yet.
- The body, mesh and stored quantities are element-local, so a move does not touch the derived
  layer (tested: `changed_layers == ["authored.usda"]`, volume and mass stay `current`). The
  footprint still declares the derived group as written, which is the F04 world-measurement
  invalidation; the engine footprint covers the layer engine's conservative write set.
- A delete removes the element completely and nothing else. Its footprint writes all seven groups
  (the contract says "write lifecycle"; removing the prim necessarily removes every group, and the
  layer engine reports exactly those writes). It reads the lifecycle and the relationships.
- Relationship preconditions come from the layer engine's element index: an incoming link is
  `DEPENDENCY_CONFLICT`, an index that cannot prove the links complete is `DEPENDENCY_UNAVAILABLE`.
  There is no cascade and no foreign-source mutation. Elements author no links to each other yet;
  the end-to-end test adds a link the way another tool or a later command would.
- Pinned definitions are immutable and shared, so they stay when their last user is deleted.
- Undo (P07) needs no change: a deleted element and a move come back with the restored layers
  (tested).

## Evidence

`apps/beamcraft-cli/tests/relocate.rs` (4 end-to-end tests): a move of a plate (with a hole and a
material) and of a beam changes the placement only (derived layer and mesh digest unchanged,
quantities current, footprint is placement and derived); error codes and replay of a move; a delete
removes the element from both layers, keeps the other element byte for byte, keeps the definitions,
writes seven groups, replays, is `OP_ID_REUSE` for another element, `NOT_FOUND` when gone, and an
empty model is still valid; a delete of a link target is `DEPENDENCY_CONFLICT` with nothing written,
and the plate that holds the link can be deleted, after which the target can too.
`crates/layer-engine/tests/authored.rs` covers `Change::Delete` (both layers, footprint, opinions,
second delete, batch rule); unit tests cover the command rules, `for_update`/`for_delete` and the
patch. `apps/beamcraft-cli/tests/undo.rs` undoes a move and a delete.

## Not done

- Rotation cannot be changed; "move" is a translation.
- No relationship-authoring command exists, so a blocked delete is only reachable through layer
  content that adds a link.
- Replay is decided from the head state (a retry after a later edit of the same element is
  `OP_ID_REUSE`), as in P04 and P05.
- Deleting does not garbage-collect blobs or definitions.
