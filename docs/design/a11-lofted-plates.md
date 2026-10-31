# A11: lofted plate

Date: 2026-10-30. Status: first slice on `feat/a11-lofted-plate`; the acceptance items
(closed certified mesh, twisted/self-intersecting refusals) are delivered; numbering, copy
and drafting integration stays open. Task card: `docs/plan/tasks.md` A11 / M; issue #191
(fork mirror). Depends on G13 slice 1 (`docs/design/g13-ruled-surfaces.md`).

## What is implemented

A lofted plate is a plate whose body is the G13 ruled loft of two authored face contours
that need not be coplanar: one straight ruling per vertex pair, wall quads along the
contour edges, the two caps as the G05 triangulations, sealed by the G09 certification.
The sheet thickness is the distance between the contours themselves, so the command has
no thickness, holes, cuts or corner treatments — those belong to extruded plates and the
stored form keeps them apart.

```text
beamcraft create-lofted-plate --store ROOT --start "X,Y,Z;..." --end "X,Y,Z;..."
    [--name] [--class] [--translation] [--orientation-xyzw] [--material-digest] [identities]
beamcraft inspect-lofted-plates --store ROOT
  model::CreateLoftedPlate -> engine loft_plate.rs: contour admission + ruled::loft
    -> one revision; certified mesh, volume, area and mass published in the same transaction
```

## Decisions

1. **Same register, one token.** The element keeps `bim:elementKind = "plate"` and the
   `PRT_` path; the stored record is discriminated by `steel:plate:form = "lofted"` with
   the two contour arrays (`steel:plate:loftStartMm`, `steel:plate:loftEndMm`), all three
   or none, like the polygon-cut kind token shares the hole record form. A lofted plate
   carries no `steel:plate:contourPointsMm`/`thicknessMm`; an extruded plate stores none
   of the loft attributes. The structural validator enforces the pairing.
2. **Shared contour admission, exact planarity.** Each contour goes through the F04
   contour admission (count, floor, winding, planarity band, exact simplicity — the
   self-intersecting refusal), then is re-encoded in its certified frame for the ruled
   primitive. A near-planar admitted contour is refused (`UNSUPPORTED_GEOMETRY`) instead
   of repaired into an exact frame, as for framed plates.
3. **The certification owns the correspondence.** A renumbered (twisted) end contour
   twists the walls through the body and is refused by the G09 seal; the engine maps that
   to `UNSUPPORTED_GEOMETRY` without trying to guess the intended renumbering.
4. **Quantities.** Volume is the certified enclosure midpoint; mass is current only with a
   density the pinned material authors, exactly like a plate. The shape signature covers
   the two contours and the algorithm; with no operations the net and gross signatures are
   the same digest. Stored lofted quantities are verified at creation; the committed-plate
   re-verification does not re-evaluate them yet (the weld precedent).

## Not done (not claimed)

* `update-element`, `copy-element`, numbering, drafting and IFC/BOM export do not yet
  address lofted plates; the element views list them, the plate pipeline ignores them.
* Arc-contour inputs (`RuledContour::arc`) are reachable only through the engine API;
  the CLI takes authored polygons.
* A10 bent plate shares the loft primitive and states its own outer/inner bend contours.
