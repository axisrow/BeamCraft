# USA profile pack: S and C shapes of ASTM A6/A6M-05a

Date: 2026-11-25. Status: the second slice of the USA profiles (fork issue #708), same route
as the W and HP slice ([`usa-astm-a6-profiles.md`](usa-astm-a6-profiles.md)). It closes the
slice's blocker — the slope and radius convention of the sloped-flange families — from the
admitted sources themselves, under the owner's standing analogy rule: the S and C rows stand
on the same footing as the landed W and HP rows.

## Problem

The W and HP slice left the S, M, C and MC shapes of the same annex out: the `tapered_i` and
`tapered_c` families need an inner-flange slope `k` and radii `r`, `r1`, and the data-source decision memo
of 2026-11-23 (its §2.3) read A6 as printing "no radii; slope only as terminology". Reading
the admitted sources for the S and C tables answers the question without a new decision: the
slope is printed, for S and C exactly.

## Sources

| Source | Edition or revision | Where | Retrieved | Role |
|---|---|---|---|---|
| ASTM A6/A6M-05a, Annex A2, tables A2.2 (S) and A2.5 (C), with the terminology of 3.1.2.5 and 3.1.2.7 | approved 1 September 2005 | <https://www.holapl.com/wp-content/uploads/2025/06/ASTM-A-6-A-6M-05a.pdf> (a scan with a text layer) | 2026-11-25 | the declared edition: every shipped value and the slope statement |
| ASTM A6/A6M-05, Annex A2 | approved 1 April 2005 | <https://www.womicsteel.com/uploads/ASTM-A-36A-36M_Standard-Specification-for-Standard-Specification-for-General-Requirements-for-Rolled-Structural-Steel-BarsPlates-Shapes-and-Sheet-Piling.pdf> (the typeset text) | 2026-11-25 | the second reading of every row |
| Nucor-Yamato Steel Company, Structural Shapes | cover and general notes dated April 2023; the section-size list and the dimension pages are stamped March 2024 | <https://nucoryamato.com/staticdata/catalog.pdf> | 2026-11-25 | the root radius; the row cross-check; the same slope statement |

The same two public copies of the standard that carried the W and HP rows carry the S and C
tables: the 05a revision reprints Annex A2 of the 05 edition unchanged, and the copies agree
value for value on all 59 rows, twelve printed values each. The edition question, the newer
assortment and the search for a newer public copy are the W and HP note's to explain; nothing
changed here. The catalog's general notes (April 2023) print, for S and C, the same slope the
standard's terminology prints: "The inside flange surfaces of S and C shapes have approximately
a 16-2/3 % slope."

## The slope convention

The standard's terminology (3.1.2.5) defines "S" shapes as "doubly-symmetric beam shapes with
inside flange surfaces that have a slope of approximately 16 2/3 %", and (3.1.2.7) "C" shapes
as "channels with inside flange surfaces that have a slope of approximately 16 2/3 %". 16 2/3
percent is 1/6, the rise-over-run `k` the families store; the GB/T 706 pack stores the same
slope for its 1:6 series as `0.16666666666666666`, and every row here carries that value. The
word "approximately" is the standard's own rounding of 1:6, not an uncertainty this pack could
resolve better: no admitted source prints a per-row slope for S or C, and a value derived from
the catalog's detailing distances (T, k, k1) would be a derived value, which the pack
discipline refuses ("no radius is derived or interpolated", the W and HP note says of the same
catalog).

## What ships

`environments/usa/profiles-astm-a6-s-c.usda`: 59 entries, 28 S shapes of table A2.2 as
`tapered_i` rows and 31 C shapes of table A2.5 as `tapered_c` rows, each declaring
`steel:profile:standard = "ASTM A6/A6M-05a"`. The family mapping is the model's own:
`tapered_i` is the inclined-flange I family (GB/T 706 工字钢, GOST 8239, "ASTM S style", the
`section.rs` doc says), `tapered_c` the inclined-flange channel family. `s` is the web
thickness `tw` of the SI column, `t` the flange thickness `tf`, which the A6 SI column prints
for S and C as the mean thickness at the middle of the leg, exactly what the families want.

- **Values.** Depth, flange width, web thickness, flange thickness and area are the
  standard's own SI column. Nothing is converted and no rounding rule is BeamCraft's. The
  inch and SI readings agree through the unit ratios: depth and width within 1.2 mm (three C
  rows exceed one millimetre — C3X5 at 1.05, C10X15.3 and C10X30 at 1.04 — the SI column's
  own rounding, not a reading), thicknesses within 0.15 mm, areas within one percent.
- **Mass.** `steel:profile:massPerMeterKg` is the nominal SI mass of the row's metric
  designation (S 610 X 180 gives 180), the reference field it always was.
- **Designations.** The EDI inch form without spaces: `S24X121`, `C15X50`, with the decimal
  of the inch weight where the standard prints one (`S4X9.5`, `C4X5.4`). The library has no
  alias field, so the metric designation of the same row (`S610X180`, `C380X74`) does not
  resolve; the usa pack names its rows in the inch form only.
- **Root radius.** The standard prints none. `steel:profile:rMm` is the catalog's fillet
  radius R for the 17 S and 6 C shapes it rolls (the metric dimension pages print R in
  millimetres: 19 mm on the S 610 and S 510 rows, 14 on S 460 and S 310 X 74/60.7, 13 on
  S 380 and C 380, 11 on S 310 X 52/47.3, 10 on C 310) and 0, a sharp corner, for the 36
  shapes its size list does not carry: the S 10, S 8, S 6, S 5, S 4 and S 3 groups and every
  C group below C 15 except C 12. No radius is derived or interpolated; the catalog's metric
  table also cross-checks the shared rows (area, depth, both thicknesses, width agree with
  the standard's SI column on all 23).
- **Toe radius.** No admitted source prints one: the catalog's symbols page defines only R
  (the fillet radius), and its T, k and k1 are detailing distances with no field in the
  families. Every row keeps a sharp toe (`r1Mm = 0`), the same simplification the parallel
  families make where no radius is printed.

## The declared-area warning is expected here

27 of the 59 rows warn (10 S, 17 C) — twenty-six of them sharp-corner rows, where the
standard's printed area carries the real root fillet and the toe rounding the row omits, and
S20X86, whose closed form with the catalog's theoretical root radius sits just over one
percent from the declared area. The SI column's rounding contributes on every one of them.
The widest is C3X3.5 at 4.2 percent; the widest S row is S3X5.7 at 3.0 percent; no row
reaches 5 percent. The owner decision of 2026-10-18 stands: a difference above one percent
is a warning, never a refusal, and the quantities use the declared catalogue area. The tests
pin the count (27) and refuse any row beyond the bound.

## What stays NOT_FOUND, and why

- **M shapes** (table A2.3, 22 rows with SI values): the annex lists them, but the model's
  `tapered_i` family needs the slope, and the standard's terminology gives the M shapes no
  slope at all — 3.1.2.6 defines them only as "doubly-symmetric shapes that cannot be
  classified as W, S, or HP shapes" — and the Nucor-Yamato catalog rolls none (its size list
  and AISC's sloped-flange flag agree M shapes are inclined-flange, but neither prints a
  value). Storing `k = 0` would be false parallel-flange geometry and 1/6 would be an
  unsourced value, so the pack ships no M row. The blocker is a source gap, not a model gap.
- **MC shapes** (table A2.6, 41 rows): the catalog's general notes state "The slope of the
  inside flange surfaces of MC shapes varies", and neither the standard nor the catalog
  prints a per-row slope. The same reasoning as for M applies; the pack ships no MC row.
- **L shapes** (tables A2.7 and A2.8): NOT_FOUND in this slice, for a different reason than
  the first-slice note guessed. The model does admit angles — the `angle` family (`h, b, t,
  r`) has been in `section.rs` all along, and A6 prints exactly what the family needs (size,
  thickness, mass, area) with the catalog covering R for the angles it rolls. The shapes are
  shippable and are the next profiles slice; they wait because the designation form needs its
  own decision (the inch sizes print fractional thicknesses: `L8 X 8 X 1-1/8`) and the two
  tables are a ~140-row verification effort of their own. The first-slice note's "the sloped-
  flange families need a slope and radii convention decision" was about S, M, C and MC; for L
  the premise "no model family admits angles" was wrong and is corrected here.
- **HSS and pipe, tees, double angles**: not in Annex A2; no admitted source (unchanged).
- **The 27 shapes of newer editions** the Nucor-Yamato size list names (unchanged); the
  edition-bump path is the W and HP note's.

**Edition-bump path.** As before: a newer openly readable edition of the standard enters as
its own pack under the standing pack discipline and never edits these rows in place.

## Verification

- Two independent public copies of the standard (a typeset text and a scan) agree on all 59
  rows in both unit columns, and on the SI designation of every row.
- The catalog's metric dimension table equals the standard's SI values on all 23 shared rows
  (S 610 to S 310, C 380 and C 310), and its size list settles the rolled/sharp split.
- `beamcraft-commands` `library::tests::the_shipped_usa_s_and_c_library_reads_with_every_
  entry_admitted`: every entry admitted, both families with their full assortments, the slope
  1/6 and the zero toe on every row, the rolled/sharp radii split, spot values, the warning
  count and bound, the nominal mass against the area, and the refused designations (M, MC, L,
  the metric forms, other spellings).
- `beamcraft-cli` `tests/usa_environment.rs`: the listing of two profile libraries on two
  pages with provenance, the S and C rows and their warnings on named designations, and a
  project created from the environment resolving an S beam and a C column through the project
  copy while the M, MC and L designations stay refused.

## Not in this slice

- **L shapes**: the next profiles slice (designation-form decision first).
- **M and MC shapes**: a source that prints their per-row slope, or an owner convention
  declaring one, reopens them.
- **HSS and pipe, tees, double angles; the 27 shapes of newer editions; fasteners**: as the
  W and HP note records.
