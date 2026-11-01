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
| `apps/beamcraft-cli/src/plate.rs` | 2,816 | The platform pipeline (layer names, `PlateError`, `RevisionSummary`, open the base, check the footprint, claim the project) next to the plate decoders and `create_plate`. About 60 CLI files import `PlateError` and the pipeline from here. |
| `engine/src/element.rs` | 246 | `ElementKind` is one closed enum: Plate, Beam, Column next to Wall, Slab, Opening, Penetration, Pipe, Duct, Support and the generic kinds. Used by query, scene, filter, bom and synthetic. |
| `model/src/prim.rs`, `instance.rs` | 119, 77 | The category scheme is generic but hard-codes plate, beam, column and the `/World/Steel/` prefix; `PartKind` is Plate, Beam, Column. |
| `model/src/limits.rs`, `definition.rs` | 92, 455 | Weld, bolt, member and fastener caps next to generic limits; the generic definition container imports `section::SectionShape`. |
| `engine/src/{update,transform,quantity,footprint,transaction}.rs`, `query/committed.rs` | | Generic frameworks that import `AuthoredPlate`, `AuthoredMember`, `CornerTreatment` or `SectionShape`; `footprint.rs` has `for_weld`, `for_bolt`. |
| `engine/src/{numbering,numbering_approval}.rs` | 1,200; 913 | The series enum is plate, beam, column. |
| `kernel/src/lib.rs` | 41 | Exports `PlateResult` and `rectangular_plate`; `engine::{scene,synthetic,solid}` import `PlateResult`. |
| `apps/beamcraft-cli/src/registry.rs` | 2,487 | A generic command inventory that lists steel commands. |

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
each layer it extends (`steel::model`, `steel::kernel`, `steel::engine`). The platform crates do
not depend on it: `rg "steel"` finds nothing in their `src`. `layer-engine`, the CLI and the
window depend on it, so `layer-engine` sits above it in the F08 table (layer 5) and `session`
and the apps above that (6 and 7).

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

## What is left

The crate is kind `native` because `layer-engine` and the CLI still branch on the kind of
element and so depend on it; the rule that only an app or a tool may depend on a `discipline`
holds once they do not. What ties them to steel is:

- `layer-engine`: the `Shape` and `Change` enums and the `validate.rs` and `edit.rs` handlers
  list plate, member, weld and bolt next to grid, level and assembly;
- the CLI: `CopiedElement`, `Decoded` and the decoders in `plate.rs`, and the registry;
- `ElementPatch`, one structure that carries plate fields; `ElementKind`, `prim.rs` and
  `layer-engine/src/index.rs`, which name plate, beam and column.

The way out is one element interface in the platform, implemented in `steel` for `Plate` and
`Beam`/`Column`, so that the engine, the CLI and `layer-engine` call it and no longer branch on
a steel kind. The operations already exist as symmetric pairs (`copy_plate`/`copy_member`,
`patch_plate`/`patch_member`, `mirror_plate_local`/`mirror_member_local`,
`array_plate_copies`/`array_member_copies`), so the interface is taken from them, not guessed.
The stored `steel:*` attributes and the schema descriptor stay byte-identical. The work is
tracked in issue #795; [`element-interface.md`](element-interface.md) is its design note: the
two seams (typed operations and layer persistence), the decisions and the order of the code
slices.
