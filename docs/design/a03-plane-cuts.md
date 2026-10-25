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

## Not done (not claimed)

* No `UpdateElement`/remove-cut command, no operation records, no cut features in the
  FeatureId table, no second cut per element.
* No UI authoring of cuts; the view snapshot shows the cut mesh like any other.

## Evidence

* Engine tests: exact volume of a cut plate (`250 * 500 * 10 mm3`) and of a cut after a hole,
  a scaled normal giving the same body, divergence-theorem volume of the returned mesh,
  member rectangle and section cuts, every refusal (missing plane, tangent plane, zero normal,
  non-finite).
* Layer-engine test: the attributes are authored for a plate and a member, the exported layers
  pass the structural validator, and invalid or half-authored cuts are refused before anything
  is written.
* `cargo test -p beamcraft-cli --test cuts` (real binary): a cut plate and a cut beam with
  exact volume and mass, fresh-process reopen and re-verification, the saved layer text read by
  plain string matching, every rejection code, replay and `OP_ID_REUSE`, and a copy carrying
  the cut.
