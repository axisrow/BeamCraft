# beamcraft-kernel

Contributor-original BeamCraft code and documentation, MPL-2.0.

G02 implements an allocation-free rectangular plate primitive in millimetres,
right-handed, Z up. It depends only on the existing `beamcraft-geom` math types
and `beamcraft-model` error codes and SHA-256. No new third-party dependency,
external asset, catalogue, network capability or unsafe code is introduced.

## API

`rectangular_plate(RectangularPlate { width_mm, length_mm, thickness_mm })`
returns a sealed `PlateResult` or typed `diagnostic::Failure`. The dimensions
describe an origin plate spanning `[0,width]`, `[0,length]`, `[0,thickness]`.
The raw dimension input is a primitive convenience API, not F04 authored input.
The caller must perform F04 contour admission before using this API for authored
geometry. No supplied contour is accepted, inferred, repaired or rotated here.

The result exposes eight G01 points, twelve canonical triangles, six outward
unit face normals, the ordered local face table, the exact 392-byte BCMESH01
blob and its full SHA-256. Normals travel separately: F06's blob has no normals
section. Subdivision is explicitly `none`, orientation `rightHanded`. Local
face, edge and vertex ids and edge endpoint mappings are in `feature_id`;
vertex roles follow `i=xb+2*yb+4*zb`. Engine qualification with ElementId is
outside this crate's G02 slice. Indices are snapshot-local, not stable addresses.

Volume uses `(width*length)*thickness`; cubic metres divide that result by 1e9.
It is returned only after mesh self-checks. This is analytic primitive volume,
not a complete F04 quantity record or a cache freshness claim.

Validation runs finiteness for all dimensions first, then one-sided submitted
upper caps, local D and geometric floor, then positivity and inclusive minima.
It returns all failures of the first failing stage in width/length/thickness
order (at most three, below F06's 32-record bound). Failure contains no geometry.
Nonpositive prospective width/length contribute zero to scale; no huge negative
input is squared. Dimensions above 1,000,000 mm are rejected before scale work.
For admitted dimensions D cannot exceed sqrt(2)*1,000,000, below the plate cap.
The geometric minimum is G01's `max(0.001,2^-22*D)`, never a test tolerance.

`rectangular_plate_with_cancel` accepts a host callback polled only at P0 entry,
P1 before build, P2 before checks and P3 before quantities. Cancellation discards
the candidate; no callback occurs after sealing. The host owns callback safety.

Construction follows the written F06 corner sum order without FMA. Positive,
orthogonal origin extents certify a non-self-intersecting convex box by
construction. Before sealing, bounded checks verify index validity, distinct
triangle indices, finite positive outward area, opposite paired edge incidence,
all vertices used, Euler characteristic and positive finite signed volume.
There are exactly 36 directed edge occurrences and at most 36*36 pairing
comparisons, with no recursion or allocation. All output zeros are positive.

## Verification and case coverage

The integration tests call the real public builder and read its actual buffers.
Their scalar cross/dot/volume calculations do not call production vector math.
Independent maps check edge incidence and connectivity; the frozen F06-POS-02
digest checks every buffer byte against an independently published vector.
An admitted dimension grid checks outward winding, connectedness, closedness,
Euler, analytic versus mesh volume and area, canonical zero and repeat bytes.
Volume/area comparisons use CR-TOL-001 relative 1e-6, without feeding that rule
into admission. Counts, statuses, ids, digest and minimum decisions are exact.

| F06 case | G02 obligation exercised |
|----------|--------------------------|
| POS-01 | Exact origin plate, counts, outward normals, closure, Euler and volume |
| POS-02 | Independent buffer reader, offsets, little endian and frozen SHA-256 vector |
| POS-03 | Analytic volume/unit-conversion projection; density, area records and bounds queries excluded |
| POS-14 | Plate inclusive absolute minimum and adjacent smaller float |
| POS-16, POS-17, POS-18 | Local role stability, uniqueness, grammar and qualification length; engine identity operations excluded |
| POS-37 | No cancellation poll after seal |
| POS-53, POS-54 | Dimension/scale projections of the large rectangle, above/at floor; authored origin and placement excluded |
| NEG-01 | Every dimension rejects NaN and both infinities before caps/ranges |
| NEG-03 | Every dimension below the absolute floor |
| NEG-16, NEG-17 | Cancellation at all four observing points, no partial result |
| NEG-18, NEG-20 | Internal corrupt-table injections fail self-check; imported meshes are covered by the G06 section below |
| NEG-45 | Large-rectangle thickness below scaled minimum and adjacent lower float |

Additional tests cover zero, signed zero, negative and huge finite negative
dimensions; exact upper caps and adjacent upper floats; same-stage aggregation
and first-stage precedence; edge endpoint/role consistency; malformed internal
indices and collapsed triangles; and all diagnostic mappings to existing F04 codes.

The remaining F06 cases require member frames/roll, authored contour validation
(including rectangle start/direction POS-48/49), placement, density and engine
quantity status, cache keys, queries, engine state or
transport. They cannot be fulfilled by a dimensions-only origin builder and
are not claimed as completed cases. The general F06 result/request adapter,
runtime feature lookup, complete quantities and shape
keys remain reserved at the F08 paths; the generic blob validator and mesh
checks are the G06 section below. Numerical certification of arbitrary
rounded/translated contours remains with G03/G04 and the F06 E-1 erratum.

## Integration and provenance

The kernel call is the headless API. The CLI command and engine/CLI end-to-end
gate are owned by P01, not G02. No worker protocol, USD, UI, hole, cut, contour
evaluation, member or quaternion is implemented. Member sin/cos reproducibility
remains OPEN and is not needed here. Native tests alone do not demonstrate
cross-target byte conformance. K0 and the complete geometry feature remain open.

Implementation and synthetic test inputs come from frozen F06 0.1.0 sections
2-6, 7 (encoding conventions), 10 and its case file. Mathematical provenance is
F06's public IEEE 754-2019 arithmetic reference, the divergence theorem for
signed triangle volume, and Euler's characteristic for a closed genus-zero shell.
These are original scalar implementations of public mathematical rules; no
external implementation, fixture, commercial software material or asset is copied.
Existing model SHA-256 is reused, without duplicating its implementation.

Registration changes are restricted to Cargo.lock (the root workspace glob
already discovers the crate), the package-specific MPL-2.0 exception in deny.toml
and the F08 table/creation note. The boundary checker and CI derive the portable
crate list from that table, so neither their code nor CI needs an edit. G01,
frozen F04/F06, ROADMAP.md, root Cargo.toml and other crates stay untouched.

## G05: profiles, extrusion and triangulation (`profile` module)

G05 is the base for BeamCraft's own round and oval holes: a planar profile (one outer
contour plus cavities) is admitted, its caps are triangulated and it is extruded straight
into a sealed closed mesh. It adds no dependency (no triangulation or geometry crate), no
asset and no I/O, and stays pure and WASM-compatible. The existing G02 plate API is untouched.

