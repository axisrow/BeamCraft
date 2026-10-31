# D04: drawing views — slice 1, view placement core; slice 2, section and detail views

Scope: fork #228 (epic #224, roadmap M4). The issue splits D04 into 2–3 slice PRs:
view placement core → section/detail views → titles and scales. This note records
slice 1 only. Sheets, frames and title blocks are D05; rendering several views onto
one output stays with D05.

Baseline: verified fork `master` `b2d5ee72` (2026-10-31). D01–D03 ship one admitted
document shape: one member, one primary view, one dimension, drawn objects. Slice 1
keeps that contract intact and adds placed views beside it.

## Placed views

`DrawingSpec.views` holds up to `MAX_VIEWS` (16) additional `ViewSpec`s, each with
its own basis (front/top/right), scale denominator and paper origin, admitted by the
same bounds as the primary view. Nothing is placed automatically: a created or
refreshed document carries exactly the views its caller named, and placement changes
only through a whole-document intent operation (`Drawing::with_views`, the CLI
`edit-views`), never as a side effect of model or drafting edits.

Identity discipline mirrors the drawn objects: view UUIDs are pairwise distinct and
distinct from the document objects and drawn objects, checked in both replacement
directions (`with_views` sees existing drafting, `with_drafting` sees existing
views).

## Resolution and staleness

`projection::resolve_view` resolves one placed or primary view from the document's
exact pinned snapshot: same pin check, same owner/feature semantics, twelve named
edges or an explicit unresolved reason. The view shows exactly its own basis of the
captured source — scale and origin change paper millimetres, never the model. A view
outside the document refuses; a snapshot whose pin disagrees refuses before any
geometry. Freshness stays a document-level pin comparison: every placed view shares
the document's source pin, so a model change marks all views stale together, and an
explicit refresh re-binds them.

## Wire format

The closed canonical intent gains one optional `Scope "Views"` with canonical
ordinal children `V1..Vn` (identity in `drawing:id`, never the prim name). The
schema descriptor is bumped. The whole intent must still fit one closed reader page
(`MAX_INTENT_PAGE_ITEMS` = 64 prims: three fixed document prims, one optional scope
per non-empty collection, every placed view and drawn object), so admission refuses
a document whose views and drawn objects together exceed the page even when each
collection is under its own cap.

## CLI

`edit-views` mirrors `edit-drafting`: no model store on the command line, the
retained source archive and all identities carry over, and the request fingerprint
binds the incoming placement, so an exact replay returns the same revision and a
changed placement under the same operation id refuses. `inspect-drawing` reports
`placed_views` with each view's plane, scale and origin. Headless first: the UI
placement surface is a later slice of this issue.

## Slice 2: section and detail views

Baseline: verified fork `master` `587e2812` (2026-10-31), i.e. after D07 slice 3 (#347)
already owned the layers and line types in this same document model; slice 2 rebased onto it
and keeps the placement discipline, adding two hand-placed derived views beside it.

A **section view** (`SectionSpec`, up to `MAX_SECTIONS` = 16 in `DrawingSpec.sections`)
cuts the model with one axis-aligned plane: perpendicular to its basis's depth axis
(Front→Y, Top→Z, Right→X) at `cut_mm`, with `CutSide` picking the visible half-space —
the same half-space semantics as V15's cut planes, reused as a drawing intent instead of
display state. `projection::resolve_section` resolves the visible half of the member
wireframe (a crossing edge is clipped exactly to the cut and keeps its feature address) and
the exact cut contour: the strict edge/plane crossing points, floor-deduplicated and ordered
by angle around their centroid, empty when the cut misses the member box. Axis cuts only —
arbitrary cut planes are outside the contract, exactly as arbitrary cameras are.

A **detail view** (`DetailSpec`, up to `MAX_DETAILS` = 16 in `DrawingSpec.details`)
magnifies one parent view's circular region at its own scale: the parent must be the primary
view or a placed view (details of sections or of details are outside this slice),
`center_paper_mm`/`radius_mm` are authored in the parent's paper, and
`projection::resolve_detail` maps the parent's resolved wireframe by the scales' ratio and
clips it to the region (parametric line–disk clipping), keeping the parent's semantic
feature addresses. The detail is a magnification of exactly that parent content, never a
fresh projection.

Identity collapses into one shared paper space: every placed view, section, detail, drawn
object and layer UUID is pairwise distinct and distinct from the three document objects, in
both replacement directions. The one-page intent budget counts all five collections and
their optional scopes; the caps stay 16/16/16/60/16 and the page (64 prims) arbitrates. The
wire format gains optional `/Drawing/Sections` (`S1..Sn`) and `/Drawing/Details`
(`D1..Dn`) scopes beside `/Drawing/Views` and `/Drawing/Layers`; the slice extends the
drawing schema descriptor that #347 carries (`beamcraft-drawing-usd/4`) with the
`sections=`/`details=` entries rather than bumping it again. `edit-views` replaces all
three view collections in one fingerprinted document operation via
`--placed-section UUID:plane:cut:side:scale:x,y` and
`--placed-detail UUID:parent:x,y:radius:scale:x,y`; `inspect-drawing` reports them as
`placed_sections`/`placed_details`.

Known boundary: the whole-sheet renderer (`drawing::sheet`, D05) draws placed views and
the primary dimension groups; section and detail geometry resolves headlessly but does not
render on exports yet. Titles and scales are slice 3.
