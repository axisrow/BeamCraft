# BeamCraft recorded test gate

Development-only Rust tool, MPL-2.0, never shipped with the applications. The
[owner policy](../../docs/plan/test-gate.md) replaces the literal-Cargo rule from #346.
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
name. Doctests still run after ordinary test failures when preparation succeeded. Earlier
failures are retained even when a later command passes. Final runs never use the `fast` profile.

One crash-safe OS file lock is shared by worktrees through the user's cache directory (XDG
cache, LocalAppData, or home). Waiting does not consume test execution time. Use this runner
instead of concurrent manual heavy Cargo builds. Ctrl-C terminates supervised descendants and
leaves diagnostics; the lock is automatically released by the OS.

The output path printed on success is the versioned report under `target/test-gate/`. Each run
has a new directory with stdout/stderr logs and fresh JUnit evidence. `--report PATH` chooses
a new output directory. Reports include the exact commit/tree/base, native OS/architecture,
scope, tool versions, executed/expected test counts and individual stage statuses/times.
Explicitly ignored tests are listed as coverage limits, not represented as executed passes.
Existing feature deferrals such as #763 are not silently re-enabled by this maintenance change.

## Publication and CI

Publish only after the exact committed snapshot passes:

```sh
cargo run --locked -p beamcraft-test-gate -- publish --report target/test-gate/RUN/report.json --pr NUMBER
```

`--repo owner/name` overrides the GitHub origin selection. Publication patches only the
delimited machine block in the description; it preserves human text and rechecks the current
PR head/base and local clean snapshot. No publication happens during `check`. A new commit or
base requires another local gate and publication. A PR edit retriggers CI selection.

CI selects exactly the two operating systems other than the one recorded locally. Both run
the complete workspace. A local CLI-only receipt deliberately omits UI on that native OS;
the report and CI plan say so explicitly. Missing/stale/incomplete reports fail the plan and
the aggregate `CI verdict`, instead of silently choosing a default matrix. Formatter/docs
run on the first selected quality lane; WASM, licences, fixtures and boundaries remain checked.
Manual CI can name a source PR with the `pr_number` workflow input.

On integration, a merged PR's report is reusable only for an identical source tree. The no-PR
upstream sync uses `check --scope full --staged-merge`, `sync-trailer` before commit and
`verify-sync` before push. The one-line commit receipt binds the staged tree and both actual
merge parents. Do not hand-edit receipts. Release packaging and cold release coverage remain
unchanged.

The artifact builder uses `ci-archive --output DIR`; it supervises doctests and ships the
matching native gate executable beside the archive. `ci-test --archive PATH --partition I/N
--output DIR` validates the partition inventory and fresh JUnit evidence. `ci-plan` and
`ci-verdict` consume the GitHub event/needs context and fail closed.

## Fixed execution budgets

Nextest profiles enforce 120 seconds per test, 180 seconds per CI partition, 300 seconds per
local CLI run and 600 seconds per local workspace run. The doctest command has a 120-second
watchdog, including compilation of examples after dependencies are prepared. Compilation,
downloads and archive extraction are outside nextest's runner-only JUnit time/global timeout;
build commands have a separate 45-minute watchdog. No automatic retries or timeout-as-pass.

Full UI acceptance requires both the matching CLI executable and a native/software adapter.
On headless Linux, install Mesa software Vulkan. Missing prerequisites fail instead of returning
an apparently successful `SKIP`. Run fixtures under their own isolated cache/target directories;
the tool's conformance tests verify real failures, doctests, timeouts, descendant cleanup,
cross-worktree exclusion and a CLI-only workspace whose UI build intentionally refuses to run.
