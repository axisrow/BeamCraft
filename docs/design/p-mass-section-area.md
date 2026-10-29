# P-Mass: member mass from the section area, gross and net

Date: 2026-10-18. Status: **experimental slice.** Branch `feat/p-mass-section-area`, based on
`origin/master` `11a2139` (A07.1 sections, A07.2 libraries and declared values, D1/D2).

Owner decisions of 2026-10-18 implemented here: (1) the member mass is
`area x axis length x density` with an explicit origin of the area; (2) the quantities are
reported as gross (the nominal body) and net (the body after the operations).

## The problem it fixes

The member volume was `net area of the detailed polygon outline x length`. The polygon replaces
every arc by chords, so it is not the section: the four root fillets of a rolled I section are
overstated. For an I section `h=298 b=149 tw=5.5 tf=8 r=13` (a reference profile given by
dimensions only) over 6 m at 7850 kg/m3:

| Area | mm2 | Mass kg |
|---|---|---|
| Exact (closed form, true arcs) | 4080.0708 | 192.171 |
| Declared catalogue area 4080 mm2 | 4080 | 192.168 |
| Polygon of the detailed outline (old quantity) | about 4088.8 | 192.58 (+0.21 %) |
| Sharp-corner outline, r = 0 (reference only) | 3935 | 185.34 |

The polygon (about 20 edges per turn at r = 13) overstates each concave root fillet by about 2.2 mm2 (6 % of the fillet's own
`(1 - pi/4) r^2 = 36.3 mm2`; 8.7 mm2 for the four). The values are an external reference
observation of the same profile; no external material is included.

## Decisions

1. **Two origins of the area, never none.** `catalogue`: the area the pinned profile definition
   declares (a library entry, or `define-profile --area-mm2`). `computed`: the exact area of the
   section from its parameters. The effective area is the declared one when present, else the
   computed one (`ProfileInfo::effective_area`).
2. **Exact area per family** (`model::section`, next to the parameters; the polygon stays in the
   kernel for geometry, mesh and display only). A quarter-circle fillet between two perpendicular
   edges adds (concave root) or removes (convex bend) exactly `K r^2`, `K = 1 - pi/4`:

| Family | Exact area (mm2) |
|---|---|
| `rectangle` | `w * h` |
| `i_shape` | `2 b tf + (h - 2 tf) tw + 4 K r^2` |
| `channel` | `2 b tf + (h - 2 tf) tw + 2 K r^2` |
| `angle` | `t (h + b - t) + K r^2` |
| `rhs` | `(h b - 4 K r^2) - ((h - 2t)(b - 2t) - 4 K r_i^2)`, `r_i = r - t` if `r > t` else 0 |
| `chs` | `pi t (d - t)` |
| `plain_c` | `t (h + 2b - 2t)` + 2 bend pairs |
| `lipped_c`, `lipped_z` | `t (h + 2b + 2c - 4t)` + 4 bend pairs |
| `omega` | `t (b + 2h + 2c - 4t)` + 4 bend pairs |
| `custom` | shoelace(outer) - sum shoelace(cavities), taken relative to the first point, winding-independent |

   A cold-formed bend pair (inner radius `r`, outer `r + t`) contributes `K (r^2 - (r + t)^2)`
   and nothing when `r = 0`.
3. **Volume and mass.** `volume = effective area x axis length` (`MemberBody::volume_mm3`);
   `mass = volume x density` as before (unknown without a density, never guessed). For a rectangle
   with no declared area this is bit-identical to the old `(w x h) x length`.
4. **The mesh stays the detailed polygon** (display, geometry, snapping, the derived layer), so
   the mesh volume (`SolidMesh::volume_mm3`) and the quantity volume can differ by the arc
   approximation. Bound: a quarter arc of radius `r` split in `m` edges (`theta = pi / (2m)`)
   changes the area by about `pi r^2 theta^2 / 24 = (pi/3) r s` with `s` the sagitta, at most
   `max(0.25 mm, 0.5 % r)`; with at least 16 edges per turn this is at most `0.0202 r^2` per arc
   (`pi^3 / 1536`). The polygon is larger than the section at a concave root fillet and smaller at
   a convex bend. The 30B1-like I section: +8.7 mm2 (+0.21 %) in the mesh.
