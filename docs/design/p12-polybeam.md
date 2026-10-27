# P12: polybeam (a member along a polyline path)

Date: 2026-10-27. Status: design note for the P12 member command (`beamcraft-model` command
`CreatePolybeam`, `beamcraft-engine` module `member`, CLI `create-polybeam` and
`update-element --axis-point`). Task card: `docs/plan/tasks.md` P12 / M (milestone M1); issue
#187. Builds on G12 (`sweep::sweep`, the mitred polyline/arc sweep), P03 (straight members)
and P10 (rigid instance placement).

## What a polybeam is

A polybeam is a member: the same `bim:elementKind` (`beam` or `column`), the same required
pinned profile, the same optional pinned material, the same rigid placement, cuts, holes of the
part machinery and the same instance group behaviour. The one difference is the axis: a
straight member stores exactly two element-local points, a polybeam stores a polyline of three
to `MAX_MEMBER_PATH_POINTS` (1024) of them in the same `bim:axisPointsMm` attribute. The point
count is the shape: two points is the P03 straight member, more is the P12 polybeam, and every
reader distinguishes them by that count alone.

## Corner treatment: the G12 mitre

The body is the G12 sweep of the one section along the polyline: every interior vertex is a
shared mitre ring (the oblique section by the turn bisector), the end rings stay perpendicular
to the end tangents, and the section is parallel-transported in the sense of these half-turn
mitres — the fabricator's polybeam behaviour. There is no authored roll on a polybeam: the G12
sweep carries no roll input, so `CreatePolybeam` has no roll field and a stored polybeam
member always reads `steel:member:rollRad` absent. A first-scope rectangle profile is swept as
its admitted centred custom outline; an A07.1 family or custom outline is swept as its
detailed outline, like a straight member of the same section.

## Quantities: the analytic mitre volume

The quantity of record is the effective section area (declared catalogue area, else the exact
area) times the polyline path length, under the new algorithm label
`pmass-member-area-x-path-length/1;polybeam;tolerance-policy-3`. G12 corollary 1 is the
acceptance: for a section centred on the path every mitre plane crosses the centroid fibre
exactly at the vertex, so the certified mesh volume is `A x L_path` — the mitre angles drop
out. The G12 seal cross-checks the sealed mesh against the analytic truncated-prism volume
before anything is stored. A cut replaces mesh and volume with the certified replay result,
under the shared `CUT_VOLUME_ALGORITHM` label. The body cache key, the gross/net quantity
signatures and the BOM/numbering member length (the body's `axis_length_mm`, now the path
length) all follow the stored point count without a format change.

## Point editing

`UpdateElement` gains a typed patch field `axis_points_mm`: the complete replacement of the
member's polyline, authored as the layer-engine `Change::Axis` on the canonical part (instances
of the group read their shape inputs from it). Fewer than two points is `INVALID_ARGUMENT`; two
points straighten a polybeam member into the P03 shape, three or more make it a polybeam
again. The patch is a shape change: the body is re-evaluated, the mesh and quantity records are
rewritten, and the effect table of `update.rs` names the touched groups. The update
fingerprint appends the new field only when present, so earlier patch fingerprints are
unchanged.

## Storage and reads

`bim:axisPointsMm` was already `double3[]`; the schema does not change. The layer engine
re-validates the stored member by point count (2 to 1024), the total path length against the
size cap and every chord against the geometric floor; the kernel sweep refines those at
evaluation. A commit written before P12 reads unchanged: its two-point member is the straight
member it always was.

## Refusals

Everything the engine refuses keeps its established code: a two-point polybeam command is
`INVALID_ARGUMENT` (author a straight member instead), a path over the point cap or coordinate
cap is `LIMIT_EXCEEDED`, a zero-length chord or a degenerate turn is the kernel sweep's
`GEO_AXIS_TOO_SHORT` / `GEO_OP_UNSUPPORTED`, an over-budget mesh is `GEO_BUFFER_LIMIT`, an
absent profile is `NOT_FOUND`. A stored member that no longer evaluates (a model edited
outside BeamCraft) is refused by the same evaluation as an authored one.
