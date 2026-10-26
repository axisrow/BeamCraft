# T01.1: regular project grids

Contributor-original MPL-2.0 implementation based on the proposed T01 amendment in
[PR #415](https://github.com/Apibim-com/BeamCraft/pull/415), extending the draft delivered for #192.
The amendment remains a review dependency. This slice does not claim the T01 parent gate accepted.

## Headless path

`beamcraft_model::grid` owns bounded regular expressions, explicit Unicode labels, numeric and Latin
alpha label helpers, grid records and typed creation/copy requests. The engine checks project-wide
identity collisions and the 64-grid cap. Creation reads the absence of each new GridId and AxisId;
copy also reads the source's lifecycle, shape, placement and label. Publishing requires the pinned
base revision, so a concurrent project change cannot silently bypass those checks.

The layer engine authors one Xform under `/World/Grids/GRD_<uuid>` with the explicit `bim:grid:`
arrays from the contract. `T01-V01` reconstructs every array and rejects count disagreements,
expression/point drift, invalid spans or labels, repeated identities and over-limit projects.
No hidden JSON representation or authored geometry is introduced. As with other schema additions
in this pre-alpha, the mapping pin changes; older store pins are not migrated by this PR.

The frame stores Z/W quaternion components exactly, matching the authored USD orientation. Copies
preserve them; conversion back to radians is for reporting only. A grid has no elevation or body.
`bim:discipline = steel` records the currently implemented authoring module. Shared-scope rights
remain T01.5, together with U19/F12, under the contract's explicit transition rule.

## CLI

```sh
beamcraft create-grid --store /absolute/existing/store --name Main \
  --u-expression '0 3*6000' --v-expression '0 7500 4500'
beamcraft list-grids --store /absolute/existing/store
```

Each regular family accepts `--u-labels`/`--v-labels`, `--u-scheme`/`--v-scheme`,
`--u-axis-ids`/`--v-axis-ids` and `--u-span`/`--v-span`. Helpers also accept
`--u-label-start`, `--u-label-step` and `--u-label-order` (and the corresponding V options). An omitted family is empty and drawn;
nonempty drawn lines enter with T01.2. At least one expression must produce a line. U defaults to
numeric labels, V to Latin alpha. Source expressions retain repetition syntax and normalize only
separators. Explicit labels permit Cyrillic and `/`, and compare exactly across both families.
`--origin X,Y` is in millimetres; `--rotation` is in degrees within ±360. Default spans extend
2000 mm beyond the other family's coordinate range and are stored rather than recalculated later.

`inspect-elements` includes the grid records. `copy-element --id GRID` creates a fresh GridId and
fresh AxisIds, preserving the expressions and labels; `--new-axis-ids` specifies them in U-then-V
order. `--offset X,Y,0` moves the independent copy. `delete-element --id GRID` uses the common
incoming-reference guard. Grid creation/copy uses the immutable operation-evidence fingerprint for
exact replay, including a retry after later deletion. For exact retries, supply the operation,
element and all axis identities explicitly.

Workspace undo/redo remains one unit per command. Ordinary part edits preserve grid attributes.
BOM, approved numbering and ordinary IFC export exclude grids; derived viewer tubes are T01.7.
Queries, snaps and grid placement are T01.4; the native grid tool is T01.6.

## Evidence

`apps/beamcraft-cli/tests/grids.rs` runs the real binary against independent store directories:
regular source expressions and endpoints (GR-P01), Unicode labels and rotated placement,
200-line boundary, refusing malformed/over-limit requests without publishing, immutable request
replay after deletion, independent copying/deletion, damaged stored arrays, workspace undo/redo,
preservation under part creation, and exclusion from BOM/numbering/IFC. Engine tests cover the
64/65-grid boundary and project-wide AxisId absence checks without generating 64 revisions.
