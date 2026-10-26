# T04 numeric input and direction lock

Contributor-original BeamCraft code and documentation, MPL-2.0.

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
