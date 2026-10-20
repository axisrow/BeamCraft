# BeamCraft UI

V03: the native window and viewport. It shows the accepted (published) revision of a store.

```text
beamcraft-ui --store ROOT [--cli PATH]
beamcraft-ui --workspace DIR [--cli PATH]
beamcraft-ui --store PUBLISHED --workspace DIR [--cli PATH]      (publication mode, V06)
beamcraft-ui --store ROOT --metrics FRAMES [--width W] [--height H] [--cli PATH]
beamcraft-ui --synthetic COUNT --metrics FRAMES [--width W] [--height H]
```

- The model comes from the engine, through the headless CLI in a separate process:
  `beamcraft view-export --store ROOT --out -` writes the pinned view snapshot as one bundle, and
  `beamcraft_engine::ViewSnapshot::from_bundle` rebuilds and re-validates it here. The UI never
  opens USD and never depends on the CLI crate (F08: an app may not depend on an app). The CLI is
  looked up next to the UI executable unless `--cli` names it.
- Each distinct mesh is uploaded once and drawn with one instance per element placement.
  Coordinates are millimetres, re-centred on the engineering bounds in f64 before f32 conversion.
- Controls: primary drag orbits, Shift + primary drag pans (as does secondary drag), the wheel or
  trackpad scroll zooms and so does a two-finger pinch on a trackpad, `+` / `-` zoom from the
  keyboard, F fits and R reloads (F, R, `+` and `-` are ignored while a text field has the
  keyboard). The "Invert zoom" toggle in the panel flips the scroll zoom direction, for macOS
  natural scrolling; pinch and `+`/`-` keep their direction. A click selects, Esc clears the
  selection.
- V05.1 Create panel (experimental): a plate (width, length, thickness, optional name and material)
  or a straight beam/column (start, end, profile and material chosen from the store's definitions,
  optional name). Preview stages the exact `create-plate` / `create-member` argument list (one per line, not a shell
  command line; no geometry preview), Commit runs it in a separate process and reloads the accepted revision,
  Cancel drops it and publishes nothing; a CLI refusal is shown with its code and message. The
  parameter mapping and reply parsing are `tools.rs` (pure, unit-tested); the process is
  `command.rs`. See [the design note](../../docs/design/v05-1-ui-create-tools.md).
- V05.2 Edit panel (experimental): a click selects through the engine's `pick` query (the
  selection is drawn as the projected box of the element's world bounds), and Move (absolute
  position), Copy (offset) and Delete are staged and committed like the create tools
  (`update-element` / `copy-element` / `delete-element`); Undo and Redo (`workspace-undo` /
  `workspace-redo`) need `--workspace DIR`. See [the design note](../../docs/design/v05-2-ui-edit.md).
- V06 Properties panel (experimental, on the V05.2 selection): shows the selected element's full
  report from `plate-inspect` — name, kind, dimensions, profile and material (with the store's
  display names), position, and volume and mass with their statuses. Beneath it, Edit stages a
  typed property patch: name, material (keep / none / a definition), thickness (plates); empty
  fields change nothing, the position itself is the edit panel's Move action. Preview shows the
  exact `update-element` argument list, Commit runs it like a create tool and reloads, Cancel
  publishes nothing; a refresh that no longer finds the element says so. The mapping is `props.rs`
  (pure, unit-tested). See [the design note](../../docs/design/v06-ui-properties-publication.md).
- V06 Workspace panel (experimental, publication mode `--store PUBLISHED --workspace DIR`): the
  window still edits the personal draft in DIR (the model shown is the draft head), and the panel
  reports the draft and published heads, unsent revisions and whether the published head is still
  the base (`workspace-status` + `workspace-history`), and runs Publish (`workspace-publish`,
  fast-forward only; a moved published head is `BASE_REVISION_MISMATCH` and no merge exists).
  With `--workspace DIR` alone the window is exactly the V05.2 one.
- `--metrics` renders offscreen without a window and prints one JSON object (adapter, scene size,
  upload, per-frame median and p95). `--synthetic` measures synthetic kernel plates for scale.
- The UI font is the admitted Inter, read at run time from `assets/fonts/inter`; without it the
  window runs without text. `default_fonts` is never enabled.

Linux needs a Vulkan driver and the X11 or Wayland client libraries (including
`libxkbcommon-x11`) at run time; a missing window-system library is reported, not a crash.
