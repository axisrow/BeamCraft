# A06 minimal weld

Date: 2026-10-18. Status: **experimental vertical slice; A06 acceptance is limited as listed below.**
Branch `feat/a06-weld`, based on `origin/master` `7b1f2bf` (A01 and C01 merged).

Plan row: A06 / M, depends on A01 and P03. Result: relationships and properties of a supported
weld without promising full solid geometry. Acceptance: API/CLI end to end; workshop/site and the
side are authored explicitly; BC-WELD coverage is limited and listed.

## What is implemented

```text
beamcraft create-weld --store ROOT --name TEXT --main PART_UUID --secondary PART_UUID --size MM
                      --location workshop|site --side arrow|other|both [--type fillet]
                      [--new-id UUID] [--op-id UUID ...]
beamcraft welds --store ROOT
```

| Area | What exists |
|---|---|
| `beamcraft_model::command` | `WeldType` (`fillet`), `WeldLocation` (`workshop`, `site`), `WeldSide` (`arrow`, `other`, `both`), `WeldSpec` and `CreateWeld { new_id, name, main_id, secondary_id, weld }`: label, two distinct parts, a weld that is not one of its parts, a finite size in `(0, 1000]` mm (`WELD_SIZE_CAP_MM`); versioned content fingerprint. |
| `beamcraft_engine::weld` | `Parts` (element kinds of a model) decides `CreateWeld`: free identity, both parts exist and are plates, beams or columns; returns the footprint. |
| `beamcraft_engine::footprint` | `Footprint::for_weld(id, [main, secondary])`: writes what a create writes for the weld, reads each part's lifecycle. No part is written. |
| `beamcraft_layer_engine::authored` | `Shape::Weld(Weld)`: an element of kind `connection`, model kind `component`, the four `steel:weld:*` properties and one explicit, ordered `bim:connects` relationship (main part first); no material, no `Geometry`, identity placement. The structural validator adds rule `A06-V01`. The mapping descriptor gains three lines; layers pinned to the A01 descriptor and the earlier ones stay accepted. |
| `beamcraft_commands::weld` | `create_weld` and `welds`: replay by `op_id`, `OP_ID_REUSE`, `DUPLICATE_ID`, `NOT_FOUND`; the layer engine's observed writes are checked against the footprint; commit through U06/U07 with the strict verifier. `delete-element` deletes a weld; `copy-element` refuses one. |

## Decisions

- **No new kind.** The domain vocabulary registry is closed (section 2.1); adding `weld` would need
  a new vocabulary version. The reserved `connection` kind (steel) already requires one
  `connects` record with 2 to 1024 targets and leaves welds undefined, so A06 implements that kind,
  restricted to a weld. The weld properties are additive optional attributes (section 4.6).
