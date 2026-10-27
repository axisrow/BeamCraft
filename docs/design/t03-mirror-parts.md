# T03 independent mirrored parts

Contributor-original Rust and synthetic fixtures, MPL-2.0.

`mirror-elements --store ROOT --ids UUID,... --point X,Y,Z --normal X,Y,Z`
creates independent copies of evaluated owned plates, beams and columns. `--plane`
interprets the plane point and normal in the V08 work frame. Context options, group limits,
footprints, atomic publication and immutable retry evidence use the T03 group adapter.
There is no linked mirror mode or negative-scale instance representation.

For a world reflection M and source pose (R,T), choose the local axial reflection
A(v)=S*v+k in the extrusion's mid-plane. The uniform base extrusion, including an asymmetric
member section, is invariant under A. The copied part uses the proper rotation Q=M*R*S and
translation mirror(T)-Q*k. Its local cut-plane points transform by A and normals by S, retaining
the keep side. Through holes remain on the same plate-local polygon because axial reflection
exchanges their through faces. No reflected profile definition or reordered contour is needed.

The existing copy engine assigns new operation identities before reflection. The adapter
re-evaluates transformed operations and stores matching meshes and quantities in the same
revision. Source inputs and every other member of its old P11 group remain unchanged.
As with the first T03 slice, unsupported object kinds and external relationship patterns
refuse the whole group. No implicit assembly or connection expansion is performed.

Tests check the proper-pose identity, plane involution, an obliquely cut plate with a hole,
double reflection restoring the original solid, and exact world vertices of a rotated
asymmetric channel. Snapshot AABBs are conservative rotated local boxes; they are not an
oracle for tight reflection bounds when the local axial reflection is oblique.
