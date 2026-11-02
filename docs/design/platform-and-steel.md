# Platform and steel: where the code stands and how it separates

Contributor-original BeamCraft documentation, MPL-2.0. Audit of `master` at `ec04c6a3`
(2026-10-31). It is the basis of the command-layer direction in `docs/architecture.md`: one
discipline-agnostic **platform**, and **disciplines** (steel today, concrete or MEP later) that
register their commands on top of it. Nothing here is implemented yet; this note says what is
mixed, where, and in which order to pull it apart.

Classes: **P** platform (a second discipline could use it unchanged), **S** steel, **M** mixed.
Line counts are approximate and include tests.

## What the audit found

- `host` is entirely P: no steel name, no `steel:` string. Its only link to `engine` is
  `host/src/views.rs:8` (`engine::views`), which is itself P.
- The steel attribute namespace (`steel:*`, 620 occurrences) sits almost entirely in
  `layer-engine` (`authored/edit.rs`, `validate.rs`, `rebase.rs`, `instance.rs`,
  `cut_history.rs`, `authored.rs`) and in the CLI decoders (191 occurrences: `plate.rs`,
  `library.rs`, `definition.rs`, `bolt.rs`). `model`, `kernel` and `host` carry no `steel:`
  string; they carry steel only as Rust type names.
- The USD path prefix `/World/Steel/...` is hard-coded in `model/src/prim.rs:60-62` and in
  `layer-engine/src/authored.rs:44-45,122`.
- Mostly P already: `engine/query` (pick, snapshot, bundle, feature), `engine/drawing`,
  `engine/scene`, `engine/numbering` (the framework), `schedule`, `filter`, `views`, `work_plane`,
  `level`, `assembly`, `attribute`, `annotation`, `reference`, `rebase`, `grid*`; in `model`:
  `id`, `digest`, `error`, `changeset`, `attribute`, `grid*`, `construction`, `annotation`,
  `reference`, `drawing`, `sheet`; in `kernel`: the exact arithmetic, topology, trimesh, spatial
  index, profile admission, sweep, plane cut and seal.
- Mostly S: `engine/{command,member,fabricate,bolt,bend,weld,fastener,cut,corners,
  contour_edit,loft_plate,member_hole,plate_draft}`, `model/{section,fastener}`,
  `kernel/{section,member,fastener,plate_contour,ruled,build}` and `ops/member_hole`, and in the
  CLI `plate_cmd`, `bolt_cmd`, `weld_cmd`, `library_cmd`, `fastener_cmd` with `cut`, `bolt`,
  `weld`, `member`, `hole`, `loft`, `bend`, `library`.

## The mixed files

| File | Lines | What is mixed |
|---|---|---|
| `model/src/command.rs` | 6,478 | About three quarters steel (plate, member, hole, bend, weld, bolt, fabrication commands). The platform part is `CommandError`, `Placement`, `label_is_valid`, element update/delete/copy/array, assembly, level and mark commands. Other `model` files import `CommandError` and `label_is_valid` from here. |
| `layer-engine/src/authored/validate.rs` | 3,557 | About 55% steel (plate, member hole, weld, bolt, fastener, cut records). Generic: links, marks, levels, assembly links, budgets. |
| `layer-engine/src/authored/edit.rs` | 2,664 | Steel writes (corners, bends, plate, member, weld, bolt) next to generic ones (attribute definitions, erase, mesh, relationship, units). |
| `layer-engine/src/authored.rs` | 1,372 | The `Shape` and `Change` enums list Plate, Member, LoftedPlate, Weld, Bolt, BoltGroup next to Grid, Construction, Annotation, Level, Assembly. |
| `crates/commands/src/plate.rs` | 2,816 | The platform pipeline (layer names, `PlateError`, `RevisionSummary`, open the base, check the footprint, claim the project) next to the plate decoders and `create_plate`. About 60 CLI files import `PlateError` and the pipeline from here. |
| `engine/src/element.rs` | 246 | `ElementKind` is one closed enum: Plate, Beam, Column next to Wall, Slab, Opening, Penetration, Pipe, Duct, Support and the generic kinds. Used by query, scene, filter, bom and synthetic. |
| `model/src/prim.rs`, `instance.rs` | 119, 77 | The category scheme is generic but hard-codes plate, beam, column and the `/World/Steel/` prefix; `PartKind` is Plate, Beam, Column. |
| `model/src/limits.rs`, `definition.rs` | 92, 455 | Weld, bolt, member and fastener caps next to generic limits; the generic definition container imports `section::SectionShape`. |
| `engine/src/{update,transform,quantity,footprint,transaction}.rs`, `query/committed.rs` | | Generic frameworks that import `AuthoredPlate`, `AuthoredMember`, `CornerTreatment` or `SectionShape`; `footprint.rs` has `for_weld`, `for_bolt`. |
| `engine/src/{numbering,numbering_approval}.rs` | 1,200; 913 | The series enum is plate, beam, column. |
| `kernel/src/lib.rs` | 41 | Exports `PlateResult` and `rectangular_plate`; `engine::{scene,synthetic,solid}` import `PlateResult`. |
| `crates/commands/src/registry.rs` | 2,487 | A generic command inventory that lists steel commands. |

## Order of extraction

Each step is mechanical, compiles on its own and leaves the tests green.

1. **A neutral error and the pipeline out of `plate.rs`.** `PlateError` is the CLI-wide error
   type and the first blocker. Give it a neutral name and move the platform pipeline (layer
   names, `RevisionSummary`, `open_base`, `load_base`, `check_footprint`, `claim_project`,
   `host_uuid`, `integrity`) into the platform layer (`session`).
