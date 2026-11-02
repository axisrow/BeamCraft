# G14.2 oblique member holes and curved walls

The kernel member-hole operation (G14.1, `crates/kernel/src/ops/member_hole.rs`) grows its
second slice: hole directions other than the section directions, and holes through faceted
curved walls such as a round tube. The F06 command layer (A04.2) keeps refusing oblique and
ranged cutters until a later slice wires the direction through; this slice is the kernel
capability and its certification.

## Cutter frame

`MemberHoleCutter::directed(cavity, axis_y, axis_z, from, to, ordinal)` sweeps the cavity
polygon along a section-plane direction `d = (0, y, z)` normalized, perpendicular to the
member axis X (an axial component stays a command-layer refusal; the kernel normalizes and
bounds `y`/`z`). `new` remains the G14.1 form `d = +Z` and keeps its byte-exact results.

The prism frame is `(e1, e2, d)` with `e1 = +X` and `e2 = d x e1`: a body point maps to
footprint coordinates `(a, b) = (p . e1, p . e2)` and a height `t = p . d`. The map is a
linear change of orthonormal basis, so the footprint coordinates of existing mesh vertices
carry at most one rounding each. The cavity polygon is authoritative in `(a, b)`.

## Facet strips (the kink registry)

Every wall triangle of the X-extrusion is exactly parallel to the member axis, so its plane
is a straight line in the YZ section plane. A **strip** is a maximal connected set of
wall triangles lying on one such line: strips are found by walking triangle adjacency
across shared edges whose YZ projections are non-degenerate, with plane identity decided by
the exact `area_sign` filter (this re-merges a face re-triangulated by an earlier hole).
A strip carries its plane `(n, c)` with `n_x = 0`, its boundary loops, its `(a, b)` region
(the affine image of its contour — convex), and its parent face id.

Vertical mesh edges (parallel to X) join two strips of different planes. The turn between
the strip planes, taken from their YZ normals, splits these joins: a **soft kink** turns
at most `pi/4` (the steepest turn CurveSpec can emit, 8 segments per full turn), a **hard
edge** turns more (a flange-web corner, a polygon vertex). All other strip boundary edges
(cap borders, drilled-hole walls, the section outline) are hard. A **chain** is a maximal
set of strips joined by soft kinks: one smooth authoring surface, a tube wall being the
example. Hard edges and chain borders are never crossed.

## Placement and crossing

As in G14.1 the footprint must clear the end caps and every earlier drilled wall, both by
the geometric floor against the swept prism box. Each chain then classifies like a G14.1
face: the strip-plane heights `t(a, b)` over the footprint are linear on each clip piece,
so the chain's height interval over the footprint is read off the piece vertices with
outward-rounded enclosures; a chain strictly inside the swept range with margin is
crossed, a chain clear of it is free space (with the G14.1 tangent and margin refusals in
between), and a chain straddling a range end is a blind hole refused as `CutterNotThrough`.

Crossed chains pair up in height order (entering material, leaving it); the first must
face the range start and the last must face its end, with the even/odd parity and
`CutterNotThrough` refusal of G14.1. Non-convex cavity polygons are admissible only while
every crossed surface is a single strip; a chain of several strips requires a convex
footprint (the per-strip clip relies on it), otherwise the cutter is refused unchanged.

The union-aware placement asks, per crossed chain, that the footprint be covered by the
union of its strip regions: the footprint is clipped against every region (convex against
convex), the exact clip areas must sum to the exact footprint area (Big integers), and
every clip piece must keep the floor distance from the hard edges of its region while
soft kinks may be touched. A footprint reaching past the union onto a hard edge, into a
hole of the same face or into free space is refused exactly as G14.1's edge-touching
footprint.

## Construction

Each crossed strip is re-triangulated with its clip piece as the opening: a piece fully
inside the region becomes one more hole loop, a piece meeting a soft kink is spliced into
the strip's outer loop through the shared edge vertices (computed once as a point of the
body's own soft edge, parameterized along it, so both strips see the identical vertex),
and a strip swallowed whole by the opening simply disappears. Crossed strips keep their
face ids.

