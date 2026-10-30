# A10: bent plate — folds and rolls as owned operation records

Date: 2026-10-31. Status: design note for the A10 bent plate (`beamcraft-model` command
`BendPlate`, `beamcraft-engine` module `bend`, the `bend-plate` CLI command). Task card:
`docs/plan/tasks.md` A10 / M (milestone M1); issue axisrow#190. Depends on G13 (the ruled
loft primitive, merged) and A12 (thickness side and corner treatments, merged). Foundation
for nothing else; A11 (lofted plate) shares the G13 primitive but authors its two face
contours directly.

## 1. The developed blank is the authored contour

A bent plate is authored exactly like any plate — a contour in the element-local plane
`z = 0`, a thickness — plus bend operation records that say how the mid-surface leaves the
plane. The contour of a bent plate is its **developed mid-surface outline** (the unfold):

* bending about the mid-surface preserves the development, so the unfolded outline is the
  authored contour and is reported unchanged (`outline_mm`, `contour_points_mm`);
* one bend's annular-sector cross-section area is `|angle| x t x (r + t/2)` — exactly the
  thickness times the developed length the bend consumes — so the analytic volume of any
  bent or rolled plate stays `outline area x thickness`. The radii only redistribute the
  material; the sealed mesh sits inside that value within the reported facet deviation.

## 2. Records

`BendRecord { operation_id, spec }` on the plate, appended by `BendPlate` (the A-2 record
vocabulary of cuts and holes). At most `MAX_BENDS = 16` records; a roll requires the plate
to carry no other bend. The record is removed by undoing its command; there is no removal
command in this slice. Every instance of a shared definition takes the same records, as a
cut does. A committed record is re-admitted at every evaluation.

* `Fold`: the line `a - b` in the blank plane, the inner radius `r`, the signed angle
  `a` in `(-pi, pi) \ {0}`. All folds of one plate are parallel to one side pair of a
  **rectangular** blank. A fold consumes the developed length `|a| x (r + t/2)` about its
  line; the bend regions must fit the blank and must not overlap.
* `Roll`: the axis `a - b` in the blank plane. A cylinder (`radius_b` absent) wraps a
  rectangular blank across its cross extent: the wrap angle follows from the development,
  `theta = L / (r + t/2)`, and stays below a full turn (the sealed body is a genus-0 shell).
  A cone (`radius_b` present) rolls a blank that is the **developed annular sector about
  `a`**: every boundary point sits on one of two arcs about the apex; the two authored mid
  radii must share one apex ratio with the sector's radial range
  (`r_a / s_in == r_b / s_out = sin(phi)`), the axial span must satisfy
  `|b - a|^2 = (s_out - s_in)^2 - (r_b - r_a)^2`, and the developed angular extent — the
  complement of the largest gap about the apex — gives the true wrap angle
  `theta = theta_dev / sin(phi)`.

Both forms refuse corner treatments, holes, cuts and a thickness side: the unfold is the
mid-surface, so nothing of the flat-plate evaluation applies.

## 3. The body

One G13 ruled loft of the derived cross-section and its translation along the fold-line or
roll-axis direction (`ruled::loft`); the seal's analytic prism self-check applies to the
exact translations (folds and cylinder rolls). The cross-section is derived in the engine:

* folds walk the developed mid-line isometrically — flats keep their developed lengths,
  each bend is one true arc of the mid radius `r + t/2` — and the closed counter-clockwise
  section offsets both sides by `t/2` (the centre-side arc keeps `r`, the far side `r + t`);
* rolls build the annular-sector section directly from the recognized radii.

Bend arcs are faceted to at most the 0.05 mm chord deviation (the round-hole rule of F06
A-2 D-2) and the kept deviation is reported as `arc_deviation_mm`. A cross-section whose
offsets pinch (strong opposing folds) is refused by the kernel's contour admission as
`INVALID_ARGUMENT`; a wrap of a full turn is refused before the loft. The quantity record
uses the `kernel-a10-bent-plate-volume/1` label; `width_mm` / `length_mm` are the body's
element-local bounding-box extents. Numbering is unchanged: the hashed local surface
triangles distinguish two bent plates with the same blank.

## 4. Storage

Seven parallel attributes, all or none, as for the corner arrays:
`steel:plate:bendOperationId` (uuid7), `bendKind` (`fold|roll`), `bendLineAMm`,
`bendLineBMm` (`double3[]`), `bendRadiusMm`, `bendRadiusBMm` (`double[]`), `bendAngleRad`
(`double[]`). A fold stores its radius twice and its angle; a roll stores a zero angle and,
for a cylinder, its one radius twice. The `Bends` change replaces the record list as a
whole; the bend attributes are shared-definition properties (P11) and are cleared by the
mesh invalidation path. The schema descriptor's digest changes with the slice, as it does
in every slice; pre-alpha stores are deliberately not migrated.

## 5. Acceptance

The kernel fixtures (`crates/kernel/tests/a10_bent_plate.rs`) compare the sealed quantities
with independent closed forms: the right-angle fold and the two-fold channel against
`t x developed area x width` and the perimeter form, the cylinder roll against the
annular-sector prism, and the cone roll against both the band closed form and the
development alone (machine precision between them). The engine tests replay the same
invariance through `evaluate_plate`, including the two-radii-one-volume check, and the
documented refusals. The CLI end-to-end tests run `bend-plate` against the real binary:
the recorded volume, the replay, the overlapping-fold refusal, the undo, and the copy.
