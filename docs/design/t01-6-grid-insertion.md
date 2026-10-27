# T01.6 regular-grid insertion

Experimental insertion slice, not completion of T01.6 or the parent T01 acceptance gate.

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
it has no control in grid properties and is reserved for view properties. Levels remain separate
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
hidden grids cannot be picked. Drawn families can be selected but are read-only in this panel.

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

This UI slice does not add drawn-line editing, drawn-line gestures, UI snap controls or
interactive intersection placement, magnetic behaviour, radial grids, or derived USD guide tubes
(T01.7). New preview builds use the prerequisite grid schema; older stores are not silently
migrated. No new dependency or asset is introduced. Code and synthetic tests are contributor
original under MPL-2.0; no commercial screenshot or UI asset is included or traced.
