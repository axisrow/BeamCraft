# T04 numeric input and direction lock

Contributor-original BeamCraft code and documentation, MPL-2.0.

## Interactive direction and length

The 2026-10-29 plate-tool follow-up adds `coordinate::construction_step` and
`orthogonal_cursor`: a positive length along a 3D direction, and the closest projected X/Y/Z
axis through the previous point. The active frame supplies the axes; world XYZ is the default.
`resolve-construction-point --reference X,Y,Z --toward X,Y,Z --distance MM [--ortho true]`
exposes the same engine operation without a model store. An orthogonal cursor request uses
`--origin X,Y,Z --direction DX,DY,DZ --ortho true` instead of `--toward`.

After a plate vertex, typing starts a bounded distance entry and freezes the currently aimed
direction. Enter adds one vertex, clears the entry and keeps drawing. Repeated Enter events
do not add vertices or close the contour. Enter with no length finishes; Backspace edits the
entry before removing draft points. O and the bottom constraint button share session state.
Focused property fields retain ordinary text input. A failed length keeps both entry and draft.
Members use the same distance entry after the first point: numeric Enter creates the member.
The bounded label follows the cursor without opening a dialog. The viewport senses clicks and
drags without taking keyboard focus; other focused controls do not suppress construction input,
while text edits and the command dialog keep their ordinary keyboard handling. Native acceptance
reopens a member aimed along (3,4,12), types and edits 1000 mm, and checks its exact 3D axis length.
`coordinate::contour_frame` admits spatial contours without flattening them or changing the
active session plane. A non-planar contour is refused.

Real CLI tests cover a 3D 500 mm step, orthogonal Z, invalid lengths and cursor targeting.
Native egui input/controller acceptance types lengths, toggles O, ignores held-key repeats,
creates a vertical plate through the CLI and reopens its independently expected volume.

## Coordinate expressions

`beamcraft_engine::coordinate` resolves bounded numeric point expressions for both CLI and UI.
Absolute input is `X,Y,Z`; relative input is `@DX,DY,DZ`; relative XY polar input is
`@DISTANCE<ANGLE[,DZ]`, with millimetres and radians. The start reference is the model origin;
the member end reference is the resolved start. Polar distance is nonnegative, and positive
angles turn from +X towards +Y. Non-finite inputs and arithmetic overflow are refused.

`create-member --axis-lock none|x|y|z` projects the resolved end onto the selected model axis
through the start. All supplied coordinates are validated before projection. Existing Cartesian
arguments keep their meaning. The UI member form has optional start/end expressions overriding
its individual Cartesian fields; it passes the expression and lock to the same CLI command.
Viewport placement and its ghost use the engine's constraint function. Picking new endpoints
clears the typed overrides so a stale expression cannot replace the clicked points.

This task adds no dependency, asset, persistent field or model schema change. Unit tests cover
reference arithmetic, polar angles, each lock, malformed fields, non-finite values, length bounds
and overflow. Real CLI tests create and reopen members and verify refused input publishes nothing;
UI form tests verify the arguments resolve through the shared API and invalid input cannot stage.
