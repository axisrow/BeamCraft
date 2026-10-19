# BeamCraft UI

V03: the native window and viewport. It shows the accepted (published) revision of a store.

```text
beamcraft-ui --store ROOT [--cli PATH]
beamcraft-ui --store PUBLISHED --workspace DIR [--cli PATH]
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
- Controls: primary drag orbits, secondary drag pans, the wheel zooms, F fits, R reloads (F and R
  are ignored while a text field has the keyboard), a click picks.
- V05.1 Create panel (experimental): a plate (width, length, thickness, optional name and material)
  or a straight beam/column (start, end, profile and material chosen from the store's definitions,
  optional name). Preview stages the exact `create-plate` / `create-member` argument list (one per line, not a shell
  command line; no geometry preview), Commit runs it in a separate process and reloads the accepted revision,
  Cancel drops it and publishes nothing; a CLI refusal is shown with its code and message. The
  parameter mapping and reply parsing are `tools.rs` (pure, unit-tested); the process is
  `command.rs`. See [the design note](../../docs/design/v05-1-ui-create-tools.md).
- V06 Properties panel (experimental): a click picks through the engine's `pick` query (the same
  ray a CLI user types; clicking empty space selects nothing), and the panel shows the selected
  element's name, kind, dimensions, profile and material (with the store's display names), and
  its volume and mass with their statuses, exactly as `plate-inspect` reports them. Beneath the
  properties, Edit stages a typed patch: name, material (keep / none / a definition), thickness
  (plates) and an absolute translation; empty fields change nothing. Preview shows the exact
  `update-element` argument list, Commit runs it like a create tool and reloads, Cancel publishes
  nothing; a refresh that no longer finds the element says so. The mapping is `props.rs` (pure,
  unit-tested).
- V06 Workspace panel (experimental, with `--store PUBLISHED --workspace DIR`): the window edits
  the personal draft in DIR (the model shown is the draft head) and the panel reports the draft
  and published heads, unsent revisions and whether the published head is still the base
  (`workspace-status` + `workspace-history`). Undo, Redo and Publish run `workspace-undo`,
  `workspace-redo` and `workspace-publish` (fast-forward only; a moved published head is
  `BASE_REVISION_MISMATCH` and no merge exists) and reload the window.
- `--metrics` renders offscreen without a window and prints one JSON object (adapter, scene size,
  upload, per-frame median and p95). `--synthetic` measures synthetic kernel plates for scale.
- The UI font is the admitted Inter, read at run time from `assets/fonts/inter`; without it the
  window runs without text. `default_fonts` is never enabled.

Linux needs a Vulkan driver and the X11 or Wayland client libraries (including
`libxkbcommon-x11`) at run time; a missing window-system library is reported, not a crash.
