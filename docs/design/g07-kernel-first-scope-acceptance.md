# G07: acceptance of the first scope of the own geometry kernel

Date: 2026-10-17. Status: **acceptance record. Overall verdict: ACCEPTED WITH CONDITIONS.**
Base: `43026e26e631c0999dec55d56917981adb8c30a6` (G02, G03, G05, G06, G09, P03 and A04 are merged).
Task: G07, implementation plan section 7.3 (depends on G05, G06, G09). Reviewer: independent QA that
did not write the kernel and judged it on the evidence below. G08 (integrating the kernel into engine
commands) is a separate task and was not started.

The kernel in question is the `beamcraft-kernel` crate (plus the `beamcraft-geom` mathematics it
uses). It is pure Rust, depends only on `beamcraft-geom` and `beamcraft-model`, contains no `unsafe`
(`unsafe_code = "forbid"`) and no third-party geometry crate (`cargo tree -p beamcraft-kernel`
prints those two path dependencies and nothing else). No third-party backend is chosen by this
document.

## 1. Verdicts

| Criterion | Verdict | Basis |
|---|---|---|
| Correctness of the supported operations is shown by fixtures and meaningful tests | ACCEPTED WITH CONDITIONS | Section 5. 213 kernel tests, the independent G03 harness and the Python fixture check pass. Every supported class has independent oracles (closed forms, voxel counting, divergence-theorem volume, independent edge maps and blob readers, exact-integer references). Conditions: findings F2, F3, F5 |
| Speed and bounded work | ACCEPTED WITH CONDITIONS | Section 7. Every measured workload, including inputs at the F06 caps and adversarial ones, ended with a result or a typed refusal in under about 0.6 s per kernel call on the measuring machine; no hang was seen. Condition: certification dominates the cost and its default budgets end it at a few thousand triangles at most (a 2,496-triangle slab certifies, a 4,352-triangle one does not; a plate stops after 5 holes of 64 edges, 1,312 triangles; finding F2); no performance promise is made |
| List of supported operations with exact input classes | ACCEPTED | Section 3 |
| Everything unknown is explicitly unsupported | ACCEPTED WITH CONDITIONS | Section 4. Open, non-manifold, misoriented, inward, degenerate, self-intersecting, overlapping and undecidable meshes never receive an exact status or an engineering quantity (tests in section 5). Condition: queries, keys, placement and analytic area have no code path at all; they are unsupported by absence, and G08 must keep returning typed unsupported results for them |
| No third-party backend is chosen | ACCEPTED | Dependency tree above; no geometry crate in `Cargo.lock`; no copied third-party string found in the kernel sources, tests or example. |
| Contract coverage of holes, cuts and curved cavities | NOT ACCEPTED | These operations are outside the frozen F06 0.1.0 contract (finding F3). They are accepted as a tested pilot only, not as a contracted feature, until an amendment exists |
| Identical bytes on every target | NOT ACCEPTED as a claim | Evidence exists for one target (Windows x86_64) only; the wasm32 and `cargo deny` checks were not available (finding F5) |

Overall: the first scope may go to G08 only under the preconditions in section 9.

## 2. What was run

| Command | Result |
|---|---|
| `cargo fmt --all` | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass (Rust 1.96.0) |
| `cargo test --workspace` | pass: 63 test binaries, 738 tests passed, 0 failed |
| `cargo run -p beamcraft-cli -- --version` | pass |
| `cargo test -p beamcraft-kernel` | 213 passed (202 existing, 11 added by G07) |
| `python tests/g03/write_fixtures.py --check` | `G03: 56 original descriptions; 58 files verified` |
| `cargo run --release -p beamcraft-kernel --example g07_bench`, three processes | the 8 `DIGEST` and all `OUTCOME` and `SIZE` lines are identical in all three runs |
| wasm32 check of `beamcraft-geom`, `beamcraft-model`, `beamcraft-kernel` | not run: the `wasm32-unknown-unknown` target is not installed here (CI covers it) |
| `cargo deny` | not run: not installed here (CI covers it) |

Linux and macOS were not available. No result below says anything about them.

## 3. Supported operations (first scope)

"Supported" means: an engine API exists, a typed result or refusal is defined for every input,
the result is sealed and independently re-read in tests, and the work is bounded. All lengths are
millimetres, right-handed, Z up, element-local. The CLI commands and the engine/CLI end-to-end
gate are owned by P01, P03, A03 and A04 (and G08), not by the kernel.

