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
