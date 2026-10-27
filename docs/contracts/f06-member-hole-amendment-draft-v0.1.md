# F04 and F06 amendment draft: holes in members

Status: **accepted amendment (2026-10-27), current MVP rules for M5 CLI delivery.**
The draft date below records its origin; accepted amendments are versioned alongside the frozen contracts, not a rewrite of them.

Historical proposal status (superseded): **draft for owner review (2026-10-23).** [F04](f04-model-and-commands-v0.1.md) and
[F06](f06-geometry-contract-v0.1.md) 0.1.0 stay unchanged until the owner accepts this text; on
acceptance it is recorded as an amendment of both. It is the contract for G14.1, G14.2 and A04.2
in the [GitHub task catalogue](https://github.com/Apibim-com/BeamCraft/issues/474) and the hole side of the
[bolt amendment](f04-bolt-fastener-amendment-draft-v0.1.md). Today a hole in a beam flange or web is
`UNSUPPORTED_GEOMETRY` (F04 section 7), and the kernel's through hole needs a prism whose axis is
the hole axis (F06 section 2). Bolted beam connections need holes in webs and flanges, so this
text defines them.

## 1. What a member hole is

A member is an extrusion along its axis `Z_m` of a section in the plane `X_r`, `Y_r` (F04 section
3, "Member"). A hole is a straight prism cutter perpendicular to the member axis that crosses one
or more **walls** of the section (a web, a flange, a wall of a hollow section): a bolt through a
web goes along `X_r`, a bolt through a flange along `Y_r`.

A hole is an operation record of the member, as plate holes are (F04 section 4), with these
parameters:

| Parameter | Rule |
|-----------|------|
| `axis_point_mm` | A point `(u, v, w)` on the hole axis in the **member frame** (below), finite, within the coordinate cap |
| `axis_direction` | The unit direction of the axis. First slice: exactly one of `(+-1, 0, 0)` or `(0, +-1, 0)` (the section directions); any other direction is `UNSUPPORTED_GEOMETRY` until G14.2. A direction with a component along the member axis is `UNSUPPORTED_GEOMETRY` too |
| `extent` | `through`, or a range `from_mm < to_mm` measured along `axis_direction` from `axis_point_mm`. `through` reaches past the body on both sides |
| `round` | The generator: `diameter_mm`. The polygon is the regular `N`-gon of the plate hole (F06 A-2 D-2, chord deviation at most 0.05 mm, `N` from 16 to 256, a multiple of four) in the plane perpendicular to the axis; vertex 0 lies on `+Z_m` from the axis point. The polygon is authoritative |

**Member frame.** Origin at `p1` (the first axis point), `u` along `X_r`, `v` along `Y_r`, `w` along
`Z_m` (including the roll of the member). Holes are stored in this frame, so they follow the
member when it is moved or rotated. Editing the axis points or the section replays every record; a
hole that no longer fits (it leaves the member, reaches a free edge, meets a fillet) keeps the
member degenerate with a diagnostic naming the record, as F04 section 4 already says.

Plate holes keep their form (a clockwise polygon in plate-local XY, through the plate); this text
adds nothing for plates.

## 2. Geometry rules

The kernel decides, with exact predicates:

1. **Whole walls only.** Every wall the cutter enters it also leaves: both ends of the cutter lie
   in free space (outside the material), never inside a wall. A cutter ending inside material
   would be a blind hole, which is not supported (decided 2026-10-23: blind holes come after the
   MVP) and is `UNSUPPORTED_GEOMETRY` (`GEO_OP_UNSUPPORTED`).
2. **Inside a flat wall face.** At every wall the hole footprint, the polygon, lies strictly
   inside one planar wall face, clear of that face's edges by at least `eps_geom`. A hole that
   reaches a free edge of a flange, the root of a fillet or a rounded corner is refused
   (`GEO_CONTOUR_INVALID` for a footprint that crosses an edge). Because rolled sections have
   faceted fillets, the hole must stay inside the flat part of the wall.
3. **Parallel walls give exact volume; others an enclosure.** For walls whose faces are parallel
   the removed volume is the polygon area times the wall thickness, exact. For sloped faces (a
   tapered flange) the result is a certified closed shell and the quantity is the midpoint of its
   enclosure (F06 A-2 D-3).
4. **Curved walls are not supported** in the first slice: a hole in a round tube, or a wall that
   is a single faceted curve, is `UNSUPPORTED_GEOMETRY`.
5. **Clear of everything else.** At least `eps_geom` from the member's end faces, from other
   holes and from plane cuts; touching or overlapping is refused as in the plate rules.
6. **Result.** A sealed single shell whose genus grows by one for every wall crossed; new faces
   are `cut` faces. Quantities follow the rule of the plate hole (F04 section 5).
7. The member's section must be a supported extrusion (A07 families and custom outlines);
   a plane-cut member is allowed only when the cut is replayed before the hole and the result is
   still described by rules 1 to 5; otherwise `UNSUPPORTED_GEOMETRY`.

**Capacity.** The existing capacity pre-check (F04 section 4) applies and counts every wall
crossing; above it the command is `LIMIT_EXCEEDED`, `GEO_KERNEL_CAPACITY`. G10 raises the capacity.

**FeatureIds.** The wall of the hole at crossing `k` (counted from the axis point along the
direction, from 0) and side `j` of the polygon is `cut:f.h<ordinal>.w<k>.s<j>`, provisional like
the plate ids and settled by the implementation task (the FeatureId rules of F06 apply: never
reused, never renumbered).

## 3. Commands

The commands of F04 section 7 are extended, with no new command:

| Command | Change |
|---------|--------|
| `AddHole` | A member target is accepted. Arguments for a member: `axis_point_mm`, `axis_direction`, `extent`, `round` (`diameter_mm`). A polygon hole in a member is `UNSUPPORTED_GEOMETRY` in this slice. CLI `add-hole` takes `--axis-point U,V,W --axis-direction X,Y,Z --diameter D [--from F --to T]` |
| `UpdateHole` | Replaces the parameters of one member hole, keeping its ordinal and operation_id |
| `RemoveHole` | Removes one member hole; its ordinal is retired |

The sentence of F04 section 7 "a hole in a beam flange or web is `UNSUPPORTED_GEOMETRY`" is
replaced by the rules above. A hole generated by a bolt group is the same record, tagged with the
group's ElementId (see the bolt amendment): its `extent` covers only the stack the fastener
crosses, so a bolt through one flange does not cut the opposite flange.

## 4. Acceptance cases (to be bound to fixtures)

Positive (a synthetic I-section with web thickness `tw` and flange thickness `tf`):

1. A round hole through the web along `X_r` removes `A * tw`, where `A = (N / 2) * r^2 *
   sin(2 * pi / N)` is the area of its polygon; genus plus one; the new face ids follow section 2.
2. A hole through one flange with a range that ends between the flanges removes `A * tf`; the
   opposite flange is untouched. With `through` along `Y_r` at a position beyond the web it removes
   `2 * A * tf`.
3. A hollow rectangular section hole along `X_r` crosses two walls and removes `2 * A * t`.
4. Moving the member keeps the hole at the same member-frame position; `UpdateHole` changes the
   diameter and keeps the ordinal; `RemoveHole` restores the original volume.
5. A hole in a sloped (tapered) flange gives a certified shell with an enclosure midpoint.

Negative (each changes nothing, or keeps the member degenerate where F04 section 4 says so):

1. A range that ends inside a wall (blind).
2. A footprint touching a free edge, a fillet facet or another hole; a hole closer than
   `eps_geom` to an end face.
3. An axis parallel to the member axis; a direction with an axial component; a direction that is
   not a section direction (`UNSUPPORTED_GEOMETRY` until G14.2).
4. A hole in a round tube.
5. An edit of the member that leaves a hole outside it.

## 5. Provenance

The geometry is the extension of the G05 and G09 operations of our own kernel; no algorithm, text
or table of another product is used. The `A * t` checks are elementary prism volumes.
