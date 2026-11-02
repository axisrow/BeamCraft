# D1 and D2: the stored ProjectId and levels

Date: 2026-10-18. Status: **experimental vertical slice; D1 and D2 acceptance is limited as listed
below.** Branch `feat/d1-d2-project-level`, based on `origin/master` `5c54c0a`.

These are the two model prerequisites of the IFC export (`docs/contracts/e01-ifc-export-v0.1.md`
section 19, owner decisions Q-3 and Q-7). The export itself (E02) is out of scope: nothing here
writes IFC.

Result of D1: the project entry of a store records exactly one ProjectId, so a reader (the future
exporter) takes a stable project identity from a committed revision. Result of D2: a level
(storey) is a model concept with its own identity, a name and an elevation, and a plate, beam or
column can be assigned to one.

## What is implemented

```text
beamcraft create-level --store ROOT --name TEXT --elevation MM [--new-id UUID] [--op-id UUID ...]
beamcraft levels --store ROOT
beamcraft update-element --store ROOT --id UUID --level UUID|none [...]
beamcraft inspect-elements --store ROOT      (now: project_id_record, levels, "level" per element)
beamcraft delete-element --store ROOT --id LEVEL_UUID   (refused while elements use it)
```

| Area | What exists |
|---|---|
| D1 root record | The first revision of a project writes the ProjectId into the `customLayerData` of the root layer as `bimProjectId` (a UUIDv7 text), next to `bimSchemaDigest`. `plate::load_base` creates that root for a store without a revision; every later revision starts from the head's root, and the layer engine rewrites the root with the other dictionary entries preserved, so the record is carried unchanged (the test checks that the root layer digest does not change across revisions). |
| D1 reading | `beamcraft_layer_engine::authored::project_record` reads it from the opened metadata: absent is `None`, a value that is not a full UUIDv7 text is `INVALID_ID`. The structural validator (`validate::snapshot`) applies the same rule, so an invalid record is refused on every open, authoring candidate and `validate_layers`. `inspect-elements` reports `project_id_record` (a string or `null`). |
| D1 agreement | The commit verifier (`PlateVerifier`) and `inspect` require a recorded ProjectId to equal the project of the manifest (`INTEGRITY_ERROR`). With the host's existing check that a revision names the head's project (`ProjectMismatch`), every revision of one store reports the same record. A fork (`create-workspace`) copies the layers and therefore the record. |
| D1 mismatch | `plate::ensure_project` and `--project-id`: a store with a head refuses an explicit, different project id with `INVALID_ARGUMENT` and writes nothing. Before this task the id was silently ignored once the store had a revision. It runs in every command that writes a revision (`create-plate`, `create-member`, `define-*`, `update-element`, `delete-element`, `copy-element`, `create-assembly`, `assembly-*`, `create-weld`, `create-level`, `reference-*`). A command that gives no id takes the store's. |
| D2 model | `beamcraft_model::command::CreateLevel { new_id, name, elevation_mm }` (label rule; finite; absolute value at most 1,000,000,000 mm, the coordinate cap; fingerprint `BeamCraft/D2/CreateLevel/v1`). `ElementPatch.level: Option<Option<ElementId>>` (`None` no change, `Some(None)` remove, `Some(Some(id))` assign), part of the validity rule and, when present, of the `UpdateElement` fingerprint (earlier patches keep theirs). |
| D2 engine | `beamcraft_engine::level::Levels` decides `CreateLevel` (`DUPLICATE_ID`, argument rules) and an assignment: unknown element `NOT_FOUND`, an element that is not a plate, beam or column `UNSUPPORTED_PROPERTY`, an unknown level `DANGLING_RELATIONSHIP`, a target that is not a level `INVALID_ARGUMENT`. `Footprint::for_level` reads the element's lifecycle and relationships and the level's lifecycle and writes only the element's relationship group; `Footprint::union` combines it with an update's other groups. `UpdateEffect.level` says that a patch changes the level. |
| D2 layer engine | `Shape::Level { elevation_mm }` (an `Xform` element of kind `level`, USD kind `component`, `bim:elevationMm`, no material, no `Geometry`, identity placement) and `Change::Level { id, level }` (one explicit `bim:level` relationship, or an explicit empty list to unassign). Rule `D2-V01` in the structural validator: only a plate, beam or column carries `bim:level`, at most one target (`DEPENDENCY_CONFLICT`), which is an active owned level (`DANGLING_RELATIONSHIP`, `INVALID_ARGUMENT`); a level carries no `bim:level`, `bim:partOf` or `bim:profile`; elevation finite and within the cap. `copy_plan` carries a `bim:level` link to the copy. The mapping descriptor gains the root record line, the `level` element, `bim:level` and the `Level` change item; layers pinned to the A07.1 descriptor stay accepted. |
| D2 CLI | `beamcraft_commands::level` (`create_level`, `levels`, `levels_of`, `LevelView`); `level_cmd`; the level field of `edit::update_element` (replay and no-op decided from the head state, the level decision runs after them); `copy::copy_element` keeps the level; `delete::delete_element` accepts a level (`DeletedKind::Level`) and the layer engine's incoming-link index refuses it while elements use it (`DEPENDENCY_CONFLICT`). |

