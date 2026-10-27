# A13 first slice: fits, cuts, chamfers, split and combine

Date: 2026-10-28. Status: **experimental first slice; A13 acceptance is NOT claimed.**
Branch `feat/a13-cuts-chamfers-split`.

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
| polygon cut | through-cut record, kind `polygon_cut` | kernel `through_hole` prism subtraction |
| part cut | through-cut record from the tool plate's mapped prism | kernel `through_hole` |
| fit | `plane_cut` from the tool face nearest the target's extent midpoint | kernel `plane_cut` |
| chamfer | `plane_cut` through two edge distances at a contour vertex | kernel `plane_cut` |
| split | a `plane_cut` record on each half; the second element is created in the transaction | `plane_cut` twice |
| combine | the survivor's extended axis; the consumed member is deleted | member re-evaluation |

Polygon cuts share the A04 hole record form with an explicit `steel:hole:kinds` token
(`hole` / `polygon_cut`), one shared per-element ordinal counter and the same `cut:f.h<ordinal>`
wall ids, so a cut never numbers as a fastener hole. Instance prims mirror the kind token with
the other shared shape properties. A member target of a polygon or part cut is the documented
`UNSUPPORTED_GEOMETRY`; a combine refuses a parallel axis offset across the survivor's line.

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

* Member-tool fits, plate combines (polygon union), member-end chamfers, polygon cuts on
  polybeams, oriented (non-parallel) part cuts, A12 arc-contour acceptance rows.
* No UI authoring; the view snapshot shows the result like any other body.
