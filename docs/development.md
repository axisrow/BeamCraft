# Development

Requires stable Rust 1.96 or newer (`rustup update stable`).

## Build and test

```sh
cargo build --workspace
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p beamcraft-cli -- --version
```

### The full gate

The [recorded gate policy](plan/test-gate.md) supersedes the literal-Cargo rule from #346.
Commit the source, fetch the integration base, and run:

```sh
cargo run --locked -p beamcraft-test-gate -- check --scope full
# Independent CLI-only changes, with unchanged UI-consumed commands/options/JSON:
cargo run --locked -p beamcraft-test-gate -- check --scope cli --ui-independent-reason "..."
```

The runner performs non-mutating format validation, Clippy, an unfiltered nextest 0.9.149 run,
mandatory doctests and documentation with `RUSTDOCFLAGS="-D warnings"`. CLI-only checks select
all CLI targets without compiling the UI. Shared crates, UI, dependency manifests/lockfiles,
build settings and shared contracts require `full`; uncertain changes use `full` as well.

The source must be committed and unchanged throughout verification. One native file lock
serializes heavy local gates across worktrees; do not launch other manual heavy builds beside
a gate. Logs and the versioned report are retained under ignored build output, with original
exit statuses and fresh inventory/JUnit evidence. Ordinary-test failures do not suppress
doctests when preparation succeeded. A later success never erases an earlier failure.

Execution budgets are 120 s per test, 180 s per CI partition, 300 s per local CLI run, 600 s per
local workspace run and 120 s for the doctest command. Nextest timing excludes compilation,
downloads, archive extraction and runner wait. Doctests include compilation of examples after
dependency preparation. Timeouts fail and terminate descendants; no automatic retries or
budget increases. See [the runner guide](../tools/test-gate/README.md) for installation,
publication, sync and diagnostics.

Publish the successful report explicitly:

```sh
cargo run --locked -p beamcraft-test-gate -- publish --report target/test-gate/RUN/report.json --pr NUMBER
```

CI validates the current head/base/tree and checks the whole workspace on the two other OSes.
A CLI-only receipt explicitly means UI was omitted locally. Missing/stale reports fail the
aggregate `CI verdict`. Integration reuses a PR result only for the identical source tree;
upstream sync binds its full native gate to the merge tree. Release packaging is unchanged.

### Iteration runs