## Decisions (the first one needs the owner)

1. **A level is an element of kind `level`, not a project-owned record.** The task asked for an
   element with its own UUID and a relationship; the IFC contract (section 19.2) says "a Level is a
   project-owned record, not an element kind" and proposes an F10 note to that effect. The
   implemented shape keeps the contract's content (LevelId, name, `elevation_mm`, at most one
   assignment per element, a level has no geometric effect, deletion refused while referenced) and
   reuses the element machinery (identity path, undo unit, footprint, delete index, replay) the way
   A01 and A06 do. The exporter must treat `bim:elementKind = level` prims as storeys and must not
   list them among the elements it exports (`E01-POS-45` counts elements, not levels). **If the
   owner wants a record outside the element tree, this is the part to redo**; the commands, the
   validation rules and the tests would carry over.
2. **The ProjectId lives in the root layer's `customLayerData`.** The contract leaves the encoding to
   a U-task ("how it is encoded in USD or the manifest is a U-task decision"). The host manifest
   already carries a project id and enforces it per head; the root-layer record is the copy that
   an exporter reads through the layer reader from the pinned snapshot, and the verifier ties the
   two together. It is written only by the first revision (`plate::load_base`); no command, change
   item or option rewrites it.
3. **A store without a record stays without one.** Later revisions of a store whose first revision
   predates D1 keep the record absent (tested): the export refuses it with
   `DEPENDENCY_UNAVAILABLE` (`E01-NEG-48`) and the reviewed assignment command that the contract
   asks for does not exist yet.
4. **Assignment is the `update-element` path.** `--level UUID|none` combines with the other patch
   fields in one operation and one revision. The record is a relationship of the element, not an
   attribute value: it shows in the incoming-link index (so a used level cannot be deleted) and in
   `copy_plan`.
5. **Assignment writes only the `relationship` group.** It reads the level's lifecycle, so a
   concurrent delete of the level conflicts. It writes no derived group: the test compares the
   derived layer digest, the mesh digest and the stored quantity record before and after.
6. **Unassigning is an explicit empty list**, not a removed spec, so a weaker owned layer's record
   is masked rather than revealed (as in A01).
7. **Only plates, beams and columns are assigned.** An assembly, a weld or a reference has no level
   in this slice; the validator refuses a loaded record on them.
8. **The elevation cap is the F04 coordinate cap** (1,000,000,000 mm, inclusive), negative values
   allowed. NaN and infinities are `NON_FINITE_NUMBER`, beyond the cap `LIMIT_EXCEEDED`.
9. **An explicit `--project-id` is a claim bound to base loading.** The CLI runs the early typed
   check (`plate::ensure_project`) and then holds a `plate::ProjectClaim` (a thread-local claim
   with a drop guard) for the whole command. `load_base` compares the claim with the project of
   the exact head it opened, which is the base the revision is committed on, and a first revision
   records the claimed project; a store initialised by another writer between the check and the
   base loading is refused with `INVALID_ARGUMENT` (regression test
   `tests/project_claim.rs`), and a writer that raced on an empty store is stopped by the host's
   initial-base check. The library request types keep their `project` field, which a store with a
   head still ignores when no claim is held; a library caller that names a project must hold a
   claim too. The claim binds reads of the same thread as well. **Design choice to confirm:** a
   thread-local claim avoids adding a field to every request type; the alternative is an explicit
   `claimed_project` field on each request/context.
10. **Replay of a level creation is decided from the operation's own revision** (the host ledger's
    recorded sequence, as `weld::created_by`): that revision must have introduced the level id,
    with the same name and elevation, and not the one before it; another request under the same
    `op_id` is `OP_ID_REUSE` whatever the head holds.
11. **Readers apply the ownership, schema and mode guard.** Foreign, newer-schema and
    unsupported-mode elements are inert identities (`Levels::insert_other`), so they neither fail
    the level reader nor can be listed or assigned. `inspect-elements` runs the structural validator
    before decoding. An element that exists but cannot hold a level (a level, assembly, weld or
    reference) is `UNSUPPORTED_PROPERTY` for `update-element --level`, not `NOT_FOUND`. An owned,
    current-schema reference wrapper with a nonempty `bim:level` is refused by the structural
    validator (`INVALID_ARGUMENT`).

