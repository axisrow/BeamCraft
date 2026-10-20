# Architecture

BeamCraft is a desktop-only Cargo workspace for Windows, Linux and macOS with an original Rust
engineering and geometry core.
USD is the planned authoritative project state. By the owner decision of 2026-10-16, the
pure-Rust `openusd` crate (0.8.x) is the primary layer engine behind a thin BeamCraft wrapper,
with the aim of no Python/C++ in the product path. The official SDK worker remains only an
explicit fallback and an external test oracle where available, subject to F03 admission.
[F11](design/layer-engine-decision-v0.1.md) records the evidence and open integration gates;
no production USD backend is integrated. U05 implements immutable byte storage only.

## Current implementation

```
apps/
  beamcraft-cli     command-line entry point (binary: beamcraft)
  beamcraft-ui      native window and viewport (binary: beamcraft-ui)
  boundaries        module-boundary checker (development tool, not shipped)
crates/
  geom              points, vectors, coordinate systems   (no deps)
  model             beam axis, F04 error codes, SHA-256 digest  (-> geom)
  protocol          F05 frame header, limits, IPC codes   (-> model)
  ifc               IFC4 STEP writer and test reader       (-> model)
  host              native immutable blob storage        (-> model)
tools/
  fixtures          F07 fixture generator and verifier
```

`geom` currently provides points and distance. `model` provides a beam axis and its length and the
23 stable F04 error codes plus `digest::ContentDigest` and incremental `digest::Sha256`.
`host::storage::BlobStore` provides publish-if-absent and verified retrieval of immutable bytes.
`protocol` provides the F05 frame header and binary prefix, the
negotiable limits, a bundle budget and the transport codes; it does no I/O. The CLI reports its
version. These types are a scaffold, not a USD engine or profile catalogue.

P01 adds the first vertical slice on top: `model::{id, command, limits}` (full UUIDv7 identities, `CreatePlate` admission), `engine::{command, footprint, transaction}` (kernel-evaluated rectangular plate, footprint, `op_id` decision), `host::ids`, and the CLI commands `create-plate` and `plate-inspect`, which author standard USD layers with the U02 layer engine and publish them as one atomic revision through U06/U07 ([design note](design/p01-first-plate.md)). It is a rectangular plate only, with no workspace, undo or process isolation of the layer engine.

P02 adds immutable profile and material definitions on top (`model::definition`, `engine::command::{MaterialInfo, MaterialLookup}`, `engine::footprint::Footprint::for_define`, `beamcraft_cli::definition`, and the CLI commands `define-material`, `define-profile` and `definitions`): a definition is a content-addressed `/World/Definitions/D_<sha256>` prim, a plate pins a material through `bim:material`, and mass is `current` only when that material authors a density ([design note](design/p02-definitions.md)).

P03 adds the straight beam and column (`kernel::member::straight_member`, `model::command::CreateStraightMember`, `engine::member`, `beamcraft_cli::member` and the CLI command `create-member`): a member pins a rectangle profile definition, the kernel builds the box in the F06 member frame, and members are re-evaluated at publish and reopen like plates ([design note](design/p03-straight-member.md)).

A04 adds plate through holes, authored with the plate (`model::CreatePlate.holes_mm`, the engine's holed plate body in `engine::command` with its `SolidMesh`, the layer engine's `steel:plate:holePointsMm` / `steel:plate:holeVertexCounts` attributes, and the CLI options `--hole-rect`, `--hole-round`, `--hole-segments`): the CLI generators produce clockwise polygons in the plate's local XY, the kernel cuts one through hole per polygon with exact predicates (strictly inside the outline, clear of the others, otherwise `INVALID_ARGUMENT`), the volume is the certified enclosure's midpoint with the mass from the resolved density, and the stored quantity signatures include the holes, so update, copy, move and undo keep them current. The polygons are stored as two plate attributes under an amended U02 schema — a recorded deviation from the F04 operation-record rule with owner review pending; there is no hole edit command and hole FeatureIds stay undefined (F06 amendment A-1) ([design note](design/a04-plate-holes.md)).

P04 adds `model::command::UpdateElement`, `engine::{update, quantity}`, `beamcraft_cli::edit` and the CLI command `update-element`: a name, material or thickness edit rewrites only the groups it changes, and volume and mass are stored as optional attributes beside the derived mesh with their own signatures, so a reader can tell current from stale ([design note](design/p04-quantities-update.md)).

