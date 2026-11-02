# P05 copy element

Date: 2026-10-18. Status: **experimental vertical slice; P05 acceptance is limited as listed below.**
Branch `feat/p05-copy-element`, stacked on `feat/p04-quantities-update` (PR #66). Merge order:
#63, #64, #65, #66, then this PR.

Plan row: P05 / M, depends on P04 and U05. Result: `CopyElement` with a fresh UUID, remapped
internal links and a shared geometry asset. Acceptance: editing only the copy builds a new blob;
the source parameters, shape and quantities stay as they were; the IDs are independent.

## What is implemented

```text
beamcraft copy-element --store ROOT --id SOURCE_UUID [--name TEXT] [--offset X,Y,Z] [--new-id UUID]
  [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
  model::CopyElement --> engine::copy builds the copy's authored state (fresh id, label, placement
  plus offset; every shape input, profile and material by value) --> footprint check --> U06/U07 commit
```

| Area | What exists |
|---|---|
| `beamcraft_model::command` | `CopyElement { source_id, new_id, name, offset_mm }`: equal identities are `DUPLICATE_ID`, label rule, offset finite and within the coordinate cap; fingerprint. |
| `beamcraft_engine::copy` | `copy_plate`, `copy_member`: the copy's authored state; the resulting placement is re-validated (`LIMIT_EXCEEDED` out of range). |
| `beamcraft_engine::footprint` | `Footprint::for_copy`: writes what `CreatePlate` writes for the new id; reads the new id's lifecycle, the source's lifecycle and authored groups, and the pinned profile and material. The source is never written. |
| `beamcraft_commands::copy` | `copy_element` for plates and members: replay by `op_id`, `OP_ID_REUSE`, `DUPLICATE_ID`, `NOT_FOUND`; the layer engine's observed writes are checked against the footprint; the strict verifier re-checks the copy's mesh and stored records. |

## Decisions

- The body is element-local, so a copy's mesh and quantity records are the source's byte for byte.
  The mesh digest is therefore equal; the copy's later shape edit produces a different mesh and
  leaves the source untouched (tested).
- Definitions (profile, material) are immutable and shared by digest; the copy pins the same ones.
- An offset is added to the placement translation. Rotation is carried over unchanged. A general
  move belongs to P08.
- A new identity that is already in the head is `DUPLICATE_ID`; the same `op_id` with other content
  is `OP_ID_REUSE`.

## Evidence

`apps/beamcraft-cli/tests/copy.rs` (3 end-to-end tests): a plate with a hole and a material is
copied with an offset and a name; the copy has the same mesh digest, volume and mass, the
footprint writes only the copy's identity, the source reads back identical, and a thickness edit of
the copy rebuilds only its mesh; a beam copy keeps the pinned profile and material and reads the
profile without writing it; error codes, replay and `OP_ID_REUSE`. Unit tests cover the command
rules, the fingerprint and `engine::copy`.

## Not done

- "Shared geometry asset" is the equal content digest only: the derived layer still stores one mesh
  prim per element. One stored asset needs a USD reference or instancing decision (owner's call).
- Internal link remapping: elements author no relationships to each other yet, so
  `ElementIndex::copy_plan` (U03) has nothing to remap. The copy keeps its definition links. When
  inter-element relationships exist, `copy_plan` must be applied by this adapter.
- Replay is decided from the head state, not from a stored canonical request (as P04).
- A rotated placement of a copied member is carried over, not re-validated for the member kernel
  beyond what the source already passed.