5. **D3 warning compares with the exact area.** `declared_area_warning` is unchanged in shape
   (`kernel_mm2` keeps its name for compatibility and `exact_mm2` is added, both the exact area).
6. **Gross and net.** `gross` is the nominal body before any operation: members, effective area x
   axis length; plates, the exact outline box (area x thickness). `net` is the body after the
   operations (through holes of a plate today). With no operation they are equal. `removed =
   volume(mesh before) - volume(mesh after)`, measured on the kernel meshes (see below).

## The rule for future operations

`net = gross - removed_volume_mm3`, where `removed_volume_mm3` is MEASURED on the kernel meshes as
`volume(mesh before the operation) - volume(mesh after it)`. Because both meshes carry the same
arc approximation of the detailed outline, that error cancels in the difference, and the net stays
consistent with the exact gross (a cut through a detailed-outline member never subtracts the
polygon error). Neither the mesh volume nor an analytic estimate of the cut is subtracted from
the exact area.

Implemented now: `engine::command::Volumes::measured(gross, mesh_before, mesh_after)`. For a plate
the gross body is the exact box, which is also the mesh before, so the measured volume after the
holes is used as is (no rounding is added; plate net values are bit-identical to the earlier
slice). Not implemented: member operations. A03 does not exist, so a member's
`removed_volume_mm3` is `0` and its net equals its gross. The quantity record and the net
signature are ready: `quantity::member_net_signature(gross, &[operation digests])` takes the list
of operations (each by the digest of its own authored content); it is empty today.

## Quantities JSON (`create-member`, `create-plate`, `inspect-elements`, `copy`, `update-element`, ...)

`quantities` keeps `volume.{mm3,m3,status,algorithm}` and `mass.{kg,density_kg_m3,status}`. **These
old keys are aliases of the NET values**, so existing consumers keep working (for a member without
operations net equals gross; for a plate with holes they are the values they always were). New:

| Key | Meaning |
|---|---|
| `volume_gross_mm3`, `volume_net_mm3`, `removed_volume_mm3` | full precision |
| `mass_gross_kg`, `mass_net_kg` | full precision, null when the mass is unknown |
| `mass.kg_display`, `mass_gross_kg_display`, `mass_net_kg_display` | rounded to 0.1 kg for display only |
| `section_area_mm2`, `area_source` | members: the effective area and `catalogue` or `computed`; null for a plate |

Stored values are never rounded. `inspect-elements` adds `expected_gross_signature`,
`stored_algorithm` and `superseded_algorithm` to `stored_quantities`.

## Signatures and freshness (P04 rules)

| Signature | Inputs |
|---|---|
| member gross (`member_gross_signature`) | axis, roll, resolved section values, area source, declared area and effective area, algorithm, tolerance policy |
| member net (stored as `bim:shapeSignature`) | the gross signature and the list of operations (empty) |
| plate gross (`plate_gross_signature`) | outline, thickness, the gross algorithm label (no holes) |
| plate net (stored; unchanged bytes) | outline, thickness, holes, algorithm |
| mass (net and gross) | the matching shape signature and the density or its absence |

Changing a profile's declared area (or the density) changes the signature, so a stored quantity
computed from another area reads `stale` (tested). A hole edit changes the plate's net signature
and net volume and neither the gross signature nor the gross volume (tested). The derived layer
stores the NET volume, mass and signatures only: **the stored attribute set and the schema digest
are unchanged**; the gross figures, the removed volume, the area source and the gross signatures
are computed from the authored inputs and reported next to the stored net record.

## Compatibility with existing stores

New algorithm labels: `pmass-member-area-x-length/1;rectangle;tolerance-policy-3` and
`pmass-member-area-x-length/1;section-exact-area;tolerance-policy-3`. The plate labels and the
plate net signature are unchanged, so existing plate records stay `current`.

