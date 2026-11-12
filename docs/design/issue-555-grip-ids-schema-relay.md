# Issue 555: bolt grip, rotated generated holes, retry identities, schema status, relayed refusals

Findings of a CLI-modelled joint (SHS column, sloped SHS brace, through gusset, knife plate,
2 x M16), each reproduced and fixed here with a regression test through the real binary
(`apps/beamcraft-cli/tests/issue_555.rs`, the window's `tests/screenshot.rs`). The bolt
decisions themselves are recorded where they live: the grip and the length rule in
[`a14-3-bolt-group-placement.md`](a14-3-bolt-group-placement.md), the generated holes in
[`a14-4-generated-holes.md`](a14-4-generated-holes.md). The member cut and slot gap (finding 7)
is a feature, tracked in the epic axisrow/BeamCraft#556.

## What changed

| Finding | Behaviour now |
|---|---|
| 1. Grip through foreign coaxial holes | The grip is read on the bearing ring (axis plus eight parallel rays between the clearance hole and the washer's outer radius), so a coaxial hole cut by anyone grips its slab; M16 through 2 x 20 mm is 70 before and after `add-hole`, for a new group and after a rename. A listed part no bearing ring finds material in is refused (`INVALID_ARGUMENT`, `parts`). The under-head washer now counts in `auto` (audit item 9). |
| 2. `generate` on rotated plates | Decided in the plate's own frame through the inverse placement; refused only for a shank axis more than 60 degrees off the plate normal. The window preview decides over each part's local mesh box and lifts the rings by the whole placement; a ruled-loft (bent or lofted) plate gets no preview holes (below). |
| 3. Plate box ignored `thickness_side` | One helper, `AuthoredPlate::local_z_band_mm` / `local_box_mm`, feeds the bolt admission box, the box grip fallback and the generated-hole slab. |
| 4. Verbatim `create-*` retry was `OP_ID_REUSE` | Without `--new-id`, the created identity is derived from `--op-id` (below). |
| 5. Unreadable store reported `revision_verified` | `open-revision` reports `schema` and refuses an incompatible project store with both digests; a project store whose layers cannot be read is refused with the reader's code (`INTEGRITY_ERROR`, ...), never `schema: null`; content refusals say what happened. |
| 6. `screenshot-view` lost the code | The window keeps the CLI's code, message and diagnostics (`Error::Rejected`) and prints them; `screenshot-view` re-issues them with the code's exit. |
| 8. Documentation drift | The CLI README, the bolt, plate, revision and screenshot help (the generated `COMMANDS.md` follows the registry) and the `beamcraft-model` skill follow the behaviour above. |

## Decisions

- **Derived default identities.** A `create-*` (and every command that creates objects without
  an explicit id: grid axes, copies, coordination sections/zones/connections, changesets) that
  is given `--op-id` but not the explicit id derives it: the op's 48-bit timestamp, then
  SHA-256 over a domain tag, the op id and a slot name (`new_id/0`, `u-axis_id/3`, ...), with
  the UUIDv7 version and variant bits forced (`command_context::default_id`). The caller of an
  exit 4 or 124 never saw the generated id, so this is the only way its verbatim retry can name
  the same object and replay. Without `--op-id` nothing can be retried and the id stays fresh;
  an explicit id always wins. This mirrors the generated hole records, whose ids are already
  pure functions of the operation.
- **Pre-alpha stores stay unmigrated; the answer is honest.** A descriptor change of any kind,
  additive included, still changes the mapping digest, and the standing owner rule is not to
  migrate pre-alpha stores. What changes is the reporting: `open-revision` reads the head's
  owned-layer pins (`stored_schema_pins`) and returns them with this build's digest and
  `compatible`; an incompatible project store is `UNSUPPORTED_VERSION` with the schema object
  as its diagnostic instead of `revision_verified`, and the content commands' refusal message
  names the cause and points at `open-revision`. A migration or a per-section compatibility
  policy needs an owner decision and stays out of this change.
- **`schema: null` means "not a project base", nothing else.** A head whose root layer is not
  `root.usda` (a raw revision of arbitrary blobs) is read by no content command and has no
  layer schema, so `open-revision` verifies it with `schema: null`. A project head whose
  layers cannot be parsed (a damaged layer committed through `commit-revision`) is refused
  with the reader's code, the same refusal the content commands give (axisrow/BeamCraft#557 review).
- **Each blob is verified once.** `open-revision` reads the head with
  `RevisionStore::open_with_layers`, which returns the bytes of the single verification pass,
  and the schema check (`store_schema::schema_status`) opens the stage from those bytes
  (`pipeline::base_with_layers`) instead of opening and hashing the store again.
- **The bolt preview leaves a ruled-loft plate's holes undecided.** The commit decides a
  plate's holes over the authored contour's box (`AuthoredPlate::local_box_mm`); for a flat
  plate that is the box of its shown mesh, for a bent plate it is the developed (unfolded)
  outline, which the folded mesh is not and the view snapshot does not carry. The preview
  recognises a loft body by its stable face ids (`ruled:`,
  `steel::kernel::ruled::is_loft_face_table`) and draws the row and bearings over it but no
  holes, rather than holes the commit never cuts (the bent-plate evaluation refuses holes in
  any case).
- **A refusal crosses process boundaries whole.** Anything that runs the CLI and reads its JSON
  keeps `code`, `message` and `diagnostics`; a catch-all code is used only when the child's
  reply carries no stable code of its own.

## Limits

- The bearing ring is sampled by eight rays: an opening shaped to slip between all of them
  next to the hole would be missed, which no clearance or slotted hole does.
- A store written before a descriptor change still needs to be rebuilt with the current build.
