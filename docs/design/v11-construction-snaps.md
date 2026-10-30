# V11 construction snaps

Contributor-original BeamCraft code and documentation, MPL-2.0. Follow-up to #439 and
the interactive plate tool; no acceptance-gate verdict is changed here.

`query::pick::snap_with_context` resolves physical vertices and named edges, edge/reference/grid
midpoints, authored member axes and their centers, and grid intersections/endpoints/lines.
Reference axes come from evaluated member records, not a bounding box, so an offset section
or cut does not move the authored reference line. The same pinned export inspection transports
those axes in the optional `member_axes` field of its existing grid/display JSON section.
No USD model field or binary snapshot version changes.

## Switch rules

The owner's 2026-10-29 correction distinguishes **where a candidate comes from** from
**which kind of point may be used**. A source button never enables an arbitrary point on a
line. Admission requires every applicable condition:

`global Snap AND source gates AND point-kind switch AND construction context AND aperture`.

The two source masters retain independent state. Reference geometry covers authored member
axes and eligible grid guides; physical geometry covers actual body topology. Disabling both
masters leaves no snap candidates, even when every point-kind button is selected. A global
Snap toggle temporarily disables queries without changing the stored source/type choices.

| Source control | Enables consideration of | Additional gate | Does not enable |
|---|---|---|---|
| Reference geometry, crossed-square symbol | Authored member reference lines and their points | Global Snap | Endpoints, midpoints or nearest points by itself |
| Physical geometry, square symbol | Named physical vertices and edges of plates/members | Global Snap | Any point kind by itself |
| Grids | Visible project-grid guides and their points | Reference geometry and Global Snap | Nearest points on grid lines |

| Point-kind control | Reference source | Physical source | Grid source |
|---|---|---|---|
| Endpoints | Authored axis ends | Named body vertices | Finite guide-segment ends |
| Midpoints, triangle symbol | Halfway along each authored axis segment | Halfway along a named straight edge | Halfway along a finite guide and each span between adjacent opposite-family crossings |
| Grid intersections | None in this slice | None in this slice | Actual finite U/V crossings |
| Axis centers | Halfway along an authored member axis segment | None | None |
| Nearest, hourglass symbol | Arbitrary point on the reference line | Arbitrary point on a physical edge | Arbitrary point on a guide line |

The axis-center control is named explicitly: it is not a circle/arc-center capability.
The two controls can address the same member-axis midpoint; neither enables a whole line.
Endpoints is one UI switch for the three engine endpoint kinds. Nearest is one UI switch
for the three continuous line kinds. Toggling a source preserves these point-kind settings.

| Grids | Reference master | Nearest | Other selected kinds | Grid result |
|---|---|---|---|---|
| On | On | Off | None | No grid snap |
| On | On | Off | Intersections | Crossings only |
| On | On | Off | Midpoints | Segment midpoints only |
| On | On | Off | Endpoints | Segment ends only |
| On | On | On | Any | Nearest guide point, plus selected discrete kinds |
| Off | On | On | Any | No grid snap |
| On | Off | On | Any | No grid snap |

Selection-class buttons in the left strip are separate: excluding grids or parts from
object selection does not change their snap eligibility or visibility. Hiding grid guides
does exclude their snap candidates. Source/type gates run in the engine **before** result
truncation, including the generic midpoint query, so disabled sources cannot fill a page or
win a tie. The CLI and both coordinate tools use these same gates.

## Coordinate resolution and feedback

`SnapCursor` admits a finite forward ray and a bounded perspective angular aperture or a constant
orthographic radius. Cursor queries can find
geometry above or below the free-point fallback plane; point queries retain their spherical
band and ordering. The CLI `snap` accepts `--origin`, `--direction` and either `--aperture` or
`--cursor-radius MM` for this read-only query; the two aperture modes are mutually exclusive.
The UI derives the radius from the projection scale, keeping ten logical pixels in either camera
mode. An optional `--plane` filters before truncation and intersects physical edges
and vertical grid guides with the plane. There is no fabricated mesh diagonal or bounding-box
edge. Reference/display meshes receive no engineering feature names.

The UI calls the engine in memory on the displayed snapshot; mouse motion starts no CLI
process. It uses a ten-logical-pixel aperture and tests exact targets before nearest-line
candidates, so a line does not steal a nearby intersection or midpoint. Hover, marker and click
share the resolver. Hidden grids are omitted. Every supported switch changes the query;
the global Snap switch preserves individual choices while disabled. O and the orthogonal
button share the construction constraint. Unsupported perpendicular, extension, tangent and
parallel constraints remain disabled.

