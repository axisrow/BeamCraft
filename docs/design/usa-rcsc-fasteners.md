# USA fastener pack: A325 and A490 structural bolts of the RCSC 2025

Date: 2026-11-24. Status: experimental slice; the fastener slice of the USA pack (fork issue
#708). The profiles landed in #784, the grades in #648; this slice ships the bolts.

## Problem

The `usa` environment shipped material grades and W/HP profiles. Its fastener pack waited on a
dimension source: the product dimensions of heavy hex structural bolts live in paywalled
standards (ASME B18.2.6 bolts, B18.2.2 nuts, ASTM F436 washers), and the RCSC hole rows sat
under the same licence question as AISC. The fork's research admitted the specification on
boltcouncil.org (2026-11-23): the June 5, 2025 edition of the *Specification for Structural
Joints Using High-Strength Bolts* is a free public download, and it prints both halves the
pack needs — the component dimensions in Commentary Table C-2.1 and the hole dimensions in
Table 3.1.

## Sources

| Source | Edition | Where | Retrieved | Role |
|---|---|---|---|---|
| RCSC, *Specification for Structural Joints Using High-Strength Bolts* | June 5, 2025 | <https://www.boltcouncil.org/files/2025RCSCSpecification.pdf> (free download; copyright notice: all rights reserved) | 2026-11-24 | every shipped value: section 2.2 (the bolt grades), Table 2.1 (the groups), section 2.3 (the nuts), Commentary Table C-2.1 (bolt and nut dimensions), Table 3.1 (hole dimensions) |

The document carries an all-rights-reserved notice. The pack answers it the way every shipped
pack answers a public standard: it carries facts re-entered by hand — the dimensions of named
components — and copies no text or table. AISC 360-22 is the design specification that calls
the RCSC up; as an engineering-rule source it names the use the pack serves and contributes no
value.

## Why F3125 Grades A325 and A490, and what F3148 is not

Section 2.2 of the 2025 specification: "Group 120, 144, and 150 heavy hex structural bolts
shall meet the requirements of ASTM F3125 Grades A325, 144, and A490, respectively." The
heavy hex structural bolt therefore carries one product standard, ASTM F3125, with three
grades; the pack ships the two the slice names (A325 and A490) as its rows. Grade 144 has the
same C-2.1 dimensions and is a follow-up row set, not new data. **F3148 is not the bolt
standard**: section 2.4.2 admits it for *spline end fixed matched bolting assemblies* of Group
144 only, and Table 2.1 lists it under matched assemblies — the earlier memo's "A325/A490 per
F3148 consolidation" reads the consolidation that happened in F3125 (2019), not F3148.

The nuts are ASTM A563 heavy hex (grade per Table 2.2), and section 2.3.2 — new in 2025 —
permits ASTM A194 Grade 2H nuts as substitutes for A563 Grades C, D and DH. The nut
dimensions are grade-independent, so the pack carries one nut set per size and the doc string
names the substitution.

## Reading the tables

The PDF's text layer garbles every fraction glyph (Table 3.1 prints `b×n` for `9/16 × 11/16`,
`11/8` survives, `1z` stands for `1-1/16`), so the pages were rendered to PNG images and the
cells transcribed by eye, at 500–1000 dpi with the contested cells cropped singly. The
cross-checks: the specification's own ≥1 1/8 formula row (standard = d+1/8, oversized =
d+5/16, short slotted = (d+1/8) × (d+3/8), long slotted = (d+1/8) × 2.5d) reproduces the
bandings of the enumerated rows exactly; the text layer's surviving integer parts agree with
the render on every cell it does not garble; and the C-2.1 columns match the dimension series
their footnoted ASME standards are known by (head across flats 7/8 to 2-3/8, head heights
5/16 to 15/16, nut heights 31/64 to 1-15/32), value for value.

