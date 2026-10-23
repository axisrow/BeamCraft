# C03–C06: supported profile and acceptance evidence

Scope: fork #12 / upstream #126. This records the proposed implementation's executable acceptance,
not a merged integration gate or an owner/platform sign-off. Model authority remains ordinary
USD, and accepted revisions are immutable. C07 live networking, permissions/hosting, a conflict
resolution UI and native drawing/UI acceptance are separate tasks.

## Supported operations

`rebase-workspace` keeps independent, precisely evidenced unsent commands in one new personal
revision over an exact accepted basis. It never publishes automatically. Every intervening
read/write dependency participates, including unchanged copy sources and reverted writes.
Conflicts return complete bounded operation/target/group diagnostics and retain both original
models. Missing evidence cannot establish independence.

`compensate-operation` produces a new personal semantic inverse under current preconditions.
For C01/C02, `--original-revision` selects the complete published changeset as one operation,
including multiple source commands and verified administrative rebase records. The inverse uses
that publication's actual accepted base, independently of its older local source prefix. Later
unrelated opinions remain present; later overlapping writes, reads, links and reused identities
refuse. Both original and compensating publications remain immutable history.

Compensation requires a clean personal workspace at the current accepted basis, ownership of
the original and destination workspaces, retained exact source history and, for C01, its canonical
payload. Direct legacy `workspace-publish` proof is supported when one terminal command accounts
for its complete delta and dependencies. An unavailable larger legacy interval is refused;
the adapter never guesses its boundary. Legacy absence, unknown content, unsupported reference
wrapper inverses and compacted/corrupt proof refuse explicitly. Existing ordinary commands,
publication, undo of later local edits and conservative legacy changesets remain available.

The CLI actor is trusted local context and defaults to the recorded owner. Publication through
the opt-in loopback coordinator derives authority from its token. Neither operation introduces
an automatically started service, remote access or an implicit publication.

Budgets stay bounded: 4096 operations/aggregate dependency entries, 256 complete conflict pairs,
one-MiB host evidence and existing stage/history limits. C05's synthetic batches retain all eight
create entries per part and fit those limits; they are not a production bulk CLI. Recovery covers
process interruption and exact immutable commit outcomes. The host's existing limitation on
directory-entry durability is not a power-loss acceptance claim.

## C03 executable mapping

The original case definitions remain in [`c03-rebase.md`](c03-rebase.md). Paths below are relative
to the repository root; named functions are searchable test entry points.

