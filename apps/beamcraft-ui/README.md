# BeamCraft UI

V03: the native window and viewport. It shows the accepted (published) revision of a store.

```text
beamcraft-ui --store ROOT [--cli PATH]
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
  natural scrolling; pinch and `+`/`-` keep their direction.
- V05.1 Create panel (experimental): a plate (width, length, thickness, optional name and material)
  or a straight beam/column (start, end, profile and material chosen from the store's definitions,
  optional name). Preview stages the exact `create-plate` / `create-member` argument list (one per line, not a shell
  command line; no geometry preview), Commit runs it in a separate process and reloads the accepted revision,
  Cancel drops it and publishes nothing; a CLI refusal is shown with its code and message. The
  parameter mapping and reply parsing are `tools.rs` (pure, unit-tested); the process is
  `command.rs`. See [the design note](../../docs/design/v05-1-ui-create-tools.md).
- `--metrics` renders offscreen without a window and prints one JSON object (adapter, scene size,
  upload, per-frame median and p95). `--synthetic` measures synthetic kernel plates for scale.
- The UI font is the admitted Inter, read at run time from `assets/fonts/inter`; without it the
  window runs without text. `default_fonts` is never enabled.

Linux needs a Vulkan driver and the X11 or Wayland client libraries (including
`libxkbcommon-x11`) at run time; a missing window-system library is reported, not a crash.
