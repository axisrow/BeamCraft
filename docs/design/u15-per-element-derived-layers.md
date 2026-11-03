# U15: per-element derived layers

Editing one element used to re-serialize every element's derived geometry: all meshes lived in
one shared `derived.usda`, so a single thickness edit rewrote the whole layer, changed its digest
and stored a complete copy again (mesh digests stayed the same; the file did not). Derived
geometry is now isolated per element, so an edit serializes and stores only the edited element's
results.

## Layout

`derived.usda` stays a manifest asset and a root-stack entry, but becomes a thin stack: its
`subLayers` list the owned per-element layers. Each element with derived results owns exactly one
layer, `derived/<32 hex digits of the element id>.usda`, which carries its `/Geometry` prims under
the same absolute paths as before. Composition is unchanged for readers: the stage merges the
root's two sublayers and, under the derived stack, one file per element. The stack is a canonical
set — sorted, exactly the element files the manifest carries — so its bytes do not depend on edit
order.

* `authored::element_layer(id)` and `authored::element_layer_id(name)` are the naming seam; the
  `derived/` prefix is engine-reserved, and an asset outside the fixed name shape is refused.
* A mesh write creates the element's file on first use (units header included) and rewrites that
  file alone. Unchanged elements' files are retained byte for byte: the same digest, no new blob,
  no re-serialization.
* Deleting an element removes its authored prim and drops its file and stack entry with it.
* Result prims must live in exactly one owned location. A pre-U15 store keeps its meshes in
  `derived.usda`; a rewrite of such a mesh empties the legacy location and moves the results into
  the element's file. Reads of old stores are unchanged, and no migration pass exists.

## Consequences and caps

The stage holds one layer per meshed element, so the layer-count ceilings follow the element
budgets instead of a fixed small stack: the engine's model `layers` budget is 70,000 (the
65,536-element view cap plus roots, section and grids layers, external wrappers and coordination
pins), and the host manifest admits the same entry count (`MAX_ENTRIES`) with
`MAX_MANIFEST_BYTES` at 16 MiB. Composition depth stays at three for a project stage.

Manifest entries carry a role; every `derived/<id>.usda` entry is `derived`, and the U19 scope
guard exempts derived layers by role (they follow their owning authored inputs), so scoped stores
behave as before. The rebase materializer merges per-element files like the derived target and
reconciles the candidate's stack from its asset set; the pseudo-root of an edit layer (units,
schema pin, stack) is engine bookkeeping and never merges as content. Changesets already list
every manifest entry with per-layer base digests and carry blobs only for changed layers.

## Remaining limits

* `authored.usda` is still one layer per section: an edit that changes authored content
  re-serializes and re-stores the whole section layer. Splitting authored storage is the other
  half of the per-element isolation decision (U10/U15 follow-up).
* Per-revision metadata is O(elements): one manifest entry (~130 bytes) and one stack line
  (~52 bytes) per element, stored with every revision that changes anything.
* A batch that invalidates derived results without rewriting them can leave an empty element
  file; in practice every invalidating command sends the replacement mesh in the same batch.

## Measured

Rectangular 200×100 plates through the real CLI (store blob growth of one thickness edit):

| Store | Before (master `d39fee5a`) | After (this change) |
|---|---|---|
| 316 plates | derived blob **575,071 B** re-stored (whole layer); store +888 KiB | — |
| 60 plates | ~whole derived layer | element file **2,086 B**; `changed_layers` = `[authored.usda, derived/<edited>.usda]`; the 3,136 B stack unchanged |

The parent acceptance (one edit of 1,000 elements, measured before and after) stays with the
issue's gate. At that scale the CLI's 30-second per-command wall deadline and the publish-time
re-evaluation of every element dominate the edit; both are separate known limits
([layer budgets note](layer-budgets-model-profile.md)).

Regression coverage: engine tests for isolation (an edit rewrites only its element's layer;
delete drops file and stack entry; stack/asset agreement refusals; legacy migration on first
rewrite), rebase transfer of a locally created file, upstream-delete conflict scoping, and CLI
end-to-end assertions through the real binary (changed-layer names, tampered-record refusals,
independent-reader and oracle passes over the nested stack).
