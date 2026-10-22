# Development

Requires stable Rust 1.96 or newer (`rustup update stable`).

## Build and test

```sh
cargo build --workspace
cargo test --workspace
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p beamcraft-cli -- --version
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

CI (`.github/workflows/ci.yml`) runs on every pull request and every push to `master`, on Windows
and Linux. To run the same checks locally:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
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

For K1 acceptance, set `BEAMCRAFT_REQUIRE_PXR=1` and run
`cargo test -p beamcraft-cli --test pxr_oracle -- --nocapture`. Optionally set
`BEAMCRAFT_PXR_PYTHON` to the installed Python interpreter with `pxr`. This mode fails if the
independent reader is unavailable; the default developer mode prints an explicit skip instead.
No interpreter or SDK is installed by the tests. The exact contract case inventory, asserted
fields and remaining deviations are in [K1 acceptance evidence](design/k1-contract-acceptance.md).

## Releases

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

The macOS bundle uses `assets/icons/BeamCraft.icns` for Finder and the Dock. Its original
artwork is `assets/icons/beamcraft-macos-1024.png`: the official BeamCraft mark composed
on our own tile (`assets/icons/macos-tile.svg`). Regenerate the artwork, the standard/Retina
icon set and the committed ICNS with `bash packaging/generate-icon.sh`; the script runs
`packaging/generate-icon.py`, which needs locally installed Python packages (Pillow and
CairoSVG) and is never run by CI. The prebuilt ICNS is committed so release
packaging needs no image tools. Release smoke checks decode the packaged ICNS with `iconutil`.
The same PNG ships inside the bundle as `Contents/Resources/beamcraft-icon.png`: while
`beamcraft-ui` runs, the Dock shows the process icon, so the UI loads that file and applies
it (`eframe::icon_data::from_png_bytes`), falling back to the coded I-section outside a
bundle. Run `python3 packaging/test_package.py` (stdlib-only, no dependencies) to check the
icon and archive wiring without a Rust build. On macOS, also confirm the composed icon
appears in Finder, the Dock and the app switcher. The mark and `docs/brand/beamcraft-logo.svg`
are the official BeamCraft logo (see `ATTRIBUTION.md`); the brand terms in
`docs/brand/LICENSE-brand.txt` apply to them.
