# Brazil profile pack: W and HP shapes of the A6 assortment under ABNT NBR 15980:2024

Date: 2026-11-24. Status: shipped slice; the profiles slice of the Brazil regional pack (fork
issue #710, one of the regional packs of #649). It applies the owner decision of 2026-11-23
([decisions](../plan/decisions.md), the regional sourcing principle and the Brazil route) with
the producer prints verified on 2026-11-24.

## Problem

The issue was blocked on the owner-admitted source: Brazil has no single open national section
standard — ABNT NBR 15980 ("Perfis laminados de aço para uso estrutural — Dimensões e
tolerâncias", published 2024-01-17, superseding 2011/2020) is paywalled at ABNT with no official
open text — and its sections are published by producers, so route (b) needed a licence audit per
publisher. The audit found ArcelorMittal Brasil reserving all catalogue content and Gerdau
granting none. The owner's decision settles the route: Brazil's W and HP shapes are the ASTM
A6/A6M assortment under the producers' metric designations; producer catalogues are cross-check
and availability sources only; nothing is purchased; open online sources only, every source
linked.

## What the producers print (verified 2026-11-24)

- **Gerdau, Tabela de Bitolas** (revision 11/2018): the metric designations print as
  «W 310 x 60,0», «HP 310 x 79,0» — prefix, metric depth, lowercase x, one decimal place,
  decimal comma — with the EDI inch designations in the same style beside them («W 12 x 40»).
  Its footnote declares the dimensions "de acordo com as normas ABNT NBR 15980:2011 e ASTM
  A6/A6M", and the shapes ship in ASTM A572 Grade 50 (the NBR 7007 designation AR 350).
- **Gerdau, Perfis Estruturais W — Informações Técnicas e Bitolas** (revision 09/26, Gerdau's
  own CDN): the same designation form and the same declaration with the edition left off
  ("de acordo com as normas ABNT NBR 15980 e ASTM A6/A6M"). Its note maps the assortment
  explicitly: the W shapes are ASTM A6/A6M table A2.1 and ABNT NBR 15980 table A.4 alike, and
  the two light rows it rolls outside both standards (W150×9,2 and W150×10,4) are named as such.
- **ArcelorMittal Brasil**: its catalogue labels the W series NBR 15980:2024 (verified against
  the producer's materials in the #877 research of 2026-11-23). The official catalogue PDF is
  JS-gated on the producer's site as of 2026-11-24; no copy of it was downloaded and no value
  was read from it — it is not a source of this pack.

**The declared standard.** The rows declare `ABNT NBR 15980:2024`, the current edition of the
national standard whose W and HP content is exactly the A6 assortment (the producers themselves
map the two: A6 table A2.1 ↔ NBR 15980 table A.4) and which both producers print for these
dimensions — Gerdau currently undated (09/26) and dated :2011 in the 2018 table, ArcelorMittal
:2024. The values behind the declaration are not from NBR 15980, whose official text was not
purchased: they are the standard's own SI column — the annex of the edition the `usa` pack
declared — and the design note, the library metadata and the attribution say so. As with the
declared GOST 26020-83 and A6/A6M-05a editions, the declared standard is a provenance statement
of what the row is, not a claim that the pack read that text.

## Sources

| Source | Edition or revision | Where | Retrieved | Role |
|---|---|---|---|---|
| ASTM A6/A6M-05a, Annex A2, tables A2.1 (W) and A2.4 (HP) | approved 1 September 2005 | <https://www.holapl.com/wp-content/uploads/2025/06/ASTM-A-6-A-6M-05a.pdf> (a scan with a text layer) | 2026-11-24 | the SI designation column (metric depths and masses); every shipped value rides the `usa` pack's two-copy re-entry |
| the same, second public copy | approved 1 April 2005 | <https://www.womicsteel.com/uploads/ASTM-A-36A-36M_Standard-Specification-for-Standard-Specification-for-General-Requirements-for-Rolled-Structural-Steel-BarsPlates-Shapes-and-Sheet-Piling.pdf> | 2026-11-23 (by #708) | the second reading of every row (verified there, value for value) |
| Nucor-Yamato Steel Company, Structural Shapes | cover and general notes dated April 2023; the section-size list and dimension pages stamped March 2024 | <https://nucoryamato.com/staticdata/catalog.pdf> | 2026-11-24 | the root radius; the size list's inch→metric designation mapping, the row cross-check |
| Gerdau, Tabela de Bitolas | revision 11/2018 | <https://gsn.gerdau.com/sites/gsn_gerdau/files/downloadable_files/Tabela%20de%20Bitolas_1.pdf> | 2026-11-24 | the designation form; availability and the mass/designation cross-check |
| Gerdau, Perfis Estruturais W — Informações Técnicas e Bitolas | revision 09/26 | <https://d1mwt7qdngq7b2.cloudfront.net/Cat%C3%A1logos%20e%20Manuais/Cat%C3%A1logo-Perfil-Estrutura-Informa%C3%A7%C3%B5es-Tecnicas-WEB.pdf> | 2026-11-24 | the current declaration of standards; the same cross-checks on its table |

The Nucor-Yamato catalog states, in its general notes: "This catalog or any part thereof may be
reproduced without permission provided that the reproduced pages, or portions of pages, are
clearly marked and identified with the latest revision date and with Nucor-Yamato Steel Company
as the source." Nothing of it is reproduced; the pack carries facts, and `ATTRIBUTION.md` names
the source and its revision dates as the sentence asks. The Gerdau documents grant no
reproduction right and reserve their content; nothing of them is reproduced either — the pack
names them, takes no value from them, and uses their printed rows only to confirm designations
and values re-entered from the standard's own tables.

## What ships

`environments/brazil/profiles-abnt-nbr-15980.usda`: 284 entries, 273 W shapes in seventeen
nominal-depth groups and 11 HP shapes in four, each an `i_shape` row carrying
`steel:profile:standard = "ABNT NBR 15980:2024"`. The values are the `usa` pack's A6/A6M-05a
re-entry, value for value, with no new number entered.

- **Designations.** The producers' print form, verified on their own tables: «W 310 x 60,0»,
  «HP 360 x 174,0» — series prefix, the annex's metric depth, lowercase x, the designation's
  nominal SI mass with one decimal and a decimal comma. The mass tail is the row's
  `massPerMeterKg`, the annex's designation mass. Prim names replace the spaces and comma with
  underscores (`W310x60_0`). The library has no alias field, so the EDI inch form of the same
  row does not resolve here: it resolves through the `usa` environment's pack, and the library
  test loads both and shows the two forms answering with one row's facts.
- **Coverage.** 106 of the 284 designations print identically in Gerdau's own tables (the rows
  the mill rolls); the rest are the declared standard's assortment beyond the regional mills'
  programme — the pack ships the standard's rows, not a mill list. The two light rows Gerdau
  rolls outside both standards (their own note names them: W 150 x 9,2 and W 150 x 10,4) are
  not annex rows and stay `NOT_FOUND`, as do angles, flat bars, channels and the S and M
  families (no admitted source in this slice).
- **One designation, two shapes.** The annex prints the same SI designation for two of its
  shapes: W36X231 and W36X232 both carry «W920 X 345» (their inch masses, 231 and 232 lb/ft,
  round to the same SI mass column value). One library cannot carry two rows under one
  designation, and no public print distinguishes them: the Nucor-Yamato size list rolls and
  prints «W920 x 345» for W36 x 232 and lists no W36 x 231 at all. The designation therefore
  names W36X232 in this pack (h 943 mm, area 44 000 mm²), and W36X231 (h 927 mm, area 44 200
  mm²) stays out of the pack — `NOT_FOUND` under every designation any source prints for it.
  A source that distinguishes the two is an edition-bump row under the standing pack
  discipline, never an edit of these rows in place.
- **Radii.** As in the `usa` pack: the standards print no radius, so `steel:profile:rMm` is the
  Nucor-Yamato catalog's fillet radius for the shapes it rolls and 0, a sharp corner, for the
  thirteen it does not. No radius is derived or interpolated.
- **Not carried.** The catalogues' detailing dimensions (k, k1, T) have no field in the
  `i_shape` family.

## The declared-area warning is expected here

Identical to the `usa` pack, row for row: 150 of the 284 rows sit more than one percent from
the closed-form area of their own parameters, none beyond five percent (widest W 610 x 101,0 at
4.3 percent). The causes are the same two: the standard's area includes root fillets of a
radius it does not print, and the SI column rounds depth and width to the millimetre and the
thicknesses to the tenth. The owner decision of 2026-10-18 stands: a warning, never a refusal,
and the quantities use the declared catalogue area. The tests pin the count (150) and refuse
any row beyond the five-percent bound.

## Verification

- The SI designation column was re-read from the holapl.com copy of A6/A6M-05a (the edition the
  `usa` pack declared and verified value for value against a second public copy). For every one
  of the 285 annex rows the designation's mass equals the `usa` pack's re-entered nominal mass,
  so each row's SI designation matched its facts before any designation was assigned.
- The Nucor-Yamato size list maps every inch designation to its metric designation; for every
  shared row the metric depth agrees with the pack's, and the metric mass agrees within
  1 kg/m — the four wider prints (W 460 x 349,0: 348, W 460 x 315,0: 314, W 360 x 58,0: 57.8,
  W 360 x 44,6: 44) are the catalog's own unrounded masses, and the declared designation keeps
  the annex value, as the `usa` pack keeps the standard's SI column on edition drift.
- Gerdau's two tables print the designation form; for the 106 rows the mill rolls the printed
  designation equals the pack's byte for byte, including the decimal-comma masses
  («W 410 x 38,8», «W 610 x 153,0»), and their printed mass column equals the designation mass.
- `beamcraft-commands` `library::tests::the_shipped_brazil_w_and_hp_library_reads_with_every_entry_admitted`:
  every entry admitted, the full assortment in the producers' form, the declared standard on
  every row, radii, sharp rows, spot values, the warning count and bound, the nominal mass
  against the area, and the refused designations.
- `beamcraft-commands` `library::tests::the_brazil_and_usa_libraries_resolve_one_shape_under_both_designation_forms`:
  one row answers under the producers' metric designation through the `brazil` pack and under
  the EDI inch form through the `usa` pack, with the same catalogue area.
- `beamcraft-commands` `library::tests::the_shipped_brazil_nbr_7007_materials_library_reads_with_every_entry_admitted`:
  the four grades admitted, the declared edition and density on every row, and the refused
  grades (compact spellings, BR 190, the unlisted strength levels, the ASTM names).
- `beamcraft-cli` `tests/cli.rs` `brazil_environment::`: the listing with provenance on two
  pages, the warning present and absent on named rows, and a project created from the
  environment resolving «W 310 x 60,0» under the AR 350 grade and the other metric designations
  through the project copy, with every refused designation and grade refusing `NOT_FOUND`.

## The grades

`environments/brazil/materials-nbr-7007.usda` ships four NBR 7007 grade rows — MR 250, AR 350,
AR 350 COR and AR 415 — each carrying `steel:material:standard = "ABNT NBR 7007:2022"` and the
7850 kg/m3 density of EN 1991-1-1. NBR 7007 ("Aços-carbono e aços microligados para barras e
perfis laminados a quente para uso estrutural - Requisitos") was admitted for this pack by the
owner decision of 2026-11-24, per the EN 10365/CEN precedent: the shipped packs already
re-enter facts — route (a), BeamCraft's own data, no text or table copied, values from public
copies — from standards owned by private associations (CEN for the EN 10365 packs, BIS for the
IS packs, Standards Australia for the AS/NZS packs), so NBR 7007 is the same class and the
same footing as every shipped pack, and the 2026-11-23 state-owned principle governs the
upstream coverage mandate, not the pack data practice. The official text was not purchased,
and the grades are the standard's own designation system — the strength prefix (MR medium, AR
high, COR atmospheric-corrosion resistance) with the minimum yield strength in MPa.

- **The edition.** The 2022 edition (October 2022, vigente) is the one the public listings of
  the ABNT catalogue name (the Busca Normas listing, buscanormas.com.br, is derived from
  abntcatalogo.com.br and prints "Edição: 10/2022"; the current ArcelorMittal producer
  documentation cites NBR 7007:2022 the same way). Older editions cited in the wild (2002,
  2011 — the latter in Gerdau's 2018 table) precede it; the designation system and the grade
  names are the standard's own and the shipped rows declare the current edition.
- **The grade set.** The public sources corroborate four designations: Simec's quality page
  (gruposimec.com.br/padrao-de-qualidade, retrieved 2026-11-24) names MR 250 / ASTM A36, AR 350
  or AR 415 / ASTM A572, and AR 350 COR / ASTM A588; TotalMateria's public NBR 7007 standards
  page (totalmateria.com, retrieved 2026-11-24) lists AR 350, AR 350 COR, AR 415 and MR 250
  (and BR 190); Gerdau's prints map AR 350 to ASTM A572 Grade 50 (the 09/26 catalog) and AR 350
  COR to AÇO COR 500 per NBR 7007:2011 (the 2018 table). The ASTM relations are the
  standard's/producer mappings, not aliases: the ASTM names stay `NOT_FOUND` here, as in the
  NCh203 pack.
- **Designation form.** The spaced print («AR 350», «MR 250»), the form the listings and
  Gerdau's 2018 table print. Gerdau's current 09/26 catalog runs the prefix and the number
  together ("AR350"); the compact spellings stay `NOT_FOUND`.
- **BR 190 stays out.** The light serralheria grade is named by the TotalMateria listing alone;
  no producer print corroborates it, and its presence in the 2022 edition is unverified —
  `NOT_FOUND` under the standing discipline (as are AR 345, AR 500 and MR 325, which no public
  listing carries).
- **Not here:** fasteners, and the ASTM/A588 designations (the `usa` environment carries those
  grades for its own pack).

## Not in this slice
- **Angles, flat bars, channels, S and M shapes**: no admitted source; the A6 annex's sloped
  families need the slope decision the `usa` slice also left open.
- **The 27 shapes of newer editions** of A6/A6M (the Nucor-Yamato size list names them), and
  the newer editions of NBR 15980 content they may carry: `NOT_FOUND`; edition bumps get their
  own PRs under the standing pack discipline.
- **Fasteners**: a separate slice of the issue.
