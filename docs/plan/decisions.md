# Owner decisions

Decisions that shape the plan, newest first. Each names its date and, where one exists, the
document that records it in detail. A later decision overrides an earlier one where they differ.
Open questions are at the end.

## 2026-10-27

Owner answers to [issue #129](https://github.com/Apibim-com/BeamCraft/issues/129):

- **Windows releases are built and signed locally by the owner.** A local signing capability is
  available. The owner will build and release Windows x64 and ARM64 versions on their own
  computer, signing them with the company's signing identity. This is the release policy;
  it does not claim that ARM64 packaging or signing automation is already implemented.
- **Apple developer registration and distribution signing are after the MVP.** No Apple developer
  setup is currently available. Developer registration, the paid membership, Developer ID signing
  and notarization are deferred to M6 - PostMVP. Native macOS packaging remains in the MVP;
  this decision does not remove the existing ad-hoc package signing.
- **Both IFC input workflows are in the MVP.** IFC files can be loaded as read-only reference
  models so users can author their own discipline/section against them. Separately, steel IFC
  models can be imported through BeamCraft's engineering kernel into a working, editable project,
  including the use case of creating steel detailing (KMD) from steel design (KM).
  Reference loading never makes the reference editable. Native steel import is a separate entry
  point, not the deferred I01-I03 mesh-item workflow. This supersedes the earlier blanket deferral
  of these two IFC input workflows. It does not bring IFC export into the MVP or approve the
  broader persistent R06 `builtFrom`/source-change workflow. Supported IFC subsets, mapping,
  kernel admission, refusal behavior and end-to-end acceptance still need an import contract;
  approving scope is not a claim of implementation or acceptance.
- **DSTV NC export remains after the MVP**, in M6 - PostMVP.
- **Regional profile and material catalogue coverage:** Australia, Europe, Brazil, Canada, China,
  Russia, India, Middle East, Philippines, South Africa, South America and USA. Brazil and South
  America are both retained as requested. Exact standards/editions, families and data sources
  must be specified per regional pack. The existing asset policy remains binding: every shipped
  value needs admitted provenance, a provable permitted licence and attribution. Naming a region
  does not admit any catalogue or commercial-product data.
- **The open-source/commercial question is resolved for the MVP.** The existing decision stands:
  BeamCraft's own code stays MPL-2.0 through the MVP; any later split is considered after it.
- **Acceptance gates are separate from issue #129.** That issue records owner decisions only.
  Gate verdicts require their own evidence and owner acceptance; this entry changes none of them.

## 2026-10-26

- **T01 MVP defaults.** The user delegated the unanswered grid defaults to project best practices and
  current conventions. The proposed contract amendment resolves the 2026-10-23 questions for
  T01.1 to T01.7:
  - Explicit label lists accept Cyrillic and other permitted Unicode labels; automatic schemes are
    numeric and Latin alpha only.
  - A regular-family line drag rewrites the expression and matches lines by label to retain
    AxisIds. Family mode is selected at grid creation and cannot change in the MVP.
  - U lines run parallel to local Y, V lines parallel to local X, and the Z rotation moves the full
    grid frame. Default spans reach 2000 mm beyond the other family's coordinate range, including
    endpoint projections for drawn lines.
  - The intersection syntax is `GRID:LABEL,LABEL`; placement height is level elevation plus offset
    or explicit `--z`.
  - Other USD viewers receive 50 mm diameter guide tubes at `z = 0` in derived geometry. Labels
    remain native UI text.
  - Radial grids remain after the MVP with curves. Magnetic mode remains deferred until its
    ownership and scope dependencies are resolved.
  - Exact T01.1 to T01.7 acceptance is in the [T01 contract](../contracts/t01-project-grids-draft-v0.1.md);
    magnetic mode and curved grids stay separate slices.

## 2026-10-24

- **Windows CI runs on merge to master only.** Owner decision on
  [issue #180](https://github.com/Apibim-com/BeamCraft/issues/180): the Windows legs of CI (the
  Windows quality job and the Windows test archive with its shards) run on pushes to `master` and
  on manual runs, not on pull requests. This reduces demand instead of buying capacity: pull
  request bursts no longer queue for the account's scarce Windows runners. Linux and macOS still
  gate every pull request. A Windows-only failure is found after the merge and fixed in a
  follow-up; a pull request that touches Windows-specific code can run the full matrix by hand
  with `workflow_dispatch` on its branch.
- **Gates K1 and K2 are passed.** Owner decision on [issue #125](https://github.com/Apibim-com/BeamCraft/issues/125):
  the per-gate verdicts of [the K1 and K2 evidence](k1-k2-acceptance-evidence.md) are accepted
  together with the fixes merged since (#174 for the K1 plate contours, #222 for rigid member
  placement and the negative-width code). The limits still listed there are accepted, not
  waived as passes: a rotated plate placement (F06-POS-11) stays `UNSUPPORTED_GEOMETRY`, the
  face-table strings carry the `plate:f.` prefix, and the F04 envelope, workspace and
  trusted-context cases stay unrun until a decoder and a trusted context exist. The sign-off of
  K3 to K5 was still open at that date. Update 2026-10-27: gate acceptance is tracked separately
  from [issue #129](https://github.com/Apibim-com/BeamCraft/issues/129); this changes no verdict.

## 2026-10-23

- **Development starts with grids.** Owner decision ([issue #192](https://github.com/Apibim-com/BeamCraft/issues/192)):
  grids (axes) are the first work item, ahead of the milestones M1 to M5 of 2026-10-21, which
  become M0 (grids) followed by M1 to M5. The contract is written first, as a draft for review:
  [project grids](../contracts/t01-project-grids-draft-v0.1.md), with task T01 split into the
  slices T01.1 to T01.9. The owner's answers of the same day:
  - A grid is two families, U and V, of named lines (GridLines), as in the IFC schema (`IfcGrid`,
    `IfcGridAxis`). Each line has a name such as `A`, `1` or `Г-1`; the character `/` is allowed in
    a name. In the MVP a line is straight; arcs and curves, and with them radial grids, come after
    the MVP.
  - A regular family is typed as an expression, `0 5*6000` (first position, then distances, `N*D` for
    `N` equal distances); the expression is kept so a step is edited fast. Lines can also be drawn
    one by one. Names are a list of words separated by spaces; no letters are skipped.
  - Lines are records inside one grid element, with stable ids. A point on an intersection is one
    parameter in a command. Grid names need not be unique.
  - The height of a placed element is a level plus an offset. A section project uses a grid of the
    pinned coordination project, read-only, so a grid is not created again for every section.
  - Limits: 64 grids per project, 200 lines per family.
  - Grids are visible to other USD viewers as thin tubes (details open). A **magnetic** mode, in
    which placed elements follow a grid, is wanted (details open).
  - Open questions at the time were listed in section 12 of the contract (O-G5, O-G7, O-G11 to
    O-G16). The 2026-10-26 defaults below resolve the MVP questions except magnetic mode O-G5.
  - Grids and levels share one shared scope; an ordinary section edit changes neither. No export
    (IFC, BOM, numbering) carries grids in the MVP.

- **Plate position, corners and contour editing.** Owner decisions, written up as a draft in
  [the plate amendment](../contracts/f04-plate-amendment-draft-v0.1.md) for review:
  - A plate has a thickness side `top`, `middle` or `bottom` from its contour plane: `top` lays the
    thickness off in `+Z`, `bottom` in `-Z`, `middle` half each way. The default `top` keeps every
    existing plate unchanged.
  - A contour vertex may carry a chamfer or a rounding. A chamfer takes two distances, one along
    the previous edge and one along the next, or one distance for a symmetric cut. A rounding takes
    only a radius. A rounded corner is a stored radius, not a stored polygon; its mesh is faceted
    to a stated chord deviation.
  - A plate contour is edited in its plane: drag a vertex, drag an edge, insert a vertex in the
    middle of an edge, delete a vertex. A plate never drops below three vertices; delete is
    refused on a triangle.
  - Numbering follows the geometry: plates that differ only in the thickness side are one group
    (O-2 decided). Contour vertices stay addressed by position, with no stable ids for now (O-1
    decided).
  - Process: a reference screenshot of another product stays a private observation outside the
    repository (`AGENTS.md` §1.1); only the behaviour, described in our own words, is recorded.
- **Bolt geometry, fastener library and generated holes.** Owner decisions, written up as a
  draft in [the bolt amendment](../contracts/f04-bolt-fastener-amendment-draft-v0.1.md) for review:
  - A bolt group places a real fastener (head, shank, nut, washers as geometry). Its sizes come
    from a definition in a fastener library: the user picks the standard and size, and the washer
    under the head and under the nut are switches. The definition is pinned into the project, so
    changing the library changes no existing group.
  - A group is defined by two points (the line it follows), a reference plane with a rotation,
    an offset on the plane and a depth (these position values give the shank axis; there is no
    separate axis input), offsets, and spacings or a count and a pitch; one edit command changes
    any of them. One group is one row; a rectangular arrangement is several groups.
  - A setting says whether the group generates holes in the connected parts (bolts) or none
    (screws and other fasteners that need no hole). Hole types beyond the round through hole
    (oversize, slots, different holes per part, blind holes) come after the MVP.
  - Holes in beams (web, flange, hollow-section walls) get their own contract, written up in
    [the member hole amendment](../contracts/f06-member-hole-amendment-draft-v0.1.md) (tasks G14.1,
    G14.2, A04.2): bolted beam connections need them.
  - Environments are not built for now; the first fastener library holds one European DIN set.
    Values enter only with a recorded source and licence (`AGENTS.md` §1.2); the repository also
    ships a synthetic example library.
  - Open: which DIN or ISO numbers make the first set and who enters the values (O-B2a).
- **A workshop weld builds an assembly.** Owner decision, written up as a draft in
  [the weld assembly amendment](../contracts/f04-weld-assembly-amendment-draft-v0.1.md): a weld
  made in the workshop joins its two parts into one assembly automatically. The first part named is
  the main part and the second is attached to it. A site weld changes no assembly. A part in
  another assembly is a conflict for now, not a silent move (open O-W1: nest the whole
  sub-assembly instead).

## 2026-10-22

- **Point 5 deferred for a decision after the MVP.** Revisit the KM-to-KMD `builtFrom`
  workflow and broader inter-section source dependencies after the MVP; do not implement or
  finalize that design now. R06 is deferred, and the earlier KM-to-KMD description is a candidate
  direction to reassess, not committed MVP scope. Approved opening assignments (stage 4), common
  viewing (stage 6), exact reference pins and IFC conversion provenance remain in the current
  contract package. Deferring point 5 does not remove their required version/evidence tracking.
- **Section projects and reproducible coordination: foundation approved.** Distinguish an
  engineering module (`steel`) from a responsible project section (`KM`, `KMD`) and its optional
  zones. Sections can share one project store or publish independent projects under one common
  standard. A coordination revision pins the exact source revisions; a new section publication
  does not update coordination automatically. This extends the project-organization decision of
  2026-10-20 from independent zone projects to independent section projects as well. The draft
  specifications are [F12](../contracts/f12-project-coordination-v0.1.md) and its
  [F10 0.2 amendment](../contracts/f10-domain-vocabulary-v0.2.md). Their detailed clauses are for
  review; approved direction is not a claim of implementation or frozen contract acceptance.
- **One opening assignment.** The initiating engineer creates one shared assignment for an
  opening; the responsible host-section engineer decides whether it is acceptable. The physical
  opening is separately authored in the host model and linked to the assignment. Acceptance is
  distinct from implementation, and applies to the reviewed request/host versions. The reserved
  F10 mechanism is not an implemented workflow.
- **Stage 4 approved: reference-first design and assignment files.** Services engineers take a
  pinned structural project as a read-only reference and model pipes/ducts without cutting holes.
  They collect required openings and send a file containing individual assignments; structural
  engineers decide per item and author accepted openings in their own model. Decisions and
  implementation evidence return in a response file. Retransmission creates no duplicates.
  Acceptance of a 150 mm proposal never approves its 200 mm successor and never overwrites it;
  retain the old decision with the exact proposal/host versions it reviewed. A changed structural
  basis likewise requires revalidation. The [F12 stage 4 companion](../contracts/f12-opening-assignments-v0.1.md)
  specifies this workflow in the same branch and PR. Detailed encoding and executable acceptance
  remain open; this approval adds neither a general project-review system nor runtime features.
- **Stage 6 approved: common Craft viewing, specialized editing.** Every Craft application
  can inspect admitted project content from other sections. Editing requires the compatible
  owning engineering module, for example MEP capabilities in MEPCraft, plus authority over
  the active section/zone. Application names and viewing capability grant no editing rights.
  An unavailable module leaves its content preserved and read-only. Supplied display geometry
  does not establish engineering validity; unsupported representations remain explicitly
  unavailable. The [F12 stage 6 companion](../contracts/f12-craft-read-capabilities-v0.1.md)
  belongs in the same branch and PR. This is a contract direction, not a claim that MEPCraft
  or additional engineering modules have been implemented.
- **One workflow across company arrangements.** The same section-based workflow must work
  when a company authors all sections, authors only one and receives all other models as IFC,
  or authors several and receives the rest as references. Native authoring versus foreign
  referencing is a per-model distinction, not a project-wide operating mode. Module/scope
  permissions and exact-version assignment decisions are the same in all arrangements;
  internal exchange and external reports are delivery choices. F12 PC-WORK-001 requires all
  three scenarios in acceptance, independently of single-store or multi-project organization.
- **Single-section companies and external IFC references.** A company can author only KM or
  OV in Craft while receiving the other disciplines as IFC. One IFC delivery becomes one logical
  read-only USD reference with retained original bytes, conversion provenance, source identifiers,
  units/frame mapping and an explicit loss report. It never becomes editable native elements
  merely through conversion. Another Craft project or a shared coordination project is not a
  prerequisite. Opening assignments can be issued as human-readable reports to external engineers,
  with external replies explicitly recorded against exact proposal/source versions; no Craft
  application, account or machine-readable response is required from the recipient. The
  [F12 external-reference companion](../contracts/f12-external-reference-workflow-v0.1.md)
  specifies this extension in the same PR. This selects the reference-conversion boundary for
  E04; its supported IFC subset, converter and implementation remain to be specified. I01–I03
  editable-item work remains deferred and unchanged.
- **Existing foundations retained.** USD remains the model; source element identity, immutable
  revisions, typed commands and operation-based merge remain authoritative. The frozen F04/F06/
  F10 0.1 documents and their existing schemas stay unchanged. C03–C06 remain after the MVP;
  approval/review tooling, remote hosting and additional engineering modules are not added to
  the current implementation scope.

## 2026-10-21

- **MVP scope widened.** The first version is a full manual steel-modelling tool with views,
  display control, attributes and drawings, not only a frame with a bill of materials. Still
  a free single-user desktop product, still no automation: no parametric connections, macros or
  automatic detailing. The scope of 2026-10-18 and 2026-10-20 is replaced by the list below.
  1. **Modelling:** columns, beams, polybeams, curved beams, plates, bent plates, lofted plates and
     other plates of complex contour; fits, cuts, chamfers, split and combine; the full section
     placement (plane, depth, rotation, offsets); part classes.
  2. **Connection objects and assemblies:** bolt groups with holes in all connected parts, welds,
     assemblies, nested assemblies, explode an assembly, numbering.
  3. **Attributes and schedules:** user-defined attributes on any element; schedules built from
     them. Items (elements with foreign or own mesh geometry) are after the MVP (see below).
  4. **Model tools:** copy, move, rotate, mirror, arrays; grids; construction points, lines and
     circles; numeric input and direction lock.
  5. **Views and display:** named 3D and plane views, filters, cut plane, five render modes on
     Ctrl+1 to Ctrl+5, colour by filter or class (saved representation sets and a temporary
     overlay that never changes the model), the bottom bar of selection switches, the snap panel,
     the orthographic and perspective switch.
  6. **Drawings and drafting, all manual:** the user places views on a sheet; a drawing view shows
     what the chosen model view and its filters show (a view of the columns shows the columns).
     Dimensions, marks and leaders are placed by hand; the view title is configurable. Dimensions
     and marks stay bound to model features. No automatic views, dimensioning or mark placement.
     Drafting tools, PDF and print.
  Order of work (milestones, each its own set of PRs): M1 modelling and the UI shell (selection
  bar, snap panel, projection switch, render modes); M2 model tools, views, filters, cut plane,
  colour by filter, classes; M3 attributes and schedules; M4 drawings; M5 the alpha scenario
  and packaging over the whole scope (Q01, Q02). Gates K9 to K11 are added; K8 (drawings) is no
  longer deferred. The task cards are in [`tasks.md`](tasks.md).
- **Still after the MVP:** items (I01 to I03: the item contract and reading GLB and IFC geometry), IFC export (E01, E02), multi-client rebase and conflicts (C03–C06),
  reference measurements (R02–R03), DSTV NC and DXF output, parametric connections and macros,
  concrete and reinforcement, analysis.
- **Items after the MVP.** Items (I01), and with them reading GLB (I02) and IFC geometry (I03),
  are deferred whole. When they come back, only the geometry of a product becomes an item, no IFC
  model is converted into native elements, and files are untrusted input (`AGENTS.md` §4).
- **Own palette and shortcuts data.** The class colour palette, render-mode set and shortcut
  map are our own design and user-remappable presets (environment data). The Ctrl+1 to Ctrl+5
  habit is kept as a usability choice; no colour table, label or text of another product is
  copied (`AGENTS.md` §1–2). The owner's private functional notes stay outside the repository.
- **Explode means dissolve.** Exploding an assembly deletes the assembly and leaves its parts as
  loose, unlinked elements; geometry, placement and attributes of the parts do not change. It is not
  a display-only exploded view. Exploding acts on one level: the sub-assemblies of an exploded
  assembly stay assemblies. It is undoable; numbering becomes stale.
- **Curved geometry stays faceted.** Bent, lofted and curved parts are meshes built to a stated
  chord deviation, like holes (amendment A-2, D-2); no analytical curved-surface kernel for the
  MVP. The deviation is reported with the result.

### Defaults fixed for this scope

- Plates: only the kinds listed above (contours with arcs, chamfers and rounded corners, bent and
  lofted plates); nothing further in the MVP.
- Drawing views default to scale 1:100; the scale is a property of each view and can be changed.
  Further sheet defaults (sizes, title-block fields) are settled in task D05 and are presets the
  user can change.

## 2026-10-20

- **K0 passed.** Every foundation task is done or retired by decision: F04, F06 and F10 frozen at
  0.1.0, F08 a living document checked by CI, F01, F07 and F11 done, F03, F05 and F09 retired, and
  the F02 requirements map kept outside the repository.
- **IFC export after the MVP.** IFC needs independent validation and has many details of its own;
  the MVP takes the model out as a bill of materials, and the model itself is ordinary USD. The E01
  contract stays a draft and the experimental `export-ifc` stays in the code; their acceptance moves
  after the MVP.
- **Project organization.** Layers are split by discipline and optional zone, named
  `<code>[_<zone>]_<discipline>.usda` with the discipline codes `Coord`, `Grids`, `KM`, `KMD`, `KR`, `AR`,
  `AI`, `GP`, `EOM`, `VK`, `OV`, `SS`, `TH`, each present only when the project has that discipline;
  no program version in names. The coordination layer is
  the root and carries the project code, name, organization and zones. A team may instead keep one
  project per zone and coordinate them from a project on top. Levels are elements in the `Grids`
  layer, not layers of their own. Recorded in the architecture note; tasks U19 to U21.
- **No separate USD worker process.** USD is read and written in the main process by the pure-Rust
  layer engine. The worker existed to isolate the C++ SDK, whose crashes cannot be caught; with
  `openusd` a bad file is an ordinary error, own project files are verified by SHA-256 before
  parsing, and foreign files are read under size and depth budgets. The worker binary, its host
  supervision and the F05 protocol are removed (task U17, [issue #146](https://github.com/Apibim-com/BeamCraft/issues/146)). This replaces the separate-process decision of 2026-10-16.
- **Prim naming rule** (F04 amendment A-3; [architecture](../architecture.md#prim-hierarchy-and-naming-rule)):
  paths hold only category and identity (`/World/Steel/Parts/PRT_<uuid>`, `/World/Levels/LVL_<uuid>`,
  `/World/Definitions/Profiles/PRF_<sha256>`, ...); disciplines are second-level folders; names,
  assemblies, levels and marks are attributes and relationships. Compatibility is broken on
  purpose: stores written before task U16 are not migrated while the project is pre-alpha.
- **Assemblies are relationships, not parents.** A stable path keeps every link to a part valid
  when it changes assembly, and a part belongs to several groupings at once. Splitting storage into
  more files (per assembly, zone or element) is a separate performance decision for U10/U15 and
  changes no path.
- **No clean-room process.** BeamCraft is written from scratch with its own logic and nothing is
  reverse-engineered, so the F09 process (implementer admission, exposure records, isolated kernel
  agents) is retired and its documents removed. The asset and provenance rules of `AGENTS.md` §1–2 stay:
  nothing from Tekla or Trimble, no decompiling, no copied documentation, tables or GPL code.
- **No progress log.** History is git, the pull requests and the design notes; `ROADMAP.md` keeps
  only the stage, gates, MVP and next work and changes in docs-only PRs. A single log file would
  conflict in every PR. Release notes are generated from merged PRs at tag time.
- **Review threshold for contributor PRs.** A contributor PR goes back to its author only for a
  blocking problem: failing tests, a merge conflict, broken store or schema compatibility, or a
  contract violation. Wording and stale references are fixed by us in a follow-up PR.
- **A03 cuts merged as element attributes.** PR #113 (one plane cut per element, authored at
  creation, `steel:cut:*`) is merged although amendment A-2 calls for `CutByPlane` operation
  records; moving to the records is our debt under A03.
- **A09: stored numbering of PR #103** is the implementation; the competing PR #110 (manual marks
  `number-assign`, `bim:mark`) is closed. Manual mark editing may come later on top of #103.

## 2026-10-19

**F06 amendment A-2** (recorded in the [F06 contract](../contracts/f06-geometry-contract-v0.1.md) and
[`g08-f06-a2-notes.md`](../design/g08-f06-a2-notes.md)):

| # | Decision |
|---|----------|
| D-1 | The contract covers holes, plane cuts and contour extrusion with polygonal cavities; true curved surfaces are unsupported |
| D-2 | A round hole is a regular N-gon; N grows with the diameter (chord deviation ≤ 0.05 mm, at least 16, at most 256, a multiple of 4) |
| D-3 | A certified closed mesh reports the midpoint of its enclosing volume interval as `current`, with the interval width |
| D-4 | FeatureIds `extrusion` and `cut`; tokens `GEO_OP_UNSUPPORTED` and `GEO_OP_NO_EFFECT`; index addressing is forbidden |
| D-5 | No project limit on holes; the present capacity is a certification performance defect (task G10); until G10 an honest "kernel capacity exceeded" refusal, nothing published |
| D-6 | F06-NEG-39 and NEG-08 are superseded (A02.1 and A-1) |
| D-7 | A wrongly built element (for example a reversed contour) is kept as degenerate without derived data; once fixed, geometry and quantities are computed (task P-Degenerate) |

## 2026-10-18

- **A09 without C05** for the MVP: numbering approval is sequential and single-user.
- **Mass.** Mass = section area × length × density of the project material. The area comes from
  the catalogue when a profile is inserted from a library, otherwise the kernel computes it with
  the exact arc formula and the mass is marked computed (`area_source`: `catalogue` | `computed`).
  A tabulated mass per metre is a reference field only. Full values are stored; rounding happens
  only on output (0.1 kg by default). Check case: section 30B1 of GOST 27772-2015, 6 m, 7850 kg/m³
  gives 192.2 kg from the catalogue area (40.8 cm²) and 192.17 kg from the exact arcs.
- **Declared versus computed area (A07 D3).** A difference above 1 percent is a warning
  (`declared_area_warning`), never a refusal: catalogues use simplified radii.
- **History compaction (U14).** Undo past the compaction boundary is impossible and shown in the
  dry-run report; an old `op_id` is no longer recognised once its journal entry is rotated (entries
  referenced by kept revisions are not rotated); the head keeps its UUID and number; a real
  compaction needs `--backup DIR` or an explicit `--no-backup`.
- **Retention policy (proposal, not yet in code).** An undo window of the last 50 operations,
  configurable; milestones kept by compaction (manual, and automatic before numbering approval,
  IFC/BOM export and publication); no eternal history by default; U15 planned next to U14.

## 2026-10-17

- **USD features adopted from studying a public Omniverse sample** (observation only; the sample's
  bytes, names, geometry and materials never enter the repository): `UsdPreviewSurface` appearance
  with `displayColor` fallback, separate from the engineering material (U08); model `kind`,
  `displayName`, `extent` (U09); payloads for heavy geometry (U10); instancing of shared geometry
  (U11); units and axis correction on our reference wrapper, never on foreign prims (R01); saved
  views, review notes and measurements as our own objects bound to `bim:id` and revision (W01–W03);
  keeping foreign root-layer data that other tools write (W04). Not adopted: hash prim names,
  units baked into matrices, MDL-only materials, network asset paths, numbers with units in
  strings, overrides on foreign or instance-proxy prims.
- **Design options** are named delta layers over a base revision (an unpublished named changeset),
  not variantSets in the authored model. Switching is a view setting; accepting publishes the
  layer as an ordinary changeset; rejecting deletes it. Numbering, BOM, IFC and drawings use the
  accepted revision by default. VariantSets may be generated in a viewing package only (N07).
- **Discipline views** are view state (saved views), not model data. Own element representations
  use the standard `purpose` (Full `render`, Simplified `proxy`, Axis `guide`); engineering reads
  Full only.
- **Element data for schedules.** Three levels: authored inputs, immutable definitions by digest,
  derived quantities computed in the same transaction as the edit with an input signature. A BOM
  reads stored quantities from the snapshot and never recomputes them; totals are queries, not
  model data.

## 2026-10-16

- **USD layer engine: variant C** (F11): pure-Rust `openusd` 0.8.x behind a thin BeamCraft
  wrapper; the official SDK only as an explicit fallback and test oracle.
- **Desktop only** on Windows, Linux and macOS; only `geom`, `model` and `kernel` keep the WASM
  requirement.
- **Holes and cuts are our own implementation** without third-party kernels (scope widened by
  A-2 D-1). Member roll uses `f64::sin`/`cos`.

## 2026-10-15

- **Own kernel.** IFC-lite, Manifold, truck and OCCT are not production backends and are not
  copied or ported; the kernel is ours. No external math crates either: the scalar types and the
  ordered arithmetic of F06 are BeamCraft's own.
- **MSRV 1.96** (required by `openusd`).

## Open questions

| # | Question | Blocks |
|---|----------|--------|
| 1 | Independent IFC validation tool (E01 Q-8) | E02 acceptance |
| 2 | macOS runtime verification without project hardware; Developer ID signing/notarization is deferred after the MVP by the 2026-10-27 decision | Q02 runtime acceptance |
| 3 | IFC reference conversion and editable steel import: supported subsets, mapping, kernel admission, budgets and provenance (both input workflows in the MVP from 2026-10-27) | E04 reference-loading slice and native steel IFC import |
| 4 | Live channel and permission model (C07, C08) | C07, C08 |
| 5 | Grid contract draft: owner answers to O-G5, O-G7 and O-G11 to O-G16 ([contract](../contracts/t01-project-grids-draft-v0.1.md) section 12) | T01.3, T01.4, T01.7, T01.8 |
| 5 | Where saved views and review notes live: workspace only or a shared review layer | W01, W02 |
| 6 | Variant sets in our own layers (N07) | N07 |
| 7 | Local use of the third-party USD sample under its licence | N08 |
| 8 | Design options private or visible to everyone | O01 |
| 9 | What a rejected design option keeps besides its journal entry | O01 |
| 10 | Enabling options with overlapping footprints at once, marked as conflicting | O01 |
| 11 | Retention defaults of the history policy (window size, automatic milestones) | U14 follow-up, U15 |
| 12 | Which DIN or ISO numbers make the first fastener set and who enters the values (bolt amendment O-B2a) | A14.1 |
| 13 | Whole sub-assembly nested instead of a conflict when a workshop weld meets a part already in an assembly (weld amendment O-W1) | A06.2, A16 |
| 14 | Whether bolts and welds are numbered, and where defaults for omitted command arguments live (project, user or both) | A09 follow-up |
