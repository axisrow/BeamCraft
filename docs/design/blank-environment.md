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
  `environments/europe/` holds the real EN 10365:2022 IPE, HEA and HEB nominal dimensions, the
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
  `NOT_FOUND` cases (see `ATTRIBUTION.md`), and `environments/china/`
  (2026-11-15) holds the GB/T 706-2016 tapered-flange I-beam and channel series through the
  `tapered_i` and `tapered_c` section families and, since 2026-11-15, the GB/T 5782 hex-bolt
  fastener set (M12/16/20/24, the environment's fastener library, BC-ENV-002) and, since
  2026-11-16, a material library with the complete published grade set of GB/T 1591-2018
  (fifty-three designations across the as-rolled, normalizing and thermomechanical delivery
  families; see `ATTRIBUTION.md`), `environments/usa/` (2026-11-15) holds, so far, only a material
  library with the ASTM grades A36, A992 and the complete A572 Grade 42–65 set — the USA
  profile pack waits on the #649 provenance audit, so the environment lists no profile or
  fastener libraries and the grades are its whole content — and `environments/canada/`
  (2026-11-15) is grades-only the same way: the complete weldable category of
  CSA G40.20-13/G40.21-13 (260W, 300W, 350W, 380W, 400W, 480W, 550W), and `environments/australia/`
  carries, since 2026-11-16, a material library with the AS/NZS 3678-2016 base grades
  (250, 300, 350, 400, 450 — the steel the AS/NZS 3679.2 welded sections are welded from).
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
