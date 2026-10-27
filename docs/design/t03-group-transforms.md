# T03 atomic rigid part groups

Contributor-original BeamCraft code and synthetic fixtures, MPL-2.0.

This first T03 slice adds typed `engine::transform::RigidDelta` and the headless
`beamcraft_cli::transform::transform_elements` adapter. `move-elements`, `rotate-elements`
and `copy-elements` accept an explicit UUID selection, sorted before engine admission.
Duplicate IDs, unsupported/missing parts and selections above 128 are refused as a whole.
Supported objects are evaluated owned plates, straight beams and columns, including P11
instances. Other kinds and authored-only unsupported geometry are explicitly refused.

Move/copy takes either `--vector` or `--from` with `--to`. Rotate takes `--center`,
`--axis` and `--angle` in radians. All support the V08 `--plane` frame and T04 coordinate
expressions; a relative destination resolves from the supplied starting point. Transforms
operate in world space after that one coordinate conversion. A rotation composes proper
unit quaternions and rotates the placement origin about the specified center. Local meshes
and engineering quantities are unchanged by placement edits.

Copy defaults to independent duplication. `--instance true` uses P11's canonical shared
owner. Ordinary copies resolve effective inputs and assign fresh deterministic operation
record IDs through the existing copy engine. New element IDs derive from the logical operation
ID, source ID and copy slot; the layer engine checks actual absence. A duplicate retains its
source's level, joins no assembly and takes no connections, as specified by DV-REL-011. Other
unsupported external relationships refuse the whole command. Assemblies and connections are
not implicitly expanded or silently dropped from selections.

One request prepares every change against one verified base, unions engine footprints,
checks the layer engine's actual accesses and publishes once through the existing verifier and
revision journal. One group is one workspace undo unit. Validation failures publish nothing.
The immutable request fingerprint and footprint are pinned in the revision's existing command
evidence. A retry compares that recorded fingerprint before looking at current source geometry,
so a relative move is never applied twice, even after subsequent edits. Different arguments
with the same operation ID are `OP_ID_REUSE`; an uncertain journal result requires recovery.

Mirror, arrays, multi-kind closed relationship remapping and their acceptance rows are separate
T03 slices. This slice does not close the parent task or add native UI controls.