P05 adds `model::command::CopyElement`, `engine::copy`, `beamcraft_cli::copy` and the CLI command `copy-element`: a new element with a fresh id, the source's authored inputs and pinned definitions, and the same body and mesh digest; the source is only read ([design note](design/p05-copy-element.md)).

P06 adds `beamcraft_cli::workspace` and the CLI commands `workspace-create`, `workspace-status` and `workspace-publish`: a personal workspace is a separate draft revision store forked from the published head, and it reaches the published head only by a fast-forward publish through the same verifier ([design note](design/p06-workspace.md)).

P07 adds `beamcraft_cli::history`, `RevisionStore::info_at`/`open_at` in the host and the CLI commands `workspace-undo`, `workspace-redo` and `workspace-history`: an undo commits the layers of the earlier state as a new verified draft revision (so element, mesh and stored quantities return together), and the undo and redo stacks are derived from the revision log plus an append-only `undo.log` ([design note](design/p07-undo-redo.md)).

P08 adds `Footprint::for_delete` and the placement patch of `UpdateElement` in the engine, `Change::Delete` in the layer engine, `beamcraft_cli::delete` and the CLI command `delete-element`: a move rewrites only the placement and never rebuilds the body, and a delete removes the whole element from both layers and is refused while another element links to it ([design note](design/p08-move-delete.md)).

R01 adds read-only foreign references (`model::reference`, `engine::reference`, `beamcraft_cli::reference` and the CLI commands `reference-attach`, `reference-move`, `reference-correct-units`, `reference-write` and `reference-inspect`): a foreign USDA/USDC layer is stored under its SHA-256 as an `external` layer of the revision and composed below an owned wrapper element whose placement and explicit units correction are stored apart from it; every write to the source or a foreign prim is refused, and a network path is refused before any I/O ([design note](design/r01-readonly-reference.md)).

A01 adds assemblies (`model::command::{CreateAssembly, AttachToAssembly, DetachFromAssembly}`, `engine::assembly`, `Footprint::for_part_of`, `Shape::Assembly` and `Change::PartOf` in the layer engine, `beamcraft_cli::assembly` and the CLI commands `create-assembly`, `assembly-attach`, `assembly-detach` and `assemblies`): an assembly is an element with its own identity and no body, and membership is the one `bim:partOf` relationship authored on the part, validated for kinds, cardinality, self-links and cycles ([design note](design/a01-assemblies.md)).

C01 adds changesets (`model::changeset`, `host::changeset` claims, `beamcraft_cli::changeset` and the CLI commands `changeset-create`, `changeset-inspect` and `changeset-apply`): a workspace's unsent revisions become one canonical, digest-addressed message (base, operations, claimed actor, conservative footprint, complete layer set with base digests, required blobs); the receiver checks the actor against the trusted caller, binds the changeset id to the payload before publishing, and publishes fast-forward only ([design note](design/c01-changeset.md)).

V03 adds the native window and viewport (`apps/beamcraft-ui`, `beamcraft_engine::query::bundle`, the CLI command `view-export`): the UI runs `beamcraft view-export` in a separate process, rebuilds and re-validates the engine's snapshot of the accepted revision, and draws each distinct mesh once with one instance per element ([design note](design/v03-viewport.md)).

V05.1 makes the window a command client (`beamcraft_ui::tools`, `beamcraft_ui::command`): a Create panel maps the entered plate or straight-member parameters to the argument list of the CLI's own `create-plate` and `create-member`, shows that command as the preview, and on commit runs it in a separate process, then reloads the accepted revision; a cancelled preview runs nothing, and a refusal is shown with the engine's code and message. The UI checks only that numbers are finite and that required choices are made; every engineering rule stays in the engine. Experimental: no geometry preview and no picking-driven placement ([design note](design/v05-1-ui-create-tools.md)).