| Cases | Executable evidence |
|---|---|
| C03-01 independent clients | `apps/beamcraft-cli/tests/rebase.rs`: `independent_edits_rebase_without_publishing_and_replay_after_a_later_edit`, `both_clients_new_parts_and_immutable_materials_survive_rebase` |
| C03-02 shared layer/separate groups | The same real rename/move scenario verifies authored fields, placement, volume and mass; `crates/layer-engine/tests/rebase_candidate.rs` checks retained inert opinions |
| C03-03 atomic inputs | `crates/engine/src/rebase.rs`: `shape_inputs_are_atomic_and_every_intervening_operation_participates`; `crates/layer-engine/tests/rebase_candidate.rs`: `same_group_is_atomic_even_for_different_properties`; real CLI shape-conflict scenario |
| C03-04 indirect dependencies | Real copy/source and weld/delete scenarios in `apps/beamcraft-cli/tests/rebase.rs` and `apps/beamcraft-cli/tests/rebase/conflicts.rs`, in both accepted/draft directions |
| C03-05 intermediate revisions | `semantic_conflicts_include_indirect_reads_and_intermediate_reverted_writes`; planner tests retain every intervening operation |
| C03-06 absent/forged evidence | `apps/beamcraft-cli/tests/publication_evidence.rs`: narrowed/incomplete intervals; `apps/beamcraft-cli/tests/rebase_receipt.rs`: substituted operation/request, truncated/noncanonical records; `apps/beamcraft-cli/src/workspace/pending_tests.rs`: narrowed intent and wrong candidate evidence; real missing-proof refusal |
| C03-07 replay/reuse | Real CLI historical replay after a later edit, same-call/head-moved binding, C01 refused-ID binding and coordinator publication replay |
| C03-08 head races | `apps/beamcraft-cli/tests/publication_guard.rs` and the actual draft-CAS race in `apps/beamcraft-cli/tests/rebase.rs`: accepted writes refuse while the guard is held; the winning personal edit and metadata survive |
| C03-09 crash windows | `apps/beamcraft-cli/src/workspace/pending_tests.rs`: `killed_process_at_each_storage_boundary_recovers_one_complete_draft` (blobs, intent, root switch, metadata); real runtime injected retries |
| C03-10 later edits | `recovery_keeps_a_later_edit_and_publication_retains_both_clients` and `an_interrupted_runtime_request_retries_once_and_recovers_beyond_a_later_edit` |
| C03-11 resumed publication | `apps/beamcraft-cli/tests/coordinator.rs`: `a_rebased_workspace_packages_the_new_basis_and_publishes_and_replays_under_its_token`; asserts exact accepted layer bases, token ownership and original unsent identity |
| C03-12 foreign content | Real attached reference before branching, byte-identical source through rebase/publication; materializer's foreign/ancestor/unknown-delta guards; C01 existing/new-reference regression |
| C03-13 hostile/bounded inputs | Planner 4096/4097 and 256/257 tests; receipt operation/dependency/history/byte bounds; `hostile_contexts_metadata_and_pinned_blobs_refuse_without_a_partial_rebase`; strict workspace metadata/intent cases |
| C03-14 independent reader | `apps/beamcraft-cli/tests/independent_reader.rs`: final two-client USD against literal closed forms; C05 additionally checks an actual 2001-part accepted stage with ordinary OpenUSD |

## C04, C05 and C06 executable mapping

| Package / cases | Executable evidence |
|---|---|
| C04-01/02 same property/atomic group | Real distinct/equal-label and thickness conflicts; materializer's different-property shape-group test |
| C04-03/04 dependency direction/delete-link | `apps/beamcraft-cli/tests/rebase/conflicts.rs`: accepted copy/source change and accepted weld/draft delete; opposite directions in the original rebase runtime tests |
| C04-05/06 intent/identities | Intermediate reverted writes still conflict; same new ID retains different original dimensions/names in both stores; distinct IDs/materials survive |
| C04-07/08 report/preservation | Planner complete deterministic 256-pair report and refusal above; every runtime conflict reopens complete layers, checks both operation IDs, heads and exact metadata, and repeats in another CLI process |
| C05-01/02 IDs/offline basis | `apps/beamcraft-cli/src/workspace/offline_tests.rs`: eight rounds of 125 new IDs per client, two distinct workspaces for one user; stale publication refuses; final exact set has 2000 new IDs plus seed |
| C05 allocator coverage | Existing `crates/host/src/ids.rs` tests generate 5000 increasing complete IDs and 2000 per independent system source without collisions; deterministic model fixtures also share long ID prefixes |
| C05-03/04 replay/ambiguous result | Same-source publication/rebase repeats and changed payload refusal; switched-but-unfinalized rebase recovered in a fresh process; final publication receipt replayed in another process |
| C05-05/06 workspace isolation/budgets | A's publication leaves B's metadata unchanged; all actual create footprints are retained under unchanged budgets; planner/receipt/aggregate at-cap and cap-plus-one tests remain required |
| C05-07 readers | Independent reader checks every ID, name, placement and closed-form quantity; `tools/check-c05-openusd.py` additionally opens the actual accepted layers with ordinary OpenUSD |
| C06-01/02 inverse/preserve others | `apps/beamcraft-cli/tests/rebase/compensation.rs`: rename, thickness, create/delete and C01 inverses retain another owner's later fields, placement, geometry, mass and reference bytes |
| C06-03/04 overlap/reads | Later writes, reverted writes, copy-source reads and weld links refuse; original and current heads/metadata remain unchanged |
| C06-05 lifecycle | Create/delete inverse cases; `apps/beamcraft-cli/tests/rebase/compensation_recovery.rs` verifies that restoring a deleted part never replaces another owner's new part with the same ID |
| C06-06 proof/actor | `apps/beamcraft-cli/tests/rebase/compensation_proof.rs`: ownership, wrong/unpublished source, exact direct/C01 proof and payload corruption; dirty/stale destinations refuse without replacing local work |
| C06-07 crash/race/replay | `apps/beamcraft-cli/tests/rebase/compensation_recovery.rs`: checked/switched interruption, two actual killed processes and explicit host recovery, later-edit/historical replay, real competing draft CAS, pinned accepted publication and same original/destination workspace |
| C06-08 readers/publication | Token-authenticated original/inverse publication around a different owner's move; old original receipt cannot reapply the compensated edit; independent final USD reader checks literal fields/quantities |
| Complete/rebased C01 inverse | `apps/beamcraft-cli/tests/rebase/compensation_changesets.rs`: whole multi-command changeset inverse and overlapping refusal; undo of B's rebased move keeps A's accepted name and a third workspace's later edit |

