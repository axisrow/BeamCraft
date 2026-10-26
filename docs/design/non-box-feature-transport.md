# Non-box semantic features and transport

Contributor-original BeamCraft documentation, MPL-2.0.

This implements the named-body query path of issue #190. Kernel results carry local face
roles beside the BCMESH01 blob. `SolidMesh` retains them for boxes, contour extrusions,
profile members and operation results; rotating a solid retains the roles too. `SceneMesh`
keeps them beside its shared mesh bytes.

`ElementEntry::full_face_table` and `simplified_face_table` preserve the tables per element
and representation. Mesh deduplication continues to use the blob digest alone. Two bodies
with identical geometry bytes can therefore retain different semantic mappings. Snapshot
tokens cover both tables, including role counts and byte lengths.

The view transport is now `BCVIEW03`. Each element record appends two tables after its
representation slots: a little-endian u32 count, followed by a u32 UTF-8 byte length and
the bytes of each role. Older bundle magic is refused. These are transient view bundles,
not persisted model files or a migration of stored FeatureIds. Producer and consumer must
use the same bundle version.

A nonempty table must match the blob's face count. Roles must be unique, nonempty, at most
1024 ASCII bytes, and use lowercase letters, digits, colon, dot and hyphen. The qualified
FeatureId length bound is checked separately, including implied cavity vertex names:
exceeding 64 bytes is `GEO_TOPOLOGY_LIMIT`. Empty tables
remain valid for reference meshes and existing unnamed inputs. The encoded role entries
of both representations across the scene have a separate budget equal to
`Limits::max_total_mesh_bytes`; counts add eight bytes per element to the bundle bound.
Malformed tables are refused with `GEO_BLOB_INVALID`, and budget exhaustion with
`GEO_BUFFER_LIMIT`. No additional geometry bytes are retained or duplicated.

## Query path

The naming proposal is [F06 0.2, PR #411](https://github.com/Apibim-com/BeamCraft/pull/411).
Box features keep their `fid1` names and representation. A non-box plate uses `extrusion`
and `cut` names; section members use `fid2:...:profile:*`. Rectangle member kernel results
now carry `member:` face roles and expose matching edge and vertex constants.

Vertices are intersections of authored side roles and a cap role. Adjacent wall roles
identify the authored contour point, independently of vertex order. Semantic edges join
these named vertices across different semantic faces; a triangulation diagonal within one
face is excluded. Operation-created vertices have no names, so an original edge clipped
by a cut becomes dangling rather than silently following the shortened segment.

`lookup_feature` returns local and world surface triangles for a general face, with a
single normal only when the surface is planar. Ray and point picks use those triangles,
preserving holes and nonconvex contours; hits have the actual triangle normal and a
qualified FeatureId. Point snaps use named vertices. Results keep the existing ranking,
radius and truncation rules. Shape-qualified results round-trip through bundles.

Operation-created edges/vertices remain excluded as F06 section 5 requires. Approximation
samples on a grouped cavity wall are excluded unless they have an authored point mapping;
the wall itself remains addressable. Axis snaps for non-box members remain unsupported:
there is no authored-axis descriptor in this snapshot format. Unnamed bodies retain the
explicit interim element-only mesh-ray path and null FeatureId; references stay skipped.
