# T02 construction objects

Contributor-original BeamCraft code and documentation, MPL-2.0.

Construction objects (T02) are authored helper elements with no engineering body: a point, a
line segment, a circle or a plane, each with a label and world-millimetre geometry. They exist
to be snap and placement targets and are excluded from schedules, numbering, quantities and
exports exactly like grids (T01): the BOM builds rows from plate and member records only, and
the numbering preview groups plates, members and assemblies only. Both exclusions are covered
by a real-CLI acceptance test, not by an engine invariant claim.

## Storage

The helper element kind joins the closed F10 registry as `construction` with the canonical
path category `/World/Construction/CNS_<uuid7-32>` (U16). The authored prim carries
`bim:construction:kind` (point|line|circle|plane), `bim:construction:aMm`,
`bim:construction:bMm` and `bim:construction:radiusMm` under the same 0.1.0 schema version;
the placement is the identity, because the geometry is authored directly in world coordinates
(as a weld or a level is). `SCHEMA_DESCRIPTOR` commits the mapping. Creation goes through
`CreateConstructionObject` (model admission, versioned fingerprint) and the engine's
`construction::create` admission (identity uniqueness, the 4096-object project cap) into the
ordinary atomic publication; deletion is the ordinary `DeleteElement` lifecycle, reported as
kind `construction`.

## Snapping

The snap context gains a construction source family with its own source gate
(`--construction-snaps`, default on; UI source setting included) and six kinds:
`construction_point`, `construction_end`, `construction_line`, `construction_center`,
`construction_circle` and `construction_plane`. A helper line also contributes to the generic
`midpoint` kind. Continuous kinds (line, circle, plane) require the nearest switch, as every
other line source does; source and point-kind gates run in the engine before truncation.
The nearest point of a circle is the radial projection onto the circumference, with a
deterministic perpendicular basis when the query sits on the circle's axis; the plane's
candidate is the perpendicular foot of the query point. Working-plane and cursor contexts
apply unchanged (line points resolve against the plane/cursor, discrete points are admitted
only on the plane).

## Boundaries

Construction objects have no derived display geometry, no pick/mesh presence in the view
snapshot, and no UI drawing tool yet: the snap strip gains a working "Snap construction
objects" source switch (the strip grows from thirteen to fourteen buttons), and the CLI/-
engine resolve helper snaps already; the viewport slice that draws helpers will pass them
into the UI snap context the same way. Copy, move, rotate, mirror and arrays of helpers are
T03, not this slice. No USD field or binary snapshot version of an existing element changes.

Verification: engine unit tests (kinds, points, ordering, source/nearest gates, repeated
identity refusal, creation admission) and real-CLI acceptance (exact reopen of all four
geometries, snap oracles for every kind, gate behaviour through `snap`, schedule and
numbering exclusion, delete lifecycle). All geometry and names are contributor-original.
