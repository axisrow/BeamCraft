# Blank environment and seeded new models

Date: 2026-10-30, revised 2026-11-01. Status: experimental slice; no gate acceptance is claimed.

## Problem

A new model had no profile and no material definitions. The native window lists the definitions
of the model, so the beam and column tools offered an empty list and nothing could be placed.
The `blank` environment was empty and the environment chosen on the start window was only a
saved setting.

## What changed

* `environments/blank/` is the built-in A07.3 base environment. Its library has nineteen generic
  profiles (I, H, channel, angle, RHS, SHS, CHS, bar and one cold-formed C). Every value is
  invented and rounded; none follows a standard or catalogue (`AGENTS.md` 1.2). The start window
  prefers it when no environment is saved. These profiles first shipped as a separate `starter`
  environment next to an empty `blank`; the two were merged on 2026-11-01 so that the base
  environment is usable on its own. Regional environments ship beside it since 2026-11-14:
  `environments/europe/` holds the real EN 10365:2017 IPE, HEA and HEB nominal dimensions, the
  parallel-flange UPE channel series and, since 2026-11-15, a second profile library with the
  EN 10219-2:2006 cold-formed hollow sections (SHS, RHS and CHS common sizes, validated against
  the hollow closed-form area) and, since 2026-11-15, the DIN hex-bolt fastener set completed
  with the EN 1090-2 hole-series rows (M12 to M30, the environment's fastener library,
  BC-ENV-002); `environments/russia/` holds the GOST 26020-83 parallel-flange
  beam (Б), wide-flange (Ш) and column (К) series and, since 2026-11-15, a second profile
  library with the GOST 30245-2003 cold-bent closed hollow sections (the complete square
  series, sizes 40x40 to 300x300, and the rectangular series 50x25 to 400x200 entering in
  height-band slices; validated against the hollow closed-form area), a third profile
  library with the GOST 8239-89 inclined-flange I beams and the GOST 8240-89 slope-series
  channels through the `tapered_i` and `tapered_c` families (the slopes the standards print
  as roll-pass ranges 6–12 % and 4–10 % carry as data per row), and the
  GOST 7798-70 hex-bolt fastener set (M12/16/20/24, the environment's fastener library,
  BC-ENV-002) and, since 2026-11-15, a material library with the complete grade set of
  GOST 27772-2015 (fourteen grades, the designations in the standard's own Cyrillic form,
  where the leading С of «сталь строительная» is part of the standard's designation and is
  not Latinized the way the profile series letters are);
  `environments/australia/` (2026-11-15) holds the
  AS/NZS 3679.1:2016 UB and UC series and the AS/NZS 1163:2016 Appendix D hollow series and,
  since 2026-11-15, the AS/NZS 1252 high-strength structural hex-bolt fastener set
  (M12/16/20/24/30/36, the environment's fastener library, BC-ENV-002; the holes follow the
  AS/NZS 5131 standard-hole series; see `ATTRIBUTION.md`),
  `environments/india/` holds the IS 808:2021
  narrow (NPB) and wide (WPB) parallel flange beam series and, since 2026-11-15, the IS 3757
  high-strength structural hex-bolt fastener set (M16/20/24, the environment's fastener library,
  BC-ENV-002) and, since 2026-11-16, a fourth profile library with the legacy sloping-flange
  series of the same edition (tables 1, 2, 5, 6 and 7: the MB/WB medium and wide beams, the
  JB/LB junior and light beams, the SC column and HB heavy sections and the MC/JC/LC channels,
  102 rows) through the `tapered_i` and `tapered_c` families — the slope column the tables print
  as the inner-face angle over the flange plane carries as data per row (its tangent over 90
  degrees), T enters as the mean flange thickness the standard's annex A formulas use, and the
  seventeen printed rows whose toe radius exceeds the annex-A toe thickness stay out as explicit
  `NOT_FOUND` cases (see `ATTRIBUTION.md`) and, since 2026-11-16, a material library with the
  complete published grade set of IS 2062:2011 (twenty-eight designations: the E250–E410 levels
  in the sub-qualities A, BR, B0 and C, the E450–E650 levels in A and BR), and `environments/china/`
  (2026-11-15) holds the GB/T 706-2016 tapered-flange I-beam and channel series through the
  `tapered_i` and `tapered_c` section families and, since 2026-11-15, the GB/T 5782 hex-bolt
  fastener set (M12/16/20/24, the environment's fastener library, BC-ENV-002) and, since
  2026-11-16, a material library with the complete published grade set of GB/T 1591-2018
  (fifty-three designations across the as-rolled, normalizing and thermomechanical delivery
  families; see `ATTRIBUTION.md`), `environments/usa/` (2026-11-15) holds a material
  library with the ASTM grades A36, A992 and the complete A572 Grade 42–65 set and, since
  2026-11-23, a profile library with the W and HP shapes of ASTM A6/A6M-05a (285 rows of the
  standard's own SI column, the root radius from the Nucor-Yamato catalog that permits its
  use; the declared-area warning is expected on 150 of them; see `usa-astm-a6-profiles.md`)
  and, since 2026-11-24, a fastener library with the RCSC 2025 structural bolt set (the
  F3125 A325 and A490 heavy hex bolts, 1/2 to 1-1/2 in, each row declaring the standard
  hole of Table 3.1; see `usa-rcsc-fasteners.md`) — and `environments/canada/`
  (2026-11-15) is grades-only the same way: the complete weldable category of
  CSA G40.20-13/G40.21-13 (260W, 300W, 350W, 380W, 400W, 480W, 550W). `environments/south-america/`
  (2026-11-23, #711) is grades-only as well, and national: the complete grade set of NCh203.Of2006,
  the official Chilean structural steel standard, in the standard's own designations — the
  general-construction class ES (A240ES, A270ES, A345ES, M345ES, Y345ES) and the dynamic-load class
  ESP (A250ESP, A345ESP). Its status is public:
  [Decreto 231 of 2006 of the Ministry of Public Works declares NCh203.Of2006 an official norm of
  the Republic](https://www.bcn.cl/leychile/navegar?idNorma=249463) and annuls NCh203.Of77 and
  NCh1159.Of77. The environment lists no profile library — the region's profile pack follows the
  #708 route — so a grade is its whole content: `create-project --environment south-america` lets
  a new model select one (`--material A270ES`). The standard's text is published by INN; the rows
  were read from a public technical manual that reproduces the standard's two classification
  tables and cross-checked against Chilean producer tables, with M345ES and Y345ES resting on
  that table alone (sources in `ATTRIBUTION.md`). Finally, `environments/australia/`
  carries, since 2026-11-16, a material library with the AS/NZS 3678-2016 base grades
  (250, 300, 350, 400, 450 — the steel the AS/NZS 3679.2 welded sections are welded from),
  and `environments/middle-east/` (2026-11-23, the curated mix of the owner decision of that
  day) holds one profile library with the EN 10365:2017 IPE, HEA and HEB rows of the `europe`
  pack, sizes 100 to 600 (fifty-five rows carried over unchanged), and a material library with
  the ASTM grades A36, A992 and the complete A572 Grade 42–65 set of the `usa` pack, each
  declared with the edition the regional producer prints (A36/A36M-19, A572/A572M-21,
  A992/A992M-20); the HL, HD and HP series and the sizes above 600 stay `NOT_FOUND`, the
  channels and the BS, American and Australian series are outside the mix, and the environment
  lists no fastener library (see `ATTRIBUTION.md`). `environments/south-africa/` (2026-11-23,
  #714) holds a profile library with the EN 10365:2017 IPE series (IPE 100 to IPE 600, the
  `europe` pack's IPE rows, declared against the live EN string because EN 10365 superseded
  DIN 1025-5; cross-checked against the
  [EMSTEEL Heavy Sections catalogue](https://emsteel.com/pdf/2025/05/heavy-section.pdf)) and a
  material library with the structural grade sets of SANS 50025-2/-3/-4:2009, the national
  adoptions of the EN 10025 2004 editions (twenty-seven designations: S235 to S450 in the JR,
  J0, J2 and K2 qualities, S275 to S460 in N, NL, M and ML; editions per the
  [SABS catalogue](https://store.sabs.co.za/sans-50025-2-2009-ed-1-00.html), grade sets read from
  public text copies of [EN 10025-2:2004](https://www.corrugatedmetalculvert.com/pdf/BS_EN_10025-2-2004.pdf),
  [EN 10025-3:2004](https://www.corrugatedmetalculvert.com/pdf/BS_EN_10025-3-2004.pdf) and
  [EN 10025-4:2004](https://www.corrugatedmetalculvert.com/pdf/BS_EN_10025-4-2004.pdf), all
  retrieved 2026-11-23; see `ATTRIBUTION.md`); the BS UB/UC series is an open owner question and
  the environment lists no fastener library (owner decision of 2026-11-23 in
  `docs/plan/decisions.md`).
* `create-project --environment ID` looks the environment up (built-in, then the user's), reads
  and admits it, then copies it into the project folder (`environment/`) and publishes the two
  default materials. The project therefore carries its environment: closing and reopening it, or
  opening it on another machine, needs no setting and no option. The reply gains
  `seeded {environment, materials}`. An unknown or unreadable environment is refused and the
  folder this call made is removed again. Without `--environment` nothing changes.
* A profile is not copied into the model. `--profile NAME` of `create-member`, `create-polybeam`
  and `create-curved-beam` first looks among the model's definitions; when the name is not there it
  looks in the project's environment and defines that entry in the model (content-addressed, so
  idempotent) before the member is created. This is two revisions, not one: a failed member
  leaves the harmless definition. `definitions` lists the environment entries the model does not
  define yet as `environment_profiles`.
* The window passes the chosen environment to `create-project` and lists those entries in the
  profile picker; a pick that is only in the environment is created with `--profile NAME`.
  The window's refresh reads the listing from `export-view --with-definitions true`, which names the
  store so that `environment_profiles` (and the presets below) reach the picker; before 2026-11-02
  that call named no store, so a project opened in the window offered no environment profile.
* Part presets (2026-11-02): an environment may declare `bim:environment:presets`, a contained
  relative path to a layer of `/Presets/<id>` entries (`steel:preset:kind` `beam`, `column` or
  `plate`, `name`, `class`, optional `material` display name, `profile` designation and, for
  plates, `thicknessMm`). `blank` ships `presets.usda` with three defaults (the first id of a
  kind) and variants: beam `I-300x150`, column `H-200x200`, plate 10 mm, all `Steel`. The file is
  copied into the project with the environment and `definitions` lists it as
  `environment_presets`. Choosing the beam, column or plate tool fills the form with the default
  preset (name, class, material, profile or thickness) and the Preset drop-down switches to
  another; every field stays editable. A refused entry is skipped, not fatal. Saving a preset
  from the window is not done yet.
* Placing beams and columns (2026-11-02): the pointer snaps (endpoint, midpoint, axis, grid and
  so on, as for plates) and the marker is drawn before the first click too. A column is one click:
  the base point, with the top set by the form's height or by a chosen level (`definitions` lists
  the model's `levels`; the top is straight up in world Z to the level's elevation, refused when
  the level is not above the base).
* Default materials: `Steel_Undefined` (density 0) and `Steel` (7850 kg/m3, no grade).

## Starter template (2026-11-22, #840)

A new model opened with no axes and no level, so the first column had nothing to land on. An
environment may now declare `bim:environment:template`, a contained relative path to a layer of
`/Template/<id>` entries (`steel:template:kind` `level` or `grid`; see
`crates/commands/src/template.rs`). `create-project` copies the file with the environment and,
after the default materials, creates each entry in id order through the same engine commands as
`create-grid` and `create-level` (the grid options go through the one `create-grid` parser), one
revision each; the reply counts them in `seeded.grids` and `seeded.levels`. The starter layout
is data of the environment, not of the code: `blank` ships one invented 3 x 3 grid at 6000 mm
(axes 1–3, A–C) and the level `+0.000`; the regional environments declare none yet. Unlike
presets, a template is all or nothing: one refused entry refuses the creation and rolls the new
folder back, since a half-seeded layout would look complete and not be.

## Zero density

`Steel_Undefined` is the material of auxiliary, non-steel parts. Its density is exactly zero, so
its mass is a known `current` 0 kg, not `unknown` (an absent density stays `unknown`). A negative
density is still refused (`INVALID_ARGUMENT`; `GEO_NOT_POSITIVE` in the kernel). A positive
density whose product underflows to zero is still `GEO_NUMERIC_UNCERTAIN`: only an explicit zero
gives a zero mass. The F04 admission "density greater than zero" is amended to "at least zero" in
[the density amendment](../contracts/f04-material-density-amendment-v0.1.md).

## Not done

Real regional catalogues (the policy of `AGENTS.md` 1.2 applies), typed designations such as
`I 300*150*7*10`, user-defined parametric templates, and profile definition from the window.
Libraries in the project folder beyond the copied environment, and the user library, are not
resolved yet.