The cutter's own wall is triangulated per crossed chain pair as a band between the entry
opening polyline and the exit opening polyline. Both polylines are ordered along the
footprint contour; the merge walk advances one event at a time (a footprint vertex, or an
entry or exit soft-kink crossing), interpolating the far side onto the same event
parameter, and emits two triangles per step. Local ids: `cut:f.h<ordinal>.w<k>.s<j>` with
`.p<run>` suffixes after the first planar run of a side, the G14.1 spelling kept for a
single run.

## Volume

The removed volume is certified from the footprint moments rather than assumed: on each
clip piece the removed column is `t_exit(a, b) - t_entry(a, b)` with both heights linear,
so the piece contributes `integral(t_out) - integral(t_in)` by the exact Greene moments of
its polygon, every step in outward-rounded enclosures. For one-strip planar walls this
brackets `A * thickness / |n . d|` (the parallel-wall identity of G14.1); for faceted
walls it is the certified enclosure the acceptance asks for. The seal compares the
candidate's mesh volume against the analytic enclosure and refuses a mismatch.

## Certification and refusals

Every G14.1 refusal keeps its code, defect and stage: blind, tangent, sub-floor margin,
edge-touching footprint, parallel-wall breach (now any `n . d = 0` strip whose segment
projection fails to clear the footprint), ordinal clash, end-cap clearance, and
`BodyWallUnsupported` for what still cannot be served. New tests cover the oblique flat
walls against `A * t / |n . d|`, a round tube hole against the enclosure, byte-exact
repeats of `new`, the hard-edge refusal on a flange-web corner and on a polygon-section
edge, a soft kink crossed freely, and never-crash inputs (degenerate directions, huge and
tiny ranges, non-convex footprints on multi-strip walls).

## Status after slice 1 (directed oblique cutters through flat walls)

Slice 1 is delivered: `MemberHoleCutter::directed(cavity, axis_y, axis_z, from, to,
ordinal)` sweeps the cavity along any section-plane direction `d = (0, y, z)` normalized,
with the prism frame `(e1, e2, d)`, `e1 = +X`, `e2 = d x e1`, exactly as specified above.
What landed, and where it deviates from this note:

- The crossed walls of this slice are the **axis-aligned flat faces** (a constant `z` or a
  constant `y`), crossed at any angle `n . d`; a sloped single-plane wall (the tapered
  section) stays refused with `BodyWallUnsupported` until the strip machinery of slices 2–3,
  as does every curved or faceted wall (a tube). The command layer still refuses oblique
  cutters; this slice is the kernel capability and its certification only.
- Classification is by the plane's **height interval** `[t_lo, t_hi] = [gamma * b + delta]`
  over the footprint's `b` extremes, with `(gamma, delta)` anchored at the crossed face's
  own extreme-`b` vertices: inside with margin is crossed, clear is free space, tangent and
  sub-floor keep the G14.1 refusals, and an interval straddling a range end is the blind
  `CutterNotThrough`. Crossed faces pair in interval-midpoint order; the G14.1
  even/odd and entry/exit parity checks are unchanged. Placement, the edge-touching refusal
  and the carved re-triangulation run in the projected `(a, b)` plane through the same
  loops machinery, and the parallel-wall breach is now any face whose `b` extent is
  degenerate (`n . d = 0`), refused through the same segment-clearance test.
- `new` remains `d = +Z` and is reproduced **bit for bit**: every frame expression is the
  identity on `(x, y, z)` for the unit axis, and the blob-level equality of `new` and
  `directed(_, 0, 1, _)` is certified by a test.
- The removed volume is certified from the **exact Big-rational divergence sum of the
  constructed opening polyhedron** (each pair's two rings fanned plus its band triangles,
  the entry fan reversed), widened by an explicit bound on the ring vertices' rounding
  residual against their wall planes — not from the Greene moments planned above. The
  certified real value is the same integral `integral(t_exit - t_entry)` over the footprint
  (for parallel wall pairs exactly `A * thickness / |n . d|`), and being anchored to the
  emitted vertices makes the enclosure honest by construction.
