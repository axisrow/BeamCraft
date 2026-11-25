# Canada profile pack: the ASTM A6/A6M-05a W and HP assortment under metric designations

Date: 2026-11-24. Status: the first slice of the Canada profiles (fork issue #709). It applies the
owner decisions of 2026-11-23 ([decisions](../plan/decisions.md)) and the data-source decision
memo of the same date, section 3: Canada is the ASTM A6/A6M assortment under metric designations;
CISC is not admitted.

## Problem

The `canada` environment shipped the CSA G40.20/G40.21 material grades (#702) and no profile
pack. The issue waited on the same two owner decisions as the USA pack (#708): whether the CISC
Handbook (and behind it the AISC database) could be a source, and how published inch values were
to become millimetres. Both are settled the way the USA slice settled them: CISC is not admitted
(its site terms forbid copying, and its W shapes are the A6 assortment anyway), and no pack
converts or rounds anything — the declared source is the standard's own SI column.

## Sources

| Source | Edition or revision | Where | Retrieved | Role |
|---|---|---|---|---|
| ASTM A6/A6M-05a, Annex A2, tables A2.1 (W) and A2.4 (HP), the metric designation column and the SI column | approved 1 September 2005 | <https://www.holapl.com/wp-content/uploads/2025/06/ASTM-A-6-A-6M-05a.pdf> (a scan with a text layer) | 2026-11-24 | the declared edition: every shipped value and every metric designation |
| ASTM A6/A6M-05, Annex A2 | approved 1 April 2005 | <https://www.womicsteel.com/uploads/ASTM-A-36A-36M_Standard-Specification-for-Standard-Specification-for-General-Requirements-for-Rolled-Structural-Steel-BarsPlates-Shapes-and-Sheet-Piling.pdf> (the typeset text; despite its file name the document is A 6/A 6M – 05) | 2026-11-24 | the second reading of every metric designation |
| Nucor-Yamato Steel Company, Structural Shapes | cover and general notes dated April 2023; the section-size list and the dimension pages are stamped March 2024 | <https://nucoryamato.com/staticdata/catalog.pdf> | 2026-11-24 | the root radius |

The radius source is the USA pack's, and the catalog's permission sentence, its revision dates
and the reading-around of its errata are recorded in `ATTRIBUTION.md` exactly as there. Nothing
of any source is reproduced; the pack carries facts.

**No availability cross-check is admitted for Canada.** The memo's Canada section (3a) admits
CISC only as read-only market knowledge, never for values, and no Canadian producer catalogue is
admitted yet; whether one can be (under route (b), with a provable licence) is a later slice's
decision. The pack therefore ships the standard's full declared assortment without an
availability filter, like the USA pack.

## Why the alias entries

The owner decision says: designated in the metric form the standard prints (W310x60), with the
inch form as alias. The library format has no alias field — a designation resolves by exact
string match, and the USA pack documents what that means (its rows answer only their EDI inch
names). This pack realises the alias in data: every row of the declared tables is entered twice,
once under the metric designation Annex A2 prints for it and once under the EDI inch designation
the USA pack ships. Both names resolve to the same geometry; 569 entries carry the 285 rows
(one metric name is shared by two rows, see below).

- **Metric designations.** Annex A2 prints each row's metric designation in its own column
  ("W1100 X 499", continuations "X 433" under the group's depth). The pack drops the spaces and
  spells the separator lowercase: the lowercase form is what Canadian practice prints and what
  the Nucor-Yamato catalog's size list prints ("W920 x 345"); the declared standard itself
  prints a capital X. The decimal masses keep the table's own digits ("W310x21.0", "W410x46.1").
  The trailing-zero forms are the tables' own: both declared copies print "X 13.0" (W6X8.5),
  "X 18.0" (W6X12), "X 24.0" (W6X16), "X 15.0" (W8X10), "X 21.0" (W12X14) and "W360 X 39.0"
  (W14X26) — holapl rows 3447–3450, 3441 and 3350, womicsteel rows 3448–3451, 3442 and 3350 —
  so these six designations carry the printed `.0` on purpose. The sibling south-america pack
  (#793) reads the same column of the same copies as W150x13, W150x18, W150x24, W200x15,
  W310x21 and W360x39 without the `.0`; its follow-up issue names the rows and the evidence.
  The inch alias is the USA pack's designation unchanged ("W12X40", "W6X8.5"), so the same rows
  resolve by either name across the two environments.
- **Values.** Depth, flange width, web thickness, flange thickness, area and the nominal mass of
  the designation are the USA pack's, value for value: both were read from the same SI column of
  the same declared tables. The extraction re-verified every row against both public copies
  (designation tokens and the five SI values per row) before generating the file.
- **Radii.** The Nucor-Yamato fillet radius per row, with a sharp corner (r = 0) for the
  thirteen shapes the catalog does not roll, exactly as the USA pack carries them.

**The one ambiguous designation.** The declared tables print W920 X 345 for two distinct rows:
W36X231 (d 927, bf 418) and W36X232 (d 943, bf 308). A duplicate designation refuses a whole
library, and inventing a spelling would fabricate a designation no source prints, so the metric
name is carried once, for the row the tables print first (W36X231); W36X232 answers through its
inch alias. A user typing W920x345 gets the W36X231 geometry. The newer-edition size list of the
Nucor-Yamato catalog distinguishes the two (W920 x 344 / W920 x 345), but those masses belong to
that edition, and the declared one wins. The sibling A6 packs shipped the same week resolved the
same collision differently, each within its own designation decision: the south-america pack
(#793, metric names only, no aliases) leaves the collision name on neither row, and the brazil
pack (#710, the producers' comma form) names W36X232 and leaves W36X231 out. Canada's decision —
metric designations plus inch aliases — admits this pack's third reading: the name answers the
row the declared tables print first, and the other row stays reachable under its inch name.

## Steel grades context

CSA G40.20/G40.21 is the steel of the Canadian environment: the grades shipped in the #702
material pack (260W to 550W, the weldable category). This slice adds no grade rows; a member
created from a metric designation takes a CSA grade (`--material 350W`) from the same
environment's material library.

## What ships

`environments/canada/profiles-astm-a6-w-hp.usda`: 569 entries — the 285 rows of tables A2.1 and
A2.4 (274 W shapes, 11 HP shapes) under their metric designations (284 names; W920x345 carried
once) and all 285 under their EDI inch aliases — each an `i_shape` row carrying
`steel:profile:standard = "ASTM A6/A6M-05a"`. The `canada` environment lists the library beside
its material library.

## The declared-area warning is expected here

The same physics as the USA pack: the standard prints no radius and rounds its SI dimensions, so
150 of the 285 rows sit more than one percent from the closed-form area of their own parameters.
Each of those rows warns in both of its designation forms: 300 of the 569 entries carry the
warning, none beyond 5 percent (owner decision of 2026-10-18: a warning, never a refusal; the
quantities use the declared catalogue area). The tests pin the count and the bound.

## Verification

- Both public copies of the standard agree on every metric designation token and on the five SI
  values of all 285 rows; the SI values equal the USA pack's shipped values row for row, and the
  mass number of every metric designation equals the row's `massPerMeterKg`.
- The W 36 X 800 designation cell of both copies prints `236.4800` (the USA pack's documented
  erratum); the row is read from its metric designation W920 X 1191, which is also its metric
  name here.
- `beamcraft-commands`
  `library::tests::the_shipped_canada_w_and_hp_library_reads_with_every_entry_admitted`: every
  entry admitted, 569 entries, radii and the sharp set in both designation forms, spot values in
  both forms, the ambiguous W920x345 row, the warning count (300) and bound, the nominal mass
  against the area, the full metric and alias assortments, and the refused designations (the
  newer editions' jumbo shapes in both forms, the other shape families of the same annex, and
  the wrong spellings: capital-X metric forms, lower-case alias forms, spaced forms).
  `the_shipped_canada_environment_admits_its_material_library` now sees the profile source
  beside the material one.
- `beamcraft-cli` `tests/canada_environment.rs`: the listing with provenance over three pages,
  the warning present and absent on named rows, and a project created from the environment
  resolving W310x60, its alias W12X40, the sharp W100x19.3, HP360x174 and the ambiguous
  W920x345 through the project copy — with the 350W grade of the same environment as a member's
  material, and the absent designations refused.

## Not in this slice

- **S, M, C, MC and L shapes** of the same annex: the USA pack's deferral stands (no slope and
  radii convention decision).
- **HSS** (the Canadian CSA G40.21 Class C and H hollows included): no admitted source; the
  memo leaves Atlas Tube-class sources unproven.
- **Canadian bolt and hole rules** (CSA S16, A325M): a separate decision.
- **Fasteners**: a separate slice of the issue.
- **A producer availability filter**: deferred until a Canadian producer's public catalogue is
  admitted as a cross-check.