| ID | API | Exact input class | Result | Bounds |
|---|---|---|---|---|
| S1 | `rectangular_plate`, `_with_cancel` | Three finite dimensions; each greater than zero, at least `eps_geom = max(0.001, 2^-22 * D)` with `D = sqrt(width^2 + length^2)` (thickness is excluded from D), at most 1,000,000 | Origin box `[0,w] x [0,l] x [0,t]`: 8 vertices, 12 triangles, 6 faces, 392-byte BCMESH01 blob, SHA-256, analytic volume `(w*l)*t`, local FeatureIds | Fixed 36-edge self-check, four cancellation points |
| S2 | `straight_member` | Finite axis points (each coordinate at most 1e9 in absolute value), axis length and local size at most 1,000,000 and at least `eps_geom`, centred rectangle section (each side at most 1,000,000 and at least `eps_geom`), optional finite roll of at most 2*pi in absolute value | Same sealed box type in the certified `MemberFrame` (default horizontal or near-vertical frame, certificate, roll), analytic volume `(w*h)*length` | As S1. Roll other than exactly zero uses the platform `sin` and `cos` |
| S3 | `profile::{Profile::admit, triangulate, extrude, extrude_profile}`, `Cavity::{polygon, circle, slot, ellipse}` | One counter-clockwise simple outer polygon and up to 256 clockwise simple cavities strictly inside it and apart from each other by at least `eps_geom` (3 to 1024 points each, 2048 in total, coordinates at most 1e9, edges at most 1e6, thickness in `eps_geom` to 1e6, local size at most 2e6). Curved cavities are polygons on the curve with a proven sagitta bound | Sealed closed mesh straight along +Z (genus = number of cavities), BCMESH01 blob, exact polygon volume (`Approximated` with a bound for curves), provisional `extrusion` ids | Exact-sign budget 150,000,000 evaluations, F06 mesh caps checked before allocation, four cancellation points |
| S4 | `TriMesh::{from_blob, from_buffers, transformed}`, `topology::{analyze, analyze_with, analyze_with_tolerance, classify_point}`, `measure::{measure, mass_status, analytic_mass_kg}`, `spatial::BoxIndex` | Any finite vertex and index buffers or BCMESH01 blob within the F06 caps (4096 vertices, 8192 triangles, 1024 faces, 16 shells, 213,024 bytes, coordinates at most 4e9) | A verdict. Only `Certified` (closed, 2-manifold, consistently oriented, outward, non-degenerate, free of self-intersection, nesting and contact) yields volume, area and centroid enclosures; these are mesh values with quantity status `unsupported`, never engineering quantities | Default `Limits`: 2^20 candidate pairs, 16,384 index nodes, 2^18 exact fallbacks; exceeding one gives `Uncertain` with its reason |
| S5 | `ops::Body::{from_plate, from_extrusion, from_mesh}`, `plane_cut`, `plane_cut_with` | A certified, element-local, single-shell body; one finite plane (normal not below 1e-100, components at most 4e9); a kept half-space; an ordinal at most 999. The plane must properly cross the body once with one outer section loop | Sealed, re-certified body with a planar cap; the largest plane deviation of the rounded new vertices is certified and returned | `OpLimits::DEFAULT`: 8192 triangles, 64 loops, 2048 cap points, 2^18 exact fallbacks, plus the S4 limits |
| S6 | `through_hole`, `through_hole_with` | A certified single-shell Z-prism body (plate, extrusion, a cut result that is still a prism); one cutter cavity strictly inside the outline and clear of earlier holes by at least `eps_geom`, reaching past both caps by at least `eps_geom` | Sealed body with genus plus one, mesh volume agreeing with the analytic volume | As S5. One cutter per call; further holes are further calls on the result |

Everything above holds for any combination permitted by the typed API. Anything not in the input
class is refused by a typed failure carrying no geometry (section 4).

## 4. Explicitly unsupported (and how it fails)

| Not supported | How it is reported |
|---|---|
| Open, non-manifold, misoriented, inward, degenerate, self-intersecting, overlapping, touching or undecidable meshes as bodies or for quantities | `ShellStatus` other than `Certified`; `GEO_MESH_NOT_CLOSED`, `GEO_MESH_INWARD`, `GEO_MESH_NOT_CERTIFIED`, `GEO_NUMERIC_UNCERTAIN`; `measure` returns no value and quantity status `unsupported` |
| Healing, welding by position, snapping, repair | Not performed; an unwelded cube is `Open` |
| Foreign or imported meshes as engineering quantities | Always `unsupported`; mass `unsupported` even without density (F06-NEG-18) |
| Placement, rotation and quaternions in the kernel; world-space meshes as bodies | No kernel input; `Body::from_mesh` refuses a world-space mesh |
| Plate from a general or rotated 3-D contour; non-planar contours | No kernel input (finding F4); the 2-D profile module is the only contour code |
| Member profiles other than a centred rectangle; section anchors | No input form |
| F06 section 7 keys, section 9 queries (`Pick`, `LookupFeature`, `GetChanges`, `ListElements`), element list order, stale tokens | Not implemented anywhere |
| Analytic surface area and bounds quantities, rational-interval bounds | Not outputs of the kernel (mesh area and exact vertex extents exist only as mesh values) |
| Nested islands in a profile, holes touching or crossing an outline, oblique, curved or simultaneous cutters, blind holes, holes in non-prism bodies | `GEO_UNSUPPORTED_CONTOUR`, `GEO_CONTOUR_INVALID`, `GEO_OP_UNSUPPORTED` with fixed defect tokens |
| Several outer section loops, self-touching sections, tangent or coplanar planes, planes missing the body | `GEO_OP_UNSUPPORTED` (`plane.tangent`, `plane.coplanar_face`, `section.*`), `GEO_OP_NO_EFFECT` |
| General boolean, offset, fillet, chamfer, bevel, bent or lofted solids, non-straight sweeps | No code |
| Exact curved geometry | Curves exist only as flagged polygon approximations |
| Persistent topological names across operations beyond the provisional `extrusion` and `cut` ids | Ids are provisional and need an F06 amendment before persistence |
| Certified accuracy bounds for cancellation-heavy inputs, subnormal mass, rounded-mesh cross-checks | Open (F06 erratum E-1) |
| Byte identity on any target other than the one measured; byte identity for non-zero roll | Not claimed |

## 5. Traceability

Counts: validation plan rows: 21 PASS, 8 PARTIAL, 14 NOT COVERED (12 have no code in the kernel, 2 are owned by the engine or G08), 1 SUPERSEDED. F06 cases: 61 PASS, 4 PARTIAL, 43 NOT COVERED (38 are engine, host or query obligations; 5 are kernel concerns without code: POS-11, POS-12, NEG-38, NEG-49, POS-56), 3 UNSUPPORTED by construction, 1 DIVERGENCE (NEG-39).

### 5.1 Kernel validation plan (44 rows, all still `planned` in the JSON)

Result vocabulary: PASS (supported by passing tests and fixtures), PARTIAL (part of the row passes,
the rest is stated), NOT COVERED (no code or no test; UNSUPPORTED where the capability does not
exist at all), SUPERSEDED. The plan JSON was removed on 2026-10-23 (it was stale); this table is the evidence record.
Test file names are in `crates/kernel/tests/` unless a crate path is given.