### API (engine API; CLI is owned by P01)

`profile::extrude(outer, cavities, thickness_mm)` and `extrude_with_cancel` return a sealed
`Extrusion` (mesh, volume, status, tolerance) or a `ProfileFailure`. `Profile::admit`
validates a profile alone, `Profile::triangulate` returns the cap triangles, and
`extrude_profile` extrudes an admitted profile. `Cavity::polygon`, `Cavity::circle`,
`Cavity::slot` and `Cavity::ellipse` build cavities. As in G02, the kernel call is the
headless API: the CLI command and the engine/CLI end-to-end gate are owned by P01, not G05.

Frame: millimetres, right-handed, the profile plane is XY and the solid runs from `z = 0` to
`z = thickness` (F04 P-1: the contour is the bottom face). The 3-D contour, placement,
rotation and ElementId qualification belong to the engine/P01 and A02. The outer contour must
be counter-clockwise and every cavity clockwise (material on the left of each directed edge);
F06 section 2 rejects wrong winding and never reorders points, and this module extends that
rule to cavities. Z offsets of the contour (the z origin of the G03 large fixtures) are not
modelled.

### Admission (F06 stage order, all failures of the first failing stage, at most 32)

Steps 4 to 6 are one F06 stage (ranges and contour validity): their failures are reported together in
input order (thickness; each contour's edges, area/winding and simplicity; contour pairs; containment).
A check is skipped only when its prerequisite failed (simplicity needs usable edges and a non-zero
area; a pair check needs both contours simple; containment needs no crossing between the pair), and the
nested-island support decision is made only when nothing invalid was found.

1. Counts, before any traversal: at least 3 points per contour, at most 1024 points per
   contour, at most 256 cavities and 2048 points in total (twice that is the F06 vertex cap).
2. Finiteness of every coordinate and the thickness.
3. Caps: coordinates at most 1e9 mm, thickness at most 1e6 mm, every contour edge (closing and cavity edges included) at most 1e6 mm, local size D (G01
   `Tolerance::for_plate`, from the first outer point) at most 2e6 mm.
4. Ranges: thickness `> 0` and `>= eps_geom`; every edge (including repeated points and the
   closing edge) at least `eps_geom`; exact non-zero area; exact winding; area at least
   `eps_geom^2`.
5. Simplicity, per contour: exact segment relations for all non-adjacent edge pairs (touch,
   overlap and crossing are all rejected), no adjacent backtracking, and no two non-adjacent
   edges closer than `eps_geom`.
6. Relations: no touching or crossing between contours and no two contours closer than
   `eps_geom` (inclusive minimum: exactly `eps_geom` is accepted), every cavity inside the outer
   contour (else invalid), no cavity inside another cavity (a valid island, reported
   `GEO_UNSUPPORTED_CONTOUR`, after every invalid finding).

All signs (orientation, winding, intersection, containment) are exact decisions about the
rational values of the submitted binary64 numbers: a rounding-error filter decides clear cases,
a fixed-capacity integer evaluation decides the rest, and nothing is ever decided by an epsilon.
Distances are written binary64 evaluations from local coordinate differences (so a lossless translation
near the 1e9 cap cannot move a clearance across the floor), used only for the floor comparisons and never change
a topology decision (F06 section 4). Public vertices and the blob both carry positive zero. Diagnostics use F06 `GEO_` codes with the F04 mapping, a
fixed schema field, the F06 stage and a fixed defect class, never user text or values.

### Curve cavities (explicit approximation, no contract change)

F06 0.1.0 has no curved geometry. `Cavity::circle`, `slot` (obround) and `ellipse` therefore
produce clockwise **polygons with vertices on the curve**, flagged by `CurveApproximation`
(kind, edges per full turn, requested sagitta, proven bound). The achieved bound is the
linear-interpolation error of a C2 curve, `h^2/8 * max|curve''|` with `h = 2*pi/n`, i.e.
`R*(2*pi/n)^2/8` with `R` the circle radius, the slot radius or the larger ellipse semi-axis. It excludes
the few-ulp rounding of vertex coordinates. The edge count `n` is the smallest multiple of four
in `[min_segments, max_segments]` (rounded up/down to multiples of four, 8 to 1020) that meets the
requested sagitta; if none does the call fails (`GEO_ABOVE_LIMIT`, defect `SagittaUnreachable`)
and nothing is coarsened silently. Sine and cosine use only `+ - * /`, an exact integer octant
reduction and a fixed Taylor series, so the points are bit-identical on every target and exactly
symmetric. Orientation of slots and ellipses is given by an axis vector, never an angle.
A **contract amendment is required** before the engine may treat these as exact curved features:
F06 needs a curve/face kind, the `subdivisionScheme` meaning of an approximated wall, and quantity
status rules (this module reports `VolumeStatus::Approximated` with a bound, where F06 section 8
would say `unsupported` for a curved profile).

### Triangulation, extrusion and seal

Cap triangulation joins each cavity to the outer polygon by one exact-visibility bridge (cavities
in descending right-most-x order, candidates by squared distance then position), then clips strictly
convex ears whose triangle holds no other vertex, boundary included, except copies of its own
corners. The step is not trusted: before sealing, the extrusion self-check proves a tiling (every
cap triangle exactly positive and every directed edge paired with its opposite exactly once, so
by the winding-number argument there is no overlap, gap or T-junction), index validity, all
vertices used, one connected component, Euler characteristic `2 - 2*cavities` (one genus per
cavity), a positive signed volume and agreement of the mesh volume with the analytic volume.
Work is bounded by caps (contours, cavities, F06 section 6 mesh caps checked before any
allocation) and a fixed budget of 150,000,000 exact-sign evaluations; exhausting it is a typed
failure, never a hang. No recursion, `unsafe` or panicking path is used.

Output: the F06 section 6 BCMESH01 blob (layout 1, element-local, negative zero normalised),
its SHA-256 digest, the vertex/triangle/face tables, per-face outward unit normals (`None` for the
curved wall of an approximated cavity) and `subdivisionScheme` `none`, orientation `rightHanded`.
Vertices: bottom points in contour order (outer, then each cavity in input order), then top points.
Faces: bottom cap, top cap, one wall per outer edge, then cavity walls; triangles are grouped by
face. A polygonal cavity has one planar wall per edge; an approximated cavity has a single wall
face, so a future hole feature finds all its walls through `ExtrusionMesh::cavity_walls`.
The volume is `net_area * thickness` (shoelace relative to each contour's first point; cubic metres divide
by 1e9). `VolumeStatus::Exact` holds when every contour is authored; otherwise `Approximated`
with `volume_error_bound_mm3`. The mesh/analytic agreement is a build certificate with its own
relative bound (1e-9 of the magnitude of the cancelling terms), not CR-TOL-001: F06 section 4
rule 7 forbids the kernel from reading the test comparison rule.

### Local feature ids (provisional)

F06 section 5 defines `fid1` roles only for the plate and member boxes and forbids inventing box
roles for other solids. G05 therefore uses the local kind `extrusion` with the same character
set and length bound: faces `f.z0`, `f.z1`, `f.o<k>`, `f.c<j>.s<k>` or `f.c<j>`; edges
`e.z0.<t>`, `e.z1.<t>`, `e.v.<t>`; vertices `v.z0.<t>`, `v.z1.<t>` with contour token
`<t>` = `o<k>` or `c<j>.<k>`. Indices follow the authored contour and cavity order, never mesh
order; a cyclic start change moves roles (F06 section 5 rule 9). A contract amendment must adopt
this grammar before the engine qualifies and persists these ids.

### Verification and case coverage

`tests/g05_profile_extrusion.rs` and `tests/g05_negative_and_fuzz.rs` call the public API and
read the actual buffers with `tests/common` (independent blob reader, edge-incidence and
connectivity maps, divergence volume, shoelace area, closed-form regular-polygon, obround and
ellipse areas; no production math). Covered: rectangle (equal to the G02 plate), one and many
round holes, slot and ellipse holes at several axes, L-shape and comb outlines with holes, flat
(collinear) vertices with aligned holes, cavities and thickness at the exact minimum distance and
dimension (dyadic floor `eps_geom = 0.25`) and one float below, the 1024-face cap exactly and one above, the triangle cap (8184 accepted, 8220 rejected), every contour defect class, non-finite values in every position, cap/limit boundaries,
cancellation at all four points with no poll after sealing, mapping of all `GEO_` codes, exact
translation and quarter-turn invariance, cyclic start changes, reversed winding rejected, byte
identity of repeated builds, a seeded valid-profile generator (120 cases, always accepted, then moved
exactly), seeded hostile contours (1500) and perturbations (200) and maximum-size inputs. Unit tests
check the exact predicates against an integer reference and corrupt build candidates (inward,
missing, duplicated, out-of-range, collapsed, displaced) that never seal.

G03 fixtures: `g03.plate.through-hole`, `g03.plate.cavity`, `g03.plate.thin-wall` and
`g03.plate.nonrectangular` (placeholders for 23 x 47 x 3 plates, recorded `unsupported`) get real
G05 solids (round hole, rectangular void, wall at the minimum thickness, L outline); their G03
expectation rows are unchanged `not_run`. `g03.plate.large-positive`, `large-negative` and
`large-mixed` (status `unverified`) run on the x/y translation: with the origin at +1e9 a positive
extent leaves the 1e9 coordinate cap and is rejected `GEO_ABOVE_LIMIT`, and the inward placement is
accepted; `large-minimum-cube` is decided by the submitted binary64 edge, which is below the floor
when the 0.001 mm decimal is added to 1e9. The z origin is not modelled, so those four stay
`unverified` for z. Other G03 ids are G02 dimension cases and are not run here.

F06 cases projected: NEG-37 (winding), NEG-40 (self-intersection), NEG-41/42 (point count, repeated
point), NEG-46 (D cap), NEG-01 (non-finite), POS-53/54 (scaled floor, at and above, for thickness),
NEG-16/17 (cancellation), NEG-18/20 (corrupted candidates), POS-01 (closed shell, Euler, volume,
extended to genus `n`), POS-27/NEG-12/13 (builder-side caps; no generic blob validator), POS-48/49
(cyclic start and quarter turn, projected to polygons). Not run: NEG-38 (no 3-D planarity: the
profile is planar by construction), NEG-39 (the valid-pentagon-unsupported decision is the F06 plate
request path, which G05 does not change; wiring it is A02), NEG-19 (no certifier for arbitrary
meshes), blob-validator cases, quantities beyond volume, keys, queries, density and transport.

### Not implemented (design section 8 G05 scope, owner row wins)

The kernel design's G05 allocation lists member prisms and frames with roll; those are not part of
the owner's G05 row (profile with cavity and straight sweep) and are not implemented here. Also
not implemented: rotation/placement and the 3-D contour adapter, curve-exact geometry, nested
islands, non-planar contours, bevelled or non-straight sweeps, hole features (A03/A04/G09), area and
bounds quantities, shape keys, generic mesh certification and spatial indexes (G06), cross-target
byte evidence (no `wasm32` target or `cargo deny` is installed in this environment, so neither was
run), and CLI wiring (P01).

### Provenance

Original scalar code from public mathematics only: shoelace area; divergence-theorem volume; Euler
characteristic of a closed surface; winding-number and tiling argument; exact sign of the rational
orientation determinant (with a standard rounding-error filter derived here); ear clipping with hole
bridging as publicly described by D. Eberly ("Triangulation by Ear Clipping", method only, no code
read); Taylor series for sine and cosine; the linear-interpolation error bound; F06 0.1.0 sections 2-6,
10; F04 section 3 P-1. No third-party code, crate, fixture or asset, and no Tekla/Trimble material.

## G06: topology, orientation and measurements

G06 adds a verification and measurement foundation that works on **any** BCMESH01
blob or vertex/index buffer of finite input, not only on G02 boxes. It is original
scalar code from public mathematics, adds no dependency, and keeps every G02 file,
test and result unchanged. Plan row (owner): own checks of shells, orientation,
volume and a bounded spatial index; an open or unsupported mesh never gets an exact
status; independent invariants and transforms; headless API. (G04 design section 8
lists a different, unconfirmed G06 allocation, "issue pilot-cut requirements"; the
owner's row wins and pilot cuts belong to G09. Its G05-style items "analytic
measurements, mass/status handling and independent mesh checks" are covered in part
here: status/mass rules, an independent mesh cross-check of sealed plates and exact
mesh measurements; contours, extrusion and triangulation remain G05.)

| Module | Content |
|--------|---------|
| `trimesh` | `TriMesh`: F06 section 6/10 structural gate for `BCMESH01` blobs (`from_blob`, `to_blob`) and buffers (`from_buffers`), caps (4096 vertices, 8192 triangles, 1024 face-table entries, 16 shells, 213,024 bytes, coordinates at most 4e9 mm), rigid `transformed`, local scale `tolerance` |
| `topology` | `analyze` / `analyze_with` / `analyze_with_tolerance`: sealed `Analysis` with `ShellStatus`, `Defects`, per-shell `ShellReport` (Euler characteristic, genus, orientation, signed-volume enclosure); `classify_point` for certified meshes; `Limits` (explicit caps) |
| `measure` | `measure`: volume, area, centroid enclosures and exact vertex bounds, gated by the analysis; F06 section 8 `QuantityStatus`, `mass_status`, `analytic_mass_kg`; `cross_check_plate` (independent re-read of a sealed G02 plate) |
| `spatial` | `Aabb` and `BoxIndex`: static median-split box tree (leaf size 4, at most 8192 items and 16,384 nodes, depth under 16, no recursion), overlap queries and overlapping-pair enumeration with explicit pair/result caps |
| `verdict`, `enclosure` | `MeshCode` (stable `GEO_` tokens with F04 mapping), `Fault`, `MeshError`, `ShellStatus`, `Uncertainty`; outward-rounded `Enclosure` |

`bigint`, `exact` and `intersect` are crate-private: fixed-capacity (72 x u64, no heap,
no recursion) integers, exact `orient2d`/`orient3d` and closed-set segment/triangle
contact.

**Method.** (1) Structural gate, then undirected-edge incidence by sorting directed
edges (`O(T log T)`, no welding, no healing): boundary, three-or-more-triangle,
same-direction and duplicate-triangle detection, vertex fans by corner union-find,
shells by shared vertices, Euler characteristic `V - E + F` and genus `(2 - chi)/2`
per closed shell. (2) Degeneracy under the F06 section 4 policy: a repeated index, an
edge strictly shorter than `eps_geom`, exactly collinear corners or an unused vertex.
(3) Exact signed volume per shell: every binary64 coordinate is an integer multiple
of a power of two, so `sum(v0 . (v1 x v2))` is computed exactly in integers and
enclosed once; its sign is the shell orientation (G04 section 2: exact sums, no
epsilon sign). (4) Self-intersection by two closed triangles meeting exactly when an
edge of one meets the other, decided from exact orientation signs (a floating-point
filter whose error bound is derived in `exact`, with exact integer fallback), over
candidate pairs from the box index; pairs sharing an edge or one vertex are classified
by that shared feature, so touching, crossing, folding and coincident faces are all
certified defects, and a one-binary-step gap is certified disjoint. (5) A shell inside
another is detected by exact crossing parity of axis rays from up to eight vertices
(six directions each); a grazing probe is skipped, never guessed.

**Verdict and precedence.** `Certified` needs every proof. Otherwise, in order:
`Degenerate`, `NonManifold`, `Open`, `InconsistentOrientation`, `Inward` (negative
shell volume, including a cavity shell), `Degenerate` (exactly zero volume),
`Overlapping`, `Uncertain`. `Uncertain` carries its reason (pair cap, index cap,
exact-predicate budget, numeric range, containment) and is never exact. Codes:
open/non-manifold/misoriented -> `GEO_MESH_NOT_CLOSED`, inward -> `GEO_MESH_INWARD`,
degenerate/overlapping/uncertain -> `GEO_MESH_NOT_CERTIFIED` (numeric range ->
`GEO_NUMERIC_UNCERTAIN`), all `UNSUPPORTED_GEOMETRY`; structural failures are
`GEO_NON_FINITE`, `GEO_ABOVE_LIMIT`, `GEO_BUFFER_LIMIT`, `GEO_TOPOLOGY_LIMIT`,
`GEO_BLOB_INVALID`, `GEO_CANCELLED`. These live in `verdict`; `diagnostic::Code` (G02)
is intentionally not edited and can absorb them later.

**Measurements and statuses.** Only a `Certified` analysis yields volume, area and
centroid, as outward-rounded enclosures of the polyhedron the given binary64
vertices define (area and centroid use exact cross/moment integers, one rounding
enclosure per triangle). They are mesh-computed values, never engineering
quantities: `MeshMeasurement::quantity_status()` is always `unsupported` (F06
section 8: a mesh from another source gets no engineering quantity), its mass is
`unsupported` even with an absent density (F06-NEG-18), and bounds are exact vertex
extents, not a volume. `mass_status` implements the section 8 precedence (unsupported,
then unknown for absent density, then stale, then current); `analytic_mass_kg` keeps
`fl(fl(V*density)/1e9)` and refuses an underflowed zero. Transforms: `TriMesh::transformed`
applies the production `RigidTransform` per vertex (approximate frame certificate, so
exact rigidity is not claimed); exact motions (dyadic translation, quarter turns,
axis permutations) preserve volume bit for bit in tests, power-of-two and integer
scale factors scale V and S as `k^3` and `k^2`, and a mirrored mesh is `Inward`.

**Tolerance.** `TriMesh::tolerance` derives the F06 `eps_geom = max(0.001, 2^-22 L)`
with `L` the largest distance from vertex 0 (G01 norm, capped at the 2,000,000 mm
plate cap) because a bare mesh has no body inputs. A caller that knows the body
passes its own `Tolerance` to `analyze_with_tolerance`; `cross_check_plate` does.
Known consequence, covered by a test: for a plate at exactly its inclusive minimum
thickness the plate rule (thickness excluded from `D`) is sound while the generic rule
is slightly larger and reports `Degenerate`.

**Work bounds and cancellation.** No recursion; allocations are sized by validated
counts only. `Limits::DEFAULT`: 2^20 candidate pairs, 16,384 index nodes, 2^18 exact
fallbacks; exceeding any gives `Uncertain`. Polls (stage that would run next):
`Validation` at entry, `Build` before incidence, `Checks` before degeneracy and
orientation, then for a clean closed outward mesh a `Checks` poll before the index,
one per index build and about every 1024 units of pair work, one per nested-shell
probe pair, and a final `Quantities` poll before sealing. `measure_with_cancel`
polls `Validation`, then `Quantities` before the pass and every 512 triangles. A
cancelled call returns only `MeshError` (no partial result); nothing polls after
sealing.

**Tests** (all original; oracles are voxel counting, closed forms, `std` maps and
integer/f64 scalar code, never production math): `tests/g06_shells.rs` (28: box,
prism, tunnel genus 1 and 2 by Euler, disjoint shells, 16/17 shell cap, shuffled and
relabelled copies, repeat identity, 120 seeded voxel sets against an independent
verdict, open, duplicate, three-face edge, edge/vertex contact, flipped triangle,
inverted and cavity shells, collapsed/collinear/short/unused, overlap, touching,
nested, budgets, large slab, point classification, blob and buffer validation at
and one above every cap, cancellation at every poll), `tests/g06_measure.rs` (16:
closed-form volume/area/centroid, refusal without values, mass rules, exact and
inexact rigid motions, scale, far-from-origin cancellation, measurement
cancellation, G02 cross-check), `tests/g06_fuzz.rs` (5 seeded hostile-input suites:
mutated blobs, random soups, surgery on valid shells, perturbed vertices, maximum
size soups; no panic, deterministic output, and every `Certified` verdict checked by
the independent oracle), `tests/g06_fixtures.rs` and 22 unit tests of the
integer, interval, predicate, contact and index code.

| Case or fixture | G06 obligation exercised |
|-----------------|--------------------------|
| F06-POS-27 | Blob exactly at every cap is structurally valid and round-trips |
| F06-NEG-11 to 14 | Counts above caps and 32-bit-overflowing counts rejected before any size |
| F06-NEG-15, 30 | Length disagreement and empty blob are `GEO_BLOB_INVALID` |
| F06-NEG-18 | Open mesh: three boundary edges, Euler 1, no value, mass unsupported |
| F06-NEG-19 | Overlapping closed boxes: Euler 4, `Overlapping`, the sum 2000 never reported |
| F06-NEG-20 | Fully inverted box: signed volume -5e4 enclosed, `Inward` |
| G03 `g03.plate.*` with expectation `ok` (22) | Sealed G02 blob re-read, certified under the plate tolerance, counts/Euler/bounds exact, exact rational volume inside the mesh enclosure |
| G03 invalid and limit rows (26) | Not run: the real builder rejects them, so no mesh exists |
| G03 `large-positive`, `large-negative`, `large-mixed`, `large-minimum-cube` (4, `unverified`, no expectations) | Not run: origin placement is outside the dimension-only builder and no reference exists |
| G03 `through-hole`, `cavity`, `thin-wall`, `nonrectangular` | Not run: need G05 contours and G09 cuts; not faked. A hand-made hollow body is refused as `Inward` by `tests/g06_shells.rs`, which is not a fixture claim |

**Not implemented, stated plainly.** Ray first-hit and ray-based picking (point
classification of a certified mesh exists; F06 pick queries stay analytic and belong
to later tasks); healing, welding or repair; cavities and holes as valid solids
(negative shells are refused); exact area/centroid (enclosures only); engineering
quantities for non-analytic meshes (by contract); member and contour meshes (G05);
cross-target byte conformance, which native tests do not show; the `cargo deny` and
`wasm32-unknown-unknown` checks, which were not available in the authoring
environment. The crate uses only `std` plus `geom`/`model`, so no new licence or
WASM-incompatible dependency is introduced, but the WASM claim awaits CI.

**Headless and provenance.** The kernel calls are the engine API; the CLI command and
the engine/CLI end-to-end gate stay owned by P01, as for G02. References are public
mathematics: the divergence theorem (signed tetrahedron volume), Euler-Poincare for
closed orientable surfaces, standard orientation-determinant predicates and the
two-closed-triangles-meet-iff-an-edge-meets-the-other fact, IEEE 754 rounding
(correctly rounded operations bound each widening by one representable step), and
F06 sections 4, 6, 8 and 10 with G04 sections 2-4. No third-party geometry crate,
source, test or data, and no commercial-product material, was used.

## G09: pilot plane cut and through hole (`ops` module)

G09 adds two **bounded** operations over solids that the G06 certification already accepted: a
plane cut that keeps one half-space and caps the section, and a through hole that subtracts a
straight prism from a Z-prism body. It is a pilot for A03 (plane cut) and A04 (hole), **not a
general boolean kernel**: general robust CSG (curved or oblique cutters, several outer section
loops, boundary-crossing or oblique holes, coplanar or tangent contact, non-manifold results)
remains separate work, and every such request is refused with a typed failure and no partial
result. It adds no dependency (no boolean, clipping or triangulation crate), no asset and no I/O
and stays pure and WASM-compatible; the exact predicates, enclosures and G05 triangulation are
reused, not rewritten. The kernel call is the headless API; the CLI commands and the engine/CLI
end-to-end gate are owned by P01, A03 and A04 (as for G02, G05 and G06), and this task adds no
engine or CLI command.

Owner row (wins): G09 / M, pilot of cuts and boolean operations, depends on G05 and G06; own
bounded operations over agreed solids; independent fixtures verify volume, bounds, topology and
failure; not a port of foreign functions. The kernel design section 8 lists G09 as "numeric
robustness, query/E2E validation and determinism evidence". Here: numeric robustness (exact side
predicate, bounded budgets, hostile fuzz) and determinism evidence (repeat identity and
input-order invariance of the bytes, native only) are done; **query/E2E validation is not**
(queries are P01; no mock result is claimed) and cross-target byte conformance is not shown
(`wasm32-unknown-unknown` and `cargo deny` are not installed in the authoring environment, so
neither was run).