**The 2025 Table 3.1 differs from the 2020 one**, which matters for anyone comparing packs:
the standard hole is d+1/16 only up to 7/8 in and d+1/8 from 1 in up; oversized holes grew to
d+1/8 (1/2 in), then d+3/16 (5/8 to 7/8), d+1/4 (1 in) and d+5/16 (above); short-slot lengths
band d+3/16, d+1/4, d+5/16 and d+3/8; long slots stay 2.5d. The pack ships the standard-hole
column and the tests pin the band change at 1 in.

## What ships

`environments/usa/fasteners-rcsc-2025.usda`: 18 entries, `A325-1/2` … `A325-1-1/2` and
`A490-1/2` … `A490-1-1/2`, one row per size and grade, the sizes 1/2 through 1-1/2 in
C-2.1 covers.

- **Values.** The nominal diameter, head across flats, head height, nut across flats and nut
  height are C-2.1's inch fractions converted exactly (x 25.4, the 2026-11-23 decision for
  sources that print only inches: the pack converts nothing else and rounds nothing). The
  designation carries the size the way the tables print it (`A325-1-1/8`, not a decimal).
- **Holes.** Every row declares the Table 3.1 standard hole as its clearance — 1/16 in over
  the bolt through 7/8 in, 1/8 in from 1 in (1.5875 or 3.175 mm). This is the one hole type
  the fastener model carries (the bolt amendment of 2026-10-23 keeps oversize, slots and
  blind holes post-MVP, and A14.1 records that they have no representation). The other three
  columns of Table 3.1 are therefore not in the pack; their rules are recorded above and wait
  for the hole-types mechanism. Nothing slot-shaped is faked into a round field.
- **Nuts.** One nut set per size from C-2.1 (the heavy hex nut of A563, with the A194 2H
  substitution declared in the doc string). There is no separate nut catalogue; the nut
  travels inside the bolt definition, as in every pack.
- **No washers.** F436 is paywalled ASTM; the pack ships no washer dimensions, and both
  washer switches are false. The Commentary's washer rules (Table C-2.2's length allowances)
  are engineering rules, not dimensions, and are not entered.
- **Contributor defaults.** The offered lengths ride one grid for all sizes — 1-1/4 to 5 in
  in quarter steps, 5-1/2 to 8 in in half steps, exact in millimetres; longer and shorter
  commercial lengths exist. The thread protrusion (4 to 12 mm with the size) is the same
  default the other packs declare.
- **Not carried.** The C-2.1 thread length and transition-thread columns have no field in the
  fastener definition, like the detailing dimensions the profile pack left out. The metric
  A325M series is out of this slice (the fork admission excludes it), so `A325M-16` and the
  bare metric names stay `NOT_FOUND`, as do the washer designations.
- **Resolution.** The library rides the `usa` environment (`bim:environment:fastener-libraries`),
  so a project created with the environment resolves `A325-3/4` through its own copy; since
  #787 every creation reply names the answering layer and library.

## Verification

- Table cells transcribed from rendered pages; the checks above (formula row, text-layer
  integers, the ASME dimension series) all agree with the render.
- `beamcraft-commands` `library::tests::the_shipped_usa_environment_admits_its_fastener_library`:
  the environment admits its fastener library with 18 entries and none refused, the designations
  list, spot values of both grades, the standard-hole band change at 1 in, no washer fields
  anywhere, and `NOT_FOUND` for the unpublisheds.
- `beamcraft-cli` `tests/usa_fasteners.rs`: the environment lists the library, a project
  carries the copy, a bolt group resolves `A325-3/4` with no library option and the reply's
  lookup names the project layer and the library id, the length resolution answers on the
  declared grid, and the metric, washer and lowercase designations refuse `NOT_FOUND`.

## Not in this slice

- **F3125 Grade 144 rows** (same dimensions, one grade string away) — a follow-up pack, never
  an edit of these rows.
- **Washers** (F436 paywalled) and **metric A325M** (excluded by the slice's admission).
- **Oversize and slotted holes**: no representation until the hole-types mechanism lands.