| Row | Stage | Property | Implementing code | Evidence | Result |
|---|---|---|---|---|---|
| V01 | S1 | Euclidean member length and analytic error enclosure | `geom::MemberFrame`, `Tolerance::for_member` | `geom/tests/f06_math.rs` (POS-04/07/08); `g07_acceptance.rs` member mesh | PARTIAL: written binary64 decisions pass; analytic error enclosure of the length is not implemented (A5, E-1 open) |
| V02 | S1 | Input minima, caps and quaternion admission order | `build`, `member`, `geom::tolerance` | `rectangular_plate.rs`; `geom/tests/f06_math.rs`; `g07_acceptance.rs` failure table | PARTIAL: plate and member minima, caps and order pass; quaternion admission is not a kernel input |
| V03 | S1 | Normalized active rigid rotation matches rational reference | none (`TriMesh::transformed` uses `geom::RigidTransform`) | `g06_measure.rs` rigid motion tests; no F06-POS-11/12/25 vector exists | NOT COVERED |
| V04 | S1 | Orientation, dot and cross signs never guessed | `exact`, `profile::exact`, `intersect` | unit tests against an integer reference; `g06_fuzz.rs`; `g05_negative_and_fuzz.rs` | PASS |
| V05 | S1 | Parallel branch and orthogonal member frame | `geom::MemberFrame` | `geom/tests/f06_math.rs`; `g07_acceptance.rs` near-vertical test | PASS |
| V06 | S1 | Gradual underflow and positive mass representability | `measure::analytic_mass_kg` | `g06_measure.rs` analytic_mass_uses_the_written_order_and_never_defaults_density | PARTIAL: underflow is refused, not certified; full-domain accuracy open (A5, E-1) |
| V07 | S1 | Finiteness and unsupported stage precede numerical work | `build` stage order, `member` | `rectangular_plate.rs` f06_neg_01; `g07_acceptance.rs` special-value sweeps | PASS for finiteness; unsupported kinds are UNSUPPORTED by construction (no input form) |
| V08 | S1 | Antipodal quaternion canonical equivalence | none | - | NOT COVERED (no quaternion in the kernel) |
| V09 | S2 | Plate construction matches analytic corners | `build::rectangular_plate` | `g03_fixtures.rs` (22 supported fixtures, independent harness); `tests/g03/write_fixtures.py --check` | PASS |
| V10 | S2 | Straight rectangle extrusion matches endpoint sections | `member::straight_member` | `g07_acceptance.rs` f06_member_cases_... | PASS (added by G07) |
| V11 | S2 | Near-column construction remains a rectangular prism | `geom::MemberFrame`, `member` | `g07_acceptance.rs` near-vertical test; `geom/tests/f06_math.rs` | PASS (added by G07) |
| V12 | S2 | Quad diagonal preserves oriented boundary | `mesh::self_check` | `g03_fixtures.rs` (198 corruption checks); `mesh.rs` corrupted-candidate unit test | PASS |
| V13 | S2 | Feature roles, uniqueness and grammar | `feature_id`; provisional `extrusion`/`cut` ids in `profile`, `ops` | `rectangular_plate.rs` f06_pos_16_17_18; `g05`/`g09` id tests | PARTIAL: local grammar passes; lookup, qualification and copy disjointness are not implemented |
| V14 | S2 | Rounded corners retain certified embedding or fail closed | `mesh::self_check`, `topology`, exact predicates | `g06_fuzz.rs`; `g05_negative_and_fuzz.rs` | PARTIAL: fail-closed behaviour is tested; the interval embedding certificate of A5 is not implemented |
| V15 | S2 | Repeatable build and semantic identity under edits | all builders | `g07_acceptance.rs` sealed_blobs_have_pinned_digests; `g07_bench` run three times, DIGEST lines identical; `rectangular_plate.rs` | PASS on one target (Windows x86_64) |
| V16 | S3 | Analytic volume with correct units | `PlateResult::volume_mm3`, `member` | `rectangular_plate.rs`; `g07_acceptance.rs` | PASS |
| V17 | S3 | Analytic surface area with correct units | none (mesh area enclosures exist only in `measure`, not as engineering quantity) | - | NOT COVERED |
| V18 | S3 | Mass requires explicit current density and volume | `measure::{mass_status,analytic_mass_kg}` | `g06_measure.rs` mass_status_follows_the_f06_precedence | PASS for the kernel part; stale/shape-key part is engine |
| V19 | S3 | Local and world bounds enclose ideal corners | `spatial::Aabb`, `Body` bounds; engine `query::bounds` | `g06_measure.rs` exact vertex bounds | PARTIAL: exact vertex extents only; no rational-interval enclosure, world bounds are engine |
| V20 | S3 | Closedness, Euler, outward winding and two volume methods agree | `mesh::self_check`, `topology::analyze` | `g03_fixtures.rs`; `g06_shells.rs` | PASS |
| V21 | S3 | Rigid motion leaves scalar quantities invariant | `TriMesh::transformed`, exact motions in tests | `g06_measure.rs` exact_rigid_motions...; `g05_profile_extrusion.rs`; `g09_*` | PASS |
| V22 | S3 | Dimension scaling preserves dimensionless ratios | `measure` | `g06_measure.rs` exact_scale_factors_scale_volume_cubically_and_area_quadratically | PASS |
| V23 | S3 | Density changes mass alone | `measure::analytic_mass_kg` | `g06_measure.rs` | PARTIAL: mass arithmetic only; key independence is engine/P04 |
| V24 | S3 | Blob byte accounting and canonical layout | `trimesh`, `mesh::encode` | `rectangular_plate.rs` f06_pos_02; `g06_shells.rs` caps and header tests | PASS |
| V25 | S3 | Every resource cap is checked before work/allocation | `trimesh`, `topology::Limits`, `profile` counts, `ops::OpLimits` | `g06_shells.rs`; `g05_negative_and_fuzz.rs`; `g09_budget_fuzz.rs` | PASS for mesh, profile and operation caps; pick caps are not implemented |
| V26 | S3 | Malformed blobs yield no current result | `TriMesh::from_blob` | `g06_shells.rs`; `g06_fuzz.rs` mutated blobs | PASS |
| V27 | S3 | Nonanalytic/open/inward/overlapping meshes never certify engineering values | `measure::measure` gate | `g06_measure.rs`; `g07_acceptance.rs` unwelded/T-junction/stray/duplicate/folded probes | PASS |
| V28 | S3 | Cancellation discards before seal only | every `*_with_cancel` / `*_with` entry | `rectangular_plate.rs`; `g05`; `g06_shells.rs`; `g09_budget_fuzz.rs` | PASS for build and operations; pick polling not implemented |
| V29 | S3 | Rejected edits preserve current state; accepted edits mark stale correctly | kernel calls are pure and return `Err` without data | engine/P04/P07 tests (not assessed) | NOT COVERED by G07 (engine) |
| V30 | S3 | Analytic ray-to-face picking and front/back classification | none | - | NOT COVERED: UNSUPPORTED (Pick is not implemented anywhere) |
| V31 | S3 | Point/edge/vertex picking uses closed-feature minimum distance | none | - | NOT COVERED: UNSUPPORTED |
| V32 | S3 | Quantized tie ordering and truncated prefix | none | - | NOT COVERED: UNSUPPORTED |
| V33 | S3 | Dependency keys respect their declared input dependencies | none (`kernel::keys` planned) | - | NOT COVERED: UNSUPPORTED |
| V34 | S3 | Query validation, stale tokens and list ordering | none | - | NOT COVERED: UNSUPPORTED |
| V35 | S3 | Real engine API and CLI have identical logical outcomes | engine and CLI for plates, members, plate holes | CLI tests of P01, P03, A04 (not assessed) | NOT COVERED by G07: G08 owns it |
| V36 | S4 | Pilot cuts remain unsupported before a new admission/contract | superseded by G09 `plane_cut`, `through_hole` | `g09_cut.rs`, `g09_hole.rs`, `g09_budget_fuzz.rs` | SUPERSEDED: the row expects cuts to be unsupported; G09 implements a pilot that needs a contract amendment (finding F3) |
| V37 | S2 | Projected near-vertical Y-tilted frame and roll limits | `geom::MemberFrame`, `member` | `geom/tests/f06_math.rs`; `g07_acceptance.rs` | PASS |
| V38 | S1 | Frame residual certificate rejects either-sign orthogonality faults | `geom::frame` certificate | `geom/tests/f06_math.rs` NEG-31/35 injections; `member.rs` stage mapping unit test (G07) | PASS |
| V39 | S3 | Numeric uncertainty discards complete build or query candidate | `mesh::self_check`, `geom` certificates | `mesh.rs` corrupted-candidate unit test; geom injections | PARTIAL: build side only; the query side does not exist |
| V40 | S3 | Pick band, closed boundaries and nearest face points | none | - | NOT COVERED: UNSUPPORTED |
| V41 | S3 | Strict ray-plane parallel threshold and denominator sensitivity | none | - | NOT COVERED: UNSUPPORTED |
| V42 | S3 | Pick work cap, deterministic scan and cancellation/deadline polling | none | - | NOT COVERED: UNSUPPORTED |
| V43 | S3 | Descriptor bytes and every negotiated delivery budget | host | - | NOT COVERED (host) |
| V44 | S1 | Near-threshold frame branch has no unspecified exclusion band | `geom::MemberFrame` | `geom/tests/f06_math.rs` POS-46; `g07_acceptance.rs` | PASS |

