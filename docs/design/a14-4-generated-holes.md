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
| `steel::engine::bolt` | `generate_holes`: with `generate`, one round through hole per crossing fastener on every connected plate, of diameter `nominal + clearance`, centred where the shank axis crosses the plate's mid-plane, decided in the plate's own frame (any placement rotation, any `thickness_side`; issue 555); a shank axis more than 60 degrees off the plate normal is `UNSUPPORTED_GEOMETRY`; `none` writes no part. `generate_member_holes`: the member side of the same decision (the F06 member-hole amendment section 3), one owned member-hole record per fastener whose shank crosses the member's material, its range bound to the stack the fastener crosses; a curved member or polybeam, a rolled or skewed member frame and a shank more than 60 degrees off its section-plane projection refuse with `UNSUPPORTED_GEOMETRY`. `update_hole`/`remove_hole` refuse a group-owned record (plate or member) with `DEPENDENCY_CONFLICT` (`DV-REL-009`). |
| `beamcraft_layer_engine::authored` | `steel:bolt:holeMode` (required token) and `steel:bolt:holeClearanceMm` (present exactly when `generate`) on the group record; `steel:hole:generatedBy` (a plate record attribute, equal length, a full UUIDv7 or the empty string per record) and `steel:mhole:generatedBy` (a member record attribute, same rule), so a drilled definition's instances and members read the records whole. |
| `beamcraft_commands::bolt` | `create_bolt_group`/`update_bolt_group` write the hole records of every affected plate and member in the same transaction: the record identities are derived from the operation id (SHA-256 over the pair, version and variant bits forced) over one shared serial, so a retry authors the same identities; the part is replayed before anything is written, so a hole that does not fit refuses the whole command. `update` regenerates the holes and turning `generate` off removes exactly the group's records. `delete-element` of a group removes its records from the plates and members in the same revision. |

## Decisions

- **Generated holes are ordinary A04 records** on the plate (F04 section 4): they replay in
  ordinal order with the hand-placed ones, enter the fabrication state and the equivalence of
  A08, and their `owner` field (`steel:hole:generatedBy`) is the only thing that separates
  them. Nothing else about a hole changes: no new element kind, no new numbering rule.
- **The decision runs in the plate's frame (issue 555 finding 2).** The bearing point and the
  shank direction are carried into element-local coordinates by the inverse of the plate's
  placement (the conjugate quaternion, normalized like every placement rotation); the crossing
  is decided against the plate-local box (contour extents and the material band of the
  `thickness_side`), and the centre is the local point of the axis on the band's mid-plane. So
  a gusset or a knife plate made with `--plane` or `--orientation-xyzw` takes generated holes
  like an unrotated plate, and the window's preview decides over the local box of each part's
  own mesh and lifts the rings by the whole placement.
- **The cut runs along the plate normal**, like any A04 record; the centre is the axis's
  mid-plane crossing. A shank axis more than 60 degrees off the plate normal
  (`MIN_HOLE_AXIS_COSINE`) is refused with `UNSUPPORTED_GEOMETRY`. For a shank perpendicular to the plate the record is exact; a tilted
  shank produces a normal-direction hole centred on the axis — the known ceiling of the A04
  through-hole vocabulary, shared with every hand-placed hole.