### API

| Item | Purpose |
|------|---------|
| `Body` | Immutable certified solid: BCMESH01 mesh, local face ids, outward unit normals (`None` for a curved face), the body's own `Tolerance`, its G06 `Analysis` and volume enclosure. `Body::from_plate`, `from_extrusion`, `from_mesh` admit a G02 plate, a G05 extrusion or any mesh with a face table; only a certified, element-local single shell is admitted: a world-space mesh is refused (ody.space) because operations seal local blobs, and every supplied planar normal must agree with its face's area vector (cosine at least 0.999999, ace_table.entry) |
| `plane_cut`, `plane_cut_with` | Cut by `CutPlane { point, normal }`, `Keep::{Positive, Negative}`, an ordinal naming the cap, `OpLimits` and a cancellation callback |
| `through_hole`, `through_hole_with` | Subtract `HoleCutter { cavity, z_from_mm, z_to_mm, ordinal }` (a G05 `Cavity`: circle, slot, ellipse or polygon, clockwise) |
| `OpResult` | Sealed result: `body()` (feed it to the next operation), `blob()` and `mesh_digest()` (SHA-256), face table and normals, `removed_faces()`, volume enclosure (mm3 and m3), analytic volume (holes), bounds, Euler characteristic, genus, `plane_deviation_mm()` (cuts), `OpStats` |
| `OpFailure` | `OpCode` (F06 `GEO_` tokens plus two proposed), `OpField`, `OpDefect` (fixed tokens), stage, and the first G05 admission diagnostic when a cap was refused; never geometry or user text |
| `OpLimits` | `max_triangles` 8192, `max_loops` 64, `max_cap_points` 2048, `max_exact_predicates` 2^18 and the G06 `Limits` of the certification |

