# USD SDK differential (N03)

Task N03 (Linux wrapper build and SDK-oracle verification,
[Apibim-com/BeamCraft#476](https://github.com/Apibim-com/BeamCraft/issues/476)). This note records
the first differential run of the project's layers against the official OpenUSD SDK tools, the
platform evidence for the wrapper build, and the limits that remain. The checks are reproducible
with the committed harness ([`tools/usd-sdk-differential/run.sh`](../../tools/usd-sdk-differential/run.sh))
on any machine that has the tools; machines without them skip the SDK legs explicitly and claim
nothing.

Earlier notes record this check as unavailable: U02 ("External `usdchecker`: **not run**. No
`usdchecker` ... executable found") and the layer-engine README ("**not run**, no offline
executable found"). The environment changed: recent macOS releases ship Apple's packaging of the
OpenUSD tools as a system component, so the executable exists on current macOS hosts.

## Environment of the recorded run (2026-11-05)

| Component | Value |
|---|---|
| Host | macOS 27.0.1 (26A434), arm64 |
| Official SDK tools | `Apple USD Tools (0.25.11)` — `/usr/bin/usdcat`, `/usr/bin/usdchecker`, linked against `/usr/lib/usd/libusd_ms.dylib` (system libraries, plugin set under `/usr/lib/usd/usd`) |
| Engine backend | `openusd` 0.8.0 from crates.io behind `beamcraft-layer-engine` (the F11 variant C decision; TOST-1.0 scoped exception per `AGENTS.md` §1.2, recorded in `ATTRIBUTION.md`) |
| Independent reader | `beamcraft-usd-oracle` (std-only Rust, contributor-original parser/composition/algebra), built from `e80699ec` |

The `--version` string names Apple's packaging, not an upstream OpenUSD release or commit; it is
recorded as-is. Whether this packaging counts as "the official OpenUSD SDK" for N03's acceptance
is an owner question recorded in the issue.

## Method

The harness runs committed synthetic fixtures — contributor-original, nothing derived from a real
export or a third-party model — through two readers:

1. **The independent oracle** (`beamcraft-usd-oracle revision|reference`): parses the canonical
   text with its own bounded parser, composes the stack and arcs, recomputes volumes, bounds and
   placements, and cross-checks a closed-form expectation.
2. **The official tools**, when installed:
   - `usdcat --flatten` composes the same stack and arcs; asserted lines (and asserted-absent
     lines) pin the composed result;
   - `usdchecker` runs the official compliance checker on the positive stages;
   - a USDC round-trip serializes the canonical text to the official binary form and reads it back;
   - the refusal matrix replays three corrupted variants through both readers.

Fixtures: `positive-revision` (owned `root/authored/derived` stack, one plate with authored inputs,
an instanceable internal reference arc to a shared geometry asset, pinned material and profile
definitions, closed-form volume/bounds/mass) and `positive-reference` (R01-style wrapper with an
external reference to a foreign Y-up metres layer plus the units correction ops). Negative
variants replace `root.usda` with a committed corruption: a bad layer header, an unknown layer
metadata key, an unknown `xformOp` name.

## Results of the recorded run

All 30 checks passed (`tools/usd-sdk-differential/run.sh`, exit 0).

- **Composition agreement.** The official reader composes the owned stack root-strongest: a
  probe attribute authored on `/World` in `root.usda`, `authored.usda` and `derived.usda` flattens
  to the root's value only — the same order the oracle applies. Flattening bakes the internal
  arc (the shared `Mesh` appears under the element), keeps `instanceable = true`, preserves the
  authored `xformOpOrder` and values, and drops the weaker sublayer opinions.
- **External reference.** The official reader composes the wrapper's foreign subtree (foreign root
  type through the holder, foreign attributes below it) exactly as the oracle reads it; the
  corrected placement matches the closed form with a worst point error of `6.4e-14` mm
  (double-precision rounding of the −90° rotation at 2000 mm scale; oracle tolerance 1e-6).
- **Official compliance.** `usdchecker` reports success on both positive stages.
- **Binary form.** The official writer serializes the canonical text to USDC and the official
  reader opens the result (both fixtures).
- **Refusal matrix.** A `#usda 2.0` header is refused by both readers. An unknown layer metadata
  key and an unknown `xformOp` name are accepted by the official reader and refused by the
  oracle — the documented strictness of the structural profile (U01: "strictness here is the
  structural compliance check"), not a defect. The harness asserts this matrix so a future change
  on either side is caught.

## Linux wrapper build

The wrapper (`beamcraft-layer-engine`, `beamcraft-steel`) is pure Rust with no target-specific
code (`crates/layer-engine/src` has no `target_os` branches). Build evidence on Linux comes from
the fork's CI, which runs on every pull request without any platform-specific gating:

- `quality (ubuntu-latest)`: `cargo clippy --workspace --all-targets --locked -- -D warnings` —
  the whole workspace, wrapper included, compiles warning-free on Linux;
- `build test archive (ubuntu-latest)` plus four `test (ubuntu-latest, shard N/4)` jobs: the full
  workspace test set is built and executed on Linux, including the layer-engine reference tests
  (real `openusd` 0.8.0) and the CLI independent-reader tests that drive the oracle;
- `cold release (x86_64-unknown-linux-gnu)`: a compiler-cold release-profile build of the
  workspace on Linux runs on every pull request.

The PR carrying this note is itself the evidence: its ubuntu jobs build and test the wrapper on
Linux (links in the PR description and the issue).

## Limits

- **SDK availability.** The official tools exist only on this macOS host. No Linux or Windows
  host in this environment has them, and the CI images do not, so every SDK leg there records an
  explicit SKIP banner: such runs verify the oracle only and make no SDK claim.
- **Provenance.** The tools are Apple's system packaging; the exact upstream OpenUSD release
  behind `0.25.11` is not printed by the tool and was not probed further.
- **Scope.** The differential covers the canonical text profile the writer emits plus the refusal
  matrix; the oracle reads text layers only, so the binary USDC leg exercises the official tools,
  not the oracle. The differential is read-side evidence, not the U02 engineering validation and
  not a claim about meshes the writer cannot emit.
- **Platform digests (LEW-05).** The second half of LEW-05's remaining evidence — comparing
  deterministic authored bytes/digests across platforms — was not run here; it needs a second
  platform producing digests, which this environment does not have locally.
- The fixtures are synthetic and contributor-original; they prove reader agreement on the
  composition semantics, not anything about real project content.

## Reproducing

```sh
tools/usd-sdk-differential/run.sh          # SDK legs run when usdcat/usdchecker are installed
tools/usd-sdk-differential/run.sh --no-sdk # oracle legs only; prints an explicit no-claim banner
```

The harness builds `beamcraft-usd-oracle` from the committed source (`cargo build --locked`),
needs no network, and writes only under the system temp directory.