- The union-aware placement, the kink registry with soft kinks and chains, shared kink
  vertices, the merge-walk band construction and the tube certification are **not** in this
  slice; they arrive with slices 2–3.

One known limitation of this slice, to be lifted by the strip registry: an oblique cutter's
band walls are parallel to no coordinate axis and its ring vertices sit on their wall planes
only up to rounding, so a body already cut obliquely fails the exact X-extrusion contract of
`analyze_extrusion` and a later member hole on it is refused with `BodyNotPrism`
(certified by a test; never a crash). Perpendicular cuts after perpendicular cuts keep
working as in G14.1.

## Status after slice 2 (the strip registry, union-aware placement, shared kink vertices)

Slice 2 lands the kink registry and the union-aware placement for faceted walls, behind the
same `member_through_hole` entry point: a body whose walls group into strips of several
planes now routes to the chain path, while every axis-aligned flat wall keeps the slice-1
path bit for bit (the frame identity makes the registry a no-op there).

- **Registry.** Wall triangles group into strips by exact YZ collinearity (the `area_sign`
  filter re-merges a re-triangulated face); a strip carries its plane, contour and parent
  face id. Strips join by union-find across vertical mesh edges with the `pi/4` soft-kink
  turn; each chain is walked as a ring in adjacency order (not a transitive closure), so
  the keep/cut sides of a chain are read off ring neighbours.