### Method

* **Side of the plane.** For every vertex `normal . (v - point)` has an exact sign: a binary64
  evaluation decides it when its error bound (`8u` times the permanent) allows, otherwise
  fixed-capacity integers over the scaled binary64 values (the G06 `bigint` and `scaled`) decide
  it, charged against `max_exact_predicates`. Nothing is decided by an epsilon; `0` is an exact
  zero. A scaled normal is the same plane.
* **Clip.** Every triangle is clipped against the closed half-space (one-plane
  Sutherland-Hodgman): vertices on the plane are used as they are, a strictly crossing edge gets
  one new vertex whose every coordinate is the correctly rounded (nearest, ties to even) value of
  the exact rational intersection: a seed from enclosures is moved by exact integer comparisons with
  the neighbouring midpoints until it is proved nearest, so it depends neither on numbering nor on
  rounded interpolation and never leaves the edge;
  quads are split along the diagonal at the smallest vertex.
* **Cap.** The boundary of the clipped surface is the section. Its loops are chained, rotated to
  their smallest vertex, split into the single outer loop (exact positive area) and holes, admitted
  and ear-clipped by G05 in a frame fixed by the plane alone (dominant normal axis), so both halves
  of a plane and the spellings `(n, d)` and `(-n, -d)` share one triangulation and the two halves'
  volumes add up exactly. The cap is planar **only up to the rounding of the new vertices**; the
  largest distance of a new vertex from the plane is certified (exact value, enclosed) and
  returned as `plane_deviation_mm`.
