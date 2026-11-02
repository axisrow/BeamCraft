# A14.4 generated holes

Date: 2026-10-31. Status: **experimental slice; the amendment's hole acceptance rides with the
slice (holes in every plate, one undo unit, refusals write nothing; member holes stay G14).**
Branch `feat/a14-4-generated-holes`, based on the master of 2026-10-31 (A14.3 merged).

Plan row: A14.4 / M (slice of A14, issue axisrow/BeamCraft#312, mirror of upstream #524),
contract `docs/contracts/f04-bolt-fastener-amendment-draft-v0.1.md` section 5 (with section 6's
regeneration rule). Depends on A14.3 (the placed group) and A04 (the plate hole records) and
nothing else.

## What is implemented

```text
beamcraft create-bolt-group ... [--hole-mode generate|none] [--hole-clearance MM]
beamcraft update-bolt-group ... (the same hole options; absent keeps the stored value)
beamcraft bolts ...        # groups carry hole_mode/hole_clearance_mm in their records
```

`create-bolt` (A05) is retired: the amendment replaces it, and the replacement now carries the
hole setting, so nothing is left the A05 token could do that the replacement cannot. Stored A05
annotation groups stay readable and upgradable (`bolts` lists them; `update-bolt-group`
upgrades them); no new A05 group can be authored.

| Area | What exists |
|---|---|
| `steel::model::command` | `BoltGroupInputs.hole_mode` (`Option<HoleMode>`, the definition's preference as the default: a bolt generates, a screw does not) and `BoltGroupInputs.hole_clearance_mm` (`Option<f64>`, the definition's clearance as the default) with admission and fingerprint input (`BoltGroupInputs/v2`), and `HoleRecord.owner` (`Option<ElementId>`). |
| `steel::engine::bolt` | `generate_holes`: with `generate`, one round through hole per crossing fastener on every connected plate, of diameter `nominal + clearance`, centred where the shank axis crosses the plate's mid-plane; a beam or a column is `UNSUPPORTED_GEOMETRY` (G14.1); `none` writes no part. `update_hole`/`remove_hole` refuse a group-owned record with `DEPENDENCY_CONFLICT` (`DV-REL-009`). |
| `beamcraft_layer_engine::authored` | `steel:bolt:holeMode` (required token) and `steel:bolt:holeClearanceMm` (present exactly when `generate`) on the group record; `steel:hole:generatedBy` (the sixth record attribute, equal length, a full UUIDv7 or the empty string per record); the instance projection carries the new attribute, so a drilled definition's instances read the records whole. |
| `beamcraft_commands::bolt_cmd` | `create_bolt_group`/`update_bolt_group` write the hole records of every affected plate in the same transaction: the record identities are derived from the operation id (SHA-256 over the pair, version and variant bits forced), so a retry authors the same identities; the plate is replayed before anything is written, so a hole that does not fit refuses the whole command. `update` regenerates the holes and turning `generate` off removes exactly the group's records. `delete-element` of a group removes its records from the plates in the same revision. |

## Decisions

- **Generated holes are ordinary A04 records** on the plate (F04 section 4): they replay in
  ordinal order with the hand-placed ones, enter the fabrication state and the equivalence of
  A08, and their `owner` field (`steel:hole:generatedBy`) is the only thing that separates
  them. Nothing else about a hole changes: no new element kind, no new numbering rule.
- **The cut runs along the plate normal**, like any A04 record; the centre is the axis's
  mid-plane crossing. For a shank perpendicular to the plate the record is exact; a tilted
  shank produces a normal-direction hole centred on the axis — the known ceiling of the A04
  through-hole vocabulary, shared with every hand-placed hole.
- **Record identities are pure functions of the operation**: `SHA-256(op_id || index)` into the
  random bits of a UUIDv7 (version and RFC variant forced). A retry of an accepted operation
  never reaches the author anyway, but a request replayed after a lost commit authors the same
  records as the first attempt did.
- **The plate replay is the refusal gate**: the records are appended in memory, the plate is
  re-evaluated through the ordinary kernel replay and the command is refused on the first
  unfit hole — outside the outline, into another record, past the 64-record or ordinal-999
  cap (`GEO_OP_NO_EFFECT` via the typed hole diagnostic).
- **The group footprint writes the parts**: `for_update(part, shape)` per affected plate
  unions into the group's footprint, so a concurrent edit of a drilled part conflicts as for
  any operation, as the amendment says.
- **Retire, do not migrate.** The descriptor gains the two `steel:bolt` attributes and the
  sixth hole attribute; pre-alpha stores are not migrated (the standing rule), and an A05
  annotation group stays readable exactly as before.

## Limits

- A member part refuses `generate` with `UNSUPPORTED_GEOMETRY` until the kernel slice G14.1
  (the sibling transverse-hole work) lands; `none` resolves its length over the same
  conservative member box as before, which the shipped synthetic screw lengths cannot span.
- The group's holes are not drawn by the viewport yet; the prototype placements (A14.2) and
  the records are the only output.
- One group is one straight row (O-B1); no environments (O-B2).

## Tests

`steel::model::command`: the hole defaults follow the definition, an explicit mode and
clearance enter the fingerprint, the refusals name `hole_clearance_mm`.
`steel::engine::bolt`: one hole per crossing fastener per plate at the mapped centre, the
group clearance over the definition's, the screw default and the explicit override, the member
refusal, a partly covered row cutting only the crossed holes. `steel::engine::command`: a
generated record refuses update and removal with `DEPENDENCY_CONFLICT`.
`beamcraft_layer_engine`: the record attribute set is six-or-none with the owner column
checked; the descriptor-pinned stores revalidate.
`apps/beamcraft-cli/tests/bolt_groups.rs` (end to end through the binary): the create writes
two records per plate owned by the group and rewrites the derived meshes; the edit and the
undo move the whole unit; `none` writes no part and stores no clearance; a screw defaults to
`none` and an explicit `generate` overrides it; a member part refuses; the repin regenerates;
`remove-hole`/`update-hole` on a generated record are `DEPENDENCY_CONFLICT` while a
hand-placed hole on the same plate works; turning `generate` off keeps exactly the manual
holes; deleting the group removes its records from both plates in one revision. The A05
end-to-end suite (`bolts.rs`) retired with the token; the listing and upgrade paths stay
covered by `bolt_groups.rs`.
