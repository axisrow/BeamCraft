# M2 CLI integration acceptance

This tracks the approved engine/API and CLI slice. It does not close the M2 UI
cards or claim that panels, windows, keyboard actions or GPU rendering are done.
The feature stack includes master `36ca241` (D08), fetched and merged through every
branch without rewriting published commits. Merge in the order below; no automatic
merges are authorized. Drafts remain until checks and dependent changes are accepted.

| Order | Scope | PR | CLI delivery |
| --- | --- | --- | --- |
| 1 | T02 integration of #219 | [#238](https://github.com/axisrow/BeamCraft/pull/238) | Helpers, combined snaps, inactive deletion, BOM/numbering exclusion |
| 2 | T03 rigid group transforms | [#239](https://github.com/axisrow/BeamCraft/pull/239) | Move, rotate, independent/linked copy; one revision/undo |
| 3 | T03 independent mirror | [#240](https://github.com/axisrow/BeamCraft/pull/240) | Reflected authoring/cuts, proper rigid pose, no negative scale |
| 4 | T03 linear/polar arrays | [#241](https://github.com/axisrow/BeamCraft/pull/241) | New-copy count, deterministic identities, internal P11 source remapping |
| 5 | W01 personal saved cameras | [#242](https://github.com/axisrow/BeamCraft/pull/242) | Native views.usda, private atomic CAS, provenance and restore |
| 6 | V09 plane builders | [#243](https://github.com/axisrow/BeamCraft/pull/243) | 3D, plane and two-point construction |
| 7 | V12 named anchored views | [#247](https://github.com/axisrow/BeamCraft/pull/247) | Grid/axis and level identities, re-resolution, unresolved sources, work area |
| 8 | V15 clipping | [#248](https://github.com/axisrow/BeamCraft/pull/248) | Explicit planes, work box, depth band; saved-view export |
| 9 | I04 user attributes | [#250](https://github.com/axisrow/BeamCraft/pull/250) | Five types, exact units, sets, native USD, footprints/undo and unknown preservation |
| 10 | V14 shared filters | [#252](https://github.com/axisrow/BeamCraft/pull/252) | Typed AND/OR, direct/personal queries, view visibility, pinned pagination |
| 11 | V16 modes | [#253](https://github.com/axisrow/BeamCraft/pull/253) | Five modes, explicit snapshot-bound selection, read-only presentation |
| 12 | V17 representations | [#254](https://github.com/axisrow/BeamCraft/pull/254) | Ordered filter appearances, original P15 palette, ephemeral overlay |

## Acceptance boundaries

- T03 admits owned evaluated plates and straight beams/columns, including supported
  P11 instances. Unsupported selections refuse before publication. It does not
  silently expand assemblies or transform grids, helpers, connections or foreign
  models. Existing DV-REL-011 external relationship policies remain in force.
- Personal view anchors do not enter the shared model dependency index: deleted or
  inactive grid/level sources restore as `unresolved`. Multiple view IDs exist;
  window management is UI work.
- Clipping returns effective planes and a conservative visible-element mask.
  Renderers still need to clip triangles. Complete engineering meshes, quantities,
  measurements, snapshot tokens, BOM and geometry exports remain unchanged.
- I04 definitions are immutable through this CLI. Unknown user properties survive
  ordinary edits and are inspectable; copying unsupported opaque values refuses.
  Numeric units match exactly; there is no implicit unit conversion. Old schema
  pins/formats refuse explicitly, without automatic migration or rewriting.
- V16/V17 export renderer-independent data. GPU frame-time acceptance and shortcuts
  remain open. The local CPU baseline is informational, not GPU acceptance.
- Personal-file CAS is cooperative. A stale/crashed lock refuses; it is not silently
  stolen. No new untrusted-directory or cross-filesystem durability guarantee is made.

## Validation of the combined head

Local Linux checks passed on the combined implementation (`9e052fc`), followed by
rustdoc-only range-literal fixes in final head `ee2bc51`. No engineering or display
implementation changed after the full test run. Documentation was rerun after the fix.

| Gate | Actual result |
| --- | --- |
| `cargo fmt --all --check` | Passed |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed |
| `cargo nextest run --workspace --locked --no-fail-fast` | 1,687 passed, 0 skipped, 144 binaries; 244.468 s test time |
| `cargo test --workspace --doc --locked` | Passed; no doctests defined |
| `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked` | Passed after correcting rustdoc range literals |
| WASM check: geom, model, kernel | Passed for wasm32-unknown-unknown |
| `cargo deny check` | Advisories, bans, licences and sources passed; existing unmatched allowances are warnings |
| `cargo run --locked -p beamcraft-boundaries -- check` | Passed |
| `cargo run --locked -p beamcraft-fixtures -- verify --dir tests/fixtures/f07` | 34 files and 33 manifest digests verified |

Real CLI tests cover grouped rigid operations, mirrored asymmetric sections and cut/
holed plates, double reflection, arrays with internal P11 remapping, replay after
later edits, atomic refusal and undo/redo; personal views across processes, anchor
changes/unresolved states; typed attributes, unknown preservation, filter parity;
five modes, first-match appearances, palette values and ephemeral overlays. Bundle
byte equality and unchanged model reads verify presentation isolation.

GitHub Linux/macOS checks are green through V14 at this update. V16/V17 checks
were rerun after the rustdoc correction and remain subject to their final PR check
state. Windows PR jobs are skipped by the existing CI policy; no local Windows
verification is claimed. Dependent feature PRs remain draft while prerequisite
integration/acceptance gates are open.

The design notes under `docs/design/` specify the public CLI/API contracts and
transport/schema changes. BCVXFRM3 carries presentation JSON version 3 followed by
an unchanged complete engineering bundle. Requests without display options preserve
legacy raw/V1/V2 output. UI consumers must opt into and validate the new contract.
