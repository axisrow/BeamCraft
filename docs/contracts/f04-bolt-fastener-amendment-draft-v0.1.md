# F04 amendment draft: fastener definitions, bolt geometry and bolt groups

Status: **draft for owner review (2026-10-23).** [F04](f04-model-and-commands-v0.1.md) 0.1.0 and
the domain vocabulary stay unchanged until the owner accepts this text; on acceptance it is
recorded as an amendment and the sections it replaces get a dated pointer. It is the contract for
the slices A14.1 to A14.4 and V19 in the [GitHub task catalogue](https://github.com/Apibim-com/BeamCraft/issues/474) and extends the minimal bolt
group of [A05](../design/a05-bolts.md), which is an annotation with no geometry. The behaviours
come from the owner's description of 2026-10-23 ([decisions](../plan/decisions.md)); the model
and wording are our own.

## 1. What changes

| Today (A05) | After this amendment |
|-------------|----------------------|
| A group is a line of identical annotations; diameter is a bare number | A group places a **fastener**: a pinned definition from a fastener library, with real geometry |
| No head, shank, nut or washer; no volume | Head, shank, optional nut and washers as closed meshes; counts and mass per component |
| The group never touches the connected parts | A setting decides whether the group generates **holes** in the parts it passes through, or none (screws) |
| Origin, direction, count, pitch | Two points, a reference plane with rotation and position values, offsets and spacings; one edit command |

## 2. Fastener definition and library

A fastener definition is the data of one size of one standard (for example a hex bolt of a given
size and grade). It follows the profile mechanism of
[A07](../design/a07-section-library.md):

* A definition lives in a **fastener library**, a USD layer in the format and with the loading
  rules of the profile libraries in A07 section 6 (local and contained paths, untrusted-file
  limits). **Environments are not part of this work** (owner decision 2026-10-23): there is no
  per-country choice yet. The first slice has one library, with **one European DIN set** (a hex
  bolt, its nut and its washer); the user picks a size from it. Selecting a library through an
  environment is added later without changing a definition or a group.
* When a fastener is created, the definition is **pinned**: copied into the project as an
  immutable definition addressed by its content digest, as profile definitions are. The model never
  needs the library again; changing or deleting a library changes no existing group. A group takes a
  newer definition only by an explicit edit (section 6).
* An unknown designation is refused and never replaced by a near one.

A definition carries, all as authored numbers in millimetres:

| Field | Meaning |
|-------|---------|
| `designation`, `standard`, `grade` | Text labels shown to the user and in the BOM |
| `kind` | `bolt` (through the parts, nut at the far end) or `screw` (threaded fastener, no nut) |
| `nominal_diameter` | Shank diameter |
| head | `head_kind` (first slice: `hex`), `head_across_flats`, `head_height` |
| nut | `nut_across_flats`, `nut_height`; absent for a `screw` |
| washer | `washer_outer_diameter`, `washer_inner_diameter`, `washer_thickness`; optional |
| holes | the clearance of the standard hole (hole diameter = `nominal_diameter` + clearance); the oversize and slot sizes follow after the MVP |
| lengths | the list of standard lengths; `thread_protrusion` (the minimum shank length beyond the nut) |
| `mass_each` | Optional declared mass of the bolt, nut and washer; otherwise mass is computed from volume and the density of the definition's material |
| defaults | `washer_under_head`, `washer_under_nut` and `hole_mode` as the standard prefers them |

**Data and licence.** The repository ships the mechanism and one synthetic example library with
contributor-original values marked as not a standard (as for profiles, A07 D1). The DIN set enters
as our own data, each value with its recorded source and licence (`AGENTS.md` section 1.2): no text
or table is copied from a standard document or a vendor catalogue, and no value is added without a
provable source. Which DIN or ISO numbers are used and who enters the values is open (O-B2); DIN
bolt, nut and washer standards have ISO counterparts and the designation text is only a label.

## 3. Geometry

Each fastener is built from our own kernel solids, no new kernel operation: a hex head and nut are
extrusions of a regular hexagon (G05); the shank is an extrusion of a regular polygon; a washer is
an extrusion of a polygon with a circular cavity.

* Round parts are regular polygons with a chord deviation of at most 0.05 mm and a vertex count
  that is a multiple of four (the rule of the round hole, F06 A-2 D-2).
* **Threads are not modelled**: the shank is a plain cylinder of the nominal diameter. Head and nut
  edges are not chamfered in the first slice.
* Components are separate closed meshes (head, shank, nut, washers) so that colour, quantities and
  the BOM read them one by one. A group of `n` fasteners shares one prototype mesh placed `n` times
  (instancing, U11), so a group costs one mesh, not `n`.
* Quantities per group: the count of every component, the volume of each, and the mass, from
  `mass_each` when the definition declares it and from volume and density otherwise, with the
  source of the mass shown (declared or computed), as the P-Mass decision does for sections.
* A fastener has no material in the model; a group never changes the volume or mass of a part,
  except through generated holes (section 5).

## 4. Group placement

A group is a straight pattern of identical fasteners. The user gives **two points** that define
the line the group follows, a **reference plane** with a rotation and position values, the offsets
and the spacings.

| Input | Rule |
|-------|------|
| `point1`, `point2` | World points in millimetres. The group follows the line `point2 - point1`; they must be distinct. The unit vector along it is `x` |
| `plane_normal` | The normal of the reference plane the group is laid in (the plane the user works in; a UI passes the active plane). Its component perpendicular to `x` gives the shank axis `z` at zero rotation; parallel to `x` is `INVALID_ARGUMENT`. `y = z × x` |
| `rotation` | Angle in degrees about `x`, default `0`; it turns `y` and `z` together. A UI offers the quarter turns as presets. This is how a group is aimed at a web or a flange |
| `on_plane` | Offset of the whole group along `y` (in the rotated frame), default `0` |
| `depth` | Offset of the whole group along `z`, default `0`. The bearing face of the head (under the head washer if there is one) lies at `point1 + on_plane * y + depth * z`, and the shank runs along `+z` towards the nut |
| `offset_start` | Distance along `x` from `point1` to the first fastener |
| `spacings` | The distances between neighbouring fasteners, a list; unequal values are allowed. `n = len(spacings) + 1` |
| generators | Kept for editing and expanded to `spacings`, which stay authoritative: `count` + `pitch` (equal spacing); or `count` + `offset_end` (equal spacing so that the last fastener lies `offset_end` before `point2`, re-expanded when the points move) |

The axis is therefore not a separate input: it follows from the reference plane, the rotation and
the position values. One group is one straight row. A rectangular pattern is several groups
(owner decision 2026-10-23); circular and free-point patterns are not planned for the MVP. Group
size is 1 to 512 fasteners (the cap of A05).

**Connected parts.** A group lists two or more plates, beams or columns (the `connects` record of
A05 allows 2 to 1024; A05 limited it to two). The first is the main part. Every listed part must be
crossed by the axis of at least one fastener, else `INVALID_ARGUMENT` (`GEO_OP_NO_EFFECT`).

**Length.** `length_mode` is `auto` (default) or `fixed` with a value from the definition's list.
`auto` is the smallest standard length that covers the grip (the length of the axis inside the
listed parts) plus the washers, the nut and `thread_protrusion`; when the grip differs between
fasteners the largest one decides, so every fastener in the group is identical. No standard length
long enough is `UNSUPPORTED_GEOMETRY`.

**Washers.** `washer_under_head` and `washer_under_nut` are `true` or `false`, defaulting from the
definition. A definition without a washer refuses `true`. A `screw` has no nut, so no washer
under a nut.

## 5. Holes: generated or none

`hole_mode` is a setting of the group:

| Value | Behaviour | Default for |
|-------|-----------|-------------|
| `generate` | A through hole is cut in **every** listed part at every fastener the shank crosses, with the hole diameter `nominal_diameter + clearance` (the definition's clearance, or the group's own `hole_clearance`); depth is always through the part. Blind holes and different hole types per part come after the MVP | `kind = bolt` |
| `none` | No hole is cut and no part is changed; the shank is drawn through the material (self-tapping screws, studs welded on, anchors) | `kind = screw` |

* Generated holes are ordinary hole **operation records** of the connected parts (F04 section 4);
  each carries the ElementId of the group that generated it. They are created, replayed, moved and
  removed together with the group in one transaction and one undo unit. Editing the group
  regenerates its holes; deleting the group removes them; no hole of the group is edited
  separately (`UpdateHole` and `RemoveHole` of a generated hole are refused with
  `DEPENDENCY_CONFLICT`, naming the group).
* The group now **writes** the parts' operation records, where A05 only read the parts. Its
  footprint declares those writes, so a concurrent edit of a part conflicts as for any operation.
* Hole numbers do not change the numbering rule: a plate's holes are part of its fabrication
  state (A08), so generated holes enter its equivalence.
* Holes in members (a web or a flange) are specified by the
  [member hole amendment](f06-member-hole-amendment-draft-v0.1.md). A generated hole in a member
  is a hole record with an axis and an extent that covers only the stack the fastener crosses, so
  a bolt through one flange does not cut the opposite flange. Until that contract is implemented
  (G14), a group on a member with `hole_mode = generate` is refused with `UNSUPPORTED_GEOMETRY`
  and nothing is written; `none` works at once.
* A hole that cannot be cut (outside the part, capacity, a kernel refusal) refuses the whole
  command and changes nothing.

## 6. Commands

| Command (CLI) | Arguments | Behaviour |
|---------------|-----------|-----------|
| `CreateBoltGroup` (`create-bolt`) | new_id, name, `parts` (2 or more), `fastener` (designation, with `--fastener-library FILE`), `point1`, `point2`, `plane_normal`, `rotation`, `on_plane`, `depth`, `offset_start`, spacings or a generator, `washer_under_head`, `washer_under_nut`, `hole_mode`, optional `hole_clearance`, `length_mode` | Resolves and pins the definition, validates, builds the group, generates the holes, in one transaction |
| `UpdateBoltGroup` (`update-bolt`) | target_id, changes: any of the inputs above, plus `repin` (take the current library definition) | One atomic edit and one undo unit; holes are regenerated; a failure changes nothing. `repin` is the only way a group follows a changed library |
| `DeleteElement` | target_id | Deletes the group and its generated holes; the connected parts stay |

`create-bolt` of A05 (origin, direction, count, pitch, diameter) is replaced. Groups stored in the
A05 form stay readable as annotation-only groups (no definition, no geometry, no holes) and are
upgraded by an `UpdateBoltGroup` that supplies a fastener; they are not migrated silently.

## 7. What the UI does (V19, not engine contract)

The tool first asks for the fastener (standard, size) from the library, then the washer switches
and the hole setting, then the two points on the active plane, then the position values
(`on_plane`, rotation, depth) and the offsets and spacings in a numeric panel; the preview shows
the fasteners and the holes before anything is committed. Editing selects the group and calls
`UpdateBoltGroup`. The UI never decides geometry: it calls the same commands as the CLI.

## 8. Dependencies

* A14.1 needs the library format and the digest pinning of A07; it does not need environments.
* A14.2 needs U11 (instancing) for the group cost; without it the group draws `n` meshes.
* A14.4 needs the operation records for holes (A04 acceptance, A-2) and, for members, G14 and the
  [member hole amendment](f06-member-hole-amendment-draft-v0.1.md).
* Bolts through a rotated part need P10.

## 9. Acceptance cases (to be bound to fixtures)

Positive:

1. A synthetic definition with a hex head of height `k`, a nut of height `m` and one washer under
   the head gives, for one fastener, component volumes equal to the analytic hexagon, cylinder
   and annulus volumes within the chord rule; the group volume is `n` times that.
2. `auto` length picks the smallest listed length not shorter than grip + washers + nut +
   protrusion; `fixed` takes a listed value.
3. Two points and offsets place the fasteners at `offset_start`, then each spacing; unequal
   spacings are kept; `count` + `pitch` expands to the same list; `count` + `offset_end` ends the
   row at `offset_end` before `point2` and follows a moved `point2`.
4. The position values: with `plane_normal` the plate's normal and rotation `0` the shank is
   perpendicular to the plate; rotation `90` turns it about the line; `on_plane` slides the row
   across the plane; `depth` moves the head along the shank.
5. `hole_mode = generate`: the plates get one hole per fastener of diameter
   `nominal_diameter + clearance`, the group and its holes undo as one unit; deleting the group
   removes the holes.
6. `hole_mode = none`: no part is written; the parts' volumes are unchanged.
7. A group pins its definition: deleting the library changes nothing; `repin` follows the library.
8. Two groups with the same inputs under different `op_id` are two groups; the BOM counts
   components once.

Negative (each changes nothing):

1. An unknown designation; a missing library; a washer the definition lacks.
2. `point1 = point2`; `plane_normal` parallel to the line.
3. A listed part the shank never crosses; no standard length long enough.
4. A generated hole refused by the kernel (outside the part, capacity); the whole group refused.
5. Editing a generated hole directly.
6. A member with `hole_mode = generate` before G14.

## 10. Decided and open

* **Decided (O-B1, 2026-10-23).** A rectangular arrangement is several groups; one group is one
  row.
* **Decided (O-B2, 2026-10-23).** No environments for now. The first fastener library holds one
  European DIN set.
* **Decided (O-B3, 2026-10-23).** Hole types beyond the round through hole (oversize, slots,
  different holes per part, blind holes) come after the MVP.
* **Open (O-B2a).** Which DIN or ISO numbers make the first set (a hex bolt, a nut, a washer) and
  who enters the values with a recorded source and licence.

## 11. Provenance

The solids are elementary extrusions of our own kernel; the grip and length rules are our own; no
text, table, geometry or asset of another product or a vendor catalogue is used. The example
definition values are invented for the tests and are not any standard's.
