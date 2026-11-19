# R03 explicit reference update

Date: 2026-11-19. Status: **experimental slice; R03's result (source versions and comparison of
supported properties and geometry) is delivered for the supported profile, and K3 is not
claimed**. The command is `update-reference`, per the F04 naming rule.

Plan row: R03 / M, depends on R02 and U05 (both merged). Result: explicit re-read of a
reference's source, as a new pinned version, with a comparison of the supported properties and
geometry of the two snapshots. Acceptance: no automatic update; nothing written to foreign
prims (or to the source files, or to foreign prims' namespace in any revision).

## What is implemented

```text
beamcraft update-reference --store ROOT --id UUID --source FILE [--op-id UUID ...]
  path text --check (no I/O)--> bounded read of the new file --layer engine (inspect)-->
  new pin (digest, size, format, default prim, declared units) --> version = pinned + 1
  --> replay decision --> engine admission (UpdateReferenceSource) --> R02 measurement of
  the old snapshot --> layer engine (U02, Change::ReferenceSource): pin attributes and arc
  replaced as a whole, units interpretation cleared --> footprint check --> publish (the new
  bytes carried as a retained external layer; the old layer never dropped) --> re-read -->
  R02 measurement of the new snapshot --> comparison in the reply
```

| Area | What exists |
|---|---|
| `beamcraft_model::reference` | `UpdateReferenceSource`: the new `SourcePin` plus the wrapper's identity; the versioned fingerprint `BeamCraft/R03/UpdateReferenceSource/v1`. |
| `beamcraft_engine::reference` | `update_source` admission: the pin is re-admitted, the version must move forward, the pinned bytes are refused as the target, and the result starts with no units interpretation; footprint `ShapeInputs` + `Derived`. |
| `beamcraft_layer_engine` | `Change::ReferenceSource`: `check_source_asset` (the new layer must be a non-owned manifest entry with exactly the pinned bytes), the pin attributes replaced as a whole (declared units removed first, so a snapshot that declares less records exactly that), the units level cleared, the arc re-pointed at the new layer and default prim; kind check `wrapper` in the change validator; descriptor pin extended. |
| `beamcraft_commands::reference` | `update_wrapper_source`: the attach pipeline with the replacement change, plus `Comparison`/`SourceSide`: supported properties on both sides, the R02 measurement of each composed surface (`unsupported` carries the refusal code where the supported profile cannot measure), and the bounded added/removed source-path diff. |
| `beamcraft-cli` | `update-reference`: one JSON object; registry entry, help text, `COMMANDS.md` row. |

## Decisions

- **Nothing is automatic.** A changed file on the disk is never noticed: the head keeps its pin
  until this command names a new file. The reply states the two snapshots; nothing watches,
  hashes or stats source files in the background, and there is no refresh, timestamp or
  staleness lifecycle (W03 was removed from scope by the owner's decision of 2026-10-28).
- **Explicit, but the wrapper stays the owner's.** The update keeps the wrapper's identity,
  label and placement, and the revisions that recorded them. Only the pin moves, by exactly one
  version: the version is the wrapper's own counter of accepted snapshots, not a clock and not
  a per-bytes number; the digest identifies the snapshot.
- **The units interpretation does not cross.** A correction is stored with the digest it was
  made for (`correctionSourceDigest`); a new snapshot starts uncorrected and the result says
  `units_cleared: true`. The revision that recorded the correction keeps it, and the owner
  re-states the interpretation explicitly if it still applies. Keeping it would be an integrity
  failure on the next read; asking the owner to clear first would add a step that states less.
- **The pinned bytes are never the target.** Updating to the snapshot the wrapper already pins
  is `INVALID_ARGUMENT`: they are that version, not a new one. An earlier snapshot may be
  pinned again explicitly with a new version — a rollback is an accepted snapshot like any
  other. There is no automatic rollback of any kind.
- **Old revisions and old bytes stay.** Revisions are immutable, and the update never drops a
  stored layer: the new revision retains both external layers, so the earlier snapshot remains
  pinned in the revisions that recorded it and its blob stays verified in the store. The user's
  source files are only ever read (bytes and mtime are untouched, and no store byte names the
  path of any source file).
- **Comparison: supported only, unsupported marked.** The reply compares what BeamCraft
  supports: the pin's properties (digest, version, default prim, declared units) and the
  composed prim tree (bounded added/removed source paths), and on both sides the R02 kernel
  measurement of the composed surface — the same certification, enclosures and refusals as
  `measure-reference`. A surface the supported profile cannot measure (`UNSUPPORTED_GEOMETRY`, a
  budget code) is reported as `unsupported` with its code and `measurement: null` on that side:
  the properties are still compared, no geometry is invented, and the update itself is not
  blocked by what a later file contains. Like R02, the measurement is a pure reading: the same
  head always answers with the same numbers.
- **Reads reused, nothing new invented.** The restricted local reader, the admission order, the
  manifest roles, the strict verifier and the R02 measurement are the existing ones; the update
  adds no reader, no network path, no IFC converter and no persistent measurement object.

## Negative cases (tests)

| Case | Result |
|---|---|
| File on the disk rewritten, no command | head keeps its pin (`inspect-reference` unchanged) |
| Update to the pinned bytes (`INVALID_ARGUMENT`) | refused, nothing written |
| Unknown wrapper | `NOT_FOUND`, nothing written |
| URL/network path | `PERMISSION_DENIED` before any I/O |
| Missing file / directory | `NOT_FOUND` / `INVALID_ARGUMENT` |
| New layer absent from the manifest | `DEPENDENCY_UNAVAILABLE` (layer engine) |
| Manifest bytes differ from the pin | `INTEGRITY_ERROR` (layer engine) |
| Version that does not move forward | `INVALID_ARGUMENT` (engine admission) |
| Same operation id, same content | replayed, nothing written |
| Same operation id, other content | `OP_ID_REUSE` |
| New snapshot outside the measurement profile | update succeeds; that side reports `unsupported` with the code, `measurement: null` |

## Evidence

* `crates/model/src/reference.rs` unit tests: fingerprint sensitivity (label, version, digest)
  and pin admission.
* `crates/engine/src/reference.rs` unit tests (2): the pin moves forward with the units
  interpretation reset and the `ShapeInputs`+`Derived` footprint; refusals for a stalled
  version, the pinned bytes, and another wrapper's id; rollback as an explicit new version.
* `crates/layer-engine/tests/reference.rs` (2): the update re-points the wrapper at the new
  snapshot under its own default prim, clears the correction, retains both foreign layers byte
  for byte, and validates offline; refusals for a missing layer and a layer that differs from
  its pin.
* `apps/beamcraft-cli/tests/reference.rs` (5, real binary): the full update — version 2 with
  identity/placement preserved, units cleared and stated, before/after measurements (6 m3
  corrected, then 13 units as stored), the `/Site/Annex` diff, both layers retained, the source
  files untouched; no automatic update plus every refusal writing nothing; replay and
  `OP_ID_REUSE`; two wrappers sharing one stored layer with independent version counters; an
  unmeasurable new snapshot marked, not blocking.

## Not done and limits

- **Supported profile unchanged.** The comparison measures what R02 measures (translated Cube
  and Mesh cages with explicit `subdivisionScheme = "none"`, the U01/display budgets); raising
  it is the separate decision R02 already records. A source whose *properties* the reader
  refuses (no default prim, hostile layer) is refused as an attach would be: the update never
  pins bytes the store cannot hold.
- **No detach, no per-prim comparison.** Removing a wrapper is still not a command; the prim
  diff is per source path, not per attribute value.
- **The comparison is a reading, not a stored result.** Nothing is persisted (the owner's
  2026-10-28 decision); a caller that wants the comparison later re-runs the command or reads
  the revisions.
- One wrapper per command; a batch update of many wrappers is not offered.

## Provenance

Original code and tests, MPL-2.0. The foreign models in the tests are invented text written by
the tests; nothing of any product or sample is used. The version-counter and digest policies
follow the R01 pin contract (`SourcePin::version`, "only an explicit source update changes it")
and the owner decisions recorded in issue 128. No new dependency.