- **Placement.** The footprint is clipped against every crossed strip region (Sutherland–
  Hodgman with the canonical crossing interpolation on the region's axis-parallel edges);
  clip pieces carry their source (footprint vertex, body edge or shared kink point), the
  exact Big-integer piece areas must sum to the exact footprint area, and the floor
  distance is enforced against hard edges while soft kinks may be touched.
- **Shared kink vertices.** A kink edge crossed by the opening gets one vertex
  parameterized along it, pushed into the registry and reused by both adjoining strips, so
  the carved walls of two chains meet on identical coordinates.
- **Construction.** Each crossed strip is carved to its clip piece through the generalized
  splice: keep paths and the opening arc are assembled into the strip's outer loop, the
  strip's area sign decides the winding, and a swallowed strip disappears with its face.
- **Band and volume.** Per crossed chain the band events are collected per footprint edge
  with their kink points, near-coincident events merge, and the band is emitted as the chord
  polyline between consecutive events; the removed volume is the same exact Big-rational
  divergence sum as slice 1 (rings fanned, band triangles, entry fan reversed) widened by
  the ring-rounding residual, and the seal compares the mesh against it.

Known limitations of this slice, all refusals or uncertified results rather than crashes:

- A wall face spanning several strips (the re-triangulated wall of an earlier cavity, the
  merge-walk case) is refused with `BodyWallUnsupported`; slice 3 lifts it.
- A footprint wider than one facet band cuts a band in two, and the band's chord polyline
  does not cover the body-edge arcs between neighbouring strips' kink points; the result
  stays uncertified (`ResultNotCertified`) pending the merge-walk band of slice 3 (the
  boundary question is with upstream).
- On a strip tilted against the cutter axis the clip piece currently maps the footprint
  into the strip frame without the `1/cos(tilt)` shadow scale, so the opening undercuts
  the true cutter curve by that factor while staying self-consistent (the seal-certified
  divergence sum anchors to the emitted vertices); the closed form `A * H` of the flat
  walls does not hold for a tilted wall either (the wall slope shortens the removed
  column). The fix or the accepted deviation is an open owner decision.
- Re-cutting a body after a faceted wall hole keeps the slice-1 limitation: the new band
  walls are not X-parallel, so a later hole on them is refused with `BodyNotPrism`.

## Status after slice 3 (the merge-walk band, single-strip chains)

Slice 3 delivers the merge walk of the construction section and takes over the sloped
flat walls, behind the same `member_through_hole` entry point.

- **Band events.** The crossing pass was dead in slice 2 (its sign filter skipped the
  genuine crossings), so every event came from a footprint vertex; a cutter whose
  footprint crosses a kink line mid-edge produced openings whose carved walls and band
  rings did not meet, and the seal refused the result. The walk now collects a crossing
  for every contour edge whose ends sit on strictly opposite sides of a crossed chain's
  kink line, with the canonical parameter clamped onto the edge when fp dust in the
  line's `b` (a section vertex a rounding step off the line) pushes it out of `[0, 1]`.
  Events of the same contour point merge their kink points by position alone — a
  clamped crossing lands on its vertex's event, whatever edge each was collected on —
  and the walk folds the seam, where the last event can be the first vertex's duplicate,
  into the head.
- **Shared kink vertices.** A merged event can carry the kink points of two chains whose
  lines sit a few ulps apart at the same `b` (a tube's near and far wall); the ring picks
  the first point on the chain's own planes, and the carved wall resolves its `Kink` clip
  vertices through the same chain's ring vertex for that contour position, so band and
  carve meet on one registered vertex.
- **Union-aware placement per side.** The exact area-cover check now runs per crossed
  side instead of per chain: the sides of a ring wall tile the footprint once each, so a
  chain covering it twice (near and far wall of one ring) classifies correctly. The
  underlying `big_area2` shoelace was telescoping (`p.x·p.y − q.x·q.y`, identically zero
  on any closed polygon) and is repaired to `p.x·q.y − q.x·p.y`; the cover bound is
  matched to the area's own scale (`2^(2e)`) and widened over the crossing vertices' ulp
  residual, so the check is meaningful rather than vacuous.
- **Construction.** A face with several crossed strips is carved strip by strip under the
  same face id (the per-strip splice already supported it); the slice-2 refusal is
  lifted. Single-strip chains — one sloped plane, the tapered-section wall — are served
  by the same chain path, which lifts the slice-1 `BodyWallUnsupported` refusal for
  sloped walls. A mix of crossed flat faces and crossed chains stays refused
  (`BodyWallUnsupported`), unchanged.
- **Volume.** The removed volume of a chain crossing is certified from the clip pieces'
  exact moments, not from the emitted band: a merge-walk ring crosses several strip
  planes per side and the ring-fan divergence sum of the flat walls straddles the folds,
  so it does not bracket the removed column. Each clip piece's height is linear, the
  piece contributes the exact origin-fan moment `∬ t dA` in the scaled Big integers
  (negated once for the cavity's clockwise winding), the pair's removed volume is the
  exit sides' sum minus the entry sides', and the enclosure is widened by the clip
  vertices' ulp residual. The flat walls keep the slice-1 divergence sum bit for bit.
- **Tests.** The wider-than-band tube hole (the parked test of the fixup) is unparked:
  it seals with genus 1, its removed volume matches the section's own closed form, its
  band ids carry the `.p<run>` suffixes past the mid-edge kink crossings, and it repeats
  byte for byte. New tests cover the sloped walls (a rotated square section: two
  single-strip chains, the closed `A · s / cos(tilt)` volume, byte-exact repeats) and
  the honest refusals: an oblique range that now genuinely ends inside a sloped wall is
  the blind `CutterNotThrough` instead of the blanket `BodyWallUnsupported`.

Known limitations still standing:

- A mix of crossed flat faces and crossed chains is refused with
  `BodyWallUnsupported` (the routing is conservative; lifting it is unassigned).
- On a strip tilted against the cutter axis the clip piece still maps the footprint
  into the strip frame without the `1/cos(tilt)` shadow scale (the opening undercuts
  the true cutter curve by that factor); the closed form `A * H` does not hold for a
  tilted wall. The fix or the accepted deviation remains an open owner decision.
- Re-cutting a body after a faceted wall hole keeps the slice-1 limitation: the new
  band walls are not X-parallel, so a later hole on them is refused with `BodyNotPrism`.