- **Member records are the A04.2 records, owned like the plate ones (2026-11-19).** With
  `generate`, a beam or column of the connected parts takes one `MemberHoleRecord` per
  fastener whose shank crosses its material, tagged with the group's id
  (`steel:mhole:generatedBy`, a record attribute of the member set — pre-alpha stores are
  not migrated, the standing rule). The record's axis point
  is where the shank crosses the material it actually passes through, in the member frame;
  its direction is the shank's section-plane projection; its extent is a range that covers
  only the **stack the fastener crosses** — the axis-ray material intervals of the member's
  own faces within the fastener's far seat (the distance from the bearing plane to the last
  material face the pack reaches, which `place_bolt_group` now reports per fastener) — so a
  bolt through one flange does not cut the opposite flange and a hollow section's two walls
  cut in one record (the amendment's acceptance cases 2 and 3). The same ceilings as the
  replay apply at the decision: a curved member or polybeam and a member whose element
  frame is not the coordinate axes (a roll, a skew) refuse with `UNSUPPORTED_GEOMETRY`, and
  a shank more than 60 degrees off its section-plane projection refuses with it (the plate
  ceiling mirrored; within the ceiling the axial tilt drops out of the extent exactly — the
  projection is orthogonal to the record direction). Every other member case is served by
  the accepted G14 geometry, with the kernel replay as the refusal gate.
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
- **A moved part regenerates the holes (issue 544).** Moving a connected part re-places its
  groups (A14.3 decision "a group belongs to its main part") and rewrites every group's
  records in one pass over the plates, the moved plate evaluated at its new placement; groups
  sharing a plate rewrite its record set once, with one serial for the derived record ids.
  Moving the part back restores the original meshes exactly.
- **Retire, do not migrate.** The descriptor gains the two `steel:bolt` attributes and the
  sixth hole attribute; pre-alpha stores are not migrated (the standing rule), and an A05
  annotation group stays readable exactly as before.

## Limits

- A shank axis more than 60 degrees off a plate's normal refuses `generate` with
  `UNSUPPORTED_GEOMETRY`; between 0 and 60 degrees the record is a normal-direction hole
  centred on the axis at the plate's mid-plane (the A04 ceiling above). On a member the
  same ceiling runs against the shank's section-plane projection.
- A member with a roll, a non-coordinate member axis, a curved path or a polybeam refuses
  `generate` with `UNSUPPORTED_GEOMETRY` — exactly the cases the member-hole replay
  refuses a stored record with; every other straight member is served. The hand-placed
  half of the same kernel slice (`add-hole --axis-point ...` on a member, A04.2) rides
  that replay and refuses the same member cases, so the generated and the hand-placed
  records of one member live on one record list in ordinal order (G14.1 complete on
  both halves).
- The record's extent covers only the stack the fastener crosses; a bearing whose axis
  finds no member material within the fastener's far seat (an existing larger opening,
  free fall past the outline) writes no record for that fastener.
- The records cut the connected plates' own bodies, and the placed prototypes (A14.2) are
  drawn by the view snapshot as one merged mesh per group (issue 512); the group itself still
  stores no derived mesh (U11).
- One group is one straight row (O-B1); no environments (O-B2).

## Tests

`steel::model::command`: the hole defaults follow the definition, an explicit mode and
clearance enter the fingerprint, the refusals name `hole_clearance_mm`.
`steel::engine::bolt`: one hole per crossing fastener per plate at the mapped centre, the
group clearance over the definition's, the screw default and the explicit override, a
partly covered row cutting only the crossed holes; the member decision covering only the
stack its fastener crosses, the hollow-section record spanning both walls, a never-crossed
member taking no record, and the unserved member cases (a curved or polybeam member, a
roll, an axial shank) refusing with the typed error. `steel::engine::command`: a
generated record refuses update and removal with `DEPENDENCY_CONFLICT`, and so does the
member side (`steel::engine::member_hole`).
`beamcraft_layer_engine`: the record attribute set is six-or-none with the owner column
checked; the descriptor-pinned stores revalidate.
`apps/beamcraft-cli/tests/bolt_groups.rs` (end to end through the binary): the create writes
two records per plate owned by the group and rewrites the derived meshes; a straight member
of the joint takes the same records through its wall (create, retry, the `DEPENDENCY_CONFLICT`
guards, a hand-placed hole beside them, the regeneration on an edit, `none` removing exactly
the group's records, a second group's delete removing them in one revision and undo bringing
them back); the edit and the undo move the whole unit; `none` writes no part and stores no
clearance; a screw defaults to `none` and an explicit `generate` overrides it; a member the
decision cannot serve (a polybeam) refuses; the repin regenerates;
`remove-hole`/`update-hole` on a generated record are `DEPENDENCY_CONFLICT` while a
hand-placed hole on the same plate works; turning `generate` off keeps exactly the manual
holes; deleting the group removes its records from both plates in one revision. The A05
end-to-end suite (`bolts.rs`) retired with the token; the listing and upgrade paths stay
covered by `bolt_groups.rs`.
