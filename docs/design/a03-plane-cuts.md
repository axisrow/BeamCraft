# A03 first slice: plane cuts

Date: 2026-10-19. Status: **experimental first slice; A03 acceptance is NOT claimed.**
Branch `feat/a03-plane-cuts`, from `master` at `11a2139`.

Plan: the G09 kernel plane cut is the headless API (`plane_cut`); its command and end-to-end
gate belong to A03. As for the A04 pilot, there is no edit command for detailing (the
`UpdateElement` patch of P04 is typed and carries no cut) and no operation-record vocabulary in
the frozen F04 contract, so this slice authors the cut together with the element
(`create-plate`, `create-member`). Exactly one cut per element.

## What is implemented

```text
beamcraft create-plate ... --cut-point X,Y,Z --cut-normal X,Y,Z
beamcraft create-member ... --cut-point X,Y,Z --cut-normal X,Y,Z
  model::CreatePlate.cut / CreateStraightMember.cut (PlaneCut, one per element)
    --> engine: kernel Body + plane_cut(..., Keep::Negative, 0) after any holes
    --> U02 authored element with steel:cut:pointMm / steel:cut:normal + derived mesh
    --> host commit; the verifier re-cuts every element and compares the committed mesh
```

| Area | What exists |
|---|---|
| `beamcraft_model::command` | `PlaneCut { point_mm, normal }`: element-local millimetres, finite, coordinate cap, non-zero normal; `CreatePlate.cut` and `CreateStraightMember.cut`, part of the content fingerprint (fingerprint versions v4/v2, so other cuts under one `op_id` are `OP_ID_REUSE`). |
| `beamcraft_engine` | `apply_cut` (one kernel plane cut, kept half-space opposite the normal, cap face `cut:f.p0`), applied last — after any holes; volume algorithm `kernel-a03-plane-cut-volume/1;tolerance-policy-3`; the cut enters both P04 shape signatures. |
| `beamcraft_layer_engine::authored` | `Shape::{Plate,Member}.cut`, schema descriptor amended (new schema digest; the previous A07.2 and D1/D2 pins stay accepted, stores written before this change are not readable by the new schema); two optional element attributes, a flat `double3` point and `double3` normal, both or neither; validation of both on write and on every load. |
| `beamcraft_cli` | `--cut-point` and `--cut-normal` on `create-plate` and `create-member` (both or neither); the element decoder reads the attributes (inconsistent ones are `INTEGRITY_ERROR`); `inspect-elements` reports `cut` (`null` without one); `copy-element` copies the cut. |

## Decisions (owner review needed)

1. **Contract deviation, recorded.** F04 section 4 says the initial create has no cuts and that
   engineering operations are explicit owned operation records. As in the A04 pilot, this slice
   stores the cut plane as two element attributes instead of operation records, authored with
   the element, and amends the U02 schema descriptor. A logical `CutPlane`/`UpdateElement`
   command and operation records need an F04 amendment and remain A03/P04 work.
2. **The kept side is the side the normal points away from.** The kernel offers
   `Keep::Positive|Negative`; the authored form keeps only point and normal and always keeps
   `normal . (p - point) <= 0`, so the normal points into the trimmed-off material. Any
   point/keep pair is the same plane with the normal reversed, so nothing is lost. A scaled
   normal describes the same plane (the kernel decides sides exactly and never normalises for
   that decision).
3. **Volume is an enclosure midpoint.** As for holes, the kernel certifies the closed shell and
   gives an enclosure of the exact signed-volume sum; the quantity is its midpoint with status
   `current`. The label is `kernel-a03-plane-cut-volume/1;tolerance-policy-3`, applied whenever
   a cut is present (holes and cut together are one chain ending in the cut). After the P-Mass
   sync, a cut member's volume replaces the `effective area x axis length` value with this
   midpoint; an uncut member keeps the P-Mass algorithm and value unchanged.
4. **The kernel decides geometry.** The plane must strictly cross the body: some vertex
   strictly on each side and no face in the plane. A plane that misses the element is
   `INVALID_ARGUMENT` (`GEO_OP_NO_EFFECT`, proposed code); a tangent plane is
   `UNSUPPORTED_GEOMETRY` (`GEO_OP_UNSUPPORTED`); nothing is authored.