- **Two parts, ordered.** A weld connects exactly two distinct parts (`DV-REL-004` within the
  vocabulary's 2..1024, `DV-REL-006`, `DV-REL-007`). The first target is the main part, the second
  the part welded to it; the order is stored in the relationship's explicit target list and is
  significant, so swapping the parts is a different command (and fingerprint).
- **Targets are parts.** The vocabulary also allows an `assembly` target for `connects`; a weld
  joins parts, so an assembly target is refused as unsupported (`UNSUPPORTED_GEOMETRY` from the
  command, `UNSUPPORTED_PROPERTY` from loaded content), never accepted with a guessed meaning.
- **Explicit properties.** Size, location and side have no default; omitting one is
  `INVALID_ARGUMENT`. The type defaults to `fillet`, the only supported value; any other type is
  `UNSUPPORTED_GEOMETRY`. Tokens are stored exactly (`steel:weld:type`, `steel:weld:location`,
  `steel:weld:side` as `token`, `steel:weld:sizeMm` as `double`).
- **No body.** A weld has no material and no derived mesh; a `Geometry` mesh under one is refused,
  authored or loaded, and it adds nothing to the view snapshot. Its placement is the identity.
- **The record lives with the weld** (vocabulary section 5.2). Creating a weld writes only the
  weld; the parts are read (lifecycle), so a concurrent delete of a part conflicts with it.
  Deleting a welded part is `DEPENDENCY_CONFLICT` while the weld connects it (`DV-REL-009`, through
  the P08 incoming-link index); deleting the weld removes its own record. Copying a weld is
  `DEPENDENCY_CONFLICT` (`DV-REL-011`); copying a welded part is allowed and leaves the weld on the
  original.
- **Replay is bound to the operation's own revision.** The host ledger proves that an `op_id`
  committed; the request it committed is read from that revision (the weld must be present there
  and absent from the revision before), never from the head, so a later weld with matching
  arguments under a reused `op_id` is `OP_ID_REUSE`, not a replay.
- **Only supported content is interpreted.** `welds` and `create-weld` read kinds only from owned,
  `0.1.0`, `parametric` elements, as the structural validator does; a newer-schema or foreign
  `connection` stays inert and is never decoded as a weld.
- **Stored bytes are verified, not interpreted.** `welds` and `create-weld` run the structural
  validator first, so a damaged `connects` record committed through `commit-revision` is refused
  rather than reported or replayed.
- `copy-element` used to answer `NOT_FOUND` for any source that is not a plate or member; for a
  weld it now answers `DEPENDENCY_CONFLICT` as the vocabulary requires.

## Requirement coverage

Domain vocabulary 0.1.0 rules implemented for `connects`: `DV-REL-001` (dangling target),
`DV-REL-003` (endpoint kinds, narrowed to parts), `DV-REL-004` (cardinality, narrowed to two),
`DV-REL-006` (self-link), `DV-REL-007` (duplicate target), `DV-REL-009` and `DV-REL-011` (delete
and copy with links).

BC-WELD: covered **partially**. A weld object with its own identity, the two parts it joins and
explicitly authored type, size, workshop/site and side. Not covered: weld geometry and volume,
weld length, intermittent welds, preparation and bevels, other weld types, all-around and
contour symbols, NDT and quality classes, welded-assembly rules, drawing symbols and weld
schedules. The F02 requirement texts are a local analysis artifact and are not quoted here.

Provenance: the rules come from the repository's domain-vocabulary contract (sections 2.3, 4.6 and
5.1 to 5.3) and F04; the property names and tokens are original. No external product material was
used.

## Evidence

- `apps/beamcraft-cli/tests/welds.rs` (5 end-to-end tests through the binary): a weld writes only
  `authored.usda` and only its own footprint groups, reads both parts, lists back with every
  property, leaves the parts' bodies and the view unchanged; replay and `OP_ID_REUSE`; delete of a
  welded part and copy of a weld are `DEPENDENCY_CONFLICT`; deleting the welds then the part
  works. Refusals of same part twice, unknown part, assembly or weld as a part, a weld that is its
  own part, a taken identity, each missing property, bad tokens, out-of-range and non-finite sizes,
  an unsupported type and an unknown option write no revision. A tampered one-target record is
  refused on read. An `op_id` reused for another weld is `OP_ID_REUSE`; a newer-schema connection
  beside a supported weld stays inert.
- `crates/layer-engine/tests/authored.rs` adds two layer tests (authoring, model kind, ordered
  targets, no body, reported opinions, atomic refusals, loaded-content cardinality, kind and token
  checks, delete and copy evidence from the element index).
- Unit tests cover the model command, the engine decision and the A01 descriptor pin.

## Not done

- No command renames, edits, moves or deletes a weld other than `delete-element`; no weld in an
  assembly (`part-of` from a connection is in the vocabulary but not in `attach-to-assembly` yet).
- No weld geometry, length or quantity; nothing is exported to IFC or a BOM (E02/E03).
- `bim:discipline` is still not authored (no first-slice element writes it).
- Domain vocabulary 0.1.0 lists `connection` as reserved (`DV-KIND-004`); implementing it needs
  the owner to admit the kind, with the weld attributes, in the next vocabulary version. This PR
  implements it as specified above and records that decision as open.
- The schema descriptor changed: other open PRs that also edit it will conflict and need the
  earlier-descriptor list extended.
