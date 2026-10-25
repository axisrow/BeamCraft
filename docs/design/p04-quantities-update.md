# P04 stored quantities and update-element

Date: 2026-10-18. Status: **experimental vertical slice; P04 acceptance is limited as listed below.**
Branch `feat/p04-quantities-update`, stacked on `feat/a04-plate-holes` (PR #65). Merge order:
#63, #64, #65, then this PR.

Plan row: P04 / M. Result: an element can be edited (name, material, thickness) and its stored
quantities are persisted next to the derived mesh and tracked as current, stale or missing.

## What is implemented

```text
beamcraft update-element --store ROOT --id UUID --op-id UUID
  [--name TEXT] [--material-digest HEX|none] [--thickness MM]
  model::UpdateElement { target_id, ElementPatch } --> engine::update decides which groups change
  (authored attributes, derived mesh, stored quantities) --> footprint check --> U06/U07 commit
beamcraft inspect-elements --store ROOT   (plates and members now carry stored_quantities)
```

| Area | What exists |
|---|---|
| `beamcraft_model::command` | `ElementPatch`, `UpdateElement`: empty patch is INVALID_ARGUMENT; label, thickness finiteness and cap; `material: Option<Option<digest>>` (inner `None` clears the material); fingerprint. |
| `beamcraft_engine::quantity` | `QuantityRecord` (volume, mass, statuses, shape and mass signatures, algorithm), `assess` (current, stale, missing), `plate_shape_signature`, `member_shape_signature`, `mass_signature`, `record_for`. |
| `beamcraft_engine::update` | `effect`, `patch_plate`, `patch_member`; thickness on a member is UNSUPPORTED_PROPERTY. |
| `beamcraft_layer_engine::authored` | Optional mesh attributes `bim:volumeMm3`, `bim:volumeStatus`, `bim:massKg`, `bim:massStatus`, `bim:shapeSignature`, `bim:massSignature`, `bim:quantityAlgorithm`; mesh writes replace the whole prim, so no stale attribute survives. |
| `beamcraft_cli::edit` | `update_element`: only changed groups are written; replay by `op_id`; a no-op patch is INVALID_ARGUMENT. |
| `beamcraft_cli::plate` | The publish-time verifier is strict: a stored record that disagrees with the re-evaluated one fails with INTEGRITY_ERROR. `inspect-elements` is not strict and reports freshness. |

## Decisions

- Signatures are our own: typed inputs plus algorithm id plus `TOLERANCE_POLICY_VERSION`. The
  mass signature is the shape signature plus the density. They are not the F06 section 7 shape key.
- **Update (P-Mass, [design](p-mass-section-area.md)).** The member volume is no longer the volume
  of the mesh: it is `effective section area x axis length`, where the effective area is the
  declared catalogue area of the pinned profile if it carries one, else the exact computed area.
  The member shape signature now also covers the area source and both areas (a changed declared
  area makes the stored quantity stale) and the list of operations (empty for members today). The
  member algorithm labels changed, so records written by this slice's first version read as
  `stale` (a superseded label), never as corruption. Quantities are reported as gross (nominal
  body) and net (after operations); `volume` and `mass` are the net values.
- A material change rewrites the mass record and leaves the mesh bytes untouched. A rename
  changes only the authored layer; the derived layer stays byte-identical (tested).
- Clearing the material makes the mass unknown: the attribute is absent, never zero.
- A thickness change rebuilds the mesh and the volume record.

## Evidence

`apps/beamcraft-cli/tests/update.rs` (7 end-to-end tests): rename touches only the authored layer;
a material change updates the mass at the same mesh; clearing the material leaves the mass unknown;
a thickness change rebuilds the mesh (9 800 000 mm3, 76.93 kg for the fixture); member rename plus
material; error codes, replay and OP_ID_REUSE; an external edit without the verifier reads as
stale; publish refuses a stale record. The pxr oracle reads the stored volume, mass and signature
as plain USD attributes.

## Not done

- Float32 mesh points (precision policy), mesh `extent` and the F06 shape key are still open.
- Replay by `op_id` is computed from the head state, not from a saved canonical request.
- No command envelope, workspace or undo yet (P06, P07).
- Thickness edits for members, profile edits and placement edits are out of scope (P08 for placement).
