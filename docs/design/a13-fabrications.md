# A13 first slice: fits, cuts, chamfers, split and combine

Date: 2026-10-28. Status: **experimental first slice; A13 acceptance is NOT claimed.**
Branch `feat/a13-cuts-chamfers-split`. The member polygon cut of epic #556 slice 1 joined
2026-11-20 on `feat/a13-member-prism-cuts`.

The seven fabrication operations of the A13 card, on the plate and straight-member forms
already on `master`, as A-2 operation records: line, polygon and part cuts, fitting an end to
another part, chamfers, splitting a part in two and combining two parts. Everything reuses the
two certified kernel operations; the commands derive their records at admission and the
ordinary replay rebuilds each body.

## What is implemented

```text
beamcraft cut-by-line     --id --operation-id --point-a --point-b --axis --keep
beamcraft cut-by-polygon  --id --operation-id --polygon "X,Y;..."
beamcraft cut-by-part     --id --operation-id --tool
beamcraft fit-end-to-part --id --operation-id --tool
beamcraft chamfer-part    --id --operation-id --vertex-index --first-distance --second-distance
beamcraft split-part      --id --new-id --new-name --operation-id --point --normal --keep
beamcraft combine-parts   --survivor --consumed --operation-id
  model::CutByLine | CutByPolygon | CutByPart | FitEndToPart | ChamferPart | SplitPart | CombineParts
    --> engine fabricate.rs: the derived record or state change, admitted like CutByPlane
    --> one revision, one undo unit; mesh, volume and mass rebuilt in the same transaction
```

| Operation | Stored record | Replay |
|---|---|---|
| line cut | `plane_cut` (A03 form) | kernel `plane_cut` of the plane through the line and the axis |
| polygon cut (plate) | through-cut record, kind `polygon_cut` | kernel `through_hole` prism subtraction |
| polygon cut (member) | owned `MemberHoleRecord`, kind `polygon_cut`, a through cut | kernel member prism subtraction (`member_through_hole`, the G14 cutters) |
| part cut | through-cut record from the tool plate's mapped prism | kernel `through_hole` |
| fit | `plane_cut` from the tool face nearest the target's extent midpoint | kernel `plane_cut` |
| chamfer | `plane_cut` through two edge distances at a contour vertex | kernel `plane_cut` |
| split | a `plane_cut` record on each half; the second element is created in the transaction | `plane_cut` twice |
| combine | the survivor's extended axis; the consumed member is deleted | member re-evaluation |

Polygon cuts share the A04 hole record form with an explicit `steel:hole:kinds` token
(`hole` / `polygon_cut`), one shared per-element ordinal counter and the same `cut:f.h<ordinal>`
wall ids, so a cut never numbers as a fastener hole. Instance prims mirror the kind token with
the other shared shape properties. The member polygon cut (epic #556 slice 1) mirrors the same
kind on the member's own record list: `MemberHoleRecord` grows the `kind` field, stored as the
eleventh record attribute `steel:mhole:kinds` (`hole` / `polygon_cut`; the descriptor's ten
become eleven, pre-alpha stores are not migrated, the standing rule), and joins the member's
per-member counter and `cut:f.h<ordinal>` wall ids. A part cut on a member target stays the
documented `UNSUPPORTED_GEOMETRY` (slice 2 of the epic); a combine refuses a parallel axis
offset across the survivor's line.

## The member polygon cut (#556 slice 1)

`cut-by-polygon` grows the member form: `--axis-point "U,V,W" --axis-direction "X,Y,Z"` plus
the polygon, in the A04.2 member-hole frame (the origin is the first axis point, `u` along
`X_r`, `v` along `Y_r`, `w` along the axis `Z_m`; the polygon lives in the hole-frame XY, `x`
along `+Z_m`, `y` along the direction crossed with `+Z_m`, relative to the axis point). The
cut is a through cut only; the direction must stay in the member's section plane (no axial
component — the kernel cutter is perpendicular to the axis). The command derives one owned
`MemberHoleRecord` of kind `polygon_cut` (authoritative polygon, extent `through`, no owner)
at admission — the shared project-wide record identity, the member's 64-record cap, the
member's persistent ordinal counter — and the ordinary member replay subtracts the prism with
the certified G14 operation. The frame fields must be present exactly when the target is a
member; a mismatch is an argument error, not a geometry one. `update-hole`/`remove-hole`
refuse a polygon-cut record with `INVALID_ARGUMENT`, exactly as the plate records do.

Replay ceilings (each inherited from the member-hole stack the cut rides on, all typed
refusals that change nothing): a member whose element frame is not the coordinate axes (a
roll, a skew), a curved member or polybeam; a footprint that crosses a wall outline, touches
a hard edge or another record. Composition of several records on one member works when the
records run along the member's `Y_r` (the kernel's Z, whose band walls the X-extrusion
contract admits); a record along `X_r` (the kernel's Y) leaves band walls that are neither
axis-parallel nor kernel-Z-parallel, so the next record on that member refuses with
`BodyNotPrism` (the standing G14.2 limitation — `steel::engine::member_hole` regression-tests
the rule). Hollow sections with sharp corners (a zero radius) and solid sections are served;
a hot-rolled hollow section whose walls mix axis-aligned flats with fillet arcs refuses with
the standing wall-mix refusal (`BodyWallUnsupported`, lifting unassigned). After a cut on a
faceted wall (a CHS) the member keeps one record for the same reason. The removed volume is
the certified exact value of the replay: the parallel-wall identity `A * t / |n . d|` on flat
walls, the clip pieces' moment enclosure on faceted ones.

Inventory of the first sub-task's fits-and-chamfers row: `fit-end-to-part` and `cut-by-line`
already serve member targets (derived `plane_cut` records, replayed); `chamfer-part` refuses a
member target with `UNSUPPORTED_GEOMETRY` — its admission is plate-shaped (a contour vertex
plus two distances in the contour plane), and a member-end chamfer is a plane-cut pair at an
end or a profile-level corner treatment, still unassigned.

## Decisions (owner review needed)

1. **Derived records.** The line cut, fit and chamfer parameters are resolved once at
   admission; the stored record is the derived plane, so the stored model does not keep the
   line, the tool reference or the vertex distances. Moving the tool afterwards never updates
   the cut. A kind tag on the record (as for polygon cuts) would restore the provenance when
   UI authoring needs it.
2. **Through-cuts only.** Polygon and part cuts are through-cuts with the kernel's
   strictly-inside admission; a polygon crossing the outline and a tool turned across the
   target are refused `UNSUPPORTED_GEOMETRY` until the kernel grows prism/CSG subtraction.
3. **Split and combine are state changes, not geometry records.** A split creates the second
   element in the same transaction and appends the matching `plane_cut` record to each half;
   a combine extends the survivor's axis and deletes the consumed member, with the revision
   ledger carrying the operation. Both refuse instance-shared elements.
4. **Combine restrictions.** Same pinned profile, orientation, roll, section placement and
   material, collinear touching or overlapping straight axes on one line; a consumed member
   with cut records is refused (they would be lost).

## Not done (not claimed)

* Member-tool fits, plate combines (polygon union), polygon cuts on polybeams, oriented
  (non-parallel) part cuts, `cut-by-part` with a member target (epic #556 slice 2, the
  oblique tool included), member-end chamfers, A12 arc-contour acceptance rows.
* No UI authoring; the view snapshot shows the result like any other body.