* **Hole.** The body must be a straight prism: its extreme heights `z0 < z1`, one outward horizontal
  cap face at each and every other triangle exactly vertical (exact zero projected area; extra
  vertices at intermediate heights on walls are fine). The caps are re-triangulated with the cavity
  added to their boundary loops (existing loops are kept vertex for vertex, so the walls stay
  valid), the cutter's walls are added over `[z0, z1]`, and the cavity must lie strictly inside the
  outline and clear of other holes by the G05 admission (exact signs, `eps_geom` floors,
  inclusive minimum). The cutter must reach past both caps by at least `eps_geom` of the body.
* **Seal.** Every candidate is canonicalised (unused vertices and faces dropped, vertices sorted by
  coordinates, triangles rotated and sorted by face), encoded, parsed back by the G06 blob gate and
  certified by the G06 analysis (closed, 2-manifold, outward, non-degenerate, free of
  self-intersection, one shell). Holes also require Euler characteristic exactly 2 less (genus +1)
  and a mesh volume that overlaps the analytic enclosure `V_body - |area| * thickness` (both exact
  integers, enclosed once). Anything else is refused; there is no healing.

### Refusals (all typed, none partial)

| Situation | Failure |
|-----------|---------|
| Body not a certified single shell | `GEO_MESH_*` code of the verdict; several shells `GEO_OP_UNSUPPORTED` / `body.shells` |
| Plane with NaN/inf, component above 4e9, zero normal, normal below 1e-100 | `GEO_NON_FINITE`, `GEO_ABOVE_LIMIT`, `GEO_NOT_POSITIVE`, `GEO_NUMERIC_UNCERTAIN` (`numeric.range`) |
| Whole body strictly on the kept / removed side | `GEO_OP_NO_EFFECT` `plane.outside` / `plane.empty` |
| Plane touches the body only (vertex, edge, face) | `GEO_OP_UNSUPPORTED` `plane.tangent` |
| A face of the body in the plane while the plane crosses | `GEO_OP_UNSUPPORTED` `plane.coplanar_face` |
| Section with several outer loops, a self-touching section, or a kept body pinched at an edge | `section.multiple`, `section.pinched` or `result.overlapping` |
| Section or cap not admissible (edge below `eps_geom`, island in a hole, ...) | `section.not_admissible` / the G05 diagnostic |
| Hole body not a Z-prism, caps not one face each | `body.not_prism`, `body.cap_faces`, `body.loops` |
| Hole touching or crossing the outline or another hole, wrong winding, self-intersecting | `GEO_CONTOUR_INVALID` with the G05 diagnostic (`cavity` = number of existing holes) |
| Hole inside another hole, or enclosing one | `GEO_UNSUPPORTED_CONTOUR` |
| Cutter not through, flush with a face (tangent), margin below the floor, empty range | `cutter.not_through`, `cutter.tangent`, `cutter.margin`, `cutter.range` |
| Triangle, loop, cap-point or exact-predicate budget exceeded | `GEO_ABOVE_LIMIT` (`budget.*`) or `GEO_NUMERIC_UNCERTAIN` (`budget.exact_predicates`) |
| A cap or section edge longer than the G05 per-edge cap of 1,000,000 mm, or a coordinate beyond 1e9 mm | `GEO_ABOVE_LIMIT` with the G05 diagnostic attached (G05 reports every failure of a stage; the operation attaches the first) |
| Candidate not certified (degenerate sliver, overlap, uncertain, several shells) | `result.degenerate`, `result.overlapping`, `result.uncertain`, `result.shells`; any other failed self-check `GEO_SELF_CHECK_FAILED` |
| Cancellation | `GEO_CANCELLED`, no result; polls `Validation`, `Build`, `Checks` (then the G06 polls), `Quantities`; none after sealing |

