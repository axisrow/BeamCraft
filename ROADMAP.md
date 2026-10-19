# Roadmap

**Stage:** pre-alpha, first vertical slices. Last updated 2026-10-18.

The CLI authors plates, straight rectangle-section members, definitions, assemblies, minimal
welds and read-only references into verified USD revisions, with workspaces, undo/redo and a
local loopback coordinator; the native window shows a revision and supports picking and snaps.
Most merged slices are marked experimental in their design notes: "slices merged" below means
code is on `master`, not that the gate's acceptance is claimed.

| Gate | Acceptance result | Packages | Status |
|------|-------------------|----------|--------|
| K0 | Verified base, requirements, clean-room boundaries, contracts and original fixtures | F01–F09 | in progress; contracts are freeze candidates or drafts |
| K1 | CLI creates a plate and atomically saves a complete revision readable by ordinary OpenUSD | U01–U07, G01–G02, P01 | slices merged; acceptance open |
| K2 | Straight beam/column, quantities, independent copy, personal workspace and local undo | P02–P08 | slices merged (rectangle sections only); acceptance open |
| K3 | Foreign USD references remain read-only; supported measurements are separate results | R01–R03, G08 | R01 merged; R02–R03, G08 open |
| K4 | Two clients safely publish, rebase, detect conflicts, retry and compensate accepted changes | C01–C06 | C01–C02 merged; C03–C06 open |
| K5 | Original native UI uses engine commands and saves the same result as the CLI | V01–V06 | V01–V04 merged (view, pick, snap); V05–V06 (tools, properties) open |
| K6 | Supported assemblies, cuts, holes, bolts, welds, approved numbering and IFC/BOM export | A01–A09, E01–E03 | A01, A06 merged; A04 pilot holes; E01 contract draft; rest open |
| K7 | Complete alpha scenario and native packaging rehearsal | Q01–Q02 | planned |
| K8 | First associative drawing projection, dimension and published document version | D01–D03 | planned |

## MVP scope (owner decision 2026-10-18)

The next goal is a free single-user desktop MVP: an engineer models a simple steel frame in the
native UI and takes it out as IFC and a bill of materials. Work is ordered by this list; package
boundaries follow the implementation plan and each package keeps its own PR.

1. **Real sections:** I-section families and other rolled shapes as definitions with kernel
   extrusion (A07, with G05), and contour plates (A02). Profile data stays original or from
   public standards (`AGENTS.md` §1.2); no commercial catalogue is copied.
2. **Editing in the UI:** create, select, move and delete through the same engine commands as the
   CLI (V05), and the properties and publication UI (V06).
3. **Detailing basics:** plane cuts (A03), holes to A04 acceptance, bolts.
4. **Output:** numbering (A08/A09), IFC export (E02 on the E01 contract) and BOM/reports (E03).
5. **Packaging:** the alpha scenario and native installers for Windows, Linux and macOS
   (Q01–Q02).

Deferred until after the MVP: multi-client rebase, conflicts, retry and compensation (C03–C06),
drawings (D01–D03) and reference measurements (R02–R03). Which parts later stay open source and
which become a commercial product is decided after the MVP; until then everything stays MPL-2.0.

## Next work

F01 records the MPL baseline and aligns these documents with the accepted USD architecture.
F02 (requirements), F03 (OpenUSD distribution audit) and F07 (original fixtures) can proceed
independently. F09 establishes clean-room admission after F02. F04 then fixes the model and command
contract; F05/F06 define worker and geometry contracts before F08 changes module boundaries.

The [F04 model/command contract](docs/contracts/model-and-commands-v0.1.md) and its original
acceptance descriptions are a freeze candidate, not frozen. The owner adopted G-03 to G-05 and
decided the G-01/G-02 local frames; details P-1 to P-3 await confirmation, G-06 and the profile
resource format stay open with F06, and the freeze is the owner's decision. F02 remains a local
analysis artifact. No runtime command is implemented by F04.

The [F06 geometry and viewport contract](docs/contracts/geometry-contract-v0.1.md) is a draft.
It assumes P-1 to P-3 pending owner confirmation and carries a draft G-06 tolerance policy.
No kernel, mesh or query API is implemented.

The first demonstration starts with a rectangular plate and a supported straight member.
Published USD revisions include their computed geometry. Copying, editing and reopening must
preserve the original element and produce independently verified measurements.
Each user-facing feature requires an engine API, CLI command and end-to-end test.

