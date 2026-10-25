# E02: minimal IFC export

Date: 2026-10-19. Status: **experimental vertical slice; E02 acceptance is limited as listed
below.** Branch `feat/e02-ifc-export`, based on `origin/master` `11a2139`.

This implements the writer, the headless command and the report of the E01 contract
([`docs/contracts/e01-ifc-export-v0.1.md`](../contracts/e01-ifc-export-v0.1.md), decisions Q-1 to Q-13
accepted, dependencies D1 and D2 in `master`). Behaviour comes only from that contract and the
public buildingSMART IFC4 specification; no code, schema listing, example file or naming list of
any IFC library or commercial product was used (`AGENTS.md` sections 1 and 2).

## What is implemented

```text
beamcraft export-ifc --store ROOT --out FILE [--revision UUID] [--project-name TEXT]
                     [--schema IFC4] [--format step] [--strict] [--overwrite] [--report FILE]
beamcraft ifc-guid --id UUID | --decode GLOBALID
```

| Piece | Where | What |
|---|---|---|
| Pure writer | new crate `beamcraft-ifc` (`crates/ifc`, native, layer 2, depends on `beamcraft-model` only, no third-party dependency, no file system) | `guid` (the 22-character GlobalId, its decoder and the synthetic ids), `step` (shortest REAL text per type, string escapes, UTC time stamp, bounded hashing sink), `model` (plain input and bounds), `write_ifc` (fixed entity templates in the order of contract section 11.2) |
| Structural reader | `beamcraft_ifc::verify` | parses a written file and checks gapless instance numbers, resolved references, valid unique GlobalIds, non-empty relationships, and recomputes the volume and closed-shell property of every face set from the file alone. A test harness, not an importer (`IFCX-SCOPE-003`): no command uses it |
| Atomic output | `beamcraft_host::output` (`stage_file`, `Staged::install`, `publish_file`, `resolve_destination`) | validates the destination (local path only: no UNC or device path on Windows; no symbolic link or reparse point at the name or at any directory above it), writes a temporary file in the destination directory and syncs it, and installs it only on request: a non-replacing hard link (rename only with `--overwrite`). A staged file that is dropped leaves nothing behind; an installed one can be taken back (`Installed::undo`) |
| Adapter | `beamcraft_cli::export` | opens the head or an explicit revision, runs the existing verified reader (`plate::inspect_revision`: structural validation, kernel re-evaluation of every plate, beam and column, mesh and quantity comparison), resolves the `ExportModel`, applies the strict rule, publishes the file and builds the report |
| Command | `export-ifc`, `ifc-guid` in `apps/beamcraft-cli` | parsing and JSON only |

Mapping (all from the contract): plate, beam and column become `IfcPlate`, `IfcBeam`, `IfcColumn`
with the kernel mesh as one `IfcTriangulatedFaceSet` (`Closed` true, 1-based indices, element-local
millimetres, binary32 text, placement relative to the building); levels become
`IfcBuildingStorey`; one synthetic site and building; the pinned material becomes `IfcMaterial`
(grade or `Unspecified`, standard, digest property, density only when authored); published current
quantities become `Qto_*BaseQuantities` (plate mass as properties); `BeamCraft_Element` and
`BeamCraft_Declared` property sets; project units with millimetres and no scaling; deactivated
elements are not exported; every other kind is an `IfcBuildingElementProxy`.

## Decisions and deviations from the contract (for the owner)