The first points can be spatial; once three non-collinear points define a plate, further snap
candidates respect that plane. Empty-space input uses the active/fallback plane. The final
engine contour check refuses non-planar points, including precise numeric inputs, rather than
projecting them. No magnetic grid relationship is created.

If all enabled targets miss, the point remains free construction input on the fallback plane;
it is not silently changed to a line snap. Coincidence of that free point with a displayed line
does not create a snap marker or a persistent relationship. The explicit first-vertex closing
gesture remains a contour action, independently of model snap switches.

For plates/members, discrete points are considered before continuous line points. Within a
group, the UI chooses the closest projected point inside ten logical pixels, with engine
ordering as the stable tie order. The grid-line authoring adapter also prefers discrete
points but retains its existing 100 mm point-query band. No off source is retried as a fallback.
Engine queries retain their own bounded point/ray distances and deterministic ordering.

O is an independent direction constraint through the last construction point, using the active
frame's X/Y/Z axes. It does not enable a source or point kind. Typing a positive length freezes
the aimed direction and Enter adds that length. A constraint that moves a point away from its
snap target clears the target marker. Empty Enter finishes the contour; repeated Enter events
do not add or finish again. Esc/tool changes discard the unpublished draft. None of these
session switch changes writes a model revision or changes the snapshot token.

## Headless interface

`snap` retains `--kinds` for an exact low-level query, and adds independent boolean policy
options: `--reference-snaps`, `--geometry-snaps`, `--grid-snaps`, and `--nearest`. Each defaults
to `true`; `--nearest false` excludes `axis`, `edge`, and `grid_line` even when those kinds are
explicitly requested. Discrete requested kinds are retained. Grids additionally require the
reference source. `--grids` continues to restrict visible grid identities; it is not a point-kind
switch. `--plane`, cursor-ray options and query caps keep their meanings.

For example, requesting `grid_line,grid_intersection` with grids enabled and `--nearest false`
returns only qualifying grid intersections. Requesting only `grid_line` under the same settings
returns an empty candidate list, never an implicit nearest point. These are successful read-only
queries, not invalid arguments. UI point changes call this engine policy in memory, without a
CLI process per mouse move.

## Coverage and boundaries

| Capability | Current behavior |
|---|---|
| Source/type conjunction and global Snap | Implemented for all current construction candidates |
| Endpoints, straight-segment midpoints, authored member-axis centers | Implemented |
| Finite grid intersections | Implemented; arbitrary physical-edge intersections are not added |
| Nearest points | Implemented, requiring the nearest switch for every line source |
| Circle/arc centers and tangent points | Not implemented as analytic circular features |
| Perpendicular, parallel and line extensions | Reserved controls remain disabled |
| Tab/Shift+Tab candidate cycling and a one-point override menu | Not implemented |
| Plane/Auto/3D depth selector and view-plane selector | Not implemented; the spatial contour behavior above applies |
| Occlusion-aware eligibility in opaque render modes | Not implemented; an admitted engineering feature may be behind another part |
| Read-only foreign meshes and drawing annotations | Outside this model-snap implementation |

This is not a claim of full commercial-product snapping parity. The owner-defined source/type
rules are executable now; the table above distinguishes them from further capabilities.

Verification includes real CLI coordinate oracles for a concave plate, an offset I member,
an inclined grid working plane and an elevated cursor target. Native GPU/controller tests
exercise zoom, individual/global switches, hidden grids, preview/click parity, numeric length
and O, then publish and reopen an independently sized plate. Unequal grid-bay midpoint tests
check both families, finite crossings, source gates, disabled nearest snaps and inclined planes;
native acceptance verifies that preview and click resolve the same span midpoint in both camera
projections. A real CLI regression checks that orthographic targets do not depend on view depth.
Query and switch tests assert
unchanged revisions/tokens. All shapes and test geometry are contributor-original; no external
product code, screenshots, catalogs or graphics were used.

The source/nearest regression enumerates all sixteen combinations of reference, physical,
grid and nearest settings against a point away from discrete targets, through both the real CLI
and native controller. Additional cases retain grid endpoints/midpoints/intersections with
nearest disabled, exclude physical midpoints before a one-hit limit, and exercise actual
toolbar clicks without changing the pinned snapshot.

Behavior reference consulted for the separation of source masters and point kinds:
[public snapping-toolbar description](https://support.tekla.com/doc/tekla-structures/2026/gen_snapping_toolbar).
The rules, matrices, implementation and tests here are independently written for BeamCraft;
no third-party text, table, icon or screenshot is reproduced.
