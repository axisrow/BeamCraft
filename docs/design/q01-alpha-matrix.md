# Q01 expanded alpha: execution record and coverage matrix

Evidence record for upstream issue
[#334](https://github.com/Apibim-com/BeamCraft/issues/334), the expanded Q01 acceptance.
Slice E added the acceptance
traceability section below and the documented entry point recorded in
[q01-alpha-scenario.md](q01-alpha-scenario.md).
It separates what actually executed on this machine from what stays open, and it replaces no gate:
the expanded Q01 and gate K7 stay open until the remaining matrix cells run and their dependency
issues close. Nothing here is a mock standing in for an OS/GPU cell.

## Executed locally (macOS, Apple silicon)

Machine: MacBook Pro, Apple M5 (10 cores), 24 GB, Apple M5 GPU, Metal 4, Darwin 27.0.0.
Toolchain: stable aarch64-apple-darwin, `beamcraft 0.0.1`, no GPU override.

### The expanded end-to-end CLI story

`apps/beamcraft-cli/tests/alpha_scenario_expanded.rs` (single entry point, five tests: the
expanded story and — each in its own published store so the numbered frame keeps its recorded
counts — the curved and poly member story, the bent and lofted plate story, the model-tool
transform story and the bolt-group story). Every step is
a fresh `beamcraft` process over a real blob store; no mock storage, renderer or geometry. All
steps ran and passed on this machine.

| Expanded-scope requirement | Executable check | Result |
|---|---|---|
| Modelling: frame with plates | `create-member` x3, `cut-by-plane`, `create-plate` x2 with `--hole-rect`; closed-form volumes 5,000,000 / 5,000,000 / 4,750,000 / 396,000 / 396,000 mm3 | Passed |
| Model tools | `split-part` (column at mid height, halves sum to the closed-form volume), `copy-element` (copy equals source volume), `chamfer-part` (volume strictly decreases, every other part unchanged) | Passed |
| Model tools: rotate, mirror and arrays (T03) | in their own published store, so the numbered frame keeps the counts the manual runbook records: a linear array lands three independent copies in one revision at the stepped placements with the source volume each, and one undo removes them all; a mirror reflects the member path across the plane keeping the length and the volume; a mirrored polybeam path refuses `UNSUPPORTED_GEOMETRY` with nothing written; the quarter-turn rotation composes the orientation and keeps the authored path, so the member turns in place, undoing and redoing as one step; all six members survive publication with their exact volumes, meshes and the transformed orientation | Passed |
| Member holes (A04.2, G14.1 slice) | `add-hole` on a member (`--axis-point`, `--axis-direction`, `--diameter`): a through d = 20 hole removes the regular 32-gon area times the 100 mm section width; a transverse coordinate outside the section refuses with the head unchanged | Passed |
| Member holes, oblique and sloped walls (G14.2) | dedicated members based at the world origin: an oblique non-unit `(12,5,0)` d = 20 hole through the flat walls of a square section removes `A·100·13/12` (|n·d| = 12/13); the rotated-20° square's sloped walls remove `A·100/cos(20°)`; an axial direction and a second record on one member refuse `UNSUPPORTED_GEOMETRY` with nothing written; the identical command replays the recorded revision and fresh processes read the same volumes and `mesh_digest` byte for byte | Passed |
| Member holes, curved tube wall (G14.2 remainder) | a CHS 114x5 member takes a radial d = 20 hole: the reported drop certifies within the engine suite's own `2·A·t` bound of the smooth tube (measured 3,121.4 mm3 ≤ 3,141.6 mm3) and equals the certified removed volume; the gross stays the nominal `area x length` across the hole (the plate rule: a gross that ignores the hole, a net that does not); the hole survives the reopen with the same volume and `mesh_digest`; the d = 6/10/20 triplet is pinned by the engine test (`a_chs_member_hole_certifies_the_removed_volume_within_the_engine_bound`) and the CLI suite (`sections.rs`) | Passed |
| Modelling: poly and curved members (P12/P13) | in their own published store, so the numbered frame keeps the counts the manual runbook records: a mitred L-path polybeam stores the exact `area x path length` (50,000,000 mm3, 392.5 kg at 7850 kg/m3) and numbers; a radius-mode semicircular arc reports `path_type arc`, the exact axis length `1000·pi` and the analytic `area x arc length` quantity (only the stored mesh facets); a two-point path refuses and numbering refuses the arc with `UNSUPPORTED_GEOMETRY` (a delivered-surface limit, nothing written); the arc creation undoes and redoes as one command; both members survive publication with their exact volumes and `mesh_digest`s | Passed |
| Modelling: bent and lofted plates (A10/A11) | in their own published store, so the numbered frame keeps the counts the manual runbook records: a lofted plate between two translated contours stores the exact oblique-prism volume (1,000,000 mm3) in its reply and its stored quantities (`volume_status current`); one right-angle fold on radius 8 keeps the volume at the blank's `t x area` within the certified bound (within 8,000 mm3 of 4,000,000 mm3), the outline becomes the unfold contour and the bend record carries its parameters; the identical bend replays without writing, an overlapping second fold refuses with the head unchanged, the bend undoes and redoes as one command, and publication preserves the folded plate's exact volume and `mesh_digest` and the lofted plate's exact stored quantity and triangle count | Passed |
| Workspace history | `undo-workspace` / `redo-workspace` around the chamfer: the undone volume returns, redo restores it | Passed |
| Model data: grids and levels | `create-grid` (`0 1000` u-expression), `create-level` (elevation 1000), `list-grids` / `levels` | Passed |
| User attributes (I04) | `attribute-define` (choice with options, number with a unit), `attribute-set`, `attribute-get`, `attributes`; an undefined name (`NOT_FOUND`) and an off-choice value (`INVALID_ARGUMENT`) refuse | Passed |
| Connections and assembly | `create-assembly`, `attach-to-assembly` x7, `create-bolt`, `create-weld` | Passed |
| Bolt groups with generated holes (A14.3/A14.4) | in their own published store, so the numbered frame keeps the counts the manual runbook records: a two-fastener group over two stacked plates commits one atomic edit, resolves the auto length over the grip and its hardware (40 mm), and generates one group-owned hole record per crossing fastener on every connected plate (`hole_mode generate`, `generated_by` the group); each plate's quantity drops by the measured two-hole amount (2,838.81 mm3, within 1 mm3), the undo returns the blank plates and the redo regenerates, and publication preserves the drops, the hole ownership and the group record | Passed |
| Numbering | `preview-numbering` + `approve-numbering`, 8 marks (7 parts + assembly) | Passed |
| Schedules (I05) | `schedule-export` over the numbered head: CSV, group totals that keep unknown cells counted, and HTML reproduce byte for byte; an unknown attribute column stays visible and is reported; a kind filter narrows to one row | Passed |
| Views/filters (real store) | `export-view` twice for one head: byte-identical bundles; a repeat export into an existing file is refused without overwriting | Passed |
| Named views (V12) | `save-view` stores the storey plan (`xy` at the level offset), a grid-axis elevation, a level plan and an orbit view in the W01 camera prim layer beside the store, stamped with author, comment and the captured draft revision; `list-views`/`apply-view` leave the layer bytes byte-identical, a duplicate name and an unknown view refuse with the layer and the head intact, `apply-view` restores the exact camera across the publication round trip (the captured provenance still names the draft revision), and `remove-view` drops one view | Passed |
| Filters (V14, real store) | `query-elements` typed rules over the frame, including an `all`/`any` combination and the `type` (kind) rule; a query never writes; a missing attribute value never satisfies `ne`; an unknown attribute refuses instead of matching silently | Passed |
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

The extended native-UI cells (selection switches V11, render modes V16, cut plane V15, named
views V12, colour sets V17, sheet editor D05) are traced to the "Extended native UI cells"
section of the manual runbook in [q01-alpha-scenario.md](q01-alpha-scenario.md), next to the
four-step runbook there; its preparation step generates both artifact roots with the
documented entry point and `--no-capture`. No step has been executed yet; every cell stays
open until the owner runs it and accepts each visual result by eye. Results are recorded
separately from the instructions, in this table and on upstream issue #334, never inside the
runbook.

## Acceptance traceability

The expanded acceptance (upstream issue
[#334](https://github.com/Apibim-com/BeamCraft/issues/334)) traced item by
item. "In the UI" rows point at the runbook artifacts, never at a mock; the OS/GPU rows are
recorded open and stay open until they execute on real hardware.

| Acceptance item | Executable check or recorded manual result | Status |
|---|---|---|
| Model a frame with plates, bolts and welds (CLI) | The CLI story rows above: frame, model tools, member holes, poly and curved members, bent and lofted plates, grids/levels, attributes, assembly/bolt/weld, bolt-group generated holes, numbering, publication, exact reopen | Executed locally (macOS, Apple silicon) |
| View it (CLI) | The `export-view` bundle rows and the named-views (V12) row above | Executed locally (macOS, Apple silicon) |
| Filter it (CLI) | `query-elements` (V14, real store) row above; includes the kind rule | Executed locally (macOS, Apple silicon) |
| Number it (CLI) | `preview-numbering` + `approve-numbering`, 8 marks | Executed locally (macOS, Apple silicon) |
| Take a schedule (CLI) | `schedule-export` row above: CSV, HTML, group totals, unknown column, kind filter | Executed locally (macOS, Apple silicon) |
| Take a drawing (CLI) | The drawing rows above: D03 fresh path, D04/D04.3 views and titles, D05 sheet, D06/D07 drafting, D08 PDF | Executed locally (macOS, Apple silicon) |
| An item in the model | Deferred by the owner-approved MVP (I01–I03); the upstream issue records the same | Recorded as deferred |
| Invalid files and crash regressions | Story refusal/cancellation rows above; the hook-driven interrupted-commit path lives in `apps/beamcraft-cli/tests/recovery.rs` (U07) and ran in the workspace run | Executed locally (macOS, Apple silicon) |
| In the CLI, end to end | The documented entry point of [q01-alpha-scenario.md](q01-alpha-scenario.md): one command runs both stories, exit 0 | Executed locally (macOS, Apple silicon) |
| In the UI, original four steps | Runbook in [q01-alpha-scenario.md](q01-alpha-scenario.md) | Open until the owner executes and accepts |
| In the UI, extended cells (V11, V16, V15, V12, V17, D05) | "Extended native UI cells" section of [q01-alpha-scenario.md](q01-alpha-scenario.md) | Open until the owner executes and accepts |
| Windows and Linux (native UI and packaging) | No such machine in this workspace; mocks forbidden by the acceptance rule | Recorded open |
| GPU variety beyond Apple M5/Metal 4 | Single-GPU hardware; Vulkan and D3D12 paths did not execute | Recorded open |

## Open matrix cells and why

| Cell | Why it is open |
|---|---|
| Windows, Linux (native UI and packaging) | No such machine in this workspace; the acceptance rule forbids replacing OS/GPU matrix cells with mocks. |
| GPU variety beyond Apple M5/Metal 4 | Single-GPU hardware. Vulkan and D3D12 rendering paths of the native viewport did not execute here. |
| Curved-wall (tube) member holes, corner-piece seal (G14.2 remainder) | The directed cutter through a multi-facet chain still builds an unsealed corner piece when a chain's kink line crosses a footprint edge close to a cavity vertex; the seal honestly refuses and a follow-up issue owns the fix. The volume mis-certification this cell used to carry (the constant ~10,980 mm3 past the `2·A·t` bound) is fixed and executed above; two delivered-surface limits remain as recorded by the slice-4 delivery: a member moved off the world origin refuses its hole as not fitting (the replay reads the member-frame axis point in element coordinates), and a second hole record on one member refuses `BodyNotPrism`. |
| Item (I01-I03) | Deferred by the owner-approved MVP; the upstream issue records the same. |
| `snapshot-view` over a real model | The command runs on the synthetic kernel plate scene because P01 does not exist; the output marks itself `synthetic: true`. This is no longer a stand-in for any acceptance item (resolved in slice E): the kind filter of the expanded acceptance is exercised on the real frame through `query-elements` (V14), and the real-store view path is `export-view` (V03), which this story exercises. The synthetic-scene step still runs inside the story for the command's own coverage (hidden kind recorded in the reply), without tracing to an acceptance item. |
| Full crash/interruption matrix | The hook-driven interrupted-commit regression lives in `apps/beamcraft-cli/tests/recovery.rs` (U07) and ran in the workspace run; the scenario adds the end-to-end cancellation path through the real binary. |

## Reproducing

The documented entry point of [q01-alpha-scenario.md](q01-alpha-scenario.md) runs both stories
(the delivered one and the expanded one):

```sh
cargo nextest run --locked -p beamcraft-cli --test cli alpha_scenario
```

The libtest equivalent, if cargo-nextest is not installed, is
`cargo test --locked -p beamcraft-cli --test cli -- alpha_scenario`.
`BEAMCRAFT_Q01_KEEP=1` preserves the expanded story's store, workspace, BOM, view bundles,
drawing document and PDF exports for inspection.
