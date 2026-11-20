# F06 amendment draft: edges and vertices of prism-cut walls

Status: **draft for owner review (2026-11-20).** On acceptance this is recorded as an
amendment of [F06](f06-geometry-contract-v0.1.md) 0.1 (section 5) alongside the
[cut cap corners amendment](f06-cut-cap-corners-amendment-v0.1.md), whose text it keeps. The
ids are provisional like every other `cut` id (F06 0.1 section 5). It is the contract for
[epic #556 slice 5](https://github.com/axisrow/BeamCraft/issues/556): picking and snapping on
the cut-generated edges and vertices of member bodies, through the same `LookupFeature`,
`ListFeatures`, `Pick` and `Snap` surfaces the box and profile worlds already use. Engine API
and tests only; interactive assistance needs no CLI command (`AGENTS.md` section 3).

## Why

F06 0.1 section 5 leaves every operation-created edge and vertex unaddressable, and the
cap-corners amendment names only the plane-cut corners. The seams of a polygon cut, the walls
of an oblique part cut and the rims of a member hole therefore had walls with names
(`cut:f.h<n>…`) but no edge to snap a weld line to and no vertex to dimension from: a pick on
the opening's boundary answered nothing and a snap resolved nothing.

## 1. Names

A **prism cut** is the wall set one operation record adds: a round, polygonal or part-cut
cutter swept through the walls of a body. Its walls carry the cutters' own grammar
(`cut:f.h<n>` for a one-wall curved opening, `cut:f.h<n>.s<k>` for a polygon side, with the
member cutter's `.w<crossing>` and `.p<run>` suffixes). This amendment names, on a body whose
own face is one axis-aligned rectangle role (`plate:f.…` or `member:f.…`, the flat-wall served
set of the member replay):

- **rim vertices** — a vertex of exactly two band walls of one opening and one axis-aligned
  face of the body itself:
  `fid1:<ElementId>:cut:v.h<n>[.w<k>].<axis>.<j>`, where `<n>` is the record ordinal,
  `<k>` the crossing, `<axis>` the body face role without `f.` the rim lies on, and `<j>`
  the cavity point the vertex closes (the start of side `<j>` in the cutter's clockwise
  ring order);
- **rim edges** — the opening-boundary segment between a band wall `s<j>` and the body face:
  `fid1:<ElementId>:cut:e.h<n>[.w<k>].s<j>.<axis>`;
- **corner edges** — the cut edge between the two band walls of one crossing at cavity
  point `<j>`:
  `fid1:<ElementId>:cut:e.h<n>[.w<k>].v<j>`.

The canonical decimal spelling, the component order and the bounds follow the walls' own
grammar: ordinal and crossing at most 999, cavity point at most 1023, and the 64-byte
qualified limit as F06 0.1 section 5 states it. The longest qualified rim form that fits is
`fid1:<uuid>:cut:v.h999.w999.z1.999` (64 bytes); a spelling the limit refuses stays unnamed
on its own, and the rest of the body keeps its names.

The names follow F06 section 5's rules unchanged: derived from the face table and topology,
never from indices; stable while the record keeps its ordinal and the section keeps its
topology (a move keeps every name and moves the feature); a copy re-quals them under the new
ElementId. A rim name the evaluated body would give to more than one vertex is ambiguous and
is left out on its own (the cap-corner amendment's rule).

## 2. Not named

- a rim or corner on a faceted wall, a contour wall or any non-axis-aligned face (hollow
  sections, I-sections, CHS): the walls themselves stay addressable, the rim stays unnamed
  until an amendment names it;
- a vertex of the plate cutter's one-wall curved opening (the round plate hole);
- the plane cut's cap edges, a cap-to-band edge, and any edge a cap corner takes part in
  (the cap-corner amendment names cap corners only);
- a rim vertex split by a `.p<run>` band side of a multi-facet chain (the run boundary on a
  rim is not a cavity corner).

## 3. Refusals

A well-formed cut edge or rim id over a body that names no such feature resolves as
`GEO_FEATURE_NOT_FOUND` — explicitly, never as another feature and never silently empty. A
malformed token is `GEO_FEATURE_ID_INVALID` as before. Record kinds that add no prism walls
(plane cuts, fits, chamfers) name no cut edges; their cap corners keep the cap-corner
amendment's names.

## Acceptance

- Parsing: the rim, rim-edge and corner forms above are canonical; incomplete, out-of-place,
  non-canonical, out-of-range and bare-number forms are refused.
- A solid rectangle member with a polygon cut lists its rim vertices, rim edges and corner
  edges; each resolves at the position the frame rules and the cutter's ring order give; the
  rim vertex snaps with its id, the rim edge resolves its nearest point and midpoint, and a
  point pick on the seam names the edge. A move keeps every name and moves the feature.
- A solid rectangle member with a round hole names its rims at the generator's canonical
  ring positions; a plate with a polygon hole names its rims the same way.
- A member with an oblique part cut names its rims on both crossed walls at the mapped
  positions; the leaning corner edge snaps at its midpoint.
- A member with only a plane cut (and a member with no records) names no cut edges; a
  well-formed cut edge id over either resolves as `GEO_FEATURE_NOT_FOUND`.
