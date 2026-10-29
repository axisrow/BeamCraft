# A03 cut record updates

`update-cut` takes the same target/record/plane options as `cut-by-plane`, with a
new logical `--op-id`. Its typed model command is `UpdateCut`; engine `update_cut`
replaces the existing record in memory, retaining its UUID, ordinal and order.
The adapter replays the entire body, updates the canonical P11 definition and all
linked derived bodies, checks footprints, and publishes exactly one revision.
A refused plane or failed certification writes no revision. One workspace undo
restores the complete group; redo reapplies it.

Immutable command evidence from the accepted revision decides retry identity.
Create, update and remove requests use the same immutable evidence lookup.
A matching retry returns the original accepted revision and element, even after
later edits or deletion; it does not reapply the plane. Reusing an op ID with
other parameters refuses. Missing or malformed evidence fails closed.

Cut coordinates remain element-local millimetres, as for the existing create
command. No geometry, schema or old create-request fingerprint changes are made.
Record removal and persistent ordinal retirement are a separate A03 slice:
removing the last record must never allow a future cut to reuse its FeatureId.

Synthetic tests cover plate/member volumes, retained record identity, linked
instances, undo/redo, replay after later edits/deletion, invalid planes, missing
records and op-ID reuse. Existing A03 create/copy/reopen tests remain in place.
