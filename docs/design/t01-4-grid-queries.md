# T01.4: grid queries, snaps and non-magnetic placement

The engine resolves a grid by canonical GridId or an exact unique live name. It expands the
admitted regular/drawn families into at most 400 finite segments per grid. Intersections use the
local frame before world translation, include endpoints and require different families; parallel,
collinear and outside-segment pairs are undefined. Grid-line planes are vertical.

`resolve-grid-point --grid ID_OR_NAME --labels U,V` and `resolve-grid-plane --grid ID_OR_NAME
--label TEXT` are read-only CLI queries. The snap API adds grid intersections, ends and lines;
`--grids UUIDS` filters visible grids (empty means none, omission means all). Grid hits measure XY
distance and preserve query Z, carry persistent grid/axis identities and labels, and have no
FeatureId. Only nearby segments participate in intersection pairs. Candidate storage is bounded
by the result limit and final ordering follows V04 distance bands, kind and persistent identities.
The CLI reads grids at the same immutable sequence as the engineering view snapshot.

`create-member` accepts `--start-at-grid` and `--end-at-grid` instead of the respective XYZ point;
`create-plate` accepts `--origin-at-grid` instead of a translation. The address is
`GRID:LABEL,LABEL`; a name containing a colon must be addressed by id. Height is explicit `--z` or
`--level UUID [--offset MM]`; a level supplies elevation only. Combining grid placement with a
translation/orientation, or giving an offset without a level, is refused. Other direct points keep
their explicit XYZ. The resulting engine command contains ordinary coordinates and no grid link.

For a retry, the adapter first checks the host operation outcome. An accepted operation resolves
its grid and height from that operation's verified immutable revision, with pinned command
evidence, rather than the current head. The ordinary command fingerprint still decides replay or
operation-id reuse. New requests resolve the current verified head. Explicit element/operation
identities are required for exact retries, as for the other creation commands.

This slice follows #419 and the contract proposal #415. Tests cover drawn crossings, endpoint
inclusion, rotated frames, exact names, grid filters and bounded results, real CLI query ordering,
height selection, non-magnetic placement and replay after grid edits/deletion. Historical AxisId
retirement remains the separate review point in #416.

Contributor-original MPL-2.0. No dependencies or assets added.
