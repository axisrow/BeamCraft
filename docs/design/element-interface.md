# Element interface: how the platform stops naming steel kinds

Contributor-original BeamCraft documentation, MPL-2.0. Design note for slice 1 of
[#795](https://github.com/Apibim-com/BeamCraft/issues/795), read against `master` at `94203bbd`
(2026-11-01). Sections 1 to 6 are the plan as first written; section 7 corrects it after the
first look at the code, and section 8 records what was built and what was left on purpose. It
continues [`platform-and-steel.md`](platform-and-steel.md) and the owner's comment of 2026-10-31 on
the issue.

Fixed by the issue and not reopened here: the stored `steel:*` attributes, the schema descriptor
and every store written today stay byte-identical; refusal codes and the output of existing
commands do not change.

## 1. The problem in one paragraph

The steel code is one crate, `steel`, but `layer-engine` and the CLI still branch on the kind of
element, so they depend on it and the crate cannot become kind `discipline` (nothing but an app
may depend on a discipline). Reading the code shows that these are **two different branch
points**, with two different fixes: the operations on a typed element (copy, edit, mirror,
array, evaluate) and the persistence of an element in the USD layers.

## 2. What the code does today

### 2.1 Typed operations: symmetric pairs

Every operation exists once per kind, with the same shape: the source element plus a command in,
the new element out.

| Operation | Plate | Member | Source |
|---|---|---|---|
| copy | `copy_plate(&AuthoredPlate, &CopyElement) -> AuthoredPlate` | `copy_member(&AuthoredMember, &CopyElement) -> AuthoredMember` | `steel/engine/copy.rs:148`, `:221` |
| copy as instance (P11) | `copy_plate_instance` | `copy_member_instance` | `steel/engine/instance.rs:131`, `:141` |
| edit | `patch_plate(&AuthoredPlate, &ElementPatch)` | `patch_member(&AuthoredMember, &ElementPatch)` | `steel/engine/update.rs:98`, `:160` |
| mirror, local part | `mirror_plate_local(&mut AuthoredPlate)` | `mirror_member_local(&mut AuthoredMember)` | `steel/engine/transform.rs:312`, `:337` |
| array | `array_plate_copies` | `array_member_copies` | `steel/engine/transform.rs:440`, `:466` |
| evaluate body | `BodyCache::plate`, `evaluate_committed*` -> `PlateRecord` | `BodyCache::member`, `evaluate_committed_members*` -> `MemberRecord` | `steel/engine/committed.rs:108`, `:127`, `:431-541` |
| quantity signatures | `plate_*_signature`, `plate_record` | `member_*_signature`, `member_record` | `steel/engine/quantity.rs:162-462` |

Placement-only work is already generic: `rotate_placement`, `mirror_placement`,
`array_placements` (and its helper `stepped_placement`, `:381`) take a `Placement`, not an element
(`transform.rs:187`, `:246`, `:427`). The per-kind part is only what the symmetry does to the
local shape records (cuts, thickness side, section placement, path).

The platform names the pairs in exactly these places, so the interface has few callers:

- `crates/commands/src/copy.rs:66` (`CopiedElement::{Plate, Member}`), `:98`
  (`creation_changes`), `:267-279`;
- `crates/commands/src/array.rs:160`, `:171`;
- `crates/commands/src/edit.rs:166-169` (`Target::{Plate, Member}`), `:191`, `:209`, `:324`,
  `:335`;
- `crates/commands/src/plate.rs:1471-1516` (the `BodyCache` and the two evaluations);
- the decoders (`Decoded`) and `registry.rs` for the command inventory.

`ElementPatch` (`steel/model/command.rs:3648`) carries one set of fields for both kinds. The
generic ones are `name`, `class`, `material`, `level`, `translation_mm`, `orientation_xyzw`,
`rotate`, `mirror`, `move_mm`. The kind-specific ones are `corners`, `contour_points_mm`,
`thickness_mm`, `thickness_side` (plate) and `axis_points_mm`, `profile` (member). Each pair
function already refuses the other kind's half with `UNSUPPORTED_PROPERTY`
(`update.rs:98-175`), and `effect(&ElementPatch)` (`update.rs:71`) derives which groups change.
The patch enters the content fingerprint `BeamCraft/P04/UpdateElement/v2` (`command.rs:3913`).

### 2.2 Persistence: reading is neutral, writing is not

- **Reading is already neutral.** `layer-engine/src/authored/validate.rs` checks `Prim`s by
  attribute lookup (`plate_holes(&Prim)`, `plate_hole_records`, `member_hole_records`,
  `weld_properties`, `bolt_properties`, `bolt_group_properties`). `rebase.rs:49`
  (`property_group`) already maps attribute names to the semantic write groups as strings.
- **Writing is a closed enum.** `Shape` (`authored.rs:285`: `Plate`, `Member`, `LoftedPlate`,
  `Instance`, `Weld`, `Bolt`, `BoltGroup` beside `Assembly`, `Grid`, `Construction`,
  `Annotation`, `Level`) and `Change` (`authored.rs:630`: `Contour`, `Thickness`,
  `ThicknessSide`, `Corners`, `Bends`, `Cuts`, `Axis`, `Roll`, `Profile`, `HoleRecords`,
  `MemberHoles`, `BoltGroup` beside `Label`, `Mesh`, `Delete`, `Level`, `Mark`, `Attr*`) carry
  typed steel fields. The same variants are matched in `edit.rs` (`create` `:634`, kind token
  `:658-668`, steel arms `:699-1039`, `capture` `:2039`, `author_corners` `:2517`,
  `author_bends` `:2581`), `validate.rs` (create arms `:2712-2978`, finiteness `:3263-3357`),
  `index.rs:421` (`"plate" | "beam" | "column"`) and `rebase.rs`.
- **The schema descriptor** (`SCHEMA_DESCRIPTOR`, `authored.rs:40`) is one byte string in which
  steel text and platform text alternate: the plate, cut, hole and member blocks are steel;
  the assembly, grid, level, construction and annotation blocks are platform; yet platform lines
  also name steel tokens (`kinds=plate|beam|column` in the part line, `bim:instanceOf`,
  `bim:level` and the numbering line list plate, beam and column).

### 2.3 The kind vocabulary

`ElementKind` (`engine/src/element.rs:26`) is the **closed F10 registry**: sixteen kinds,
including the reserved `Wall`, `Slab`, `Pipe`, ... F10 is frozen at 0.1.0
([`f10-domain-vocabulary-v0.1.md`](../contracts/f10-domain-vocabulary-v0.1.md) section 2.1: "the
registry is closed for a vocabulary version"). The enum is a vocabulary, not behaviour, and
stays. What the platform must not do is **branch on Plate, Beam or Column to decide behaviour**;
today that happens in `query/pick.rs:133` (the `Parts` switch), `:1503` (member axes must be a
beam or column), `query/snapshot.rs:235` (the `extrusion:` role length), and in
`model/src/prim.rs:43` (`from_kind`: plate, beam, column are the `Part` category) and `:58`
(the `/World/Steel/...` prefixes). The rest of the matches in `bom.rs` and `pick.rs` are tests.

## 3. Decisions

**D1. Two seams, not one big trait.** The issue asks to compare the full interface with the
narrow form from the owner's comment (steel converts its typed elements to a neutral prim
description that `layer-engine` writes and reads). The code says to use both, each where it fits:
the narrow form for persistence, a small trait for the typed operations. A single trait that did
both would force `layer-engine` to know the typed element, which is the dependency to remove.

**D2. Seam A (persistence): the neutral description lives in `model`.** `model` is below both
`layer-engine` and `steel`, so no back-edge appears. A discipline registers, per element
category:

- the attribute-name -> write-group table (today the `steel:*` arms of `rebase.rs:49-135`);
- the author function that turns a change into attribute writes (today `author_corners`,
  `author_bends`, `author_cuts`, the arms of `create` and `Change::*` in `edit.rs`);
- the validators over a `Prim` (today the steel arms of `validate.rs`);
- the kind/category rows (`prim.rs`, `index.rs:421`).

`Shape` and `Change` keep their platform variants and gain one neutral variant each: a category,
a label, a placement and a list of named attribute values (create), and a list of named attribute
replacements with their group (edit). The author function of the discipline produces that list
from its typed element, so the order of attribute writes is the order of today's code.

**D3. Seam B (typed operations): a trait object per element kind, in `engine`.**

```rust
// engine::element_ops (pseudo-signatures; the names are settled in slice 2)
pub trait ElementKindOps: Send + Sync {
    fn kind_token(&self) -> &'static str;                 // "plate", "beam", "column"
    fn category(&self) -> ElementCategory;                // the prim category
    fn copy(&self, e: &dyn Element, c: &CopyElement, as_instance: bool)
        -> Result<Box<dyn Element>, CommandError>;
    fn apply(&self, e: &dyn Element, p: &ElementPatch)
        -> Result<Box<dyn Element>, CommandError>;
    fn mirror_local(&self, e: &mut dyn Element) -> Result<(), CommandError>;
    fn array(&self, e: &dyn Element, c: &ArrayElements)
        -> Result<Vec<Box<dyn Element>>, CommandError>;
    fn evaluate(&self, e: &dyn Element, ctx: &EvalContext)
        -> Result<EvaluatedElement, CommandError>;         // body + quantities
    fn footprint(&self, e: &dyn Element) -> Footprint;
    fn to_description(&self, e: &dyn Element) -> ElementDescription;   // seam A
    fn from_prim(&self, p: &Prim, ctx: &ReadContext) -> Result<Box<dyn Element>, CommandError>;
}
```

`Element` is the small common view the platform needs (id, name, class, material, placement,
level); the concrete `AuthoredPlate` and `AuthoredMember` implement it and are downcast only
inside `steel`. The interface is the **intersection of the pairs in section 2.1**, nothing more:
copy, edit, mirror, array, evaluate, footprint and the two conversions. What exists for one kind
only (contour edits, corners, bends, lofted plates, welds, bolts, fabrication, numbering
geometry) is not in the interface: it stays `steel::engine` and is reached from steel's own
commands.

**D4. `ElementPatch` splits in two.** The platform keeps the generic fields; the kind-specific
fields become a discipline-owned payload that the discipline's own argument decoder builds and
its `apply` reads. Requirements:

- the refusals stay: a payload that does not belong to the kind is `UNSUPPORTED_PROPERTY` with
  the same field name and message as today (`update.rs:98-175`);
- the content fingerprint `BeamCraft/P04/UpdateElement/v2` keeps its byte layout: the platform
  writes the generic fields where it does today, and the payload writes its fields through a
  `fingerprint_into` method in today's order, so a replayed operation compares equal;
- `effect()` keeps its meaning; its `shape` flag is asked of the payload.

**D5. Registration is explicit.** `steel::register(&mut Disciplines)` is called by the apps
(`beamcraft-cli`, `beamcraft-ui`) and the registry is passed down to `engine`, `layer-engine`
and the CLI pipeline. A process-wide `OnceLock` is rejected: it makes tests depend on order,
hides the dependency from the F08 table and cannot be built twice in one test binary with
different sets.

**D6. The descriptor is composed from ordered segments.** Because steel and platform text
alternate (section 2.2), the descriptor becomes a list of segments, each owned by the platform
or by a discipline, and the registry concatenates them in the **present order**. For the steel
registry the bytes equal today's constant; a test pins the old literal byte for byte and the
digest next to it, in addition to the existing descriptor-pin tests. A registry without steel
would legitimately produce another descriptor, which is correct: another schema. Where a
platform line names steel tokens (`kinds=plate|beam|column`), the platform segment is a
template filled from the registered kind tokens in the registration order, again byte-equal for
steel.

**D7. `ElementKind` stays; behaviour moves to properties.** Slice 4 does not make the F10
registry data. It adds kind **properties** answered by the registered discipline (`is_part`,
`is_linear_member`, `extrusion_role_applies`) and replaces the three behaviour matches of
section 2.3 with them. `prim.rs` `from_kind` and `prefix` take the part category rows from the
registry.

## 4. Slice order

The crate `steel` already exists, so the issue's slice 7 shrinks to flipping its kind. Each code
slice is its own PR, the whole suite green, the `tests/fixtures/f07` read-back and the descriptor
pin tests unchanged, command output unchanged.

| # | Slice | Closed by |
|---|---|---|
| 1 | This note | review |
| 2 | Seam B: `ElementKindOps` in `engine`, implemented in `steel` for Plate and Member; the CLI `copy.rs`, `array.rs`, `edit.rs` and the evaluation in `plate.rs` call it; `CopiedElement` and `Target` become one type | no `*_plate`/`*_member` pair is called outside `steel`: `rg "(copy|patch|array)_(plate|member)"` finds only `steel` |
| 3 | `ElementPatch` split (D4) | the fingerprint test vectors of `UpdateElement/v2` unchanged; refusal messages unchanged |
| 4 | Kind properties (D7) and the category rows from the registry | `rg "ElementKind::(Plate\|Beam\|Column)"` finds no behaviour match in `engine/src` |
| 5 | Seam A in `layer-engine`: neutral variants, registered author functions, validators and group tables; the descriptor segments (D6) | `rg "steel:" crates/layer-engine/src` finds nothing; descriptor literal and digest pinned equal |
| 6 | CLI: platform commands stay, steel commands and registry entries move to `steel` | `registry.rs` lists platform commands only |
| 7 | F08 table: `steel` becomes `discipline`; the `layer-engine -> steel` and the CLI-platform edges go | `beamcraft-boundaries` passes; `rg "crate::steel\|beamcraft_(model\|kernel)::steel"` finds nothing outside the steel subtrees |

Slices 2 and 3 can be one PR if the patch split is small once the trait exists; the grep in the
last column is the judge, not the PR count. Slice 5 is the largest (about two thirds of the
6,200 lines of `validate.rs` and `edit.rs`) and may split by element family: plate and cut records,
then member and holes, then welds and bolts.

## 5. Risks

- **Hot paths.** `BodyCache` and `evaluate_committed*` run per element on every inspection. A
  trait object per call is not a measurable cost against a kernel evaluation, but the dispatch
  must be per kind once per batch (resolve the `ElementKindOps` once, evaluate the slice), not
  per element. Slice 2 measures `query_bench` before and after.
- **Down-casting.** `dyn Element` hides the concrete type; a wrong down-cast must be an internal
  error with a stable code, never a panic (`AGENTS.md` section 4: no `unwrap`, `expect` or
  indexing in non-test code).
- **Fingerprints and journals.** Command fingerprints are stored in the journal. A changed
  layout would make a replayed operation `OP_ID_REUSE`. D4 keeps the layout; slice 3 adds a test
  that fixes the bytes of one patch per field before the change.
- **Attribute write order.** The author function must write attributes in today's order or the
  `authored.usda` bytes change. The check is the existing byte-level store comparisons plus a
  golden comparison of the `f07` fixtures.
- **The descriptor.** Segmentation is a pure refactor of one constant; a mistake changes the
  pinned identity and every store written today is refused. D6's byte-for-byte test goes in
  with the first segment, before any other change in slice 5.
- **Test fixtures.** Platform tests that use a plate body as a neutral body (the box primitive
  that stayed in `kernel`, see `platform-and-steel.md`) keep it; they must not need the registry.
- **A second discipline.** The interface is cut from the plate and member pairs only. When a
  second discipline lands, expect to widen it; that is a platform change by design, not a break.

## 6. Out of scope

- A new element kind or a command: no behaviour, vocabulary or stored format changes.
- Making the F10 registry data-driven: it is a frozen contract; a change needs a new vocabulary
  version.
- The command registry and the long-lived session (`architecture.md`, "The window"): they use
  this seam, they are not part of it.
- Moving `layer-engine` below `steel`: the owner decision of 2026-10-31 keeps it above until
  slice 5 is done, then slice 7 removes the edge.

## 7. Update 2026-11-01: what the first look at the code changes

Reading the writers before starting slice 2 corrects three points above. The goal (no platform
crate depends on `steel`) and the byte-identity constraint stand.

1. **Seam A is the critical path, and the neutral description (D2) is the expensive way to build
   it.** `create`, `capture` and the `Change` arms of `edit.rs` write straight into the USD data
   (`attr(data, &p, "steel:plate:thicknessMm", "double", ...)`, `edit.rs:634-1039`), through
   helpers only `layer-engine` can name. A neutral attribute list would mean rewriting every
   writer and proving the same bytes again. The cheaper and safer form is to **move the steel
   half of `layer-engine` into `steel` verbatim** (writers, validators, the group table, the
   tests) and let `steel` depend on `layer-engine`: a discipline above the platform, which is what
   the F08 kind means. `layer-engine` then defines the hooks (trait objects in `Shape` and
   `Change` for the write side) and keeps the helpers `pub` with a "Shared with the steel crate"
   doc line. D2 stays as the fallback if a second USD writer ever appears.
2. **The read side needs a registry, and a global one is the proportionate answer (D5 revised).**
   Validating a loaded layer, building the element index and grouping rebased attributes have no
   element object to carry the behaviour. There are 28 `Author::open`, 51 `Reader`
   constructions and 17 `element_index` calls outside `layer-engine`, so threading a registry
   through is the larger change. The apps install the discipline once at start
   (`steel::install()`); the install is idempotent, and the platform tests that need a discipline
   use a stub. The write side needs no registry because the trait object travels in the change.
3. **Slices 2 to 4 are not on the way to `discipline`.** `CopyElement`, `ArrayElements` and
   `ElementPatch` moved into `steel::model::command` with the rest of the 6,478-line command
   file (#791), so a platform trait over them needs the patch split (slice 3) first, and its only
   consumers are `copy.rs`, `array.rs` and `edit.rs` in the CLI, an app that may link any
   discipline. They remove branching from the CLI; they do not remove an edge. The order becomes:
   (a) hooks and the registry in `layer-engine`; (b) the steel half and its tests move; (c) the
   edges flip, F08 and `beamcraft-boundaries` follow; then (d) the typed-operations trait and the
   patch split as the follow-up that thins the CLI.

### The hook inventory of the move (step a)

About 110 match sites on the steel variants of `Shape` and `Change` sit in two files. They are not
a uniform list: each needs local state of its function (the budgets, the `reads`/`writes`
footprint sets, the `finite`, `cap`, `count_points` and `uuid7` closures, `input`, `prims`, the
open `sdf::Data` and the element path). The hook therefore takes a small context struct that
exposes exactly these, and the steel side implements one method per role:

| Role | Where it is today | Hook |
|---|---|---|
| kind token and prim category of a created element | `edit.rs:650-677`, `Shape::category` | `kind_token`, `category` |
| initial attributes of a created element | `edit.rs:694-1039` | `author_create` |
| attribute names a created element will hold (footprint of the create) | `edit.rs:2150-2262` (`capture`) | `capture_create` |
| byte budget of a create | `validate.rs:64-102` | `charge_create` |
| engineering bounds of a create | `validate.rs:2712-2998` | `validate_create` |
| finiteness | `validate.rs:3263-3413` | `check_finite` |
| which element kind a change may target | `validate.rs:2443-2454` | `targets_kind` |
| semantic groups a change reads and writes | `validate.rs:2537-2650` | `groups` |
| bounds, reads and writes of a change | `validate.rs:3085-3198` | `validate_change` |
| attribute edit of a change | `edit.rs:1584-1973` | `author_change` |
| attribute names of a change (footprint check) | `edit.rs:2293-2462` | `capture_change` |
| read side: layer checks over `Prim`s | `validate.rs` `plate_holes`, `member_hole_records`, `weld_properties`, `bolt_*`, `plate_corner_records`, `read_definition`, `read_fastener` | `validate_prims` (registry) |
| read side: kinds that are parts, copy plan | `index.rs:421`, `prim.rs:43` | `part_kinds` (registry) |
| read side: attribute name -> group | `rebase.rs:49-135` | `property_group` (registry) |

The first eleven rows ride on the trait objects in `Shape::Discipline` and `Change::Discipline`
and need no registry; the last three need the registry of step 2. Order inside step (a): the
connection family first (`Weld`, `Bolt`, `BoltGroup`; six sites each) to settle the context
struct, then the members, then the plates and cuts, with the suite green after each family.

Known remainder, deliberately left: `SCHEMA_DESCRIPTOR` stays a single constant in
`layer-engine` (D6's segments are only needed to compose it from several disciplines, and moving
the steel text out of a pinned constant is a separate, byte-risky change). It is the stored
format's identity, not a dependency on the `steel` crate.

## 8. Closing state (#795 done)

Steps (a) to (c) of section 7 landed as the hooks and registry (#803, #805) and the move of the
steel half into `steel::layer` with the flipped edge (contributed as one series, the fork's
#388 slices 1 to 10): `steel` is kind `discipline` at layer 6, depends on `layer-engine`, and no
platform crate depends on it. Both apps call `steel::layer::install()` at start. Step (d) and D7
landed in a reduced form, and the reductions are decisions, not gaps:

- **D3 became a steel type, not a platform trait.** `steel::engine::part::PartRecord` is the
  intersection of the pairs of section 2.1 (find, copy and P11 copy, array, patch with
  re-evaluation, mesh and quantity record, and the accessors the adapters need). The CLI's
  `CopiedElement`, `UpdatedElement` and `Target` are gone, and
  `rg "(copy|patch|array)_(plate|member)" apps` finds nothing. A platform trait needs inputs the
  platform owns; `CopyElement`, `ArrayElements` and `ElementPatch` are steel commands (#791) and
  the callers are apps, which may link any discipline.
- **D4 (the `ElementPatch` split) is not done.** It only serves a platform trait, and it is the
  one change here that could move the `BeamCraft/P04/UpdateElement/v2` fingerprint bytes. It
  becomes due when a second discipline shares the copy and edit commands.
- **D7 became vocabulary properties.** `engine` sits below the registry in `layer-engine`, so the
  properties are methods of the F10 enum, `ElementKind::is_part` and `is_linear_member`, and the
  three behaviour matches of section 2.3 call them. `model::prim` keeps its token table and the
  `/World/Steel/...` prefixes: they are the stored prim layout, not behaviour.
- **D6 is not done.** `SCHEMA_DESCRIPTOR` stays one constant (section 7, known remainder).

Acceptance of the issue, as run: `rg "crate::steel|beamcraft_(model|kernel|engine)::steel"`
finds nothing outside `crates/steel`; no platform `Cargo.toml` names `steel`;
`beamcraft-boundaries check` passes; `rg "ElementKind::(Plate|Beam|Column)" crates/engine/src`
finds only the vocabulary table, the F06 `BodyKind::of` token mapping, the box demo scene and
tests; the whole suite, the `f07` fixtures and the descriptor pins pass unchanged.
