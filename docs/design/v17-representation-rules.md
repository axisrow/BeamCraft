# V17 ordered representation rules

Personal CRUD commands: `save-representation-set`, `list-representation-sets`,
`inspect-representation-set`, `delete-representation-set`, all taking
`--workspace-dir`. Save takes `--name`, optional `--id`, and `--rules FILE`:

```json
{"by_class":true,"rules":[{"filter":"FILTER_UUID","color":[1,0.3,0],"transparency":0.2}]}
```

Rules reference saved V14 filters and run in order: first match wins. RGB and
transparency are finite [0,1]; zero transparency is opaque. At most 64 rules.
Unmatched objects use the mode baseline, or the original P15 class palette when
`by_class` is true. A matching rule replaces both colour and transparency.

Bind via `update-view --id VIEW --representation SET|none`, or override the binding
for one export with `export-view --representation SET`. Missing filters/attributes,
unknown versions and invalid channels refuse before output/publication. Referenced
filters and sets cannot be deleted. Personal changes use views.usda CAS; deleting
a view removes its binding. None of this enters model revisions or undo history.

`export-view --overlay FILE` accepts an ephemeral array of
`{"id":"ELEMENT_UUID","color":[0,0,1],"transparency":0.8}` objects. It has highest
priority, at most 1024 unique known snapshot IDs, and is never persisted. Mode
surface/edge/lighting flags remain in effect. Hidden overlay targets stay hidden.

BCVXFRM3 presentation JSON version 3 includes ordered visible IDs, selection,
global mode flags, representation ID and resolved per-element appearances.
The engineering bundle is still complete and unchanged. Legacy raw/V1/V2 requests
retain their format; consumers must inspect the JSON version before reading new
fields. Metadata is bounded to 32 MiB and parser scratch to 512 MiB (no up-front
allocation of the full bound). File summaries report counts instead of large tables.

Tests cover first-match ordering, original palette values, overlay priority and
non-persistence, private CAS, dependency deletion guards, invalid channels,
unknown rules, and byte-identical engineering bundles. GPU rendering is UI work.
