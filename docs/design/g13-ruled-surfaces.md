# G13, slice 1: ruled surface between two contours, with thickness

Date: 2026-10-27. Status: design note for the G13 kernel primitive (`beamcraft-kernel`,
module `ruled`). Task card: `docs/plan/tasks.md` G13 / M (milestone M1); issue #186.
Depends on G05 (`profile` admission, cap triangulation, arc segment choice) and G09
(`ops` sealing and certification). Foundation for A10 (bent plate) and A11 (lofted
plate). The card's remaining scope — cylinder and cone patches as first-class inputs
with authored bend radii, and the unfolded outline — stays with the issue's acceptance
gate; this slice delivers the ruled surface mechanism itself.

## 1. Input and result

`ruled::loft(&Loft) -> Result<LoftResult, LoftFailure>` builds one closed solid from
two `RuledContour`s with the same vertex count:

* a `RuledContour` is a counter-clockwise polygon in a certified orthonormal frame
  about an origin: every vertex is `origin + x * X + y * Y`, so planarity is exact by
  construction. Authored polygons take 2-D points; `RuledContour::arc` generates the
  polygon of a circular arc with vertices **on** the true curve and reports the G05
  sagitta bound `r * phi^2 / 8` of its facet angle (a full turn produces the closed
  circle without a duplicated point);
* the rulings join `start[i]` to `end[i]` by index. The correspondence must be the
  continuous one (each vertex pair neighbours the previous pair on both contours):
  a renumbered end contour twists the walls through the body, and the certification
  refuses the candidate instead of repairing it;
* the sheet thickness is the distance between the contours themselves — a bent plate
  states its outer and inner bend contours, a lofted plate its two face contours — so
  the loft takes no separate thickness input. A10/A11 author the two contours of the
  sheet's faces and call this primitive.

The result is a G09-certified `Body` (one closed outward genus-0 shell, volume
enclosure), the sealed BCMESH01 blob with its SHA-256, the G06 surface-area
enclosure, the larger sagitta bound of the two contours, and the fold lines. Refusals
are typed: `LoftFailure::{Contour, Profile, Body}`.

## 2. Cylinder and cone patches

A circular-arc contour translated along its axis is a cylinder patch, and two arcs of
different radii about the same axis are a cone patch; both are plain lofts of two
arc contours. The G05 segment choice is scale-linear, so two radii `r1`, `r2` get the
same facet count when the requested sagittas scale with the radii (`d2 = d1 * r2/r1`);
different counts are refused as `ContourMismatch`, not silently resampled.

## 3. Mesh

Two rings of `S` vertices; one wall quad per contour edge (a face with its area-vector
normal, id `ruled:w<i>`), then the two caps from the G05 triangulations (`ruled:f.cap0`
reversed against the start normal, `ruled:f.cap1` along the end normal). A skew
correspondence can make a wall quad non-convex in projection, where exactly one
diagonal is interior: the split is chosen by the sign of the orientation products
against the quad's own area vector (a crossed "butterfly" quad keeps the default
split; the certification refuses it either way). The F06 mesh caps (4096 vertices,
8192 triangles) are checked with integer arithmetic before any allocation.

## 4. Volume: the analytic self-check

For an exact translation of one contour (every ruling bit-identical to the same
vector `b`) the body is an oblique prism, and its exact volume is the start area
times the height along the normal, `V = A * |b . n|` (Cavalieri). The seal carries
this value in a slack enclosure as its volume self-check. General lofts have no input
independent closed form at this slice, so their expectation carries no analytic
volume; the certification itself (closed, outward, non-degenerate, self-intersection
free) plus the fixtures' independent closed forms carry the acceptance:

* translated contour (cylinder patch): `V = h * (theta/2) * (ro^2 - ri^2)` for an
  annular sector, every chord inside the arc, so the mesh volume sits below the true
  value with a deficit bounded by the sagitta bands over the two arc lengths;
* two arcs of different radii (cone patch): `V = pi * h/3 * (r1^2 + r1 r2 + r2^2)`
  for the full cone, bounded the same way.

Surface area is measured by the G06 exact moment sum (`LoftResult::area_mm2`); the
fixtures compare it with the closed forms of the caps, the cylindrical wall and the
rectangular end faces.

## 5. Fold lines

A fold line is a transverse wall edge at which the sheet changes direction: the
ruling at contour vertex `i` when either contour does not continue straight there
(`|cross|/(|a||b|) > 1e-9`, the same bound as a G12 mitre). The indices of both
contours are unioned, sorted and deduped. Faceted arcs contribute none: their turns
belong to the flagged approximation, not to an authored fold — a bent plate reads the
reported indices as its bend lines.

## 6. Determinism

Authored-polygon lofts use only IEEE `+ - * /` and `sqrt` (frame certification, area
vectors, fold measures): their bytes may be pinned across targets. Arc contours use
platform `sin`/`cos` for the facet vertices, exactly like the G12 arc sweep and the
member roll boundary; no cross-target byte identity is claimed for them.