A member record written by an earlier binary carries a superseded label
(`engine::member::SUPERSEDED_MEMBER_ALGORITHMS`) and different signature bytes. It reads as
`stale` (volume and mass; `superseded_algorithm: true`), the engine reports its own exact values,
and the store stays fully usable: the strict publish-time verifier accepts a stale record with a
superseded label (it is a value to recompute, not damage) and still refuses any other record that
disagrees with the kernel, including one that carries the current label (tested). The record is
rewritten as `current` by the next edit that rewrites the element's quantities (tested with a
material `update-element`). There is no dedicated "recompute quantities" command yet.

## Not done

- **Dropped by the owner's wrap-up request (2026-10-18): `mass_simplified_kg`**, the optional
  reference mass of the sharp-corner (simplified) section, and the `simplified_area_mm2` helper
  behind it. The sharp-corner figure (185.34 kg for the reference I section) appears only in the
  table above, as an external reference.
- No member cuts, bolt holes, copes (A03 does not exist); only the structure is ready.
- Gross figures, the removed volume and the area source are not stored in the derived layer (a
  schema change); a plain USD reader sees only the net record and the signatures.
- No recompute command for superseded records; they are refreshed by the next quantity-writing edit.
- The declared mass per metre and the declared surface are still not read; mass uses the area.
- A rectangle's `kernel_mm2`-style D3 warning is not added (the rectangle keeps its refusing P02.2
  0.1 % check).
- The IFC contract ([e01-ifc-export-v0.1](../contracts/e01-ifc-export-v0.1.md)) maps one volume to both
  `NetVolume` and, only for elements without operations, `GrossVolume`; it is not changed here.
  With explicit gross and net figures a plate with holes could also publish `GrossVolume` (open).

## Bill of materials: proposal pending the owner

Not hard-coded anywhere. Proposal: **gross** for the material order and purchase (the stock that
must be bought: nominal body before cutting), **net** for the fabricated or shipping weight (what
the finished part weighs). Both are available now; the BOM chooses by use.

## Evidence

- `cargo test -p beamcraft-model section`: hand values of the exact area of every family (30B1-like
  I 4080.0708; channel, angle, RHS with outer and inner radii, CHS ring, cold-formed sums), the
  sharp-corner reference, a custom area independent of winding and of the distance from the origin.
- `cargo test -p beamcraft-kernel --lib section`: for every family a polygon with 16,384 edges per
  turn converges to the exact area (1e-7 relative, error falling with the square of the edge
  length), the sharp polygon equals the closed form to 1e-12, and the stored detailed polygon of
  the I section overstates the exact area (< 0.35 %) while the exact area matches the closed form
  to 1e-9.
- `cargo test -p beamcraft-engine --lib`: 30B1 192.17 kg from the exact area (the mesh volume
  gives the old 192.58 kg, +0.21 %), declared 4080 gives 192.168 kg, hostile declared areas
  refused, rectangle, custom, the warning against the exact area, gross and net of a 1000 x 500 x
  10 plate with two holes (5,000,000 and 4,875,000 mm3, removed 125,000, 39.25 and 38.26875 kg),
  signature and freshness tests (declared area, future operation, hole edit, superseded label).
- `cargo test -p beamcraft-cli --test cli -- pmass::` (real binary, 9 tests): the 30B1-like member computed
  (volume = 4080.0708 x 6000 to 1e-9, mass 192.168 within 0.01 kg, display 192.2, gross equals
  net), the same with `--area-mm2 4080` (catalogue, 192.168),
  the warning against the exact area, no density (mass null, volumes present), custom polygon,
  rectangle (computed and declared), members from `assets/libraries/synthetic-sections.usda`
  (catalogue and computed entries), a plate with holes, a store from before this slice (stale,
  usable, refreshed by an edit) and a wrong record under the current label (still
  `INTEGRITY_ERROR`).
