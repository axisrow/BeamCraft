# A12.3 prerequisite: atomic corner replacement

This slice reuses A12.2 PR #275. `UpdateElement.patch.corners` replaces the complete sparse
list of `PlateCorner` records. CLI: `update-element --store ROOT --id UUID --corners
"0=chamfer:100,50;2=round:25"`; `--corners none` clears it. Unmentioned vertices become sharp.

The engine marks the patch as shape-changing, validates vertex references against the sharp
contour and replays the full body with every existing hole and cut. Invalid geometry refuses
before publication. Members refuse the field. Authored sharp points, placement, identities
and operation records are preserved. P11 updates the canonical definition and every instance
mesh/quantity record in one revision and one undo unit. Independent copies remain independent.

Native `Change::Corners` writes all five A12.2 arrays, including five explicit empty arrays
when clearing, so weaker opinions cannot reappear. Composed-stage validation checks types,
array lengths, counts, vertex uniqueness/range, tokens and finite positive parameters. Capture
and shape footprints cover the complete replacement. The schema descriptor adds this change
kind: unsupported old pins refuse explicitly without automatic rewriting. Existing patch
fingerprints stay unchanged when corners are absent; present replacement/clear has its own
fingerprint extension.

This is a prerequisite slice, not completion of A12.3 or P-Degenerate. Vertex/edge editing,
dedicated set-plate-corner and keep_degenerate/recovery remain outstanding. Generic
UpdateElement retry behavior remains its existing contract; this slice tests immediate retry,
not immutable historical replay after later edits. No UI acceptance is claimed.

Checks and their actual results are recorded in the PR. Synthetic fixtures cover replacement,
clear, a linked group, invalid input without publication, restart, retry and undo/redo. Original
Rust and elementary geometry using the existing A12.2 engine, MPL-2.0; no assets or dependencies.

## The remainder: contour editing commands (A12.3)

The editing commands of the amendment section 3 are delivered on top of the atomic replacement:
`move-plate-vertex`, `move-plate-edge`, `insert-plate-vertex`, `delete-plate-vertex` and
`set-plate-corner`. The engine part is the pure `contour_edit` module: each edit maps the
authored contour and its sparse corner list to the edited pair, remapping treatment positions
after an insertion or deletion and carrying the deleted vertex's treatment away. The CLI commits
the result through `UpdateElement` as one shape-changing `contour_points_mm` + `corners` patch,
so the kernel revalidates the new contour, every hole and cut replays, a linked definition group
updates atomically and the revision is one undo unit. `delete-plate-vertex` refuses on a
three-vertex contour whatever the caller asks. Retry follows the documented `UpdateElement`
contract, decided from the head state: an identical retry replays, an insert or delete under an
accepted op id with moved-on content is `OP_ID_REUSE`. keep_degenerate/recovery (P-Degenerate)
remains outstanding: invalid geometry refuses, as for every update today.
