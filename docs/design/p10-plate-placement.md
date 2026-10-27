# P10 plate placement slice

Contributor-original BeamCraft code and documentation, MPL-2.0.

The plate slice of P10 removes the engine's non-identity placement refusal. `create-plate` accepts
`--orientation-xyzw X,Y,Z,W`; `update-element` changes the same authored placement. The quaternion
uses the existing model admission and normalization rules. Contours, cutters, holes, meshes and
quantities remain in element-local coordinates. A pose edit changes the authored layer, not the
local mesh or quantity record.

A committed plate's snapshot entry carries the quaternion alongside translation. The existing
snapshot bounds, geometry features, picking, binary bundle and UI instance transform apply it.
IFC export already pairs the local plate mesh with its full pose; this slice enables that path.
USD likewise retains a local mesh under the ordinary Xform pose, with no baked rotation.

This slice enables plate placement needed by V08 work planes. The subsequent
[member instance pose change](p10-member-instance-pose.md) completes the corresponding local-body
representation for members. That follow-up changes the schema pin to distinguish older baked
member meshes from local ones.
Tests independently check a plate rotated 90 degrees about X, unchanged local mesh/quantities,
world bounds, a face pick and real IFC export after reopening. Engine tests accept rigid plate
creation and patches while retaining quaternion validation.

The former F06-POS-11 unsupported-placement deviation now checks world vertices and bounds
against the frozen fixture, alongside unchanged local mesh and quantities. This remains partial
F06 evidence: the adapter does not expose the section 7 shape/world measurement keys.
