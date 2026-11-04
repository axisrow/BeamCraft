# W01 saved views: provenance on the camera of the views layer

A saved view is the camera prim of the views layer: its frame (`ModelView::camera_plane`), its
projection, visible height, rotation and display properties, plus the W01 provenance stamp that
says who saved the camera, when, why and against which model revision. The record lives in the
views layer (`beamcraft_engine::views::ViewProvenance`, mapped into the session sidecar by
`beamcraft_host::views`), never in the model: saving or restoring a view still writes no store,
so model keys and numbering are untouched — the e2e test proves the store head byte-identical
across every view operation.

## The provenance record

`ViewProvenance` carries exactly the contract's four facts: `author` (1 to 128 characters, no
controls, trimmed), `created_at_ms` (unix milliseconds of the save), `comment` (at most 512
characters, no controls, may be empty) and `model_revision` (the store head `Uuid7` at save
time). The engine validates the record wherever a view is validated (book insert, update and
the file read), so a stamp that breaks the bounds refuses the whole layer file.

## Delivery and reuse

The V12 named-view commands carry the feature — no parallel feature was built:

* `save-view --author TEXT [--comment TEXT]` stamps every view the command creates (a batch
  shares one stamp: one capture event). `--store` is required so the stamp can read the model
  head revision, and the time comes from the clock. A `--comment` without `--author` is
  refused, so a stamp never records an unknown author. Without a provenance option the save
  behaves exactly as in V12 and stores no stamp.
* `update-view` never rewrites the stamp (`--author`/`--comment` are not its options): the
  provenance records the capture, and a new capture saves a new view.
* `apply-view` restores the camera and returns the stamp beside it; `list-views` lists it.

The session sidecar becomes format version 2 and writes the stamp as
`provenance: {author, created_at_ms, comment, model_revision}` or null; version 1 files
(without the key) still read, stampless.

## The USD form of the views layer

The USD form is delivered in two steps. The layer profile:
`beamcraft_layer_engine::authored::views` is the closed views-layer profile — `bytes`
produces the canonical layer text (a `bimViewsSchemaDigest` pin, then one standard USD
`Camera` prim per saved view under `/World/Views`, plus `bim:view:folders` on the views
scope for the folders no view uses), `read_layer` reads the owned camera prims and the
folder record of a composed snapshot back, and the mapping identity (`views_schema_identity`)
is separate from the element profile's pin. A view prim carries no `bim:id`: a saved view is
display state, not model data, so it enters no element index and no model key or number
can move. The W01 provenance rides on the prim as `bim:view:author`, `bim:view:savedAtMs`,
`bim:view:comment` and `bim:view:modelRevision`, all four or none, with the same bounds as
the record above.

The restore path: the session file of a store is now the views layer itself
(`beamcraft_session::views_layer` maps the book to the layer and back, through the reader's
manifest commitment). The V12 commands keep their surface: `--file` stays the caller-chosen
path, a save writes the layer form, and a legacy JSON sidecar (versions 1 and 2) still
loads until the next save rewrites it in the layer form. The window's sidecar is the
sibling `<store>.views.usda`, with the legacy `<store>.views.json` still loading beside a
layer. Restoring a view restores its camera and its provenance; the store head stays
byte-identical across every view operation, so model keys and numbering never move.
Whether the layer additionally becomes a manifest layer of the store revision stays with
the owner question above.

