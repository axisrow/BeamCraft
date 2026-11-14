# F06 and D01 amendment: cut cap corners

Status: **amendment, accepted when the PR that adds it merges (owner decision 2026-11-14).**
[F06](f06-geometry-contract-v0.1.md) 0.1.0 stays frozen and
[F06 0.2 profile features](f06-profile-features-v0.2.md) keeps its text; this document is
recorded as an amendment of both and of the [D01 drawing contract](d01-drawing-contract-v0.1.md)
section 4. Everything not named here keeps the rules of those documents.

## Why

A plane cut or a fitted end shortens a member and removes its `z1` (or `z0`) end face. The end
vertices `member:v.….z1` and `profile:v.z1.<t>` are clipped and dangle, and F06 0.1 section 5
leaves the vertices the operation creates unaddressable. A drawing could therefore not dimension
the length of a cut member at all: no qualified vertex existed at the cut end.

## 1. Names

The corners of the planar cap of plane cut `n` are named. A cap corner is a vertex shared by
exactly three faces: the cap `cut:f.p<n>` and two faces of the member itself whose common edge
runs along the member and is crossed by the cut plane:

- a box member (`fid1:…:member:*`): `fid1:<ElementId>:cut:v.p<n>.<a>.<b>`, where `<a>` and
  `<b>` are the two side face roles without `f.`, the `x` face first (`x0.y1`, `x1.y0`). The
  member axis is local `z`, so the longitudinal edges are exactly the four `x`/`y` pairs; a pair
  with `z0` or `z1` is an end-face edge and is not a cap corner name;
- a profile member (`fid2:…:profile:*`): `fid1:<ElementId>:cut:v.p<n>.<t>`, where `<t>` is the
  contour point of the longitudinal edge, the same `o<k>` or `c<j>.<k>` token as
  `profile:e.v.<t>` and `profile:v.z0.<t>`.

The cut's ordinal rules apply unchanged (assigned by the engine, never reused or renumbered,
at most 999), so a corner keeps its name while its cut keeps its ordinal and the section keeps
its topology; moving the cut plane moves the corner, not its name. The canonical decimal
spelling and the 64-byte qualified limit apply; the longest form,
`fid1:<uuid>:cut:v.p999.c255.1023`, is 62 bytes.

Not named, and still unaddressable as in F06 0.1 section 5:

- a vertex where the cap meets an end face (`z0`, `z1`), another cut's cap or a hole wall;
- a vertex on a triangulation diagonal or on a grouped approximated wall;
- the edges of the cap.

A corner name that the evaluated body would give to more than one vertex (a cap crossing one
member edge twice) is ambiguous. Such a name is left out on its own; it is never repaired by
choosing a mesh index, and the other names of the body stay addressable.

## 2. Drawing anchors (D01 section 4)

A dimension anchor is a qualified vertex of the drawing's selected member: a vertex of its own
body (`member` for a box member, as before, or `profile` for a profile member, whose vertices
F06 0.2 named after D01 0.1 was written) or a cap corner `cut` of that member as defined above.
Every other admission rule of D01 section 4 and section 5 is unchanged: the element must be the
selected member, the feature kind must be `vertex`, and the two anchors must be distinct
semantic references. An anchor whose cut was removed resolves as explicitly unresolved, never
as another vertex, so a draft over it cannot be published.

## Acceptance

- Parsing: `cut:v.p<n>.<a>.<b>` and `cut:v.p<n>.<t>` are canonical; two faces on one axis, a
  pair with an end face (`z0`, `z1`), a wrong axis order, a non-canonical decimal and an
  ordinal above 999 are refused.
- A box member and an I-section member cut by a plane list their cap corners; each corner lies
  on the longitudinal edge its name gives. A bevel cut that leaves through the end face names
  only the corners on longitudinal edges.
- A drawing of a cut member anchored on an end vertex and a cap corner is created, measures the
  kept length, publishes and exports.