## Reproduction and recorded checks

Run the normal required workspace formatting, clippy, tests, docs, module-boundary, fixture,
licence and portable-crate checks from `docs/development.md`. Relevant focused suites are:

```sh
cargo test --locked -p beamcraft-cli --test rebase --test coordinator --test independent_reader --test changeset
cargo build --locked --release -p beamcraft-cli --bin beamcraft-usd-oracle
cargo test --locked --release -p beamcraft-cli --lib offline_tests -- --ignored --nocapture
```

The combined focused suites at implementation PR #97 passed 50 tests, with strict workspace
clippy and CLI documentation. The earlier runtime passed all 350 CLI tests before the later
named acceptance/aggregate additions. Required tip CI remains the integration check; earlier
full successful runs include [C05 #90](https://github.com/axisrow/BeamCraft/actions/runs/39906400797),
[proof #91](https://github.com/axisrow/BeamCraft/actions/runs/39906571899),
[runtime #92](https://github.com/axisrow/BeamCraft/actions/runs/39906829156),
[R01/C01 fix #93](https://github.com/axisrow/BeamCraft/actions/runs/39907751073) and
[C01 proof #94](https://github.com/axisrow/BeamCraft/actions/runs/39908173791).
These include Windows/Linux tests and the explicitly scheduled release stress, portable checks,
licence checks and Linux/macOS cold-release builds.

The complete local release stress passed in 739.47 seconds, including both fresh-process replay
points. Installed official OpenUSD 0.26.5 on Linux read all 2001 identities/placements, independently
summed every mesh to 600 mm³ and passed compliance without errors, failed checks or model warnings.
The SDK emits its separate checker-deprecation notice. Set `BEAMCRAFT_C05_OUTPUT` to a nonexistent
directory to retain the exact stores/materialized layers, then use that SDK's Python interpreter:

```sh
python tools/check-c05-openusd.py OUTPUT/layers
```

The separate independent oracle originally failed on this real stage at its JSON scratch and
USD-value budgets; its bounded repairs pass the same stage. Production protocol/stage caps were
unchanged. An actual two-binary compatibility check also replayed a pre-aggregate C01 compensation
record with the aggregate runtime while retaining a later local revision and unchanged accepted
head. This is additional interoperability/idempotency evidence, not an ordinary-OpenUSD OS matrix,
native UI acceptance, a power-loss guarantee or an assertion that the fork's PR stack is merged.

Implementation and acceptance merge order: #71 → #72 → #73 → #74 → #75 → #76 → #77 → #78 →
#79 → #80 → #81 → #83 → #86 → #87 → #88 → #89 → #90 → #91 → #92 → #93 → #94 → #96 → #97.
Retarget each dependent PR to `master` after its prerequisites integrate. Every named slice
stays within the issue's 1000-line limit; creating these PRs does not authorize merging them.

All added models, references, tokens and reader drivers are contributor-original MPL-2.0 under
the existing fixture attribution. No commercial asset, external SDK dependency, GPL/AGPL code
or new runtime network access is introduced.
