# I05 schedules by attributes

Contributor-original Rust and synthetic fixtures, MPL-2.0.

A schedule is a declarative template over the same verified read that `export-bom` uses: columns
from standard report fields and typed user attributes (I04), one optional element filter,
grouping and totals, rendered as CSV and HTML. The acceptance — unknown and stale shown,
reproducible — means an element never disappears because an attribute is missing, blocked or
stale, and the same store head plus the same template file render byte-identical output.

## Layers

```text
beamcraft_engine::schedule  pure computation: a template and typed rows in, the element table
                            and the group table out, with CSV and HTML rendering and the
                            filter evaluator (strings and cells, no I/O)
beamcraft_commands::schedules    maps one Inspection (plates and members) plus the pinned I04
                            attribute state of the same revision to engine rows; decodes the
                            template and the filter rule
beamcraft schedule-export   CLI command: verified head -> filtered rows -> CSV/HTML files
```

The engine module knows nothing about USD, stores or definitions, mirroring
[the E03 note](e03-bom-reports.md): report rules live there over neutral `CellValue` cells and
are unit-tested headlessly; the adapter fills the cells; the command writes files.

## Template

A template is a JSON file, so a schedule is reproducible from two plain inputs:

```json
{
  "version": 1,
  "columns": [
    {"column": "mark"},
    {"column": "profile"},
    {"attribute": "Paint_status"},
    {"attribute": "Count", "total": true}
  ],
  "filter": {"key": "profile", "op": "eq", "value": "PL10"},
  "group_by": ["profile"],
  "sort_by": ["mark"]
}
```

* `columns` (1..64) selects one `{"column": TOKEN}` standard field or one
  `{"attribute": NAME}` user definition per column, with an optional `"total": true`.
  Standard tokens are the E03 CSV header tokens: `mark`, `element_id`, `kind`, `name`,
  `profile`, `material`, `length_mm`, `area_mm2`, `volume_mm3`, `volume_m3`, `mass_kg`.
* `filter` compares the template's own columns by key. Closed operators: `eq`, `ne`, `lt`,
  `le`, `gt`, `ge`, `contains`, `starts-with`; text admits equality, substring and prefix,
  numbers and integers admit equality and order, booleans admit only equality, and a literal
  of another type refuses. A missing, blocked or stale cell never matches, `ne` included.
  Rules are `all`/`any` groups of comparisons, at most 256 nodes and depth eight, admitted
  against the template before any row is evaluated. This is a deliberately minimal evaluator,
  not a second general rule engine: it shares the template's columns, the schedule's cell
  types and the V14 matching semantics for the cases it covers; full saved-filter parity
  (named rules, view bindings, UUID selectors) stays with V14 and is a separate task.
* `group_by` and `sort_by` reference columns by the same keys. Sorting defaults to
  `element_id`; the element id is always the final tiebreaker, so the order is total. Missing
  cells sort before present values.
* An attribute name resolves against the pinned schema, exactly and case-sensitively. A
  column naming an unknown definition stays in the schedule with empty cells and is listed in
  the command's `unknown_columns` result — shown, never dropped; a `total` on such a column
  refuses, because an unknown column has no values to sum, and a filter naming it refuses.
* `total` admits numeric standard fields and `number`/`integer` attribute definitions only;
  text, choice and boolean columns refuse. Integer totals are exact `i64` sums with an
  overflow refusal; number totals are f64 sums in row order; unknown cells never enter a sum
  and are counted next to it, `mass_kg` included. There is no unit conversion, as in I04.

## Unknown and stale shown

Only active engineering elements of the BOM universe (plates, beams, columns of the verified
head) are scheduled. Within that universe a row is never dropped:

* A missing, blocked or undecodable value renders as an empty cell — never zero, never a
  default. This covers stale values in both forms the storage produces: a value left by an
  undefined definition, and a stored value whose type no longer matches its definition (for
  example after the definition was undefined and redefined with another type).
* Every totalled column counts its unknown cells; the group table carries `<label>_unknown`
  next to each total, the generalization of the BOM's `mass_unknown`.
* An unknown attribute definition shows up as the named empty column plus the
  `unknown_columns` list in the JSON result.

## Reproducibility

The element table sorts by the template's `sort_by` columns with the element id as the final
tiebreaker; groups follow first-seen order of the sorted rows and group keys are the rendered
cell strings, so equal labels group together without float-equality traps. Output carries no
timestamps, tool name or version. Numbers render in the shortest round-trip form with a `.` and
no separators, as the BOM does. Same head, same template file: byte-identical CSV and HTML.

## Files

`elements` file — one row per element, header = column labels (an attribute may repeat a
standard token; values still differ per column):

```csv
mark,profile,Paint_status
```

`summary` file — one row per group: the group columns, `count`, and for each totalled column
its sum (empty when no known value contributed) and `<label>_unknown`.

The HTML file renders both tables of the same schedule as one deterministic document: fixed
headings, plain tables, the five HTML entities escaped, no styles or scripts.

All files follow the E03 write discipline: everything is rendered before the first file is
created, every path is reserved with `create_new` before any bytes are written, and a refusal
on any path leaves nothing on disk.

## Command

```text
beamcraft schedule-export --store ROOT --template FILE.json --out SCHEDULE.csv
                          [--summary GROUPS.csv] [--html SCHEDULE.html]
```

It reads and verifies the head exactly like `export-bom` (a store without a revision refuses;
rows cap at `MAX_ELEMENTS`). The attribute state is pinned first and the inspection must name
the same revision, so quantities and attribute values are one head. The JSON result names the
revision, row and group counts, `unknown_columns` and the written files.

## Not done

Named templates saved in the personal store, grand-total rows, bolts/welds/assembly-level
schedules, more renderers, any unit conversion (I04 has none) and full V14 filter parity
(named saved rules, view bindings, selectors beyond the template's own columns) are
follow-ups. The property editor and native UI face are separate tasks.

## Evidence

`cargo test -p beamcraft-engine schedule` (template admission, sorting determinism, grouping,
totals with unknown counting, exact integer sums, filter admission and matching, CSV/HTML
escaping and byte-determinism), `cargo test -p beamcraft-cli --test cli schedules::` (real
binary: a store with members, plates, definitions and values; a filter narrows rows; grouping
and totals match the element table; an unknown attribute column stays and is named; a stale
value after redefinition is an empty counted cell next to a fresh value; two runs render
byte-identical files; refusals: no revision, unknown standard token, total on text, filter
naming an unknown column or carrying a wrong literal, existing output file, unknown option).