V05.2 adds edit tools to the window (`beamcraft_ui::edit`, `beamcraft_ui::command::{select_at, element_info, commit_edit}` and `beamcraft-ui --workspace DIR`): a click is turned into a display ray and the engine's `pick` decides what was hit, the selection is drawn from the engine's element bounds, and move, copy and delete are previewed and committed as the CLI's own `update-element --translation`, `copy-element --offset` and `delete-element`, while undo and redo are `workspace-undo` and `workspace-redo` on a personal workspace the window was opened on. The UI checks only that numbers are finite and that something is selected; every refusal is the engine's, shown with its code and message. Experimental: no property editing (V06), no drag or snapping ([design note](design/v05-2-ui-edit.md)).
A08 adds the numbering preview (`beamcraft_engine::numbering`, `beamcraft_cli::numbering` and the CLI command `numbering-preview`): plates, beams and columns of the verified head with the same kind, pinned profile, pinned material, dimensions within a tolerance and holes form one group; the oldest part is the reference and groups are numbered in the order of their references, per kind series (prefix and start number), assemblies are grouped by the groups of the parts they hold, a position that an earlier group already takes is reported as overlapping, and a preview token changes exactly when an edit would change the proposed numbering. It is read-only and approves nothing: stored numbers and their approval are A09 ([design note](design/a08-numbering-preview.md)).
E02 adds the minimal IFC4 export (`beamcraft-ifc`, `beamcraft_host::output`, `beamcraft_cli::export` and the CLI commands `export-ifc` and `ifc-guid`): a new pure crate holds the deterministic 22-character GlobalId (the encoding of the UUIDv7 and a SHA-256 construction for entities that are not elements), the STEP lexical rules, the fixed entity templates and a structural reader that only tests use; the adapter opens a verified revision through the same reader as `plate-inspect`, so the kernel has already re-evaluated every plate, beam and column and compared its mesh, then writes plates, beams and columns as tessellated `IfcPlate`, `IfcBeam` and `IfcColumn`, levels as `IfcBuildingStorey`, every other kind (assemblies, welds, references) as `IfcBuildingElementProxy`, the pinned material, the published current quantities (a stale, missing or unknown one is left out and listed, never zero) and two property sets, through an atomic file install that leaves no output on a refusal. The store is only read, the GlobalId is computed and never stored, and the report lists every element with its GlobalId and every loss. Experimental: it has been checked only by the repository's own reader, an independent validation tool is not admitted yet ([design note](design/e02-ifc-export.md)).

A06 adds a minimal weld (`model::command::CreateWeld`, `engine::weld`, `Footprint::for_weld`, `Shape::Weld` in the layer engine, `beamcraft_cli::weld` and the CLI commands `create-weld` and `welds`): a weld is a `connection` element with no body, explicitly authored fillet size, workshop or site and side, and one ordered `bim:connects` record towards two distinct plates, beams or columns; a welded part cannot be deleted under it and a weld cannot be copied ([design note](design/a06-welds.md)).

D1 and D2 add the project-level model prerequisites of the IFC export (`model::command::CreateLevel` and `ElementPatch::level`, `engine::level`, `Footprint::for_level`, `Shape::Level`, `Change::Level` and the `project_record` reader in the layer engine, `beamcraft_cli::level`, `plate::ensure_project` and the CLI commands `create-level`, `levels` and `update-element --level`): the first revision records its ProjectId in the `customLayerData` of the root layer, where later revisions keep it and the commit verifier requires it to equal the manifest's project; a level is an element of kind `level` with its own identity, name and elevation and no body, and a plate, beam or column is assigned to it by the one `bim:level` relationship authored on the element, so a level never changes a body; a level in use cannot be deleted and a copy keeps the level ([design note](design/d1-d2-project-level.md)).

V04 adds picking and snaps (`engine::query::feature`, `engine::query::pick` and the CLI commands `pick`, `feature-lookup` and `snap`): `fid1` FeatureIds for box bodies derived from the snapshot's validated blobs, the F06 ray and point `Pick` with its ordering and limits, `LookupFeature`, and vertex and member-axis snaps; no result carries an index and an index is refused as an address ([design note](design/v04-picking-snaps.md)).

C02 adds a local coordinator (`host::coordinator`, `beamcraft_cli::coordinator` and the CLI commands `coordinator-serve`, `coordinator-publish` and `coordinator-head`): started only explicitly, bound to loopback, with clients authenticated by token files that also decide the trusted actor, and one bounded publish queue in which a publication names its expected head and is applied fast-forward through C01, so concurrent publications from one base accept exactly one ([design note](design/c02-coordinator.md)).

A07.1 adds section profiles (`model::section`, `kernel::section`, `Definition::SectionProfile` in the layer engine, `ProfileInfo::Section` in `engine::member` and `define-profile --family`): nine closed parametric families (I, channel, angle, RHS, CHS, lipped C, plain C, lipped Z, omega) and custom outlines are immutable, content-addressed profile definitions; the kernel generates a detailed outline (radii as polygons on the arcs) and a simplified one (sharp corners) centred on the section's bounding box and extrudes the detailed one along the member axis. Only the detailed body is stored so far; environments are A07.3 ([design note](design/a07-section-library.md)).

