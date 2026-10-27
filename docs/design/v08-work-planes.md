# V08 session work planes

Contributor-original BeamCraft code and documentation, MPL-2.0.

`beamcraft_engine::work_plane::WorkPlane` is a validated, right-handed session frame. It has an
origin and normalized X/Y directions, a normal, local/world conversion, a rigid pose and forward
ray intersection. Constructors support XY/XZ/YZ with an offset along the omitted positive model
axis, three points, a planar face, and a vertical plane through two points. Invalid, collinear,
non-orthogonal, non-finite or over-cap input is refused. Axis construction normalizes directions;
three-point construction uses the third point's positive-Y side to determine the normal.

The read-only `work-plane` CLI constructs a frame or picks a supported planar face and returns
its reusable `--plane OX,OY,OZ,XX,XY,XZ,YX,YY,YZ` expression. `create-plate` stores the resolved rigid
placement of its local contour; `create-member` transforms plane-local endpoints into world
coordinates. Members accept `X,Y` with local Z zero, `X,Y,Z`, and T04 relative/polar expressions.
Numeric direction locks use the active plane axes; viewport clicks and ghost endpoints use
the same constraint through local/world conversion. Combining a plane with explicit
placement, grid addresses or grid-height options is refused. Grid queries remain independently
available; no grid or element follows a later plane change.

The native create inspector has a work-plane editor and a face-pick action. Apply changes only
session state, cancels staged commands and resets viewport placement/ghost state. The member tool
intersects viewport rays with the active plane, displays local point fields and submits the same
CLI arguments as a headless caller. Global coordinates restore the existing placement behavior.
Changing a plane does not change a snapshot token, a revision, model keys, quantities or numbering.
The frame itself is never written to model USD. W01/V12 can later save it as view state.

Face construction uses semantic feature geometry and refuses grouped nonplanar surfaces and
unnamed display/reference triangles. A round hole in the current kernel is an authored polygon;
its named planar facets can define their actual planes. No tangent plane to an idealized circle
is invented from the hit point.

Tests cover all construction modes, quaternion mapping (including half turns), coordinate caps,
finite/degenerate input, bounded round-trip expressions, ray behavior, real CLI plane-local plate
and member placement, face picks without publication, grid-placement isolation, and native form
commands versus equivalent headless CLI input. No dependency, asset or model schema is added.