### 5.2 Frozen F06 cases (112, every case appears exactly once)

The validation plan references only 83 of the 112 cases. The 29 cases added later
(NEG-37 to NEG-53 and POS-48 to POS-59) are in no plan row (finding F6); they are traced here.
Cases that F06 assigns to the engine, the host or the query layer are listed as NOT COVERED because
the kernel has no code for them; G07 does not judge the engine.

| F06 cases | Concern | Implementing code | Evidence | Result |
|---|---|---|---|---|
| POS-01, POS-02 | Plate mesh, counts, Euler, outward winding; blob bytes and digest | `build::rectangular_plate`, `mesh::{encode,self_check}` | `rectangular_plate.rs` f06_pos_01, f06_pos_02 (independent reader, published digest); `g03_fixtures.rs`; `g06_fixtures.rs` | PASS |
| POS-03, POS-13 | Plate and member quantities, mass unknown without density | `PlateResult::volume_mm3/m3`, `measure::{mass_status,analytic_mass_kg}` | `rectangular_plate.rs` f06_pos_03; `g06_measure.rs` mass tests; `g07_acceptance.rs` member volumes | PARTIAL: volume and mass status pass; analytic surface area and local bounds are not kernel outputs |
| POS-14, POS-15, NEG-03, POS-53, POS-54, NEG-45 | Minimum and cap dimensions inclusive; scaled floor for plates | `geom::Tolerance`, `build::rectangular_plate`, `member::straight_member` | `rectangular_plate.rs` f06_pos_14, f06_neg_03, f06_pos_53_54_neg_45; `g07_acceptance.rs` f06_member_roll_and_axis_minimum, f06_member_scaled_floor_cap_and_failure_cases | PASS |
| POS-04, POS-05, POS-06, POS-07, POS-08 | Member frames, vertices, bounds, volume, area (horizontal, column, down, sloped, diagonal) | `member::straight_member`, `geom::MemberFrame` | `g07_acceptance.rs` f06_member_cases_match_the_published_vertices_volumes_and_areas (G07, new); `geom/tests/f06_math.rs` | PASS (mesh level added by G07) |
| POS-09, POS-10, POS-41, POS-42, POS-46, POS-52 | Parallel threshold, Y-tilted near-vertical axes, roll after the column rule | `geom::MemberFrame` (default frame, certificate), `member::straight_member` | `g07_acceptance.rs` f06_near_vertical_members_keep_sections_perpendicular_to_the_axis (G07, new); `geom/tests/f06_math.rs` | PASS (mesh level added by G07) |
| POS-50, POS-51 | Explicit zero roll equals omitted roll; quarter-turn roll | `geom::MemberFrame`, `member::straight_member` | `g07_acceptance.rs` f06_member_roll_and_axis_minimum, f06_member_scaled_floor_cap_and_failure_cases; `member.rs`; `geom/tests/f06_math.rs` | PASS (roll bytes for non-zero angles are platform `sin`/`cos`, not pinned) |
| POS-55, POS-57, POS-58, NEG-47, NEG-48 | Scaled floor of members (section half-diagonal and axis); small body keeps the absolute floor (POS-55, geom only, no placement in the kernel) | `geom::Tolerance::for_member`, `member::straight_member` | `g07_acceptance.rs` f06_member_scaled_floor_cap_and_failure_cases; `geom/tests/f06_math.rs` | PASS |
| NEG-04, NEG-05, NEG-06, NEG-07, NEG-43, NEG-44, NEG-51, NEG-52, NEG-53 | Member axis, section and roll failures | `member::straight_member` (`Code::AxisTooShort` added by G07) | `g07_acceptance.rs` f06_member_scaled_floor_cap_and_failure_cases; `member.rs`; `geom/tests/f06_math.rs` | PASS (token of NEG-04/05/47 fixed by G07, finding F1) |
| NEG-01, NEG-02, NEG-50 | Non-finite dimension, translation, derived scale | `build::rectangular_plate`, `geom` finiteness | `rectangular_plate.rs` f06_neg_01; `g07_acceptance.rs` special-value sweeps; `geom/tests/f06_math.rs` (NEG-02 translation, NEG-50 injection) | PASS (placement itself is not a kernel input) |
| NEG-31, NEG-35 | Frame certificate failure is numeric uncertainty at the build stage | `geom::frame` certificate, `member::locate` | `geom/tests/f06_math.rs` (fault injection); `member.rs` unit test `every_math_error_has_the_f06_code_and_stage` (G07, new) | PASS (stage mapping fixed by G07, finding F1; not reachable through the public builder) |
| POS-16, POS-17, POS-18 | Local FeatureId stability, uniqueness, grammar, length | `feature_id` | `rectangular_plate.rs` f06_pos_16_17_18 | PASS (local part only; ElementId qualification is engine work) |
| POS-27, NEG-11, NEG-12, NEG-13, NEG-14, NEG-15, NEG-30 | Blob caps exact, counts above caps, overflow, length mismatch, empty | `trimesh::TriMesh::{from_blob,from_buffers}` | `g06_shells.rs` caps_are_exact_at_the_limit_and_rejected_one_above, blob_header_cases_follow_the_f06_codes, blob_body_corruption_is_rejected_and_the_good_blob_is_not; `g06_fuzz.rs` | PASS |
| POS-37, NEG-16, NEG-17 | Cancellation before the seal point, none after | `build::rectangular_plate_with_cancel`, `profile::extrude_with_cancel`, `topology::analyze_with`, `ops::*_with` | `rectangular_plate.rs` f06_neg_16_17_and_pos_37; `g05_negative_and_fuzz.rs`; `g06_shells.rs`; `g09_budget_fuzz.rs` | PASS |
| NEG-18, NEG-19, NEG-20 | Open, self-intersecting and inward meshes never get an exact status | `topology::analyze`, `measure::measure`, `ops::Body::from_mesh` | `g06_shells.rs`, `g06_measure.rs`, `g06_fuzz.rs`; `g07_acceptance.rs` unwelded/T-junction/stray/duplicate/folded probes (G07, new) | PASS |
| NEG-37, NEG-40, NEG-41, NEG-42, NEG-46 | Contour winding, self-intersection, point count, repeated point, local-size cap | `profile::Profile::admit` | `g05_negative_and_fuzz.rs` contour_defect_classes_have_precise_diagnostics, valid_baseline_and_caps_are_inclusive, counts_and_caps_are_checked_before_any_traversal | PASS (2-D profile projection; the 3-D contour adapter does not exist) |
| POS-48, POS-49 | Rectangle contour starting on +Y edge / opposite corner | `profile` (cyclic start), no 3-D contour adapter | `g05_profile_extrusion.rs` cyclic_start_change_moves_roles_but_not_geometry_or_quantities | PARTIAL: polygon projection only; contour to plate-frame derivation (with z offset) is not in the kernel |
| NEG-38, NEG-49, POS-56 | Non-planar contour, planarity band | - | - | NOT COVERED: the profile is planar by construction; there is no 3-D contour input |
| NEG-39 | Valid pentagon is unsupported, not invalid | `profile::extrude` accepts any valid simple polygon | `g05_profile_extrusion.rs` L-shape and comb outlines | DIVERGENCE: the kernel supports more than F06 0.1.0; the engine must keep answering `GEO_UNSUPPORTED_CONTOUR` until A02 (finding F3). Update 2026-10-19: NEG-39 is superseded by A02.1 and F06 amendment A-2 (decision D-6), so this is no longer a divergence |
| NEG-08, NEG-09, NEG-10 | Curved profile, scaled or sheared transform, imported mesh frame | no kernel input form exists for them (member profile is a rectangle by type; no matrix input) | `geom/tests/f06_math.rs` NEG-09; absence by API shape | UNSUPPORTED by construction (the kernel does offer polygonal approximations of curved cavities, flagged `Approximated`, finding F3). Update 2026-10-19: NEG-08 is superseded by F06 amendment A-1 (decision D-6); no `Approximated` status exists in 0.1.x |
| POS-11, POS-12 | Rotated and translated plate and member, world mesh and bounds | none: the kernel builds element-local meshes only (`TriMesh::transformed` exists but has no F06 vector) | `g06_measure.rs` exact_rigid_motions..., inexact_rotation_changes_quantities_only_within_rounding (no F06 vector) | NOT COVERED (the engine returns `UNSUPPORTED_GEOMETRY` for a rotated placement) |
| POS-19, POS-20, POS-21, POS-22, POS-23, POS-24, POS-25, POS-26, POS-34, POS-35, POS-36, POS-38, NEG-21, NEG-25 | Shape/world/presentation keys, invalidation, incremental updates, stale quantity, token | none in the kernel (`kernel::keys` is planned; engine P04 signatures are not the F06 section 7 keys) | - | NOT COVERED: UNSUPPORTED in the kernel scope |
| POS-28, NEG-34 | Whole-blob snapshot chunking, negotiated buffer limits | host delivery | - | NOT COVERED (host) |
| POS-29, POS-30, POS-31, POS-32, POS-33, POS-40, POS-43, POS-44, POS-45, POS-47, POS-59, NEG-22, NEG-23, NEG-24, NEG-26, NEG-27, NEG-28, NEG-29, NEG-32, NEG-33, NEG-36 | Pick (ray, point, finite horizon), LookupFeature, element list order, query validation | none (`Selection` in the engine says Pick and LookupFeature are not implemented) | - | NOT COVERED: UNSUPPORTED (no implementation anywhere) |
| POS-39 | Engine API and CLI return the same logical result | engine and CLI for plates, members and plate holes (P01, P03, A04), not the full kernel scope | CLI tests of those tasks (not assessed by G07) | NOT COVERED by G07: G08 owns it |