## Requirement coverage

Contract section 19.1 (D1): (1) a new store records a valid UUIDv7 ProjectId: tested; (2) every
later revision reports the same one: tested; (3) a fork reports its parent's: tested; (4) cannot be
changed by any command: no command writes it, and a different explicit id is refused;
**`UNSUPPORTED_PROPERTY` is not used, the refusal is `INVALID_ARGUMENT`** (the existing host code
for a project mismatch); (5) an existing store without one is not silently given one: tested for
the CLI path; the export's `DEPENDENCY_UNAVAILABLE` is E02's; (6) an invalid stored value is
`INVALID_ID` when read: tested; (7) engine API, CLI and end-to-end test: yes.

Contract section 19.2 (D2): (1) create, assign, unassign and delete (refused while referenced)
have an API, a CLI command and an end-to-end test; **rename and change of elevation do not**; (2) a
level change leaves every element's shape key, mass signature and mesh bytes unchanged: tested;
(3) an assignment to an unknown LevelId is `DANGLING_RELATIONSHIP`: tested, for the command and for
a stored graph; (4) a project with no level and elements with no level reopens unchanged: tested;
(5) levels and assignments survive an edit of anything else: tested by the rename, move and copy
steps; unknown namespaces are preserved by the layer engine generally (U02), not by a D2-specific
test; (6) the cases `E01-POS-45`, `E01-POS-46` and `E01-NEG-49` are reachable (and `E01-NEG-48` for
D1): the stores they need can be built with these commands, the dangling case by storing the
record outside BeamCraft as the test does.

Provenance: the rules come from the repository's contracts (IFC export contract section 19, F04,
the domain vocabulary patterns for relationships); no external product material was used. IFC is
not written by this task.

## Evidence

* `apps/beamcraft-cli/tests/levels.rs` (8 end-to-end tests through the binary): the project is
  recorded by the first revision and the root digest is stable across revisions; an explicit
  different project is refused by every revision-writing command with no revision written; a
  generated project is recorded; a fork and edits inside it keep it; a store without a record
  stays without one; an invalid record is `INVALID_ID`, a valid foreign one `INTEGRITY_ERROR`, and
  nothing builds on either; levels are created, listed in identity order, assigned, replaced,
  cleared and combined with a move, and the derived layer, the mesh digest, the stored quantities
  and the snapshot meshes do not change; duplicate ids, non-finite and out-of-range elevations, a
  malformed id, a missing option, an empty name, a dangling level, a non-level target, a missing
  element and no-op updates are refused with no revision written; replay and `OP_ID_REUSE` for
  creation and assignment; a copy keeps the level and a level is not copied; a level in use is
  `DEPENDENCY_CONFLICT` until the last element leaves; stored graphs with a non-level target, two
  targets and a missing target are refused when read.
* `apps/beamcraft-cli/tests/project_claim.rs`: a store initialised by another writer after a
  caller's early check is not adopted by the caller's claimed project; claims nest and restore.
* `crates/layer-engine/tests/authored.rs`: authoring and validating levels and `bim:level` records
  (kinds, groups written and read, bodies, elevation, loaded tampering, `check_delete`,
  `copy_plan`) and the project record (read, validated, kept through an edit, absent, invalid);
  `src/authored.rs` pins the A07.1 descriptor digest.
* Unit tests: model (`CreateLevel`, the level patch and its fingerprint), engine (`Levels`,
  `Footprint::for_level` and `union`, `UpdateEffect`).

## Not done

* No rename and no change of elevation of a level (an `UpdateLevel` command and a layer-engine
  change item), no multi-level operations, no ordering by elevation beyond what the reader sorts.
* No assignment at creation (`create-plate --level`), no assignment of assemblies, welds or
  references, no levels in the viewport.
* No reviewed command that assigns a ProjectId to a store that has none, and no migration.
* The raw `commit-revision` path does not require a store that had a record to keep it: the verifier
  checks that a recorded id equals the manifest's project, not that a record present in the base
  is still there. Removing it outside BeamCraft makes the store a "store without a record".
* The explicit project claim is a thread-local guard, not a field of the request types (decision 9).
* The F04 and F10 amendments that the contract proposes are still not applied.
* The schema descriptor changed (`bimProjectId` line, `level` element, `bim:level`, `Level`
  change): other open PRs that also edit it (for example A07.2) will conflict; the earlier
  descriptor list needs both entries.
