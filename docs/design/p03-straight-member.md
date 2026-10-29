# P03 straight member (beam and column)

Date: 2026-10-17. Status: **experimental vertical slice; P03 acceptance is limited as listed below.**
Branch `feat/p03-straight-member`, stacked on `feat/p02-definitions` (PR #63), which is based on
`origin/master` at `3dc1f47`. Merge order: #63, then this PR.

Plan row: P03 / M, depends on P02 and G01. Result: a minimal straight member from a supported
profile with placement and FeatureId. Acceptance: an end-to-end test checks the length, the
local/world transforms and the saved shape; BC-PART-002 coverage is partial.

## What is implemented

```text
beamcraft create-member --store ROOT --start X,Y,Z --end X,Y,Z --profile-digest SHA256
  [--kind beam|column] [--roll RAD] [--material-digest SHA256] [--translation X,Y,Z]
  [--orientation-xyzw X,Y,Z,W] [--name TEXT]
  model::CreateStraightMember (F04 caps) --> engine resolves the pinned profile (required) and
  material (optional), kernel::straight_member builds the body --> U02 Shape::Member + Mesh
  --> host U06/U07 commit; the verifier re-evaluates every plate and member with the kernel
beamcraft inspect-elements --store ROOT   (plates and members, one pinned view snapshot)
```

| Area | What exists |
|---|---|
| `beamcraft_kernel::member` | `straight_member`, see the kernel README. |
| `beamcraft_model::command` | `CreateStraightMember`, `MemberKind`: label, finiteness, coordinate (1e9 mm) and roll (2 pi) caps, placement; fingerprint including kind, profile, material, roll. |
| `beamcraft_engine::member` | `ProfileInfo`/`ProfileLookup`, `AuthoredMember`, `evaluate_member` (profile re-admitted), `create_straight_member`, `admit_create_member` (replay/reuse), `Footprint::for_create_member` (profile and material are read, never written). Order: arguments, duplicate id, profile, material, geometry. |
| `beamcraft_engine::query::committed` | `MemberRecord`, `evaluate_committed_members`, `scene_elements_with_members` (beam and column kinds in the snapshot). |
| `beamcraft_cli::member` | `create_member`; `plate.rs` now decodes beams and columns, and the publish-time verifier checks their derived mesh against the kernel's. Shared `load_base`, `publish_revision`, `prior_op_with`, `replay_summary`, `check_footprint`, `mesh_change`. |

## Decisions

1. **Axis points define the local layout.** A horizontal beam is the local axis
   `(0,0,0) -> (L,0,0)`; the frame rules of F06 section 4 then fix which section edge is vertical
   (width along `Y`, height along `Z`). The P10 member-instance follow-up keeps this body local:
   placement maps the rolled member frame at the scene/USD/IFC instance. The roll composes first,
   the placement second, and neither translation nor placement rotation changes the local mesh,
   shape signature or mass signature. Invalid placement quaternions are still refused.
2. **Profile is required and pinned.** A missing digest is `NOT_FOUND`, a material digest where a
   profile is required is `INVALID_ARGUMENT`; the profile values are re-read from the committed
   definition and re-admitted on every evaluation.
3. **Mass follows P02.** `current` only with a pinned material that stores a density.
4. **The kernel builder returns `PlateResult`.** The type name is historical (G02); the mesh layout
   and blob are identical for plates and members, which keeps the view contract, the mesh blob
   store and the verifier unchanged.

## Not done (not claimed)

* No logical `UpdateElement`, move or delete of a member (P04, P08); no signatures or stored
  quantities (P04); `shapeKey` is the mesh digest as in P01.
* Other profile kinds beyond families and custom outlines, curved members, holes, bolts and
  welds. The [P10 member-instance follow-up](p10-member-instance-pose.md) replaces baked
  rotations with a full instance pose for bounds, features, picking, viewport and IFC export.
* Sin/cos reproducibility across targets for a nonzero roll is still an open owner decision.
* The layer engine runs in the CLI process (LEW-03 open); no workspace or undo (P06, P07).

## Evidence

* `cargo test -p beamcraft-kernel --test member` (6 tests), model, engine and `beamcraft-cli` unit
  tests, and `cargo test -p beamcraft-cli --test cli -- members::` (real binary): a 6000 mm beam of
  200 x 300 mm is 0.36 m3 and 2826 kg at 7850 kg/m3; fresh-process reopen, world bounds from the
  pinned snapshot, a rolled column, plates and members together, `NOT_FOUND`, `INVALID_ARGUMENT`,
  `NON_FINITE_NUMBER`, `LIMIT_EXCEEDED`, replay, `OP_ID_REUSE`, `DUPLICATE_ID` across kinds.
* pxr oracle (local, opt-in, usd-core 25.11): pxr reads the beam and the rolled column, their
  `bim:profile` target and definition, the axis and roll, derives the volume from its own mesh and
  matches the hand-derived closed forms and bounds, and `ComplianceChecker` reports nothing.