### 5.3 G03 fixtures and the independent harness

The 56 G03 descriptions (22 supported, 26 invalid, 4 unverified, 4 unsupported) are checked by
`g03_fixtures.rs`, which calls only the public builder and compares with exact rational references
from `tests/g03/write_fixtures.py` (the writer never loads the kernel); nine corruptions of each of
the 22 real outputs (198 checks) must fail. Both pass. The four `unsupported` placeholders
(through-hole, cavity, thin-wall, non-rectangular) and the four `unverified` large translations are
exercised by the real G05 and G09 operations in `g05_profile_extrusion.rs` and `g09_hole.rs`, not by
the G03 harness, so their rows in `expectations.tsv` correctly stay `not_run` (finding F8).

## 6. Evidence added by G07

| Item | What it shows |
|---|---|
| `tests/g07_acceptance.rs` f06_member_cases_match_the_published_vertices_volumes_and_areas | Straight-member meshes for F06-POS-04 to POS-08 match the published vertex lists, volumes and triangle areas, closed, Euler 2, outward, with independent code |
| f06_near_vertical_members_keep_sections_perpendicular_to_the_axis | POS-09, POS-10, POS-41, POS-42, POS-46, POS-52: the end sections stay perpendicular within 1e-12 (the historical fixed +Y fallback leaves 5e-7), the branch (column or horizontal) is the published one |
| f06_member_roll_and_axis_minimum, f06_member_scaled_floor_cap_and_failure_cases | POS-14, POS-15, POS-50, POS-51, POS-57, POS-58 and NEG-04 to NEG-07, NEG-43, NEG-44, NEG-47, NEG-48, NEG-51 to NEG-53 through the real builder: code, F04 mapping, status and stage |
| plate_and_member_builders_survive_every_special_scalar | 23 special values (NaN, both infinities, signed zeros, subnormals, caps and neighbours, huge finite) over every plate triple and every pair of member scalars: no panic, a non-finite input is never accepted, every success is re-read as a closed outward box |
| cut_hole_and_profile_operations_survive_every_special_scalar | The same over every plane component, every cutter range pair, circle centre, radius, sagitta and thickness |
| unwelded_junctioned_stray_duplicated_and_folded_meshes_never_get_an_exact_status | Unwelded cube (`Open`), T-junction (`Open`), stray triangle (`Open`), duplicated triangle (`NonManifold`) and a closed folded shell (`Overlapping`) are not certified, have no volume, area or centroid, status `unsupported`, and are refused as bodies |
| a_plane_cut_leaving_edges_below_the_floor_is_refused_and_one_above_is_sealed | Near-tolerance cut: a corner shave with edges of half `eps_geom` is a typed refusal, one with edges of ten times `eps_geom` seals and is verified |
| sealed_blobs_have_pinned_digests | Pinned SHA-256 digests of a plate (the F06-POS-02 vector), a member, an extrusion with a circle, a plane cut and a through hole. They use only IEEE `+ - * /` and `sqrt`, so they must be identical on every target; the Linux CI job is the first cross-target observation |
| `member.rs` unit test every_math_error_has_the_f06_code_and_stage | The code and stage mapping of each member error |
| `examples/g07_bench.rs` | Reproducible timing and determinism harness (section 7) |
| Fix `Code::AxisTooShort` (`GEO_AXIS_TOO_SHORT`) and stages | Finding F1 |