A07.2 (experimental slice) adds profile libraries and readable names (`beamcraft_cli::library`, the CLI commands `library-list`, `library-show` and `define-profile --library FILE --designation NAME`, `--profile NAME` and `--material NAME`). A library is a USD layer of named section entries (`/Library/Profiles/*`, the `steel:profile:*` attributes of a definition) that the layer engine reads under tight budgets (one self-contained layer, 8 MiB, 4096 entries); it is an input, never model state: resolving a designation publishes a complete, content-pinned definition, so a model never depends on the library file again, and a duplicate designation refuses the library. Every definition now carries a `displayName` on its prim, which is part of its content and therefore of its digest (the schema descriptor gains one optional line; the previous digest stays accepted); `create-member` and `create-plate` resolve a name against the head's definitions and refuse an ambiguous one with its candidates. A single `--library FILE` is read; the project, user and environment lookup order stays design only. A declared section area that differs from the exact section area by more than 1 percent is reported as a `declared_area_warning` and never refuses the definition (owner decision D3: catalogue values are authoritative) ([design note](design/a07-section-library.md)).

P-Mass (experimental slice) makes the member quantities `area x axis length x density` with an explicit area origin: the declared catalogue area of the pinned profile when it carries one (`area_source: catalogue`), else the exact closed-form area of the section with true circular arcs (`model::section::SectionShape::exact_area_mm2`; `computed`). The mesh stays the detailed polygon outline for display and geometry, so a mesh volume and a quantity volume can differ by the arc approximation. Quantities are reported as gross (the nominal body before any operation) and net (after the operations: the through holes of a plate today); with no operation they are equal, and `removed_volume_mm3` is measured on the kernel meshes so the arc error cancels. The old `volume` and `mass` keys are the net values. Stored records written by earlier slices read as stale, not corrupt ([design note](design/p-mass-section-area.md)).

A09 (experimental slice) adds approved element marks (`model::command::{AssignMark, ApproveMark, MarkStatus}`, `engine::marks`, `Group::Mark` in the footprint, `Change::Mark` in the layer engine, `beamcraft_cli::marks` and the CLI commands `number-assign`, `number-approve` and `marks`): a mark is a human-authored string of 1 to 64 bytes on a plate, beam, column, assembly or weld, unique in the model, stored as `bim:mark` and `bim:markStatus` (`assigned` or `approved`) on the element prim; an approval freezes the value, `--revoke` withdraws it, and a thickness change of an approved element commits the withdrawal in the same batch — the value is kept and the status falls back to `assigned` ([design note](design/a09-marks.md)). The A08 preview proposes series and stores nothing; marks store and approve what a person assigns. No output consumes marks yet.

The layer engine's default stage ceilings are the model profile (`Budgets::MODEL`: 32 MiB per layer, 64 MiB per stage, 32,768 scalars per array, sized from the host blob and verification limits and the engine view caps); the U01 first-slice profile (`Budgets::FIRST_SLICE`) remains for small untrusted inputs such as R01 reference sources ([design note](design/layer-budgets-model-profile.md)).

[Module boundaries (F08)](contracts/module-boundaries-v0.1.md) give the exact crate and module path
of every contract area, the allowed dependency edges and which crates are planned. A test and CI
enforce them.

## Planned boundaries

- Extend the existing `geom` and `model` crates. Do not create duplicate geometry or domain cores.
- Keep domain mathematics, BIM identities, commands and invariants independent of UI, filesystem,
  scheduling runtimes and the C++ ABI. Only `beamcraft-geom`, `beamcraft-model` and
  `beamcraft-kernel` stay WASM-compatible as a layering discipline.
- The pure `protocol` crate holds the F05 wire rules (framing, limits, transport codes). Process
  supervision, pipes and deadlines stay in the native host.
- Place filesystem access, process supervision, storage and coordination in a native engine host.
  The host depends on domain crates, not the reverse.
