# Q01 alpha scenario

One documented command runs the whole alpha story — the original five-part detailing story and
its expanded story — on Windows, Linux or macOS:

```sh
cargo nextest run --locked -p beamcraft-cli --test cli alpha_scenario
```

The `alpha_scenario` substring filter selects both stories of the package's `cli` integration
target: `alpha_scenario.rs` (the original) and `alpha_scenario_expanded.rs` (the expanded one,
which includes the original frame as its opening). The expected exit is 0. On success the
stories themselves are silent; the runner prints one result line per test. A failing step
aborts its story with the exact command and its expected and actual JSON in the assert panic
message. If cargo-nextest is not installed, the libtest
equivalent on a clean checkout is:

```sh
cargo test --locked -p beamcraft-cli --test cli -- alpha_scenario
```

The same substring runs both stories there too. Setting `BEAMCRAFT_Q01_KEEP=1` (PowerShell:
`$env:BEAMCRAFT_Q01_KEEP = '1'`) keeps both stories' stores, workspaces, exports and drawing
documents for inspection, and each story then prints its absolute artifact directory path on
stderr when it ends (`Q01 artifacts:` for the original story, `Q01 expanded artifacts:` for
the expanded one). A runner captures a passing test's output, which would hide those paths,
so run the nextest command with `--no-capture`, or the libtest form as
`cargo test --locked -p beamcraft-cli --test cli -- --nocapture alpha_scenario` (the flag
goes after the `--`, before the filter; a second `--` would leave capture on),
whenever KEEP is set. This invocation
is the acceptance run of the two stories, not the recorded test gate: the gate
(`tools/test-gate`) remains the workspace-wide evidence behind a PR. Every item of the expanded
acceptance is traced to its executable check or recorded manual result in
[q01-alpha-matrix.md](q01-alpha-matrix.md).

## The original five-part story

Its test runs the real CLI in a new process for every step, against a fresh published store and
personal workspace. Its invented dimensions and expected values are contributor-original test
data under MPL-2.0, covered by `ATTRIBUTION.md`. No commercial section catalogue is used.

The story defines a 100 by 50 mm rectangular section and steel density of 7850 kg/m³, forks a
workspace, creates two 1000 mm columns and a 1000 mm top beam cut to 950 mm, and creates two
200 by 200 by 10 mm base plates with 20 by 20 mm rectangular holes. It groups all five parts
in an assembly, adds a two-bolt group and a workshop weld, previews and approves numbering,
publishes, exports the BOM and experimental IFC, and reopens the accepted result.

Assertions cover each command's exit status and bounded JSON, five independent closed-form
volumes, total mass of 122.0047 kg, draft isolation, refusal without a revision, six approved
marks (five parts and their assembly), publication status, reopened meshes and quantities,
deterministic BOM bytes with exactly five unique parts, IFC entity counts, and publication retry
without another revision. Bolts and welds are records; the current bolt slice does not generate
holes automatically. The holes in this story are explicitly authored plate holes.

## Published drawing slice

The same entry point also creates and publishes a drawing of the first column with semantic
anchors. It saves `drawing-version.json`, reopens the document and its archived model source,
and checks exact publication replies and the single retained version. A subsequent draft rename
is isolated until publication; after publication the drawing reports `stale` and still refers to
the original source. Retrying its publication returns the original version bytes.

This exercises the existing minimal D03 slice only. It does not cover drawing sheets, broader
views, PDF/print, or drawing native UI. The retained final model has the same five parts with the
first column renamed. The original BOM remains the output
of the earlier numbered source revision.

## Native UI runbook

Keep the generated stores and output files for the manual portion:

```sh
BEAMCRAFT_Q01_KEEP=1 cargo test --locked -p beamcraft-cli --test cli -- alpha_scenario:: --nocapture
```

In PowerShell, set `$env:BEAMCRAFT_Q01_KEEP = '1'` before the same Cargo command. The test prints
an absolute `Q01 artifacts` directory. Set `ROOT` to that path and run:

```sh
cargo run --locked -p beamcraft-ui -- --workspace "$ROOT/draft" --published "$ROOT/published"
```

1. Confirm five visible parts, orbit and zoom, and inspect the two columns, the cut beam and the
   two holed plates. Record OS, GPU/backend, commit and any failed selection or unreadable field.
2. In Create, select Member, section `Q01-RECT`, material `Q01-steel`, start `0,500,0` and end
   `1000,500,0`. Preview, then Cancel. Reload: there must still be five parts and no new draft.
3. Repeat Preview and Commit. Confirm a sixth visible part, its coordinates, volume
   5,000,000 mm³ and mass 39.25 kg, and one unsent draft revision in Publish. Record stale
   status as a failure, even when a restart repairs it (fork issue #64).
4. Undo and confirm the five-part scene returns. Reopen the same workspace and confirm it
   persists. Do not infer a pass from a screenshot or from the automated CLI story alone.

The UI portion is a manual runbook, not an automated or completed OS/GPU acceptance claim.
Clean up only the printed artifact directory when finished.

## Acceptance boundary

This implements the original fork issue #9 story and the frame/BOM story of upstream #123.
It does not declare gate K7 passed. The expanded Q01 plan now also depends on modelling,
views/filters, attributes/schedules and drawings (K9–K11 and K8); those packages retain their
own acceptance and are not replaced by this test. IFC export remains experimental and outside
the current MVP; entity counts are not independent IFC schema or geometry certification.
