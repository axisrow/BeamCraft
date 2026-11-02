# A15 weld geometry

Date: 2026-10-27. Status: **experimental vertical slice; A15 acceptance is limited as listed
below.**
Branch `feat/a15-weld-geometry`, based on fork `origin/master` (issue axisrow/BeamCraft#195,
milestone M1).

Plan row: A15 / M, depends on A06 and G05. Result: an optional solid along an authored path with
symbol data, beside the unchanged A06 weld properties. Acceptance: workshop/site and the side are
kept as authored; the solid volume is checked against analytical values; engine, CLI and
end-to-end.

## What is implemented

```text
beamcraft create-weld --store ROOT --name TEXT --main PART_UUID --secondary PART_UUID
                      --size MM --location workshop|site --side arrow|other|both
                      [--path X,Y,Z;X,Y,Z;...] [--all-around true|false]
                      [--contour none|flush|concave|convex] [--type fillet] [--new-id UUID]
beamcraft welds --store ROOT
```

| Area | What exists |
|---|---|
| `beamcraft_model::command` | `WeldContour` (`none`, `flush`, `concave`, `convex`), `WeldSymbol { all_around, contour }`, `WeldGeometry { path_mm, symbol }` and `CreateWeld.geometry: Option<WeldGeometry>`: a world polyline of 2 to `MAX_WELD_PATH_POINTS` points, each finite and within the coordinate cap, consecutive points at least `MIN_WELD_SEGMENT_MM` (0.001 mm) apart, a total length of at most `MAX_WELD_LENGTH_MM` (1e6 mm). A second fingerprint domain (`BeamCraft/A15/CreateWeldGeometry/v1` after the v1 body) keeps every A06 request at its exact previous bytes. |
| `beamcraft_engine::weld` | `solid(size_mm, geometry)` builds the fillet solid: one right-triangle prism (legs = the weld size) per segment, extruded by the kernel (G05) with its per-segment volume certificate and placed by a deterministic frame rule (segment tangent; reference up axis Z, then X when the tangent is near-vertical; the perpendicular the up axis projects onto). `WeldSolid` carries the merged sealed mesh, the analytic volume `size^2/2 * length` (the sum of the certified per-segment extrusion volumes) and a shape signature over the size and the whole path. `path_length` is pure. |
| `beamcraft_engine::solid` | `SolidMesh::composed`: an admitted, placed triangle set the kernel verified segment by segment; no kernel face table (a weld solid never reaches the scene). |
| `beamcraft_layer_engine::authored` | `Shape::Weld` gains `geometry: Option<WeldPath>`; when present the mapping authors `steel:weld:pathMm` (`double3[]`), `steel:weld:allAround` (`token(true|false)`) and `steel:weld:contour` (`token`) and accepts a `Change::Mesh` for the connection. The structural validator: a Geometry mesh under a `connection` is refused unless the weld carries `steel:weld:pathMm` (assemblies, levels, grids and plain welds stay bodyless); the path and symbol attributes are validated on loaded bytes and on candidates, and symbol data without a path is refused. The mapping descriptor gains the three optional lines; stores are repinned on edit as before. |
| `beamcraft_commands::weld` | `create_weld` builds the solid with the engine before anything is written (a kernel refusal refuses the command), appends the mesh change with a quantity record (`bim:volumeMm3` `current`, mass `unknown` — a weld has no material — algorithm `weld-fillet-prism-v1`) and compares the whole geometry on replay. `welds` lists `path_mm`, `all_around`, `contour`, `length_mm` and the stored `volume_mm3`; an A06 weld reads back with `path_mm: null`. |

## Decisions

- **Additive, not a semantics change.** `--path` absent means byte-for-byte the A06 weld: the same
  four properties, no body, the same v1 fingerprint. A06.2 (workshop weld builds an assembly) is
  not touched.
- **An edge is a two-point polyline.** One representation covers both; there is no separate edge
  picking.
- **The solid is a prism, not a joint simulation.** Each segment is one fillet prism with its
  right angle on the path; corners are plain butt joins, so the volume is exactly
  `size^2/2 * length` and the length is exactly the polyline length. The cross-section's rotation
  about the path follows the documented frame rule and does not change the volume. Mitred corners
  are G12 sweep work and stay open; the weld does not trim into the parts.
- **Symbol data is authored data.** All-around and the contour symbol are stored as authored and
  change no geometry; field/site stays `--location` (A06), so the symbol carries no duplicate
  field flag.
- **`allAround` is a token, not a USD bool.** The codebase authors no bool attributes; the
  token encoding is the proven one for this read profile.
- **The volume is analytic by construction and certified per segment.** The kernel's extrusion
  self-checks certify every prism; the reported total is the analytic `size^2/2 * length`, never
  a mesh estimate. No weld quantity enters BOM or IFC in this slice.
- **The mesh is written with the create, in one batch.** A standalone later mesh write to an
  existing weld is outside this slice (as for every non-plate element).

## Requirement coverage

BC-WELD, from A06's not-covered list, this slice adds: weld geometry and volume, weld length,
all-around and contour symbol data. Still not covered: intermittent welds, preparation and
bevels, other weld types (butt, plug, spot), NDT and quality classes, welded-assembly rules
(A06.2, separate), drawing symbols and weld schedules, IFC/BOM output, view-snapshot display of
the weld solid (the snapshot builds plates and members only).

Provenance: the geometry rules are our own, from the public definition of a fillet weld
(cross-section area `size^2/2`); the symbol fields follow the common ISO 2553 symbol structure
without quoting its text. No external product material was used.

## Evidence

- `apps/beamcraft-cli/tests/welds.rs` adds an end-to-end test: a weld along `0,0,0;0,500,0`
  (size 6) lists `length_mm` 500 and `volume_mm3` 9000 — exactly `6^2/2 * 500`; both layers
  change; workshop/site and side round-trip as authored; the operation replays and different
  symbol data under the same `op_id` is `OP_ID_REUSE`; symbol data without `--path` and a
  degenerate path write no revision; an A06 weld beside it lists `path_mm: null`.
- `crates/layer-engine/tests/authored.rs` adds a path-weld test: the three attributes author and
  read back, a derived mesh under the connection is accepted in the create batch, and bad paths,
  a bad contour token and a mesh under a weld without a path are refused with nothing written.
- `crates/engine/src/weld.rs` unit tests: straight and placed segments keep
  `size^2/2 * length` exactly (relative 1e-12 where the length is irrational), a three-segment
  polyline sums its segments, degenerate and non-finite segments fail closed, and the shape
  signature binds the size and the whole path.
- `crates/model/src/command.rs` unit tests: path and symbol validation with every error code, and
  the fingerprint — A06 bytes unchanged, geometry distinguishes equal commands.
- `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo test --workspace` (1532 tests) pass; `cargo run -p beamcraft-cli -- --version` runs.
  The wasm32 check of the pure crates runs in CI (the target is not installed locally).

## Not done

- No command edits, moves or re-derives a weld geometry after creation; no mitred corners; the
  solid does not interact with the parts' bodies; nothing is exported (E02/E03); the view
  snapshot does not show the weld solid.
- `bim:discipline` is still not authored. Domain vocabulary 0.1.0 keeps `connection` reserved
  (`DV-KIND-004`); admitting the kind with the A15 attributes in the next vocabulary version
  stays the owner's decision, as A06 recorded.
