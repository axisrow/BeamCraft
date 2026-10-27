# V12 named model views and work areas

Contributor-original Rust and synthetic fixtures, MPL-2.0.

The existing `create-view` command adds `--kind 3d|level|grid-line` beside V09's plane and
two-point modes. Level views take a level UUID. Grid-line views resolve `--grid ID_OR_NAME`
once, then store its persistent GridId with `--axis AXIS_UUID`; labels and indices are never
saved as identities. Three-dimensional views default to a proper isometric camera around the
world origin, width 10000 mm and aspect 1, with world XY as the tool work plane.

Restore resolves anchors from one verified model revision. Grid-line views follow the current
line plane (including later moves or label changes), and level views follow current elevation.
Missing/inactive grids, axes or levels return `view_unresolved` with `status=unresolved`;
the stored camera is provenance, not a fallback display claim. Restore changes neither file.
Malformed or unsupported construction data remains an error. Model-dependent property edits
recompute the stored native Camera and refuse if the model moved during the read/capture.

`--work-area MINX,MINY,MINZ,MAXX,MAXY,MAXZ` records a finite ordered world box per view;
`update-view --work-area none` clears it. Constructed views also accept depth and side edits.
Their width/aspect/depth/side and work area are returned by inspection/restoration. V15 turns
these presentation inputs into clip parameters; this slice does not modify engineering bounds.
Multiple cameras are independent records/UUIDs; native windows and window management are out.

Views are private workspace state by the owner's decision. Their anchors do not enter the
model index or introduce cross-user model dependencies. Deleting an otherwise unused grid or
level can therefore leave a personal view unresolved; it is not blocked by this private file.
The shared-view dependency-conflict acceptance in the grid contract is not claimed here.

CLI tests cover stable grid/axis binding through a grid move, level/3D construction, multiple
views, saved work areas and property edits, deleted-anchor diagnostics and unchanged personal
bytes during restore. Engine tests cover elevation changes, missing anchors, default 3D camera
admission and invalid work areas. Prior W01/V09 CLI tests remain part of the regression run.
