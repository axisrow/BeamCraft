# K1 contract acceptance evidence

This is the K1 verification slice of [issue #125](https://github.com/Apibim-com/BeamCraft/issues/125),
based on integration commit `09403ce8fabf973a975e523734af3dab8dcd90fa` (U16 merged, U17 already
integrated). It adds a fixture-driven plate adapter and an explicit inventory, not a new modelling
feature or a change to a roadmap gate. K2 is separate.

## Evidence rules

- **Passed** means the stated assertions ran and matched. An admission code is not evidence of
  saved USD, a geometry diagnostic, a feature protocol or quantity invalidation.
- **Partial** names the fields actually checked and the fields not checked. It is not a passed
  contract case. **Deviation** records an observed disagreement, not an accepted change to F04/F06.
- **Not run** means this adapter did not execute the fixture case. Other manually encoded tests
  may cover its subject; their existence does not make this fixture-driven run pass.
- The F04 and F06 JSON files remain unchanged. No expected value comes from a production geometry
  helper. Numeric references use F06's quantity relative tolerance and coordinate absolute plus
  relative tolerance; identities, codes, tables and digests are compared exactly.
- The independent reader is the std-only `beamcraft-usd-oracle` process. It runs in every
  `cargo test` and skips nowhere, so there is no optional-skip path to guard.

## K1 requirement to executable evidence

Paths below are relative to the repository root. These runtime tests predate this verification
slice; the new contract inventory does not replace them.

| Requirement | Test and assertions | Scope |
|---|---|---|
| P01: two plate IDs, complete saved revisions, fresh-process reopen | `apps/beamcraft-cli/tests/plate.rs::two_plates_in_two_commits_are_independent_complete_revisions_readable_without_beamcraft` | Real CLI, separate commits, authored attributes and derived mesh, closed shell, winding, analytic volume, units and placement |
| U16: current category paths and identities | `apps/beamcraft-cli/tests/prim_naming.rs` | Full UUID in `/World/Steel/Parts/PRT_<uuidhex>`; links, save/reopen and view export |
| U02/U09: ordinary OpenUSD interoperability | `apps/beamcraft-cli/tests/independent_reader.rs::the_reader_reads_the_committed_revision_and_agrees_with_the_closed_form` and `beamcraft-usd-oracle` | The independent reader opens the materialized revision layers; two plates, attributes, units, local/world geometry, model kinds, display name, volume and structural checks |
| U06: atomic publication | `plate.rs::a_killed_process_leaves_the_old_or_the_new_complete_revision_at_every_commit_point`, `concurrent_creates_publish_only_complete_revisions_with_distinct_ids` | Killed child at commit checkpoints; old or new complete revision, no partial head; concurrent writers |
| U07: recovery | `apps/beamcraft-cli/tests/recovery.rs::an_interrupted_commit_is_resolved_by_the_binary_at_every_crash_point`, `a_torn_ledger_tail_is_reported_and_recovery_rebuilds_the_record_from_the_root`, `a_ledger_that_contradicts_the_root_is_an_integrity_error_not_an_acceptance` | Journal outcomes, interrupted commits, torn tails and contradictory records |
| P01 replay and rejection | `plate.rs::the_same_operation_is_replayed_and_a_changed_one_is_refused`, `rejected_commands_leave_no_revision_and_report_the_f04_code`, `a_library_error_is_typed_and_a_failed_commit_leaves_the_old_revision` | Stable operation result; typed errors; preserved previous head |
| U14: retained content and corruption | `apps/beamcraft-cli/tests/history_compact.rs::compaction_keeps_the_head_meshes_quantities_and_the_foreign_layer`, `a_damaged_retained_blob_refuses_and_a_restored_backup_serves_the_old_history` | Real CLI compaction, unchanged head content, damaged blob refusal and restored backup |
| G01/G02: frozen geometric references | `apps/beamcraft-cli/tests/f06_cases.rs` plus existing `crates/geom/tests/f06_math.rs`, `crates/kernel/tests/rectangular_plate.rs` | New fixture-driven plate projections below; existing unit tests remain lower-level evidence |
| F04 command admission | `apps/beamcraft-cli/tests/f04_cases.rs` | Typed engine admission only; no claim of a full F04 envelope decoder |

The current owner decisions supersede historical P01/G07 references to an unpassed K0, a mandatory
SDK 26.08 or USD worker containment. U17 intentionally reads USD in process. This verification
uses the installed reference implementation only as a test oracle; it adds no SDK or dependency.

## F04 inventory (69 logical cases and 7 raw cases)

`every_decidable_create_plate_case_matches_f04_or_is_a_declared_deviation` runs 37 of the 42
CreatePlate cases: 33 matching admission outcomes/codes and 4 explicit deviations. It skips the
other 5. It does not verify every field of the accepted response. The inventory test also checks
the exact IDs of the other 27 logical cases and 7 raw envelopes, so they cannot disappear silently.
Every ID in this section has prefix `F04-`.

| Cases | Result in this adapter | Reason / evidence |
|---|---|---|
| POS-01, POS-04, POS-08, POS-09, POS-11, POS-12, POS-14, POS-17, POS-19, POS-20, POS-25, POS-26, POS-27; NEG-01, NEG-02, NEG-03, NEG-04, NEG-05, NEG-06, NEG-10, NEG-19, NEG-22, NEG-23, NEG-24, NEG-25, NEG-26, NEG-27, NEG-28, NEG-30, NEG-31, NEG-32, NEG-34, NEG-35 | Passed admission projection (33) | Fixture arguments go through typed `admit_create_plate`; outcome/code matches |
| POS-18, POS-28 | Deviation (2) | Contour outside `z=0`: engine returns `UNSUPPORTED_GEOMETRY` instead of accepting the authored draft |
| NEG-29, NEG-38 | Deviation (2) | Non-planar contour: engine returns `UNSUPPORTED_GEOMETRY` instead of `INVALID_ARGUMENT` |
| NEG-07 | Not run | Envelope `contract_version` is not decoded by typed admission |
| NEG-08 | Not run | This adapter supplies no workspace/base-revision context; workspaces exist and have separate tests |
| NEG-09 | Not run | This adapter supplies no workspace generation context |
| NEG-17 | Not run | No authenticated actor/permission context in this adapter |
| NEG-18 | Not run | Raw envelope `expected_generation` decoding is outside typed admission |
| POS-02, POS-06, POS-24, NEG-11, NEG-14, NEG-15 | Not run (6) | UpdateElement fixture adapter not supplied by this K1 slice |
| POS-03 | Not run (1) | CopyElement is assigned to K2 |
| POS-05, NEG-12, NEG-13 | Not run (3) | DeleteElement fixture adapter not supplied by this K1 slice |
| POS-07, POS-10, POS-13, POS-15, POS-16, POS-21, POS-22, POS-23, POS-29, POS-30, NEG-16, NEG-20, NEG-21, NEG-33, NEG-36, NEG-37, NEG-39 | Not run (17) | CreateStraightMember is assigned to K2 |
| RAW-01, RAW-02, RAW-03, RAW-04, RAW-05, RAW-06, RAW-07 | Not run (7) | These are F04 JSON envelopes, not CLI flag requests or typed commands; the existing bounded JSON decoder alone is not the command decoder |

## F06 inventory (112 cases)

`f06_cases.rs` reads the actual fixture using the bounded JSON decoder, asserts version 0.1.0,
maps typed inputs and invokes the public engine `create_plate` API. Non-finite sentinels become
typed NaN/Inf as the logical fixture specifies. It drives 21 plate cases: one passed case, 16
partial projections (including the face-table discrepancy below), and four rejected-input
deviations. A second test accounts for the remaining 91 cases exactly once. Every ID below has
prefix `F06-`. The test output says PASSED, PARTIAL or DEVIATION separately.

| Driven cases | Result | Asserted fields and limits |
|---|---|---|
| POS-01 | Partial; face-table deviation | Counts; fixture vertex/triangle order; triangle-to-face mapping; axis roles; outward normals; independent edge census, reverse incidences, shell connectivity, both Euler counts and signed volume. Fixture face roles are `x0` etc.; the public table returns `plate:f.x0` etc. The driver explicitly projects and checks the roles, asserts the raw strings still differ, and does **not** claim the identifier format passes |
| POS-02 | Passed | Exact blob length, section offsets/lengths, published SHA-256 and identical bytes on a second engine evaluation |
| POS-03 | Partial | Volumes and volume status, local bounds, unknown mass without density and current mass with fixture density. Engine area value/status and bounds status are not exposed or asserted |
| POS-11 | Deviation | Expected rotated world geometry; actual `UNSUPPORTED_GEOMETRY`. No world mesh, key or quantity assertion is claimed |
| POS-48 | Deviation | Expected plate in its body frame at `z=30`; actual `UNSUPPORTED_GEOMETRY` |
| POS-49 | Partial | Opposite-corner contour: local bounds, XY extents, volume, vertex and triangle counts. Plate frame, feature count and surface area are not exposed/asserted |
| POS-53, POS-54, POS-55 | Partial (3) | Accepted authored arguments remain unchanged; repeat at identity placement also accepted. Boundary admission is exercised; the internal local-size/floor numbers are not engine outputs or checked here |
| POS-56 | Partial | `UNSUPPORTED_GEOMETRY` matches the requested error; authored draft acceptance, numeric scale, geometry diagnostic/stage and state preservation are not checked |
| NEG-01, NEG-02, NEG-03, NEG-37, NEG-40, NEG-41, NEG-42, NEG-45, NEG-46 | Partial (9) | Fixture F04 code matches; no returned success body. Geometry diagnostic/stage and persisted state preservation are not checked by this pure adapter |
| NEG-38, NEG-49 | Deviation (2) | Expected `INVALID_ARGUMENT`; actual `UNSUPPORTED_GEOMETRY` |

The remaining cases are **not run by this fixture adapter**. This does not assert that their
features are absent: the older G07 report and current runtime tests provide other evidence.

| Cases | Disposition and reason |
|---|---|
| POS-04, POS-05, POS-06, POS-07, POS-08, POS-09, POS-10, POS-12, POS-13, POS-14, POS-15, POS-41, POS-42, POS-46, POS-50, POS-51, POS-52, POS-57, POS-58; NEG-04, NEG-05, NEG-06, NEG-07, NEG-43, NEG-44, NEG-47, NEG-48, NEG-51, NEG-52, NEG-53 | Member/mixed adapter (30), assigned to K2; POS-14 contains both a plate and member and remains unclaimed as a whole |
| POS-16, POS-17, POS-18; NEG-22, NEG-23, NEG-24 | Feature identity/lookup adapter not supplied (6); role checks in POS-01 are not this protocol |
| POS-19, POS-20, POS-21, POS-22, POS-23, POS-24, POS-25, POS-26, POS-34, POS-35, POS-36, POS-38; NEG-21, NEG-25 | Keys, invalidation, changes, stale results and snapshot token adapter not supplied (14) |
| POS-27; NEG-11, NEG-12, NEG-13, NEG-14, NEG-15, NEG-30 | Raw blob/header adapter not supplied (7); byte generation in POS-02 is not hostile blob validation |
| POS-28; NEG-34 | Negotiated delivery/paging adapter not supplied (2) |
| POS-29, POS-30, POS-31, POS-32, POS-33, POS-43, POS-44, POS-45, POS-47, POS-59; NEG-26, NEG-27, NEG-28, NEG-29, NEG-33, NEG-36 | Pick/query adapter not supplied (16) |
| POS-39, POS-40 | Canonical API/CLI parity and canonical list order adapter not supplied (2); the existing P01 round-trip is separate evidence |
| NEG-09, NEG-10 | Matrix/imported-stage request adapter not supplied (2) |
| NEG-18, NEG-19, NEG-20 | Uncertified mesh measurement adapter not supplied (3) |
| POS-37; NEG-16, NEG-17, NEG-31, NEG-32, NEG-35, NEG-50 | Cancellation/certificate/derived-scale injection adapter not supplied (7); existing lower-level injection tests remain separate evidence |
| NEG-08, NEG-39 | Superseded by amendment A-2 (2), never counted as passed |

## Reproduction and environment

Recorded on 2026-10-21, Windows 10 build 19045, Rust/Cargo 1.96.0. All test models are contributor-original synthetic data
covered by the test-data entry in `ATTRIBUTION.md`. No network, runtime dependency or asset is added.

```powershell
cargo test --offline --locked -p beamcraft-cli --test cli -- f04_cases:: f06_cases:: --nocapture
cargo test --offline --locked -p beamcraft-cli --test cli -- independent_reader:: --nocapture
cargo test --offline --locked --workspace --no-fail-fast
cargo fmt --all -- --check
cargo clippy --offline --locked --workspace --all-targets -- -D warnings
$env:RUSTDOCFLAGS = '-D warnings'
cargo doc --offline --locked --workspace --no-deps
cargo run --offline --locked -p beamcraft-boundaries -- check
```

The missing-expectation control is `a_missing_expectation_file_fails_the_reader`: the reader
fails rather than passing without an expectation.

## Acceptance limits

The reproduced gaps are tracked in [issue #171](https://github.com/Apibim-com/BeamCraft/issues/171).
This slice does not declare K1 passed. The ordinary two-plate scenario and storage tests are
evidence of their stated scope; they do not erase the F04/F06 deviations or supply the missing
fixture adapters. The findings need focused fixes and reruns, followed by an owner-reviewed
docs-only verdict under `docs/plan/` and any corresponding `ROADMAP.md` change. K2 and the
unrelated worker/SDK proposals are outside this slice.
