# beamcraft-engine

Contributor-original BeamCraft code and documentation, MPL-2.0.

V02 adds the renderer-agnostic **view snapshot and query contract** to the engine. The crate
is pure, bounded and deterministic. It has no UI, wgpu, egui, I/O, clock or third-party
dependency; it depends on `geom`, `model` and `kernel` only. A renderer receives mesh blobs by
digest, instance and triangle tables, ids and bounds, and computes no engineering geometry
(F06 section 9).

## What is implemented

K1 authored-only plate admission is exposed by `plate_draft::{create_plate_draft,
admit_create_plate_draft, evaluate_committed_drafts}`. Its typed unsupported state retains valid
inputs without a body or quantity values. The existing `command::create_plate` API remains an
evaluated-body API and refuses unsupported geometry without changing state. See the
[authored-only plate design note](../../docs/design/k1-unsupported-plate-drafts.md).

| Area | API | Notes |
|------|-----|-------|
| Pinned snapshot | `ViewSnapshot::build(ViewPin, Vec<SceneElement>, &Limits)` | `ViewPin` is a revision digest and a workspace generation. Elements are ordered by `ElementId`; a duplicate id is rejected. |
| Revision token | `SnapshotToken` (`vs1-` + 64 hex) | Opaque, equality only. Covers the pin and the authoritative content (ids, kinds, placements, mesh digests). Never view settings or selection. |
| ID mapping | `ElementEntry::index`, `ViewSnapshot::{element, element_at}`; `ViewProjection::{instance_of, element_for_instance, element_for_triangle}` | Indices are snapshot-local positions and are never identities. A triangle or instance resolves to an `ElementId` only through a projection of one snapshot. |
| Bounds | `Aabb`; `ViewSnapshot::{element_bounds, engineering_bounds}`; `ViewProjection::bounds` | Computed by the engine from kernel vertices plus the translation placement. Display aid, never a volume. |
| Mesh buffers | `ViewSnapshot::{meshes, mesh, mesh_at}`; `MeshEntry` | Exact BCMESH01 bytes identified by SHA-256, stored once. Every blob is re-inspected (F06 section 6 header, caps, sizes, finite and non-negative-zero coordinates, zero padding, index ranges) and its digest recomputed. |
| Representations | `Representation::{Full, Simplified}` | A part with no operations references one mesh from both. A distinct simplified mesh is flagged not exact; `Representation::feeds_engineering()` is false for it, and `ViewSnapshot::engineering_representation` always returns the full one. |
| View settings | `ViewSettings { hidden_kinds: KindFilter, representation }`; `ViewSnapshot::project` | A read-time projection: it changes no token, no authored or derived data. It exposes the instance/triangle tables, the meshes a view must load (`required_meshes`) and what is hidden. |
| Incremental delta | `diff`, `SnapshotHistory::{push, changes_since}`; `ChangeSet` | Added/changed/removed in id order; `new_meshes` lists only blobs the consumer lacks, so a placement-only edit resends none. An unknown, expired or evicted token is `GEO_STALE_TOKEN` (`STALE_WORKSPACE`) with no partial list. History holds at most 8 snapshots. |
| `ListElements` | `ViewSnapshot::list_elements(after, page_size)` | Id-ordered, cursor is the last id, page size 1 to 1,024. |
| Semantic features (D01.1) | `query::feature::{list_features, lookup_feature}` | Enumerate 26 qualified face/edge/vertex roles of one canonical box body at the supplied snapshot. Stable semantic addresses include the owner; no indices or bounding-box substitutes for unsupported topology. CLI: `feature-list --store ROOT --id UUIDV7`. |
| Drawing intent (D01.2) | `drawing::{Drawing::admit, DrawingSpec, SourceModelPin, LiveSource, Freshness}` | Bounded immutable intent: distinct typed drawing/view/dimension IDs, an exact source pin, one explicit orthographic view and two qualified member vertices. Explicit refresh retains references/IDs; freshness requires a supplied verified live context. This prerequisite adds no drawing storage, projection or publication; the headless adapter follows with D02. |
| Selection | `Selection::{from_ids, from_instances, from_triangles, instances, rebased}` | Holds ids only; display hints are resolved through the projection and discarded. `rebased` enforces the destination snapshot's selection cap (`VIEW_SELECTION_LIMIT`). Selection is viewport state and never enters a key. |
| Errors | `ViewCode`, `ViewError` | 11 closed codes, `GEO_*` where F06 section 3 names one and `VIEW_*` otherwise, each mapped onto an existing F04 `ErrorCode`. |
| Headless | `beamcraft view-snapshot`, `beamcraft view-changes` | See the CLI README. |
| `CreatePlate` (P01, P02) | `command::{create_plate, evaluate_plate, origin_rectangle_contour, AuthoredPlate, MaterialInfo, MaterialLookup, no_materials}`, `transaction::admit_create_plate`, `footprint::Footprint::{for_create, for_define}` | Admission (F04 caps, label, placement, `D`, floor, repeated points), a pinned material resolved through the caller's lookup (`NOT_FOUND` before any geometry), kernel body, quantities (volume current; mass `current` only with a material that authors a density, otherwise `unknown`), declared footprint (the pinned definition is read, never written), `op_id` replay/reuse decision. Origin rectangle only, see below. |
| Committed plates (P01, P02) | `query::committed::{evaluate_committed, scene_elements}` | Authored plates read from a committed revision are re-evaluated by the kernel (with their material's re-admitted density) and become the `ViewSnapshot` input. `beamcraft plate-inspect` uses it. |
| Body cache (issue 26) | `query::committed::{BodyCache, BODY_CACHE_BYTES, evaluate_committed_cached, evaluate_committed_members_cached}` | Kernel bodies by the exact bits of their complete shape inputs: contour, thickness, holes and cut of a plate; axis, roll, cut and the resolved profile content of a member. Id, label, kind, placement, material and the profile digest are not inputs, and no digest stands in for one, so `-0.0` and `0.0` are different keys. At most 32 MiB and `MAX_ELEMENTS` entries, cleared when full; a failed evaluation is never kept. The CLI keeps one per thread, so a command's read, its commit verifier and the copies of one model share bodies, while every stored mesh is still compared with the body. See the [design note](../../docs/design/issue-26-inspection-cache.md). |
| `CreateStraightMember` (P03) | `member::{create_straight_member, admit_create_member, evaluate_member, AuthoredMember, ProfileInfo, ProfileLookup, no_profiles}`, `footprint::Footprint::for_create_member`, `query::committed::{evaluate_committed_members, scene_elements_with_members, MemberRecord}` | Admission, duplicate id, pinned profile (required) then material, kernel member body (F06 member frame), volume and mass, footprint (profile and material read, never written), replay/reuse. A rotated placement is `UNSUPPORTED_GEOMETRY`. |
| Through holes (A04) | `command::{HOLED_VOLUME_ALGORITHM, PlateBody}`, `solid::SolidMesh`, `AuthoredPlate.holes_mm` | `CreatePlate.holes_mm` (model caps: 64 holes of 3 to 1024 points) is cut by the kernel, one `through_hole` per polygon in order; a refusal is `INVALID_ARGUMENT` and nothing is applied. The certified mesh's volume is the enclosure midpoint (`kernel-g09-through-hole-volume/1;tolerance-policy-3`) and the P04 shape signatures include the holes. See the [A04 design note](../../docs/design/a04-plate-holes.md). |
| `UpdateElement` (P04) | `update::{effect, patch_plate, patch_member, UpdateEffect}`, `quantity::{QuantityRecord, StoredRecord, Freshness, Assessment, assess, record_for, plate_shape_signature, member_shape_signature, mass_signature}`, `footprint::Footprint::for_update` | Name, material, thickness and placement-translation (move, P08) patch; decides which groups change (authored, mesh, stored quantities); a move keeps the rotation and never rebuilds the body. Stored volume and mass records carry their own shape and mass signatures and are judged current, stale or missing. Thickness on a member is `UNSUPPORTED_PROPERTY`. |
| `CopyElement` (P05) | `copy::{copy_plate, copy_member}`, `footprint::Footprint::for_copy` | The copy's authored state: fresh id, optional label, placement plus an offset, every other input and pinned definition by value. The source is only read; the body and mesh are the source's. |
| `DeleteElement` (P08) | `footprint::Footprint::for_delete` | Reads the lifecycle and the relationships, writes every group of the element and its derived group. Whether something links to the element is the layer engine's element index (`Author::check_delete`), applied by the adapter. |
| `AttachReference`, `MoveReference`, `CorrectReferenceUnits`, `WriteForeign` (R01) | `reference::{attach, move_reference, correct_units, write_foreign, ReferenceRecord, UnitsStatus}`, `footprint::Footprint::for_attach` | A read-only foreign USD source and its owned wrapper. Admission and footprints (for a reference the `shape_inputs` group is the source pin and its units interpretation; a move writes placement and derived; none names the foreign layer). `write_foreign` is the one door for a write to the source or a foreign prim and its success type is `Infallible`. `ReferenceRecord::source_point_to_world` is the transform measurements of the source must use (units interpretation, then placement); `units_status` says when the source is not in project units. See the [R01 design note](../../docs/design/r01-readonly-reference.md). |

## Limits

Read-only reference display is exposed by `scene::{ReferenceGeometry, reference_element}`:
bounded Cube and triangle/convex-quad Mesh cages become ordinary reference-kind scene elements,
without acquiring certified bodies, quantities or engineering pick features. Source translations
and explicit R01 units correction are supported; wrapper rotation and other foreign transforms
are not. See the [R01 viewport profile](../../docs/design/r01-readonly-reference.md#translation-only-viewport-path-issue-137).
`drawing::projection::resolve` (D02.1) consumes admitted intent and the exact captured snapshot.
It returns twelve semantic wireframe edges and a projected, scale-independent model-mm dimension,
or typed unresolved states for lost/unsupported/collapsed geometry. It checks the pin before
geometry, caps all paper output, and performs fixed work over 26 roles and two anchors. The CLI's
`drawing-preview` strictly inspects one opened model revision before calling it; no UI geometry
or buffer address enters a measurement. Drawing USD persistence/publication remains D01.3/D03.

`drawing::projection::resolve` (D02.1) consumes admitted intent and the exact captured snapshot.
It returns twelve semantic wireframe edges and a projected, scale-independent model-mm dimension,
or typed unresolved states for lost/unsupported/collapsed geometry. It checks the pin before
geometry, caps all paper output, and performs fixed work over 26 roles and two anchors. The CLI's
`drawing-preview` strictly inspects one opened model revision before calling it; no UI geometry
or buffer address enters a measurement. Drawing USD persistence/publication remains D01.3/D03.

All caps are checked before the work they bound, with checked arithmetic and no
input-derived indexing. A caller may lower a limit with `Limits::new`, never raise it.

| Limit | Maximum | Error |
|-------|---------|-------|
| Elements per snapshot | 65,536 | `VIEW_ELEMENT_LIMIT` |
| Mesh blob | 213,024 bytes, 4,096 vertices, 8,192 triangles, 1,024 faces | `GEO_BUFFER_LIMIT`, `GEO_TOPOLOGY_LIMIT` |
| Distinct mesh bytes per snapshot | 64 MiB | `GEO_BUFFER_LIMIT` |
| Triangles per projection | 4,194,304 | `GEO_BUFFER_LIMIT` |
| Selection | 65,536 ids | `VIEW_SELECTION_LIMIT` |
| `ListElements` page | 1,024 | `VIEW_ARGUMENT_INVALID` |
| Retained snapshots | 8 | (older tokens are `GEO_STALE_TOKEN`) |

`GEO_NUMERIC_UNCERTAIN` is a defensive finite check in the bounds computation; typed kernel
inputs cannot reach it through `ViewSnapshot::build`.

## What is not implemented (and is not faked)

- **P01 is a first slice only.** `CreatePlate` is implemented for the origin rectangle contour
  `(0,0,0),(w,0,0),(w,l,0),(0,l,0)` (F04-POS-01), simple CCW polygons in `z=0`, and exact
  rectangular contours in other planes. Ordered 3D contour admission rejects outside-band
  nonplanarity, clockwise winding and self-intersection as `INVALID_ARGUMENT` before support.
  The evaluated-body API refuses valid near-planar input and other polygons outside `z=0` with
  `UNSUPPORTED_GEOMETRY`; `plate_draft` separately preserves those authored-only inputs. Holes on framed
  rectangles are unsupported. See the [K1 contour/frame slice](../../docs/design/k1-plate-contour-frame.md).
  A placement rotation other than the identity is
  `UNSUPPORTED_GEOMETRY`. There are no dependency keys (F06 section 7), no stored quantity or
  signature, no other command. Profile and material definitions are published by the CLI adapter
  (P02); the engine only resolves a pinned material to its density and no command consumes a
  profile yet (P03). The snapshot builder still takes sealed kernel `PlateResult`s;
  `ElementId` is `beamcraft_model::id::ElementId`. `synthetic` scenes remain stand-ins and every
  command output marks them (`stand_ins`).
- **Holes are authored with the plate (A04), and no other operations exist yet** (A03), so a
  distinct simplified mesh can only be a stand-in supplied by the caller: there is no hole
  edit command, and no cut edit. Measuring the load-time gain on the 10 000-part scene is a
  V01 task.
- **Welds and bolts are not registered kinds** (F10 vocabulary 0.1). The kind filter works on
  the 13 registered kinds and will cover welds and bolts when a vocabulary version adds them.
- **No geometric picking.** F06 `Pick` and `LookupFeature` need kernel feature geometry (G04).
  Selection by display hint (instance or triangle index from a GPU pick) is supported as a hint
  only; anything selecting for engineering use must come from the engine's `Pick`.
- **No keys, statuses or local-space bounds** (P01), no presentation-key preimage (V01/V03),
  no cancellation or deadline polling, no transport schema for deltas (U02), no transform
  other than translation (the first-scope placement).
- **No mesh export from the CLI.** The CLI reports digests, sizes and counts; delivering blob
  bytes is an I/O-edge decision for the UI/host tasks.

## Tests

`cargo test -p beamcraft-engine` runs unit tests (ids, kinds, error table, limits, blob
inspection with every structural failure and cap) and `tests/view_snapshot.rs`, which uses the
real kernel for determinism, bounds, deduplication, settings, tables, paging, deltas, staleness,
selection and a negative test for each error and cap, including the 65,536-element hard cap.
