# Issue 26: measured native viewport CPU and memory

Measurements taken on 2026-10-19 UTC. Baseline: `11a213976ec592bf031f51b0bbdd35bcdcaf1776`
(the verified fork master). Both binaries use the same locked dependencies and release profile
(thin LTO, one codegen unit, stripped symbols). Binary SHA-256 values and environment details
are in [environment.json](raw/environment.json). No compilation or tests overlapped these runs.

This is a **cloud software-rendering result**, not a measurement of the reporter's hardware:
Debian 13, 2 vCPU quota, 7 GiB RAM cap, Rust 1.96.0, Vulkan llvmpipe LLVM 19.1.7,
Mesa 25.0.7-2+deb13u1, `LP_NUM_THREADS=2`, Xvfb 1920×1200×24. The app starts at its
unchanged 1400×900 default window size; both sides retain the same automatic MSAA policy (4× here).
No render-scale, geometry-detail or antialiasing reduction is used.

Five independent processes per binary/fixture, observed successful CLI completion plus ten
seconds of startup warmup, approximately 20 Hz process samples, three seconds per activity
phase and approximately 10 Hz injected input. Values below are **median [minimum–maximum]**
over the five process runs. CPU is UI-process user+system CPU time / elapsed wall time:
**100% is one occupied logical CPU**, so values above 100% are possible. The UI process
includes the software driver's CPU work. Child CLI CPU is reported separately.

The first idle phase starts immediately after focusing the window, so it includes focus/egui
settling. Idle after orbit includes release/animation settling. The final idle phase is the
best quiescent comparison. A measured zero means no additional CPU tick was observed in
that interval, not a claim of zero operating-system overhead indefinitely.

## Native store: CPU

100 plates sharing one mesh, 1,200 drawn triangles, 32 original material definitions and
32 original rectangle profile definitions, created through the real public CLI. The same
frozen store was used for both series. Its bundle is 5,384 bytes, SHA-256
`7d5a987257e8c99813003c335bd9334605c092b04f1f0ab113d569c5654c5757`.

| Activity (sequence order) | Before CPU % | After CPU % |
|---|---:|---:|
| Idle after focus | 182.015 [179.280–182.898] | 3.298 [2.972–10.914] |
| Orbit | 180.803 [172.479–182.568] | 138.556 [110.809–153.533] |
| Idle after orbit | 182.016 [168.701–184.654] | 3.995 [2.973–16.972] |
| Refresh: first 3 s | 165.884 [154.834–171.733] | 8.297 [7.574–9.201] |
| Resize | 172.087 [170.600–177.275] | 57.578 [54.768–82.494] |
| Final idle | 172.194 [163.288–177.435] | 0.000 [0.000–0.333] |

## Native store: memory and CLI work

RSS/PSS include CPU-side driver memory, not just Rust heap; they are not GPU VRAM or live
allocation measurements. Native-store memory changes are small because its expanded vertex
buffer is only 864 bytes. The large fixture below isolates display-buffer retention.

| Activity | Before UI RSS, MiB | After UI RSS, MiB |
|---|---:|---:|
| Idle after focus | 166.508 [165.648–166.719] | 166.387 [165.402–166.500] |
| Orbit | 166.660 [165.648–166.719] | 166.797 [165.621–167.121] |
| Idle after orbit | 166.660 [165.648–166.719] | 167.000 [166.027–167.121] |
| Refresh: first 3 s | 166.711 [165.695–166.770] | 167.035 [166.090–167.160] |
| Resize | 225.316 [224.414–225.621] | 224.621 [223.703–224.781] |
| Final idle | 270.473 [257.492–275.574] | 262.727 [261.684–266.395] |

| Work per unchanged refresh | Before | After |
|---|---:|---:|
| CLI invocations | 2.000 [2.000–2.000] | 1.000 [1.000–1.000] |
| CLI child CPU, seconds | 0.371 [0.350–0.718] | 0.176 [0.165–0.195] |
| Sum of CLI invocation wall times, seconds | 0.576 [0.535–1.073] | 0.199 [0.192–0.207] |

CLI wall sums exclude time between invocations and UI decoding/expansion/upload: they are
**not end-to-end refresh latency**. The trace wrapper delegates to the real baseline/after CLI
for this fixture. All initial-load and refresh calls succeeded.

## Large display-only fixture

