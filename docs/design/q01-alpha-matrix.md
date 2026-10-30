# Q01 expanded alpha: execution record and coverage matrix

Evidence record for fork issue [#318](https://github.com/axisrow/BeamCraft/issues/318) (mirror of
upstream [#334](https://github.com/Apibim-com/BeamCraft/issues/334)), the expanded Q01 acceptance.
It separates what actually executed on this machine from what stays open, and it replaces no gate:
the expanded Q01 and gate K7 stay open until the remaining matrix cells run and their dependency
issues close. Nothing here is a mock standing in for an OS/GPU cell.

## Executed locally (macOS, Apple silicon)

Machine: MacBook Pro, Apple M5 (10 cores), 24 GB, Apple M5 GPU, Metal 4, Darwin 27.0.0.
Toolchain: stable aarch64-apple-darwin, `beamcraft 0.0.1`, no GPU override.

### The expanded end-to-end CLI story

`apps/beamcraft-cli/tests/alpha_scenario_expanded.rs` (single entry point, one test). Every step is
a fresh `beamcraft` process over a real blob store; no mock storage, renderer or geometry. All
steps ran and passed on this machine.

| Expanded-scope requirement | Executable check | Result |
|---|---|---|
| Modelling: frame with plates | `create-member` x3, `cut-by-plane`, `create-plate` x2 with `--hole-rect`; closed-form volumes 5,000,000 / 5,000,000 / 4,750,000 / 396,000 / 396,000 mm3 | Passed |
| Model tools | `split-part` (column at mid height, halves sum to the closed-form volume), `copy-element` (copy equals source volume), `chamfer-part` (volume strictly decreases, every other part unchanged) | Passed |
| Workspace history | `undo-workspace` / `redo-workspace` around the chamfer: the undone volume returns, redo restores it | Passed |
| Model data: grids and levels | `create-grid` (`0 1000` u-expression), `create-level` (elevation 1000), `list-grids` / `levels` | Passed |
| Connections and assembly | `create-assembly`, `attach-to-assembly` x7, `create-bolt`, `create-weld` | Passed |
| Numbering | `preview-numbering` + `approve-numbering`, 8 marks (7 parts + assembly) | Passed |
| Views/filters (real store) | `export-view` twice for one head: byte-identical bundles; a repeat export into an existing file is refused without overwriting | Passed |
| Views/filters (kind filter) | `snapshot-view --hide support` over the synthetic kernel plate scene; hidden kind recorded in the reply | Passed (stand-in, see below) |
| Exact stored outputs, reopen | After `publish-workspace`: every part's volume and `mesh_digest` equal the draft-side values; `export-bom` + `--summary` reproduce byte for byte | Passed |
| Published drawings | `create-drawing`, `publish-drawing`, `inspect-drawing` freshness `current` over the published head | Passed |
| Invalid-file regressions | `put-blob` with a missing file and `open-revision` on an empty store: both refused (`rejected`), heads unchanged | Passed |
| Cancellation/crash regression | `put-blob --cancel-after-ms 0` answers `cancelled` (exit 130), `recover-revision --min-age-secs 0` reports `recovery_complete`, head unchanged | Passed |

The delivered original scenario (`alpha_scenario.rs`) still runs unchanged next to it: numbering,
publication, reproducible BOM, IFC export, the drawing stale/version cycle and the fresh-reader
reopen of the archived drawing source.

### Native UI, existing pattern only

The `run_ui` headless-GPU screenshot suites of `beamcraft-ui` (viewport, properties, selection,
create tools, edit tools, grids, appearance, cancel) executed as part of the workspace test run on
this machine. No new native-UI automation was added to Q01, per the standing rule: screenshot
tests without kittest, and visual acceptance belongs to the owner's eyes.

## Open matrix cells and why

| Cell | Why it is open |
|---|---|
| Windows, Linux (native UI and packaging) | No such machine in this workspace; the acceptance rule forbids replacing OS/GPU matrix cells with mocks. |
| GPU variety beyond Apple M5/Metal 4 | Single-GPU hardware. Vulkan and D3D12 rendering paths of the native viewport did not execute here. |
| User attributes and attribute schedules | Fork issues [#316](https://github.com/axisrow/BeamCraft/issues/316) (I04) and [#317](https://github.com/axisrow/BeamCraft/issues/317) (I05) are open. The story carries the actual schedule surface that exists today: `export-bom --summary`. |
| Member holes | Fork issue [#315](https://github.com/axisrow/BeamCraft/issues/315) (A04.2) is open; holes in this story are plate holes (`--hole-rect`). |
| Drawing views/sheets/dimensions/drafting tools (D04-D07) | Fork issues [#228](https://github.com/axisrow/BeamCraft/issues/228)-[#231](https://github.com/axisrow/BeamCraft/issues/231) are open. The drawing part of the story is the delivered D03 slice: create, publish, inspect, stale/version cycle. |
| Item (I01-I03) | Deferred by the owner-approved MVP; the upstream issue records the same. |
| `snapshot-view` over a real model | The command runs on the synthetic kernel plate scene because P01 does not exist; the output marks itself `synthetic: true`. The real-store view path is `export-view` (V03), which this story exercises. |
| Full crash/interruption matrix | The hook-driven interrupted-commit regression lives in `apps/beamcraft-cli/tests/recovery.rs` (U07) and ran in the workspace run; the scenario adds the end-to-end cancellation path through the real binary. |

## Reproducing

```sh
cargo nextest run -p beamcraft-cli --test cli alpha_scenario
```

runs both stories (the delivered one and the expanded one). `BEAMCRAFT_Q01_KEEP=1` preserves the
expanded story's store, workspace, BOM, view bundles and drawing document for inspection.
