# E04: IFC reference conversion study — the minimum input contract

Date: 2026-11-05. Status: **input-side slice; the conversion itself is not implemented and is
blocked on owner decisions recorded in
[Apibim-com/BeamCraft#527](https://github.com/Apibim-com/BeamCraft/issues/527)**.

The owner requires a read-only IFC reference workflow in the MVP (decision of 2026-10-27) and
separately a native editable steel IFC import. This slice serves only the first: it never makes
a reference editable, never builds native elements, and leaves item import (I01–I03) untouched.
The workflow contract is
[`docs/contracts/f12-external-reference-workflow-v0.1.md`](../contracts/f12-external-reference-workflow-v0.1.md);
XR-SOURCE-002 pins the conversion record facts, and this slice delivers the *input* half of
those facts — what any future converter consumes and is audited against.

## What is implemented

```text
beamcraft ifc-inspect --source FILE
```

| Piece | Where | What |
|---|---|---|
| Bounded scanner | `beamcraft_ifc::read` (new module of the E02 crate; still std-only, `beamcraft-model` only) | `inspect(bytes) -> Inspection` with the pins and losses below; lenient framing for received files (CRLF, gaps in instance numbers, blank lines, several DATA sections); linear time; every allocation bounded by module constants |
| Command | `ifc-inspect` (family `ifc`, route `Ifc`, access `read`) | reads one file through the shared bounded reader (`source_file::read_file`, 32 MiB cap) and prints one JSON object; refuses on the common shape, exit 2 |
| Tests | `crates/ifc/src/read.rs` (unit), `apps/beamcraft-cli/tests/ifc_inspect.rs` (end to end through the real binary) | synthetic contributor-original IFC text only; nothing from any commercial product (`AGENTS.md` §1, §2) |

## The minimum input contract (executable)

The same bytes always answer with the same JSON. Refusal cases stop before any inventory.

| Fact or rule | Where | Notes |
|---|---|---|
| Original-byte SHA-256 and size | `source.sha256`, `source.bytes` | XR-SOURCE-002; the received filename stays descriptive (`header.file_name`) |
| Input cap | 32 MiB (`read::MAX_INPUT_BYTES`) | the same ceiling as a stored foreign source (`model::reference::MAX_SOURCE_BYTES`), because the received IFC bytes are the source evidence the workflow retains |
| Framing refusal | `NOT_A_STEP_FILE` (`INVALID_ARGUMENT`) | bytes that are not ASCII text at all, that do not start `ISO-10303-21;`/`HEADER;`, or that end without `ENDSEC;` and `END-ISO-10303-21;` — a truncated or undecodable delivery is refused, never pinned |
| Declared schema | `declared_schema.token`, `.recognized` | recognized means the IFC4 subset this crate writes and studies; any other declared token (IFC2X3, IFC4X3, …) is a `SCHEMA_OUTSIDE_STUDIED_SET` loss, not a refusal — the supported subset is an owner decision |
| Descriptive header | `header.{file_name,time_stamp,preprocessor_version,originating_system}` | `FILE_NAME` facts; omitted as a loss when unreadable or over 256 bytes; never version identity |
| Entity inventory | `entities.{count,names,unclassified,non_instance_lines}` | one instance per line is interpreted; multi-line instances are counted as unclassifiable lines, never guessed |
| Identity inventory | `global_ids.{count,distinct,duplicates,duplicate_samples,project_without_global_id}` | an `IfcRoot`-shaped GlobalId is a 22-character first-attribute string that decodes; duplicates are listed with the instance number and block nothing yet — matching across deliveries (XR-IDENTITY-001) is converter-level work |
| Declared units | `length_unit.{meters_per_unit,origin}` | the `IfcProject` → `IfcUnitAssignment` → `IfcSIUnit` chain in metres per unit (`project_units`); when there is no project or its chain is unreadable, the only distinct SI length unit present (`si_units_only`); when the declaration resolves but holds no SI length unit, there is **no fallback** — the declaration is authoritative and the gap is a loss |
| Non-SI units | `conversion_based_length_units` | names only (e.g. `INCH`); their factors are not interpreted here |
| Losses | `losses[].{code,detail}`, `losses_truncated` | capped at 64 entries; every deviation (non-ASCII bytes, BOM, multiple DATA sections, repeated instance numbers, dangling unit references, unknown units, schema outside the studied set, missing project, overlong lines) is counted or listed, never guessed away |

Refusals: `INVALID_ARGUMENT` (not an ISO 10303-21 physical file; URL or network path on its
text alone), `NOT_FOUND` (unreadable), `LIMIT_EXCEEDED` (over the cap). The command opens no
store and changes nothing; the same head-less answer is a pure function of the bytes.

## What is deliberately not here

- **No conversion, no converter, no USD output.** The F12 workflow converts the supported
  subset into an immutable USD reference root; which converter, which licence, which schema
  versions and which entity families are the owner decisions this study records as questions
  in the upstream issue. The scan above is the audit basis a converter will be pinned to.
- **No native import.** The separate editable-import requirement (2026-10-27 decision) needs
  its own contract; nothing here touches the kernel or the element model.
- **No IFC authoring changes.** The E02 writer and the E01 contract are unchanged; the new
  module shares the crate's lexical helpers only.
- **No identity matching across deliveries.** XR-IDENTITY-001 lineage rules stay
  unimplemented; the inventory only makes duplicates and missing identifiers visible.

## Limits

- One instance per DATA line: the scanner reads instance heads and the attributes of the few
  interpreted entities (`IfcProject`, `IfcSIUnit`, `IfcUnitAssignment`,
  `IfcConversionBasedUnit`). Instances split over several lines are counted as
  `non_instance_lines`, so a file in that form inventories partially and says so.
- GlobalId candidates are first-attribute strings of the exact 22-character form; a property
  name of that shape would also count. The class-level distinction needs the schema and is a
  converter-level concern.
- Units: SI length units only, with the schema's prefix factors. Per-representation contexts,
  true north and georeferencing are not interpreted.
