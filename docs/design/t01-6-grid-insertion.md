# T01.6 grid insertion and line tools

Local grid UI on the existing engine and CLI commands. Parent T01 acceptance still requires
the separately scheduled T01.5/U19 rights and pinned coordination grids.

## Dependencies and merge order

This implementation now includes verified `origin/master` `9cfa681`, the integration of #421.
That commit includes both the T01.3 regular-family editing originally reviewed in #419 and the
T01.4 queries, snaps and non-magnetic placement. #419 is closed without a separate merge and has
no remaining diff. The native form uses these existing commands and stored grid records;
it does not introduce a second implementation of regular-family editing.

The local development branch also carries the accepted command-navigation UI changes. For review,
integrate those first or use their branch as the base so the insertion PR shows only its own
changes. This note does not change the implementation-plan status or claim the parent gate.

## Behaviour

The Model ribbon's Grid tool opens an original BeamCraft inspector. It creates a regular grid
through the existing `create-grid` command: name, U/V coordinate expressions, explicit or
automatic labels, optional line limits, XY origin and Z rotation in degrees. Empty line limits
use the engine defaults. The form uses the shared model parsers; all engineering admission
remains with the engine. Creating is one atomic revision. Cancel runs no command. A refusal
leaves the model unchanged and is shown as a readable message.

Explicit label input may contain more words than coordinates produce lines. The shared model
parser uses only the first label per existing line; unused trailing words are ignored by both the
UI and CLI before building a canonical command. Missing labels continue from the last supplied
label: letters advance in the same alphabet and case; trailing numbers increment. Other names
gain a numeric suffix. Already-used labels in the family are skipped. Blank input in the form
selects the default automatic labels. This does not add axes or retain unused words in the
model. The accepted labels retain their validation and grid-wide uniqueness requirements.

The creation and cancel buttons stay below the scrollable fields. The inspector can be resized
horizontally and wraps its contents at the chosen width. Grid visibility is session view state;
the View ribbon's Grid guides switch hides guides and labels without publishing or reloading. Levels remain separate
from plan grids.

## Selection and properties

Click a visible guide line or an endpoint label to select its whole grid. Screen hit testing
uses the projected engine endpoints and a small pixel tolerance, without inventing engineering
geometry or grid intersections. The selected guides are highlighted. The Select inspector shows
the grid's name, coordinate expressions, labels, stored line limits, XY origin and Z rotation.
These values come from the same pinned inspection as the displayed lines, including the additive
`span_mm` field on regular families in `list-grids` and the view envelope.

Apply changes invokes `update-grid` once. Only edited fields are sent, so unchanged frames keep
their exact stored values. Retained labels preserve AxisIds according to T01.3; the combined edit
is one workspace undo unit. Cancel discards the draft and selection. A background reload refreshes
untouched forms; a dirty form whose grid changed is disabled until the grid is selected again.
A removed grid clears its selection. Empty space or Esc clears selection;
hidden grids cannot be picked or snapped to. Drawn families use the line tools below.

The form shows the start limit as a nonnegative distance; an edited start is negated for the
existing signed CLI coordinate. Rotation is shown to at most three decimal places, with no
negative zero. Unchanged fields retain their exact stored values, including a higher-precision
rotation or legacy signed limits. Delete outside text fields and the ribbon's Delete action
invoke `delete-element` for the selected grid; engine refusals remain visible and workspace undo
restores the complete grid.

Changing a coordinate expression additionally requests `--fit-spans true`. The engine fits regular
line limits to the opposite family's complete new coordinate range, with 2000 mm extensions at
both ends. This keeps enlarged grids connected rather than leaving new axes beyond the old line
ends. Explicitly edited line limits remain an advanced override. There is no separate Fit action
in the panel. Ordinary CLI updates retain their old span
behaviour unless the fit option is requested. Fit operations have a distinct fingerprint domain,
while existing ordinary-update fingerprints remain unchanged for exact replay.

## Pinned display transport

`export-view --with-grids true --out -` adds the engine-computed world endpoints and labels from
the same verified inspection as the mesh snapshot and definitions. The opt-in `BCVXFRM2` envelope
has three bounded length-prefixed sections (definitions, selected element, grids), then the
ordinary engine bundle. Existing exports retain `BCVXFRM1` or their unframed format. The UI reads
both envelopes, using one CLI process per successful refresh.

The UI bounds and validates guides before displaying them, frames the camera to their union with
the engineering bounds, and paints guides/labels as a construction overlay. It does not create
engineering meshes, quantities or materials. A failed scene/frame update keeps the previously
displayed scene and its guides together. Guide visibility and label appearance are not authored
model properties.

## Verification and limits

Actual-binary acceptance compares form insertion with the direct CLI, checks rotated endpoints,
labels, one pinned refresh, framing, and refusals without publication. Native egui input tests
cover opening the form, resizing the inspector, cancel, text-safe Delete, and preserving the snapshot. Both legacy and new
frame readers remain bounded. A combined regression edits a rotated rectangular grid through
the form, resolves and snaps its four outer intersections through the T01.4 CLI, preserves the
query height, and checks that already placed parts stay unchanged and queries publish nothing.

## Drawn lines and interactive snaps

Choose Drawn grid to create a grid with one labelled line in U or V. After selecting a grid,
Line tools add a drawn line, reshape its endpoints, relabel or remove an existing AxisId using
`draw-grid-line`, `reshape-grid-line`, `relabel-grid-line` and `remove-grid-line`. The axis selector
shows stored labels; the engine admits family modes, duplicates, lengths, limits and dependencies.
Common grid properties use `update-grid`; untouched frame fields retain their exact stored values.
A regular family is never converted to drawn implicitly. A mixed grid can be created with one
regular family and an empty opposite family, then extended using the line tools.

Endpoints may be entered numerically in world XY or filled by two clicks on the horizontal plane.
Clicks only change the draft. Apply publishes one command; Cancel and Esc publish nothing.
Esc or changing tools cancels point picking. A changed source grid disables a pending line edit
until it is selected again, including changes to endpoints that leave the regular form unchanged.

Line drawing and member placement ask `snap_with_grids` on the displayed snapshot and grid records.
The bottom snap panel controls vertex, axis and grid snap kinds; disabling all kinds disables
snapping. The acceptance radius is 100 mm. Hidden grids are excluded. Grid line inputs remain
plan-only XY; member snaps preserve the engine's resolved height. Axis locks and the active work
plane still constrain member placement after snapping. Snapping never creates a stored magnetic link.

The pinned grid reply adds `start_local_mm` and `end_local_mm` alongside world endpoints. This
avoids reversing a world transform at the local coordinate limit. The UI validates the bounded
local records, identities and agreement with the displayed world segments. Older replies without
these additive fields use the existing inverse-frame calculation.

The real-binary drawn-line regression covers a rotated frame, an independently expected crossing,
read-only hidden-grid snapping, stable AxisIds after reshape/relabel, duplicate-label refusal without
publication, removal and reopening. A native-adapter test drives the two-click controller, checks
engine snapping and hidden-grid exclusion, and confirms no command starts before Apply.

Magnetic following, curved/radial grids and shared-scope rights/pinned coordination grids remain
separate. New preview builds use the prerequisite grid schema; older stores are not silently
migrated. No new dependency or asset is introduced. Code and synthetic tests are contributor
original under MPL-2.0; no commercial screenshot or UI asset is included or traced.
