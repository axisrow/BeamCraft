# V04 picking and snaps

Issue #190 extends this original box-only design. See
[non-box semantic features](non-box-feature-transport.md) for `fid1` extrusion/cut roles,
F06 0.2 profile roles, surface lookup, point/ray picking, snaps and bundle transport.
The historical interim mesh-only path below now applies only to unnamed bodies.


Date: 2026-10-18. Status: **experimental vertical slice; V04 acceptance is limited as listed below.**
Branch `feat/v04-picking-snaps`, based on `origin/master` `7b1f2bf`.

Plan row: V04 / M, depends on V02, R01 and G01. Result: minimal picking, axis and vertex snap
queries and stable ids. Acceptance: headless API/CLI end to end with independent expected
positions; references read-only; frame indices never become ids.

## What is implemented

```text
beamcraft pick --store ROOT (--origin X,Y,Z --direction X,Y,Z [--t-max MM] [--include-backfaces true|false]
                             | --point X,Y,Z) --targets face,edge,vertex [--radius MM] [--max-hits 1..64]
beamcraft lookup-feature --store ROOT --feature-id FID
beamcraft snap --store ROOT --point X,Y,Z [--radius MM] [--kinds vertex,axis_end,axis] [--max-hits 1..64]
```

| Area | What exists |
|---|---|
| `beamcraft_engine::query::feature` | `FeatureId` (`fid1:` + ElementId + `:` + `plate:`/`member:` + face, edge or vertex role; at most 64 bytes; parse and display), `FeatureKind`, `BodyKind`, `lookup_feature` (kind, owner, local id, local and world geometry: face normal and corners with vertex ids, edge ends with vertex ids, vertex position), `has_features`. |
| `beamcraft_engine::query::pick` | `pick` with `RayPick` and `PointPick` as F06 section 9 specifies (closed rectangles enlarged by `eps_pick`, parallel test, back faces, closed finite segment with clamped `t`, point-form distances, inclusive band, quantized ordering, `max_hits` and `truncated`, the limit and input codes), and `snap` (`vertex`, `axis_end`, `axis`). A ray pick additionally hits bodies without FeatureIds at element level, by their mesh (amendment of 2026-10-23 below). |
| `beamcraft_engine::error::ViewCode` | `GEO_NON_FINITE`, `GEO_PICK_INPUT_INVALID`, `GEO_PICK_LIMIT`, `GEO_FEATURE_ID_INVALID`, `GEO_FEATURE_NOT_FOUND`, `GEO_TRIANGLE_ADDRESS_FORBIDDEN`, mapped to F04 codes as F06 section 3 lists. |
| CLI | `pick`, `lookup-feature` and `snap` over the verified head's view snapshot (the one `inspect-elements` builds); errors carry the F04 `code` and the F06 `reason` token. |

## Decisions

- **Features come from the snapshot, not from new kernel calls.** The engine re-reads each
  element's BCMESH01 blob (already validated and digest-checked by the snapshot) and grants the 26
  box roles only when the blob has exactly the canonical layout of F06 section 6: 8 vertices, the
  12 canonical triangles with their face indices, 6 faces, and every face a rectangle (relative
  tolerance `1e-9` on orthogonality and closure). Geometry is the blob's element-local positions
  plus the snapshot's translation. Any other body has no FeatureIds (F06 section 5 rule 9), and a
  lookup of one of the box ids on it is `GEO_FEATURE_NOT_FOUND`.