1. **Assemblies, welds and references are proxies.** The contract has no mapping for them
   (section 2.3 excludes assemblies and connections), so the E01 rule for kinds without a mapping
   applies: `IfcBuildingElementProxy`, listed with `IFC_KIND_PROXY`. They have no body, so no
   `IFC_GEOMETRY_ABSENT` is raised for them (that finding, which is fatal in strict mode, is for a
   plate, beam or column that loses its mesh). **Additive amendment proposed:** a part of an
   assembly carries the property `AssemblyId` (the assembly's ElementId) in `BeamCraft_Element`,
   so membership is not lost (L-09 is narrowed). No `IfcElementAssembly` and no `IfcRelAggregates`
   between elements are written; a weld's connection record is not exported. If the owner wants a
   real IFC assembly, that needs a contract change (new entities, rules and cases).
2. **The adapter is in the CLI, not in the engine.** The contract proposed
   `beamcraft_engine::export` and a `beamcraft-engine` to `beamcraft-ifc` edge. The committed
   records (`Inspection`, stored quantity reports, definitions) are CLI adapter types in every
   earlier slice (P01 to D2), so the adapter sits there and the engine gains no edge. The module
   table gains the `beamcraft-ifc` row (layer 2, `beamcraft-model` only; the contract said `geom`
   and `model`, but nothing uses `geom`) and the CLI row gains `beamcraft-ifc`. `deny.toml` has
   the package exception. The contract areas table gains two rows.
3. **Levels are storeys, not proxies.** The D1/D2 note decided that a level is an element of kind
   `level`; the adapter lists it as `IfcBuildingStorey` and not among the elements.
4. **A refusal is the CLI's common rejection shape** (`outcome: rejected`, F04 `code`, exit 2) with
   `diagnostics: [{"ifc_code": "IFC_...", "status": "failed" | "incomplete"}]`. `incomplete` is
   used for `DEPENDENCY_UNAVAILABLE` (a revision that cannot be read, or one with no recorded
   ProjectId). A successful report has `primary_code: null` and `incomplete: false`. A refused
   export leaves no file.
5. **The report is printed by the command itself**, not through `print_json`: that path enforces
   the 65,536-token control-message bound, and the per-element table of a large model would turn a
   successful export into `LIMIT_EXCEEDED` after its file exists. The renderer writes sorted keys,
   no whitespace, no path, host name or time (`IFCX-SURF-002`); `--report FILE` writes the same
   text through the same atomic output.
6. **Mesh source.** The face set is the kernel's mesh converted to the stored binary32 form
   (`plate::kernel_mesh`). The verified reader has already required that mesh to equal the stored
   derived mesh (`IFCX-GEO-005`), so a layer edited behind BeamCraft's back refuses the export with
   `INTEGRITY_ERROR` instead of reaching the file.
7. **Explicit revision.** `--revision UUID` searches the head log from the head downwards by
   `RevisionId` and opens that sequence; no match is `NOT_FOUND`, a malformed id `INVALID_ID`.
8. **`--strict` and `--overwrite` take an optional `true` or `false`**; a bare flag means true.
   `--project-id` does not exist and is `INVALID_ARGUMENT` (`IFCX-REQ-002`).
9. **`ifc-guid` is implemented** (section 15.4); the proposed `ifc_global_id` field of
   `inspect-elements` is not.
10. **Unreachable branches are unit-tested, not end to end.** The structural validator refuses a
    transform that is not the translate-and-orient pose, and the revision reader refuses a
    plate, beam or column the kernel cannot evaluate, so `IFC_PLACEMENT_ABSENT` and
    `IFC_GEOMETRY_ABSENT` cannot be produced from a real store yet (A02 will add non-rectangular
    plates). Their rule, the strict-mode refusal order and the pose reader have unit tests.

11. **Destinations are checked before anything is created** (Copilot review of the first
    version). An output, the IFC or the `--report` file, may not lie inside the store it is
    exported from (a file there can break the revision log and `--overwrite` could replace a
    record or blob), and the two outputs may not be the same file (compared by canonical
    directory and name, ignoring case where the file system usually does). The IFC and the report
    are both staged, then installed; if the report cannot be written the IFC is not installed
    (and an IFC that was installed an instant earlier by this command is removed again if it was
    newly created; a replaced file cannot be restored).
12. **The report's level id is the GlobalId of the written storey** (`storey_global_id`: a
    synthetic id of the ProjectId and the LevelId), so a consumer can match it to the file.
13. **Revision lookup reads the head log once** (`RevisionStore::sequence_of`) instead of
    re-scanning the directory for every sequence.
14. **A proxy keeps its published mesh.** The mesh of an unmapped kind (valid triangles, valid
    indices, finite points) is exported with `Closed` unset when the element has a rigid pose,
    and is not re-verified (the kernel does not know the kind). No store built with the CLI
    holds such an element yet (every owned kind with a body is a plate, beam or column), so this
    is covered by the unit tests of the mesh reader and the pose rules, not end to end.

## Requirement coverage

Covered by tests (writer tests in `crates/ifc`, end to end in `apps/beamcraft-cli/tests/export_ifc.rs`
through the real binary): the encoder, decoder and synthetic-id vectors of section 3 (all 11 and
`V-SYN-01`/`V-SYN-02` reproduced with the in-repo SHA-256); REAL, string and time-stamp rules;
determinism (same bytes for the same revision, header time from the RevisionId); structure of the
file by the own reader; the volume recomputed from every face set against the stored volume;
placement relative to the building; storey containment and building containment; optional entities
only with content (`IFCX-EMIT-001`); material grade, `Unspecified`, density; quantity sets and the
plate mass as properties; gross values only without holes; unknown, stale and missing quantities
omitted, listed and never zero; deactivated elements; a copy gets its own GlobalId; GlobalIds
survive a rename; export changes nothing in the store and writes no `ifcGlobalId`; refusals
(version, schema, format, ids, unknown revision, no revision, no recorded ProjectId, integrity,
existing output, links, missing directory, unknown or repeated option, empty project name); strict
mode not refusing proxies, omissions or truncated names; labels over 255 characters cut with the
full text in `Label`; the report equals the `--report` file and is bounded by no control limit.

## Not done

* **Independent validation (E01 Q-8, `IFCX-VAL-001`).** Not run. Altiro Desktop is not admitted
  (owner decision) and no external program was run. The file has been checked only by this
  repository's own reader. **The IFC4 attribute lists, enumerations and string escapes are written
  from the public schema as the contract describes it; the buildingSMART pages in section 18 were
  not opened by the contract's author or by this task, so a stricter consumer may still reject the
  file.** L-14 (escapes) and L-15 (no model view) remain open, and so does the Reference View check
  of contract section 19.3 (c).
* Cross-platform byte identity was not run (Linux only). Members with a roll depend on `sin` and
  `cos` (`IFCX-DET-007`); `export-ifc` has not been exercised with a nonzero roll.
* No `ifc_global_id` field in `inspect-elements`; no export by selection (contract 17.3); no
  `IfcElementAssembly`, weld, bolt or hole entities.
* The cases of the contract's case file are not mapped one by one; the ones above are covered by
  the tests, and the rest (limits at their real sizes, a 300-proxy store, cancellation) are not.
  The size bounds are tested with small limits.
* `IFC_CANCELLED` and `IFCX-ERR-004` are not implemented: the command has no cancellation point.
