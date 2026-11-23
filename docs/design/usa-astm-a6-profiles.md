# USA profile pack: W and HP shapes of ASTM A6/A6M-05a

Date: 2026-11-23. Status: experimental slice; the first slice of the USA profiles (fork issue
#708). It closes the issue's profiles-geometry blocker and answers its inch-to-millimetre
question (question 6). Owner decisions of 2026-11-23 in [decisions](../plan/decisions.md).

## Problem

The `usa` environment shipped material grades only. Its profile pack waited on two owner
decisions: whether the AISC Shapes Database could be a source, and how published inch values
were to become millimetres. Both are settled: the AISC Shapes Database and the AISC Manual
tables are not an admitted source in any form, and no pack converts or rounds anything.

## Sources

| Source | Edition or revision | Where | Retrieved | Role |
|---|---|---|---|---|
| ASTM A6/A6M-05a, Annex A2, tables A2.1 (W) and A2.4 (HP) | approved 1 September 2005 | <https://www.holapl.com/wp-content/uploads/2025/06/ASTM-A-6-A-6M-05a.pdf> (a scan with a text layer) | 2026-11-23 | the declared edition: every shipped value |
| ASTM A6/A6M-05, Annex A2 | approved 1 April 2005 | <https://www.womicsteel.com/uploads/ASTM-A-36A-36M_Standard-Specification-for-Standard-Specification-for-General-Requirements-for-Rolled-Structural-Steel-BarsPlates-Shapes-and-Sheet-Piling.pdf> (the typeset text; despite its file name the document is A 6/A 6M – 05) | 2026-11-23 | the second reading of every row |
| Nucor-Yamato Steel Company, Structural Shapes | cover and general notes dated April 2023; the section-size list and the dimension pages are stamped March 2024 | <https://nucoryamato.com/staticdata/catalog.pdf> | 2026-11-23 | the root radius; the row cross-check |

The catalog states, in its general notes: "This catalog or any part thereof may be reproduced
without permission provided that the reproduced pages, or portions of pages, are clearly marked
and identified with the latest revision date and with Nucor-Yamato Steel Company as the
source." Nothing of it is reproduced; the pack carries facts, and `ATTRIBUTION.md` names the
source and its revision dates as the sentence asks.

## Why A6/A6M-05a

A6/A6M-05a is superseded (editions to A6/A6M-26a followed). The pack declares the edition
whose public tables it used, as the Russia packs stay declared GOST 26020-83 (owner decision
of 2026-11-16): the declared standard on each row is a provenance statement, not a claim that
the edition is current.

The search for a newer openly readable copy, on 2026-11-23:

- The 05a revision reprints Annex A2 of the 05 edition unchanged (its summary of changes
  lists terminology references and section 14.1.1 only), and the two public copies agree value
  for value on all 285 W and HP rows, twelve printed values each.
- Newer editions of the standard were found only behind sign-up walls, as front-page
  previews, or inside a copy whose licence cannot be proved from its origin. `AGENTS.md`
  §1.2 forbids the last class outright, so none of them was used or linked.
- The standard's Annex A2 prints no radius and no HSS; that is a property of every edition,
  not of the one declared.

The gap to the current assortment is visible in the Nucor-Yamato size list, which follows the
current edition of the standard. It names 27 W and HP shapes the 05a tables do not carry:
W14X808, W18X41, W18X45, W21X223, W21X248, W21X275, W24X56, W24X61, W30X433, W36X286,
W36X318, W36X350, W36X387, W40X655, W44X368, W44X408, HP12X89, HP16X88 to HP16X183 and
HP18X135 to HP18X204. They stay `NOT_FOUND`. The same catalog prints revised areas for eleven
W36 rows, W36X231 to W36X652 (W36X231: 43 900 against 44 200 mm²), and three single cells
(W40X392 flange width 318 against 314 mm, W18X192 flange thickness 44.5 against 44.4 mm,
W27X539 web thickness 50.1 against 50 mm); the declared edition wins on all of them.

**Edition-bump path.** When a newer edition of the standard is openly readable, a follow-up
pack declares that edition under the standing pack discipline: its own pull request naming the
standard, the edition and the verification sources. It never edits these rows in place.

## What ships

`environments/usa/profiles-astm-a6-w-hp.usda`: 285 entries, 274 W shapes in seventeen
nominal-depth groups and 11 HP shapes in four, each an `i_shape` row carrying
`steel:profile:standard = "ASTM A6/A6M-05a"`.

- **Values.** Depth, flange width, web thickness, flange thickness and area are the
  standard's own SI column (A2.1: "The values stated in inch-pound units are independent of
  the values stated in SI units"). Nothing is converted and no rounding rule is BeamCraft's;
  the standard's SI rounding is the declared value. This answers question 6 for this pack.
- **Mass.** `steel:profile:massPerMeterKg` is the nominal SI mass of the row's metric
  designation (W1100 X 499 gives 499), the reference field it always was; the mass of a member
  is computed from the catalogue area and the material density, as for every pack.
- **Designations.** The EDI inch form without spaces: `W44X335`, `HP14X117`, with the decimal
  of the inch weight where the standard prints one (`W6X8.5`). The library has no alias field,
  so the metric designation the same row prints (`W1100X499`) does not resolve; the Canada and
  Brazil packs ship the same shapes under their own metric designations.
- **Root radius.** The standard prints none. `steel:profile:rMm` is the catalog's fillet radius
  R for the 272 shapes Nucor-Yamato rolls (8, 8.5, 10, 13, 15, 19, 20 or 30 mm, taken from the
  catalog's metric table and equal to its inch R converted) and 0, a sharp corner, for the
  thirteen it does not roll: W40X331, W36X800, W12X14, W10X19, W10X17, W10X15, W10X12, W8X15,
  W8X13, W8X10, W5X19, W5X16, W4X13. No radius is derived or interpolated.
- **Not carried.** The catalog's detailing dimensions (k, k1, T) have no field in the
  `i_shape` family.

## The declared-area warning is expected here

Every other pack keeps the D3 detector silent (`declared_area_warning` is null on every
entry). This pack cannot: 150 of its 285 rows sit more than one percent from the closed-form
area of their own parameters, 138 of the 272 rows with a catalog radius and 12 of the 13 sharp
rows, the widest being W24X68 at 4.3 percent. Two causes, both in the sources:

1. The standard's area includes root fillets of a radius it does not print. The catalog's
   theoretical radius is Nucor-Yamato's own and larger than the fillet the standard's area
   reflects, so the closed-form area lands above the declared one on most rows; with no radius
   it lands below.
2. The SI column rounds depth and width to the millimetre and the thicknesses to the tenth,
   which moves the closed-form area of a light section by about one percent either way.

The owner decision of 2026-10-18 stands: a difference above one percent is a warning, never a
refusal, and the quantities use the declared catalogue area. The bound is 5 percent; the tests
pin the count (150) and refuse any row beyond it, which keeps the detector a typo check for
this pack too.

## Verification

- Two independent public copies of the standard (a typeset text and a scan) agree on all 285
  rows in both unit columns.
- In every row the inch and the SI reading agree through the unit ratios inside the standard's
  own rounding: depth and width within 1 mm, thicknesses within 0.15 mm except four flange
  thicknesses the SI column rounds from a more exact value (W40X183, W14X605, W14X665,
  W14X730), areas within 0.6 percent except five rows (W36X800, W24X370, W24X192, W21X48,
  W5X16, at most 0.9 percent).
- The catalog's metric table equals the standard's SI values on 258 of the 272 shared rows;
  the fourteen differences are the edition drift listed above.
- Two print errata are read around and named in the tests: the W 36 X 800 designation cell of
  both copies prints `236.4800` (its metric designation W920 X 1191 and its area fix it), and
  the catalog's W6 x 12 depth prints 3.03 in (the standard prints 6.03 in, 153 mm).
- `beamcraft-commands` `library::tests::the_shipped_usa_w_and_hp_library_reads_with_every_entry_admitted`:
  every entry admitted, the full assortment, radii, spot values, the warning count and bound,
  the nominal mass against the area, and the refused designations.
- `beamcraft-cli` `tests/cli.rs` `usa_environment::`: the listing with provenance on two
  pages, the warning present and absent on named rows, and a project created from the
  environment resolving its designations through the project copy, with the A992 grade of the
  same environment as a member's material.

## Not in this slice

- **S, M, C, MC and L shapes** of the same annex: the sloped-flange families need a slope and
  radii convention decision, which the standard does not print.
- **HSS and pipe, tees, double angles**: not in Annex A2; no admitted source.
- **The 27 shapes of newer editions** listed above.
- **Fasteners**: a separate slice of the issue.
