# P11 shared part instances

Contributor-original BeamCraft code and documentation, MPL-2.0.

P11 implements the instance semantics of F10 section 5.3 as a separate `CopyInstance` request
and `copy-instance` CLI command. `CopyElement` / `copy-element` remains an independent duplicate.
Both modes accept the same part identity, optional name, offset and command context fields.
Duplicated hole record IDs are derived deterministically from the destination part and source
record, so retries do not generate different hole identities.
Their fingerprints have separate domains, so an operation ID cannot change copy mode on retry.

## Authoritative definition and instance state

The canonical source part owns the shared definition: shape inputs, pinned profile and material,
and owned plane-cut and through-hole operation records. An instance authors `bim:instanceOf` towards that part.
Its own UUID, kind, label, placement, level and other relationships remain explicit USD fields.
The canonical part also keeps its own identity, placement and relationships; its pose does not
enter the shared definition. Creating an instance of an instance links directly to the canonical
owner. Chains and cycles are refused, as are missing, foreign, different-kind or unsupported
schema targets and shadow opinions on the instance's shared inputs.

The source's current level is copied initially. The new part joins no assembly and takes no
connections. Later placement, label and level edits affect the selected part alone. A duplicate
of an instance resolves and copies its effective inputs, gives its cut records fresh IDs, and
stores no instance link. It stays independent after the source definition changes.

This representation keeps the shared definition in ordinary USD and introduces no second saved
model. Readers materialize a transient, admitted projection of shared inputs for the existing
engine evaluators. The authored stage contains the shape and operation records once. Each part
has its own derived Geometry and quantity record; equal local bodies reuse their mesh digest,
body-cache entry and native mesh buffer. Shared operation IDs belong to the canonical authored
record; derived feature addresses still include each part's own ID.

There are at most 256 linked instances per model, keeping a group edit within the existing
4096-entry footprint cap. Resolution charges at most one million expanded shared values before
cloning, caps their conservative expanded storage at 64 MiB, traverses no deeper than 16 array levels, and expands only the closed set of supported
shape/operation properties. Unknown properties are preserved on their original prims and are
not expanded as shape inputs.

## Edits and atomic publication

`update-element` routes supported shared fields (material and plate thickness) to the canonical
owner. It updates the evaluated body/quantities of every group member in the same prepared
revision. Name, placement and level changes in the same patch still apply only to the requested
part. A member pose keeps its local body under the P10 instance representation.

`cut-by-plane` and `add-hole` append one operation record to the canonical definition and rematerializes every
group member's derived mesh and quantities. Its plane is in the shared element-local frame,
consistent with A03/P10. A cut on a rotated instance therefore follows that part's local axes;
the same local cut appears under every other instance's own pose. Hole polygons likewise stay
in the shared plate-local frame. No connection, assembly or
level relationship is remapped by a definition edit.

One accepted command is one revision and one workspace undo unit, including the canonical edit
and all derived results. Admission, footprint bounds and the strict verifier run before the head
changes. Membership/definition reads and all affected writes participate in the footprint.
New instance creation reads the canonical definition, so conflicting definition edits do not
silently rebase past its source inputs. Kernel or budget refusal publishes nothing.

An instance is a live incoming dependency on its definition owner. The existing complete index
therefore refuses deletion of an owner while instances remain (F10 section 5.3, DV-REL-009).
Deleting a leaf instance removes its own relation and geometry. Automatic promotion, detach and
cascade deletion are outside this command; the owner can be deleted after its instances are
explicitly removed. Replay retains the existing adapter's head-state behavior: later edits to a
copy's source or placement can make an old retry refuse as `OP_ID_REUSE`.

## Native controls and inspection

The Copy inspector has an Instance switch. Preview stages `copy-instance`; Commit runs the same
CLI command. `inspect-elements` reports the canonical owner and group part count for a shared
part. The properties inspector warns that shape/material edits affect the whole group while
name, position and level remain local. Selection and external relationships continue using IDs.
The snapshot bundle contains evaluated local meshes with full per-part placements; it does not
need a second representation of the shared-definition graph.

The schema pin changes; older pre-alpha stores are refused without migration. Existing field
limits still apply, and editing axes/profile/roll remains subject to the ordinary command
surface's existing support. No dependencies, external data or assets are added. Runtime
acceptance and cross-platform CI remain separate from compilation checks.