One cutter per call. Several holes are several calls on the result body (`OpResult::into_body`);
a second cutter that overlaps, touches or nests with an earlier hole is refused by the same
admission. The pilot refuses a plate whose plane cut leaves kept pieces touching along an edge
(`result.overlapping`) rather than sealing a pinched solid.

### Local feature ids (provisional, contract amendment required)

Input faces keep their ids, including a clipped face, and keep their relative order; a face whose
triangles all disappear leaves the table and is reported in `OpResult::removed_faces()` (the id is
lost, not renamed). New faces use the local kind `cut`, ordinal 0 to 999 supplied by the caller (the
engine owns the mapping from its authored feature and must keep it stable), appended after the
surviving faces: the plane-cut cap `cut:f.p<ordinal>`; hole walls `cut:f.h<ordinal>.s<k>` (edge `k` of
a polygonal cavity, authored order, as G05's `f.c<j>.s<k>`) or `cut:f.h<ordinal>` (the one curved wall
of an approximated curve cavity). All ids are at most 22 bytes (F06 allows 64 bytes in total with
`fid1:`, the 36-byte ElementId and the colon), use `a-z0-9:.-`, are unique, and a reused
ordinal is refused. Edges and vertices are not named. F06 0.1.0 (frozen) defines only the plate
and member boxes; before an engine may qualify and persist the `extrusion` and `cut` kinds, an
amendment must adopt this grammar. Needed amendments, none made here: operation results and the
`cut` kind; the proposed tokens `GEO_OP_UNSUPPORTED` and `GEO_OP_NO_EFFECT`; the curved-wall and
approximated-volume statuses already noted for G05; the BCMESH01 face-table meaning of a face lost by
an operation.