1,000 unique closed polygon-prism meshes, 512 sides, 1,024 vertices and 2,044 triangles per
mesh: 2,044,000 drawn triangles, 53,224,000 raw mesh bytes, **147,168,000 expanded vertex
bytes plus 12,000 instance bytes**. This is contributor-original bounded display data,
**not a native BIM store or kernel benchmark**. A tracing adapter supplies the identical
engine bundle; production engine and UI validation/expansion/upload/rendering still run.
Bundle SHA-256: `bc78e409825385f1bcb0a9c565bf3be2c37f20b832cf2553e326fdda835ce963`.

The asynchronous refresh is explicitly allowed to finish: after the three-second activity
phase, wait for the expected complete CLI call set and ten more seconds, recording the whole
wait's memory samples. Only then measure final idle. Earlier pilot series with insufficient
warmup or incomplete refreshes were discarded. Their memory figures are not used here.

| Activity | Before CPU % | After CPU % |
|---|---:|---:|
| Idle after focus | 178.456 [173.648–181.187] | 5.527 [4.642–7.594] |
| Refresh: first 3 s | 175.088 [168.461–177.732] | 77.463 [63.115–87.748] |
| Idle after completed refresh | 175.289 [155.017–177.517] | 0.000 [0.000–0.000] |

| Activity | Before UI RSS, MiB | After UI RSS, MiB |
|---|---:|---:|
| Idle after focus | 835.848 [799.012–838.422] | 695.684 [689.688–761.484] |
| Refresh: first 3 s | 912.426 [842.316–926.936] | 746.496 [741.504–812.297] |
| Idle after completed refresh | 1310.199 [1308.160–1338.848] | 695.703 [690.711–825.617] |

Peak process-tree RSS across the entire refresh (three-second activity plus the completion/settling wait), MiB: **1337.672 [1309.234–2052.258] → 854.570 [819.062–924.070]**.

Expanded CPU display buffers are no longer owned by the loaded window after upload: it retains
only a fixed-size `SceneInfo`, the validated engine snapshot and GPU buffers. This ownership
change does not guarantee that the allocator or graphics driver returns the same number of
bytes to the OS. RSS varies across runs; raw byte samples and PSS are retained below.

## What changed and what remains

- Remove unconditional egui repaint; wake on worker success, failure or unwind. Reuse the
  resolved viewport texture when camera, uploaded scene and target dimensions are unchanged.
- Drop expanded CPU display buffers after upload. Reuse the existing GPU upload when the
  complete validated snapshot compares equal; do not rely on digest equality alone.
- Refresh geometry and definitions from one verified inspection with the opt-in bounded
  `BCREFR01` envelope. The old raw `BCVIEW01` export remains unchanged. Metadata and scene
  failures remain independently visible. Definition paging remains the existing default page.
- Borrow definition choices in the form instead of deep-cloning both lists each UI update.
- Resize only color/depth/MSAA targets; retain shader, pipeline, camera buffer and bind group.

This is the measured window/refresh implementation slice of #26. The fork baseline has no UI
selection/property worker, so upstream-only selection/cancellation cases do not apply here.
Mesh-admission hashing/bounds caches, GPU reuse across changed snapshots, adjustable MSAA/render
scale, cached form preparation, batching/frustum culling and pick/snap spatial indexing remain
follow-up work. No speedup is claimed for those paths; #26 remains open to track them.

## Evidence and reproduction

- Compressed JSON: [raw before/native](raw/before-native.json.gz), [after/native](raw/after-native.json.gz),
  [before/display](raw/before-stress.json.gz), [after/display](raw/after-stress.json.gz): every sample,
  CPU-time delta, phase duration, successful child trace and full refresh-settling samples.
- [Summary](raw/summary.json): medians, extrema and all per-process aggregate samples.
- [Native fixture manifest](raw/native-100.manifest.json), [display manifest](raw/stress.manifest.json).
- [Reproduction commands and scripts](../../../tools/performance/README.md).
- [Real-window visual smoke](raw/visual-smoke.json): identical viewport RGB hash and 129,309 lit blue pixels in both versions (100 elements, one mesh, 1,200 triangles, 4× MSAA).
- [Validation](validation.md): 960 passing Rust tests, all local required checks, and a changed-model idle-refresh smoke (101 elements / two meshes / 1,212 triangles).

The report establishes CPU/RSS behavior on this one software adapter. It does not measure live
heap allocation stacks, discrete-GPU VRAM, large engineering-store validation, Windows/macOS
behavior or production selection latency. No cross-hardware extrapolation is made.
