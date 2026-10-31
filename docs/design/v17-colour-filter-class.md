# V17 colour by filter and class

Saved representation sets and a temporary colouring overlay in the model window. The V14
evaluator (`beamcraft_engine::filter`, answered headlessly by `query-elements`) stays the only
rule semantics; this slice adds the display layer over it.

## Result

The View tab gains a Colour tool. Its inspector offers the saved representation sets: the
built-in **Part classes** preset (one opaque rule per `DEFAULT_CLASS_PALETTE` entry, classes
0 to 7, P15) plus the user's own sets. *Show colours* evaluates the picked set against the
store head and colours every matching element; *Hide colours* clears the overlay. A legend
lists the applied set's rules with their colour swatch, field/operator/value line and
transparency. Colours multiply the shaded base colour (also in wireframe, which shares the
fragment entry); a rule's transparency scales the transparent mode's opacity. Grey mode stays
neutral by design, and the selected-only draw and the ghost preview are never tinted.

## The overlay is never stored in the model

The overlay is a map from element id to tint held in window state (`views::Overlay`). It is
produced by read-only `query-elements` runs, applied when a scene is built from the engine's
pinned `ViewSnapshot` (`Scene::from_snapshot_reusing` writes the tint into the per-instance
bytes), and cleared with the window. No snapshot, store, revision or model command carries it:
the scene build reads the overlay and nothing else, the snapshot token and every mesh digest
are untouched, and nothing is written to the store. Tests prove the coloured scene differs
from the plain one in the tint words only, and a GPU readback test proves the tint reaches
the drawn pixels while a white tint leaves them byte-equal in behaviour.

## A saved set lives in the views layer

`colour-sets.json` in the same per-user configuration directory as `shortcuts.json`
(`BEAMCRAFT_CONFIG_DIR` overrides it):

```json
{"version":1,"sets":[{"name":"Bold plates","rules":[
  {"rule":{"field":"class","op":"eq","value":2},"colour":[225,157,65],"transparency":0.25}]}]}
```

A set is an ordered list of rules; the first matching rule wins. A rule is the V14 rule JSON,
handed to the CLI unchanged; colours are RGB bytes; transparency runs 0.0 (opaque) to 1.0.
The file is bounded (64 KiB, 16 sets, 32 rules per set, names unique and at most 64 bytes) and
strictly versioned; a missing file is a normal first run, a damaged one keeps the preset and
is reported, never overwritten — the file is the configuration surface, as with shortcuts.
The name `Part classes` is reserved by the preset. V12 delivered the named-views layer (the
`ViewBook` and its per-store sidecar) while this slice was in flight; the sets stay personal
per-user display state for now, and moving them into the view book is a later schema
migration of one small file, not a model change.

## Evaluation

Applying a set runs one paged `query-elements` per rule (page 1024, at most 65 536 ids per
rule, the engine's element cap) on a worker thread, each rule JSON handed over as a temporary
file that is removed again. A refused rule — an unknown attribute, an incompatible literal —
fails the whole set with the CLI's stable code, so an overlay is never a silent partial match.
An empty match is reported and colours nothing. Switching sets or hiding colours cancels the
queries in flight.

## Limits

- Only the loaded model window is coloured; the start-screen library preview is not.
- Transparency applies where the pipeline blends (transparent mode); opaque pipelines ignore
  alpha, and wireframe lines have no depth offset (unchanged from V16).
- Rule authoring is the file; an in-app editor is a later slice.
