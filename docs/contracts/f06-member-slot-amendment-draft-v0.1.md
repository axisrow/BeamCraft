# F06 amendment draft: rectangular through-slots in members (the gusset slot)

Status: **draft for owner review (2026-11-14).** On acceptance this is recorded as an amendment
of [F06](f06-geometry-contract-v0.1.md) (0.1 and 0.2) and of the
[member-hole amendment](f06-member-hole-amendment-draft-v0.1.md). It is the narrow contract for
the first standalone slice of [the cut/slot/drill epic #556](https://github.com/axisrow/BeamCraft/issues/556):
a rectangular through-slot in a hollow rectangular section so a gusset can pass through it.

Today a member hole accepts only the round generator (`diameter_mm`); the engine refuses any
other member-hole generator with `UNSUPPORTED_GEOMETRY`, and a member target of `cut-by-polygon`
or `cut-by-part` stays refused. The kernel's certified member-hole cutter, however, already
sweeps an arbitrary polygon cavity ("round, oval or polygonal, clockwise in the section plane")
over any certified section direction, and the stored `MemberHoleRecord` already holds an
authoritative polygon. This slice is therefore the input edge only: a rectangular generator on
the existing record family. No kernel change, no record schema change, no replay change.

## 1. What a member slot is

A slot is a member-hole record (the member-hole amendment section 1) whose generator is a
rectangle instead of a circle:

| Parameter | Rule |
|-----------|------|
| `axis_point_mm`, `axis_direction`, `extent` | Unchanged: the member-frame point, the section-plane direction (G14.2), `through` or a free-ending range |
| `slot_width_mm` | The rectangle extent along `axis_direction x Z_m` (the plate-thickness direction of a gusset slot), finite, greater than zero, within the coordinate cap |
| `slot_height_mm` | The rectangle extent along `Z_m`, finite, greater than zero, within the coordinate cap |

The rectangle is centred on the axis point, its sides parallel to the hole-frame axes
(`x` along `Z_m`, `y` along `axis_direction x Z_m`). The generator emits its four clockwise
corners in the hole frame; the stored polygon is authoritative exactly as for the round
generator, so the record, its ordinal and the `steel:mhole:*` attribute family are unchanged and
old records replay identically. A gusset slot through an SHS column is `slot_width_mm` = plate
thickness plus clearance, `slot_height_mm` = the gusset's height in the section plane.

Distinct from bolt holes: the owner decision of 2026-10-23 keeps bolt-generated hole types
beyond the round through hole (oversize, slots, per-part holes, blind) after the MVP. This
amendment is about an authored member cut; a slot record is not a fastener hole and never
numbers as one.

## 2. Geometry rules

The member-hole amendment's rules 1 to 7 apply verbatim; nothing new is added:

1. Whole walls only: both cutter ends in free space; a blind range is refused.
2. At every crossed wall the rectangular footprint lies strictly inside one planar wall face,
   clear of its hard edges, corner radii, other holes and plane cuts by the geometric floor.
3. Parallel flat walls give the exact removed volume `A_slot x t / |n . d|` (a perpendicular
   cut: `A_slot x t` per wall; a through-slot in a hollow rectangular section crosses two
   walls); sloped walls give the certified enclosure midpoint.
4. The engine's served-frames rules are unchanged: a straight member whose frame axes are the
   coordinate axes, no roll, no axial direction component; records replay in ordinal order and
   the existing refusal rules for mixed or oblique record sequences apply unchanged.
5. Sections: the acceptance fixtures bind flat-wall sections (the rectangle profile and the
   hollow rectangular families of A07). A slot on a CHS is not an acceptance target of this
   slice; the existing admission decides and no new rule is added for it.

**FeatureIds.** The slot's wall bands follow the existing grammar
`cut:f.h<ordinal>.w<k>.s<j>` with `j` in 0..3, provisional like the other cut ids. This slice
names no vertices: the slot's corner vertices stay unnamed. The plane-cut cap-corner naming of
PR #623 (`cut:v.p<n>...`) is the precedent for a later amendment; this contract pre-claims
nothing beyond the band ids.

## 3. Commands

No new command; the member forms of F04 section 7 gain the second generator:

| Command | Change |
|---------|--------|
| `AddHole` | The member generator accepts round **or** rectangular. The arguments are exclusive: `--diameter` on one side, `--slot-width` + `--slot-height` on the other; mixing them is `INVALID_ARGUMENT` |
| `UpdateHole` | Replaces the rectangle like the diameter, keeping the record's identity and ordinal |
| `RemoveHole` | Unchanged |

CLI `add-hole` for a member target:

```text
add-hole --id MEMBER --axis-point U,V,W --axis-direction X,Y,Z
         (--diameter D | --slot-width W --slot-height H) [--from F --to T]
```

Still refused, unchanged: an arbitrary `--polygon` on a member, and a member target of
`cut-by-polygon` / `cut-by-part` (epic sub-tasks 1 and 2: full prism/CSG subtraction and the
oblique tool are separate slices; this contract covers neither).

## 4. Quantities

No mechanism change. The replay rebuilds mesh, volume and mass in the same transaction; the
member quantity triple keeps its meanings (gross = the uncut P-Mass volume, removed = the exact
or enclosure-midpoint volume of the constructed openings, net = gross − removed), and BOM,
reports and IFC read the net numbers unchanged (F04 section 5, P04, E03). For the acceptance
fixtures the removed volume is exact and elementary: a perpendicular through-slot in an SHS
`h x h x t` removes exactly `2 x slot_width_mm x slot_height_mm x t`.

## 5. Acceptance cases (to be bound to fixtures)

Positive:

1. An SHS 200x200x6 column, frame on the coordinate axes; slot 12x160 through along `X_r` at
   mid-height: removed volume exactly 23040 mm3 (the engine's 1 mm3 bound), genus +2, walls
   named `cut:f.h0.w0.s0..3` and `cut:f.h0.w1.s0..3` in the feature table, gross unchanged.
2. A solid rectangle profile 200x300: a through slot along `Y_r` removes `A_slot x 200`; the
   quantity triple agrees to 1e-6 relative; two evaluations return byte-equal bodies.
3. A range reaching 10 mm past both walls removes the same volume as `through`; blind ranges
   refuse. An oblique section direction removes `A_slot x t / |n . d|` per wall.
4. `UpdateHole` changes the rectangle and keeps the ordinal; `RemoveHole` restores the gross
   volume; moving the member keeps the slot at its member-frame position.
5. CLI end to end: create a project and the SHS member, `add-hole` with the slot pair,
   `list-feature` shows the eight band ids, the reported net volume equals gross − 23040,
   and removal restores the gross through the ordinary replay.

Negative (each changes nothing, or keeps the member degenerate per F04 section 4):

1. `--diameter` together with the slot pair; a zero or negative side; non-finite values.
2. A footprint crossing a wall edge, a corner radius, another record or a plane cut.
3. An axial direction, a rolled or non-coordinate frame, a curved member or polybeam: the
   existing typed refusals, byte for byte.
4. An arbitrary polygon on a member, and a member target of `cut-by-polygon` /
   `cut-by-part`: still `UNSUPPORTED_GEOMETRY`.

## 6. Provenance

The geometry reuses the project's own certified kernel operation; no algorithm, text or table
of another product is used. The acceptance checks are elementary prism volumes. No new assets,
no new dependencies.
