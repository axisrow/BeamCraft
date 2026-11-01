# Issue 26 follow-up: viewport CPU and memory

Date: 2026-10-21. Status: **implemented on the fork; measured on a software adapter only.**
Branch `bug/26-viewport-cpu-memory`, based on `origin/master` `538cbed` of `axisrow/BeamCraft`;
carried to `Apibim-com/BeamCraft` rebased onto its `master`.

**Ported to upstream.** The CLI options, the frame protocol, the mesh admission and reuse, the
renderer split, the perf harness and the `--samples` default port as they are. The fork's
window-side differences this note describes do not all exist upstream, so the port adapts them:
the upstream window draws its 3D image every frame (no on-demand redraw, so finding 1 and the #39
regression are upstream no-ops and `App::show` only consolidates the scene swap), the refresh asks
for one framed export of the snapshot and the definitions (the `--with-element` reply has no
upstream consumer, whose properties panel reads on selection), and the per-frame copies finding is
applied to the upstream Create and properties panels' borrow pattern. The numbers below were
measured on the fork implementation.

Fork issue #26 is a static audit of the native window and its read paths, with no run behind it.
#39 (merged) addressed part of it: the window redraws its 3D image only when it changes and the
workers wake it (finding 1), a refresh runs its CLI queries concurrently instead of one after the
other, and the snapshot builder looks a mesh digest up before validating it and computes local
bounds once per mesh (part of finding 4). Since then V05.2 and V06 added the selection, edit and
properties paths that findings 2 and 9 describe.

This change measures what is left on `538cbed`, fixes it and measures again with the same
harness: one CLI process and one inspection per refresh, a click that starts one process instead
of three and works at any model size, lookups that stop when they are no longer wanted, no CPU
display buffers kept after upload and no repeated uploads, one hash per received mesh blob, no
per-frame copies, a resize that keeps the pipeline, 1x MSAA on software adapters, and the fix of
a redraw regression of #39.

## What is implemented

