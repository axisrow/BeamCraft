# Roadmap

**Stage:** pre-alpha. Scope updated 2026-10-30; gate verdicts are unchanged.

This file is the short summary of the [implementation plan](docs/plan/README.md): the stage, the
acceptance gates, the MVP scope and the next work. Task IDs, scope, dependencies, acceptance and
status live in [GitHub issues](https://github.com/Apibim-com/BeamCraft/issues/474), grouped by
[milestone](https://github.com/Apibim-com/BeamCraft/milestones). Owner decisions are in
[`docs/plan/decisions.md`](docs/plan/decisions.md). It is updated when a gate or the scope changes,
not for every merge. What each merged
package does is in its note under [`docs/design/`](docs/design/) and in
[`docs/architecture.md`](docs/architecture.md); the history is git and the pull requests.

## Where the product is

The CLI authors plates (rectangular and contour, with holes), straight members with rolled and
custom sections from definitions and libraries, one plane cut per element, assemblies, welds, bolt
groups and levels into verified USD revisions. It moves, copies, deletes and edits elements, keeps
personal workspaces with undo/redo and publishes them, compacts history, previews and approves
numbering, and exports IFC4 and a CSV bill of materials. The native window shows a revision,
picks and snaps, creates plates and members, moves, copies and deletes them, edits their
properties and publishes the workspace.

Every merged slice is experimental: "slices merged" below means the code is on `master`, not
that the gate's acceptance is claimed.

## Acceptance gates

| Gate | Acceptance result | Packages | Status |
|------|-------------------|----------|--------|
| K0 | Verified base, requirements, contracts and original fixtures | F01–F11 | **passed 2026-10-20**: F04, F06 and F10 frozen 0.1.0, F08 a living document, F05, F03 and F09 retired by decision; E01 is part of K6 and deferred after the MVP; F08 a living document; F05 retired |
| K1 | CLI creates a plate and atomically saves a complete revision readable by ordinary OpenUSD | U01–U07, U09, U14, G01–G02, P01 | **passed 2026-10-24** (owner decision; evidence and accepted limits in [`docs/plan/k1-k2-acceptance-evidence.md`](docs/plan/k1-k2-acceptance-evidence.md)) |
| K2 | Straight beam/column, quantities, independent copy, personal workspace and local undo | P02–P08, A07 | **passed 2026-10-24** (owner decision; rolled, custom and library sections, rigid member placement; same evidence document) |
| K3 | Foreign USD references remain read-only; supported measurements are separate results | R01–R03, G08 | R01 merged; R02–R03 deferred |
| K4 | Two clients safely publish, rebase, detect conflicts, retry and compensate accepted changes | C01–C06 | C01–C02 merged; C03 and C06 merged, C04 delivered, C05 delivered at suite scale (32 + 32 edits per client, its 1000 + 1000 sweep open); experimental, acceptance open |
| K5 | Original native UI uses engine commands and saves the same result as the CLI | V01–V06 | V01–V04, V05.1–V05.2 and V06.1 merged; acceptance open |
| K6 | Supported assemblies, cuts, holes, bolts, welds, approved numbering and IFC/BOM export | A01–A09, E01–E03 | slices merged for all; A03 cuts and A04 holes still use element attributes instead of the A-2 operation records; E01 contract draft; acceptance open |
| K7 | Complete alpha scenario and native packaging rehearsal | Q01–Q02 | Q02 packaging merged, release rehearsal not run; Q01 open |
| K8 | Associative drawings, dimensions and marks, drafting, a published document version | D01–D08 | in the MVP; open |
| K9 | Full manual steel modelling: polybeams, curved members, complex plates, fits and cuts, bolts, welds, nested assemblies | P12–P15, G12–G13, A10–A16 | open |
| K10 | Model tools, views, filters, cut plane, render modes, colour by filter, selection and snap panels | T01–T04, V11–V17 | open |
| K11 | User attributes and schedules | I04, I05 | open; items I01–I03 deferred |

## MVP scope

A free single-user desktop product for manual steel detailing without automation (no parametric
connections or macros). The model is ordinary USD that other USD tools open. The scope was widened
on 2026-10-21 and extended with both IFC input workflows on 2026-10-27
([decisions](docs/plan/decisions.md)); package boundaries follow the implementation plan and each
package keeps its own PR.

1. **Modelling:** columns, beams, polybeams, curved beams, plates, bent and lofted plates and
   other plates of complex contour; full section placement; fits, cuts, chamfers, split and
   combine; part classes. Rolled and custom sections, contour plates, plane cuts and holes are
   merged as slices (A02, A03, A04, A07).
2. **Connections and assemblies:** bolt groups with holes in all connected parts, welds, nested
   assemblies, explode, numbering (A05, A06, A08, A09 merged as slices; A14 to A16 open).
3. **Attributes and schedules:** user attributes on any element, schedules from them (I04, I05).
4. **Model tools:** copy, move, rotate, mirror, arrays, grids, construction objects, numeric input
   (T01 to T04).
5. **Views and display:** named views, filters, cut plane, five render modes on Ctrl+1 to Ctrl+5,
   colour by filter or class, a bottom bar of selection switches, a snap panel, an orthographic and
   perspective switch (V11 to V17).
6. **Drawings, all manual:** views taken from model views and filters, sheets, hand-placed
   dimensions, marks and leaders, configurable view titles, drafting tools, PDF and print (D01 to D08).
7. **Output and packaging:** numbering, schedules and BOM (E03), the alpha scenario (Q01) and
   native installers for Windows, Linux and macOS (Q02; Q02 merged, Q01 open).
   The owner builds and company-signs Windows x64/ARM64 releases locally. Apple developer
   registration, Developer ID signing and notarization are after the MVP.
8. **IFC input:** read-only IFC reference models for authoring one's own section, and a separate
   steel IFC import through the engineering kernel into an editable project, including KM to KMD.
   Both are required in the MVP; the import contracts and implementation remain open. This is
   separate from the deferred I01-I03 mesh-item workflow and IFC export.

Regional profile/material coverage targets Australia, Europe, Brazil, Canada, China, Russia,
India, Middle East, Philippines, South Africa, South America and USA. Exact standards, families
and admitted sources remain to be specified. All shipped data must satisfy `AGENTS.md` §1.2;
no commercial-product catalogue, palette or text is copied.

Deferred until after the MVP: items (I01 to I03: the item contract and reading GLB and IFC geometry), IFC export (E01, E02; the experimental `export-ifc` stays in the code
but is not part of the MVP), the multi-client live delta channel and permissions with layer
storage (C07, C08), reference measurements (R02–R03), DSTV NC and DXF output, parametric
connections and macros. The multi-client rebase, conflict, retry and compensation slices
(C03–C06) are merged or delivered on `master` as experimental slices; their acceptance is open.
Which parts later stay open source and which become a commercial product is decided after the MVP;
until then everything stays MPL-2.0.

## Next work

In this order (milestones M0 to M5; M1 to M5 are those of the decision of 2026-10-21, and the owner
put grids first on 2026-10-23); the task cards are indexed in the
[GitHub task catalogue](https://github.com/Apibim-com/BeamCraft/issues/474).

0. **M0, grids:** review the [T01/F04/F10 grid contract amendment](docs/contracts/t01-project-grids-draft-v0.1.md)
   with its MVP defaults, then T01.1 to T01.4 (model, layer, engine, CLI, drawn lines, snaps and
   placement), T01.5 (rights, pinned grids, with U19), T01.6 (UI) and T01.7 (50 mm derived USD
   guide tubes). Magnetic mode (T01.8) waits on its ownership/scope decision; curved and radial grids
   (T01.9) stay after the MVP. Views by grid line (V12) and the grid switch (V11) build on it.
1. **M1, modelling and the UI shell:** A03/A04 to the operation records (A-2); kernel sweep and
   ruled surfaces (G12, G13); polybeams, curved beams, section placement, complex, bent and lofted
   plates (P12 to P14, A10 to A13); complete bolts, welds and nested assemblies (A14 to A16);
   selection bar, snap panel, projection switch and render modes (V11, V13, V16).
2. **M2, tools and views:** construction objects, copy and arrays, numeric input (T02 to T04);
   views, filters, cut plane, colour by filter, classes (V12, V14, V15, V17, P15).
3. **M3, attributes and schedules:** I04, I05.
4. **M4, drawings:** D01 to D08.
5. **M5, alpha scenario and packaging:** Q01 over the whole scope and a first run of the release
   workflow (Q02); acceptance of the gates K5, K6 and K8 to K11.

Later work expands supported operations, additional data packs and DSTV NC export. The post-MVP
milestone M6 adds the collaboration cards C07 (live delta channel) and C08 (permissions and
layer storage). The owner
approved IFC reference loading and native steel IFC import for the MVP on 2026-10-27; broader
persistent source-change coordination remains separate. BeamCraft is desktop-only on
Windows, Linux and macOS; only `beamcraft-geom`, `beamcraft-model` and `beamcraft-kernel` keep the
WASM compatibility requirement as a layering discipline.
