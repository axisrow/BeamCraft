# A05 minimal bolt groups

Date: 2026-10-19. Status: **experimental vertical slice; A05 acceptance is limited as listed
below.**
Branch `feat/bolts`, based on `origin/master` `11a2139` (A01, A06, A04 pilot and A07.2 merged).

Plan row: A05 / L (issue axisrow/beamcraft#5; the letter is read from the K6 package order
A01–A09 in `ROADMAP.md`, the implementation plan itself is not in the repository). Depends on
A01 and P03, and follows the A06 weld pattern. Result: relationships and properties of one
supported bolt group without promising solid bolt geometry. Acceptance: API/CLI end to end; the
two joined parts are authored explicitly; the group lies on both parts; BC coverage is limited
and listed.

## What is implemented

```text
beamcraft create-bolt --store ROOT --name TEXT --main PART_UUID --secondary PART_UUID
                      --origin X,Y,Z --direction X,Y,Z --count N --pitch MM --diameter MM
                      [--new-id UUID] [--op-id UUID ...]
beamcraft bolts --store ROOT
```

| Area | What exists |
|---|---|
| `beamcraft_model::command` | `BoltSpec` and `CreateBolt { new_id, name, main_id, secondary_id, bolt }`: label, two distinct parts, a group that is not one of its parts, a finite diameter in `(0, 1000]` mm (`BOLT_DIAMETER_CAP_MM`), a group size of 1 to 512 bolts (`MAX_BOLT_GROUP_SIZE`), a finite pitch in `(0, 1000000]` mm, a finite origin within the coordinate cap and a finite unit direction (squared norm within 1e-6 of one); versioned content fingerprint. The direction is stored normalized; the CLI normalizes its input. |
| `beamcraft_engine::bolt` | The same `Parts` element index as A06 decides `CreateBolt`: free identity, both parts exist and are plates, beams or columns, and every bolt position `origin + direction * (pitch * i)`, `i` in `0..count`, lies within the world bounds of **both** parts (inclusive componentwise). The adapter supplies the conservative world bounds of the parts; the decision stays pure. Returns the footprint. |
| `beamcraft_engine::footprint` | `Footprint::for_bolt(id, [main, secondary])`: writes what a create writes for the group, reads each part's lifecycle. No part is written. |
| `beamcraft_layer_engine::authored` | `Shape::Bolt(Bolt)`: an element of kind `connection`, model kind `component`, five `steel:bolt:*` properties and one explicit, ordered `bim:connects` relationship (main part first); no material, no `Geometry`, identity placement. A `connection` now carries exactly one of the weld or the bolt property set (never both, never neither). The structural validator adds rule `A05-V01`; `A06-V01` keeps checking weld records. The mapping descriptor gains the bolt line and rewords the connection line; the previous digest stays accepted and stores are repinned on edit. |
| `beamcraft_commands::bolt` | `create_bolt` and `bolts`: replay by `op_id` decided from the operation's own revision, `OP_ID_REUSE`, `DUPLICATE_ID`, `NOT_FOUND`, `INVALID_ARGUMENT` for a group that does not lie on both parts; the layer engine's observed writes are checked against the footprint; commit through U06/U07 with the strict verifier. `delete-element` deletes a bolt group (kind `bolt`) and refuses a bolted part while the group connects it; `copy-element` refuses a bolt group. |

## Decisions

- **No new kind.** The domain vocabulary registry is closed; `connection` (steel) is the kind that
  carries one `connects` record with 2 to 1024 targets and leaves the connection itself
  undefined. A06 implemented it restricted to a weld; A05 extends the same kind to bolt groups.
  The discriminator on stored bytes is the authored property set: `steel:weld:*` is a weld,
  `steel:bolt:*` is a bolt group, anything else (or both) is refused as content this version
  cannot read — never guessed.
- **A group, not single bolts.** One element is one group of `count` bolts in a straight line:
  `p_i = origin + direction * (pitch * i)`. The origin and direction are world coordinates in
  millimetres (the element placement is the identity, as for a weld); the direction is stored as
  a unit vector. Multi-row rectangular patterns, holes and slots are not done.
- **Two parts, ordered.** Like a weld, a group connects exactly two distinct plates, beams or
  columns (`DV-REL-004`, `DV-REL-006`, `DV-REL-007`); the first target is the main part. An
  assembly target is refused as unsupported (`UNSUPPORTED_GEOMETRY`). The record lives with the
  group (vocabulary section 5.2), so the parts are only read, a concurrent delete of a part
  conflicts, deleting a bolted part is `DEPENDENCY_CONFLICT` while the group connects it
  (`DV-REL-009`), and copying the group itself is `DEPENDENCY_CONFLICT` (`DV-REL-011`); copying
  a bolted part succeeds and carries no bolt record, exactly as the copy of a welded part
  leaves the weld on the original.
