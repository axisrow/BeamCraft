# Validation

Final local Linux checks passed on production/test source commit
`820ee15c4ef83e4e35aab080243221ab86979912`; the final documentation commit changes only
validation/evidence records. `Cargo.lock` is unchanged. See [machine-readable results](raw/validation.json).

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS on final Rust source |
| `cargo clippy --locked --workspace --all-targets -j 2 -- -D warnings` | PASS |
| `cargo test --locked --workspace -j 2 -- --test-threads=2` | PASS: 960 tests reported passed, 0 failed, 0 ignored, 83 suites including doc-test harnesses |
| `cargo run --locked -p beamcraft-fixtures -- verify --dir tests/fixtures/f07` | PASS |
| `cargo run --locked -p beamcraft-boundaries -- check` | PASS |
| `python3 tools/fixtures/verify_expectations.py` | PASS |
| `python3 tools/fixtures/test_verify_expectations.py` | PASS: 8 tests |
| `python3 -B -m unittest discover -s tools/audit -p 'test_*.py' -v` | PASS: 12 tests |
| `RUSTDOCFLAGS='-D warnings' cargo doc --locked --workspace --no-deps -j 2` | PASS |
| `cargo check --locked --target wasm32-unknown-unknown -p beamcraft-geom -p beamcraft-model -p beamcraft-kernel -j 2` | PASS |
| `cargo deny check` 0.20.2 | PASS: advisories, bans, licenses and sources; existing unused-license-allowance warnings |
| Performance-tool Python syntax and compressed-data aggregation | PASS |
| Before/after legacy raw `view-export` | PASS: byte-identical, SHA-256 `7d5a987257e8c99813003c335bd9334605c092b04f1f0ab113d569c5654c5757` |

Both release builds passed with Rust 1.96.0. Debug checks used
`CARGO_PROFILE_DEV_DEBUG=0` and `CARGO_PROFILE_TEST_DEBUG=0` to keep build artifacts bounded;
release benchmark settings were unchanged. The official cargo-deny release archive was checked
against its published SHA-256 (recorded in the environment manifest).

New regression coverage passed for bounded refresh envelopes (bad lengths, truncation, trailing
bytes and caps), opt-in CLI bundle/definitions equivalence and invalid options, worker wakeup
after success/unwind, buffer-free scene metadata and resize readback equality against fresh
targets. Existing real-store UI refresh tests also passed.

The real-window before/after smoke displayed 100 elements, one mesh, 1,200 triangles and 4×
MSAA. The sampled 1167×884 model-region RGB bytes matched exactly, with 129,309 lit blue pixels.
In a separate copied store, creating a plate and sending one R to the idle window updated it
without further input: 101 elements, two meshes, 1,212 triangles, generation 165 and the added
plate/new bounds visible. The benchmark store remained unchanged. See
[visual-smoke.json](raw/visual-smoke.json).

Separate offscreen smoke runs used 100 synthetic distinct plate meshes, 30 measured frames
after ten warmup frames, 1280×800, Vulkan llvmpipe and 4× MSAA. Both uploaded exactly 87,600
bytes and drew 1,200 triangles. Raw [before](raw/before-offscreen-smoke.json) and
[after](raw/after-offscreen-smoke.json) timings are diagnostic single runs, not the five-run
CPU/RSS comparison. This change reduces redundant rendering; no individual-frame throughput
improvement is claimed.

Optional external OpenUSD/PXR oracle execution was not configured. Its Rust wrappers are
included in the test count but may return successfully without running the external oracle.
Windows/macOS and discrete-GPU validation are outside this local cloud run. The PR stays a
draft while GitHub CI or other required acceptance gates remain open.