## 7. Measurements

Reproduce with `cargo run --release -p beamcraft-kernel --example g07_bench`. Times are wall clock
of single-threaded calls (the kernel starts no threads). Medians of three separate process runs; the
machine was shared with other builds, so individual medians varied by up to a factor of two and the
spread is shown where it matters. These numbers are what this machine did, not a promise.

Machine class: Intel Core i5-10400F (6 cores, 12 threads, 2.9 GHz base), 64 GB RAM, Windows 10
Enterprise 10.0.19045, x86_64, rustc 1.96.0, release profile (thin LTO, one codegen unit).

Note (2026-10-21): those release settings are now the `dist` profile, and `--release` builds with
16 codegen units and no LTO. Reproduce these conditions with
`cargo run --profile dist -p beamcraft-kernel --example g07_bench`.

| Workload | Time | Outcome |
|---|---|---|
| Plate 1000 x 500 x 10, build and seal | 6.4 microseconds | sealed |
| Member 3-4-5 diagonal, build and seal | 7.0 microseconds | sealed |
| Profile extrusion with 1 circle cavity (64 edges) | 0.26 ms | sealed |
| Profile extrusion with 16 circle cavities (32 edges each) | 3.0 ms | sealed |
| Profile extrusion with 128 circle cavities (8 edges each) | 11.8 ms | sealed |
| Plane cut of a plate (oblique, hexagonal section) | 0.79 ms | sealed |
| Through hole (64-gon) in a plate | 28 ms | sealed |
| Sequence of 64-edge holes, each call re-certifying the body | 70, 150, 270, 415 ms for holes 2 to 5; the 6th is refused after 0.53 s | 5 holes accepted, `GEO_NUMERIC_UNCERTAIN` (`result.uncertain`) |
| Sequence of 32-edge holes | 45 ms for the 2nd, 470 ms for the 9th; the 10th is refused after 0.53 s | 9 holes accepted |
| Sequence of 16-edge holes | 34 ms for the 2nd, 460 ms for the 16th; the 17th is refused after 0.55 s | 16 holes accepted |
| The refused call repeated with every certification budget raised 16 times | 0.56 to 0.60 s | sealed: the refusal is the budget, not the geometry |
| Certify a closed welded slab, 96 / 320 / 672 / 1,152 triangles | 8 / 33 / 78 / 138 ms | `Certified` |
| Certify slabs of 1,760 / 2,496 triangles | 0.21 to 0.34 s / 0.30 to 0.37 s | `Certified` |
| Certify slabs of 4,352 / 8,096 triangles | 0.36 to 0.38 s / 0.37 to 0.39 s | undecided: `Uncertain(PredicateBudget)`, not exact |
| The same 1,152-triangle slab at 3e8 mm from the origin | 0.14 to 0.25 s | `Certified` (no extra cost from distance) |
| Mesh measurement (volume, area, centroid) of the 2,496-triangle slab | 8.7 ms (one run 16 ms) | values |
| Structural gate of a 64,976-byte blob / a 210,576-byte near-cap blob | 13.5 / 42.5 microseconds | accepted |
| Adversarial closed bipyramid, 400 / 2,000 / 8,000 long thin triangles with heavily overlapping boxes | 21 ms / 0.27 to 0.33 s / 0.29 to 0.39 s | `Certified` / `Uncertain(PairBudget)` / `Uncertain(PairBudget)` |