- Use the thin BeamCraft wrapper around the pinned Rust layer engine for primary USD operations;
  engineering rules and geometry computations remain in the independent Rust core. In-process
  module versus separate Rust process remains open under F11/U01. F05 continues to govern the
  SDK fallback and any reviewed separate-process route; in-process semantics need explicit review.
  [Wrapper requirements](design/layer-engine-wrapper-requirements-v0.1.md) cover foreign writes,
  numeric/input validation, deterministic saves and deltas. F08 dependency/placement review and
  package/file-level licence admission precede integration; this record adds no dependency.
- Use the sibling PdfCraft UI stack only under the
  [V01.0 audit](audits/ui-stack-audit-v0.1.md). That audit admits eframe/egui 0.36.2, wgpu 30.0.1,
  the PdfCraft-patched winit 0.30.13, egui-wgpu 0.36.2 and vello_common 0.0.8, plus craft-ui and
  craft-quit from storytold/craft-libs at rev `83bf4cb25c01ed46e7645c4172974239868aaf06`, all with
  conditions. `default_fonts` and craft-print are rejected. No UI crate, vendored patch or PdfCraft
  asset is added by that audit. Keep frontend crates above the engine, in F08's planned app; F08's
  audited domain allowlist stays empty until an explicit reviewed table amendment.
- Add steel, export and drawing modules when their contracts require them. F08 fixed the concrete
  module paths after F04–F07 and creates no empty crates: `ui` is a planned path in the boundary
  document, created by its own task with its first real code. `engine` exists from V02 with the
  view snapshot and query slice only.

Prefer well-maintained permissively licensed Rust crates for math, geometry primitives and robust
predicates, preferably pure Rust; write own algorithms only where no suitable crate exists. Each
dependency needs the AGENTS.md section 5 audit, deny.toml checks and any F08 allowlist amendment
before integration. Copyleft or weak-copyleft third-party crates, including MPL-2.0, need an
explicit owner exception for each package. No code is copied from other projects; audited dependency
use is distinct from copying implementation text. The restricted commercial-material and clean-room
rules remain unchanged. The layer-engine choice is recorded in F11; this paragraph admits no
additional math dependency. Start with bounded primitives and original fixtures, expanding through
reviewed scopes.

## Project state and revisions

One `project.usda` is the project entry point. USD stores authored parameters, placements,
operations and relationships. Derived USD layers contain computed geometry and quantities. Memory
snapshots, indexes, caches and journals do not replace the authoritative model format. JSON control
messages describe operations, not a second persisted native model.

A published root references complete immutable generations of authored and derived layers.
Publishing replaces the root atomically after verifying assets and the expected base revision.
Recovery must expose either the previous or the new complete revision. U05 stores the blobs and
U06 (`beamcraft_host::revision`) commits a revision as a canonical manifest blob plus an
append-only head log whose records are installed by a non-replacing hard link, which is both the
atomic root switch and the compare-and-swap on the expected base; the reader verifies the
manifest and every layer blob and refuses partial state. U07 (`beamcraft_host::ledger`, see
[its record](design/u07-journal-recovery.md)) adds a durable append-only outcome ledger that
records an intent before the root switch and the outcome after it, recovers an unfinished commit
from the root, and answers QueryOutcome; a sequence of
uncoordinated file replacements is insufficient.

Every placed element has a full UUIDv7. Prim names retain all 32 hexadecimal digits of the UUID;
renaming a user label does not change identity. Immutable resources use full SHA-256 digests and
verified bytes. Relationships are validated for duplicate identities and dangling targets.

U05 stores exact bytes at `<authorized-root>/blobs/<first-two-hex>/<all-64-lowercase-hex>`.
The root must be an existing absolute directory. Resource operations accept a `ContentDigest`,
never an asset path or URI. Publication flushes and syncs a temporary file within the store,
then installs it with a non-replacing hard link; unsupported filesystems fail without a rename
fallback. Concurrent identical publishers return one `Created` and otherwise `AlreadyPresent`.
Existing bytes are compared in full; differences return `IntegrityConflict` without overwriting.
Retrieval, presence and verification check the complete SHA-256. A digest is not a mathematical
proof that collisions are impossible. The 32 MiB F05 sections 3/4 bundle budget is the upper
bound for each blob and in-memory read; a store may choose a lower cap.