The owner selected variant C on 2026-10-16: pure-Rust `openusd` 0.8.x behind a thin
BeamCraft wrapper is the primary layer engine, aiming for no Python/C++ in the product path.
The official SDK worker remains only an explicit fallback and a test oracle where available;
its F03 admission stays separate. [F11](docs/design/layer-engine-decision-v0.1.md#9-re-scoped-implementation-and-verification-tasks)
re-scopes U01 (bounded Rust open/read), U02 (codeless schemas and authored layers), U03 (UUID and
relationship index) and U04 (shared CLI/host API), with inputs, outputs and acceptance.
[Wrapper requirements](docs/design/layer-engine-wrapper-requirements-v0.1.md) define the N01 gaps
to close. N03 is planned Linux wrapper-build and available-SDK-oracle verification, not done.
In-process versus separate Rust process and the corresponding F05 mapping remain open.
Crate/graph/notice admission and wrapper safety evidence precede integration. No production USD
backend is integrated; the decision does not complete U01-U04, N01 or K1. These gates do not block
independent mathematical requirements or fixtures. Full alpha readiness requires requirement
coverage; the first demonstration covers only a subset.

Later work expands supported operations, licensed data packs, drawings and DSTV NC export.
IFC import and foreign-to-native conversion require a new scope decision. BeamCraft is
desktop-only on Windows, Linux and macOS. Only `beamcraft-geom`, `beamcraft-model` and
`beamcraft-kernel` retain the WASM compatibility requirement as a layering discipline.

## Progress log

- 2026-10-20: synced upstream A08 numbering preview (upstream PR #96) and V05.2 UI edit tools
  (upstream PR #97) into the fork as verbatim cherry-picks (PR #35); with PR #31 the fork
  master matches upstream `master` byte for byte. PR #6 (fork V05.2) is superseded and
  recommended for closing; the numbering WIP is to be re-scoped to A09 over the upstream
  preview.
- 2026-10-19: synced upstream P-Mass (member mass from the exact section area with gross/net
  quantities, upstream PR #94) and E02 minimal IFC4 export on the E01 contract (upstream PR
  #95) into the fork as verbatim cherry-picks plus one merge-commit tolerance fix (PR #31).
  Upstream V05.2 and A08 stay deferred pending the PR #6 comparison and the numbering-WIP
  re-scope to A09; per-feature critique and verdicts are in the PR #31 description.
- 2026-10-18: owner set the free single-user desktop MVP scope above (sections, UI editing,
  cuts/holes/bolts, numbering, IFC/BOM, packaging); C03–C06, D01–D03 and R02–R03 are deferred.
  Gate statuses were brought up to date with `master` at `d893f94`. Documentation only.
- 2026-10-18: merged A01 assemblies (PR #80), C01 changesets (PR #81), V03 native window and
  viewport (PR #82), A06 minimal weld (PR #84), V04 picking and snaps (PR #85), C02 local loopback
  coordinator (PR #86) and the dev-profile optimisation (PR #87). Each is an experimental slice
  described in its `docs/design/` note and in `docs/architecture.md`; no gate acceptance is claimed.
- 2026-10-17: merged P02 definitions (PR #63), P04 quantities and update (PR #66), P03/A04 review
  fixes (PR #71), P05–P08 copy, workspace, undo/redo, move and delete (PR #72), U09 standard USD
  metadata (PR #73), P02.2 section properties (PR #74), the G07 kernel first-scope acceptance record
  (PR #76, accepted with conditions), the E01 IFC export contract draft (PRs #75, #78, #79) and R01
  read-only references (PR #77). P03 straight members and the A04 pilot plate holes landed through
  the stacked P02 branch; A04 acceptance is not claimed.
- 2026-10-16: merged G01 math core (PR #38), G02 rectangular plate (PRs #39, #41), the F11 variant
  C decision (PR #42), the desktop-first WASM scope (PR #44), U01 bounded read (PR #43), the V01 UI
  stack audit (PR #45), N04 truck spike (PR #46), U02 authored layers (PR #47), the USD process
  integration note (PR #48), U04 CLI host (PRs #49, #52), the UI font (PR #50), V01 viewport spike
  (PR #51), U03 UUID and relationship index (PR #53), V02 view snapshot (PR #54), G05 profile
  extrusion (PR #55), G06 topology and measurements (PR #56), U06 revision commit (PRs #57, #62),
  G09 pilot cuts (PR #58) and the zip advisories update (PR #59). U07 journal recovery (PR #60)
  and P01 first plate (PR #61) landed through the stacked U06 branch.
- 2026-10-15: merged the G01 math-crates spike (PR #33), N02 IFC-lite spike (PR #34) and the
  roadmap log rule (PR #35).

- 2026-10-15: N01 option C first slice ran the pure-Rust `openusd` 0.8.0 spike in `spikes/n01-openusd` (isolated workspace,
  stacked on PR #22). On Windows with Rust 1.96.0 it showed USDA/USDC round trip with unknown-field preservation, sublayer/over/active
  composition, delta capture and replay with `(UUID, attribute)` collision detection, and 1184 hostile inputs with no panic or hang.
  The crate does not prevent local overrides of foreign prims (a BeamCraft wrapper must); non-finite numbers and some truncated or
  corrupted input are accepted. G01 is `failed` as measured (TOST-1.0 generated file in an MIT-declared crate; accepted by owner decision, PR #29); MSRV 1.96 versus 1.95, WASM,
  Linux/macOS and the SDK oracle are not run, so all other claims stay `not_run`. No backend, MSRV or architecture change is
  accepted; owner decisions on MSRV and maintenance remain. Report: `docs/design/layer-engine-spike-n01-report-v0.1.md`.

- 2026-10-15: U05 added original incremental SHA-256 and strict full ContentDigest identities in
  `beamcraft_model::digest`, plus native `beamcraft_host::storage` with synced temporary files,
  non-replacing hard-link publication, complete existing-byte comparison and verified retrieval.
  Base `501e9d476dc187b63e6fd61e2908d1c51ea280d9`, stacked on F08 PR #17. All seven required
  local commands passed; 112 workspace tests passed. WASM and cargo-deny were not run.
  Strict containment against hostile concurrent directory replacement remains open; the current
  implementation requires a protected root namespace. U04 owns the CLI; U06/U07 own revision
  publication and recovery. U05 acceptance and K1 are not complete.

- 2026-10-15: Owner decision on OpenUSD-derived TOST-1.0 material in the `openusd` crate (0.8.x,
  file `src/usd/core_schemas.rs`): it stays under its upstream licence and copyright, BeamCraft
  claims no rights in it, and BeamCraft's own code stays MPL-2.0. `AGENTS.md` section 1.2 gains a
  scoped extension; `NOTICE` carries the upstream notice, `licenses/TOST-1.0-openusd.txt` the
  unmodified upstream licence, `ATTRIBUTION.md` three rows, and `deny.toml` a comment block (no
  allowlist change). The F11 record has a dated note. Documentation only; the crate is not adopted.

- 2026-10-15: F04.3 applied the four F04 amendments proposed by F06 section 12 to the F04 freeze
  candidate on base `b996c58fe850642f12b92b25b23c441fbd41ed47` (stacked on PR #20): the draft
  minimum nonzero dimension of 0.001 mm and the section-dimension cap, the corrected near-vertical
  P-3 roll rule, the resolved-display-reference reading of material-dependent appearance, and
  G-06 and the profile resource format accepted by F06 under its draft policy. Cases `F04-POS-17`
  and `F04-NEG-27` were added (44 logical, 6 raw). F04 stays a freeze candidate at 0.1.0, not
  frozen; P-1 to P-3 still await owner confirmation; nothing is implemented and K0 is not passed.

- 2026-10-15: MSRV raised from Rust 1.95 to 1.96 by owner decision. The `openusd` crate under
  evaluation needs 1.96, and the local and CI toolchains are already at least 1.96. Only the
  `rust-version` in the root `Cargo.toml` and the minimum stated in `README.md` and
  `docs/development.md` change; there is no code change and Rust 1.95 is no longer supported. The
  F01 baseline record keeps its 1.95 wording with a dated note.

- 2026-10-15: F11 drafted the USD layer-engine decision record 0.1.0 on base
  `20d26cfb107d51e7a2c76c25ca0191b8115ec301` (stacked on F08). It compares four options (official
  SDK worker, own Rust subset, `openusd` adapter, hybrid) and recommends retaining the accepted SDK
  worker baseline pending owner-approved spike N01 evidence. The JSON plan defines 40 unexecuted
  claims, ten gates and 12 original scenario families. No architecture, dependency or runtime
  change is made; the recommendation is not a decision. Local WASM and cargo-deny checks were not run.

- 2026-10-15: F10 drafted the domain vocabulary and discipline-module contract 0.1.0 on base
  `20d26cfb107d51e7a2c76c25ca0191b8115ec301` (stacked on F08), consuming F04 0.1.0, F06 0.1.0
  and F08. It fixes a closed element-kind registry (steel `plate`, `beam`, `column` implementable;
  ten kinds reserved with no behaviour), the common base, shared and per-discipline namespaces
  with unknown-content preservation and versioning, a shared relationship vocabulary with a
  penetration request lifecycle, a 52-rule validation catalogue mapped to the existing F04 codes
  (client, host and post-merge stages) and the discipline-module contract, with 149 original
  acceptance descriptions (73 positive, 76 negative). Nothing is implemented, the contract is not
  frozen and K0 is not passed. Local WASM and cargo-deny checks were not run.

- 2026-10-15: G04 drafted the first-scope kernel design 0.1.0 on base
  `29d962e790ffd21ecd9a45143f052d4b4d53d3f7` (stacked on F06), consuming F06/F04 0.1.0 and the
  first-scope package 0.1.0. It covers representation, numeric derivations, predicates, verdicts
  for every F06 draft constant, failure/work bounds and staged implementation guidance. The
  validation plan was first written against the 70 cases of F06 0.1.0 as merged. Review rounds
  against the corrected F06 (PR #20) then reconciled it: 44 rows now cover all 83 F06 cases (47
  positive, 36 negative), amendments A1-A11 are tracked as applied, partly applied or open, and
  its own findings A7-A11 led to F06 corrections. Nothing is implemented, no implementer is
  declared eligible and K0 is not passed.
  Local WASM and cargo-deny checks were not run.

- 2026-10-15: F08 fixed the module boundaries on base `29d962e790ffd21ecd9a45143f052d4b4d53d3f7`
  (stacked on F06). `docs/contracts/module-boundaries-v0.1.md` gives the exact crate/module path,
  allowed dependencies and now/planned status of every F04/F05/F06 area. New code: the pure
  `beamcraft-protocol` crate (F05 frame header, binary prefix, limits, bundle budget, IPC codes),
  `model::ErrorCode` (F04 section 8) and the std-only `beamcraft-boundaries` checker, whose test
  and CI step enforce the module table; the CI wasm crate list is now derived from that table.
  `kernel`, `engine`, `host` and `ui` are planned paths only. No kernel, worker or command is
  implemented; the contracts are not frozen and K0 is not passed. Local WASM and cargo-deny
  checks were not run.

- 2026-10-15: F06 drafted the geometry and viewport contract 0.1.0 on base
  `9d40abc34b8a5686358b78adee5f9c52b9c830bd`, consuming F04 0.1.0, F05 0.1.0 (host boundary
  only) and the first-scope package 0.1.0. It fixes the kernel input and evaluation order, the
  result envelope and diagnostics, a draft G-06 tolerance policy, FeatureId, the mesh blob and
  caps, three separate dependency keys with an invalidation matrix, quantity statuses and the
  logical query and pick API, with 83 original acceptance descriptions (47 positive, 36
  negative). A kernel-design review (G04) led to a first correction round (near-vertical frame,
  numeric-uncertainty diagnostic, pick rules, resource bounds) and a review round that bounded
  the absolute value of each frame dot product. P-1 to P-3 are assumed pending
  owner confirmation. The contract is not frozen, no kernel is implemented and K0 is not passed.

- 2026-10-14: F05 drafted the private USD-worker protocol contract 0.1.0 on base
  `478f330e15819b574c62227bc23009729adb4fdc`, consuming F04 0.1.0 sections 2, 3, 6, 7 and 8.
  It fixes framing, attachments, handshake, correlation, deadlines, cancellation, restart
  limits, transport error codes and the canonical replay profile, with 60 original acceptance
  descriptions (19 positive, 41 negative), including review fixes for candidate discard, host
  outcome queries, bounded diagnostics, event sequencing and timeout budgets. The contract is
  not frozen, no worker is implemented and K0 is not passed.

- 2026-10-15: F04.2 made the model/command contract a freeze candidate (base
  `478f330e15819b574c62227bc23009729adb4fdc`). The owner adopted G-03 (zero-length axis is
  `INVALID_ARGUMENT`), G-04 (quaternion convention) and G-05 (CR-TOL-001 as a test comparison
  rule, with added cases) and decided G-01 and G-02 (plate and member local frames). The detail
  rules P-1 to P-3 await owner confirmation. G-06 and the profile resource format stay open with
  F06. The case file has 42 logical and 6 raw cases. The contract is not frozen, K0 is not passed
  and no command is implemented.

- 2026-10-14: integrated F03 and the PR-workflow policy into `master`. PRs #4 and #5 had merged
  into the F01 branch after F01 itself reached `master`, so their commits were not on `master`.
  Kept both F07 and F03 CI steps, all attribution rows and progress entries; the PR-workflow
  rules became `AGENTS.md` section 7 after the branch-naming section 6. The F03 entries below
  are historical: the TOST question was later decided by PR #7, while notices, native closure
  and runtime checks remain open.

- 2026-10-14: F07 added original synthetic fixtures on base
  `b34205281ab8a1f95052c4dc7d84f0995633e179`. The new std-only dev tool `tools/fixtures`
  (`beamcraft-fixtures`) provides a generator API, a CLI and E2E tests. `tests/fixtures/f07`
  holds 33 fixtures: 10 accepted (one unverified), 2 unsupported and 21 invalid. A manifest
  records full SHA-256 digests. Separately, `tests/fixtures/expectations/f07.json` holds
  hand-derived expectations, derived in `docs/fixtures/expectations.md`, and a Python oracle
  checks them independently. CI verifies the committed bytes. This is test data only: it adds
  no geometry, USD or copy feature and does not pass K0.

- 2026-10-14: F09 added the clean-room admission policy and templates and an admitted
  first-scope package (rectangular plate, straight member, identities, units, placement,
  declared failures) under `docs/clean-room/`. Base `ebb5b928dda3d36d6b6570549c3f745bb1883ff7`.
  Admission covers that limited scope only; broader requirements stay pending and the F04
  contract remains a review draft. Exposure admission is separate and no implementer is declared
  eligible. Windows fmt, Clippy, three tests, docs and CLI checks passed; local WASM and
  cargo-deny checks remain unverified because the target and tool are absent; K0 is not passed.

- 2026-10-14: synchronized the branch-naming policy PR with master after F04 integration;
  retained both attribution rows and all task progress entries when resolving conflicts.

- 2026-10-14: recorded task-type branch prefixes in AGENTS.md and contributor instructions;
  new branches use feat/bug/docs/chore/refactor/test rather than agent/provider names.

- 2026-10-14: synchronized the F04 review branch with master after the OpenUSD policy merge;
  resolved the progress-log conflict while retaining both F04 and licence-decision entries.

- 2026-10-14: drafted F04 model/command version 0.1.0, including full identities, explicit units,
  bounded inputs, typed mutations, revision/generation preconditions, semantic footprints and
  atomic errors. Added original logical acceptance descriptions; F09 and freeze remain open.

- 2026-10-14: owner approved TOST-1.0 only for official OpenUSD and its SDK resources.
  Recorded the scoped exception in AGENTS.md; kept the global Cargo licence allowlist unchanged.
  Provenance, notices, native closure and runtime checks remain prerequisites for integration.

- 2026-10-14: addressed F03 review findings with bounded ZIP directory preflight and text reads,
  plus input/output alias protection. Twelve synthetic audit tests pass; added their Windows/Linux
  CI execution. SDK integration remains gated separately from audit-tool correctness.

- 2026-10-14: established one focused PR per implementation-plan task in AGENTS.md, including
  verified bases, explicit stacked-PR dependencies, English descriptions and no agent co-author trailers.
  Opened separate draft reviews for the MPL prerequisite, F01 and F03; no merge was performed.

- 2026-10-14: F03 pinned and statically audited three `usd-core` 26.8 CPython 3.12 wheels.
  Added an offline inventory tool with seven passing synthetic tests. SDK admission remains open:
  TOST/resource policy, missing native notices, runtime/native closure and platform smoke tests.
  See [the audit](docs/audits/openusd-26.8.md); no runtime dependency or policy exception was added.

- 2026-10-14: F01 aligned the roadmap, architecture and development documentation with the
  accepted USD direction. Implementation base: `c6a1e73aea9c41bf88db96281040043471b80d0a`
  (MPL-2.0); local `master` still pointed to the initial scaffold at audit time.
  Windows fmt, Clippy, three tests, docs and CLI version checks passed. Local WASM and cargo-deny
  checks remain unverified because the target and tool are absent; K0 is not passed.

- 2026-10-14: switched the current source and documentation to MPL-2.0; updated Cargo metadata,
  source notices, contribution rules and release packaging. Third-party licence policy remains
  permissive, with explicit MPL-2.0 exceptions for project-owned workspace crates.

- 2026-10-14: scaffold created: workspace (`geom`, `model`, `beamcraft-cli`), dual MIT/Apache-2.0
  licence, CI on Windows and Linux, release workflow, agent rules.