- **"Face reachable" is a conservative bounds check.** The engine requires every bolt position to
  lie within the world bounding box of both parts, inclusive of the boundary (a bolt on a face is
  reachable). The adapter derives the bounds from the authored inputs without the kernel: a plate
  contributes the exact box of its contour extruded by its thickness; a member contributes its
  two world axis points expanded by half the section's bounding-box diagonal in every axis, which
  contains the member under any roll. A part with a non-identity placement rotation is refused as
  unsupported in this slice rather than bounded wrongly. The check accepts boxes that overlap in
  mid-air and never proves a real face intersection; a face-exact test needs the kernel in the
  decision path and stays open.
- **Explicit properties, no defaults.** Diameter, count, pitch, origin and direction are all
  required; omitting one is `INVALID_ARGUMENT`. Tokens are stored exactly (`steel:bolt:diameterMm`
  and `steel:bolt:pitchMm` as `double`, `steel:bolt:count` as `int`, `steel:bolt:originMm` and
  `steel:bolt:directionMm` as `double3`).
- **No body, no volume, count is the quantity.** Following the A06 decision, a bolt group is an
  annotation with no material and no derived mesh; a `Geometry` mesh under one is refused,
  authored or loaded, and it adds nothing to the view snapshot. It changes no part's volume or
  mass; the group's quantity is its authored `count`, which a later BOM (E03) can read — no
  quantity is stored or computed in this slice.
- **Replay is bound to the operation's own revision**, exactly as A06: the request is read from
  the revision the ledger recorded, so a later group with matching arguments under a reused
  `op_id` is `OP_ID_REUSE`, not a replay.
- **Only supported content is interpreted.** `bolts` and `create-bolt` read kinds only from
  owned, `0.1.0`, `parametric` elements; a weld is never decoded as a bolt group and vice versa,
  and newer-schema or foreign `connection` elements stay inert.
- **Stored bytes are verified, not interpreted.** `bolts` and `create-bolt` run the structural
  validator first, so a damaged `connects` record is refused rather than reported or replayed.

## Requirement coverage

Domain vocabulary 0.1.0 rules implemented for `connects` again cover `DV-REL-001`, `DV-REL-003`
(narrowed to parts), `DV-REL-004` (narrowed to two), `DV-REL-006`, `DV-REL-007`, `DV-REL-009` and
`DV-REL-011` — now for bolt groups as well as welds.

Bolt coverage: a bolt group object with its own identity, the two parts it joins, an explicit
world placement (origin, unit direction), count, pitch and diameter, and a conservative
reachability check. **Not covered:** solid bolt geometry and volume, bolt length and grip, holes
cut by the group, multi-row and circular patterns, bolt grades and classes, washers and nuts,
slotted holes, assembly-level bolts, BOM output. The F02 requirement texts are a local analysis
artifact and are not quoted here.

Provenance: the rules come from the repository's domain-vocabulary contract (sections 2.3, 4.6
and 5.1 to 5.3) and the A06 precedent; the property names and tokens are original. No external
product material was used.

## Evidence

- `apps/beamcraft-cli/tests/bolts.rs` (end-to-end tests through the binary): a bolt group writes
  only `authored.usda` and only its own footprint groups, reads both parts, lists back with every
  property, leaves the parts' bodies and the view unchanged; replay and `OP_ID_REUSE`; delete of
  a bolted part and copy of the group are `DEPENDENCY_CONFLICT`; deleting the group then the part
  works; a weld and a bolt group coexist and neither listing decodes the other. Refusals of same
  part twice, unknown part, assembly or bolt group as a part, a group that is its own part, a
  taken identity, each missing property, bad tokens and numbers, a non-unit direction, an
  out-of-range count, pitch or diameter and an unreachable origin write no revision. A tampered
  one-target record is refused on read.
- `crates/layer-engine/tests/authored.rs`: bolt authoring, model kind, ordered targets, no body,
  reported opinions, atomic refusals, loaded-content checks (property-set discriminator, bounds,
  unit direction), delete and copy evidence.
- Unit tests cover the model command, the engine decision (including per-position reachability)
  and the mapping pin of the previous descriptor.

## Not done

- No command edits, moves or deletes a bolt group other than `delete-element`; no bolt group in
  an assembly; no `part-of` from a connection.
- No bolt geometry, length, holes or quantity records; nothing is exported to IFC or a BOM
  (E02/E03). `bim:discipline` is still not authored.
- The face reachability check is a bounding-box proxy (see Decisions), not a face test.
- Domain vocabulary 0.1.0 lists `connection` as reserved (`DV-KIND-004`); as A06 recorded,
  admitting the kind with its attributes in the next vocabulary version is the owner's decision.
- The schema descriptor changed: other open PRs that also edit it will conflict and need the
  earlier-descriptor derivation extended.
