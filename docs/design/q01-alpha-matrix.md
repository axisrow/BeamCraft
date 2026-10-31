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
| Member holes (A04.2, G14.1 slice) | `add-hole` on a member (`--axis-point`, `--axis-direction`, `--diameter`): a through d = 20 hole removes the regular 32-gon area times the 100 mm section width; a transverse coordinate outside the section refuses with the head unchanged | Passed |
| Workspace history | `undo-workspace` / `redo-workspace` around the chamfer: the undone volume returns, redo restores it | Passed |
| Model data: grids and levels | `create-grid` (`0 1000` u-expression), `create-level` (elevation 1000), `list-grids` / `levels` | Passed |
| User attributes (I04) | `attribute-define` (choice with options, number with a unit), `attribute-set`, `attribute-get`, `attributes`; an undefined name (`NOT_FOUND`) and an off-choice value (`INVALID_ARGUMENT`) refuse | Passed |
| Connections and assembly | `create-assembly`, `attach-to-assembly` x7, `create-bolt`, `create-weld` | Passed |
| Numbering | `preview-numbering` + `approve-numbering`, 8 marks (7 parts + assembly) | Passed |
| Schedules (I05) | `schedule-export` over the numbered head: CSV, group totals that keep unknown cells counted, and HTML reproduce byte for byte; an unknown attribute column stays visible and is reported; a kind filter narrows to one row | Passed |
| Views/filters (real store) | `export-view` twice for one head: byte-identical bundles; a repeat export into an existing file is refused without overwriting | Passed |
| Views/filters (kind filter) | `snapshot-view --hide support` over the synthetic kernel plate scene; hidden kind recorded in the reply | Passed (stand-in, see below) |
| Filters (V14, real store) | `query-elements` typed rules over the frame, including an `all`/`any` combination; a query never writes; a missing attribute value never satisfies `ne`; an unknown attribute refuses instead of matching silently | Passed |
| Exact stored outputs, reopen | After `publish-workspace`: every part's volume and `mesh_digest` equal the draft-side values; `export-bom` + `--summary` reproduce byte for byte | Passed |
| Published drawings (D03 fresh path) | `create-drawing` (primary view titled `COLUMN A` at 1:20), `publish-drawing`, `inspect-drawing` freshness `current` over the published head | Passed |
| Drawing views (D04) | `edit-views` places a top view, a right section (cut through the column axis, positive side) and a detail over the placed view; every view carries its own title and display scale (1:20 primary, 1:10, 1:5, 1:2) | Passed |
| View titles and scales, deep path (D04.3) | `create-drawing`/`refresh-drawing` carry `--view-title` with the display `--scale` (a 257-byte title refuses); `edit-views` places titled placed/section/detail specs, one placed view stays unannotated, and the exact replays return the same revisions byte for byte; the a3 sheet export annotates all five views (title plus `1:N`, scale alone when untitled) byte-identically across repeats; `inspect-drawing` reports the title fields and the stored intent binds `drawing:title` to the schema descriptor (`view-title=drawing:title`) | Passed |
| Sheet (D05) | `edit-sheet` lands the document on an a3 sheet with frame inset and title block; later exports render that sheet | Passed |
| Marks, leaders, layers (D06/D07) | `edit-drafting` replaces layers and drawn objects in one atomic intent: two layers; a leader line, a text mark, a free dimension on the visible layer; a circle on a hidden layer; `visible_drafting` excludes the hidden object and the pinned source stays `current` | Passed |
| PDF export (D08) | `publish-drawing` again as version two; `inspect-drawing-versions` lists both published versions; `export-drawing --format pdf` per version: byte-identical across repeats, distinct between versions, `%PDF-` header present | Passed |
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
| Oblique member holes and curved-wall geometry (G14.2) | Fork issue [#314](https://github.com/axisrow/BeamCraft/issues/314) is open; those cases stay out of the story until it closes, and no mock stands in for the blocked geometry. |
| Item (I01-I03) | Deferred by the owner-approved MVP; the upstream issue records the same. |
| `snapshot-view` over a real model | The command runs on the synthetic kernel plate scene because P01 does not exist; the output marks itself `synthetic: true`. The real-store view path is `export-view` (V03), which this story exercises. |
| Full crash/interruption matrix | The hook-driven interrupted-commit regression lives in `apps/beamcraft-cli/tests/recovery.rs` (U07) and ran in the workspace run; the scenario adds the end-to-end cancellation path through the real binary. |

## Reproducing

```sh
cargo nextest run -p beamcraft-cli --test cli alpha_scenario
```

runs both stories (the delivered one and the expanded one). `BEAMCRAFT_Q01_KEEP=1` preserves the
expanded story's store, workspace, BOM, view bundles, drawing document and PDF exports for
inspection.