2. **Split `model/command.rs`.** `CommandError`, `Placement`, `label_is_valid` and the generic
   commands stay; the steel commands move to a steel module. Move `section.rs`, `fastener.rs`,
   the steel part of `definition.rs` and `limits.rs`, `PartKind`.
3. **Make `prim.rs` and `ElementKind` data-driven** (a category and kind table that a discipline
   fills), so `/World/Steel/` and the plate/beam/column matches stop being platform code.
4. **Decouple the engine frameworks** (`update`, `transform`, `quantity`, `footprint`,
   `committed`, `transaction`, `numbering`) from `AuthoredPlate` and `AuthoredMember` by a trait
   or table; then move the steel engine files out.
5. **Decouple the kernel** (`PlateResult`, `rectangular_plate`, the section and member code) and
   point `engine::{scene,synthetic,solid}` at the generic body and mesh types.
6. **Split the `layer-engine` enums** `Shape` and `Change` and give each discipline its own
   validate and edit handlers; the `steel:*` text goes with the steel discipline.
7. **Last, split the CLI**: `registry.rs` into one command table per discipline, and the steel
   `*_cmd.rs` and library files into the steel crate. The CLI keeps argument parsing and printing.

Steps 1 to 3 change no behaviour and no stored format. From step 6 on, the `steel:*` attribute
names and the schema descriptor must stay byte-identical, because stores written today have to
read back (the F04 and U03 rules, and the pinned schema identity).

## Layer rule this implies

A **discipline** crate may depend on the platform and on lower crates, never on another
discipline; the platform never depends on a discipline; both apps may depend on any. This is
checked mechanically by `apps/boundaries` through a table kind `discipline`, added by the first
discipline crate (F08 section 2).

## State after the split

The steel code of `model`, `kernel` and `engine` is one crate, `crates/steel`, with a module for
each layer it extends (`steel::model`, `steel::kernel`, `steel::engine`, `steel::layer`). The
platform crates do not depend on it: `rg "steel"` finds nothing in their `src`. Only the apps
depend on it: the dependency reversal of #795 landed, steel is kind `discipline` at layer 6 in
the F08 table, with `session` beside it (6) and the apps above (7).

## The reversal landed

What moved, in order:

- The steel write authors (`steel_write`) and the steel readers and definition writers
  (`steel_read`, `steel_defs`) moved out of the platform's `validate.rs` and `edit.rs` into
  their own modules behind the discipline seam.
- The steel attribute lists the platform matched by hand — the shared P11 shape inputs, the
  rebase write groups, the definition properties, the delete opinions of a whole-record change —
  moved into the steel discipline (`steel::layer::discipline`), and the platform callers ask the registry
  (`registered()`, `is_part_kind`) instead of matching steel kinds.
- The platform's only steel surface is the seam: the hook variants `Shape::Discipline` and
  `Change::Discipline` with the definition hook beside them, the re-exported surface a hook may
  use (`authored::seam`), and the facade of free constructors the apps build steel shapes and
  changes with, so no call site outside the facade names a steel variant.
- A discipline installs itself through the installable registry (`authored::register`,
  pointer-deduplicated, registration order preserved; both apps call `steel::layer::install()`
  at start, and the registry is empty without it), and every read-back question — part kinds, part and connection validation, write
  groups, delete attributes, definition read-back — is answered by the registered disciplines.

The stored format is untouched: the `steel:*` attributes, the prim paths and the schema
descriptor are byte-identical, so stores written before the reversal read back (the F04 and U03
rules, and the pinned schema identity).

Two corrections to the audit came from reading the code:

- The rectangular plate builder (`kernel::build`, `input`, `result`), its cross-check,
  `Body::from_plate`, `SolidMesh::from_box`, the `SceneMesh` conversion and
  `engine::synthetic::plate_scene` are one box primitive with its demo scenes. The platform
  tests (pick, bundle, feature, clipping), the window and the headless view commands build
  their scenes from it, and nothing in it is steel specific, so it stayed in the platform.
- The update, copy, array, transform, quantity and committed-plate files looked like generic
  frameworks and are per-kind code on `AuthoredPlate` and `AuthoredMember`, so they moved.

To let a crate outside `model`, `kernel` and `engine` build their types, some internals became
`pub` with a "Shared with the steel crate" doc line (a handful of kernel and scene fields and
helpers, `SolidMesh::from_parts`). The inherent impls that cannot live outside the type's crate
became free functions in `steel::engine` (`solid_mesh::from_section`, `from_loft`, `from_sweep`,
`numbering_geometry::member_geometry`).

## The element interface

The reversal took the dependency away; the element interface (#795, design note
[`element-interface.md`](element-interface.md)) took away the remaining branching on the kind:

- **Typed operations.** `steel::engine::part::PartRecord` holds a committed plate or member and
  chooses between the per-kind pairs (`copy_plate`/`copy_member`, the P11 instance pair,
  `array_plate_copies`/`array_member_copies`, `patch_plate`/`patch_member`, the two evaluations
  and quantity records). The CLI adapters `copy`, `array` and `update-element` call it and no
  longer carry their own plate/member enums; `steel::layer::part_shape` builds the create shape
  of a copied part. The pairs stay in `steel` as the implementation.
- **Kind behaviour.** The engine's queries ask `ElementKind::is_part` and `is_linear_member`
  instead of matching `Plate`, `Beam` or `Column`.

Deliberately left, each with its reason in `element-interface.md` section 8: no platform trait
over the typed operations and no `ElementPatch` split (the inputs are steel commands and the
callers are apps; the split risks the `UpdateElement/v2` fingerprint bytes); one
`SCHEMA_DESCRIPTOR` constant (the stored format's identity); the `/World/Steel/...` prim paths,
the F06 body tokens and the numbering series names (stored or contract format); the steel
decoders and `registry.rs` in the CLI (they move with the command layer, `architecture.md`, "The
window").
