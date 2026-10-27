# G12: sweep along a path (polyline or arc, mitred corners)

Date: 2026-10-27. Status: design note for the G12 kernel primitive (`beamcraft-kernel`,
module `sweep`). Task card: `docs/plan/tasks.md` G12 / M (milestone M1); issue #185.
Depends on G05 (`profile` admission, triangulation, segment choice) and G09 (`ops::Body`
certification). Foundation for P12 (polybeam), P13 (curved beam) and A12 corner treatment.

## 1. Input and result

`sweep::sweep(&Sweep) -> Result<SweepResult, SweepFailure>` builds one closed solid from

* a `SweepPath`: a polyline of at least two element-local points, or an arc through three
  distinct points `start`, `on`, `end` (the arc runs from `start` to `end` passing through
  `on`; three points fix the plane, centre and direction);
* a section: an admitted `model::section::SectionShape` in a chosen
  `section::Representation` (the A07.1 `outline`, centred on its bounding-box centre);
* `chord_deviation_mm`: the stated bound on the distance between a curved path and its
  facet chords (unused by a pure polyline, which has no curve).

The result is a G09-certified `Body` (one closed outward shell, volume enclosure), the
sealed BCMESH01 blob with its SHA-256, the net polygon area, whether any curve was
approximated, the number of prism segments and, for an arc, the achieved chord-deviation
bound of the swept surface. Refusals are typed and carry no geometry:
`SweepFailure::{Section, Path, Profile, Body}` mirroring `SectionFailure`.

## 2. Mitre construction

The path is reduced to a chord polyline: the polyline itself, or the arc faceted as in
section 3. A sweep of `n` chords has `n + 1` rings. Every ring is a rigid image of the
section polygon (outer contour then cavities, in profile point order); rings are welded:
each interior ring is one shared row of vertices.

* **Frames.** The first segment gets the F06 default member frame: section `Z` along the
  chord, `Y` the projection of global `+Z` (the near-vertical `+Y` branch when the chord is
  within 1e-12 of vertical — the written `MemberFrame` rule, rebuilt here over a unit
  chord so no per-segment axis floor applies). `Frame3::try_new` certifies it.
* **Parallel segment frames.** Segment `j > 0` turns from chord `j-1` by the angle
  `theta_j` (`cos = dot`, `sin = |cross|` of the unit chords). Its frame is the previous
  segment's frame rotated by `theta_j` about `u = (d_{j-1} x d_j)/|..|` (Rodrigues, checked
  ops only). No roll is introduced anywhere else: the section stays parallel-transported
  in the sense of these half-turn mitres, which is the fabricator's polybeam behaviour.
* **Mitre rings.** The ring at an interior vertex is the **oblique section** of either
  adjacent prism by the mitre plane (the plane through the vertex, normal to the bisector
  `(d_{j-1} + d_j)/|..|`): each section point rides its fibre and is cut where the fibre
  meets the plane. Cutting the same plane from the previous segment's frame or from the
  next one reproduces the same points point for point (the half-turn rotation about the
  vertex fixes the bisector and preserves the cutting fractions), so one shared ring welds
  the two prisms and the boundary stays closed and 2-manifold. End rings are the same cut
  of the first/last parallel prism by the plane perpendicular to the true end tangent, so
  the end faces are perpendicular to the tangent, not to the first chord.
* **Degenerate turns.** A vertex whose unit chords have `|d_{j-1} x d_j| <= 1e-9` has no
  geometric mitre (collinear continuation, or a fold-back within the same bound of a
  180-degree turn): refused (`PathCode::DegenerateTurn`). Sharper turns between those
  bounds build long mitre wedges; G06 certification refuses any self-intersecting result,
  so no separate mitre limit is invented here.
* **Bad paths refused before any work:** non-finite or over-cap point coordinates
  (`COORDINATE_CAP_MM`), fewer than two polyline points, a chord shorter than the
  section's own `eps_geom` floor, total length above `DIMENSION_CAP_MM`, an arc whose
  three points are collinear (zero denominators) or whose radius exceeds
  `DIMENSION_CAP_MM`, a nonpositive/over-cap `chord_deviation_mm`, and counts that would
  exceed the F06 mesh caps (below).

## 3. Chord-deviation policy

Curves exist only as flagged polygon approximations. An arc is faceted into `n` equal
chords; the ring at facet joint `j` sits **on** the arc. The deviation governand is the
outermost fibre: the section centroid rides within one section radius `r_s` of the path,
so the farthest surface point runs at most at `r_max = R + 2 r_s` (the arc radius plus
twice the section radius), which the facet choice uses directly. G05's
`choose_segments(r_max, CurveSpec { max_sagitta_mm: chord_deviation_mm, .. })` returns
edges per full turn `N` with `r * (TAU/N)^2 / 8 <= deviation`; the per-facet
angle is `theta_f = theta / ceil(theta * N / TAU) <= TAU/N`, so every chord of the swept
surface stays within `r_max * theta_f^2 / 8 <= chord_deviation_mm` of the true surface.
The achieved bound is reported. If the bound is unreachable inside G05's segment cap, the
call is refused (`PathCode::DeviationUnreachable`) rather than silently coarsened.

## 4. Volume: the analytic references

Every prism segment is cut by two planes (mitre or perpendicular end planes). For a
prism of section area `A` along unit direction `d`, a cutting plane crosses the parallel
fibre through section point `(u, v)` at an affine function of `(u, v)`; the volume of the
truncated prism is therefore `A` times that affine function averaged over the section,
which is its value at the section centroid. Two corollaries carry the acceptance:

1. **Polyline.** If the section centroid lies on the path (every centred outline, and the
   fixtures below), each mitre plane crosses the centroid fibre exactly at the vertex, so
   the mesh volume is `A * L_path` **exactly** — mitre angles drop out.
2. **Arc (Pappus).** The true rotated sweep is a solid of revolution: `V = A * R̄ * theta`
   (second Pappus theorem over the sector angle). The mitred faceted mesh satisfies
   `V_mesh = A * L_chords(R̄)` with `L_chords = 2 n R̄ sin(theta / 2n)`, so

   `V_true - V_mesh = A * (R̄ * theta - 2 n R̄ sin(theta / 2n)) <= A * R̄ * theta^3 / (24 n^2)`,

   using `x - sin x <= x^3 / 6`. The fixtures compute both sides independently (closed
   forms, no kernel code) and require the certified mesh volume enclosure to meet the
   bound; for centred sections the mesh volume also sits below the Pappus value because
   every chord is shorter than its arc.

Cap triangulation reuses the G05 ear-clipping output, so hollow sections sweep with
watertight caps; the divergence-theorem volume of the certified body is the quantity of
record (`Body::volume_mm3`, an `Enclosure`).

## 5. Caps and bounded work

With `S` the profile point count, `m` the ring count and `T` the cap triangle count, the
mesh has `S * m` vertices and `2 S (m - 1) + 2 T` triangles. Both are checked against the
F06 caps (4096 vertices, 8192 triangles) with checked integer arithmetic before any
allocation; the binding constraint for fine deviations is the triangle cap, which limits
a 4-point rectangle to 1025 rings. Face ids are `sweep:w<edge>.<segment>` per wall quad
and `sweep:f.cap{0,1}`, all inside the 22-byte local-id grammar.

## 6. Determinism

Polyline sweeps use only IEEE `+ - * /` and `sqrt` (frames from chord dots/crosses,
Rodrigues half-angles): their bytes may be pinned across targets. Arc sweeps additionally
use platform `sin`, `cos` and `atan2` for facet positions and angles, exactly like the
member roll boundary; no cross-target byte identity is claimed for them.
