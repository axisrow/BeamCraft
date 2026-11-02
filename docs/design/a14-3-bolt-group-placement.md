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
| `steel::engine::bolt` | `place_bolt_group`: the frame (`x` along the row, `z` the plane normal's component perpendicular to `x` turned by the rotation, `y = z × x`), the authoritative spacings (the generators expand once and travel with the group), the bearing points (`point1 + on_plane·y + depth·z + s_i·x`) and the one length for the whole group. The grip of a fastener is the union length of its axis inside the same conservative boxes A05 uses (slab intervals, overlapping parts merged); `auto` picks the smallest listed length over the largest grip plus the far washers, the nut and the protrusion; a fixed length must be listed and must cover the joint; a part the axis never crosses is refused. `Parts::create_bolt_group`/`update_bolt_group` add the identity, the parts' kinds and the footprints. |
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
- **The grip is the conservative union over the A05 boxes.** The engine never proves a face
  intersection (the A05 decision); for plates the boxes are exact, so the grip is exact there,
  and for members it errs long. The length rule reads "plus the washers" as the washers the
  shank must reach across: the under-nut washer lies between the grip and the nut and adds its
  thickness, while the under-head washer lies before the shank starts and adds nothing — the
  A14.2 stack fixes the shank at the under-head plane.
- **The digest is the definition identity.** The command carries the digest and the resolved
  spec; the adapter re-renders the canonical resource and refuses a mismatch, the engine
  re-admits the spec for its decisions, and the layer validator re-checks the stored record
  against the pinned definition. A group never re-reads a library file.
- **`repin` is explicit and value-taking.** `--repin true` names the library entry the group
  follows; without it an update keeps the pinned digest, so a changed library file changes
  nothing until a person asks. A stored A05 group is upgraded by naming the placement inputs
  and a library entry (its `--repin`-guard does not apply: the annotation has no definition to
  protect).

## Limits

- No holes (A14.4), no quantities stored on the group, no derived mesh or instancing (U11);
  the prototype placements are decided and reported, not drawn.
- The reachability and the grip stay box-approximate for members (the A05 limit).
- The UI (V19) is not started; the CLI is the only surface.
- One group is one straight row (O-B1); no environments (O-B2).

## Tests

`steel::model::command`: the inputs admit two parts, generators and a listed fixed length;
refusals name their field; every input and the part order enters the fingerprint.
`steel::engine::bolt`: the frame under rotation, `on_plane`/`depth`, the bearing points,
the generator expansions and the offset-end re-expansion, the auto boundary (grip + far
washer + nut + protrusion, the under-head washer adding nothing, overlapping parts counted
once), the fixed and crossing refusals, the washer switches against the definition, and the
identity/parts/footprint decisions. The existing descriptor-pin tests accept the previous
descriptors and refuse a forged pin (the standing pattern).
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
