# Validation

Baseline and after release builds succeeded with Rust 1.96.0 and unchanged `Cargo.lock`.
The complete performance series is finished; raw data and aggregation are linked in the report.

- PASS: `cargo fmt --all -- --check`.
- PASS: `cargo deny check` 0.20.2 (official release archive verified against its published
  SHA-256). Advisories, bans, licenses and sources passed; existing unused-license-allowance
  warnings remain.
- PASS: real-window before/after visual smoke. Both windows displayed 100 elements, one mesh,
  1,200 triangles and 4× MSAA. The 1167×884 model-region RGB bytes match exactly, with 129,309
  lit blue pixels. See [visual-smoke.json](raw/visual-smoke.json).
- IN PROGRESS: `cargo test --locked --workspace -j 2 -- --test-threads=2`.
- PENDING RECHECK: `cargo clippy --locked --workspace --all-targets -j 2 -- -D warnings`.
  Its first pass caught a new unit test using egui's renamed `run` method; the test now uses
  `run_ui`. Product release code and measured binaries are unaffected.
- PENDING: fixtures, independent Python oracle/audit tests, module boundaries, rustdoc and
  geom/model/kernel wasm32 checks.

New regression coverage exercises bounded refresh envelopes (bad magic/lengths, truncation,
trailing bytes and caps), opt-in CLI output equivalence and invalid options, worker wakeup after
success/unwind, buffer-free scene metadata and resize readback equality against fresh targets.
The existing real-store UI refresh tests remain in the workspace suite.

Debug checks use `CARGO_PROFILE_DEV_DEBUG=0` and `CARGO_PROFILE_TEST_DEBUG=0` to keep build
artifacts bounded; release benchmark settings are unchanged. Native GPU tests use the available
Vulkan software adapter. Optional external OpenUSD/PXR checks may skip when no oracle
interpreter is installed; this is not evidence of running the external oracle. Windows/macOS
and discrete-GPU validation are not available in this cloud container. The PR remains a draft
while required checks or acceptance gates are open.
