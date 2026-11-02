# A01 assembly relationships

Date: 2026-10-18. Status: **experimental vertical slice; A01 acceptance is limited as listed below.**
Branch `feat/a01-assemblies`, based on `origin/master` `3f9f0f5`.

Plan row: A01 / M, depends on P03, P08 and U03. Result: a minimal assembly hierarchy and a
validator of cycles and ownership. Acceptance: API/CLI create/attach/detach end to end; component
and assembly UUIDs are independent; BC-ASM coverage is listed.

## What is implemented

```text
beamcraft create-assembly --store ROOT --name TEXT [--new-id UUID] [--op-id UUID ...]
beamcraft attach-to-assembly --store ROOT --id PART_UUID --assembly-id UUID [--op-id UUID ...]
beamcraft detach-from-assembly --store ROOT --id PART_UUID [--op-id UUID ...]
beamcraft assemblies --store ROOT
```

| Area | What exists |
|---|---|
| `beamcraft_model::command` | `CreateAssembly { new_id, name }`, `AttachToAssembly { part_id, assembly_id }` (self-link is `INVALID_ARGUMENT`), `DetachFromAssembly { part_id }`, each with a versioned content fingerprint. |
| `beamcraft_engine::assembly` | `Membership` (kinds and `part-of` targets of a model) decides attach and detach: kinds, cardinality, self-link, cycles of any length; returns the chain it read and the footprint. |
| `beamcraft_engine::footprint` | `Footprint::for_part_of(part, chain)`: writes only the part's relationship group, reads the part and every assembly on the chain. `conflicts_with` is now symmetric (see Decisions). |
| `beamcraft_layer_engine::authored` | `Shape::Assembly` (an element of kind `assembly`, model kind `assembly`, no shape inputs, material or `Geometry`, identity placement) and `Change::PartOf { id, assembly }` (explicit one-target `bim:partOf` relationship, or removal). The structural validator adds rule `A01-V01`: target is an active owned assembly, at most one target, no self-link, no cycle (linear walk). The mapping descriptor gains these two lines; layers pinned to the two earlier descriptors stay accepted. |
| `beamcraft_commands::assembly` | `create_assembly`, `attach`, `detach`, `assemblies`: replay by `op_id`, `OP_ID_REUSE`, `DUPLICATE_ID`, `NOT_FOUND`; the layer engine's observed writes are checked against the footprint; commit through U06/U07 with the strict verifier. `delete-element` accepts an assembly. |

## Decisions

- An assembly is a placed element with its own UUIDv7 under `/World/Steel`, a sibling of its
  parts, not their USD parent. Hierarchy is the `bim:partOf` relationship, so prim paths and
  identities never change when membership changes (F04 section 2), and a part's identity is
  independent of its assembly's.
- The record lives with its source (vocabulary section 5.2): attaching or detaching writes only the
  part. The assembly is read; its own chain is read too, because it decides the cycle check.
- One assembly per part (`DV-REL-004`): attaching a member of another assembly is
  `DEPENDENCY_CONFLICT` ("detach first"), never a silent re-parent. Attaching to the same assembly
  again with a new `op_id` is `INVALID_ARGUMENT` (no change).
- A detach authors an explicit empty `bim:partOf` target list rather than removing the spec, so a
  weaker owned layer's record is masked, never revealed. A `Geometry` mesh under an assembly is
  refused on authored candidates and loaded layers alike.
- The layer engine's read set for an attach covers the target's whole chain (with the batch's own
  records applied), matching `Footprint::for_part_of`. `assemblies` and every A01 command run the
  structural validator before interpreting the hierarchy, so a cyclic or wrong-kind graph stored
  through `commit-revision` is refused, not reported or replayed.
- Nested assemblies are allowed (`DVC-REL-005-P`); there is no depth limit in the vocabulary, so the
  walks are bounded by the element count.
- `bim:geometryMode` is `parametric` for an assembly to keep the common base uniform; it has no
  body of its own. Placement is fixed at the identity in this slice, so it cannot disagree with
  the parts' world placements.
- `Footprint::conflicts_with` compared `other.reads` against `self.writes` twice and never
  `other.writes` against `self.reads`, so a command whose read set another command writes did not
  conflict in one direction. The new attach test exposed it; the method now matches its doc
  comment ("one writes an entry the other reads or writes"). No caller relied on the asymmetry.
- Copy refuses a part with a `part-of` record (`DV-REL-011`, already enforced by the layer
  engine's copy plan). Delete refuses an assembly with parts (`DV-REL-009`, the incoming-link
  index of P08); deleting a part removes its own record.

## Requirement coverage

Domain vocabulary 0.1.0 rules implemented: `DV-REL-001` (dangling target), `DV-REL-003` (endpoint
kinds), `DV-REL-004` (cardinality), `DV-REL-005` (cycles), `DV-REL-006` (self-link), `DV-REL-009`
and `DV-REL-011` (delete/copy with links). Cases `DVC-REL-005-P`, `DVC-REL-005-N` and
`DVC-X-REL-02` have direct tests (nested chain, two-cycle, three-cycle).

BC-ASM: the architecture maps BC-ASM-001 and BC-ASM-002 to assembly relationships. A01 covers
them **partially**: an assembly object with its own identity and explicit membership. Main part,
assembly marks/numbering, sub-assembly types, welded/bolted assembly rules and assembly-level
drawings are not covered. The F02 requirement texts are a local analysis artifact and are not
quoted here.

Provenance: the rules come from the repository's domain-vocabulary contract (sections 2.3, 5.1 to
5.3) and F04; no external product material was used.

## Evidence

`apps/beamcraft-cli/tests/assemblies.rs` (3 end-to-end tests through the binary): create, attach,
nest and detach write only `authored.usda`, the footprint writes only the part's relationship
group, the hierarchy reads back, the part's mesh digest is unchanged; refusals of wrong kind,
self-link, second assembly, two- and three-cycles, unknown ids, duplicate id and empty label write
no revision; replay and `OP_ID_REUSE`; delete of an assembly with parts and copy of a member are
`DEPENDENCY_CONFLICT`; deleting the part then the empty assembly works.
`crates/layer-engine/tests/authored.rs` adds the layer test (authoring, model kind, no body,
atomic refusals, two targets in loaded content). Unit tests cover the model commands, the
membership decisions and `for_part_of`.

## Not done

- No rename, move or placement for an assembly; no main part; no numbering, marks or BOM
  (A08/E03).
- No multi-element (assembly) copy; the vocabulary leaves it undefined.
- Replay is decided from the revision of the accepted operation, as for bolts, levels and welds:
  the requested state must hold there and not in the revision before, otherwise `OP_ID_REUSE`. A
  revision at or below the compaction floor cannot be checked and answers `NOT_FOUND`.
- `bim:discipline` is still not authored (no first-slice element writes it); the lifecycle
  attribute is now the boolean `bim:isDeactivated`.
- Domain vocabulary 0.1.0 lists `assembly` as reserved (`DV-KIND-004`); implementing it needs the
  owner to admit the kind in the next vocabulary version. This PR implements the reserved rules as
  specified and records that decision as open.
- The schema descriptor changed: other open PRs that also edit it (for example R01) will conflict
  and need the earlier-descriptor list extended.