### Numbers and bounds

Volumes are enclosures of the exact signed volume of the binary64 mesh (G06); they are mesh values,
never engineering quantities (F06 section 8: `unsupported`). A hole's analytic volume is exact for
the polygon the cavity really is; for an approximated curve it is the polygon's, and the G05
`CurveApproximation` carries the bound to the true curve. A plane cut has no closed form
independent of its own section, so `analytic_volume_mm3` is `None`; the closed forms live in the
tests. The result keeps the body's `Tolerance`, and cap admission uses the larger of that floor and the one derived from the (possibly smaller) cap outline, so a hole after a cut cannot be finer than the original body allowed. Work is bounded by the budgets above and the F06
mesh caps (4096 vertices, 8192 triangles); no recursion, no `unsafe`, no panicking path, and every
allocation is sized by a validated count. Certification of large bodies is bounded by the G06
pair budget: a 16-hole body of 7,756 triangles is refused at admission as not certified (tested).

### Verification and case coverage

Tests call the public API and reread the actual buffers with independent code in `tests/common`,
`tests/common_mesh` and `tests/common_ops` (blob reader and re-encoder, edge-incidence map, Euler,
divergence volume, per-face area vectors, independent canonical-order check; no production math).

| File | Content |
|------|---------|
| `tests/g09_cut.rs` (17) | Corner, hexagon and general planes of boxes against the inclusion-exclusion closed form `sum (-1)^k max(0, d - n.s)^3 / (6 n1 n2 n3)` and voxel counting; both halves add up; vertex-on-plane, edge-on-plane and reflex-vertex planes; horizontal cut of a plate with a hole (section with a cavity, genus kept); cut of an L prism against an independent polygon clip; translation, quarter-turn and mirror; repeat identity and 12 renumberings per body with identical bytes; cut of a cut; tangent, coplanar, outside, empty, non-finite, zero and tiny normals, ordinals, id collision, several section loops, pinched kept side, and the whole face-table gate |
| `tests/g09_hole.rs` (14) | Round, slot, ellipse and polygon holes against closed-form areas (`n/2 r^2 sin(2 pi/n)`, obround, ellipse), the same vertex set, volume and surface area as the G05 extrusion of the same profile, genus and Euler change (+1 per hole, `2 - 2g`), one to three holes in any order, offset and L prisms with existing holes, quarter turns and mirrors, voxel counting, canonical bytes under renumbering, G02 plate and box and triangular prism bodies, shear and pyramid refusals, every contact, range, winding, non-finite and ordinal refusal |
| `tests/g09_budget_fuzz.rs` (6) | Every budget at its value and one below (triangles, loops, cap points, exact predicates, certification pairs) with identical bytes at the limit; cancellation at every poll of a hole, a cut and a cut through vertices (poll order fixed, no poll after the cancelled one, none after sealing); seeded hostile planes (vertices, midpoints, three-vertex planes, special values such as `1e-300`, `5e-324`, `1e9`; 192 cuts of which 65 seal and 32 give both halves, each success re-verified and the halves summing to the body); seeded hostile cutters (240 attempts over four sequential holes, each success checked against the shoelace area); 600 mutated cube blobs |
| `src/ops` unit tests (12) | Exact side sign against a rounded-sum cancellation and an integer reference, distance bound, validation order, loop chaining, canonical loops, exact area sign against an `i128` reference, cap tiling and budgets, loop splitting, corrupt candidates that never seal, canonical bytes |

