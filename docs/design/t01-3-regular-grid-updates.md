# T01.3: regular grid updates

Contributor-original MPL-2.0. Depends on T01.1/T01.2 and the proposed T01 amendment in #415.

`UpdateGrid` carries optional name, world XY origin, Z-only quaternion and regular-family patches.
For a family, expression and full label list are supplied together; span-only patches retain the
expression and identities. The engine matches each label exactly within its old family. A retained
label keeps its AxisId; a new label consumes one supplied fresh AxisId in new-line order. The new
identities must be unused across the pinned project; extra or missing ones refuse the command.
Removing a referenced AxisId is a dependency conflict. Family modes remain fixed.

For example, replacing `0 2*6000` with `0 6500 5500` and the same three labels moves the middle line
from 6000 to 6500 while every AxisId survives. Negative or sub-millimetre spacing refuses the whole
operation. A new expression does not recalculate a stored span. Updating a frame changes the grid
only: it never rewrites a placed plate, member or level relationship.

The CLI is `update-grid --store ROOT --id GRID`, with `--name`, `--origin X,Y`, `--rotation DEGREES`,
`--u-expression TEXT --u-labels TEXT`, `--u-new-axis-ids UUIDS`, `--u-span START,END` and equivalent
V options. Omitted new identities are generated only for new labels. For an exact retry after later
changes, supply the explicit new identities, including an empty `--u-new-axis-ids ''` when none are
created. Request evidence decides replay before the current target is resolved.

The revision adapter shares the T01.2 mutation pipeline: index-backed ownership, a complete pinned
source, engine decision, validated attribute/label/placement changes, footprint check and atomic
publication. A combined family/frame/name update is one undo unit. Incoming grid relationships
conservatively protect all its axes until W01 introduces axis-specific stored view references.
The historical AxisId lifetime policy remains the review point recorded in #416.

Actual-binary tests cover identity preservation on the one-line rewrite, insertion/removal by
label, combined name/frame/span edits without part movement, refused order/count/mode/ID requests,
exact retry after deletion and one-step undo/redo. Engine tests independently check kept-ID geometry
and referenced-axis removal. Parent T01 acceptance is not claimed by this slice.
