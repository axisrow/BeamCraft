# T01.2: drawn lines and explicit relabelling

Contributor-original MPL-2.0. Follow-up to T01.1 and the proposed contract in #415.
The stored grid format is unchanged; engine commands atomically replace the family attributes.

`GridLineEditRequest` carries a caller-supplied target, operation-specific arguments and, for a
new line, its fresh AxisId. The engine converts world XY points through the pinned grid frame,
validates the complete replacement record and produces one grid's shape-input footprint. A
failed operation leaves its input and store unchanged. Frame, grid name and family modes stay
unchanged. Relabelling works on one line or the complete ordered label list of a family; it does
not rewrite identities or points.

The CLI adds `draw-grid-line`, `reshape-grid-line`, `remove-grid-line` and `relabel-grid-line`.
An initial drawn line can be supplied during `create-grid` using `--u-from X,Y --u-to X,Y
--u-label TEXT [--u-axis-id UUID]`, with the equivalent V options. This makes a fully drawn grid
reachable without first storing an invalid empty grid or converting a regular family. Points
are world XY millimetres. Grids carry no elevation.

The adapter uses the shared ownership index before edits. An incoming grid reference
conservatively blocks removing a line until axis-specific stored view references enter with W01.
The last line cannot be removed. Commands retain the existing operation-evidence replay rule;
a retry does not resurrect a subsequently removed line. One edit is one workspace undo unit.
Historical AxisId retention across deletion/history compaction remains the lifetime-policy review
point identified in #416; current collision checks use the complete pinned live graph.

Actual-binary scenarios cover a fully drawn grid, mixed/immutable family modes, degenerate and
colliding lines, reshaping and relabelling with stable identities, removal/last-line refusal,
rotated world points, family relabelling, and exact retry after removal. Engine tests cover
reference refusal and input immutability. Acceptance of the parent T01 gate is not claimed.
