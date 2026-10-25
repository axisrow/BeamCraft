# E03 bill of materials and reports

Date: 2026-10-19. Status: **experimental vertical slice.** Branch `feat/bom-reports`, based on
`origin/master` `11a2139`.

Plan row: E03. Result: a verified store's head revision exports a bill of materials — one
per-element table and one summary table — as CSV files from the CLI, backed by a pure engine
API. IFC export (E02) is a separate task; plain-text and HTML reports stay open.

## 1. What exists today

* `beamcraft_cli::plate::inspect` opens and verifies the head revision, re-evaluates every plate
  and member with the kernel, and reports `PlateRecord`/`MemberRecord` (authored inputs, bodies,
  re-evaluated `Quantities`), the definitions by digest, the levels and the stored-quantity
  freshness. This is the same verified read every other reporting command uses.
* Quantities (P04): volume is always `current` after a re-evaluation; mass is `current` only
  when the element pins a material whose definition declares a density, and is `unknown`
  otherwise. Density is never guessed, and a designation never selects a number (A07.2).
* Marks (A08/A09): a plate, beam or column may carry an approved mark, stored by
  `approve-numbering` as `steel:numbering:series`, `steel:numbering:number` and
  `steel:numbering:basis`; equivalent parts share a mark, and `inspect-numbering` lists them.
  Not every element is numbered, and every element has its full UUIDv7 and an optional user
  label (`bim:name`).
* A member pins a profile definition (rectangle, an A07.1 family or a custom outline) that may
  declare a designation and section numbers; a plate has no profile definition, only its contour
  and thickness. A definition may declare `mass_per_meter_kg`, but the A07.2 note leaves its use
  to a follow-up: this slice does not change how mass is computed.

## 2. Design

### 2.1 Layers

```text
beamcraft_engine::bom      pure computation: rows in, sorted element table and grouped
                           summary out; CSV rendering of both (strings, no I/O)
beamcraft_cli::bom         maps an Inspection to engine rows (labels, derived areas)
beamcraft export-bom       CLI command: verified head -> rows -> CSV files
```

The engine module knows nothing about USD, stores or definitions. Its input is one neutral row
per element (`BomRow`: id, kind, name, profile label, material label, length, area, volume,
mass); its output is the sorted per-element table, the grouped summary and the CSV text. All
report rules (sort order, grouping, unknown handling, rounding) live there and are unit-tested
headlessly; the adapter only fills the rows from a verified revision, and the command only
writes files. This is the `AGENTS.md` section 3 order: engine API, then CLI, then end-to-end.

### 2.2 Which quantities feed the report

The report reads the engine's re-evaluated quantities, not the stored ones. A stored record is
an output of a previous evaluation; the verifier of `inspect` has already checked every mesh
against the kernel, and the quantities the engine computes at read time *are* the `current`
values by construction. The `current`-only rule of the issue therefore holds without a second
code path: no stale or missing stored value can enter the report, and `inspect-elements` remains
the command that reports stored-quantity freshness.

* `volume_mm3` — always current. Reported as stored, in mm3, and as m3.
* `mass_kg` — reported only when current (`Some`). Unknown mass is never zero: the CSV cell is
  empty, the element is excluded from every mass total, and the summary counts it in a
  `mass_unknown` column so the exclusion is visible. A group whose mass is entirely unknown
  shows an empty `total_mass_kg`.
* Declared `mass_per_meter_kg` of a profile definition is **not** read (A07.2 follow-up).

### 2.3 Derived columns

No new geometry is computed. Two columns are divisions of already-current quantities, exact for
the stored algorithms (a member's volume is its section area times its axis length; a plate's
volume is its net face area times its thickness, holes included):

* `length_mm` — members only: the length of the authored axis `p1..p2`.
* `area_mm2` — members: `volume_mm3 / length_mm` (the detailed outline's net area); plates:
  `volume_mm3 / thickness_mm` (the net face area). Empty when the denominator is not positive.

Dimensions (`width`/`length`/`thickness`, `width`/`height` of a section's bounding box) are
reported from the bodies, not re-derived.

### 2.4 Marks

The `mark` column carries the element's stored mark when it has one (assigned or approved —
both statuses carry a value; the approval is A09's workflow, not the report's), and falls back
to the element id (full canonical UUIDv7) otherwise, exactly as the issue's fallback requires.
The `element_id` column is written next to it, so a consumer always has the identity whatever
the numbering state; the engine API carries both (`mark: String`, `id`).

### 2.5 Labels

* `profile` — members: the pinned profile definition's designation, else its display name, else
  the digest (last resort, honest, never invented). Plates have no profile: the cell is `PL`
  followed by the thickness in millimetres (`PL10`), the conventional plate label, derived —
  not stored.
* `material` — the pinned material definition's grade, else its display name, else the digest;
  empty when the element pins no material.
* `name` — the user label as authored.

### 2.6 Sorting and grouping

* Element table: kind in registry order (`plate`, `beam`, `column`), then profile label, then
  mark. Deterministic, and it keeps same-profile parts together for the shop.
* Summary: group key `(kind, profile label, material label)`; for a plate the thickness inside
  the `PL` label is the group's profile part, so plates group by thickness and grade. Columns:
  `count`, `total_length_mm` (members), `total_area_m2`, `total_volume_m3`, `total_mass_kg`
  (current masses only), `mass_unknown` (elements of the group with unknown mass). Same order
  as the element table.

### 2.7 CSV schema

Two files, one per table, so each file has a single header row and machine readers need no
section logic. RFC 4180 quoting (a field is quoted when it contains `,` `"` CR or LF); the
labels come from definitions, whose value caps bound every field, so no unbounded cell exists.
`.` decimal separator, no thousands separators, shortest round-trip formatting.

`elements` file:

```csv
mark,element_id,kind,name,profile,material,length_mm,area_mm2,volume_mm3,volume_m3,mass_kg
```

`summary` file:

```csv
kind,profile,material,count,total_length_mm,total_area_m2,total_volume_m3,total_mass_kg,mass_unknown
```

### 2.8 Command

```text
beamcraft export-bom --store ROOT --out ELEMENTS_CSV [--summary SUMMARY_CSV]
```

* Reads and verifies the head exactly like `inspect-elements`; a store without a revision is
  refused. The JSON result names the revision, the row and group counts and the files.
* Files are created new (`create_new`), never overwritten, synced, as `export-view` does;
  `BackendUnavailable` when writing fails. Both files are reserved before any bytes are
  written, so a refusal on either path — an existing or uncreatable summary included, and the
  same path given twice — leaves nothing on disk; the files reserved so far are removed again.
* The engine caps rows at the snapshot's element bound (`MAX_ELEMENTS`); a model beyond the
  view contract's own bound is `LIMIT_EXCEEDED` before any file is written.

### 2.9 Not done

* `mass_per_meter_kg` as a mass source; level, assembly and mark-status columns;
  plain-text/HTML rendering; more formats; a report catalogue in the store. Each is a follow-up
  that extends `BomRow` or adds a renderer next to the CSV one.

## 3. Evidence

`cargo test -p beamcraft-engine bom` (sort order, grouping, unknown mass, derived areas, CSV
quoting), `cargo test -p beamcraft-cli --test bom` (real binary: a store with a material, an
I-section with designation, plates and members exports both files; the rows match
`inspect-elements`; the mass of an element without a material is an empty cell, counted in
`mass_unknown`, and excluded from totals; a beam numbered by `approve-numbering` reports its mark,
an unnumbered column falls back to `mark` equal to `element_id`; refusals: no revision,
existing output file, unknown option, a colliding or identical summary path).