Reading of the numbers:

* Building is cheap (microseconds to a few milliseconds). Certification is the cost: about 0.08 to
  0.14 ms per triangle on cap-rich meshes, because exactly coplanar neighbouring triangles need an
  exact-integer fallback. Every operation re-certifies its candidate, so a through hole costs about
  28 ms even though the build takes a fraction of it.
* Work is bounded by counts and budgets: the largest single call observed (a refused one) was about
  0.6 s, and the refusals are typed, deterministic and carry no partial geometry. The F06 cap of
  8,192 triangles is not reachable for such meshes; the default budgets end certification first
  (finding F2).
* Memory was not measured (a counting allocator needs `unsafe`, which the workspace forbids). It is
  bounded by the caps: a blob is at most 213,024 bytes, a mesh at most 4,096 vertices and 8,192
  triangles, the integers have fixed capacity, and every allocation is sized by a validated count.
* The CI test profile is a debug build: the kernel test suite takes about 1 minute 50 seconds there
  (`g09_budget_fuzz` 26 to 42 s, `g09_cut` 12 to 19 s, `g06_fuzz` about 15 s).
* End to end through the real `beamcraft create-plate` binary (release): one or two round holes with
  the default 32 edges are authored in 90 to 160 ms. A third is refused with `LIMIT_EXCEEDED`
  ("stage budget exceeded", the message of `beamcraft-layer-engine`) after 0.15 s, although the
  kernel itself accepts up to 9 such holes; 12 and 16 holes were refused by the kernel
  (`UNSUPPORTED_GEOMETRY`) after about 2.7 s. Refusals exit with code 2 and a typed rejection. The
  store contents after a refusal were not inspected by this task.

## 8. Findings