Every existing path component is checked for symbolic links/reparse points, and non-regular
blob entries and canonical containment violations are rejected. The host must protect the root
namespace from hostile concurrent local writers: portable safe std checks cannot prevent a
directory replacement between validation and opening. Windows additionally opens blob reparse
points without following them. These checks are not an OS sandbox. U05 promises neither
directory-entry crash durability nor crash-atomic multi-file publication; crashes may leave
temporary files. U06 adds the revision commit and reader (see
[its record](design/u06-revision-commit.md)) and likewise promises no directory-entry crash
durability. U07's recovery removes only abandoned temporary files (`.tmp-*` record temps and
`.publish-*` blob temps) and *reports* unreferenced blobs; it deletes no blob. Garbage
collection is deferred until retained revisions and drafts are accounted for. The asset CLI
belongs to U04; `revision-commit` and `revision-open` are the U06 commands, `revision-recover`
and `revision-outcome` the U07 ones.

Copies receive independent identities and share immutable geometry until a shape edit.
Personal workspaces preserve unpublished authored edits, derived results and journals across
restart. They are valuable draft state and are excluded from the published root, not deleted
with disposable caches.

## References, quantities and collaboration

Foreign USD references must remain read-only (R01 implements the attachment wrapper for a single
self-contained layer). Edits and overrides of foreign prims are forbidden,
including overrides authored in our layers. Our attachment wrapper may have a placement;
measurements and annotations are separate owned results. Reference updates are explicit and preserve
the original source bytes. IFC import and native conversion are outside the initial scope; IFC is a
later export adapter for an explicitly supported subset.

Shape, mass, appearance and world measurements have separate dependency signatures. Renaming or
moving the camera need not rebuild shape; changing density invalidates mass. Measurements carry
freshness and support diagnostics. Unknown density, stale results and unsupported geometry must not
be presented as current exact values.

A single backend owner behind the BeamCraft wrapper owns the live USD stage; its execution
topology remains open under F11. Independent calculations may run over immutable inputs;
stage mutation and publication are coordinated by the host. Command footprints include reads, writes
and relationships so rebase can detect semantic conflicts. Conflicts preserve local edits rather than
silently accepting a server value.

For a separate-process backend, worker IPC follows the reviewed F05 profile: versioned, bounded,
length-prefixed control frames with separate binary buffers over private pipes. An in-process
route still needs F11's reviewed semantic mapping of limits, cancellation and revision checks. The
[F05 worker protocol draft](contracts/worker-protocol-v0.1.md) specifies this private IPC contract
and its proposed acceptance cases. The [F06 geometry and viewport contract
draft](contracts/geometry-contract-v0.1.md) specifies the kernel input, mesh, key and query contract
in the same way. The [F10 domain vocabulary and module contract
draft](contracts/domain-vocabulary-v0.1.md) specifies the shared element kinds, relationships,
validation rules and discipline-module boundary in the same way; only the steel module is in scope
and nothing in it is implemented. The [E01 IFC export contract
draft](contracts/ifc-export-v0.1.md) specifies, in the same way, the IFC subset, the deterministic
GlobalId derived from the UUIDv7, the representation, the losses and the writer design of the
export adapter; import is out of its scope and nothing in it is implemented. Coordination and control servers require explicit opt-in, loopback
binding and token authentication. Resource paths do not authorize outbound access; local resolution
and containment are validated.

## Part representations and view filters (planned requirement)

Owner wish of 2026-10-16, to be planned with A03/A04 (cuts, holes), U02 (USD authoring) and V02
(view snapshot and query). It changes no frozen contract: F04/F06 0.1.0 have no operations.

- A part with operations (holes, cuts) is published with two representations in the same revision:
  a **full** one with the operations applied and a **simplified** one without them. A part with no
  operations has one mesh referenced by both. Identical content is stored once (SHA-256).
- The full representation is the default geometry of the revision, so an ordinary USD reader sees
  the complete part without BeamCraft. The simplified one is a lightweight stand-in (USD `proxy`
  purpose is the candidate mechanism). A payload for the heavy mesh is a candidate for fast opening
  of very large models. The USD encoding is decided by the U-tasks.
- Each representation has its own shape key and freshness; an operation change makes both stale. The
  representation is an enumeration, not a flag, so coarser levels can be added later.
