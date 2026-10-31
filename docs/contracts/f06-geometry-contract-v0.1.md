# F06: geometry and viewport contract, version 0.1.0

Status: **frozen 0.1.0** (2026-10-20). Changes only by a new version.

This contract fixes the logical interface between the engine and the geometry kernel, and between
the engine and any viewport or query consumer: what goes in, what comes out, which numbers are
bounded, which identifiers are stable and which results may be called current. It describes no
algorithm and no code. The [acceptance cases](../../tests/fixtures/contracts/f06-geometry-contract-cases-v0.1.json)
are the expected behaviour in machine-readable form.

## 1. Scope and status

The model contract ([F04](f04-model-and-commands-v0.1.md)) defines identities, units, input caps,
error codes and quantity statuses; this contract never weakens or widens them. It owns the
geometric tolerance, the minimum dimension, the "parallel" test of the member frame and the logical
content of a profile definition.

**Excluded from 0.1.0:** general boolean operations and fit or weld geometry; curved or
multi-segment members; bent plates; imported meshes; scale, shear and reflection; drawing
projection and dimensions; rendering style, shading and
lighting; persisted storage formats (the USD encoding is not defined here, and this contract defines
no second persisted model format); and any claim of numerical accuracy beyond determinism (section
2). In scope are exactly three operation classes: through holes, plane cuts and the extrusion of a
contour with polygonal cavities (section 2, "Operations").
True curved surfaces (analytic cylinders, arcs, splines) stay unsupported with typed results: a
round hole is a regular polygon (section 2), never an `Approximated` result.

**Public references** (rules are referenced, not implementations copied):

