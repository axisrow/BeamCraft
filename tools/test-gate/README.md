# BeamCraft recorded test gate

Development-only Rust tool, MPL-2.0, never shipped with the applications. This
runner replaces the literal-Cargo rule.
Requires Rust 1.96+, Git and cargo-nextest **0.9.149**; report publication/CI discovery also
requires an authenticated `gh` CLI. No Python or native GUI dependency is used by this tool.

## Local command

Commit the source first. From any worktree:

```sh
cargo run --locked -p beamcraft-test-gate -- check --scope full
# Only independent CLI source/tests, with optional ordinary Markdown changes:
cargo run --locked -p beamcraft-test-gate -- check --scope cli --ui-independent-reason "The independent console command does not change UI-consumed options or JSON"
```

Use `--base REF` for a stacked PR; the default is `origin/master`. Fetch the base beforehand.
The branch must include its integration base. The gate refuses dirty/untracked source and
checks the snapshot again at completion. It never formats or commits source on your behalf.
Shared crates, UI, Cargo manifests/lockfile, workflow/build configuration and shared contracts
require `full`. The explanation is the developer's assertion about CLI/UI protocol semantics;
file paths alone cannot prove that a changed command is not used by the UI.

The gate runs format validation, Clippy, a compiled nextest inventory, all selected ordinary
tests with `--no-fail-fast`, mandatory doctests, and documentation with warnings denied. CLI-only
selects the complete CLI package, including its real-binary integration targets, not one test
name. A selection without a library target (the CLI package ships none) has no doctests to run:
the doctest step passes vacuously and cargo's refusal stays in the stage's stderr log. Doctests
still run after ordinary test failures when preparation succeeded. Earlier
failures are retained even when a later command passes. Final runs never use the `fast` profile.

One crash-safe OS file lock is shared by worktrees through the user's cache directory (XDG
cache, LocalAppData, or home). Waiting does not consume test execution time. Use this runner
instead of concurrent manual heavy Cargo builds. Ctrl-C terminates supervised descendants and
leaves diagnostics; the lock is automatically released by the OS.

The output path printed on success is the versioned report under `target/test-gate/`. Each run
has a new directory with stdout/stderr logs and fresh JUnit evidence. `--report PATH` chooses
a new output directory. Reports include the exact commit/tree/base, native OS/architecture,
scope, tool versions, executed/expected test counts, individual stage statuses/times and, when
one was set, the job ceiling (see "Load on a shared machine").
Explicitly ignored tests are listed as coverage limits, not represented as executed passes.
Test builds omit debug symbols and incremental state, matching CI; debug assertions and
overflow checks remain enabled.
Child Cargo artifacts use `test-gate/target` below the selected Cargo target directory.
This keeps workspace builds from replacing the running `cargo run` wrapper or locking its
executable on Windows. CI extracts archives into the same directory layout.
Existing feature deferrals such as #763 are not silently re-enabled by this maintenance change.

## Publication and CI

Publish only after the exact committed snapshot passes:

```sh
cargo run --locked -p beamcraft-test-gate -- publish --report target/test-gate/RUN/report.json --pr NUMBER
```

`--repo owner/name` overrides the GitHub origin selection. Publication patches only the
delimited machine block in the description; it preserves human text and rechecks the current
PR head/base and local clean snapshot. No publication happens during `check`. A new commit or
base requires another local gate and publication. A PR edit no longer retriggers CI selection:
`ci.yml` does not trigger on the `edited` event.

CI selects exactly the two operating systems other than the one recorded locally for the
quality lanes of `ci.yml`: format, Clippy, fixtures, boundaries, docs on the first selected
lane, plus WASM, licences and the cold release gate. The complete workspace runs on all three
operating systems in the separate platform-track workflow (`ci-platform.yml`) on every
integration push, nightly and manual dispatch, with the same fixed budgets and evidence; it
does not wait inside the CI wall. A local CLI-only receipt deliberately omits UI on that
native OS; the report and CI plan say so explicitly. Missing/stale/incomplete reports fail
the plan and the aggregate `CI verdict`, instead of silently choosing a default matrix.
Manual CI can name a source PR with the `pr_number` workflow input.

A `pull_request` run starts before its receipt exists, because publication follows the push that
triggered the run. `ci-plan` keeps the event snapshot as the fast path and, when it holds no
receipt current for the run's head and base, polls the live description for at most two minutes
before failing; every attempt keeps the head/base/tree binding, and a window without a receipt is
a failure. Master pushes and manual dispatches read the live description directly. Publication
fires nothing: `ci.yml` does not trigger on the `edited` event, so no receipt-bearing run follows
the first one and no run ends cancelled because of a publish; the first run finds the receipt
through its own grace-window poll.

Make `CI verdict` the required PR check and require branches to be up to date in the integration
branch's GitHub protection/ruleset. The verdict rechecks live head/base before it succeeds;
the repository rule also blocks previously completed green checks after a later base update.
Replace old per-platform CI requirements with the aggregate check; retain unrelated required
checks. An intentionally omitted OS cannot satisfy an obsolete per-platform requirement.
Preserve the repository's existing authorized sync push access when configuring that rule.

