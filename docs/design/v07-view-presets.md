# V07 discipline view presets

A discipline view preset is a named bundle of view display state that composes existing
primitives only — it introduces no new semantics of its own. Defined, listed, applied and
removed through the engine API (`beamcraft_engine::presets`, the `PresetBook`) and the CLI
(`views define-preset`, `list-presets`, `apply-preset`, `remove-preset`).

## The bundle and its component mapping

| Card component | Primitive it composes |
|---|---|
| Reference visibility | the `KindFilter` kind `reference` (the same filter the view already carries) |
| Kind filter | `KindFilter` (V12 view field, V14 stays the rule evaluator for colour sets) |
| Underlay | the view's depth slab `depth [above, below]` — the existing distance-below-the-plane primitive of a plan or section view; `none` shows everything |
| Representation level | `Representation` (U12's view selector, `full`/`simplified`) |
| Colours | the *name* of a saved V17 colour set, resolved at display time against the user's `colour-sets.json` |
| Enabled options | the display toggles a view carries: grid guides (`grids_visible`) |

`ViewPreset::apply_to` writes exactly these five fields onto a `ModelView` and nothing else:
the camera, span, rotation, projection, render mode, colour, opacity, work area, folder,
favourite and the W01 provenance stay. A preset holds no camera, no provenance and no model
revision, so applying one can never touch a store: the acceptance ("switching never changes
the model or the snapshot token") is proven by the CLI e2e test, which compares the whole
`open-revision` reply (revision id and manifest digest) byte-for-byte across every preset
operation, and by the engine test that feeds a view's representation and kinds into
`ViewSettings` and reads the same snapshot token back.

## Persistence: a reasoned deviation from the saved-views precedent

The card offers the saved-views precedent (the session views layer, #457) or a reasoned
choice. The choice here is **personal per-user configuration**, the `colour-sets.json` /
`shortcuts.json` precedent: a bounded, strictly versioned JSON file (version 1, at most
64 presets, at most 64 KiB) written atomically through a sibling temporary file. The reasons:

- A preset is store-independent display state. Unlike a saved view it carries no camera to
  resolve against a model's grids or levels, no W01 provenance and no model revision — there
  is nothing in it that belongs to one store's sidecar.
- One of its components is a colour set, and colour sets already live in the per-user
  configuration directory; bundling them into a per-store USD layer would split one bundle
  across two differently-scoped files.
- The W01 views layer is a closed profile whose prims are standard USD cameras; a preset is
  not a camera, and growing the profile for it would move the mapping identity that every
  existing sidecar pins, for no card value.

The CLI takes the file explicitly (`--file`), as every views command does; the window names
it `view-presets.json` in `BEAMCRAFT_CONFIG_DIR` when the switcher wave lands.

The two fields the preset needed on the view itself — `ModelView.representation` and
`ModelView.colour_set` — ride the saved-views precedent: additive optional keys in the JSON
view format (`representation`, `colour_set`, absent reads as the display default) and two
additive attributes in the W01 views layer (`bim:view:representation` token, default `full`;
`bim:view:colourSet` string), so files written by earlier builds still read. The views layer
descriptor moves with them, as does its pinned digest. `save-view` and `update-view` gained
`--representation` and `--colour-set`, so the two view fields are settable on their own.

## Deliberate limits, recorded so they are decisions and not holes

- **No update-preset.** A preset is re-defined by `remove-preset` + `define-preset`; a
  `--replace` flag is added when a real workflow asks for it.
- **The colour set name is not resolved headlessly.** `colour-sets.json` is the window's
  configuration surface; the CLI carries and bounds-checks the name only. An unknown name is
  not a refusal at define or apply time — the set may exist on the machine that displays.
- **The native-UI preset switcher is not in this card** (owner acceptance wave).
- **Depth rule aligned, not extended.** The views layer admitted strictly positive depth
  distances while the engine (the admission source of truth) admits zero — e.g. a plan view
  showing `[0, 3000]` below the plane only. The layer now mirrors the engine rule (`[0, 2e7]`
  each, positive total thickness); layers written by earlier builds still read, since their
  distances were always positive.
- `view-snapshot` keeps its own `--representation` flag; wiring a preset into one snapshot
  call is the switcher wave's decision.
