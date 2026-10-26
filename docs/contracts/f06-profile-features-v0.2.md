# F06 0.2: profile features and evaluated surfaces

Contributor-original BeamCraft specification, MPL-2.0. Proposed amendment for #190.
F06 0.1.0 remains frozen; this document defines the additional supported profile.

## Names

Existing `fid1` box, extrusion and cut names keep their meaning. A non-box member uses
`fid2:<ElementId>:profile:<role>`. The role grammar is the extrusion grammar of F06 section 5:
`f.z0`, `f.z1`, `f.o<k>`, `f.c<j>.s<k>`, `e.z0.<t>`, `e.z1.<t>`, `e.v.<t>`,
`v.z0.<t>` and `v.z1.<t>`, where `<t>` is `o<k>` or `c<j>.<k>`. Grouped approximated
cavity walls may use `f.c<j>`. The 64-byte qualified limit and canonical decimal spelling
(no leading zero except zero itself) apply. Box members keep `fid1:...:member:*`.

The outer and cavity order is the admitted section outline order used by the kernel's
section builder, before rigid placement. For parametric families it is the deterministic
construction recipe's order; for a custom section it is the authored contour order. A
topology-preserving dimension, roll or placement edit preserves these roles. Changing
section family, contour topology or cyclic start may retire them. No rule promises a
physical-face identity across such edits. A copy qualifies the same local roles with its
new ElementId. `z0` and `z1` follow the member axis endpoints.

Plane cuts and holes retain the base body's surviving names and use the existing
`fid1:<ElementId>:cut:f.p<n>`, `cut:f.h<n>.s<k>` and grouped `cut:f.h<n>` faces.
An original vertex or edge removed or clipped by an operation is dangling. An operation
does not rename the surviving original vertices and edges. Operation-created vertices
and edges remain unaddressable, as F06 0.1 section 5 specifies. Vertices on a grouped
approximation wall are exposed only when an explicit authored point-role mapping exists;
mesh tessellation order must never invent such a mapping.

## Lookup and picking

The 0.1 box lookup representation stays available. General faces use a `surface` geometry
record: `triangles` is a bounded list of oriented triples of coordinates and `vertex_ids`
lists the addressable boundary vertices. These are evaluated coordinates at the query's
pin, not persistent buffer indices. `normal` is an outward unit vector for a planar face
and null for a grouped nonplanar approximation wall. Triangles preserve holes and
disconnected components after operations; a bounding rectangle is never a replacement.
Local and world geometry use the same names and triangle winding.

Ray and point picks of supported faces, edges and vertices return a non-null FeatureId
resolvable at the same snapshot. A surface pick reports the actual hit triangle's outward
normal, including grouped nonplanar walls. Edges and vertices obey the same radius,
distance ordering and tie rules as box features. Shared triangulation diagonals are not
semantic edges. Vertex snaps use addressable semantic vertices; new operation vertices
and approximation samples without a mapping do not become snap addresses. The interim
element-only mesh hit remains available solely for bodies without supported semantic
metadata, and is explicitly distinguishable by null `feature_id`.

## Acceptance

- Canonical positive/negative parsing for each family, including malformed versions,
  decimal forms, length bounds and forbidden bare indices.
- Ray and point picks plus lookup and vertex snaps on a contour plate, a plate with a
  polygonal and a round hole, and an I-section member; independent expected positions.
- Move and rigid rotation retain names; copies change their owner; operations retire
  removed/clipped original features while retaining unaffected names.
- A bundle round trip retains semantic metadata and all query results; metadata changes
  alter the snapshot token even when geometry blobs are shared.
- Kernel rectangle members return `member:` roles rather than `plate:` roles.