On integration, a merged PR's report is reusable only for an identical source tree. The no-PR
sync — an upstream-sync merge pushed straight to the fork's `master` (owner decision
2026-11-04), and the same flow toward upstream — uses `check --scope full --staged-merge`,
`sync-trailer` before commit and `verify-sync` before push. The one-line commit receipt binds
the staged tree and both actual merge parents. Do not hand-edit receipts. Release packaging and
cold release coverage remain unchanged.

The artifact builder uses `ci-archive --output DIR`; it supervises doctests and ships the
matching native gate executable beside the archive. `ci-test --archive PATH --partition I/N
--output DIR` validates the partition inventory and fresh JUnit evidence. `ci-plan` and
`ci-verdict` consume the GitHub event/needs context and fail closed.

## Fixed execution budgets

Nextest profiles enforce 120 seconds per test, 180 seconds per CI partition, 300 seconds per
local CLI run and 600 seconds per local workspace run. The one exception is versioned in the
profiles themselves: `.config/nextest.toml` gives the load-sensitive R01 bounding test
(`reference::a_reply_lists_a_bounded_number_of_foreign_prims_however_many_the_source_has`)
a 240-second budget, because it deliberately attaches a 9,500-prim source; every other test
keeps 120 seconds. The doctest command has a 120-second
watchdog, including compilation of examples after dependencies are prepared. Compilation,
downloads and archive extraction are outside nextest's runner-only JUnit time/global timeout;
build commands have a separate 45-minute watchdog. No automatic retries or timeout-as-pass.

Full UI acceptance requires both the matching CLI executable and a native/software adapter.
On headless Linux, install Mesa software Vulkan. Missing prerequisites fail instead of returning
an apparently successful `SKIP`. Run fixtures under their own isolated cache/target directories;
the tool's conformance tests verify real failures, doctests, timeouts, descendant cleanup,
cross-worktree exclusion and a CLI-only workspace whose UI build intentionally refuses to run.

## Load on a shared machine

A full gate saturates a desktop for a few minutes. Two runner settings keep an interactive
session responsive while the gate keeps every core; neither changes what a gate verifies, and
no budget moves.

- **Children run below interactive work.** Every command the runner starts (format, Clippy,
  nextest, doctests, docs) and everything those commands start inherits it: on macOS the child
  is clamped to utility QoS with `taskpolicy -c utility`, on other Unix systems it runs with
  `nice -n 10`, on Windows with the below-normal priority class. Measured on the shared Mac
  (Apple M5, 4 performance and 6 efficiency cores) with ten CPU-bound processes: a
  normal-priority thread waking every 2 ms waited 3.2 ms at the median without the setting and
  3.0 ms with `nice -n 10`, against 0.5 ms, the idle figure, with the utility clamp, which cost
  ten CPU-bound processes 20% of their throughput in the same experiment. macOS schedules by QoS
  class, so niceness alone is not enough there. `taskpolicy -b` gave the same latency at 28%
  throughput (efficiency cores only) and would not fit the test budgets. The setting keeps the
  owner's session responsive; it does not make a gate faster than a neighbour's build, because
  a lowered gate yields to every normal-priority process, a colleague's build included. Beside a
  saturating targeted test run, one cold gate took 80 s for Clippy instead of 22 s, was still
  building its nextest inventory at 340 s instead of 62 s when it was stopped, and took 90 s for
  documentation instead of 5 s; beside another gate's test stage this tool's own conformance
  fixtures took 50 s and 76 s instead of 2.6 s and 7.7 s, against their 120-second budget. The
  quiet-window discipline stays. The clamp also costs on an otherwise idle machine: there a cold
  gate took 43 s for Clippy, 138 s for its nextest inventory, 323 s for its tests and 520 s in
  total, against 25 s, 142 s, 257 s and 434 s unclamped on the same master, while a
  normal-priority thread kept its idle wake-up latency of 0.5 ms instead of 2-5 ms.
- **`BEAMCRAFT_GATE_MAX_JOBS=N` caps concurrency.** A `check` then limits Cargo's build jobs
  (`CARGO_BUILD_JOBS`) and nextest's test threads (`-j N`) to N. Unset or empty keeps both
  tools' defaults, so nothing changes for anyone who does not opt in. Any other value must be a
  positive integer; a mistyped ceiling stops the gate before it builds anything instead of
  silently lifting it. The report records the ceiling as `max_jobs` when it is set and omits
  the field when it is not, so the receipt of an uncapped gate is unchanged.

The ceiling trades wall time for headroom, and a capped run must still meet the same fixed
budgets. Replaying the per-test times of one full local run (3,335 tests, 225 s with ten
threads) gives about 285 s with eight threads, 365 s with six, 440 s with five and 550 s with
four, against the 600-second budget: four leaves no margin on this machine. A cold run with six
threads and the clamp then measured 386 s for the stage. Leave roughly a third of the logical
cores free, for example `BEAMCRAFT_GATE_MAX_JOBS=6` on ten.

```sh
BEAMCRAFT_GATE_MAX_JOBS=6 cargo run --locked -p beamcraft-test-gate -- check --scope full
```

These settings cover the runner's children. `cargo run` builds the runner first, a short build
outside them; to lower that build as well, start the runner under the same policy, for example
`taskpolicy -c utility cargo run --locked -p beamcraft-test-gate -- check --scope full`.