| Finding | On `538cbed` | This change |
|---|---|---|
| Regression of #39 | A refresh whose scene kept its bounds and the viewport aspect replaced the scene without asking for a draw: after an edit inside the bounds, a rename, an undo or a reload the old image stayed on screen until the camera moved. | Every scene swap goes through one `App::show`, which asks for a draw; an unchanged framing still keeps the camera. A window test on the headless adapter fails without the fix. |
| 2, refresh | 2 or 3 concurrent CLI processes per refresh read the model (`export-view`, `definitions` and, with a selection, `inspect-elements`), each a full verified inspection; the head could move between them. | `export-view --out - --with-definitions true [--with-element ID]` frames the `definitions` and `inspect-elements --element ID` replies of the same inspection before the unchanged bundle (`BCVXFRM1`, then per reply a little-endian `u32` length and the reply as its command prints it, 0 for one not asked, then the bundle). The refresh runs one such process; only a failed export asks the other two on their own. The workspace queries read the draft's history and still run alongside. |
| 2, selection | A click ran CLI `pick`, then `inspect-elements` twice (editor details, properties panel): three processes and three full inspections in sequence. Both `inspect-elements` replies list the whole model, which exceeds the CLI's bounded reply encoder (1 MiB, 65,536 JSON tokens) at 280 elements: every click there failed with `LIMIT_EXCEEDED`. | The engine's `pick` runs in the window's process on the snapshot it shows (`edit::pick_in`, the query `pick --targets face --max-hits 1` makes; `Arc<ViewSnapshot>` shared with the worker), then one `inspect-elements --element ID` (a reply of one element) gives both the details and the properties. The re-read after an edit uses the same single lookup. |
| 9 | Esc, an accepted edit or the re-read after an edit dropped the lookup's receiver; its CLI processes ran to the end. | `invalidate_select` sets the lookup's cancel flag: a lookup not started yet never starts its process, a running one is killed (`command::run_cancellable`, answer `CANCELLED`, counted as `cli_killed`). Only read-only lookups take the flag. A click while a lookup runs is still ignored, so at most one lookup is wanted at a time. |
| 3 | `Loaded` kept the expanded CPU display buffers for as long as the scene was shown; every refresh expanded and uploaded every mesh again, also for an unchanged snapshot; the upload waited for the device on the UI thread. | `Loaded` keeps a `SceneSummary` (origin, framing, counts); the expanded buffers are dropped once they are on the device. The worker gets what the window shows (`source::Known`: snapshot token and resident mesh digests): the same token is `Refreshed::Unchanged` and nothing is built, uploaded or drawn; meshes the device holds are not expanded again and `Uploaded::upload` reuses their vertex buffers, sends only new meshes and the instance buffers, and does not wait for the device. |
| 4, rest | `from_bundle` hashed and copied each blob three times (its digest, a throwaway `MeshEntry` for its positions, the builder's validation). | `MeshEntry` shares its bytes (`Arc<[u8]>`). `from_bundle` admits each blob once (one structural check, one SHA-256, one copy) and hands the entries to the builder, which takes one without validating it again only when an element offers exactly those bytes. #39's refusal of other bytes under a known digest is kept. |
| 5 | The Create panel cloned both definition lists and the store path every frame and prepared its staged command twice; the properties panel cloned the selected report, the materials and the path and prepared its patch twice. | Borrowed beside the forms (disjoint fields); a Commit click is applied after the button row; the staged command is prepared once per frame, again only when a button acted. |
| 6 | 4x MSAA whenever supported, also on a software adapter, where every sample is rasterized by the processor (36 instead of 8 target bytes per pixel). | `render::samples_for(device type, 4x support)`: 4x on a hardware adapter that supports it, 1x on `DeviceType::Cpu`; `--samples 1|4` for the window and `--metrics`. The multisampled colour target is discarded after its resolve. |
| 7 | A resize built a whole new renderer: shader, layouts, pipeline, camera buffer and targets. | `Renderer::resize` rebuilds the targets only and keeps the old ones on error. |
| 8 | Full scans in pick, snap and draw. | Measured only (engine bench below); no change. |

The measurement harness is part of the change and was committed first, without any change in
behaviour, so that "before" is that commit (`3b2324e`, `538cbed` plus the harness) and "after" is
the head, with identical counting:

- `beamcraft_ui::stats`: process-wide relaxed atomic counters (frames, viewport draws, submits,
  uploads and bytes, pipeline and target builds, CLI processes started, killed and running, their
  wall time, resize time, scene swaps, selection errors, retained display bytes) and the Linux
  `/proc/self` CPU ticks and RSS. Nothing reads them unless a perf run is active.
- `beamcraft-ui --store ROOT --perf-run PLAN`: a scripted run inside the real window. Its phases
  act through the window's own paths: `reload` is the R key, `select` a click straight down onto
  a box element (`start_select`), `cancel` a click, Esc once its lookup runs a CLI process, and
  another click, `edit` the Create panel's preview and commit, `resize` an `InnerSize` request.
- `crates/engine/examples/query_bench.rs`: snapshot build, bundle encode and decode and the pick,
  point and snap queries on synthetic and copied scenes.

## Decisions

- **One process, one revision.** The definitions and the selected element's report now come from
  the same inspection as the snapshot, so they can no longer belong to another head than the one
  drawn. The frame is untrusted input to the window: magic and lengths are checked, each length
  is bounded by the CLI reply cap and checked against the bytes received, the replies go through
  the bounded JSON decoder and the bundle through the engine's validation. The default
  `export-view` output and the default `inspect-elements` reply are unchanged.
- **`inspect-elements --element ID`, not a new command.** The reply has the shape the window already
  reads (`parse_element`, `parse_element_info`), restricted to one element: the plates, members
  and snapshot entries of that element and its meshes, none when the head does not hold it. The
  counts still describe the whole revision and `element_filter` names the filter. A malformed id
  is `INVALID_ID` before the store is opened; the window never sends one (an id no revision can
  hold is answered as not found without a process).
- **The click picks what is on screen.** The pick runs on the snapshot the window shows rather
  than on the store's head at the moment of the click: the same engine query on the same bytes
  the CLI would read, without opening, verifying and re-evaluating the revision. The details are
  then read from the head; an element that is gone by then is `NOT_FOUND`, as before. The click
  test checks that the in-process pick names the expected element for rays through known
  points (the CLI `pick` it was first compared with was removed with #763).
- **Cancel only reads.** A lookup's answer can be dropped at no cost, so its process can be
  killed; a change command is never cancelled, since killing it could leave its outcome unknown.
  The flag is checked before the process starts and every 10 ms while it runs.
- **One way onto the screen.** The regression came from a second branch that replaced the scene
  without the redraw request; both branches now call one `show`.
- **Reuse by digest.** Equal mesh digests are equal blobs, so equal anchors and display vertices;
  only instance buffers, which depend on the scene origin, are rebuilt. A resident mesh still
  counts against the scene vertex budget, so the same snapshot is accepted or refused either way.
  Buffers the new scene does not use are released with the old one; wgpu keeps a buffer alive
  while submitted work uses it. An unchanged token still refits the camera when the viewport
  aspect changed, as a reload did before.
- **MSAA default by adapter type.** A software adapter rasterizes every sample on the processor.
  The default changes only there; `--samples 4` restores the previous image. Hardware adapters
  keep 4x.

## Evidence

All numbers are measurements on one machine with release builds (the workspace release profile:
thin LTO, one codegen unit). Calculated figures are labelled as such.

- **Machine:** 4 vCPU Intel Xeon @ 2.80 GHz, 15 GiB, Linux 6.18, `CLK_TCK` 100. Adapter as
  wgpu reports it: `llvmpipe (LLVM 20.1.2, 256 bits)`, `DeviceType::Cpu`, Vulkan (Mesa 25.2.8
  lavapipe), window on Xvfb 1600x1000. A software adapter rasterizes on the same four cores the
  window and its CLI processes use.
- **Builds:** "before" is `3b2324e` (`538cbed` plus the harness, no behaviour change), "after"
  is `c2ae9c6` (the head without this note); the per-commit columns are the commits between.
- **Fixtures:** contributor-original synthetic stores built with `master`'s CLI and fixed ids.
  *280 elements*: 120 plates (width 200 to 1390 mm, every third with a round 32-segment hole),
  100 copies of the first plate, 60 straight members (I 300 and a 200x300 rectangle), one
  material. *140 elements*: 60 plates, 50 copies, 30 members, same rules. The full
  `inspect-elements` reply of the 280-element store exceeds the CLI's reply limits
  (`LIMIT_EXCEEDED`); the 140-element store's reply (329 KB) fits, so selection is also measured
  where `master` works.
- **Window:** `beamcraft-ui --store COPY --perf-run
  load,idle:20,ui:120,orbit:120,reload:5,select:5,cancel:5,edit:3,resize:20`, each run on a
  fresh copy of the fixture, five runs per build, interleaved (before, after, before, after...).
  Cells are the median with the minimum and maximum in brackets; "within spread" means the two
  ranges overlap. CPU is `/proc/self/stat` ticks (10 ms) of the window process and, as "CLI CPU",
  of its finished CLI children.
- **CLI:** each command run ten times per build on the pristine fixture, the builds alternating;
  wall clock, CPU and maximum RSS from `wait4`.

What the numbers say, at 280 elements unless noted:

- **Selection** works at any size: 5 of 5 clicks failed on `master` and none do now; a click
  starts 1 CLI process instead of 3 and answers in 1.8 s instead of 5.6 s (0.94 s instead of
  2.7 s at 140 elements, where `master` still works).
- **Giving a lookup up** stops its process: the cancel phase starts 10 processes instead of 30,
  kills the 5 it gave up, and its CLI CPU falls from 54.6 s to 9.2 s.
- **A refresh** is 1 process instead of 2 (with a selection, 1 instead of 3), its CLI CPU halves,
  and a refresh of an unchanged store sends nothing to the device (4.5 MB for five refreshes
  before). Its wall time barely moves: the remaining work is the one full inspection.
- **An edit** shows its result: 3 draws for 3 edits instead of 0, and 12.7 KB sent to the device
  instead of 2.7 MB. The time to the new image is the CLI's (create, then one inspection).
- **Orbiting** costs 72 ms of CPU per frame instead of 124, the MSAA default on this software
  adapter; with `--samples 4` the head is back at 126 ms (control table).
- **Memory:** 142 MiB resident after loading instead of 170, 144 MiB after the resizes instead of
  222, a peak of 149 MiB instead of 227; the window keeps no CPU display buffer (882 KiB before).
  Most of it is the 1x targets: with `--samples 4` the head ends the resizes at 219 MiB.
- **A resize** builds no pipeline (20 before) and takes 3 ms of UI time for 20 resizes instead
  of 37.
- **Unchanged:** an idle window (2 frames in 20 s, since #39) and the cost of a frame that does
  not redraw the 3D image (about 62 ms of CPU on this adapter, egui's own drawing).

### The window, 280 elements

| Phase | Metric | master + harness (n=5) | this PR (n=5) | change |
|---|---|---|---|---|
| load | wall, ms | 2,137 (1,925–2,214) | 1,785 (1,691–2,092) | within spread |
| load | CLI processes | 2 | 1 | −50% |
| load | RSS after load, MiB | 169.9 (169.8–170.1) | 141.6 (141.1–141.7) | −17% |
| load | CPU display buffers kept, KiB | 882 | 0 | −100% |
| idle | frames in 20 s | 2 | 2 | same |
| idle | UI CPU, ms | 130 (120–130) | 120 (110–130) | within spread |
| ui | UI CPU per frame, ms | 61.5 (59.8–64.8) | 63.8 (59.4–65.8) | within spread |
| ui | viewport draws | 0 | 0 | same |
| orbit | UI CPU per frame, ms | 123.8 (116.3–128.6) | 71.6 (70.2–75.5) | −42% |
| orbit | wall per frame, ms | 45.5 (42.2–48.0) | 30.2 (28.8–32.2) | −34% |
| reload | CLI processes (5 refreshes) | 10 | 5 | −50% |
| reload | CLI CPU, ms | 18,100 (17,650–19,430) | 9,080 (8,640–9,290) | −50% |
| reload | refresh, median ms | 1,914 (1,858–2,077) | 1,784 (1,773–1,878) | within spread |
| reload | uploads / bytes sent | 5 / 4,509,600 | 0 / 0 |  |
| reload | viewport draws | 0 | 0 | same |
| select | CLI processes (5 clicks) | 15 | 5 | −67% |
| select | failed selections | 5 | 0 | −100% |
| select | click to answer, median ms | 5,600 (5,286–5,932) | 1,838 (1,730–1,963) | −67% |
| select | CLI CPU, ms | 27,490 (25,970–28,970) | 9,140 (8,710–9,690) | −67% |
| cancel | CLI processes started / killed | 30 / 0 | 10 / 5 |  |
| cancel | CLI CPU, ms | 54,620 (52,080–57,940) | 9,170 (8,600–9,720) | −83% |
| cancel | step, median ms | 5,623 (5,339–5,965) | 1,813 (1,741–1,973) | −68% |
| edit | CLI processes (3 edits) | 12 | 6 | −50% |
| edit | edit to new image, median ms | 6,191 (6,057–6,487) | 6,029 (5,888–6,213) | within spread |
| edit | viewport draws | 0 | 3 | 0 → 3 |
| edit | bytes sent to the device | 2,711,016 | 12,744 | −99.5% |
| resize | pipeline / target builds (20) | 20 / 20 | 0 / 20 |  |
| resize | UI time in resize, ms | 37.4 (35.2–49.6) | 3.1 (2.9–3.2) | −92% |
| resize | RSS at the end, MiB | 222.5 (219.7–223.4) | 144.0 (143.4–144.2) | −35% |
| resize | peak RSS (VmHWM), MiB | 226.6 (224.1–227.5) | 148.5 (147.9–148.7) | −34% |

### The window, 140 elements

| Phase | Metric | master + harness (n=5) | this PR (n=5) | change |
|---|---|---|---|---|
| load | wall, ms | 1,021 (1,009–1,114) | 955 (886–1,089) | within spread |
| load | CLI processes | 2 | 1 | −50% |
| load | RSS after load, MiB | 167.1 (166.8–167.3) | 138.7 (138.6–138.9) | −17% |
| load | CPU display buffers kept, KiB | 437 | 0 | −100% |
| idle | frames in 20 s | 2 | 2 | same |
| idle | UI CPU, ms | 130 (110–140) | 110 (110–130) | within spread |
| ui | UI CPU per frame, ms | 61.6 (59.7–63.9) | 62.6 (61.0–64.6) | within spread |
| ui | viewport draws | 0 | 0 | same |
| orbit | UI CPU per frame, ms | 113.5 (113.0–114.2) | 71.2 (67.9–71.7) | −37% |
| orbit | wall per frame, ms | 42.0 (41.1–42.5) | 30.0 (28.7–31.0) | −28% |
| reload | CLI processes (5 refreshes) | 10 | 5 | −50% |
| reload | CLI CPU, ms | 8,690 (8,270–9,250) | 4,490 (4,360–4,670) | −48% |
| reload | refresh, median ms | 928 (844–970) | 905 (885–992) | within spread |
| reload | uploads / bytes sent | 5 / 2,234,640 | 0 / 0 |  |
| reload | viewport draws | 0 | 0 | same |
| select | CLI processes (5 clicks) | 15 | 5 | −67% |
| select | failed selections | 0 | 0 | same |
| select | click to answer, median ms | 2,700 (2,560–2,912) | 938 (859–958) | −65% |
| select | CLI CPU, ms | 12,960 (12,380–14,190) | 4,610 (4,390–4,700) | −64% |
| cancel | CLI processes started / killed | 30 / 0 | 10 / 5 |  |
| cancel | CLI CPU, ms | 25,920 (25,320–26,930) | 4,360 (4,280–4,990) | −83% |
| cancel | step, median ms | 2,725 (2,667–2,863) | 883 (854–1,003) | −68% |
| edit | CLI processes (3 edits) | 12 | 6 | −50% |
| edit | edit to new image, median ms | 3,079 (2,772–3,328) | 3,005 (2,849–3,259) | within spread |
| edit | viewport draws | 0 | 3 | 0 → 3 |
| edit | bytes sent to the device | 1,346,040 | 7,704 | −99% |
| resize | pipeline / target builds (20) | 20 / 20 | 0 / 20 |  |
| resize | UI time in resize, ms | 37.0 (35.2–40.3) | 3.0 (3.0–3.0) | −92% |
| resize | RSS at the end, MiB | 224.8 (222.8–228.3) | 140.7 (140.2–140.8) | −37% |
| resize | peak RSS (VmHWM), MiB | 229.2 (226.7–232.4) | 145.1 (144.5–145.2) | −37% |

### Per commit, 280 elements

Two runs per commit; each column adds one commit to the one before.

| Phase | Metric | 1 `3b2324e` (n=5) | 2 `08a075c` (n=2) | 3 `4211eb2` (n=2) | 4 `d51f211` (n=2) | 5 `dcd0a15` (n=2) | 6 `7419bcb` (n=2) | 7 `ec13849` (n=2) | 8 `c2ae9c6` (n=5) |
|---|---|---|---|---|---|---|---|---|---|
| load | RSS after load, MiB | 169.9 (169.8–170.1) | 169.7 (169.6–169.9) | 169.6 (169.4–169.8) | 169.7 (169.6–169.8) | 170.1 (170.1–170.1) | 170.1 (170.1–170.1) | 170.1 (170.1–170.1) | 141.6 (141.1–141.7) |
| load | CPU display buffers kept, KiB | 882 | 882 | 882 | 882 | 0 | 0 | 0 | 0 |
| orbit | UI CPU per frame, ms | 123.8 (116.3–128.6) | 123.1 (121.3–125.0) | 120.2 (118.0–122.3) | 126.8 (126.4–127.1) | 128.2 (127.2–129.2) | 130.0 (130.0–130.1) | 126.7 (122.1–131.4) | 71.6 (70.2–75.5) |
| reload | CLI processes (5 refreshes) | 10 | 10 | 5 | 5 | 5 | 5 | 5 | 5 |
| reload | refresh, median ms | 1,914 (1,858–2,077) | 1,997 (1,960–2,035) | 1,868 (1,828–1,909) | 1,964 (1,964–1,964) | 2,009 (1,935–2,084) | 1,945 (1,867–2,022) | 1,892 (1,889–1,895) | 1,784 (1,773–1,878) |
| reload | uploads / bytes sent | 5 / 4,509,600 | 5 / 4,509,600 | 5 / 4,509,600 | 5 / 4,509,600 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| reload | viewport draws | 0 | 5 | 5 | 5 | 0 | 0 | 0 | 0 |
| select | CLI processes (5 clicks) | 15 | 15 | 15 | 5 | 5 | 5 | 5 | 5 |
| select | failed selections | 5 | 5 | 5 | 0 | 0 | 0 | 0 | 0 |
| select | click to answer, median ms | 5,600 (5,286–5,932) | 5,564 (5,486–5,643) | 5,702 (5,398–6,005) | 1,989 (1,901–2,077) | 2,101 (2,028–2,174) | 2,106 (2,078–2,135) | 1,879 (1,808–1,950) | 1,838 (1,730–1,963) |
| cancel | CLI processes started / killed | 30 / 0 | 30 / 0 | 30 / 0 | 10 / 5 | 10 / 5 | 10 / 5 | 10 / 5 | 10 / 5 |
| edit | edit to new image, median ms | 6,191 (6,057–6,487) | 6,141 (6,050–6,231) | 6,274 (6,202–6,346) | 6,166 (5,997–6,336) | 6,591 (6,521–6,661) | 6,871 (6,460–7,281) | 6,180 (6,157–6,203) | 6,029 (5,888–6,213) |
| edit | viewport draws | 0 | 3 | 3 | 3 | 3 | 3 | 3 | 3 |
| edit | bytes sent to the device | 2,711,016 | 2,711,016 | 2,711,016 | 2,711,016 | 12,744 | 12,744 | 12,744 | 12,744 |
| resize | pipeline / target builds (20) | 20 / 20 | 20 / 20 | 20 / 20 | 20 / 20 | 20 / 20 | 20 / 20 | 20 / 20 | 0 / 20 |
| resize | UI time in resize, ms | 37.4 (35.2–49.6) | 40.8 (36.8–44.8) | 43.3 (40.8–45.8) | 43.1 (40.8–45.4) | 42.9 (41.1–44.8) | 41.0 (38.6–43.4) | 40.9 (38.9–42.9) | 3.1 (2.9–3.2) |
| resize | RSS at the end, MiB | 222.5 (219.7–223.4) | 222.6 (221.9–223.3) | 222.0 (220.5–223.5) | 221.8 (220.7–222.9) | 219.8 (219.7–219.9) | 219.4 (219.2–219.6) | 219.3 (219.3–219.4) | 144.0 (143.4–144.2) |

### 4x MSAA control

The head with `--samples 4` (three runs), to separate the MSAA default from the other changes.

| Phase | Metric | this PR, 1x (n=5) | this PR, --samples 4 (n=3) | change |
|---|---|---|---|---|
| load | wall, ms | 1,785 (1,691–2,092) | 2,005 (1,972–2,159) | within spread |
| load | CLI processes | 1 | 1 | same |
| load | RSS after load, MiB | 141.6 (141.1–141.7) | 170.1 (169.9–170.2) | +20% |
| load | CPU display buffers kept, KiB | 0 | 0 | same |
| idle | frames in 20 s | 2 | 2 | same |
| idle | UI CPU, ms | 120 (110–130) | 120 (100–150) | same |
| ui | UI CPU per frame, ms | 63.8 (59.4–65.8) | 62.0 (60.7–63.3) | within spread |
| ui | viewport draws | 0 | 0 | same |
| orbit | UI CPU per frame, ms | 71.6 (70.2–75.5) | 125.8 (118.6–126.2) | +76% |
| orbit | wall per frame, ms | 30.2 (28.8–32.2) | 45.9 (42.6–48.5) | +52% |
| reload | CLI processes (5 refreshes) | 5 | 5 | same |
| reload | CLI CPU, ms | 9,080 (8,640–9,290) | 8,780 (8,710–8,830) | within spread |
| reload | refresh, median ms | 1,784 (1,773–1,878) | 1,783 (1,763–1,803) | within spread |
| reload | uploads / bytes sent | 0 / 0 | 0 / 0 |  |
| reload | viewport draws | 0 | 0 | same |
| select | CLI processes (5 clicks) | 5 | 5 | same |
| select | failed selections | 0 | 0 | same |
| select | click to answer, median ms | 1,838 (1,730–1,963) | 1,806 (1,740–1,853) | within spread |
| select | CLI CPU, ms | 9,140 (8,710–9,690) | 8,890 (8,700–9,030) | within spread |
| cancel | CLI processes started / killed | 10 / 5 | 10 / 5 |  |
| cancel | CLI CPU, ms | 9,170 (8,600–9,720) | 9,240 (8,730–9,430) | within spread |
| cancel | step, median ms | 1,813 (1,741–1,973) | 1,893 (1,807–1,896) | within spread |
| edit | CLI processes (3 edits) | 6 | 6 | same |
| edit | edit to new image, median ms | 6,029 (5,888–6,213) | 6,014 (5,983–6,398) | within spread |
| edit | viewport draws | 3 | 3 | same |
| edit | bytes sent to the device | 12,744 | 12,744 | same |
| resize | pipeline / target builds (20) | 0 / 20 | 0 / 20 |  |
| resize | UI time in resize, ms | 3.1 (2.9–3.2) | 3.9 (3.6–5.3) | +24% |
| resize | RSS at the end, MiB | 144.0 (143.4–144.2) | 218.5 (218.2–218.7) | +52% |
| resize | peak RSS (VmHWM), MiB | 148.5 (147.9–148.7) | 222.7 (222.5–223.1) | +50% |

### CLI commands

Ten runs per build, the two builds alternating; exit 2 is `LIMIT_EXCEEDED`.

| Elements | Command | master + harness: wall, ms | this PR: wall, ms | change | CPU, ms (before → after) | max RSS (this PR), MiB | exit |
|---|---|---|---|---|---|---|---|
| 280 | `export-view` | 1,982 (1,706–2,523) | 1,990 (1,879–2,365) | within spread | 1,961 → 1,946 | 45.7 (45.5–45.9) | 0 |
| 280 | `definitions` | 1,972 (1,703–2,197) | 1,909 (1,816–2,182) | within spread | 1,941 → 1,878 | 46.1 (46.0–46.2) | 0 |
| 280 | `inspect-elements` | 1,925 (1,661–2,098) | 1,918 (1,744–2,129) | within spread | 1,879 → 1,867 | 46.1 (45.8–46.2) | 2 |
| 280 | `pick` | 1,846 (1,659–2,162) | 1,931 (1,738–2,451) | within spread | 1,828 → 1,868 | 45.9 (45.8–46.2) | 0 |
| 280 | `export-view --with-definitions true --with-element ID` | — | 2,053 (1,863–2,303) | new | — → 1,985 | 45.9 (45.7–46.1) | 0 |
| 280 | `inspect-elements --element ID` | — | 1,841 (1,672–2,420) | new | — → 1,805 | 46.1 (45.9–46.2) | 0 |
| 140 | `export-view` | 898 (803–1,192) | 881 (800–907) | within spread | 880 → 858 | 27.4 (27.2–27.6) | 0 |
| 140 | `definitions` | 901 (839–1,117) | 989 (854–1,140) | within spread | 881 → 956 | 27.8 (27.6–27.9) | 0 |
| 140 | `inspect-elements` | 922 (882–1,332) | 933 (829–1,180) | within spread | 916 → 908 | 27.8 (27.6–28.0) | 0 |
| 140 | `pick` | 963 (857–1,271) | 998 (878–1,324) | within spread | 953 → 983 | 27.7 (27.5–27.9) | 0 |
| 140 | `export-view --with-definitions true --with-element ID` | — | 954 (893–1,136) | new | — → 922 | 27.7 (27.6–27.9) | 0 |
| 140 | `inspect-elements --element ID` | — | 904 (787–1,082) | new | — → 879 | 27.9 (27.6–28.0) | 0 |

### Engine queries (`query_bench`)

One scenario per process, five runs per build, alternating; each cell is the fastest run's
median of 20 iterations (single runs on this shared VM vary by up to 60 %). The pick code is
unchanged; its column shows that it stays within that noise.

| Scenario | elements / meshes | build, ms | from_bundle, ms | pick ray, ms | runs |
|---|---|---|---|---|---|
| `copies:10000` | 10,000 / 1 | 11.3 → 11.5 (+2%) | 13.1 → 12.2 (−7%) | 20.19 → 19.01 (−6%) | 5 + 5 |
| `copies:65536` | 65,536 / 1 | 76.5 → 76.8 (+0%) | 82.1 → 82.8 (+1%) | 127.47 → 121.20 (−5%) | 5 + 5 |
| `holed-copies:10000` | 10,000 / 1 | 16.1 → 15.7 (−3%) | 21.1 → 20.6 (−2%) | 0.08 → 0.09 (+8%) | 5 + 5 |
| `synthetic:10000` | 10,000 / 1,496 | 27.4 → 26.6 (−3%) | 39.1 → 27.4 (−30%) | 15.95 → 15.84 (−1%) | 5 + 5 |
| `synthetic-simplified:10000` | 10,000 / 2,431 | 36.0 → 36.0 (+0%) | 53.9 → 35.0 (−35%) | 15.66 → 16.08 (+3%) | 5 + 5 |

### Headless renderer (`--metrics 300`, 1280x800, three runs)

| Scene | build | samples | per-frame complete, median ms | p95 ms |
|---|---|---|---|---|
| synthetic 10,000 | before | 4x | 55.2 (53.6–56.4) | 71.4 (68.2–71.5) |
| synthetic 10,000 | after | 1x | 38.6 (33.1–39.6) | 52.4 (42.8–52.6) |
| synthetic 10,000 | after | 4x | 56.0 (55.4–56.2) | 70.2 (69.9–70.5) |
| store 280 | before | 4x | 21.8 (20.9–22.2) | 26.5 (25.3–27.8) |
| store 280 | after | 1x | 5.1 (5.1–5.3) | 7.2 (6.8–7.4) |
| store 280 | after | 4x | 21.0 (20.9–21.6) | 25.1 (25.0–27.1) |

### At the time of the issue

The same plan without `select` and `cancel` (those paths did not exist yet), five runs on the
fork's `master` at `583340a` before #39, with a 280-element store built by that commit's CLI:
the idle window drew 583 frames in 20 s and used 53.0 CPU-seconds in them (265 % of one core),
against 2 frames and 0.12 to 0.13 CPU-seconds on `538cbed` and on this head. #39 removed that; this
change does not alter it.

Calculated, not measured: the colour and depth targets of a 1280x800 viewport take 35.2 MiB
with 4x MSAA and 7.8 MiB with 1x (36 and 8 bytes per pixel).

## Not done

- Finding 8: no spatial index, culling, batching or allocation-free tie-breaks. The bench shows
  a ray pick at about 16 ms over 10,000 synthetic elements, linear in the element count; the
  window now runs it in process on every click, so a spatial index is the next step for much
  larger models.
- The CLI's inspection itself: each refresh still opens and verifies the revision and evaluates
  every body once (no cache of evaluated geometry across refreshes), and `definitions` still scans
  the plates for every listed definition (`used_by`). These are now the whole refresh cost (one
  process instead of two or three) and are left for a focused CLI change.
- No render-resolution scale and no MSAA switch in the panel (`--samples` covers the choice).
- Only a software adapter (lavapipe under Xvfb) was measured. The counts (frames, draws, uploads,
  processes, kills, pipeline builds) carry over to hardware adapters; the times do not. The
  Apple M5 baseline in the issue was taken on another machine at `11a2139` and is not compared.