- **Amendment 2026-10-23 (issue #63): a ray pick also hits bodies without FeatureIds, by their
  mesh.** The nearest front-facing triangle of the body's own blob answers an element-level hit
  with `feature_id` `None`, so an I-section member or a plate with a through hole is selectable
  without inventing box roles for it; a ray through the hole hits nothing, and back faces follow
  `include_backfaces`. The hit is the exact triangle surface, not an approximation. This extends
  the F06 0.1.0 hit record, which does not know a null `feature_id`; recorded here as this
  fix's decision (issue #63): a body the Create panel builds and renders must be
  selectable, and no stable role exists for it yet. The point form and
  `snap` still consider box features only. A body a query cannot consider at all (the point form,
  no face target) is listed in `skipped`. The mesh scan is bounded by a ray–AABB
  test against the element's world bounds, and the per-element work by the blob caps the snapshot
  already validated on admission.
- **Body kind from the element kind.** Plates are `plate:`, beams and columns `member:`, as F06
  section 5 requires. The kernel's member mesh reuses the plate face-table constants, whose tokens
  carry a `plate:` prefix; the engine does not copy those tokens, it builds the local id from the
  element kind and the role. (Recorded as a kernel naming finding below.)
- **Ids are names, never indices.** A FeatureId is built from the rule version, the identity, the
  body kind and the role only. A move keeps it and moves the feature; a copy gets new ids with the
  same local parts. No hit, lookup or snap result carries a triangle, vertex, instance or buffer
  index. A local part that is a bare number, and the CLI options `--triangle`, `--vertex-index`
  and `--instance`, are `GEO_TRIANGLE_ADDRESS_FORBIDDEN`.
- **Snap.** `vertex` candidates are box vertices with their FeatureIds. A member's axis runs between
  the centres of its `f.z0` and `f.z1` faces, which F06 section 6 places at the authored axis points
  plus the translation; `axis_end` returns those two points (parameter 0 and 1) and `axis` the
  nearest point of the segment with its parameter. An axis has no FeatureId in rule `fid1`, so it is
  addressed by element, kind and parameter, not by a new id form. Plates have no axis. Ordering:
  quantized distance, kind (vertex, axis end, axis), ElementId, FeatureId, parameter.
- **Read-only.** Pick, lookup and snap only read the pinned snapshot; the head is unchanged
  afterwards. A foreign reference wrapper has no engine body in the snapshot, so foreign content is
  never a hit and nothing about it is writable through these queries.
- **Exact band arithmetic.** The band test is `distance <= radius + 0.001` in binary64. A point "at
  exactly 0.001 mm" is not representable at every magnitude, so the tests probe just inside and
  just outside the band.
- **Work bounds.** At most 65,536 elements are scanned (`GEO_PICK_LIMIT` above), each with at most
  26 features; kept candidates are cut back to `max_hits` whenever more than 1,024 accumulate, so
  memory does not grow with the model.

## Requirement coverage

F06 section 9 `Pick` and `LookupFeature` for first-scope boxes: ray and point forms, faces, edges,
vertices, back faces, boundary flag, ordering and truncation, the input and limit codes; F06
section 5 rules 1 to 5 and 8 to 10 (`F06-POS-28` to `F06-POS-33`, `F06-POS-43` to `F06-POS-45`,
`F06-POS-47`, `F06-NEG-36`, `F06-NEG-22` to `F06-NEG-29` as far as they concern pick and lookup
inputs). Engine unit tests and CLI tests exercise them; no external oracle compares results.

Provenance: everything follows the repository's F06 contract; no external product material was
used.

## Evidence

- `apps/beamcraft-cli/tests/pick.rs` (5 end-to-end tests through the binary): positions derived from
  the command inputs and the F06 frame rules (plate top face, a horizontal beam's `f.y1`, the
  plate's far corner, vertex and edge and face counts around it, axis start and mid point); ids
  survive a move and differ on a copy; index addresses, dangling and malformed ids; an I-section
  member and a holed plate are hit at element level by their mesh, a ray through the plate's hole
  hits nothing, and an attached foreign reference is never hit (the holed plate is in `skipped`
  only on the point form); every limit and input error; the head is unchanged by queries.
- Engine unit tests: FeatureId grammar and refusals, the 26 features with outward rectangular faces,
  world equals local plus translation, front and back face hits, the inclusive band with `t`
  clamped at the segment end, ordering and truncation, input validation, snap; a body without box
  features (a kernel prism) is picked by its mesh at element level, with the mesh hit's front and
  back faces, the miss beyond the slanted edge while still scanned, and the `skipped` cases.

## Not done

- No cancellation token or request deadline polling inside a pick (the engine's query API has none
  yet); the scan is bounded by the element and feature caps instead.
- No local-space pick, no pick against a simplified representation, no paging of pick results.
- The V03 viewport does not call these queries yet (V05 adds tools).