[cargo nextest](https://nexte.st) runs ordinary tests both in the recorded gate and during
iteration. Install the pinned version with `cargo install cargo-nextest --version 0.9.149 --locked`.
A standalone nextest invocation does not include doctests or produce the gate receipt.

```sh
# Inner loop: workspace libs and the quick CLI suites (~1 min warm on the reference Apple M5).
cargo nextest run --profile fast

# One crate, suite or test while working on it.
cargo nextest run -p beamcraft-cli -E 'test(/^numbering::/)'
```

Use `--no-fail-fast` for whole-workspace iteration sweeps. Use the recorded runner for the
final result, including doctests and source identity.

The `fast`
profile (`.config/nextest.toml`) skips only the integration suites that each measure at least
~40 s of serial time on the reference Apple M5 (baseline 2026-10-25): in
`beamcraft-cli` `export_ifc`, `numbering_approval`, `numbering`, `libraries`, `environments`
and `pick`; in `beamcraft-host` `recovery`, `revision` and `compaction`; in `beamcraft-kernel`
`g09_budget_fuzz` and `g05_profile_extrusion`. The render/viewport suites in `beamcraft-ui`
measured fast enough to stay in the inner loop. Nothing is removed or permanently skipped: the
unfiltered `default` profile and CI always run everything, and an explicit command-line filter
overrides the profile's `default-filter`.

The `beamcraft-cli` integration suites build into one `cli` test binary (`tests/cli.rs`, one
module per suite file), so they link once and `cargo test` runs all their cases on one thread
pool. Run one suite with `cargo test -p beamcraft-cli --test cli -- <suite>::` or
`cargo nextest run -p beamcraft-cli -E 'test(/^<suite>::/)'`; list a new suite file in
`tests/cli.rs`. `macos_icon` checks the desktop app's icon and stays its own target.

The drawn-grid viewport controller test requires a native wgpu adapter and fails explicitly if
none is available. On a headless Linux worker, Mesa's software Vulkan driver (`mesa-vulkan-drivers`)
can provide it; a physical GPU or display server is not required. The Vulkan loader must be able
to find the driver and its ICD configuration. Build `beamcraft-cli` before running the UI's
real-binary grid integration tests, which also fail when that executable is missing.

### What each layer tests

- **Unit tests** sit as `#[cfg(test)]` modules next to the code and pin engine invariants:
  millimetres and radians inside the engine, right-handed coordinates with Z up, conversion
  only at the UI and IO edge.
- **Integration tests** live in a crate's `tests/` directory and drive its public API only.
- **CLI end-to-end tests** drive `beamcraft-cli` (headless first, `AGENTS.md` §3): a
  user-facing feature is asserted through a real command — stdout, stderr, exit code and the
  store it writes.
- **UI tests** are screenshot tests through `run_ui` and the `egui_wgpu` readback; kittest is
  not used because it pulls in accesskit, which is disabled by owner decision.
- **Doc tests** keep the `///` examples of the public API compiling.
- **Schema stability:** a new `ElementKind` must also update `SCHEMA_DESCRIPTOR` and pass the
  strips screenshot tests in `beamcraft-ui`. The kind registry is closed: adding or removing a
  kind needs a new vocabulary version ([F10 vocabulary
  contract](contracts/f10-domain-vocabulary-v0.1.md), section 2.1).

### Acceptance first

The acceptance criteria of a feature or a fix are formulated before the implementation: the
observable fact that proves the work, and a method proportionate to it — a visual change by a
real screenshot, an interactive element by a real click, a CLI change by the command's stdout,
stderr and exit code, business logic by a behavioural test of the new effect. Tests cover
behaviour and refusals, not a getter beside its setter; invalid input produces an error, never
a panic, and the tests assert that. Fixtures stay deterministic: seed the RNG before creating
the workspace, keep temporary names unique per process (the fixture-name collision fix,
PR #157), and never wait with sleeps — wait on the condition.

## Faster test execution

The test profile optimizes only `beamcraft-geom`, `beamcraft-model`, `beamcraft-kernel` and
`openusd` at level 1. These libraries handle the repeated geometry certification, blob hashing
and USD parsing in the integration tests. Other workspace packages, GUI dependencies and host
build tools keep Cargo's default optimization settings. Debug assertions and overflow
checks remain enabled, and the test cases, fuzz seeds, iteration counts and independent oracles
are unchanged. The first test build recompiles the selected libraries and their dependants;
subsequent runs reuse those artifacts. Development and release profiles are unaffected.
`cargo test --workspace` remains supported and uses the same optimized test libraries.
CI's test archives and every selected platform partition use this test profile automatically;
no partition or platform is excluded.

Compare runtime on the same machine and toolchain after building both variants with
`cargo test --workspace --no-run --locked`. Time the subsequent `cargo test --workspace --locked`
commands separately from compilation. To reproduce the unoptimized baseline without editing
the manifest, pass all four overrides to **both** the build and timed commands:

```sh
cargo test --workspace --locked \
  --config 'profile.test.package.beamcraft-geom.opt-level=0' \
  --config 'profile.test.package.beamcraft-model.opt-level=0' \
  --config 'profile.test.package.beamcraft-kernel.opt-level=0' \
  --config 'profile.test.package.openusd.opt-level=0'
```

## Optimized local builds and cold timings

For an optimized local build of both applications:

```sh
cargo build --release --locked -p beamcraft-cli -p beamcraft-ui
```

The `release` profile keeps workspace code at optimization level 3, uses 16 codegen units and
disables LTO; third-party dependencies use optimization level 0 to reduce compile time. The
`dist` profile keeps application and dependency runtime code at optimization level 3, thin LTO
and one codegen unit for packaged builds; host build tools keep Cargo's defaults. Both profiles
use the same features and runtime source code. Use `--profile dist` to reproduce package
optimization locally; its executables are under `target/dist/` (or `target/<triple>/dist/` with
an explicit target).

Local dependency code prioritizes compile time over throughput; use `dist` for performance
measurements and distribution. Release runtime checks are unchanged; workspace engineering code
remains at level 3. Explicit workspace package overrides keep those crates at level 3 without
raising the optimization cost of Cargo's host build scripts or proc macros.

Measure compiler-cold builds with the helper, from a Bash shell on Linux, macOS or Windows:

```sh
bash tools/build-timings.sh --target aarch64-apple-darwin --output target/timings-after
# Optional comparison with package optimization:
bash tools/build-timings.sh --profile dist --target aarch64-apple-darwin --output target/timings-dist
```

Omit `--target` for the host target. Each output directory must be new. The helper uses an empty
target directory inside it and disables both Cargo compiler wrappers. It preserves `build.log`,
`metadata.txt`, the standalone `cargo-timing.html` report and build artifacts; it never clears the
normal target directory. Artifacts can occupy several GB. The registry download cache is reused,
and missing dependencies are downloaded during the timed command. Record whether downloads were
already cached when sharing a result. For a compile-only comparison, run `cargo fetch --locked`
first for both revisions and disclose that prefetch.

Compare before and after on the same machine, target, Rust toolchain, build-job count and registry
state, with no custom Cargo profile overrides or Rust flags. The helper records the revision,
working-tree status, platform, CPU, toolchain, command, exit status and elapsed seconds. Attach the
HTML and log when reporting timings; the HTML shows crate durations and the remaining critical
path. A Linux x86_64 measurement does not establish the goal in
[Apibim-com/BeamCraft#153](https://github.com/Apibim-com/BeamCraft/issues/153) of at most two minutes
on the reference Apple M5 for the `aarch64-apple-darwin` build. That acceptance requires the same
cold command on that machine, workspace tests, and opening the window against a demo store.

## Reusing dependencies

Ordinary Cargo commands reuse the local target directory. For several worktrees, optionally set
`CARGO_TARGET_DIR` to a persistent directory outside them. Cargo separates profiles and target
triples there; artifacts for different target ABIs still need separate compilation. Unset this
variable when following the packaging commands below, which use the repository's `target/`.

An optional installed `sccache` can reuse dependency compilation after the target directory is
removed:

```sh
cargo install sccache --locked
RUSTC_WRAPPER=sccache cargo build --release --locked -p beamcraft-cli -p beamcraft-ui
sccache --show-stats
```

In PowerShell, set `$env:RUSTC_WRAPPER = 'sccache'` for the build and remove it afterward.
No wrapper is required by the repository. Cache hits depend on the compiler, target and flags;
they help repeated builds and do not replace a genuinely cold benchmark. CI already uses
`Swatinem/rust-cache` for registry and target dependencies, with separate WASM and distribution-target
keys.

The cold release CI jobs on Linux and macOS deliberately skip compiled caches and upload their
timing HTML, log and machine metadata as `build-timings-<triple>` artifacts. Downloads happen before
the measured command. These artifacts document compiler-cold builds on the hosted runners;
reference-machine acceptance still needs the Apple M5 run described above.

## Everything CI checks

CI (`.github/workflows/ci.yml`) runs on pull requests, pushes to `master` and manual dispatches.
A fresh local report chooses the two other operating systems; both test the whole workspace.
The first selected quality lane runs format/docs checks, with WASM, licences, fixtures and
boundaries retained. New commits/base snapshots need a new local report. The commands below
show the component checks; use the recorded runner for an eligible final result. To run the
same checks locally:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
cargo test --workspace --doc
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo run -p beamcraft-boundaries -- check
cargo check --target wasm32-unknown-unknown $(cargo run -q -p beamcraft-boundaries -- portable)
cargo deny check
```

- **Module boundaries.** `beamcraft-boundaries check` compares the crates' dependencies with the
  table in [`contracts/f08-module-boundaries-v0.1.md`](contracts/f08-module-boundaries-v0.1.md); it also
  runs inside `cargo test --workspace`.
- **WASM.** `beamcraft-boundaries portable` prints the crates that must build for WASM
  (`-p beamcraft-geom -p beamcraft-model -p beamcraft-kernel`). In PowerShell, run it first and
  pass its output to `cargo check`; set `$env:RUSTDOCFLAGS = '-D warnings'` for the docs command.
- **Tools.** The WASM check needs `rustup target add wasm32-unknown-unknown`; the dependency check
  needs `cargo install cargo-deny --locked`. A check that could not run is not a passed check.
- **Offline.** Add `--offline --locked` to Cargo commands to work without downloads; never update
  `Cargo.lock` as a side effect.

The independent-reader gate runs in every `cargo test`: run
`cargo test -p beamcraft-cli --test cli -- independent_reader:: reference_reader:: --nocapture`.
The `beamcraft-usd-oracle` process re-reads the committed layer text with its own parser,
composition and vector algebra and cross-checks the closed forms; a failure fails the tests, and
no SDK or interpreter is needed. The exact contract case inventory, asserted fields and remaining
deviations are in [K1 acceptance evidence](design/k1-contract-acceptance.md).

## CI wall time and Windows runner queues

The historical full master runs in the fork observed on 2026-10-30–31 had a median wall time
of 6 min 27 s (14 successful samples, range 5 min 45 s–9 min 33 s). The new CI selection runs
the two operating systems other than the locally verified one, so its wall time depends on
that pair and queue conditions. These are observations, not execution-budget thresholds.
The recorded gate enforces runner-only test budgets separately from compilation and queue
wait. Existing job watchdogs (5–45 minutes) still bound hung setup/build commands.

Windows jobs queue for runners that the account shares across all of its repositories and
workflows, and hosted Windows runners are the scarcest standard tier: each Windows minute is
billed at twice the Linux rate, and plan concurrency counts Windows and macOS jobs against a
smaller allowance. During a burst — several pull requests, merges and retries at once — a Windows
job can sit in the queue for hours with no failure signal, because the run reads "queued", not
"failing". On 2026-10-21, run
[39648960935](https://github.com/Apibim-com/BeamCraft/actions/runs/39648960935) waited about
5 hours 40 minutes for a Windows runner while the rest of its jobs had long finished.

Two workflow-level guardrails bound our own contribution to such bursts; neither widens the
account quota:

- The `concurrency` group `ci-<ref>` with `cancel-in-progress` keeps at most one active CI run per
  branch or pull request; a new push cancels that ref's superseded run instead of queueing it.
- Per-job `timeout-minutes` fail fast when a job hangs after it starts. They do not bound queue
  time, and no workflow setting does: the 6-hour default is itself a per-job execution timeout
  that starts when a runner picks the job up. Run 39648960935 finished green after 6 hours
  4 minutes of wall time, queue included.

When CI looks stuck, first check whether the Windows job is queued rather than running, then look
for a burst across the account's Actions runs, cancel superseded runs and re-run the starved job
once the burst subsides.

The scheduled `CI runner queue alert` workflow checks queued jobs every 15 minutes, including
jobs waiting inside workflows that have already started. If any job has waited for at least 15
minutes, it opens one issue labelled `ci-queue` with the affected refs, jobs and run links. While
that issue remains open, later checks do not create duplicates. Close it after the queue drains; a
later incident then opens a new alert. The workflow uses the repository's `GITHUB_TOKEN` with
Actions read, Contents read (for checkout) and Issues write permissions.

## Releases

Owner policy (2026-10-27, [decision](plan/decisions.md#2026-10-27)): Windows x64 and ARM64
releases are built on the owner's computer and signed there with the company's signing identity.
Apple developer registration, Developer ID signing and notarization are scheduled after the MVP.
This records the intended release process; it does not add ARM64 or company-signing automation
to the workflow below. Existing macOS ad-hoc package signing remains unchanged.

Push a tag such as `v0.1.0`, or run `.github/workflows/release.yml` by hand. It builds unsigned
packages and attaches them to a GitHub release:

| Platform | Target | Package |
|----------|--------|---------|
| Windows | `x86_64-pc-windows-msvc` | `beamcraft-<tag>-x86_64-pc-windows-msvc.zip` |
| Linux | `x86_64-unknown-linux-gnu` | the same layout as a `tar.gz` |
| macOS | `aarch64-apple-darwin` | `BeamCraft.app` in a `tar.gz`, ad-hoc signed; Gatekeeper warns on a fresh download |

Every package holds `beamcraft`, `beamcraft-ui`, the Inter font with its
OFL-1.1 licence (in the .app under `Contents/Resources/`) and the project documents.
Build the package executables and package them:

```sh
cargo build --profile dist --locked --target <triple> -p beamcraft-cli -p beamcraft-ui
bash packaging/package.sh <triple> <win|linux|macos> <tag>
```

The script uses the `dist` output; `packaging/Info.plist` is the .app template. CI extracts each
archive into a fresh directory and runs `beamcraft --version` from it.

Manual check on each OS: extract the package, run `beamcraft --version`, start `beamcraft-ui` and
confirm the window opens with text (without the font the window has no text).

Every platform uses one application icon: the official logo `docs/brand/beamcraft-logo.svg` on a
rounded tile, committed as `assets/icons/beamcraft-icon.png` (edge to edge, for Windows and
Linux), `beamcraft-icon-macos.png` and `BeamCraft.icns` (on Apple's grid), `beamcraft-windows.ico`,
its compiled resource `beamcraft-windows.res` (linked into `beamcraft-ui.exe` by the crate's
`build.rs` on MSVC targets, so no resource compiler is needed; the patched eframe sets it as
the window icon) and `beamcraft-linux-256.png`.
Release packaging needs no image tools; `assets/icons/README.md` describes the files. Release smoke
checks decode the packaged ICNS with `iconutil`. The grid raster ships as
`Contents/Resources/beamcraft-icon.png` in the macOS bundle and the edge-to-edge raster as
`assets/icons/beamcraft-icon.png` in the Windows/Linux archives: while `beamcraft-ui` runs, the Dock and taskbar show the process
icon, so the UI loads that file and applies it, falling back to the coded I-section when it is
missing. `cargo test -p beamcraft-cli --test macos_icon` checks the committed ICNS structure
without a Rust build of the UI or macOS. On macOS, also confirm the icon in Finder, the Dock and
the app switcher. The brand terms in `docs/brand/LICENSE-brand.txt` apply to the icon.
