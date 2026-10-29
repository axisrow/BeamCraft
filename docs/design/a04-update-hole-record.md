# A04 hole record replacement

`update-hole --store ROOT --id PLATE_UUID --operation-id RECORD_UUID` takes one
replacement `--polygon`, `--hole-rect` or `--hole-round`, plus common identities.
`--op-id` is the new logical request identity; the existing record UUID is required.
The typed `UpdateHole` command and engine `update_hole` replace its authoritative
polygon while keeping its ordinal/order. Legacy create-authored holes and all
other operation records stay intact. The complete body replays before publication;
failed validation/certification changes no revision.

The existing AddHoleRequest context is reused by update_hole_command. A P11 edit
writes the canonical definition and every derived instance in one transaction;
placements remain local to instances. Workspace undo/redo restores the whole group.
There is no schema change in this slice beyond the preceding A03 retirement base.

Both add-hole and update-hole now decide retries from immutable committed request
evidence and return their original accepted revision/result, even after later edits
or deletion. Missing evidence fails closed. Reused op IDs with different typed
arguments refuse. No member-hole support or legacy index-addressed edits are added;
G14.1/A04.2 and remove-hole are separate slices.

Original synthetic engine/CLI cases verify exact rectangular-hole quantities,
legacy-hole preservation, identity/order, linked-instance edits, unchanged placements,
undo/redo, retries after edits/deletion, missing records, bad winding, overlapping
holes and resource limits. No external geometry or assets are introduced.
