# T01.5: pinned grids and rights

Contributor-original MPL-2.0. Closes fork #360 (upstream #461). Builds on the delivered U19
scope routing (#371), the F12 descriptor and pins (#372) and the zoned Grids layers (#375);
behaviour follows the T01 contract (`docs/contracts/t01-project-grids-draft-v0.1.md`) §9 and
the F10 0.2 / F12 scope rules. This slice does not claim the T01 parent gate.

## Rights

Grid and level writes answer to the shared grids/levels scope of the workspace's own zone
partition, exactly as U19 routes `create-grid`, `update-grid`, the four line commands and
`create-level`. The generic commands now follow the same rule: `delete-element` routes by the
resolved element kind, so deleting a grid or a level from a workspace without the shared
authority is `PERMISSION_DENIED` (DV-SCOPE-004), and an authorized workspace can actually
delete them instead of being refused by the element scope. `copy-element` of a grid already
dispatched into the grid copy path and inherits the same gate. Reads stay open for every
section user (`list-grids`, `resolve-grid-*`, `snap`, `--start-at-grid` placement, `--level`
heights).

An ordinary section edit authors only into the section layer of its authorized partition; the
grids/levels layers (`<code>[_<zone>]_Grids.usda`) are carried byte for byte, which the
publication scope admission (DV-SCOPE-003) and the new end-to-end scenarios assert on the
real layer bytes of both partitions.

## Pinned coordination grids, read-only

A section project pins a coordination project with `connect-project` (F12, exact source
revision, external `pins/<connection>/<layer>` entries). This slice consumes those pins for
grids:

- **Addressing** (the part T01 §9 left to this task): a pinned grid is selected as
  `CONNECTION:GRID` — two UUIDv7 identities, the connection and the GridId (the identity of a
  pinned grid is `(project, GridId)`). `resolve-grid-point` accepts it in `--grid`, and the
  placement addresses (`--start-at-grid`, `--end-at-grid`, `--origin-at-grid`) accept it as
  the grid token of `GRID:LABEL,LABEL`, so `CONN:GRID:B,3` parses by splitting at the last
  colon (labels never contain one). Any other colon form is refused `INVALID_ARGUMENT`, never
  silently read locally. `list-grids --connection UUID` lists the connection's verified grids.
- **Read path**: the pinned source revision is opened as a standalone verified view of its own
  identity (the same machinery as `resolve-connection`); its grids are read with the ordinary
  grid reader and never join this store's composition. The intersection point is computed in
  the source frame and mapped by the connection's declared rigid placement (PC-FRAME-001) to
  the parent-frame plan point. A connection without a placement is an explicit
  `DEPENDENCY_UNAVAILABLE` unknown alignment, never identity. Height stays the caller's
  (`--z` / `--level` + `--offset`); the stored command keeps plain parent-frame coordinates,
  so the non-magnetic replay semantics are unchanged.
- **Refusal matrix**: a grid write whose target resolves in a pinned view but not locally is
  `FOREIGN_READ_ONLY` with rule `PC-OWN-002`, decided before any local miss and before the
  scope gate — no authority of this store reaches another project's bytes. Covered for
  `update-grid`, `draw-grid-line`, `relabel-grid-line`, `remove-grid-line`, `copy-element`
  and `delete-element`, from a classic section store and from scoped workspaces with and
  without the shared authority. A locally unknown id that no pin authors keeps the existing
  refusals. Pinned `resolve-grid-plane` is refused `INVALID_ARGUMENT` in this slice (point
  queries only); pinned level reads and snap candidates over pinned grids are follow-ups,
  outside the acceptance rows.

## Evidence

`apps/beamcraft-cli/tests/t015_grid_rights.rs` runs the real binary: the byte-equality of both
grids layers across a KMD rename and move (GR-P11 positive), the full `PERMISSION_DENIED`
surface for ten grid/level write commands from a workspace without the shared authority
(GR-P11 negative), the GR-P08 coordinates of a member placed on a pinned grid under a declared
translation (GR-P12), the untouched coordination head, and the `FOREIGN_READ_ONLY` matrix
(GR-N16) with unchanged heads after every refusal. The `COMMANDS.md` reference is regenerated
for the new `--connection` option and the `CONN:UUID` grid selector form.

Contributor-original MPL-2.0. No dependencies or assets added.
