# A03 removal and persistent cut identities

`remove-cut --store ROOT --id PART_UUID --operation-id RECORD_UUID` removes one
owned record. Common operation/revision/actor/project/workspace identity options
remain available; the logical request op ID is separate from the removed UUID.
Model `RemoveCut`, engine `remove_cut`, and the adapter's typed `RemoveCutRequest`
rebuild the whole canonical P11 group and publish once. Unknown records or failed
body certification refuse the entire command. Undo/redo restores the complete
operation state and quantities; nothing writes to a foreign model.

## Identity and USD contract

The schema descriptor adds two optional, shared shape-definition properties:

- `steel:cut:nextOrdinal`: int 0–1000; 1000 is exhausted.
- `steel:cut:retiredOperationIds`: sorted unique UUIDv7 strings, at most 1000.

Absent counters derive from the highest live ordinal plus one. Explicit counters
cannot regress below live records. Removing a record retires its UUID and keeps
the counter. Future cuts allocate from the counter, so `cut:f.pN` never rebinds
an explicitly removed cap on the current branch of model history. A new hole
cannot reuse the retired UUID either. New independent copies have new owners and
new operation UUIDs; they do not inherit irrelevant retired identities. P11
instances share the canonical history. Undo restores that earlier history state,
just as it restores earlier authored operations.

An empty replacement writes five typed empty arrays, masking weaker owned-layer
opinions. Fresh uncut elements still write no cut attributes. Retirement reads
the composed source, not just the edit-target layer. Capture, structural
validation, semantic footprints and rebase recognize both new fields. The
structural validator also checks project-wide cut/hole record UUID uniqueness
on owned canonical parts. Foreign and future-schema content remains inert.

The schema pin changes. Unsupported old pins refuse explicitly under the
existing pre-alpha policy; no automatic migration or rewrite is performed.
History metadata does not enter body shape or quantity signatures.

## Retries and evidence

Immutable accepted command evidence pins the removal arguments. A matching retry
returns the accepted post-removal revision/body after subsequent cuts or deletion
without applying anything again. The response names the removed UUID and does
not invent a live ordinal for a deleted record.

Native layer tests cover retirement, exhausted/malformed history and rejected
reactivation. Real CLI tests cover the last-record case, fresh-process reopen,
linked instances, undo/redo, later request replay, unknown records and cross-kind
UUID reuse refusal. Engine tests cover surviving ordinals and exhausted counters.
