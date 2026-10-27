# V16 render modes

Five display styles of the model window, the V16 slice. Not completion of the parent V16 card's
review or the K10 gate.

## Result

The viewport draws the one loaded scene in five modes, switched with `Ctrl+1` to `Ctrl+5` or the
View tab's DISPLAY buttons:

| Key | Mode | Drawing |
|-----|------|---------|
| Ctrl+1 | wireframe | triangle edges only (`PolygonMode::Line`) |
| Ctrl+2 | transparent | filled faces at 0.45 opacity, depth-tested without depth writes |
| Ctrl+3 | grey | filled, lit faces in one neutral grey |
| Ctrl+4 | shaded | the default: filled, lit faces in the part colours |
| Ctrl+5 | selected only | only the selected element, nothing when none is |

The engine owns the definition: `beamcraft_engine::RenderMode` (`crates/engine/src/presentation.rs`)
is the single query the UI reads (`ALL`, `as_token`, `parse`, default `Shaded`). Switching a mode
changes only how the loaded scene is drawn: the snapshot, its token, the camera and every
engineering quantity stay untouched, and no refresh or upload runs.

## Selected only

The scene's instance buffers hold every element, so the selected element is drawn from the mesh
buffer the device already holds: `SelectedDraw::of` (`apps/beamcraft-ui/src/scene.rs`) runs one
engine query (`ViewSnapshot::element`), takes the full-representation mesh digest and builds the
one instance bytes. The window caches that draw against the selected id and scene origin, and the
cache resets when a new revision is shown. `Renderer::draw_one` draws that one mesh with one
instance through the shaded pipeline; the ghost placement preview still draws in every mode.

## Shortcuts

The bindings live in one bounded JSON file, `shortcuts.json`, in the same per-user configuration
directory the CLI keeps `settings.json` in (`BEAMCRAFT_CONFIG_DIR` overrides it; `--config-dir`
is accepted with `--store` too). Shape:

```json
{"version":1,"wireframe":"Ctrl+1","transparent":"Ctrl+2","grey":"Ctrl+3","shaded":"Ctrl+4","selected":"Ctrl+5"}
```

A binding is `Ctrl+N` or a bare `N`, with `N` a digit 1 to 9 (`egui::Key::Num1` to `Num9`). A
missing file is the defaults. A damaged file keeps the defaults and the Details window reports
why; the file is never rewritten by the app, so remapping is the user's edit. One key never
drives two modes: the loader refuses a duplicate.

## Frame-time baseline

`--perf-run` accepts a phase per mode name (`shaded:K`, `wireframe:K`, `transparent:K`,
`grey:K`, `selected:K`): K frames drawn in that mode, each with a light orbit like the `orbit`
phase. Each phase reports the standard counters, so one run gives the per-mode frame-time
baseline over the loaded scene. The baseline recorded for this PR came from a store of synthetic
plates on the development machine; it is a run on one adapter, not a hardware survey.

## Limits

- Transparent faces are not sorted back to front; overlapping transparent parts blend in draw
  order. Sorting is a later view-property concern.
- Wireframe lines have no depth offset: an edge of a front face can z-fight with its own fill at
  equal depth (the fill is not drawn in this mode, so only coincident geometry can fight).
- The Shortcuts ribbon button (Project tab) stays a disabled placeholder; the file is the
  configuration surface for now, an in-app editor is a later slice.
