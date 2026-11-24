# South America profile pack: the W shapes of ASTM A6/A6M-05a

Date: 2026-11-24. Status: experimental slice; the second slice of the South America pack
(fork issue #711), after the grades-only environment of #782. It completes the pack the
owner decision of 2026-11-23 defined (NCh203 materials, the profiles following the #708
route) and answers the designation question the route left open.

## Problem

The `south-america` environment shipped the NCh203.Of2006 material grades only. NCh203
carries material requirements and no section geometry: the W shapes of Chilean structural
practice are rolled abroad and imported. The pack needed a geometry source and a
designation form for a market whose profiles have no national section standard.

## Sources

| Source | Role | Retrieved |
|---|---|---|
| ASTM A6/A6M-05a, Annex A2, table A2.1 | the declared edition: every shipped value and every metric designation | 2026-11-24 |
| Nucor-Yamato Steel Company, Structural Shapes catalog | the root radius, as in the USA pack | 2026-11-24 |
| CINTAC public product pages | the availability check (see below) | 2026-11-24 |
| Aceros AZA, Catálogo Técnico 2024 | the availability check (see below) | 2026-11-24 |

The geometry values are the USA pack's re-entered A6/A6M-05a SI-column rows, carried over
value for value; the source copies behind them are the ones the USA pack's
[design note](usa-astm-a6-profiles.md) and `ATTRIBUTION.md` row link. This slice re-read
the standard's two public copies — the [holapl scan](https://www.holapl.com/wp-content/uploads/2025/06/ASTM-A-6-A-6M-05a.pdf)
and the [womicsteel typeset text](https://www.womicsteel.com/uploads/ASTM-A-36A-36M_Standard-Specification-for-Standard-Specification-for-General-Requirements-for-Rolled-Structural-Steel-BarsPlates-Shapes-and-Sheet-Piling.pdf)
— for the one fact the USA pack does not carry: the metric designation of every row, the
column the SI table prints beside the inch one. The two copies agree value for value on
all 274 metric designations, and every printed metric mass equals the `massPerMeterKg`
the USA pack already ships, which joins the two re-enterings row by row. The
[Nucor-Yamato catalog](https://nucoryamato.com/staticdata/catalog.pdf) is the radius
source unchanged (its reproduction-permission sentence is quoted in `ATTRIBUTION.md`);
nothing of it is reproduced. The [CINTAC](https://www.cintac.cl/tubos-perfiles-plancha/)
public pages and the [Aceros AZA Catálogo Técnico 2024](https://www.aza.cl/2024/wp-content/uploads/2024/07/Catalogo-Tecnico-2024-QR.pdf)
are the availability check below; no table is copied from any of them.

## What ships

`environments/south-america/profiles-astm-a6-w.usda`: 274 entries, the complete W-shape
series of table A2.1, each an `i_shape` row carrying
`steel:profile:standard = "ASTM A6/A6M-05a"`.

- **Designations.** The standard's own metric form, without spaces: `W310x60`, with the
  printed decimal kept where the standard prints one (`W150x13.5`, `W360x32.9`). This is
  the form the route's owner direction names (the metric form of the regional producer
  prints) — the verification the direction asked for found no Chilean producer print to
  override it: the CINTAC public pages list no W shapes at all (their rolled profiles are
  tubes, channels, z-purlins and angles under their own ET-M/MA-2 sheets and ASTM A-500),
  and the AZA catalog carries angle and Estrella profiles only. The Chilean W assortment
  being imports, the standard's metric designation is the one national-independent name
  the declared edition itself provides.
- **The one collision.** The standard prints one metric designation for two rows: W 36 X
  231 and W 36 X 232 both carry W920 X 345 (the nominal masses round to the same printed
  value). The pack refuses to pick a winner: the two rows keep their EDI inch designations
  (`W36X231`, `W36X232`) and `W920x345` names no row. The tests pin both rows resolving
  and the collision designation refused.
- **Values.** Depth, flange width, web thickness, flange thickness, area and nominal mass
  are the USA pack's rows value for value — the standard's own SI column, nothing
  converted, no rounding rule of BeamCraft's. The declared-area warning is expected on
  part of the rows for the two causes the USA pack documents (the unprinted root radius
  and the SI rounding); the tests pin the count and the 5 percent bound.
- **Root radius.** The Nucor-Yamato catalog's fillet radius for the shapes it rolls and
  zero for the thirteen it does not, exactly as in the USA pack.
- **Not carried.** The HP shapes of table A2.4 and the S, M, C, MC and L shapes of the
  same annex; the catalog's detailing dimensions.

## Availability: the full declared W series, and why

The route's direction offered two shapes for the pack: a CINTAC-verified availability
subset, or the full A6 W list with `NOT_FOUND` for what no public Chilean source lists.
The verification chose the second form, because the first does not exist: no public
Chilean source lists a W-shape assortment. CINTAC's public pages — the pages the
direction named — carry no W shapes and no designation print to borrow a form from; the
AZA catalog has no W shapes; and a wider search found no producer or distributor catalog
of Chilean W imports under a licence that admits its use. An availability subset would
therefore have been invented, not verified. The pack ships the declared edition's
complete W series as reference geometry and says so; which of the shapes reach Chilean
stock in a given month is an import-market fact no admitted public source states. The HP
shapes stay `NOT_FOUND` under the same rule: no public Chilean source lists them. A
future slice with an admitted availability source (a producer catalog that permits its
use, as Nucor-Yamato's does) can mark or trim the series; the edition-bump path of the
USA pack applies unchanged.

## Verification

- The metric designations of both public copies agree value for value on all 274 W rows,
  and every printed metric mass equals the `massPerMeterKg` of the USA pack's row with
  the same inch designation.
- `beamcraft-commands` `library::tests::the_shipped_south_america_a6_w_library_reads_with_every_entry_admitted`:
  every entry admitted, the full assortment in the metric designations, the collision rows
  under their inch names, radii, spot values, the warning count and bound, and the refused
  designations (HP, inch forms of metric-named rows, `W920x345`, newer-edition shapes).
- `beamcraft-cli` `tests/south_america_environment.rs`: the listing with provenance on two
  pages, the warning present and absent on named rows, and a project created from the
  environment resolving its designations through the project copy, with the A270ES grade
  of the same environment as a member's material.
- The material library of the environment is unchanged; its tests pin the grades.

## Not in this slice

- **HP shapes**: `NOT_FOUND` (no admitted Chilean source), not dropped — a slice with an
  admitted source re-enters them.
- **An availability annotation or subset**: waits for an admitted public source that
  states one.
- **Fasteners**: a separate slice of the issue.
