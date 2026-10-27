# T03 linear and polar arrays

Contributor-original Rust and synthetic fixtures, MPL-2.0.

`array-linear --store ROOT --ids UUID,... --step X,Y,Z --count N` creates N new
copies per selected part at 1..N times the world step. `array-polar` replaces `--step` with
`--center X,Y,Z --axis X,Y,Z --angle RADIANS`; each copy rotates its original placement
by its slot's angle, not the previously rounded copy. Both accept `--plane`, common context
options and `--instance true|false` (default false). Originals remain. Counts are positive
integers; at most 128 new parts are admitted across the entire request.

The existing group transaction handles all copies in a single prepared revision and undo
unit. IDs derive from the operation, source and slot. Fingerprints include the ordered admitted
transform list and copy mode; retries read the accepted revision's immutable evidence.
Coordinate overflow, an invalid target, missing definitions or a footprint/instance budget
refusal publishes nothing. Zero step and full-circle rotations still create distinct parts.

Independent group copies preserve internal P11 links when their canonical owner is selected:
new instances link to that slot's new canonical owner. An instance selected without its owner
becomes an independent effective-input duplicate. Linked mode intentionally shares the original
canonical owner across every slot. Layer admission now allows canonical parts created in the
same prepared group, validates kind/no chains, and includes their semantic dependency reads.
Level links retain the existing external-level policy. Unsupported object kinds and unknown
external relationships still refuse the selection; no implicit assembly expansion occurs.

This completes the supported part-only transform surface. Native UI and transforms of assemblies,
connections, grids, helpers or foreign references remain outside this headless part selection.