5. **Scope of the slice.** Plate or member (rectangle box or A07.1 section extrusion), one cut,
   authored at create. Cut features are outside the FeatureId table (the cap id
   `cut:f.p0` is the kernel's provisional local id, as in G09); placement rotation is still
   `UNSUPPORTED_GEOMETRY`, so the cut coordinates are element-local in the unrotated frame.

Update 2026-10-23 (F06/F04 amendment A-2, see [the A-2 notes](g08-f06-a2-notes.md)): decision 1's
deviation is replaced by the `plane_cut` operation record and the `CutByPlane` command of F04
(sections 4 and 7). The contract names the kept side explicitly (`keep`: `positive` or
`negative`); decision 2's fixed "keep the side the normal points away from" describes this slice
only. The two `steel:cut:*` attributes are deprecated, and moving to the records belongs to the
A03 acceptance task. Until then the code in this note is not the contract.

## A03 acceptance slice: the owned `CutByPlane` operation record

Status: **experimental; A03 acceptance is the owner's.** Branch `feat/a03-cut-operation-record`.
The cut is no longer an element attribute authored at create: it is an owned operation record
(F04 section 4, amendment A-2), added by the `CutByPlane` command (`cut-by-plane` on the CLI) to
an already stored plate, beam or column.

| Area | What changed |
|---|---|
| `beamcraft_model::command` | `CutByPlane { target_id, operation_id, point_mm, normal, keep }` with `KeepSide`; the stored record `CutRecord { operation_id, ordinal, point_mm, normal, keep }`; `create-plate`/`create-member` no longer take a cut (F04 section 4: the initial create has none). `CutByPlane` fingerprints as `BeamCraft/A03/CutByPlane/v1`; the create fingerprints moved to `CreatePlate/v5` and `CreateStraightMember/v3`. |
| `beamcraft_engine` | `AuthoredPlate.cuts` / `AuthoredMember.cuts: Vec<CutRecord>`, replayed in ascending ordinal order after any holes, each keeping its own side (`cut.rs::replay_cuts`). `cut_by_plane` admits the command: target lifecycle (`NOT_FOUND`), record identity across the project (`DUPLICATE_ID`), the per-element ordinal counter (past the kernel's `cut:f.p<ordinal>` naming capacity of 999 it is `LIMIT_EXCEEDED`, field token `kernel_capacity`); the replay refusal of the whole command stays `INVALID_ARGUMENT`/`UNSUPPORTED_GEOMETRY`. Signatures and body-cache keys take the record list (identity-free content, in order). `CopyElement` mints fresh deterministic record ids (UUIDv7 hashed from the copy, the source record and the ordinal) and keeps the ordinals. |
| `beamcraft_layer_engine` | `Shape::{Plate,Member}.cuts`; five parallel attributes replace the old two (`steel:cut:operationId=string[]`, `steel:cut:ordinal=int[]`, `steel:cut:pointMm=double3[]`, `steel:cut:normal=double3[]`, `steel:cut:keep=token[]`; all five or none, one length, ascending ordinals ≤ 999); `Change::Cuts` rewrites them as a whole for an existing element (empty removes them); new schema digest — stores written before this change are not readable, as with every earlier U02 pin. |
| `beamcraft_cli` | `cut-by-plane --id --operation-id --point --normal --keep positive|negative` publishes the next revision: record + rebuilt mesh + rebuilt quantity records in one transaction, so the stored volume/mass are current the moment the command is accepted. `inspect-elements` reports `cuts` (empty array without records); `copy-element` copies the records with fresh ids. The create-time `--cut-point/--cut-normal` options are gone. |

**Freshness.** One command is one revision and one undo unit (the P07 snapshot undo takes the cut
back and `redo` reapplies it; quantities are rebuilt in the same transaction, never left stale).
Feature stability is the kernel's: base-body faces keep their ids, the cap face is
`cut:f.p<ordinal>`, and a feature the plane removes or splits no longer resolves — the explicit
loss (`lookup-feature` answers `GEO_FEATURE_NOT_FOUND`).

**Scope of the slice.** One element carries many cut records (no designed limit; the ordinal
capacity is an implementation limit), but there is still no `UpdateCut`/`RemoveCut` (they follow
`UpdateHole`/`RemoveHole`), no cut features beyond the cap face, and no UI authoring.

## Not done (not claimed)

* No `UpdateElement`-level cut removal, no `UpdateCut`/`RemoveCut` command.
* No UI authoring of cuts; the view snapshot shows the cut mesh like any other.

## Evidence

* Engine tests: exact replay volumes of a cut plate (`250 * 500 * 10 mm3`), a kept opposite
  half-space, two records in ordinal order and a cut after a hole, the cap named by the
  record's ordinal, every refusal (missing plane, tangent plane, zero normal, non-finite);
  `cut_by_plane` admission (next ordinal, footprint, `NOT_FOUND`, `DUPLICATE_ID`,
  `kernel_capacity`); a copy with fresh record ids, the same ordinals, deterministic.
* Layer-engine test: the five parallel attributes are authored for a plate and a member, the
  exported layers pass the structural validator, and invalid or half-authored records are
  refused before anything is written.
* `cargo test -p beamcraft-cli --test cuts` (real binary): a cut plate and a cut beam with
  exact volume and mass, a second record taking the next ordinal, fresh-process reopen and
  re-verification, the saved layer text read by plain string matching, every rejection code
  (`INVALID_ARGUMENT`, `UNSUPPORTED_GEOMETRY`, `NON_FINITE_NUMBER`, `LIMIT_EXCEEDED`,
  `NOT_FOUND`, `DUPLICATE_ID`, `OP_ID_REUSE`), a retry replaying, a copy carrying the records
  with fresh ids, feature lookup resolving the cap and reporting the clipped corner, and the
  workspace undo/redo taking the cut back and reapplying it.
