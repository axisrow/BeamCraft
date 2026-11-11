# A14.3 bolt group placement and editing

Date: 2026-10-31. Status: **experimental slice; A14 acceptance (holes in all parts, conflicts
as in A05) stays open and is claimed by A14.4.**
Branch `feat/a14-bolt-group-placement`, based on the master of 2026-10-31 (A14.1, A14.2 merged).

Plan row: A14.3 / M (slice of A14, issue axisrow/BeamCraft#311), contract
`docs/contracts/f04-bolt-fastener-amendment-draft-v0.1.md` sections 4 and 6 (without section 5:
the generated holes are A14.4's). Depends on A14.1 (the pinned definition and the library
lookup) and A14.2 (the prototype frame) and nothing else.

## What is implemented

```text
beamcraft create-bolt-group --store ROOT --name TEXT --parts UUID,UUID[,...]
    --fastener-library FILE --designation TEXT
    --point1 X,Y,Z --point2 X,Y,Z --plane-normal X,Y,Z [--rotation-deg DEG]
    [--on-plane MM] [--depth MM] --offset-start MM
    (--spacings a,b,c | --count N --pitch MM | --count N --offset-end MM)
    [--washer-under-head true|false] [--washer-under-nut true|false]
    [--length-mode auto|fixed --length MM] [--new-id UUID] [--op-id UUID ...]
beamcraft update-bolt-group --store ROOT --target GROUP_UUID [any of the inputs above]
    [--repin true --fastener-library FILE --designation TEXT]
beamcraft bolts --store ROOT        # A05 groups as annotations, A14.3 groups with records
```

| Area | What exists |
|---|---|
| `steel::model::command` | `BoltGroupInputs` (2 to 1024 distinct parts, the digest and complete `FastenerSpec` of the pinned definition, two distinct points, a unit plane normal, rotation in radians, `on_plane`/`depth`/`offset_start`, `BoltSpacing` and `BoltLength`), `CreateBoltGroup` and `UpdateBoltGroup` with admission (`F04` codes, the vocabulary rules) and versioned content fingerprints. The definition travels by digest: equal digests are equal definitions, so the fingerprint carries the digest, not the fifteen values. |
| `steel::engine::bolt` | `place_bolt_group`: the frame (`x` along the row, `z` the plane normal's component perpendicular to `x` turned by the rotation, `y = z × x`), the authoritative spacings (the generators expand once and travel with the group), the bearing points (`point1 + on_plane·y + depth·z + s_i·x`) and the one length for the whole group. The crossing admission stays the conservative A05 boxes (a part the axis never crosses is refused); the grip itself is face-exact (issue 519 defect B): the union length of the shank inside the actual faces of the crossed parts — the world triangles of their stored scene meshes, exact under any placement rotation, overlapping parts merged. A part without a usable stored mesh grips per its conservative box, which errs long and never under. `auto` picks the smallest listed length over the largest grip plus the head height, the far washers, the nut and the protrusion; a fixed length must be listed and must cover the joint. `Parts::create_bolt_group`/`update_bolt_group` add the identity, the parts' kinds and the footprints. |
| `beamcraft_layer_engine::authored` | `Shape::bolt_group` and `Change::bolt_group` (the discipline hooks of the element interface): fifteen `steel:bolt:*` attributes (the definition digest, both points, the normal, rotation, the position values, the spacing mode and value, the authoritative spacings, the length mode and the resolved length, the washer switches) replaced as a whole on every write; the `connects` record carries 2 to 1024 targets. The loaded-content discriminator is the definition digest: an A05 group (the five annotation attributes) and an A14.3 group (the fifteen) are never mixed, and a stored group re-validates against its pinned definition (the resolved length is a listed length, a washer switch honours the definition). The schema descriptor gains the A14.3 lines; pre-alpha stores are not migrated (the standing rule). |
| `beamcraft_commands::bolt_cmd` | `create_bolt_group`: resolves the library entry, pins its definition **in the same transaction** when the model does not hold the digest yet, decides the placement, authors the group and publishes one revision. Replay is decided from the operation's own revision (the group created there, present, and matching the complete record); otherwise `OP_ID_REUSE`. `update_bolt_group`: the complete new inputs replace the stored record as one atomic edit and one undo unit; a `repin` takes the current library entry and pins it; the stored definition is otherwise kept verbatim even if the file changed. `bolts` lists both forms. |

## Decisions

- **Holes are absent, not defaulted.** `hole_mode` is not admitted in this slice; the group
  changes no part. A14.4 adds the hole records and the `generate`/`none` setting over the same
  command shapes.
- **`create-bolt` (A05) stays until A14.4.** The amendment replaces it, but the replacement's
  argument list carries `hole_mode`; retiring the token in the same PR that cannot yet admit
  the field would leave annotation groups uncreatable. `create-bolt-group`/
  `update-bolt-group` are the new surface; the A05 token retires with A14.4.
- **The generators travel with the group.** The stored record keeps the spacing mode and the
  generator value next to the authoritative (expanded) spacings, so a moved `point2`
  re-expands an `offset_end` row at the CLI without a second input; an explicit list stays
  authoritative. A partial edit that names a spacing form replaces the stored pattern.
- **The grip is the clamped pack on the bearing ring; the crossing stays box-conservative.**
  The crossing admission keeps the A05 rule (every listed part crossed by a fastener axis, by
  its conservative box). The box is the plate's contour extruded on its authored
  `thickness_side` (`AuthoredPlate::local_box_mm`: top `[0, t]`, middle `[-t/2, t/2]`, bottom
  `[-t, 0]`), turned by the placement — never `+ t` upwards by assumption (issue 555 finding 3).
  The grip that decides the length is the pack the fastener clamps between its bearing faces
  (issue 555 finding 1). A fastener always stands in a hole and the head and the nut bear on the
  ring of material around it, not on the axis line, so the engine measures each part along the
  axis and along eight rays parallel to it on the bearing ring — midway between the clearance
  hole's radius (nominal plus the resolved clearance) and the washer's outer radius (the
  head's, without a washer) — and the part contributes the ray with the most material. Each ray
  is face-exact: the adapter turns the part's stored scene mesh into world triangles and the
  engine pairs the ray's crossings with them, exact under any placement rotation (issue 519
  defect B: a 40-degree brace's box claimed a 49 mm grip where its solid grips 37 mm). Hits
  that land on one boundary within rounding distance merge into one crossing (a ray through a
  shared triangle edge hits both triangles a few ulps apart); an odd count pairs the first
  crossing with the bearing face, so a bearing inside a part grips its remaining material.
  Consequences: a coaxial clearance hole grips its slab whoever cut it — the group's own
  generated record, another group's, `add-hole`, `create-plate --hole-round` — so the length
  no longer depends on the order of the operations and a plain rename or a retry re-places to
  the stored length (the earlier "own holes heal from the box" special case is gone). A listed
  part whose box the axes cross but whose bearing ring no fastener ever finds material in (an
  opening wider than the washer, or a box overlap past the part's outline) is refused with
  `INVALID_ARGUMENT`, field `parts`: the group would not clamp it, and it is never silently
  dropped from the grip.
  The length rule reads "plus the washers" as every washer the shank passes through, and the
  length requirement adds the head height (EN 15048-1 / EN 1993-1-8 3.6 clamp length): the
  bearing plane is the joint face, the head washer and the head stack outside the joint below
  it, the shank starts inside the head and runs through it into the joint, and the nut stack
  hangs downward from the thread tip, so a length covering grip + head + head washer + nut
  washer (each when placed) + nut + protrusion seats the nut washer exactly on the far plate.
  `auto` is the smallest listed length covering that requirement (issue 555 audit item 9; the
  earlier rule left the head out and could resolve one step short, and the earlier prototype
  stacked the whole assembly inside the joint).
- **The digest is the definition identity.** The command carries the digest and the resolved
  spec; the adapter re-renders the canonical resource and refuses a mismatch, the engine
  re-admits the spec for its decisions, and the layer validator re-checks the stored record
  against the pinned definition. A group never re-reads a library file.
- **`repin` is explicit and value-taking.** `--repin true` names the library entry the group
  follows; without it an update keeps the pinned digest, so a changed library file changes
  nothing until a person asks. A stored A05 group is upgraded by naming the placement inputs
  and a library entry (its `--repin`-guard does not apply: the annotation has no definition to
  protect).
- **A group belongs to its main part (issue 544).** `update-element` that moves, rotates or
  re-orients a connected part carries the groups over it in the same transaction
  (`bolt::follow_moved_part`, called from `edit::update_element`): the moved part's world
  boxes and faces are carried by the rigid delta `new · old⁻¹`; a group whose main part
  (`parts[0]`) moved has its row points and plane normal carried by the same delta, a group
  whose other part moved keeps its fasteners; every such group is re-placed against the moved
  geometry and its generated holes regenerated (A14.4), all in one undo unit. A part that no
  longer crosses the row, or a hole the engine cannot generate, refuses the whole edit with
  nothing written, so the stored node never comes apart silently. A mirror is not rigid and
  refuses `DEPENDENCY_CONFLICT` on a bolted part, as does a move combined with a shape or
  material change of that part in one edit.

## Limits

- No holes (A14.4), no quantities stored on the group, and no derived mesh or instancing in
  the layer (U11): the placements are decided and reported, and — since issue 512 — drawn, as
  one read-only scene mesh of the placed A14.2 prototypes merged per group.
- The crossing admission stays box-approximate (the A05 limit); the grip is face-exact over
  the stored scene meshes, so a part without one (an authored-only element, a tampered derived
  layer) grips per its conservative box, which errs long.
- The UI (V19) is not started; the CLI is the only surface.
- One group is one straight row (O-B1); no environments (O-B2).

## Tests

`steel::model::command`: the inputs admit two parts, generators and a listed fixed length;
refusals name their field; every input and the part order enters the fingerprint.
`steel::engine::bolt`: the frame under rotation, `on_plane`/`depth`, the bearing points,
the generator expansions and the offset-end re-expansion, the auto boundary (grip + head
height + the far washers + nut + protrusion, the under-head washer counted when placed,
overlapping parts counted once), the fixed and crossing refusals, the washer switches
against the definition, and the
identity/parts/footprint decisions. The face grip (issue 519 defect B) has its own tests: the
rotated knee brace resolves to its face length and not the box one, with the exact grip
boundary pinned by a fixed length that refuses and one that covers, a part without faces
falling back to its conservative box, a centred bolt over the ear-clip diagonal resolving the
full thickness (the merged double hit), a generate group re-placing over its own coaxial
holes at the healed length (and collapsing without the healing), and the knee brace of the
CLI repro carrying the face-sane length end to end, a rename and a moved row keeping it over
the rewritten meshes, and a create retry replaying. The existing descriptor-pin tests accept
the previous descriptors and refuse a forged pin (the standing pattern).
`apps/beamcraft-cli/tests/bolt_groups.rs` (end to end through the binary; every read re-runs
the loaded-content validator, so the stored-form rules — the digest discriminator, the
mixed-set refusals, the record-versus-definition checks — are exercised on real commits): one
undo unit (undo reverts the whole record), the footprint (the parts and the definition read,
the group and the pinned entry written, one authored layer), replay and `OP_ID_REUSE`; the
generators and the offset-end re-expansion after a moved `point2`; the length modes and the
washer cases (a switch the definition lacks, a screw under the nut, a fixed length not listed
or not covering the joint); pinning (a deleted library file changes nothing), `repin`
following the library, and the A05 upgrade; the amendment's negative cases (unknown
designation, missing library, coincident points, a parallel normal, a negative offset, an
uncrossed part, an assembly part, one part, a taken identity, exclusive spacing forms) write
no revision.
