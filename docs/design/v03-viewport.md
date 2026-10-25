# V03 window and model display

Date: 2026-10-18. Status: **experimental vertical slice; V03 acceptance is limited as listed below.**
Branch `feat/v03-viewport`, based on `origin/master` `3f9f0f5`.

Plan row: V03 / M, depends on V01 and V02. Result: the selected viewport, camera/orbit and loading of
the accepted snapshot. Acceptance: a real model from the engine, not a mock; units and instancing;
baseline GPU metrics.

## What is implemented

```text
beamcraft export-view --store ROOT --out FILE|-
beamcraft-ui --store ROOT [--cli PATH]
beamcraft-ui --store ROOT --metrics FRAMES [--width W] [--height H]
beamcraft-ui --synthetic COUNT --metrics FRAMES [--width W] [--height H]
```

| Area | What exists |
|---|---|
| `beamcraft_engine::query::bundle` | `ViewSnapshot::to_bundle` and `from_bundle`: the pinned snapshot as one bounded byte bundle (pin, token, elements, mesh blobs). Import re-inspects every blob, recomputes every digest, rebuilds with `ViewSnapshot::build` and requires the rebuilt token to equal the bundled one. |
| `beamcraft-cli` | `export-view`: the verified head's snapshot (the same one `inspect-elements` reports) as a bundle on standard output or in a new file. |
| `apps/beamcraft-ui` (new, F08 row now `now`) | `source` (runs the CLI, bounded read, engine rebuild), `scene` (one vertex buffer per distinct mesh, one instance per element, f64 re-centring), `camera` (orbit/pan/zoom/fit, Z up, mm), `render` (wgpu offscreen target, depth, 4x MSAA when supported, one draw call per distinct mesh), `metrics` (headless baseline), `app` (eframe window, info panel, controls, original window icon). |

## Decisions

- **Viewport stack.** The V01 spike's stack: wgpu 30.0.1 with eframe/egui 0.37.0, default features
  off (no `default_fonts`), Inter at run time. PdfCraft vendor patches and craft-libs are not used;
  `craft-quit` is not needed yet. `accesskit` is **off**: on Linux it pulls zbus 5 crates that
  RustSec now marks unmaintained (zbus_names, zvariant, zvariant_derive), which fails `cargo deny`
  advisories. Re-enabling accessibility (and ignoring or upgrading those advisories) is an owner
  decision; no ignore entry was added.
- **Process boundary instead of a crate edge.** F08 forbids an app depending on an app, and only the
  CLI links the layer engine. The UI therefore runs `beamcraft export-view` and receives the
  engine's snapshot as bytes; the engine re-validates them. The renderer computes no engineering
  geometry: it decodes BCMESH01 positions and indices and derives display-only flat normals (F06
  section 6 has no normal section).
- **Accepted snapshot.** The published head as `inspect-elements` sees it (verified revision, kernel
  re-evaluation, view snapshot); the viewport shows the same token.
- **Instancing.** Equal meshes are equal digests in the snapshot (copies share their body, P05), so
  each distinct mesh is uploaded once and drawn with an instance buffer of translations. Placements
  are translations only in this scope.
- **Units and precision.** Millimetres throughout; the info panel shows bounds in metres and says so.
  World coordinates are re-centred on the engineering-bounds centre in f64 before the f32 upload,
  and each mesh's vertices are made relative to the centre of its own bounds (the anchor, added
  back to its instance translations in f64), so large element-local coordinates keep their size.
- **Budgets.** Instancing decouples drawn triangles from uploaded bytes, so the scene refuses
  more than the snapshot's `max_projected_triangles` per frame. Snapshot meshes share their bytes
  (`SceneMesh` holds `Arc` storage), so a bundle that references one mesh many times cannot
  multiply memory before validation.
- **Framing.** The camera fits the bounding sphere into both the vertical and the horizontal field
  of view of the actual viewport aspect; a scene it cannot frame keeps the previous one and shows
  the error.
- **Never crash.** Window-system libraries are dlopened by third-party crates, which panic when one
  is missing (seen with `libxkbcommon-x11`); the window start is wrapped in `catch_unwind` and
  reported as an error.

## Evidence

- `apps/beamcraft-ui/tests/viewport.rs`: a real store built with the CLI (one plate, five copies,
  one other plate) is exported by the CLI process and rebuilt; the token equals `inspect-elements`'s;
  seven elements become two meshes (instances 6 and 1); bounds are 7000 x 2300 x 20 mm and the
  origin is the bounds centre; an empty store is the CLI's `NOT_FOUND`; an actual offscreen render
  and pixel readback shows background corners and lit geometry. A synthetic engine snapshot renders
  a lit plate at the centre. Both render tests skip (and say so) without an adapter.
- `apps/beamcraft-cli/tests/view_export.rs`: bundle equals the inspected snapshot, copies share
  one mesh, an existing file is never overwritten, an empty store and bad options are refused.
- Engine unit tests: bundle round trip (with and without simplified meshes), truncation, padding,
  a moved placement and every flipped byte refused or token-identical.
- UI unit tests: scene instancing and re-centring, flat normals, empty scene, camera fit keeps every
  corner of a 6 x 3 x 0.5 m box in view, percentiles.
- The window was **run** on Linux under Xvfb with the software adapter (llvmpipe, Vulkan): it showed
  the store's seven plates, the info panel and Inter text (screenshot not committed).

### Baseline metrics (software adapter)

Linux x86_64 container, Mesa llvmpipe (LLVM 20.1.2, 256 bits), `DeviceType::Cpu`, Vulkan, release
build, 1280 x 800, 4x MSAA, 300 measured frames after 10 warm-up frames, CPU wall clock.

| Source | Elements | Meshes | Triangles | Upload bytes | Submit median / p95 ms | Completion median / p95 ms |
|---|---:|---:|---:|---:|---:|---:|
| store (7 plates) | 7 | 2 | 84 | 1 812 | 0.071 / 0.099 | 5.80 / 6.76 |
| store (37 plates) | 37 | 2 | 444 | 2 172 | 0.077 / 0.113 | 6.21 / 7.71 |
| synthetic | 1 000 | 1 000 | 12 000 | 876 000 | 1.19 / 1.65 | 10.76 / 13.32 |
| synthetic | 10 000 | 1 496 | 120 000 | 1 412 544 | 2.17 / 3.08 | 29.84 / 37.49 |

These are software-rasterizer numbers on one machine, not hardware GPU results; hardware runs
(Windows/macOS, discrete and integrated GPUs) are not done. Synthetic plates vary in size, so
1 000 of them are 1 000 distinct meshes and cost one draw call each; merging small distinct meshes
(V01's merged path) is not implemented here.

## Not done

- **Store scale is blocked upstream:** in this test a store stopped accepting elements at 37
  plates: the next `copy-element` is refused by the layer engine with `LIMIT_EXCEEDED` ("stage
  budget exceeded"), so a real store cannot yet reach the V01 scales; the 1k/10k rows use synthetic
  engine snapshots. That budget is outside V03.
- No picking, selection, snapping (V04), tools (V05), properties or publication UI (V06); no
  representation switch in the UI (the engine supports full and simplified).
- Rigid member placements, including rotation, are drawn from the snapshot: the rotation is
  composed into the member mesh (issue #130) and the snapshot carries the translation; rotated
  plate placements remain outside the current placement admission path.
- No live refresh: R reloads; no change subscription.
- Windows and macOS builds and runs, and the Rust 1.96 MSRV build of this stack, were not run here.
- The window was run only under Xvfb with a software adapter; no interactive acceptance on a
  physical display.