- [IEEE 754-2019](https://standards.ieee.org/ieee/754/6210/) binary64 arithmetic and correctly
  rounded square root.
- [Euler characteristic](https://en.wikipedia.org/wiki/Euler_characteristic) of a closed polyhedral
  surface, and the [divergence theorem](https://en.wikipedia.org/wiki/Divergence_theorem) volume of
  a closed oriented triangle mesh, `V = (1/6) * sum(v0 . (v1 x v2))`.
- [UsdGeomMesh](https://openusd.org/release/api/class_usd_geom_mesh.html): `orientation`
  (`rightHanded` means counter-clockwise winding seen from the front, normals by the right-hand
  rule), `subdivisionScheme` and `faceVertexCounts`.
- [RFC 8259](https://www.rfc-editor.org/rfc/rfc8259.html) for the logical JSON forms and [RFC
  6234](https://www.rfc-editor.org/rfc/rfc6234.html) for SHA-256.

**Units inside are always millimetres.** Whatever units a project shows to its user, metric or
imperial (inches, feet-and-inches, fractions such as 3/16"), the kernel, the stored model, every
key, tolerance and quantity work in millimetres, radians, kilograms and cubic metres. Display units
are a presentation setting of the UI and of reports, never a property of the geometry:

- Input in imperial units is converted once, at the UI or IO edge, with the exact factor
  1 in = 25.4 mm, to the nearest binary64 millimetre value; nothing downstream sees the original
  unit.
- Output is converted at the edge and rounded only for display; the stored value keeps full
  precision.
- USD layers keep `metersPerUnit = 0.001`; switching a project between metric and imperial display
  changes no authored value, no shape key and no quantity.
- Limits and tolerances (`eps_geom`, the 0.001 mm floor, caps) stay defined in millimetres.

## 2. Kernel input

A kernel call takes one closed, finite, bounded `GeometryInput`. Unknown fields are rejected
(`UNSUPPORTED_PROPERTY`). It contains no element identity, label, appearance, camera, clock or
random seed: the result depends on this input only.

| Field | Rule |
|-------|------|
| `contract_version` | Exactly `0.1.0`; otherwise `UNSUPPORTED_VERSION` |
| `body` | Exactly one of the two bodies below |
| `placement` | `translation_mm` (3 numbers) and `orientation_xyzw` (4 numbers), F04 |
| `density_kg_m3` | Optional; a number or null. Absent or null means unknown (F04) |
| `outputs` | Nonempty set drawn from `mesh` and `quantities`, no duplicates |

**Plate**: `contour_points_mm` and `thickness_mm`, using the frame, normal
sign, bottom face, bounds and validation of F04. `width=|p1-p0|`, `length=|p2-p1|` are
derived only for an exact four-point rectangle. A valid simple polygon in the plane `z = 0` is
supported: it is extruded along `+Z` by the thickness, exactly as authored,
without reordering, merging or repair; the origin rectangle keeps the box body, mesh and bytes below.
A valid contour outside that plane is `unsupported`, `GEO_UNSUPPORTED_CONTOUR`, mapped to existing
`UNSUPPORTED_GEOMETRY`. An invalid contour is `GEO_CONTOUR_INVALID`, mapped to `INVALID_ARGUMENT`. These two GEO codes are necessary to
distinguish unsupported scope from invalid authored geometry; no F04 code is added. For a rectangle,
`o=p0`, `e1=p1-p0`, `e2=p3-p0`, `e3=thickness*Z_p`. The origin rectangle retains the same body, mesh
vertices, volume, area and bounds as the previous dimensions input.

**Straight member**: `axis_points_mm` (two element-local points),
`profile`, and optional `roll_rad` (finite, default 0; absolute cap 6.283185307179586 rad). The
profile kind `rectangle` has `width_mm` and `height_mm`; the nine parametric section families and
custom outlines are centred on their bounding box and built by the kernel's `section` module; no
other kind is supported. Its section is centred on the axis, width along rolled X_m, height
along rolled Y_m, perpendicular ends through p1/p2 and no extensions. Section 4 builds the default
frame first, then applies explicit right-handed roll about +Z_m.

**Profile resource.** The kernel consumes the
logical content of a profile definition, not its file format. For `rectangle` that content is the
section kind, `width_mm`, `height_mm` and units `mm`, carried by an immutable definition with a full
content digest as F04 requires. Its persisted encoding is not defined here. This contract defines no second persisted format and no catalogue.

**Caps (from F04, never widened; F06 additions marked).**

| Input | Rule | Origin |
|-------|------|--------|
| Plate thickness/edge, member axis length | greater than zero, at most 1,000,000 mm | F04 |
| Contour/axis point coordinates, translation components | absolute value at most 1,000,000,000 mm | F04 |
| Contour D / count | at most 2,000,000 mm / 1024 points | F04 |
| Roll | absolute value at most 6.283185307179586 rad | F04 |
| Density | at least zero, at most 1,000,000 kg/m3, when present (amended 2026-10-30, [density amendment](f04-material-density-amendment-v0.1.md)) | F04 |
| Quaternion | components in [-1, 1]; squared norm within 1e-6 of one | F04 |
| Section `width_mm`, `height_mm` | at most 1,000,000 mm | F06 |
| Local size L | plate D <= 2,000,000 mm; member max(axis length, section half-diagonal) <= 1,000,000 mm | F04 |
| Minimum nonzero dimension | at least eps_geom (section 4), inclusive | F06 |

The geometric floor is eps_geom, with an absolute minimum of 0.001 mm = 1e-6 m. Contours contain
at most 1024 points; roll has the F04 cap. A maximum plate call has 3081 numeric scalars
(3072 coordinates, thickness, seven
pose components and density); a member has 17 including explicit roll. Bound counts before any
allocation and simplicity work to at most 523,776 non-adjacent/pair candidates (1024*1023/2); no
recursion is needed. Invalid contour checks precede the contour support decision, so a
malformed pentagon is invalid, never merely unsupported.

**Deterministic evaluation order.** A call runs these stages in order and stops at the first stage
that reports a failure; it reports all failures of that stage (at most 32) in input field order:

1. **Support**: body kind, profile kind, transform form and mesh source (`unsupported`).
2. **Finiteness**: every f64 (`NON_FINITE_NUMBER`).
3. **Caps and scale**: submitted caps, derived norms and local size L, then eps_geom as in
   section 4. A nonpositive section dimension contributes 0 to L (stage 4 reports it). A finite
   cap excess is `LIMIT_EXCEEDED`; a non-finite derived intermediate is
   `GEO_NUMERIC_UNCERTAIN` (`UNSUPPORTED_GEOMETRY`), without a substituted floor.
4. **Ranges and contour validity**: greater than zero, minimum dimensions, axis length, density,
   quaternion and F04 contour validation (`INVALID_ARGUMENT`). Then decide contour
   support: a valid contour that is not a simple polygon in the plane `z = 0` is
   `unsupported`. The stage-1 body check admits every plate contour
   to these bounded checks; it never skips invalid-input validation.
5. **Build** (derive frame, certify it, derive vertices, tables), **checks** (section 6 self-checks)
   and **quantities** (section 8), with cancellation points between them (section 10).

Arithmetic uses binary64 with round-to-nearest-even, `+ - * /` and `sqrt`, with certified exact
signs for contour topology and `sin`/`cos` for explicit roll, no fused multiply-add contraction, and
the operand order written in this contract. Negative zero is normalized to positive zero in every
output and every key. No clock, random source, thread count, locale, hash-map iteration order or
environment variable influences a result. The selected audited math crate, its version and trig
evaluation profile are part of the algorithm version. Omitted roll and explicit zero bypass
trigonometry. Nonzero roll needs cross-target conformance evidence before identical bytes can be
claimed; no libm choice or dependency is made by this document. This is a determinism rule, not an
accuracy promise; accuracy bounds come from fixtures.

**Order of evaluation for quantities and dot products.** Call the three box edge lengths `a`, `b`,
`c` (plate `width, length, thickness`; member `w, h, ell`). Then `V_mm3 = (a*b)*c`,
`S_mm2 = 2*((a*b + a*c) + b*c)`, `V_m3 = V_mm3 / 1e9`, `S_m2 = S_mm2 / 1e6` (division by
exactly representable integers) and `mass = (V_mm3 * density) / 1e9`. A dot product is `(r1*v1 +
r2*v2) + r3*v3`; world coordinate `i` of a vertex `(x, y, z)` is `(((R_i0*x + R_i1*y) + R_i2*z) +
t_i)`. Bounds take the minimum and maximum of the eight computed vertices with `<` and `>`, scanning
vertex indices in ascending order.

**Thresholds, certification and conformance.** Every decision of this contract (caps, minimum
dimension, parallel test, status) is the written binary64 comparison on the binary64 values defined
above; there is no unspecified rounding band, and a value exactly at an inclusive limit is accepted.
Where a wrong decision would yield an invalid result rather than a different valid one, the result
carries a numeric certificate (for example the frame certificate of section 4, rule 2): a build,
mesh or query result whose certificate fails is `GEO_NUMERIC_UNCERTAIN` (`UNSUPPORTED_GEOMETRY`) and
is never returned as a partial or current result. The parallel test needs no band because both of
its branches give a certified frame on either side of the threshold. The claim of identical bytes
and decisions on every platform is a rule to be demonstrated, not a verified property: the
fixtures must supply conformance evidence on each target, including `wasm32`. Any change that alters
output bytes or a decision for some admitted input raises `algorithm` and, with it, every key
(section 7).

### Operations on a plate or extrusion body

An operation takes one sealed body and returns one new sealed body or a typed failure;
like any kernel call it returns no partial geometry and mutates no authored state (section 3). The
engine always evaluates a body as base shape plus its operations replayed in ascending ordinal
order, never incrementally from a stored mesh, so the result depends on the authored inputs only.

| Operation | Input class | Result |
|-----------|-------------|--------|
| Extrusion of a contour with polygonal cavities | One counter-clockwise simple outer polygon in `z = 0` and clockwise simple cavity polygons strictly inside it and apart from each other by at least `eps_geom`; thickness as for a plate | Sealed closed mesh straight along `+Z`, genus equal to the cavity count, blob, and the exact polygon volume |
| Through hole | A certified single-shell prism along `Z` (plate, extrusion, or a cut result that is still such a prism); one clockwise simple polygon strictly inside the outline, clear of earlier holes by at least `eps_geom`, reaching past both caps by at least `eps_geom` | Sealed body with genus plus one, new `cut` faces, exact polygon volume (section 8) |
| Plane cut | A certified element-local single-shell body; one finite plane; the kept half-space; an ordinal; the plane must properly cross the body once with one outer section loop | Sealed, re-certified body with one planar cap face and the certified largest deviation of the rounded new vertices from the plane |

Every result is re-certified (closed, outward, manifold, free of self-intersection and numeric
certificate, section 6) before sealing. A result that cannot be certified is `GEO_NUMERIC_UNCERTAIN`
or `GEO_MESH_NOT_CERTIFIED`, never a current result.

**Round holes.** A round hole of radius `r` is the regular
`N`-gon inscribed in its circle. `N` is the smallest multiple of 4 with `N >= 16` such that the chord
deviation (the sagitta) `r * (1 - cos(pi / N))` is at most 0.05 mm, computed from the radius; `N` is
at most 256. For example diameters up to about 5.2 mm give `N = 16`, 100 mm gives 72, 500 mm gives
160 and about 1328 mm gives 256. A radius that needs `N > 256` is refused as `GEO_OP_UNSUPPORTED`
(field token `hole.radius`, because no `Approximated` status exists). The
decision of `N` compares `r` with a fixed threshold table of the algorithm version, so no `cos`
result decides it; vertex coordinates use the platform `sin` and `cos` at the input edge and are
authored as the doubles they round to (the cross-target question of roll, section 2, applies).
The stored polygon, not the circle, is the geometry. No `Approximated` status exists in 0.1.x: the kernel's curve-approximation entry points and their `Approximated` volume are
not part of this contract and an engine does not use them.

**Failures.** A cutter or cavity polygon that is invalid under F04 is
`GEO_CONTOUR_INVALID`. A well-formed request outside the supported scope (a hole touching or
crossing the outline or another hole, nested islands, oblique, curved or simultaneous cutters,
blind holes, a body that is not a prism, a plane that is tangent, coplanar with a face, or makes
several outer section loops) is `GEO_OP_UNSUPPORTED`, or `GEO_UNSUPPORTED_CONTOUR` where the
contour is the cause. A request that would leave the body
unchanged or empty (a plane missing the body or keeping all or nothing) is `GEO_OP_NO_EFFECT`. The
`field` token of these diagnostics is a fixed defect token of at most 64 ASCII bytes
(`plane.tangent`, `plane.coplanar_face`, `section.not_admissible`, `result.uncertain`, ...), never an
input value. Kernel work budgets (certification, exact-sign and pair budgets) that end a call early
are `GEO_NUMERIC_UNCERTAIN` at kernel level and are reported by the command layer as the capacity
result below, never as invalid geometry.

**Capacity is an implementation limit, not a product limit.** There is no
designed limit on the number of holes of a plate or beam. The kernel's present capacity is a
certification performance limit that lives in code (`OpLimits`, `Limits`), not in this contract.
The command layer checks the request against the implementation limits before it runs
the kernel and refuses it with the distinct result `GEO_KERNEL_CAPACITY`, mapped to the existing
F04 code `LIMIT_EXCEEDED` (status `limit_exceeded`) with the diagnostic `field` token
`kernel_capacity`; it publishes nothing (F04). The section 6 mesh caps (262,144
vertices, 524,288 triangles) are hard validator caps of this contract, not designed hole limits; a
request whose derived mesh would exceed them keeps `GEO_BUFFER_LIMIT` or `GEO_TOPOLOGY_LIMIT`
(also `LIMIT_EXCEEDED`). A call that passes the pre-check but ends on a kernel budget is
mapped to the same result. A capacity refusal is never `INVALID_ARGUMENT` and never "invalid
geometry".

## 3. Result and diagnostics

Every kernel call returns one `GeometryResult`:

| Field | Rule |
|-------|------|
| `contract_version` | `0.1.0` |
| `status` | One of `ok`, `unsupported`, `invalid_input`, `limit_exceeded`, `cancelled` |
| `diagnostics` | Array of at most 32 records; empty exactly when `status` is `ok` |
| `versions` | `algorithm`, `tolerance_policy`, `feature_id_rule`, `mesh_layout` (section 7), each an integer |
| `shape_key` | Section 7; present only when `status` is `ok` |
| `mesh` | Descriptor and blob of section 6; present only when `ok` and requested |
| `quantities` | Records of section 8; present only when `ok` and requested |

**Diagnostic record.** Exactly `code` (a stable `GEO_` code below), `f04_code` (its F04
mapping), `field` (a fixed schema token of at most 64 ASCII bytes, never an input path or value) and
`stage` (`support`, `finiteness`, `caps`, `ranges`, `validation`, `build`, `checks` or
`quantities`). Messages use fixed English text and never echo input values, private paths or
exceptions. When a command reports a record it uses `field`, a null `target_id` and
`reason_code = f04_code`; the `GEO_` code stays in the geometry result body.

**Status mapping.** `ok` has no code. `unsupported` maps to `UNSUPPORTED_GEOMETRY`. `invalid_input`
maps to `INVALID_ARGUMENT`, `NON_FINITE_NUMBER` or (blob structure only) `INTEGRITY_ERROR`.
`limit_exceeded` maps to `LIMIT_EXCEEDED`. `cancelled` maps to `CANCELLED`. The overall status is
the status of the first diagnostic.

| `GEO_` code | F04 code | Meaning |
|-------------|----------|---------|
| `GEO_CONTOUR_INVALID` | `INVALID_ARGUMENT` | Invalid contour under F04 |
| `GEO_UNSUPPORTED_CONTOUR` | `UNSUPPORTED_GEOMETRY` | Valid contour outside the supported evaluation (not a simple polygon in the plane `z = 0`) |
| `GEO_NON_FINITE` | `NON_FINITE_NUMBER` | NaN or infinity in any number |
| `GEO_NOT_POSITIVE` | `INVALID_ARGUMENT` | Dimension or similar not greater than zero; a negative density (amended 2026-10-30, [density amendment](f04-material-density-amendment-v0.1.md)) |
| `GEO_BELOW_MINIMUM` | `INVALID_ARGUMENT` | Dimension greater than zero but below the minimum |
| `GEO_ABOVE_LIMIT` | `LIMIT_EXCEEDED` | Input above its cap |
| `GEO_AXIS_TOO_SHORT` | `INVALID_ARGUMENT` | Axis length zero or below the minimum |
| `GEO_QUATERNION_INVALID` | `INVALID_ARGUMENT` | Component range or norm rule violated |
| `GEO_UNSUPPORTED_PROFILE` | `UNSUPPORTED_GEOMETRY` | Profile kind other than `rectangle` |
| `GEO_UNSUPPORTED_TRANSFORM` | `UNSUPPORTED_GEOMETRY` | Scale, shear, reflection or a matrix form |
| `GEO_UNSUPPORTED_STAGE_FRAME` | `UNSUPPORTED_GEOMETRY` | Source unit or up axis other than mm and Z |
| `GEO_UNSUPPORTED_MESH_SOURCE` | `UNSUPPORTED_GEOMETRY` | Any imported mesh as kernel input |
| `GEO_BUFFER_LIMIT` | `LIMIT_EXCEEDED` | Vertex, triangle or byte cap exceeded |
| `GEO_TOPOLOGY_LIMIT` | `LIMIT_EXCEEDED` | Face, edge, shell or feature cap exceeded |
| `GEO_BLOB_INVALID` | `INTEGRITY_ERROR` | Blob structure disagrees with its header |
| `GEO_MESH_NOT_CLOSED` | `UNSUPPORTED_GEOMETRY` | Open shell: no exact status |
| `GEO_MESH_INWARD` | `UNSUPPORTED_GEOMETRY` | Closed but wound inward |
| `GEO_MESH_NOT_CERTIFIED` | `UNSUPPORTED_GEOMETRY` | No certificate of freedom from self-intersection |
| `GEO_SELF_CHECK_FAILED` | `UNSUPPORTED_GEOMETRY` | The kernel's own result failed section 6 checks |
| `GEO_NUMERIC_UNCERTAIN` | `UNSUPPORTED_GEOMETRY` | A numeric certificate failed (frame orthonormality, non-finite intermediate) in a build or query |
| `GEO_CANCELLED` | `CANCELLED` | Cancellation observed before the seal point |
| `GEO_FEATURE_NOT_FOUND` | `NOT_FOUND` | Well-formed FeatureId naming no feature |
| `GEO_FEATURE_ID_INVALID` | `INVALID_ARGUMENT` | Malformed FeatureId |
| `GEO_TRIANGLE_ADDRESS_FORBIDDEN` | `UNSUPPORTED_PROPERTY` | A triangle or vertex index used as an address |
| `GEO_STALE_TOKEN` | `STALE_WORKSPACE` | Snapshot token older than the retained history |
| `GEO_PICK_INPUT_INVALID` | `INVALID_ARGUMENT` | Bad ray or point query input |
| `GEO_PICK_LIMIT` | `LIMIT_EXCEEDED` | Hit count or radius above its cap |
| `GEO_OP_UNSUPPORTED` | `UNSUPPORTED_GEOMETRY` | Valid operation request outside the supported scope; status `unsupported` |
| `GEO_OP_NO_EFFECT` | `INVALID_ARGUMENT` | Operation that would leave the body unchanged or empty; status `invalid_input` |
| `GEO_KERNEL_CAPACITY` | `LIMIT_EXCEEDED` | Request above the kernel's implementation capacity, refused before or after the kernel run; status `limit_exceeded`; never invalid geometry |

The first 21 codes belong to the kernel and blob stages (`GEO_NON_FINITE` and
`GEO_NUMERIC_UNCERTAIN` also occur in queries); the next six to queries (section 9); the last three
to operations (failures of an operation use stage `build`, or `checks` for a failed
re-certification). Every F04 code used here exists in F04.

**Failure never returns partial geometry as current.** A result whose status is not `ok` has no
`mesh`, `quantities` or `shape_key`. A kernel call alone never mutates authored state. A rejected
mutation or cancelled build retains the previous result and generation (F04). A separately
accepted valid contour outside the supported evaluation (for example not in the plane `z = 0`)
create/update commits authored input with geometry and quantities
`unsupported`; an old result cannot remain current under changed shape inputs. This is model
admission, not partial geometry returned by a failed evaluation. An element whose authored shape
is invalid is a separate state of the engine, `degenerate` (F04): it has no kernel result, and its derived data are `unknown`, never guessed. A mesh that
fails self-checks or a
numeric certificate is discarded, never displayed as current. A mesh that is not a sealed result
of the analytic builders or of the operations of section 2 may be shown by a viewport
only if the host flags it "not exact"; it never receives engineering quantities.

## 4. Tolerance and degeneracy policy

All constants carry integer `tolerance_policy` version 3. The algorithm version is 3
because local-size scaling changes geometry admission; feature and mesh layout versions stay
1. Changing any constant raises that version and therefore every shape key (section 7).

| Constant | Value | Use |
|----------|-------|-----|
| Geometric length floor `eps_geom`; minimum `eps_dim = eps_geom` | max(0.001 mm, 2^-22 * L), inclusive for minima | Plate thickness/edges, section dimensions, axis length; local L below |
| Parallel threshold `eps_par` | 1e-6, strict | The parallel test of the member frame |
| Planarity `eps_plane` | eps_geom + 1e-6*D | Contour admission; F04 |
| Frame/quaternion certificate/admission | 1e-6, dimensionless | Absolute deviations from unit identities |
| Coincidence | Two points coincide when their distance is below `eps_geom` (strict) | Axis endpoints; contour edge minima reject close consecutive points without merging |

**Geometric length floor.** `eps_geom = max(0.001 mm, 2^-22 * L)`.
Here `L` is an element-local body size in mm, evaluated before placement: for a plate `L = D`,
the maximum distance from contour point p0 to any contour point; for a member `L = max(ell, H)`,
where `ell` is the axis length and `H` is the section half-diagonal. World coordinates and
translation magnitude never enter this scale. Thickness does not enter plate D; roll and rigid
placement do not enter member L.

Use binary64 round-to-nearest-even at every written operation, without reassociation or fused
multiply-add. Define `norm(v) = sqrt((v_x*v_x + v_y*v_y) + v_z*v_z)`. Subtract submitted
element-local point components first, then evaluate each norm in that order. For a plate scan
`norm(p_i - p0)` in ascending point index, starting with D = 0 and retaining the larger value
with `>`; set L = D. For a member compute `ell = norm(p2 - p1)`, `hw = width_mm/2`,
`hh_section = height_mm/2`, `H = sqrt(hw*hw + hh_section*hh_section)`, then L = max(ell, H). A
section dimension that is not greater than zero contributes 0 to H (`hw = 0` or
`hh_section = 0`): it is rejected in the range stage as `INVALID_ARGUMENT`
(`GEO_NOT_POSITIVE`), never as an above-limit or numeric-uncertainty result of this derivation.
Finally multiply L by the exactly representable binary64 constant `2^-22 = 1/4194304`, then
take max with binary64(0.001 mm). Thus eps_geom is never smaller than binary64(0.001 mm).
All length operands and results are in mm; the scale factor is dimensionless.

Check submitted finiteness and F04 caps before using the floor. Plate L must be at most
2,000,000 mm (the existing D cap); member L must be at most 1,000,000 mm (the existing axis
and section caps imply this). An excessive finite L is `LIMIT_EXCEEDED` (`GEO_ABOVE_LIMIT`
in F06), before minima or rectangle-only contour support decisions. Non-finite submitted input is
`NON_FINITE_NUMBER` (`GEO_NON_FINITE`). A non-finite derived L or norm intermediate from
otherwise finite capped input is numeric uncertainty: `UNSUPPORTED_GEOMETRY`
(`GEO_NUMERIC_UNCERTAIN`), with no floor, partial geometry or current key. Never clamp L,
substitute the absolute floor for a failed scale, or expand any F04 cap. Under the stated
caps, with nonpositive dimensions contributing 0, these square sums cannot overflow; the
diagnostic also covers arithmetic faults.

**Application and exclusions.** eps_geom replaces a fixed absolute length floor in minimum
dimensions, close consecutive contour points, coincident member endpoints, axis degeneracy,
planarity and geometric distance-to-boundary decisions. The planarity threshold is evaluated
as `scaled_relative = binary64(1e-6) * D`, then `eps_plane = eps_geom + scaled_relative`.
It is inclusive. A geometric distance to a boundary at most eps_geom is in the boundary band;
a greater distance is outside that band. This band classifies distance only: it cannot change
an exact sign, identity, intersection or topology, and it does not authorize snapping or healing.

Parallel/orthogonality checks expressed as a length or distance use eps_geom as their absolute
length floor. Dimensionless relative tests keep their existing bounds: in particular member
`eps_par = 1e-6` and its written squared multiplier 1e-12 are unchanged. Frame/quaternion
certificates and relative bounds retain 1e-6. CR-TOL-001 retains its absolute 0.001 mm term
and relative comparison term; it never uses eps_geom. Pick inclusion, its boundary flag,
distance buckets and the exact ray parameter interval remain governed solely by section 9:
eps_pick and the tie quantum stay absolute 0.001 mm even on a large body. Exact winding,
simplicity, rectangle closure/orthogonality, identities, topology, hashes and cap checks remain
exact. A valid near-planar contour outside the plane `z = 0` still has unsupported evaluation.

**Why this scale.** The floor follows the body's own size, never its distance from the world
origin. It supplies no runtime accuracy proof. The absolute floor protects
small bodies, and the relative term grows only with the body's own size inside existing caps.

**Rules.**

1. A dimension (plate thickness, section width/height, axis length) of zero or less is
   `GEO_NOT_POSITIVE`; a value above zero and below `eps_dim` is `GEO_BELOW_MINIMUM`; a value equal
   to `eps_dim` is accepted. A zero or too-short axis is `GEO_AXIS_TOO_SHORT` (`INVALID_ARGUMENT`). The axis length is `ell = sqrt((dx*dx + dy*dy) + dz*dz)`, subtracting
   `d = p2 - p1` per component before taking the norm.
   Contour edges below eps_geom, including zero edges, retain `GEO_CONTOUR_INVALID` under
   F04; they are never merged or reordered. A supported rectangle's width and length
   are its already validated contour edge lengths.
2. **Default zero-roll frame and parallel test.** Let `d = p2 - p1`, `hh =
   dx*dx + dy*dy`, `LL = hh + dz*dz`, `ell = sqrt(LL)`. Here ell is axis length,
   distinct from the tolerance scale L = max(ell, H). The frame is built in this order:
   - a. `Z = (dx/ell, dy/ell, dz/ell)`.
   - b. The first direction `Y`, by the first branch that applies. **Horizontal rule** when `hh >=
     1e-12 * LL`: with `h = sqrt(hh)`, `Y = ((-dx*dz)/(h*ell), (-dy*dz)/(h*ell), hh/(h*ell))`,
     which is local +Z projected onto the plane normal to `Z`. **Near-vertical rule** (also called
     the column
     rule) when `hh < 1e-12 * LL` (strict; anti-parallel counts; an exactly vertical axis is
     included): with `U = (0, 1, 0)` and `c = Z_y`, the vector `P = (-Z_x*c, 1 - Z_y*c, -Z_z*c)` is
     `U - Z*(U . Z)`, and `Y = P / |P|` with `|P| = sqrt((P_x*P_x + P_y*P_y) + P_z*P_z)`. An exactly
     vertical axis gives `c = 0` and `Y = (0, 1, 0)`. For a tilted near-vertical axis
     `Y` stays perpendicular to `Z`; a fixed `(0, 1, 0)` would not be (it would leave the section
     off perpendicular by the tilt toward Y).
   - c. `X0 = Y x Z`, `X = X0 / |X0|`, then `Y = Z x X`. The cross product is `(a2*b3 - a3*b2, a3*b1
     - a1*b3, a1*b2 - a2*b1)`. This applies to both rules and gives a right-handed frame (`X x Y =
       Z`).
   - d. For r=`roll_rad`, apply `X_r=cos(r)*X+sin(r)*Y` and `Y_r=-sin(r)*X+cos(r)*Y`, with Z
     unchanged. Positive roll takes X toward Y about +Z. At zero use X/Y directly; this is the only
     roll mechanism. Certify the rolled frame too.
   - e. **Frame certificate.** `| |X|*|X| - 1 |`, the same for `Y` and `Z`, the absolute values `|X
     . Y|`, `|X . Z|` and `|Y . Z|` of the three pairwise dot products, and `| (X x Y) . Z - 1 |`
     must each be at most 1e-6 (every bound is on an absolute value, so a negative error is rejected
     like a positive one); otherwise the build fails with `GEO_NUMERIC_UNCERTAIN`
     (`UNSUPPORTED_GEOMETRY`) and no partial result (`F06-NEG-31` injects `X . Y = +2e-6`,
     `F06-NEG-35` injects `X . Y = -2e-6`). In the near-vertical rule `|P|^2 = 1 - c^2 >= 1 -
     1e-12`, so the normalization does not amplify rounding error.

   The decision `hh < 1e-12 * LL` is the written binary64 comparison (section 2). Cases keep a
   margin on both sides, and one case sits at a relative margin of 1e-12 (`F06-POS-09`,
   `F06-POS-10`, `F06-POS-46`). Y-tilted near-vertical axes are `F06-POS-41` and `F06-POS-42`.
3. The roll is generally discontinuous at the threshold by construction; the jump depends on the
   tilt direction. Compare the two branch frames on the **same nonvertical unit axis** `Z` near the
   threshold, rotating from the near-vertical frame to the horizontal frame by the right-hand rule
   about `Z`. In exact arithmetic, let `r = sqrt(Z_x^2 + Z_y^2)` and `n = sqrt(1 - Z_y^2)`. Then
   `cos(theta) = -Z_y*Z_z/(r*n)` and `sin(theta) = Z_x/(r*n)`. For tilts from local +Z:

   | Tilt toward | Exact branch rotation | |-------------|-----------------------| | +Y | 180
   degrees (half turn) | | -Y | 0 degrees (no roll jump on this meridian) | | +X | +90 degrees
   (quarter turn) | | -X | -90 degrees (quarter turn) |

   These angles hold on either side of the threshold and give the one-sided limiting roll
   differences there. Frames on distinct finite axes also differ in axis direction, so their
   relative three-dimensional rotation is not solely a roll. For tilts from local -Z, the +Y and -Y
   entries exchange; the +X and -X entries keep their signs. Other tilt directions follow the
   formula above. The discontinuity is kept by design; the user sets `roll_rad` for another
   orientation. Optional `roll_rad` is the
   explicit section rotation about that default frame's +Z. Other orientation belongs to the element
   placement. No up-vector or hidden roll preset exists.
4. The kernel never snaps, welds or merges vertices, and never heals geometry. Vertices are distinct
   by construction, not by a tolerance test, and every box edge is at least `eps_dim` long after the
   range checks.
5. **Quaternion handling (F04 left this to F06).** The accepted quaternion is normalized:
   `n = ((x*x + y*y) + z*z) + w*w`, `r = sqrt(n)`, each component divided by `r`. The rotation
   matrix uses only products and sums of those components: `R00 = 1 - 2*(y*y + z*z)`, `R01 = 2*(x*y
   - z*w)`, `R02 = 2*(x*z + y*w)`, `R10 = 2*(x*y + z*w)`, `R11 = 1 - 2*(x*x + z*z)`, `R12 = 2*(y*z -
     x*w)`, `R20 = 2*(x*z - y*w)`, `R21 = 2*(y*z + x*w)`, `R22 = 1 - 2*(x*x + y*y)`. The matrix is
     quadratic in the components, so `q` and `-q` give bit-identical matrices and meshes.
     **Canonical sign for keys:** after normalization negate all four components when `w < 0`, or
     when `w = 0` and the first nonzero of `x, y, z` is negative; then normalize negative zero.
     Scale, shear and reflection never reach this step (they are unsupported).
6. **Tolerances in signatures.** The first scope has no caller-supplied tolerance. The
   `tolerance_policy` version is an input of the shape key and the world-measurement key (section
   7). A policy change is never silent and never leaves a current result keyed by the old policy.
7. **Separate from test comparison.** The test comparison rule (F04 section 10) is a
   rule for comparing test results only. The kernel never reads it, no signature contains it, and
   sharing a numeric value does not turn the test rule into a geometric decision rule. A test that
   needs "equal" for a decision (a branch, a status, a FeatureId) compares exactly.

## 5. FeatureId

A FeatureId is a stable semantic name for a face, edge or vertex of a body. It replaces
triangle, vertex and buffer indices as the only address that persists outside a mesh buffer.

**Form.** A qualified FeatureId is `fid1:` +
ElementId + `:` + local id, with the ElementId in its canonical lowercase 36-character form (F04
section 2). The local id is `kind:token`, where `kind` is `plate`, `member`, `extrusion` or `cut`
(the last two below), and `token` is one
of:

- face: `f.` + axis role, where the role is one of `x0`, `x1`, `y0`, `y1`, `z0`, `z1`;
- edge: `e.` + two roles of different axes in axis order (`x` before `y` before `z`), for example
  `e.x0.y1`;
- vertex: `v.` + three roles, one per axis, in axis order, for example `v.x0.y1.z1`.

The axes are those of the body frame: for a plate, contour-derived X_p, Y_p, Z_p (width, length,
thickness; the digit 0 is the minimum side and 1 the maximum side); for a member, `X_m`, `Y_m`,
`Z_m`, so `z0` is the end face at `p1` and `z1` the end face at `p2`. A body has 6 faces, 12 edges
and 8 vertices: 26 features. Allowed characters are `a-z`, `0-9`, `:`, `.` and `-`. The longest
form, a member vertex, is 59 bytes; the maximum length is **64 bytes**. The kernel produces local
ids; the engine prepends `fid1:` and the ElementId.

**Kinds `extrusion` and `cut`.** Both are
role-based, like the box kinds, and name features by authored order and operation ordinal, never by
triangle, vertex or buffer index.
A contour point is addressed by its position `k` in the authored contour; the contour token `<t>` is
`o<k>` for point `k` of the outer contour and `c<j>.<k>` for point `k` of cavity `j`.

- `extrusion` (a contour extruded along `+Z`; every contour that is not the origin rectangle, and
  any extrusion with cavities): faces `f.z0` (bottom, the contour plane), `f.z1` (top), `f.o<k>`
  (wall on outer edge `k`, from point `k` to `k+1`) and `f.c<j>.s<k>` (wall on edge `k` of cavity
  `j`); edges `e.z0.<t>` and `e.z1.<t>` (bottom or top boundary edge starting at point `<t>`) and
  `e.v.<t>` (vertical edge at point `<t>`); vertices `v.z0.<t>` and `v.z1.<t>`.
- `cut` (a face created by an operation with ordinal `n`, section 2): `f.h<n>.s<k>` (wall on edge
  `k` of the polygon of hole `n`) and `f.p<n>` (the planar cap of plane cut `n`). Edges and
  vertices created by an operation have no FeatureId in 0.1.x: they are not addressable until a
  later version, and a lookup of one is `GEO_FEATURE_NOT_FOUND`.
- The origin rectangle plate keeps the `plate` ids of the box above, also after operations: the
  faces of the base body keep their ids when an operation is added, changed or removed, and an
  operation adds or removes only its own `cut` faces.
- An operation ordinal is assigned by the engine from a per-element counter in creation order; it
  is never reused after a removal and never renumbered, so removing hole 3 does not rename hole 4.
- A feature that an operation removes or splits (an edge or vertex clipped by a plane cut, a wall
  removed by a cut) no longer resolves and is dangling (`NOT_FOUND`, rule 8); a face that a
  cut only shortens keeps its id.
- An edit that keeps the topology (moving a hole without changing its vertex count, thickness,
  placement) keeps every id (rule 4); changing the vertex count of a hole changes its `.s<k>` set.
  A cyclic start change of a contour moves its roles (rule 9).
- The 64-byte maximum of the qualified form stays: `cut` ids fit up to ordinal 999 and `k` up to
  1023 (58 bytes). An `extrusion` edge or vertex id of a cavity with large indices can exceed it
  (for example `extrusion:e.z0.c255.1023` gives 66); the implementing task must then refuse the
  build with `GEO_TOPOLOGY_LIMIT` (an implementation limit).

**Rules.**

1. **Independent of mesh order.** A FeatureId is built only from the rule version, the ElementId,
   the body kind and role tokens. It does not depend on dimensions, placement, vertex order,
   triangle order, buffer offsets or the platform.
2. **Uniqueness.** All FeatureIds of one result are pairwise different, and a FeatureId contains its
   owning element's identity, so two elements never share one.
3. **Rebuild with unchanged geometry** gives identical FeatureIds and identical mesh bytes.
4. **Edits that keep the topology** (dimension, roll, placement, density, rename) keep every
   FeatureId. A placement or dimension change moves the feature, not its name.
5. **Copy.** A copy has a new ElementId (F04 `CopyElement`), so all its FeatureIds are new; the
   local parts are the same, and the shape key and element-local mesh bytes are shared until an
   edit. The source's ids never change.
6. **Axis reversal.** Swapping `p1` and `p2` swaps the physical faces that carry `z0` and `z1` (and
   `x0`/`x1`, because `X_m` changes sign): ids follow roles, not world positions. A reference to a
   physical face is therefore tied to the frame rule and survives only as long as that rule gives
   the same role.
7. **Rule changes.** A change of the naming rule raises the version tag (`fid2`). Old references are
   migrated only by an explicit reviewed step, never reinterpreted silently.
8. **Never a triangle index.** A drawing, dimension, annotation or any stored reference addresses
   geometry by `(ElementId, local id, rule version)` only. Triangle, vertex and buffer indices are
   valid only inside one mesh snapshot and are never stored, returned by pick results, or accepted
   by a lookup (`GEO_TRIANGLE_ADDRESS_FORBIDDEN`). A reference whose FeatureId no longer resolves is
   dangling and reported as `NOT_FOUND`, not repaired by index.
9. **Contour start and roll.** Plate roles follow p0 and the first edge, not world positions. A
   cyclic start change can move physical face roles; no promise of a physical-face reference
   surviving that edit exists. Roll moves member roles with the section. Bodies that are not boxes
   use the kinds `extrusion` and `cut` defined above; no `fid1` box roles are invented for them.
10. **Mapping inside a snapshot.** The face table (section 6) maps a triangle to the local id of its
    face; vertex index `i = xb + 2*yb + 4*zb` maps to the vertex role bits. Edge geometry comes from
    `LookupFeature` (section 9), not from the buffer.

## 6. Mesh and buffers

**Layout rules.** Positions are binary64 millimetres. Indices are u32. The mesh is a triangle mesh
whose winding is counter-clockwise seen from outside, so face normals computed by the right-hand
rule point out of the body. This matches
[UsdGeomMesh](https://openusd.org/release/api/class_usd_geom_mesh.html) `orientation = rightHanded`;
a USD adapter must author `orientation` as `rightHanded`, `subdivisionScheme` as `none` and every
`faceVertexCounts` entry as 3 (the default scheme subdivides, so it is never left implicit). A
proper rotation preserves winding; reflections are unsupported.

**Closed-shell requirement.** A mesh may receive volume and area quantities only if all hold: every
vertex index is in range; every triangle has three distinct indices and nonzero area; every
undirected edge belongs to exactly two triangles, used once in each direction; the signed volume
`(1/6) * sum(v0 . (v1 x v2))` is positive (outward). For one genus-0 shell `V - E + F = 2`; for `k`
such shells it is `2k`. Euler's formula alone is not a closure test (`F06-NEG-18` has `V - E + F =
1`, `F06-NEG-19` has 4 for two shells, and an inward mesh has 2). The box builders
also self-check this before sealing (`GEO_SELF_CHECK_FAILED`). The results of the operations of section 2 are additionally certified free of self-intersection by the numeric certificate
before they are sealed; for them genus may exceed zero and the shell count stays 1.

**Canonical box mesh.** Both bodies are boxes in a right-handed body
frame with origin corner `o` and edge vectors `e1, e2, e3` (`e1 x e2` along `e3`): plate `o = p0`,
`e1 = p1-p0`, `e2 = p3-p0`, `e3 = thickness*Z_p`; member `o = (p1 - (w/2)*X_m) - (h/2)*Y_m`, `e1 =
w*X_m`, `e2 = h*Y_m`, `e3 = p2 - p1`. Vertex `i` (`xb = i & 1`, `yb = (i >> 1) & 1`, `zb = i >> 2`)
is `((o + xb*e1) + yb*e2) + zb*e3` component by component. The 6 faces, in table order, are quads of
vertex indices wound counter-clockwise from outside:

| Face index | Local id token | Quad |
|------------|----------------|------|
| 0 | `f.x0` | 0, 4, 6, 2 |
| 1 | `f.x1` | 1, 3, 7, 5 |
| 2 | `f.y0` | 0, 1, 5, 4 |
| 3 | `f.y1` | 2, 6, 7, 3 |
| 4 | `f.z0` | 0, 2, 3, 1 |
| 5 | `f.z1` | 4, 5, 7, 6 |

Quad `[a, b, c, d]` gives the triangles `(a, b, c)` and `(a, c, d)`; triangle `2*f` and `2*f + 1`
belong to face `f`. Counts: 8 vertices, 12 triangles, 6 faces, 12 topological edges, 18
triangle-mesh edges (12 box edges and 6 face diagonals), 1 shell; `8 - 18 + 12 = 2` and `8 - 12 + 6
= 2` (`F06-POS-01`).

**Mesh blob.** One body's mesh is one blob. All integers are **little-endian**, all positions are
little-endian binary64 with negative zero normalized.

| Offset | Size (bytes) | Content |
|--------|--------------|---------|
| 0 | 8 | magic, ASCII `BCMESH01` |
| 8 | 4 | layout version, `1` |
| 12 | 4 | vertex count `V` |
| 16 | 4 | triangle count `T` |
| 20 | 4 | face table entry count `K` |
| 24 | 4 | space: 0 element-local, 1 world |
| 28 | 4 | reserved, zero |
| 32 | `24*V` | positions `x, y, z` per vertex |
| next | `pad8(12*T)` | indices, 3 u32 per triangle, zero padding to a multiple of 8 |
| next | `pad8(2*T)` | per-triangle face index (u16, an index into the face table), zero padding |

`pad8(n)` rounds `n` up to a multiple of 8. The total is `32 + 24*V + pad8(12*T) + pad8(2*T)`; the
box body is 392 bytes (`F06-POS-02`). Section starts are multiples of 8 relative to the blob
start, but a blob's start in memory need not be aligned, so consumers read with unaligned loads or
copy. The face table (an ordered array of at most `K` local ids) travels in
the JSON descriptor, not in the blob. The encoded descriptor of one body (face table and fields) is
at most 2,097,152 bytes of UTF-8, and a page ends when any of its budgets would be exceeded
(section 10).

**Primitive limits and general validator limits.** The box builders (plate and member)
always produce exactly 8 vertices, 12 triangles, 6 faces, 12 topological edges, 18 triangle-mesh
edges, 1 shell and 26 features. These **primitive limits** are constants of the builders, not caps.
The **hard caps** below are the general structural-validator limits for any blob (including a future
body or a blob received from outside); a blob inside them is structurally valid, not thereby an
engineering result (section 8).

**Hard caps.**

| Item | Cap |
|------|-----|
| Vertices per body | 262,144 |
| Triangles per body | 524,288 |
| Faces per body (topological) | 65,535 (the per-triangle face index is a u16) |
| Topological edges per body | 786,432 (3/2 of the triangle cap) |
| Shells per body | 16 (box bodies produce 1) |
| Features per body | 1,114,111 (262,144 + 786,432 + 65,535) |
| Mesh blob size | 13,631,520 bytes (the value of the formula at the vertex and triangle caps) |

These caps hold a plate with 64 holes of 1024 sides each.

A validator checks the three counts against their caps before computing any size, computes sizes
with checked 64-bit arithmetic, compares the supplied length with the computed length and only then
reads sections. Zero counts, a wrong magic, an unknown layout version, a nonzero reserved field,
non-finite or negative-zero coordinates, indices out of range and a face index outside the table are
`GEO_BLOB_INVALID`. Counts above a cap are `GEO_BUFFER_LIMIT` (vertices, triangles, bytes) or
`GEO_TOPOLOGY_LIMIT` (faces, edges, shells, features). Exactly at a cap is accepted (`F06-POS-27`);
one above is rejected (`F06-NEG-11` to `F06-NEG-14`).

**Delivery.** A mesh blob is **never split**: a snapshot or change delivery holds whole blobs. If a
delivery limit is smaller than a body needs, the body is not delivered and the query fails with
`GEO_BUFFER_LIMIT`; it is never fragmented. `F06-POS-28` shows the page arithmetic.

**Digest rule.** The `mesh_digest` is the SHA-256 (RFC 6234) of the exact blob bytes, as 64
lowercase hexadecimal digits. The digest identifies bytes; it is not the shape key (F04). The same shape key,
space and placement give the same bytes under the declared algorithm profile and the same digest
(`F06-POS-02`). A world blob differs from the local blob and has its own digest.

**Element-local versus world space.** The kernel's primary mesh is element-local (space 0), shared
by every element with the same shape key. A world mesh (space 1) is a derived result: `world =
R*local + t` per vertex (section 7). A viewport should keep the local mesh and apply the element
transform when drawing; the transform application for drawing is presentation, while world bounds,
measurements and picks come from the engine.

## 7. Transforms and measurement keys

**Spaces and transforms (rigid only in the first scope).**

| Space | Definition |
|-------|------------|
| Body | The box frame of section 6 where the body is axis-aligned from `o` |
| Element-local | F04 element-local coordinates; kernel meshes are in this space. Plate: `p_local = p0 + [X_p Y_p Z_p]*p_body`. Member: `p_local = o + B * p_body`, with `B = [X_m Y_m Z_m]` |
| World | `p_world = R(q) * p_local + translation_mm` (rotate, then translate; F04) |
| Unit | Lengths in mm, areas mm2, volumes mm3 internally; reports use m2 and m3 by division by 1e6 and 1e9 (section 2) |

The transform is a rigid pose and nothing else: a unit quaternion and a translation, with
determinant +1. Any matrix, scale, shear or reflection is `GEO_UNSUPPORTED_TRANSFORM`. A parent
transform is outside this contract (F04). For USD, F04 authors `metersPerUnit = 0.001` and `upAxis =
Z`, so native results are millimetres and Z up and the stage conversion is the identity. **Imported
meshes with another `metersPerUnit` or `upAxis` are unsupported until the R-tasks**
(`GEO_UNSUPPORTED_STAGE_FRAME`); in 0.1.0 no imported mesh is accepted as kernel input at all
(`GEO_UNSUPPORTED_MESH_SOURCE`), and the kernel never guesses a conversion.

**Canonical numeric encoding for keys.** Key preimages are byte
strings: ASCII tags, unsigned integers big-endian, and every f64 as 8 bytes big-endian binary64
after normalizing negative zero. NaN and infinity never occur (rejected earlier). Decimal text,
locale and JSON formatting are never part of a preimage. Keys are SHA-256 digests shown as 64
lowercase hexadecimal digits; they are not content digests of stored bytes.

**Three separate dependency keys.** The shape and world-measurement preimages share the `algorithm`
and `tolerance_policy` fields (`00 03 00 03` for their two u16 values). Changing either version intentionally invalidates both keys. Their remaining fields
are disjoint: shape has body kind, ordered plate contour/thickness or member axis, roll, section
kind and dimensions, `feature_id_rule` and `mesh_layout`; world has the shape-key digest,
translation and canonical quaternion. World therefore also depends transitively on every shape
input. Their existing domain tags, `BCGEO-SHAPE` + `0x00` and `BCGEO-WORLD` + `0x00`, make the
complete preimage byte strings distinct across these two domains; this does not promise that SHA-256
itself is collision-free. Presentation inputs (resolved appearance digest, visibility and camera)
are disjoint from the geometry inputs; their preimage is the viewport's.

1. **Shape key**: the element-local geometry. Preimage: `BCGEO-SHAPE` + `0x00`, a body kind byte (1
   plate, 2 member), then for a plate a u16 point count, each ordered point's three f64 components
   and f64 thickness; for a member the six f64 of `p1, p2`, f64 `roll_rad` (omitted means zero), a
   section kind byte (1 rectangle) and f64 width/height; then four u16 big-endian: `algorithm`,
   `tolerance_policy`, `feature_id_rule`, `mesh_layout` (3, 3, 1, 1).
   It excludes placement, density, material, label, appearance, camera, ElementId and the profile
   resource digest (two profiles with the same dimensions give the same geometry; the digest stays
   in the F04 footprint instead). A body with operation records uses a distinct body kind byte and
   appends its operations in ascending ordinal order (kind byte, u16 ordinal, canonical f64
   parameters); its byte-exact layout is fixed and recorded here before any such key is stored.
2. **World-measurement key**: everything measured in world space. Preimage: `BCGEO-WORLD` + `0x00`,
   the 32 raw bytes of the shape key, the three f64 of `translation_mm`, the four f64 of the
   canonical quaternion (section 4), then u16 `algorithm` and `tolerance_policy`. The version pair
   is (3, 3). It excludes density, label, appearance and camera.
3. **Presentation key**: how an element is shown. Inputs: the resolved display appearance reference
   (digest), visibility and the camera description of the view. It contains no shape, placement or
   physical-material input. It is computed by the presentation layer and
   never by the kernel. Selection and hover are viewport state outside all three keys and are never
   engineering input.

**Mass signature.** Mass is a quantity with its own input signature (F04), not a fourth
key: SHA-256 of `BCGEO-MASS` + `0x00`, the shape key, a presence byte (0 or 1) and the f64 density
(0 when absent), then u16 `algorithm` (3). L and eps_geom are deterministically derived from
shape inputs; no independent scale field is added to any preimage.

**Invalidation matrix.** "Inval." marks a changed key or signature; "-" marks an unchanged one.

| Change | Shape key | World key | Presentation key | Mass signature | Local mesh bytes | World bounds |
|--------|-----------|-----------|------------------|----------------|------------------|--------------|
| Rename | - | - | - | - | - | - |
| Camera change | - | - | Inval. | - | - | - |
| Selection, hover | - | - | - | - | - | - |
| Visibility or appearance change | - | - | Inval. | - | - | - |
| Density value change | - | - | - | Inval. | - | - |
| Placement change | - | Inval. | - | - | - | Inval. |
| Contour/thickness, axis, roll or profile-dimension change | Inval. | Inval. | - | Inval. | Inval. | Inval. |
| Hole or plane-cut operation added, changed, removed | Inval. | Inval. | - | Inval. | Inval. | Inval. |
| Copy (new ElementId, same inputs) | - | - | - | - | - | - |
| Algorithm or tolerance-policy version change | Inval. | Inval. | - | Inval. | Inval. | Inval. |

Reading the rows: a rename or a camera change does not invalidate shape (F04); a density
change invalidates mass only; a placement change invalidates world measurements but not shape
(`F06-POS-19` to `F06-POS-24`). A copy has the same keys as its source; only its FeatureIds differ
(`F06-POS-26`). Changing the material reference to a definition with the same density and the same
resolved display appearance changes nothing visible; changing only the density value changes the
mass signature and not the presentation key, because the presentation key holds the resolved display
reference, not the material. F04's wording "material-dependent appearance" is read this way.

**Quantity signatures.** Volume, surface area, member length and local bounds carry the shape key.
World bounds carry the world-measurement key. Mass carries the mass signature. A quantity is
`current` only if its signature matches the key computed from the current authored inputs.

## 8. Quantities

Each quantity record has value (if any), unit, status, input signature and algorithm version (F04
section 5) and, for a volume taken from an enclosure, the `enclosure_width`. Status values are exactly F04's: `current`, `stale`, `unknown`, `unsupported`.

| Quantity | Unit | Definition for the first scope | Required inputs |
|----------|------|--------------------------------|-----------------|
| Volume | m3 (mm3 internally) | Rectangle contour `width*length*t` with width=norm(p1-p0), length=norm(p2-p1); member `w*h*ell`. Other supported contours and bodies with polygonal holes or cavities: net polygon area times thickness (exact); a sealed plane-cut or other certified closed shell: the midpoint of the enclosure (below) | Valid shape |
| Surface area | m2 (mm2 internally) | `2*(a*b + a*c + b*c)` over the three box edge lengths (box bodies only; not defined for operation bodies in 0.1.x) | Valid shape |
| Local bounds | mm | Minimum and maximum of the eight vertices in element-local space (plate: p0 plus the contour-derived box vectors); box bodies only | Valid shape |
| World bounds | mm | Minimum and maximum of the eight vertices after the rigid transform; box bodies only | Shape and placement |
| Member length | mm | `ell = sqrt((dx*dx + dy*dy) + dz*dz)` of `d = p2 - p1` (members only); distinct from tolerance L | Valid shape |
| Mass | kg | Volume in m3 times density in kg/m3 | Current volume and an explicit density |

Bounds and vertices follow the element-local frames of F04. A valid contour outside the supported
evaluation (not in the plane `z = 0`) returns unsupported without quantities or partial geometry;
it does not receive bounding-box volume, mass, box features or a shape key. The authored element
remains valid.

**Status rules.**

- `current`: computed analytically by the box builders, or by the sealed
  operations and extrusions of section 2 under the rule below, from a sealed build whose signature
  matches the current authored inputs.
- `stale`: authored inputs changed and the rebuild is not complete; the previous value may be
  retained, clearly marked, and is never presented as current (`F06-POS-38`).
- `unknown`: a required input is absent or unavailable. **Unknown density keeps mass unknown** even
  when volume is current; density is never defaulted, guessed from a catalogue or taken as zero when it is absent; an explicit
  zero density gives a `current` mass of zero (amended 2026-10-30, [density amendment](f04-material-density-amendment-v0.1.md))
  (`F06-POS-03`, `F06-POS-13`). A dependency that cannot be read gives `unknown`
  (`DEPENDENCY_UNAVAILABLE` at the command level). An element in the `degenerate` state has every
  derived quantity `unknown`.
- `unsupported`: geometry outside the scope (a true curved surface, a body of no supported class) or
  a mesh that is open, wound inward
  or not certified free of self-intersection (`F06-NEG-18` to `F06-NEG-20`). No value.
- Precedence for mass: `unsupported` if volume is unsupported; else `unknown` if density is absent;
  else `stale` if volume or density is stale; else `current`. Mass of an unsupported body is
  `unsupported` even when density is absent, because `unsupported` describes the geometry and
  supplying a density could never make the mass current; this keeps F04's statuses unchanged and is
  why `F06-NEG-18` expects mass `unsupported`.

**Volume of a certified closed shell.** For a closed shell that
is certified (closed, outward, manifold, free of self-intersection, numeric certificate passed) the
kernel computes an enclosure `[lo, hi]` of the exact signed-volume sum `(1/6) * sum(v0 . (v1 x
v2))` of its triangles. The volume is the midpoint of that enclosure, with status `current`, and
the quantity record reports the interval width `hi - lo` as `enclosure_width` in the same unit
(zero for an analytic value). The width is rounding only and is never hidden. For a body whose
shape is a polygon extrusion with polygonal holes or cavities the polygon volume is exact (net area
times thickness, as stated in the table); the certified enclosure must contain it, otherwise the
build fails as `GEO_SELF_CHECK_FAILED`. A shell without the certificate gets `unsupported` as
before. Mass uses the volume like any other (precedence above). This supersedes, for certified
sealed results only, the 0.1.0 rule that no mesh quantity is exact.

**Bounds are not volume.** A bounding box is never reported as volume, mass or any exact quantity,
even when it happens to have the same size as an axis-aligned body.

**Open or self-intersecting meshes never receive exact status.** Every engineering quantity comes
from the analytic builders or from a sealed result of the operations of section 2
whose shell passed the certificate. A blob from any other source gets no engineering quantity at all,
whether or not it passes structural validation; the certificate exists only inside the kernel's own
sealing step and is never applied to a received blob. (`F06-NEG-19`: two closed boxes overlapping
give a divergence sum of 2000 mm3 for
a union of 1500 mm3, so the sum is never reported; the certificate refuses that mesh.)

## 9. Query API (logical)

All queries are read-only and bounded. They name no transport, no USD path and no file path. A query
is pinned to one project, workspace, base revision and workspace generation and returns a
`snapshot_token`: an opaque string of at most 256 bytes,
compared for equality only (ordering and structure are never inferred). A result is answered at
exactly one pinned revision; data from two revisions is never mixed. An unknown, expired or
older-than-retained token is `GEO_STALE_TOKEN` (`STALE_WORKSPACE`). A successful result is `ok`; a
failed one carries an F04 code and bounded diagnostics (section 3) with no partial data. Request
schemas are closed (unknown field: `UNSUPPORTED_PROPERTY`).

| Query | Request fields | Result |
|-------|----------------|--------|
| `QueryBounds` | `scope` (`element` with an ElementId, or `all`), `space` (`local` for one element or `world`) | Bounds with status and key; with scope `all`, paged like `ListElements` |
| `ListElements` | `cursor` (null or string), `page_size` | Elements in ascending canonical ElementId order: id, kind, shape key, world key, presentation key, mesh digest, counts, blob size |
| `GetMeshSnapshot` | `cursor`, `space` | Pages of whole blobs plus descriptors with face tables, and the `snapshot_token` |
| `LookupFeature` | `feature_id` | Feature kind, owning ElementId, role, geometry in local and world space (face: outward unit normal and four vertex ids; edge: two vertex ids; vertex: position). For the kinds `extrusion` and `cut` a face reports its outward unit normal and the ordered vertex ids of its polygon, and `LookupFeature` of an operation-created edge or vertex is `GEO_FEATURE_NOT_FOUND` |
| `Pick` | Ray form or point form below | Ordered hits |
| `GetChanges` | `since_token` | Changed, added and removed elements with their keys; blobs for those whose shape key changed; new token; or `resnapshot_required` |

**Pick contract.**

- *Ray form:* `origin_mm` (3 finite numbers, absolute value at most 4,000,000,000), `direction` (3
  finite numbers, absolute value at most 1,000,000,000, Euclidean norm at least 1e-6; normalized by
  the engine), `t_max_mm` (default and maximum 4,000,000,000), `targets` (a nonempty subset of
  `face`, `edge`, `vertex`), `radius_mm` (default 0, maximum 1,000,000), `max_hits` (default 16, 1
  to 64) and `include_backfaces` (default false). The bound 4e9 covers every world coordinate of an
  admitted body: a local point has magnitude at most about 1.8e9 and the translation components are
  at most 1e9, so each world component stays below 3.5e9.
- *Point form:* `point_mm` (as `origin_mm`), `radius_mm`, `targets`, `max_hits`.
- *Face hits (ray):* faces are closed rectangles in their planes, enlarged by the pick band
  `eps_pick = 0.001 mm` (one micrometre) in each in-plane direction. With the unit ray
  direction `u` and the outward unit normal `n`, a face is **parallel** to the ray, and not hit,
  when `|u . n| < 1e-6`. Otherwise `t = ((p0 - o) . n) / (u . n)` for a point `p0` of the face and
  the ray origin `o`; the face is hit when `0 <= t <= t_max` and the in-plane coordinates of the hit
  point lie in the enlarged rectangle. A face is front-facing when `u . n < 0`; back-facing faces
  are skipped unless `include_backfaces` is true. A hit that lies on the exact rectangle's boundary
  or in its enlargement has `boundary` true, and a hit on a shared edge is reported for every
  non-parallel face that qualifies (`F06-POS-43`, `F06-POS-44`). A non-finite intermediate value is
  `GEO_NUMERIC_UNCERTAIN`, with no partial hits.
- *Edge and vertex hits:* ray candidates use the same closed finite segment as faces: `S = {o + t*u
  : 0 <= t <= t_max}` with normalized direction `u`. The distance is the minimum Euclidean distance
  between a point of `S` and a point of the closed feature (an edge segment or a vertex), never the
  distance to the unbounded ray. For a vertex `v`, the closest segment parameter is `t = min(t_max,
  max(0, (v - o) . u))`; for an edge, minimize over both closed segments, including their endpoints.
  Report the minimizing segment parameter `t`; if several minimizing pairs exist, use the smallest
  such `t`. Accept exactly when the minimum distance is at most `radius_mm + eps_pick`. This closed
  distance band also applies at both segment ends: a nearby feature beyond an end may qualify by
  distance to that endpoint, but `t` is clamped to the endpoint and the parameter interval is never
  extended. Face hits keep their exact parameter test `0 <= t <= t_max` above. With radius 0 the
  edge/vertex endpoint band is inclusive at `eps_pick = 0.001 mm`; a farther feature is rejected
  (`F06-POS-47`, `F06-NEG-36`). In point form, use the minimum distance from the query point to the
  closed edge or vertex, with the same inclusive distance threshold and no ray parameter. Zero
  radius accepts a touching feature up to the band (`F06-POS-32`).
- *Point form distances:* the distance to a face is the Euclidean distance to the nearest point of
  the closed face rectangle (the foot of the perpendicular when it lies inside, otherwise a point of
  the rectangle's boundary); a point inside the solid still measures to surfaces, never zero. A face
  is accepted when that distance is at most `radius_mm + eps_pick`, front or back (`F06-POS-33`,
  `F06-POS-45`).
- *Hit record:* `element_id`, `feature_id`, `feature_kind`, `t_mm` (ray form), `distance_mm`
  (distance to the feature; 0 for a ray hit on a face), `point_world_mm` (the hit or closest point),
  `normal_world` (outward unit normal for a face, null otherwise), `back_facing` and `boundary`
  (faces; false otherwise). A hit never contains a triangle, vertex or buffer index.
- *Ordering and tie-breaking:* sort ascending by the tuple (`k`,
  ElementId, kind rank, local id, `t_mm`), where `k = floor(t_mm / 0.001)` for the ray form
  (`floor(distance_mm / 0.001)` for the point form), the ElementId and local id compare as ASCII
  strings and the kind ranks are face 0, edge 1, vertex 2. Quantizing first makes the order a pure
  function of the inputs; hits whose `t` differs by less than one micrometre (`0.001 mm`, the same
  quantum as `eps_pick`) may therefore be ordered by identity instead of by `t`. At most `max_hits`
  records are returned; if more qualify, the first `max_hits` of this order are returned and a
  `truncated` flag is true.
- *Tolerance:* `radius_mm` is the caller's query parameter. It is not the geometric tolerance of
  section 4 and not the test comparison rule CR-TOL-001. A viewport that converts pixels to a radius
  does so before calling; the engine uses the number it receives.
- *Limits:* `max_hits` above 64 or `radius_mm` above 1,000,000 is `GEO_PICK_LIMIT`; a zero or
  too-short direction, an empty or duplicated `targets`, a negative radius or `t_max_mm` is
  `GEO_PICK_INPUT_INVALID`; a non-finite number is `GEO_NON_FINITE`.
- *Work and deadline:* a `Pick` considers at most 65,536 elements and at most 26 features
  per element; a model with more elements fails before the scan with `GEO_PICK_LIMIT` and no partial
  hits (`F06-NEG-33`). The scan visits elements in ascending ElementId order and polls the
  cancellation token and the request deadline every 1,024 elements. A cancelled query returns
  `CANCELLED` and a missed deadline `OUTCOME_UNKNOWN`; neither returns partial hits. Only a
  completed scan returns hits.

**Incremental update.** `GetChanges` answers from a bounded retained history. It lists at most
4,096 entries per page and at most 1,024 pages per delivery; an entry reports the element,
the kind of change (`added`, `changed`, `removed`), the three keys and, only when the shape key
changed, the new blob. A placement-only edit therefore resends no mesh (`F06-POS-34`); a dimension
edit resends the blob (`F06-POS-35`). A token the engine cannot answer gives the outcome
`resnapshot_required` with `STALE_WORKSPACE` and no partial list (`F06-POS-36`); the consumer then
reads a fresh snapshot. A delivery that would exceed the delivery caps fails before dispatch instead
of being truncated silently.

**The renderer never computes engineering geometry.** Bounds, quantities, normals used for
measurement, feature geometry, pick results and FeatureIds come from the engine. A viewport may
upload meshes, apply the element transform for drawing, compute display-only shading normals from
the supplied mesh and run a display-only GPU pick as a hint. Anything that selects an element or
feature for engineering use is resolved by the engine's `Pick` or `LookupFeature`; a display hint is
mapped to a FeatureId only through the snapshot's face table.

**Headless surface (logical only).** Every query has an engine API first and a CLI command with the
same fields and the same logical result: build and mesh, quantities, bounds, element list, feature
lookup, pick and changes. The CLI adds no computation and no private path to geometry (`AGENTS.md`
section 3; `F06-POS-39`). How the CLI delivers mesh bytes, for instance to a file, is an I/O-edge
decision of the CLI task; it does not alter the logical result.

## 10. Topology and bounds limits

| Item | Rule |
|------|------|
| Counts | Vertices, triangles, faces, topological edges, shells and features are capped by the table in section 6 |
| Arithmetic | Counts are compared with their caps before any product; sizes use checked 64-bit arithmetic; any overflow is `GEO_BUFFER_LIMIT` |
| Allocation | Sized only by validated input: the box builders allocate for exactly 8 vertices, 12 triangles and 6 faces; a blob validator allocates only after the length check |
| Recursion | None. A later traversal states its own depth bound, at most 16 |
| Input size | At most 3081 numbers for a plate or 17 for a member (section 2); at most 1024 contour points |
| Operation bodies | No designed limit on holes or cuts per element; counts stay inside the section 6 caps. Kernel budgets (certification, exact-sign, pair and operation limits) are implementation limits whose numeric values live in code; exceeding one is `GEO_KERNEL_CAPACITY` (section 2, "Operations") |
| Primitive versus validator limits | Primitive counts are exact constants (section 6); the hard caps bound any blob a validator accepts |
| Descriptor bytes | At most 2,097,152 encoded bytes per body descriptor |
| Query output and work | At most 64 hits per pick; at most 65,536 elements and 26 features per element considered; cancellation and deadline polled every 1,024 elements; at most 4,096 entries per page and 1,024 pages per delivery |
| Delivery limits | A body that cannot be delivered whole fails with `GEO_BUFFER_LIMIT`; it is never split or truncated (`F06-NEG-34`). F06 never widens an F04 limit |

**Cancellation points and deterministic result.** The kernel polls a host-supplied cancellation
token at these points only: P0 on entry, P1 before the build, P2 before the checks and P3 before the
quantities. The seal point follows the quantities; a result is sealed there. A request observed at
P0 reports stage `validation`; at P1 stage `build`; at P2 stage `checks`; at P3 stage `quantities`
(the stage that would have run next). Work between two points is bounded by a fixed number of
operations, so cancellation latency is bounded. A cancellation observed at or before P3 gives status
`cancelled`, `GEO_CANCELLED`, and no mesh, quantities or key: the partial work is discarded. A
cancellation after the seal point is a no-op and returns the complete result (`F06-POS-37`). For a
fixed input and a fixed observing point the outcome is the same every time (`F06-NEG-16`,
`F06-NEG-17`). Cancellation never changes engineering state; `CANCELLED` is reported only when the discard is
proven.

## 11. Acceptance cases

The [case file](../../tests/fixtures/contracts/f06-geometry-contract-cases-v0.1.json) holds 112
logical cases: 59 positive (`F06-POS-01` to `F06-POS-59`) and 53 negative (`F06-NEG-01` to
`F06-NEG-53`). Acceptance needs a real kernel, real buffers and independent readers. Numeric
expectations use exact forms or decimal approximations checked with rational arithmetic; the
derivation is in each case. Numeric comparison follows the test rule of F04 section 10; statuses,
codes, counts, FeatureIds and digests compare exactly. F04 cases are referenced, not duplicated.

| Topic | F06 cases | Existing cases referenced |
|-------|-----------|---------------------------|
| Plate and member meshes: counts, Euler, closed shell, orientation | `F06-POS-01`, `F06-POS-04` to `F06-POS-08` | `F04-POS-14` to `F04-POS-16` |
| Blob bytes and digest | `F06-POS-02` | none |
| Quantities, mass, unknown density | `F06-POS-03`, `F06-POS-13`, `F06-POS-38` | `F04-POS-01` |
| Rotated placement | `F06-POS-11`, `F06-POS-12` | `F04-POS-10` |
| FeatureId stability, uniqueness, grammar | `F06-POS-16` to `F06-POS-18` | `F04-POS-03` |
| Three keys and invalidation matrix | `F06-POS-19` to `F06-POS-26` | `F04-POS-02`, `F04-POS-06` |
| Cap boundaries, exactly at limit accepted | `F06-POS-14`, `F06-POS-15`, `F06-POS-27` | `F04-POS-08`, `F04-POS-13` |
| Cap boundaries, just above rejected | `F06-NEG-06`, `F06-NEG-11` to `F06-NEG-14` | `F04-NEG-19`, `F04-NEG-21` |
| Degenerate axis, minimum dimension | `F06-NEG-03` to `F06-NEG-05` | `F04-NEG-20` |
| Parallel-up boundary, near-vertical frame, frame certificate | `F06-POS-05`, `F06-POS-06`, `F06-POS-09`, `F06-POS-10`, `F06-POS-41`, `F06-POS-42`, `F06-POS-46`, `F06-NEG-31`, `F06-NEG-35` | `F04-POS-16` |
| Non-finite input | `F06-NEG-01`, `F06-NEG-02`, `F06-NEG-27` | `F04-RAW-01`, `F04-RAW-02`, `F04-RAW-04` to `F04-RAW-06` |
| Buffer cap exceeded, invalid blob | `F06-NEG-11` to `F06-NEG-15`, `F06-NEG-30` | none |
| Cancellation | `F06-POS-37`, `F06-NEG-16`, `F06-NEG-17` | none |
| Unsupported profile, transform, stage frame (`F06-NEG-08` superseded, see below) | `F06-NEG-08` to `F06-NEG-10` | none |
| Open, self-intersecting, inward mesh not exact | `F06-NEG-18` to `F06-NEG-20` | none |
| Failure keeps previous result | `F06-NEG-21` | `F04-NEG-14` |
| Queries: pick, lookup, incremental, chunking | `F06-POS-28` to `F06-POS-36`, `F06-POS-39`, `F06-POS-40`, `F06-POS-43` to `F06-POS-45`, `F06-NEG-22` to `F06-NEG-29`, `F06-NEG-32` to `F06-NEG-34` | `F04-NEG-09`, `F04-NEG-15` |
| Finite pick segment: endpoint hit and short-horizon rejection | `F06-POS-47`, `F06-NEG-36` | none |
| Contour rectangle rotation/start | `F06-POS-48`, `F06-POS-49` | `F04-POS-18`, `F04-POS-19` |
| Invalid contours and valid unsupported pentagon (`F06-NEG-39` superseded, see below) | `F06-NEG-37` to `F06-NEG-42` | `F04-NEG-28` to `F04-NEG-32`, `F04-POS-20` |
| Explicit axial roll, default and near-vertical | `F06-POS-50` to `F06-POS-52` | `F04-POS-21` to `F04-POS-24` |
| Non-finite/out-of-cap roll | `F06-NEG-43`, `F06-NEG-44` | `F04-RAW-07`, `F04-NEG-33` |
| Local-size floor, small-body invariance, planarity, cap/fault and absolute pick band | `F06-POS-53` to `F06-POS-59`, `F06-NEG-45` to `F06-NEG-53` | `F04-POS-25` to `F04-POS-30`, `F04-NEG-34` to `F04-NEG-39` |

The invalid-quaternion, translation-limit and density cases are already covered by `F04-NEG-22` to
`F04-NEG-26` and are not repeated. The file also carries a
`cross_references` list of these topics.

**Superseded cases.** `F06-NEG-39` (a valid pentagon must be `GEO_UNSUPPORTED_CONTOUR`) no longer
holds: any simple polygon in the plane `z = 0` is supported. `F06-NEG-08` (a curved profile is
`GEO_UNSUPPORTED_PROFILE`) no longer holds: the section families, including round ones, are
supported as polygons. Both stay in the file with a `superseded` marker and are not acceptance
targets. Cases for holes, plane cuts, polygonal cavities, round-hole polygons, the `extrusion` and
`cut` FeatureIds, the volume enclosure and `GEO_KERNEL_CAPACITY` are still to be written.

## 12. Version

This is version **0.1.0**: `algorithm` 3, `tolerance_policy` 3, `feature_id_rule` `fid1`,
`mesh_layout` 1. A change that alters an output byte, a key or a decision for an input already
evaluated raises the affected version as section 2 says.