| ID | Severity | Finding | State |
|---|---|---|---|
| F1 | Low | A member axis shorter than the floor reported `GEO_BELOW_MINIMUM` instead of the contract's `GEO_AXIS_TOO_SHORT` (F06-NEG-04, NEG-05, NEG-47), a failed frame certificate reported stage `ranges` instead of `build` (NEG-31, NEG-35) and a non-finite derived intermediate stage `ranges` instead of `caps` (NEG-50). The F04 code was already right | **Fixed in this change** with `Code::AxisTooShort`, the stage mapping, and regression tests |
| F2 | Medium | Capacity and speed. Certification dominates the cost (section 7) and its default budgets end it early: a welded slab of 2,496 triangles certifies, one of 4,352 does not; a plate takes 16 holes of 16 edges, 9 of 32 or 5 of 64. The model cap of 64 holes per plate (A04) cannot be used, and the CLI is limited further by the layer-engine stage budget to two holes of 32 edges. All refusals are typed and harmless | Open. Not a correctness defect. G08/A04 must publish and enforce the real capacity or the owner must decide the budgets with evidence. Update 2026-10-19: the owner decided (D-5) that there is no designed limit; this is a certification performance defect owned by new task G10, and until G10 the command layer checks capacity before the kernel and refuses with a distinct kernel-capacity result (F06 A-2, F04 A-2) |
| F3 | Medium | Contract coverage. Holes, plane cuts, extrusion with cavities, polygon-approximated curves (`Approximated` volume), genus above zero, the provisional `extrusion` and `cut` FeatureId kinds and the proposed `GEO_OP_UNSUPPORTED` and `GEO_OP_NO_EFFECT` tokens are outside frozen F06 0.1.0 (its scope excludes booleans, cuts and holes; F06-NEG-08 calls a curved profile unsupported; F06-NEG-39 requires a valid pentagon to be `GEO_UNSUPPORTED_CONTOUR` while the kernel extrudes it). The A04 pilot already ships holes under a recorded deviation, and stores a mesh-enclosure midpoint as a `current` volume (A04 decision 3) although F06 section 8 gives non-analytic meshes no engineering quantity | Open. Needs an owner-approved amendment or new contract version before G08. Update 2026-10-19: owner-approved F06 amendment A-2 (decisions D-1 to D-7) covers holes, plane cuts, polygonal cavities, the `extrusion` and `cut` FeatureIds, the `GEO_OP_*` tokens and the certified-shell volume rule; polygon approximations carry no `Approximated` status. G07 precondition 1 is met by the text; no implementation or kernel acceptance of it is claimed |
| F4 | Medium | Missing F06 outputs: analytic surface area, bounds as quantities, section 7 keys, section 9 queries, rotated placement (POS-11, POS-12), the 3-D contour to plate-frame derivation (POS-48, POS-49, NEG-38, NEG-49, POS-56). The engine's own quantity signatures are not the F06 keys, and `Selection` documents that Pick and LookupFeature do not exist | Open. Unsupported by absence; owned by later tasks |
| F5 | Medium | Cross-target conformance is unproven: only Windows x86_64 was run; wasm32 and `cargo deny` were not available; non-zero roll uses platform `sin` and `cos` (F06 section 2 requires conformance evidence before identical bytes are claimed) | Open. The pinned-digest test makes the Linux CI the first second target; wasm32 is checked by CI |
| F6 | Low | The validation plan is stale: all 44 rows still say `planned`, and 29 of the 112 F06 cases are referenced by no row | Closed 2026-10-23: the stale plan JSON was removed; this document carries the trace |
| F7 | Low | A plane cut that shaves a corner of a 100 mm cube leaves a section with edges of a few `eps_geom`. Observed with the plane x + y + z = d: edges from 0.1 to 3.75 times `eps_geom` are refused (`GEO_OP_UNSUPPORTED`, `section.not_admissible`), 4 times and above are accepted. The pilot is therefore conservative near the floor; the exact admission rule applied to the cap polygon was not traced | Documented; behaviour is safe (no result is returned) |
| F8 | Low | The G03 `expectations.tsv` rows of through-hole, cavity and thin-wall stay `not_run` although `g09_hole.rs` runs them on the real operation (the G03 harness reads no operation result) | Bookkeeping only |

No crash, panic, hang, unsound `Certified` verdict or partial result was found. The hostile sweeps
and the existing fuzz suites (seeded mutated blobs, random soups, surgery on valid shells, perturbed
vertices, hostile planes and cutters, maximum-size inputs) found nothing besides F1.

## 9. Preconditions for G08

1. **Contract first.** Before the engine treats holes, cuts, extrusion profiles or curved cavities
   as product features, an owner-approved F06 amendment (or a new contract version) must cover
   them: operation results, the `extrusion` and `cut` FeatureId kinds, the `GEO_OP_*` tokens, the
   `Approximated` status of polygon-approximated curves, and the quantity status of mesh-computed
   volumes (F3).
2. **Keep the unsupported gates.** The engine must keep answering `UNSUPPORTED_GEOMETRY` for valid
   non-rectangular plate contours until A02, for rotated placement until placement exists with F06
   vectors, and for every query and key the kernel does not implement (F4).
3. **Capacity and honest errors.** The command layer must apply the measured capacity before it
   runs the kernel (hole and triangle estimates), and must not present a budget refusal
   (`GEO_NUMERIC_UNCERTAIN`, defect `result.uncertain`) as "invalid geometry". The limits are the
   explicit `OpLimits` and `Limits`; changing them needs new measurements (F2).
4. **Cancellation and revision checks.** Kernel cancellation is cooperative at a handful of polls and
   bounded by counts, not by wall-clock time (a single call may take up to about 0.6 s here). G08 must
   run it where the host can cancel, check the revision before publication and discard the result on
   any mismatch. Nothing may be published from a cancelled or failed call: the kernel returns no data
   on failure, and G08 must show by an end-to-end test that a failure leaves the USD store unchanged.
5. **Use sealed results only, with their status.** Use `OpResult`/`PlateResult` digests and blobs as
   given; treat `OpResult` volumes as mesh enclosures unless the F06 amendment says otherwise; never
   persist the provisional `extrusion` and `cut` ids before it.
6. **Cross-target evidence.** Require the Linux CI run of `sealed_blobs_have_pinned_digests` and the
   wasm32 check of the three pure crates to pass, and add a roll conformance vector, before any
   cross-target byte identity is claimed (F5).
7. **Stay inside the rules.** The kernel keeps `unsafe_code = "forbid"` and the workspace
   `unwrap`, `expect`, `panic` and indexing lints; G08 code must too, and a fixed crash gets a
   synthetic regression test.

## 10. Limits of this acceptance

* The tests I added are independent of production geometry code but share the repository's test
  helper readers (`common`, `common_mesh`, `common_ops`), which earlier tasks wrote.
* "No third-party code" is based on the dependency tree and a string search of the kernel sources,
  tests and example; no code-similarity analysis was done.
* Speed numbers are from one machine under shared load. They show order of magnitude and bounded
  work, not a guarantee or a regression threshold.
* Nothing here validates the engine, the host or the CLI beyond the single observation in section 7.

## 11. Reproduction

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p steel --test steel -- g07_acceptance::
python tests/g03/write_fixtures.py --check
cargo run --release -p beamcraft-kernel --example g07_bench
cargo run -p beamcraft-cli -- --version
```

Provenance of this document: original text by the G07 reviewer from the repository's own contract,
design, validation plan, fixtures, sources and the runs above. No third-party kernel material,
Tekla or Trimble material, or external test data was used.
