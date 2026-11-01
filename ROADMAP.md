# Roadmap

**Stage:** pre-alpha. Last changed 2026-10-31; gate verdicts are unchanged.

This file holds what the owner decides: the stage, the acceptance-gate verdicts and the MVP scope
with what is deferred. It lists no tasks and no task status. Task IDs, scope, dependencies,
acceptance, status and the order of work live in the
[GitHub task catalogue](https://github.com/Apibim-com/BeamCraft/issues/474), its linked issues,
the [milestones](https://github.com/Apibim-com/BeamCraft/milestones) and the GitHub Project. Owner
decisions are in [`docs/plan/decisions.md`](docs/plan/decisions.md). This file changes only when a
gate verdict or the scope changes, not for every merge. What each merged package does is in its
note under [`docs/design/`](docs/design/) and in [`docs/architecture.md`](docs/architecture.md); the
history is git and the pull requests.

## Acceptance gates

Every merged slice is experimental: its code is on `master`, but the gate's acceptance is not
claimed until the gate below says **passed**. Closing a task does not pass its gate.

| Gate | Acceptance result | Packages | Verdict |
|------|-------------------|----------|---------|
| K0 | Verified base, requirements, contracts and original fixtures | F01–F11 | **passed 2026-10-20**: F04, F06 and F10 frozen 0.1.0, F08 a living document, F03, F05 and F09 retired by decision |
| K1 | CLI creates a plate and atomically saves a complete revision readable by ordinary OpenUSD | U01–U07, U09, U14, G01–G02, P01 | **passed 2026-10-24** (owner decision; evidence and accepted limits in [`docs/plan/k1-k2-acceptance-evidence.md`](docs/plan/k1-k2-acceptance-evidence.md)) |
| K2 | Straight beam/column, quantities, independent copy, personal workspace and local undo | P02–P08, A07 | **passed 2026-10-24** (owner decision; rolled, custom and library sections, rigid member placement; same evidence document) |
| K3 | Foreign USD references remain read-only; supported measurements are separate results | R01–R03, G08 | open; R02–R03 deferred after the MVP |
| K4 | Two clients safely publish, rebase, detect conflicts, retry and compensate accepted changes | C01–C06 | open; the acceptance beyond the independent CLI rebase is tracked in [#331](https://github.com/Apibim-com/BeamCraft/issues/331) |
| K5 | Original native UI uses engine commands and saves the same result as the CLI | V01–V06 | open |
| K6 | Supported assemblies, cuts, holes, bolts, welds, approved numbering and IFC/BOM export | A01–A09, E01–E03 | open |
| K7 | Complete alpha scenario and native packaging rehearsal | Q01–Q02 | open |
| K8 | Associative drawings, dimensions and marks, drafting, a published document version | D01–D08 | open; in the MVP |
| K9 | Full manual steel modelling: polybeams, curved members, complex plates, fits and cuts, bolts, welds, nested assemblies | P12–P15, G12–G13, A10–A16 | open |
| K10 | Model tools, views, filters, cut plane, render modes, colour by filter, selection and snap panels | T01–T04, V11–V17 | open |
| K11 | User attributes and schedules | I04, I05 | open; I01–I03 deferred after the MVP |

## MVP scope

A free single-user desktop product for manual steel detailing without automation (no parametric
connections or macros). The model is ordinary USD that other USD tools open. The scope was widened
on 2026-10-21 and extended with both IFC input workflows on 2026-10-27
([decisions](docs/plan/decisions.md)); package boundaries follow the implementation plan and each
package keeps its own PR.

1. **Modelling:** columns, beams, polybeams, curved beams, plates, bent and lofted plates and
   other plates of complex contour; full section placement; fits, cuts, chamfers, split and
   combine; part classes; rolled and custom sections, contour plates, plane cuts and holes (A02,
   A03, A04, A07, P12 to P15, A10 to A13).
2. **Connections and assemblies:** bolt groups with holes in all connected parts, welds, nested
   assemblies, explode, numbering (A05, A06, A08, A09, A14 to A16).
3. **Attributes and schedules:** user attributes on any element, schedules from them (I04, I05).
4. **Model tools:** copy, move, rotate, mirror, arrays, grids, construction objects, numeric input
   (T01 to T04).
5. **Views and display:** named views, filters, cut plane, five render modes on Ctrl+1 to Ctrl+5,
   colour by filter or class, a bottom bar of selection switches, a snap panel, an orthographic and
   perspective switch (V11 to V17).
6. **Drawings, all manual:** views taken from model views and filters, sheets, hand-placed
   dimensions, marks and leaders, configurable view titles, drafting tools, PDF and print (D01 to D08).
7. **Output and packaging:** numbering, schedules and BOM (E03), the alpha scenario (Q01) and
   native installers for Windows, Linux and macOS (Q02). The owner builds and company-signs
   Windows x64/ARM64 releases locally. Apple developer registration, Developer ID signing and
   notarization are after the MVP.
8. **IFC input:** read-only IFC reference models for authoring one's own section, and a separate
   steel IFC import through the engineering kernel into an editable project, including KM to KMD.
   Both are required in the MVP. This is separate from the deferred I01-I03 mesh-item workflow and
   IFC export.

BeamCraft is desktop-only on Windows, Linux and macOS; only `beamcraft-geom`, `beamcraft-model` and
`beamcraft-kernel` keep the WASM compatibility requirement as a layering discipline.

Regional profile/material coverage targets Australia, Europe, Brazil, Canada, China, Russia,
India, Middle East, Philippines, South Africa, South America and USA. Exact standards, families
and admitted sources remain to be specified. All shipped data must satisfy `AGENTS.md` §1.2;
no commercial-product catalogue, palette or text is copied.

## Deferred until after the MVP

- Items (I01 to I03: the item contract and reading GLB and IFC geometry).
- IFC export (E01, E02): the experimental `export-ifc` stays in the code but is not part of the
  MVP.
- The multi-client live delta channel and permissions with layer storage (C07, C08), the
  post-MVP milestone M6. The multi-client rebase, conflict, retry and compensation slices
  (C03 to C06) belong to gate K4, not to this list.
- Reference measurements (R02 to R03).
- DSTV NC and DXF output, additional data packs, and broader persistent source-change
  coordination.
- Parametric connections and macros.

Which parts later stay open source and which become a commercial product is decided after the MVP;
until then everything stays MPL-2.0.