G03 fixtures, run by `tests/g09_hole.rs` on the real G02 plate and operation: `g03.plate.through-hole`
(23 x 47 x 3, round hole of diameter 8 at the centre), `g03.plate.cavity` (rectangular void
`11 x 23`, exact volume `2484`) and `g03.plate.thin-wall` (a void leaving the minimum wall of
0.001 mm on every side, accepted at exactly `eps_geom` and refused one representable step
tighter). Their rows in `docs/fixtures/g03/expectations.tsv` stay `not_run` (G03 is not edited and
its harness reads no operation results); this is a mapping, not a changed record.
`g03.plate.nonrectangular` is an outline (A02/G05), not a cut, and is **not run**. The large and
dimension fixtures remain with G02 and G05.

### Not implemented

Oblique, curved or multiple simultaneous cutters; holes that touch or cross the outline; blind or
partial holes; non-prism bodies for holes (a tilted cut result is refused as `body.not_prism`);
several outer section loops; sections that touch themselves; coplanar and tangent planes; edge and
vertex names; a general boolean, offset or fillet; placement and rotation of the body (the caller
supplies the body-frame plane and cutter; exact quarter turns and mirrors are tested by transforming
both); persistent topological naming across operations beyond the rules above; engine and CLI
commands, keys and queries (P01, A03, A04); cross-target byte evidence; `cargo deny` and
`wasm32` checks (not installed here). The two design-section-8 G09 items "query/E2E validation" and
"no claim from mock-only checks" are respected by leaving the first undone.

### Provenance

Original scalar code from public mathematics only: half-space clipping of a convex polygon (the
Sutherland-Hodgman idea for one plane), exact signs of integer-scaled binary64 values with a
static rounding-error filter derived in the code, the shoelace area, the divergence theorem for
signed volume, the Euler-Poincare relation `V - E + F = 2 - 2g`, inclusion-exclusion for the
volume of a box cut by a half-space (tests only), and the G05/G06 mathematics of this crate. No
third-party code, crate, test or data and no commercial-product material was consulted; there is
no copied asset. The clean-room input list of this task is the repository's own kernel code and
documents named in the task, and the public mathematics above.

## P03: straight member (`member` module)

`straight_member(StraightMember { start, end, width_mm, height_mm, roll_rad })` builds the F06
section 6 box of a centred rectangle section along an axis, in the certified `geom::MemberFrame`
(F06 section 4 rules 1 to 3: default frame by the horizontal or near-vertical rule, explicit roll,
frame certificate). It returns the same sealed `PlateResult` as `rectangular_plate`: 8 vertices,
12 triangles, 6 faces (`f.x0 .. f.z1`, `z0` is the end face at `p1`), one 392-byte BCMESH01 blob,
volume `(width * height) * axis length`, and the body-scaled tolerance. The shared self-check now
takes the six outward face normals, so a rotated frame is checked against its own normals and not
against the axes. Failures are one typed diagnostic with the new fields `axis_points_mm`, `profile`
and `roll_rad`; an axis shorter than the floor is `GEO_BELOW_MINIMUM` on the axis (`INVALID_ARGUMENT`).

Verified by `tests/member.rs` against hand-derived closed forms: a vertical axis (`X_m = +X`,
`Y_m = +Y`), a horizontal axis along `+X` (`Z_m = +X`, `Y_m = +Z`, `X_m = +Y`), a quarter-turn roll,
a tilted axis with a 7000 mm length from `(2000, 3000, 6000)`, divergence-theorem volumes of the mesh,
and the typed failures. Not here: other profile kinds, element placement rotation, sin/cos
reproducibility across targets (still the open owner decision of `geom`), keys and FeatureId tables
beyond the plate/member box tokens.

## G07: acceptance of the first scope

[`docs/design/g07-kernel-first-scope-acceptance.md`](../../docs/design/g07-kernel-first-scope-acceptance.md)
records the independent acceptance (ACCEPTED WITH CONDITIONS): the supported and unsupported lists,
a trace of every validation-plan row and every F06 case to code and tests, the measured numbers and
the preconditions for G08. G07 added `tests/g07_acceptance.rs` (member meshes against the F06 vertex
lists, special-value sweeps over every public builder and operation, never-exact mesh probes, a
near-tolerance cut, pinned blob digests), `examples/g07_bench.rs` (timing and determinism harness,
`cargo run --release -p beamcraft-kernel --example g07_bench`) and `Code::AxisTooShort`
(`GEO_AXIS_TOO_SHORT`), which the member builder now reports for an axis below the floor together
with the contract's stages for a failed frame certificate and a non-finite derived scale.

## A07.1: section outlines and members of any section (`section` module)

`outline(shape, representation)` turns an admitted `model::section::SectionShape` into a G05
profile centred on the section's bounding box: a detailed outline whose radii are quarter arcs
(vertices on the arc, the G05 deterministic angular grid, sagitta bound `max(0.25 mm, 0.5 % of
the radius)`) and a simplified one with sharp corners and a 16-edge circle. `section_member`
extrudes the outline along the member axis with G05, places it with the `geom::MemberFrame`
basis (`world = R*local + p1`) and certifies one closed shell with G06. No dependency, asset or
dimension table is added; family geometry follows public engineering descriptions. The polygon of
the detailed outline is geometry and display only: `SectionResult::net_area_mm2` is its area and
is not the section area. The exact area (closed form with true arcs) lives with the parameters in
`model::section` (`exact_area_mm2`) and feeds the quantities (P-Mass,
`docs/design/p-mass-section-area.md`). `SectionResult::axis_length_mm` is the extrusion length.
Evidence: `tests/section.rs` and the unit tests of `section.rs` (a finely subdivided polygon
converges to the exact area of every family). Details: `docs/design/a07-section-library.md`.