- The simplified representation is flagged not exact. Volume, mass, bills of materials and snapping
  use only the full one, and the two are never summed (see "References, quantities and
  collaboration" and F06 section 3).
- View settings choose what to load, not what exists. A per-view filter can hide whole element
  kinds that are separate objects (welds, bolts, references) and can switch parts between the
  simplified and the full representation. The filter belongs to user or workspace settings, not to
  the shared revision, and changes no authored or derived data.
- V02 must carry both parameters: an element-kind filter and a representation selector. F06 gains a
  `representation` field in the version that introduces operations.
- **V02 status (2026-10-16).** `beamcraft_engine::presentation` carries both parameters
  (`ViewSettings`: `KindFilter` and the `Representation` enumeration, `full` and `simplified`) as a
  read-time projection of a pinned snapshot; they never enter the snapshot token. A part without
  operations references one mesh from both representations. The kernel has no operations yet, so a
  distinct simplified mesh can only be a caller-supplied stand-in (flagged not exact); volume, mass,
  bills of materials and snapping read the full representation only. Welds and bolts are not
  registered element kinds yet (F10 vocabulary 0.1), so the filter works on the registered kinds.
  Measuring the gain on the 10 000-part scene remains a V01 task.
- The gain is measured, not assumed: V01 reports load time, memory and frame time on the 10 000-part
  scene with each setting.

## Rules

- Dependencies point one way, from higher-level crates down to `model` and `geom`. No back-edges.
  The allowed edges are the module table of the boundary document; `cargo test --workspace` checks
  the manifests against it. The three pure crates (`kind = domain`) have no UI, OS, filesystem,
  process or async-runtime dependency; CI derives their WASM check list from the same table.
  Protocol, USD layer engine, host, IO, CLI, UI and all later crates are desktop-native and
  have no WASM requirement (owner decision of 2026-10-16, WS amendment to F08).
- Units: millimetres, radians internally, right-handed, Z up. Convert only at the IO/UI edge.
- Every feature is reachable headlessly: engine API → CLI command → end-to-end test.
- Input is untrusted; see `AGENTS.md` §4.
- Project-owned code uses MPL-2.0; each new workspace package needs its own `deny.toml` exception.
  Third-party dependency and asset policies remain unchanged. F03 audits the actual OpenUSD
  distribution, bundled libraries and licence obligations before integration.

## Decision provenance

This original design summary follows the owner's local `architecture-proposal.md` dated 2026-10-14
(§1, §3–7, §9–17) and `implementation-plan.md` revision 1.1 (§1–4, §5–7, §14). Detailed Russian
plans and observation materials remain outside the repository. Those local plans describe intended
acceptance, not completed features. See [the roadmap](../ROADMAP.md) for gates and
[development](development.md) for the verified F01 baseline and remaining checks.

The [F04 model and command contract](contracts/model-and-commands-v0.1.md) is a review draft of
these logical boundaries. Its examples are proposed acceptance descriptions, not implemented APIs or
saved models. F09 admission precedes freezing it for kernel implementers.

### Owner decisions of 2026-10-15

1. Math/dependencies: prefer suitable permissive Rust crates, retaining per-package audit and
   restricted-material rules — recorded; no dependency added.
2. UI: adopt the PdfCraft stack above — recorded; craft-libs licence, git revision and vendor patch
   audit remain open.
3. Plate: contour, first-edge X, normal/thickness Z and Y=Z x X — recorded; winding, limits and
   planarity assumptions noted in F04/F06; nonrectangle evaluation waits for A02.
4. Member frame: P-2 confirmed unchanged, with Y derived through a cross product — recorded.
5. Roll: optional explicit rotation about member local +Z after the default frame — recorded;
   missing prior angle cap addressed by an explicit assumption in F04.
6. Constants: owner-confirmed first-slice draft values, relative 1e-6 and length floor 1e-6 m=0.001
   mm — assumption noted; fixture validation open;
   ifc-lite comparison recorded in the tolerance report.

F04 and F06 were frozen at 0.1.0 by the owner on 2026-10-16. See the [tolerance
comparison](design/tolerance-comparison-v0.1.md). Nothing is implemented by this record, and
existing admission requirements remain in force.

### Owner decisions of 2026-10-16

1. F04 and F06 are frozen at 0.1.0; P-1a to P-1d, P-2, P-3a and P-3b are confirmed. P-3c (roll jump
   at the parallel threshold) is accepted as erratum E-2 until a black-box observation of the
   reference behaviour for a column whose axis moves is recorded.
2. A future `section_anchor` (nine positions plus offsets) is reserved in F04 section 3.
3. Part representations and per-view filters are a planned requirement (section above).
4. The core is extensible by discipline modules (F10); new element kinds, commands and rules are
   registered by a module, not hard-coded. Importing foreign IFC (E04) remains an open owner decision.
