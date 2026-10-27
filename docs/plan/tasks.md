# Tasks

Every task by ID. Size: **S** is about 2–4 hours of focused work, **M** about 4–8; a guide, not
a promise. A large task is split into numbered slices (A07.1, V05.2, ...) before it starts, and
the parent ID stays the acceptance gate. Status is the state on `master`:

- **merged**: slices are on `master`, experimental, acceptance not claimed (see the design note);
- **accepted**: the acceptance criteria hold on the integrated result;
- **done**: a research, audit or contract task that delivered its report or document;
- **open**, **deferred** (after the MVP), **later** (needs an owner decision).

Status as of 2026-10-20, `master` at `d456670`. New tasks are added here in a docs-only PR.

## F: foundations and contracts

| ID | Task | Depends on | Result | Acceptance | Status |
|----|------|------------|--------|------------|--------|
| F01 / S | Integration baseline | — | Audit of the base commit, licence and rules; list of roadmap discrepancies | Exact base SHA, clean state, discrepancy matrix | done |
| F02 / M | Requirements map | — | Index requirement → source → gate → test; owner decisions above the specification | Every source classified; unknowns marked; IFC import not in the MVP | done, kept outside the repo |
| F03 / M | OpenUSD distribution audit | — | Audit of the SDK version, TOST, dependencies, packaging and platforms | Reproducible delivery option and its conditions; no exception added unasked | done; superseded by F11 variant C, audit removed |
| F04 / M | [Model and command contract](../contracts/f04-model-and-commands-v0.1.md) | F01, F02, F09 | UUID and path, parameter types, schema version, errors, base revision, read/write dependencies, create/update/delete | Contract and negative cases agreed; no hidden JSON model; units and limits set | frozen 0.1.0 |
| F05 / M | Private USD-worker protocol (contract deleted by U17) | F04 | Versioned requests and responses, capabilities, timeouts, cancellation, limits, revision token | Same contract for CLI and host; private IPC without a listening socket | retired; removed by U17 |
| F06 / M | [Geometry and viewport contract](../contracts/f06-geometry-contract-v0.1.md) | F04 | Kernel inputs, results and diagnostics, FeatureId, mesh buffers, transforms, query API | Shape, world measurement and presentation have separate keys; topology and bounds bounded | frozen 0.1.0 |
| F07 / M | Original synthetic fixtures | — | Generator and manifest of our own test bodies, provenance, analytical references | No vendor asset; references separate from unverified cases; stable fixture IDs | done (`tests/fixtures/f07`) |
| F08 / M | [Module boundaries](../contracts/f08-module-boundaries-v0.1.md) | F04, F05, F06, F07 | Crate layout and allowed dependencies; CI targets | Pure crates pass the WASM check; native host isolated; checker in CI | living document; checker in CI |
| F09 / M | Clean-room separation for the kernel | F01, F02 | Admission rules, requirement provenance, a package without source-derived material | Independent review of the specification passed | done, then retired by decision 2026-10-20 |
| F10 / M | [Domain vocabulary and discipline modules](../contracts/f10-domain-vocabulary-v0.1.md) | F04, F08 | Common element types and attributes, namespaces, ownership, Opening/hosted-in relations, rule catalogue, discipline-module contract | Steel is the only implemented module; every rule has a positive and a negative case | frozen 0.1.0 |
| F11 / M | USD layer-engine decision | F03, F05, F08 | Comparison of SDK worker, own subset, `openusd` crate and hybrid | Recommendation marked as a proposal to the owner | done: variant C chosen ([decisions](decisions.md)); the comparison document is removed |
| F12 / M | [Section projects and pinned coordination](../contracts/f12-project-coordination-v0.1.md), [F10 0.2 amendment](../contracts/f10-domain-vocabulary-v0.2.md), [stage 4 opening assignments](../contracts/f12-opening-assignments-v0.1.md), [stage 6 Craft reading](../contracts/f12-craft-read-capabilities-v0.1.md), [external IFC references/reports](../contracts/f12-external-reference-workflow-v0.1.md) | F04, F08, F10 | One documentation package: module/section/zone separation, standalone projects, coordination pins, file-based assignment batches, exact-version decisions, common viewing, module-gated editing, single-section work with foreign IFC references and reports | Rules and positive/negative cases reviewed; frozen contracts preserved; late acceptance of an older proposal cannot approve or overwrite its successor; implementation gates explicit | open; foundation, stages 4/6 and external-reference workflow approved 2026-10-22, contract drafts; implementation tasks U19–U23 and X01–X06 (#189) |

## N: spikes and evaluations

Time-boxed research with a report; never a production gate.

| ID | Task | Depends on | Result | Status |
|----|------|------------|--------|--------|
| N01 / M | Layer-engine spike on `openusd` | F11 | Prototype on synthetic fixtures, measurements, differential check | done; spike and report removed |
| N02 / M | IFC-lite evaluation | — | Evaluation as an IFC path | done; not adopted; spike and report removed |
| N03 / M | Linux wrapper build and SDK-oracle verification | N01 | Build and differential checks against the official SDK where available | open |
| N04 / M | `truck` kernel evaluation | — | Evaluation as a kernel | done; not adopted; spike and report removed |
| N05 / M | SolidCraft kernel evaluation | — | Evaluation for contour extrusion and plane cuts | done, report not in the repo; rejected by decision 2026-10-16 (own holes and cuts) |
| N07 / S | Variant sets for viewing packages only | N01, O02, U12 | variantSets generated only in an export package for USD viewers; authored model keeps none | open |
| N08 / S | Local compatibility run on a third-party USD sample | U01, R01 | Black-box run of the reader and R01 outside the repository; report without the sample's bytes | open, needs a licence check |

## U: USD storage, layers and revisions

| ID | Task | Depends on | Result | Acceptance | Status |
|----|------|------------|--------|------------|--------|
| U01 / M | Open and read USD | F08, F03 | Bounded open/read with verified roots and traversal limits | API and CLI open our fixture; a damaged file is an error | merged (partial) |
| U02 / M | Codeless schemas and authored layers | U01, F04 | Element and plate attributes, model/derived composition | OpenUSD reads the attributes without schema registration; own validator separate | merged |
| U03 / M | UUID and relationship index | U02 | Full UUIDv7, ID → prim path, relationship validation | Same-millisecond IDs not shortened; duplicate and dangling targets refused; rename keeps the ID | merged |
| U04 / M | CLI and native host | U01, F05 | Command dispatcher, machine-readable results | One command contract, correct exit codes, timeout and cancel | merged (partial) |
| U05 / M | Immutable assets | F08, F04 | Full SHA-256, publish-if-absent, byte verification, path containment | A repeated publication never overwrites a blob; a bad digest is diagnosed | merged |
| U06 / M | Commit of a complete revision | U02, U03, U05 | Immutable generations and an atomic root switch | The root reads only complete generations; expected base checked; no partial save | merged |
| U07 / M | Journal and recovery | U06 | Consistent intent/commit record and recovery of an interrupted save | Fault injection before and after the switch opens the old or the new snapshot; recovery idempotent | merged |
| U08 / M | Portable appearance | P02, U09 | Visual material: `UsdShade` Material + `UsdPreviewSurface`, `material:binding` on the element, `primvars:displayColor` as fallback; colour comes from presentation, not from the engineering material | A colour change keeps shape and mass keys; usdview and Omniverse show the colour; no MDL or external textures | open |
| U09 / S | Standard metadata for other programs | P01, P03 | Model `kind`, `displayName` from `bim:name`, authored `extent`, normals for flat faces | `extent` matches the computed bounds; a rename changes `displayName`, not path, ID or shape key | merged |
| U10 / M | Payload geometry and selective loading | U09, N01, V02 | Element or zone geometry as payloads, `extentsHint`, a bounds-only snapshot | Opening with unloaded payloads gives IDs, properties and bounds; measured on 10 000 elements | open |
| U11 / M | Instancing of shared geometry | P05, U09 | `instanceable` reference to an immutable geometry asset | 1000 copies give one prototype; editing one copy publishes a new blob only for it | open |
| U12 / M | Representation levels | U09, V02, U10 | Axis (`guide`), Simplified (`proxy`), Reduced (cuts without holes), Full (`render`) | Viewers switch `purpose`; quantities, BOM and snaps read Full only; the others are marked inexact | open |
| U13 / S | Lighting as a presentation layer | U08, U09 | Optional weakest sublayer with a dome and a distant light outside `defaultPrim` | Light is in no shape or mass key; a reference to our model carries no lamps | open |
| U14 / M | History compaction and garbage collection | U06, U07, P06, P07 | `inspect-history`, `compact-history` with `--dry-run`, `--keep-head`, `--keep N`, backup, journal rotation | Dry run changes nothing; compaction atomic and fault-tolerant; only unreachable blobs removed; refused with a live workspace base or a running coordinator | merged |
| U15 / M | Per-element derived layers | U06, P04 | Unchanged elements are not rewritten on every edit | Editing one element of 1000 adds about one element's size to the store; measured before and after | open |
| U16 / M | Prim naming rule | U02, U03 | Paths by the [naming rule](../architecture.md#prim-hierarchy-and-naming-rule): category folders, three-letter prefixes, discipline folders; schema, layer engine, engine, CLI, fixtures | Every element and definition is written and read at its rule path; no store of the old layout is migrated; tests and fixtures regenerated | open |
| U17 / M | Remove the USD worker process | U01, U04 | Delete `beamcraft-usd-worker`, its host supervision, the F05 wire rules and contract; USD is read in the main process ([issue #146](https://github.com/Apibim-com/BeamCraft/issues/146)) | No process is spawned to read USD; a corrupt or hostile file gives a stable error code, never a panic or hang | merged |
| U18 / M | Project upgrade | U06 | Explicit "import into the current version" of a project written on an older vocabulary line; the result is a new revision, the original stays in the history (F10 section 4.6) | Older projects open read-only; upgrade migrates every element; nothing is upgraded silently | open |
| U19 / M | Section and zone layers, active section | U02, P06, U22, F12 | Authored section/zone scopes named `<code>[_<zone>]_<section>.usda`; one active section and authorized zone set per workspace; F10 0.2 scope rules distinguish KM/KMD from the steel module | Same-module edits outside the active scope and stronger foreign overrides refused; shared metadata uses explicit authorized commands | open |
| U20 / S | Project metadata and zones | U06, D1, U22, F12 | Project metadata, stable section/zone registries, frame and contract requirements in the coordination root; CLI to set and show them; exact schema/commands required before implementation | Metadata survive every revision and workspace fork; labels grant no authority; checkout uses the naming rule | open |
| U21 / M | Coordination of several projects | U20, R01, F12 | Read-only connections to independent section or zone projects, pinned by project/revision/manifest; explicit updates and placement; retained dependency closure coordinated with U14 | Source publication leaves old coordination unchanged; verified update is atomic; missing dependencies, bad frames and live-pin deletion refused | open |
| U22 / M | [Vocabulary 0.2.0](../contracts/f10-domain-vocabulary-v0.2.md) schema | U02, U03, F12 | Exact USD serialization and schema pin of the section, zone and provenance records; typed command envelopes and bounded parsers for them and for the shared commands (grids, metadata, decisions) | Every record round-trips through save and reopen; an unknown, oversized or malformed record is refused with an F04 code; 0.1 stores and commands are unchanged; nothing advertises 0.2.0 before this passes | deferred |
| U23 / S | Retained non-USD resources | U05, U06, F12 | Manifest entries for retained resources that are not USD layers (a received IFC, an issued report, an assignment package): role, media type, digest and size bound | Bytes verified like blobs; unknown roles and resources over the bound refused; existing manifests read unchanged | deferred |

## G: geometry kernel

| ID | Task | Depends on | Result | Acceptance | Status |
|----|------|------------|--------|------------|--------|
| G01 / M | Kernel math | F08, F06 | Checked vectors, planes, transforms, finite f64 policy | Zero axes, NaN/Inf and bad transforms diagnosed; Rust and WASM tests | merged |
| G02 / M | Rectangular plate | G01 | Standard mesh, analytical volume, local FeatureIds | Synthetic sizes and negative inputs checked; topology and winding | merged |
| G03 / M | Original kernel fixtures | G02, F07 | About 50 own descriptions with an independent harness | Supported and unsupported marked; references with provenance | merged |
| G04 / M | Kernel design | F06, F09 | Public mathematical sources, representation, predicates, tolerances, failure contract | Algorithms justified independently of other kernels | done; the kernel is built and accepted with conditions ([G07](../design/g07-kernel-first-scope-acceptance.md)); the draft design note is removed |
| G05 / M | Profiles, extrusion and triangulation | G03, G04 | Profiles with cavities and straight sweeps | Own fixtures check topology and volume; bad contours refused | merged |
| G06 / M | Topology and measurements | G03, G04 | Shell, orientation and volume checks, bounded spatial index | Open or unsupported meshes never get an exact status | merged |
| G07 / S | [Acceptance of the first kernel scope](../design/g07-kernel-first-scope-acceptance.md) | G05, G06, G09 | Evidence of correctness and speed, list of supported operations | Readiness shown by fixtures; unknown explicitly unsupported | accepted with conditions |
| G08 / M | Kernel integration for the accepted scope | G07 | Bounded API, cancellation and revision checks, engine commands | Real API and CLI end-to-end use our kernel; a failure never damages USD | open; contract amendment A-2 done |
| G09 / M | Pilot plane cuts and booleans | G05, G06 | Bounded own operations on agreed bodies | Independent fixtures check volume, bounds, topology and failure | merged |
| G10 / M | Faster kernel certification | G08 | Spatial index instead of a face-pair search; tens of holes per plate or flange | Measured before and after; `OpLimits` revised by measurement | open |
| G11 / M | Accuracy fixtures | G06, F07 | Fixtures for rounded-mesh cross-checks, subnormal mass, very large coordinates and cancellation-heavy inputs, with references from exact rational arithmetic | The kernel stays within the F06 comparison rule on every fixture, or the constants of F06 section 4 are tightened | open |
| G12 / M | Sweep along a path | G05, G09 | A section swept along a polyline or an arc, mitred at corners, curves faceted to a stated chord deviation | Volume against the analytical (Pappus) value within the stated deviation; bad path refused | open |
| G13 / M | Ruled and bent surfaces | G12, G09 | Ruled surface between two contours, cylinder and cone patches, fold lines, with thickness | Closed certified mesh; area and volume against analytical references; deviation reported | open |
| G14.1 / M | Transverse holes through member sections | G09, G05, [member hole amendment](../contracts/f06-member-hole-amendment-draft-v0.1.md) | Slice of G14: a round hole perpendicular to the member axis along a section direction, through whole flat walls (web, flange, hollow-section walls), through or over a range ending in free space, certified closed shell | Volume `A * t` per wall for parallel walls; blind, edge-touching, fillet and curved-wall cases refused; within the kernel capacity | open |
| G14.2 / M | Oblique member holes and curved walls | G14.1 | Slice of G14: hole directions other than the section directions, polygon holes, holes in round tubes | Volume against the analytic value or an enclosure; refusals as in G14.1 | open |
| A04.2 / M | Member holes: engine and CLI | A04 acceptance, G14.1 | `AddHole`, `UpdateHole`, `RemoveHole` accept a member target (`add-hole` with an axis point, a direction and an extent), stored as operation records in the member frame, replayed on edit | Geometry, volume and undo end to end; a hole that no longer fits keeps the member degenerate | open |

## P: core model

| ID | Task | Depends on | Result | Acceptance | Status |
|----|------|------------|--------|------------|--------|
| P01 / M | First end-to-end plate | U03, U04, U07, G02 | CreatePlate → USD authoring → derived → commit → open | Two independent IDs saved; ordinary OpenUSD reads geometry and attributes; volume checked | merged |
| P02 / M | Profile and material as USD assets | P01, U05 | Parametric profile, engineering material, immutable references (P02.2 section properties) | Material separate from shading; digest and units pinned; unknown density stays unknown | merged |
| P03 / M | Straight beam and column | P02, G01 | Straight member from a supported profile, placement, FeatureId | Length, local and world transforms and saved shape checked end to end | merged |
| P04 / M | Quantities and freshness | P01, P02 | Volume, mass, status and input signature; shape and mass invalidated separately | A material change updates mass without copying the shape; a rename rebuilds nothing; stale never shown as current | merged |
| P05 / M | Copy-on-write | P04, U05 | CopyElement with a new UUID, remapped internal links, shared geometry asset | Editing the copy creates a new blob; the original is unchanged | merged |
| P06 / M | Personal workspace | P01, U07 | Workspace and user IDs, base revision, draft save and reopen | Two workspaces independent; drafts never in the published root; unsent changes survive a restart | merged |
| P07 / M | Local undo | P06, P05 | Undo and redo of the draft with links and quantities | A copy or change is undone whole; errors and restarts leave no half operation | merged |
| P08 / M | Move and delete with dependencies | P03, P06 | Placement update and deletion with relationship preconditions | Moves never copy the shape; deletion never leaves a dangling link | merged |
| P09 / M | Validation layer | F10, U03 | One validator (dangling links, duplicates, module rules) for client and post-merge | Each rule has a positive and a negative case; bounded and deterministic | open |
| P-Mass / M | Mass from the section area | A07 | Mass = area × length × density; area from the catalogue or computed exactly | Source of the area visible; see decision 2026-10-18 | merged |
| P-Degenerate / M | Degenerate element state | G08, F04 amendment | A wrongly built element is kept without derived data and recomputed once fixed (decision A-2 D-7) | — | open |
| P10 / M | Rotated placement | G08 | Engine and kernel accept any rigid placement (translation and rotation); world bounds, picking, snaps, quantities and IFC use it; the view snapshot carries the full transform; cut points of every element in its local frame | A rotated plate is created, measured, picked and exported end to end; a member's rotation never changes shape or mass keys. Member rotation through save/reopen, world bounds, picking and feature geometry is implemented for issue [#68](https://github.com/axisrow/BeamCraft/issues/68); the broader plate and IFC acceptance remains open | open |
| P11 / M | Instance copy | P05, F04 next version | A copy that shares the source's definition (shape inputs, profile, material, operations); editing it changes every instance; each instance keeps its own ID, placement, level and relationships (F10 section 5.3) | Editing the shared definition updates all instances in one transaction; a duplicate stays independent | open |
| P12 / M | Polybeam | P03, P10, G12 | A member along a polyline path with one section, corner treatment and point editing | Constant section along the path; mitre volume checked analytically | open |
| P13 / M | Curved beam | G12, P12 | A member along an arc (start, end and radius, or three points) | Length and volume against analytical values within the stated deviation | open |
| P14 / M | Section placement | P10, P03 | Position of the section about the member axis: plane, depth, rotation, offsets at both ends; columns | Own fixtures per combination; the default placement unchanged | open |
| P15 / S | Part classes | P01 | An integer class on every part, our own palette as an environment preset, filter and display by class | The palette is our own data; the class is an authored attribute | open |

## R: references

| ID | Task | Depends on | Result | Acceptance | Status |
|----|------|------------|--------|------------|--------|
| R01 / M | Read-only USD reference | P01 | Attach wrapper, pinned source, no edits or overrides, explicit units correction on the wrapper | Writing the source or a foreign prim is refused; bytes preserved; network paths refused | merged |
| R02 / M | Measurements of a foreign model | R01, P04, G08 | Supported mesh measurements as our own result objects (W03) | Units and transform respected; open or self-intersecting meshes never exact; mass never invented | deferred |
| R03 / M | Explicit reference update | R02, U05 | Source versions and comparison of supported properties and geometry | No automatic update; nothing written to foreign prims | deferred |
| R05 / M | BIM properties of foreign models | R01 | Read-only "declared properties" from common USD BIM conventions; external IDs for R03 | Own synthetic fixtures; strings with units never become exact values | open |
| R06 / M | "Built from" link, KM to KMD (point 5) | U19, R01 | Earlier candidate: source-element/revision links, creation of KMD from KM and source-change review; revisit scope and design after the MVP | Acceptance to be decided after the MVP; current reference/assignment evidence requirements remain in F12 | deferred; owner decision after the MVP |

## X: section coordination and external references

The implementation of the F12 package after the MVP (#189). U19 to U21 come first, proven on two
small synthetic projects that use the steel module for both KM and KMD. Each rule named below
needs a positive and a negative executable case; no existing test is relabelled as evidence.

| ID | Task | Depends on | Result | Acceptance | Status |
|----|------|------------|--------|------------|--------|
| X01 / M | [Opening assignment](../contracts/f12-opening-assignments-v0.1.md) identity and versions | U19, U22, F12 | Assignment identity and immutable proposal versions; an edit publishes a successor; competing successors need an explicit resolution | OA-ID-001, OA-VER-001: identity kept across an edit; other content under an existing version is `INTEGRITY_ERROR`; competing successors are `DEPENDENCY_CONFLICT` | deferred |
| X02 / M | Assignment packages: export and import | X01, U23 | Request and response package files with receipts, deduplication, bounded input and recovery of an interrupted import | OA-PKG-001, OA-PKG-002; unknown package versions, hostile paths and truncated input refused; a receipt alone approves nothing | deferred |
| X03 / M | Host decisions and opening implementation | X02, A04 | Decision records bound to the exact proposal version and the reviewed host revision; explicit supersession; the authorized command that creates the opening | OA-AUTH-001, OA-DEC-001 to OA-DEC-004 in both delivery orders, OA-IMPL-001: imported acceptance creates no hole, a later version never inherits an earlier acceptance | deferred |
| X04 / M | [Common Craft read API](../contracts/f12-craft-read-capabilities-v0.1.md) | U19, U22, F06 | Identity, properties, representations and capability descriptors over one pinned snapshot; unavailable-geometry and display-only states; content of unknown modules kept on publish | CR-READ-001 to 003 and CR-EDIT-001 to 003 on an original synthetic unknown-module fixture, preserved through save and reopen; a skipped render is not evidence | deferred |
| X05 / M | [External IFC reference](../contracts/f12-external-reference-workflow-v0.1.md) conversion | E04, R01, R03, R05, U23 | One IFC delivery becomes one logical read-only USD reference with its original bytes, a conversion record, GlobalId mapping, units and frame mapping and a loss report; explicit update | XR-SOURCE-001/002, XR-IDENTITY-001, XR-READ-001, XR-FRAME-001, XR-LOSS-001, XR-UPDATE-001 on independent synthetic conversion fixtures | deferred |
| X06 / M | Opening reports and external replies | X03, X05, U23 | A report of opening requests readable without Craft, with exact proposal versions; an external response recorded as evidence; external completion kept apart from observed geometry | XR-REPORT-001, XR-REPLY-001, XR-IMPL-001: report delivery grants no approval; a reply never cuts the reference | deferred |

## C: collaboration

| ID | Task | Depends on | Result | Acceptance | Status |
|----|------|------------|--------|------------|--------|
| C01 / M | Changeset and idempotency | P06, P08, F05 | Base, op_id, actor, dependencies, USD delta, assets; bounded validation | The same op_id with the same payload is safe, with another refused; an author string grants nothing | merged |
| C02 / M | Local coordinator | C01, U07 | Explicit loopback server, token auth, one publish queue, expected head | Off by default; refused without a token; no parallel overwrite of the head | merged |
| C03 / M | Rebase of independent edits | C02, P04 | Footprint comparison with intermediate revisions | Two independent edits kept; unsent draft never lost | deferred |
| C04 / M | Semantic conflicts | C03, R01 | Same property, atomic groups, read dependencies, delete/link | A conflict keeps both sides; no silent server-wins | deferred |
| C05 / M | Two offline clients | C04, P05, P07 | Harness for 1000 + 1000 IDs, retries, crash and two workspaces | No path collisions; accepted snapshot readable by OpenUSD | deferred |
| C06 / M | Undo of a published operation | C05 | Compensating command with current preconditions | A later edit by someone else is never rolled back | deferred |
| C07 / M | Live delta channel | C05 | Optional loopback channel over the same changesets | Off by default; same conflict semantics | later |
| C08 / M | Permissions and layer storage | C02 | Permission model and remote storage | Separate owner decision on hosting | later |

## V: native UI

| ID | Task | Depends on | Result | Acceptance | Status |
|----|------|------------|--------|------------|--------|
| V01 / M | Viewport spike | F06, F07 | `wgpu` + `egui` spike on 10 000 own objects; UI stack audit | Measurements and font and asset audit | done |
| V02 / M | View snapshot and query contract | P01, F06 | Incremental buffers, revision token, bounds, ID mapping, selection API | Available through API and CLI; the renderer computes no engineering geometry | merged |
| V03 / M | Window and model display | V01, V02 | Viewport, camera, loading the accepted snapshot | Real engine model, not a mock; instancing; baseline GPU metrics | merged |
| V04 / M | Picking and snaps | V02, R01, G01 | Picking, axis and vertex snaps, stable IDs | Headless end to end with independent expected positions | merged |
| V05 / M | Create and edit tools | V03, V04, P07 | V05.1 create plate and member; V05.2 select, move, copy, delete, undo | The UI calls the same commands; cancel publishes nothing; saved result equals the CLI's | merged |
| V06 / M | Properties and publication | V05, C05 | Property editor, local/accepted/stale states, publish (V06.1 without conflicts) | Unknown mass and unsupported states visible | merged (V06.1) |
| V07 / M | Discipline view presets | V03, R01, W01, U12 | Presets: reference visibility and underlay, kind filter, representation, colours, enabled options | Switching never changes the model or the snapshot token | open |
| V08 / M | Work plane | P10, V04 | Session work plane (origin, X and Y axes): parallel to XY/XZ/YZ with an offset, by three points, by a picked face, vertical through two points; tools enter 2D points in it; the CLI takes `--plane ORIGIN,XDIR,YDIR` | A plate drawn in any work plane gets the matching placement; the work plane is never stored in the model | open |
| V09 / M | Views by two points and by plane | V08, W01 | A view looking across a line or onto a plane, with depth clipping and its work plane; switch between views | A view never changes the model or the snapshot token; saved views restore it | open |
| V10 / M | Ribbon | V05, V06 | Own ribbon of tabs and command buttons replacing the side panels; own icons only | Every button runs the same engine command as the CLI; nothing taken from other products | open |
| V11 / M | Selection switches and snap panel | V04, T01.4 (grid switch and snaps) | A bottom bar of selection switches (parts, assemblies, bolts, welds, holes, grids, construction objects, items) and a snap panel | A switch changes picking only, never the model or the snapshot token | open |
| V12 / M | Views | V08, V09, W01, T01.4 (view by grid line) | Named 3D and plane views (by grid line, level or two points), several windows, view properties, work area | A view never changes the model; saved views restore it | open |
| V13 / S | Projection switch | V03 | Orthographic and perspective camera in one window | Picking and snaps stay correct in both | open |
| V14 / M | Filters | I04, P15 | A rule engine on type, name, class and attributes with AND/OR groups, saved sets; object filters and view filters | Same result through API, CLI and UI; unknown attribute never matches silently | open |
| V15 / M | Cut plane | V12 | Clip planes and a work-area box per view | Non-destructive; no change to the model or measurements | open |
| V16 / M | Render modes | V03, U12 | Five modes on Ctrl+1 to Ctrl+5 (own set, user-remappable): wireframe, transparent, grey, shaded, selected only | One engine query; frame time baseline; shortcuts configurable | open |
| V17 / M | Colour by filter and class | V14, V16, P15 | Saved representation sets (ordered filter rules with colour and transparency) and a temporary colouring overlay | The overlay is never stored in the model; a saved set lives in the views layer | open |
| V18 / M | Plate contour grips | A12.3, V05 | Highlighted contour with vertex and edge-middle grips; drag, insert, delete a vertex, corner treatment editor; the plate stays in its plane | The UI calls the A12.3 commands; a drag is one undo unit; the saved result equals the CLI's | open |
| V19 / M | Bolt group tool | A14.3, V05 | Pick the fastener from the library, the washer switches and the hole setting, then two points on the active plane, the position values (`on_plane`, rotation, depth), offsets and spacings; preview of fasteners and holes; edit a selected group | The UI calls the A14.3 commands; cancel publishes nothing; the saved result equals the CLI's | open |

## A: fabrication operations

| ID | Task | Depends on | Result | Acceptance | Status |
|----|------|------------|--------|------------|--------|
| A01 / M | Assemblies | P03, P08, U03 | Minimal assembly hierarchy, cycle and ownership checks | Create, attach and detach end to end | merged |
| A02 / M | Contour plate | G08, P04 | Closed planar contour, triangulation, stable features | Bad contour refused; authored geometry kept | merged (contour in z = 0) |
| A03 / M | Plane cut | G08, P03, P05 | Cut as an owned `CutByPlane` operation record (amendment A-2) | Volume and freshness updated; stable features or an explicit loss | merged as element attributes; operation record open |
| A04 / M | Plate hole | G08, A02, P05 | Through hole as an `AddHole` operation record, parameters and tolerance | Geometry, volume and undo end to end; other hole types explicitly unsupported | merged pilot; acceptance and operation record open |
| A05 / M | Minimal bolt group | A01, A04 | Bolt group, links to the parts, generated holes | Read/write dependencies touch the linked parts; deleting a parent conflicts | merged without generated holes |
| A06 / M | Minimal weld | A01, P03 | Weld links and properties without full solid geometry | Workshop/site and side explicitly authored | merged |
| A06.2 / S | Workshop weld builds an assembly | A06, A16.1, [weld assembly amendment](../contracts/f04-weld-assembly-amendment-draft-v0.1.md) | Slice of A06: a workshop weld joins its two parts into one assembly in the same transaction (the first part is the main part, the second is attached to it); a site weld does not | Engine, CLI and end-to-end test of the amendment cases; one undo unit; a conflict changes nothing | open |
| A07 / M | Section families and libraries | P02, G08 | A07.1 nine families and custom outlines; A07.2 names and libraries; A07.3 environments | Analytical references; unknown designations never substituted | merged |
| A08 / M | Numbering preview | A01, P04, P05 | Equivalence of supported parts, series and overlap, stale tracking | Preview approves nothing; material and operations counted | merged |
| A09 / M | Numbering approval | A08, A03–A06 | Serialisable approval over the current revision; single-user for the MVP | A fabrication change makes it stale; a camera change does not | merged |
| A10 / M | Bent plate | G13, A12 | A plate folded along lines or rolled to a cylinder or cone section, with thickness and bend radii | Analytical area and volume; the unfolded outline is reported | open |
| A11 / M | Lofted plate | G13, A12 | A plate between two contours that need not be coplanar | Closed certified mesh; refusal for twisted or self-intersecting input | open |
| A12 / M | Complex plate contours | A02, G12 | Contours with arcs, chamfered and rounded corners, plates in any plane, point editing | Replaces the z = 0 limit of A02; arc deviation reported | open |
| A12.1 / S | Plate thickness side | A02, [plate amendment](../contracts/f04-plate-amendment-draft-v0.1.md) | Slice of A12: `top`, `middle`, `bottom` laid off from the contour plane; the default `top` keeps old plates; numbering normalises the side away | `z` ranges, equal volume, an older plate reads as `top`, plates differing only in the side share a number; engine, CLI and end-to-end test | open |
| A12.2 / M | Plate corner treatment | A12.1, plate amendment | Slice of A12: chamfer (one or two distances) and rounding (radius) on a contour vertex, authored as parameters, arcs faceted to the stated deviation, `arc_deviation_mm` reported; needs no sweep, so no G12 | Amendment cases 2 to 5 and the negative cases; undo, copy and reopen exact | open |
| A12.3 / M | Plate contour editing commands | A12.2, P-Degenerate, plate amendment | Slice of A12: move vertex, move edge, insert vertex, delete vertex (never below three), set corner, as engine commands and CLI | Amendment cases 6 and 7 and the negative cases; one undo unit each | open |
| A13 / M | Fits, cuts, chamfers, split and combine | A03, A12 | Line, polygon and part cuts, fit an end to another part, chamfers, split a part in two, combine two parts, as A-2 operation records | Volume, mass and stable features; undo of every operation | open |
| A14 / M | Complete bolt groups | A05, A04, A12 | A group is one straight row of real fasteners (a rectangular arrangement is several groups; circular and free-point patterns are not planned for the MVP); round through holes generated in every connected part or none; length, washers and nuts; split into the slices A14.1 to A14.4 below (contract: [bolt amendment](../contracts/f04-bolt-fastener-amendment-draft-v0.1.md)); oversize and slotted holes, blind holes and per-part hole types come after the MVP | Holes match in all parts; deleting a part conflicts as in A05 | open |
| A14.1 / M | Fastener definitions and libraries | A07, bolt amendment | Slice of A14: a fastener definition (head, nut, washer, hole clearance, standard lengths) in a fastener library in the A07 format, pinned into the project by digest; a synthetic example library and one European DIN set (bolt, nut, washer); no environments yet | A group never needs the library after pinning; an unknown designation is refused; every shipped value has a recorded source and licence | open |
| A14.2 / M | Fastener geometry | A14.1, A05, U11 | Slice of A14: head, shank, optional nut and washers as closed meshes (hex and round polygons to the 0.05 mm chord rule, no threads), one prototype per group, counts and mass per component | Volumes against the analytic values; a group of `n` shares one mesh; mass source shown | open |
| A14.3 / M | Group placement and editing | A14.2 | Slice of A14: two points, a reference plane with rotation, `on_plane` and `depth`, offsets, spacings or generators, automatic or fixed length, washer switches, `CreateBoltGroup` and `UpdateBoltGroup` as engine commands and CLI; A05 groups stay readable as annotations | Positions, length and washer cases of the amendment; one undo unit per edit | open |
| A14.4 / M | Generated holes | A14.3, A04 acceptance, A04.2 | Slice of A14: `hole_mode` `generate` or `none`; generated holes are operation records of every connected part (plates, and members after A04.2), owned by the group, regenerated by an edit and removed with the group | Holes match in all parts and undo as one unit; `none` writes no part; a refused hole refuses the command | open |
| A15 / M | Weld geometry | A06, G05 | Weld types and sizes along an edge or polyline, an optional solid shape, symbol data | Workshop/site and side kept; solid volume against analytical values | open |
| A16.1 / S | Assembly main part | A01 | Slice of A16: the main part of an assembly, stored on the assembly, one per assembly and always one of its parts | A foreign part cannot be main; detaching the main part is refused or names a new main part; engine, CLI and end-to-end test | open |
| A16 / M | Nested assemblies and explode | A01, P08 | Sub-assemblies at any depth, a main part, add and remove parts, explode (dissolve one level into loose, unlinked parts, no change to the parts) | Cycle and ownership checks; explode and its undo leave the parts unchanged; numbering and BOM count nested parts once | open |

## E, D: exchange, reports and drawings

| ID | Task | Depends on | Result | Acceptance | Status |
|----|------|------------|--------|------------|--------|
| E01 / M | [Own IFC export contract](../contracts/e01-ifc-export-v0.1.md) | F02, P03, P04 | IFC version and subset, stable GlobalId mapping, own Rust writer | Mapping to the public specification and a list of losses | draft; deferred after the MVP |
| E02 / M | Minimal IFC exporter | E01, P03, P04 | Beams, columns and plates with placement, material, shape and a loss report | End to end; independent schema and geometry validation | merged; acceptance deferred after the MVP |
| E03 / M | BOM and reports | A01, P04, A09 | BOM of the accepted revision for supported parts | No double count; unknown and stale shown; reproducible | merged |
| E04 / S | IFC reference conversion study | F02, E01, F12 | Select supported IFC subset and converter for the approved read-only USD reference workflow; retain original bytes, identity mapping, units/frame and losses | Independent synthetic conversion cases; bounded input and provenance; no implicit native-element conversion; item import remains separate | open; reference workflow selected 2026-10-22 |
| D01 / M | Drawing contract and FeatureIds | F02, F06, P03, A01 | Drawing IDs, model revision, views, dimensions, marks, stale states | A dimension never addresses a triangle index | accepted (PR #233; contract v0.1) |
| D02 / M | First projection and dimension | D01, A03, A04, V04 | One view, one dimension with a stable reference, headless output | A lost feature is explicitly unresolved | accepted (PRs #102, #106) |
| D03 / M | Drawing manager and publication | D02, A09 | Document version, freshness, minimal print/export | Publication bound to a snapshot | accepted (PR #170) |
| D04 / M | Drawing views | D02, V12, V14 | The user places views on a sheet: a view from a model view with its filters, cut plane and direction, or of a chosen part or assembly; scale per view (default 1:100, set in the view properties); section and detail views by hand; a configurable view title | A view shows exactly what the model view shows; a model change marks it stale; nothing is placed automatically | open |
| D05 / M | Drawing sheets | D04 | Sheets of a chosen size, frame and title block fields filled from attributes, several views per sheet, moving and aligning views by hand | The same sheet reproduces the same output; unsupported sizes refused | open |
| D06 / M | Dimensions, marks and leaders | D01, D02, I04 | Dimensions, marks and leader callouts placed by hand and bound to FeatureIds; mark text from attributes | A lost feature is unresolved, never silently re-bound | open |
| D07 / M | Drafting tools | D03 | Lines, polylines, arcs, circles, rectangles, text, hatching, symbols, free dimensions, layers and line types | Drawn objects belong to the drawing only, never to the model | open |
| D08 / M | PDF and print output | D03 | A published drawing version to PDF and to the printer | Output is bound to a document version; fonts and symbols are our own | open |
| D1, D2 / S | Project ID and levels | P06 | ProjectId in the root layer; levels (storeys) in the model | Needed by E02 | merged |

## I: items and attributes

| ID | Task | Depends on | Result | Acceptance | Status |
|----|------|------------|--------|------------|--------|
| I01 / M | Item contract | F04, F06, U05 | An element with its own mesh geometry: an immutable mesh asset (full SHA-256), placement, units, own attributes; the geometry is read-only, the placement editable | Copy, move, delete and undo like any element; the asset is never rewritten | deferred |
| I02 / M | GLB items | I01 | A bounded reader of glTF 2.0 / GLB (public specification) into an item | Hostile files refused within size and depth budgets; units and axes explicit | deferred |
| I03 / M | IFC items | I01, E04 | Geometry of IFC4 products (extrusions and tessellations) read by our own reader into items; IFC class and name kept as attributes | No IFC model is converted into native elements; unsupported geometry is reported | deferred |
| I04 / M | User attributes | F04, U02 | Defined attributes (text, number, integer, boolean, choice, with units) on any element, attribute sets, property editor | Stored in USD; an unknown attribute is shown, never dropped | open |
| I05 / M | Schedules by attributes | I04, E03 | Schedule templates: columns from standard and user attributes, filters, grouping, totals; CSV and HTML | Unknown and stale shown; reproducible | open |

## T: model tools

| ID | Task | Depends on | Result | Acceptance | Status |
|----|------|------------|--------|------------|--------|
| T01 / M | [Grids](../contracts/t01-project-grids-draft-v0.1.md) | T01.1 to T01.7 (T01.8 and T01.9 are separate) | Grids as two families (U and V) of named straight GridLines, regular (from an expression such as `0 5*6000`) or drawn, with labels, snaps, placement on intersections and viewer geometry; first work item of the plan (owner decision 2026-10-23) | The acceptance cases of the contract (positive and negative) pass end to end; snaps on lines and intersections; hiding a grid never changes the model; ordinary section edits never change a grid | open; proposed T01 contract defaults under review; only magnetic T01.8 remains deferred |
| T01.1 / M | Grid model, regular families, labels and expressions: model, layer, engine, CLI | contract accepted, D2 (levels, merged) | Slice of T01: element kind `grid` at `/World/Grids/GRD_<uuid>`, GridLine records with AxisIds, the expression parser and label lists (`numeric`, `alpha`), limits, `CreateGrid`, `DeleteElement` and `CopyElement` of a grid, `list-grids` query, structural validator `T01-V01`; the exporters, BOM and numbering skip grids | Cases GR-P01 to GR-P04, GR-P09, GR-P10, GR-P14 and the creation negatives (GR-N01 to GR-N07, GR-N11 to GR-N13, GR-N15); reopen exact; one undo unit; engine, CLI and end-to-end test | open |
| T01.2 / S | Drawn lines | T01.1 | Slice of T01: drawn families, `draw-grid-line`, `reshape-grid-line`, `remove-grid-line`, `relabel-grid-line` | GR-P05, GR-N09, GR-N10; AxisIds kept; each command one undo unit; a refusal changes nothing | open |
| T01.3 / M | Editing a regular family | T01.1 | Slice of T01: `UpdateGrid` with a new expression and label list, lines matched by label, span and frame edits | GR-P06, GR-P15, GR-N19; AxisIds of kept lines unchanged; a removed line that is referenced is refused | open |
| T01.4 / M | Grid queries, snaps and placement | T01.1, V04 (merged) | Slice of T01: `grid_intersection`, `grid_end` and `grid_line` snap kinds with the `--grids` filter, `resolve-grid-point`, `resolve-grid-plane`, one-parameter `GRID:LABEL,LABEL` placement of members and plates with `--z` or `--level` and `--offset` | GR-P07, GR-P08, GR-P13, GR-N08, GR-N17, GR-N18; snap work bounded; an edited grid moves no element | open |
| T01.5 / S | Grid and level rights, pinned grids, the `Grids` layer | T01.3, U19, F12 | Slice of T01: grid and level commands under the shared scope (`DV-SCOPE-004`, `PC-OWN-002`); use of a grid of a pinned coordination project, read-only; grid and level elements move to the `Grids` layer | GR-P11, GR-P12, GR-N16; an ordinary section edit leaves grids and levels byte-equal; unauthorized command `PERMISSION_DENIED` | open |
| T01.6 / M | Grid tool, line drawing and snap switch in the UI | T01.2, T01.4, V05 | Slice of T01: grid creation and edit panel on the T01.1 to T01.3 commands (regular/drawn mode, expression and label fields), a line-drawing tool with snaps, display and labels, snap switch; visibility kept in the session until saved views (W01, V12) | The UI calls the engine commands; cancel publishes nothing; the saved result equals the CLI's; hiding changes no revision | open |
| T01.7 / S | Grid geometry for other USD viewers | T01.1 | Slice of T01: one 50 mm tube per GridLine at z = 0 in derived USD data with purpose guide; labels remain native UI text | GR-P16; another viewer shows the lines; authored geometry, picking, BOM and quantities ignore them | open |
| T01.8 / M | Magnetic mode | T01.4, T01.5, O-G5 | Slice of T01: placed elements follow a grid when the mode is on | Cases written when O-G5 is answered; a shared grid edit never rewrites section elements outside their scope | blocked on O-G5 |
| T01.9 / M | Curved lines and radial grids | T01.1, G12 | Slice of T01: arcs and other curves as GridLines, rings, radial grids, partial arcs | Written when scheduled | deferred (after the MVP) |
| T02 / M | Construction objects | V08, V04 | Helper points, lines, circles and planes as snap targets | Excluded from schedules and numbering; stored as helper elements | open |
| T03 / M | Copy, move, rotate, mirror, arrays | V05, P10, P11 | By vector or two points, linear and polar arrays, mirror about a plane | One undo step per operation; links and quantities remapped | open |
| T04 / S | Numeric input and direction lock | V04 | Typed absolute, relative and polar coordinates, axis lock | The same values give the same result in the CLI and the UI | open |

## Q: integration and delivery

| ID | Task | Depends on | Result | Acceptance | Status |
|----|------|------------|--------|------------|--------|
| Q01 / M | End-to-end alpha | V06, A09, E03, and the K9 to K11 tasks (for the MVP without E02, C06 and R03) | Model a frame with plates, bolts, welds and an item, view and filter it, number it, take a schedule and a drawing, in the UI and the CLI | Invalid files and crash regressions; OS and GPU matrix not replaced by mocks | open |
| Q02 / M | Packaging and release rehearsal | Q01 | Reproducible native packages of CLI, UI and licences | A clean machine opens a complete USD project; attribution checked | merged; rehearsal not run |

## W, O: views, review and design options

| ID | Task | Depends on | Result | Acceptance | Status |
|----|------|------------|--------|------------|--------|
| W01 / M | Saved views | V03 | Camera prim with author, time, comment and model revision in a views layer | Restored through API and CLI; no effect on model keys or numbering | open |
| W02 / M | Review notes | W01, U03 | Note bound to `bim:id`, revision, 3D point and camera; open/approved/rejected | A deleted element gives an explicit unresolved state; BCF later | open |
| W03 / S | Saved measurements | R02, V04 | Measurement object with references, units, precision, value, input signature, status | A shape change makes it stale | open |
| W04 / S | Compatibility with edits made in other USD tools | U06, U02 | Policy for foreign content in our root and layers | Publication keeps such data or moves it with a diagnostic; nothing lost silently | open |
| O01 / M | Design-option contract | F04, F10, C01 | Identity, lifecycle, layer layout, composition, footprint exclusion, effect on numbering, BOM, IFC and drawings | Acceptance follows the changeset path | open |
| O02 / M | Local design options | O01, P06, P07, P08 | Create, switch, compare, accept, reject in one workspace | A rejected option and its blobs are removed; the base is unchanged until acceptance | open |
| O03 / M | Comparing an option with its base | O02, V03 | Diff of added, changed and removed elements | Built on `bim:id` and footprints, not paths | open |
| O04 / M | Shared design options | O02, C04, W02 | Proposals for review, notes, acceptance through the coordinator | No silent server-wins | open |
