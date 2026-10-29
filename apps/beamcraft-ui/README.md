# BeamCraft UI

V03: the native window and viewport. It shows the accepted (published) revision of a store.

```text
beamcraft-ui --store ROOT [--cli PATH]
beamcraft-ui --store ROOT --metrics FRAMES [--width W] [--height H] [--samples 1|4] [--cli PATH]
beamcraft-ui --synthetic COUNT --metrics FRAMES [--width W] [--height H] [--samples 1|4]
beamcraft-ui --store ROOT --perf-run PLAN [--samples 1|4] [--cli PATH]
```

- The model comes from the engine, through the headless CLI in a separate process:
  `beamcraft export-view --store ROOT --out - --with-definitions true` writes the pinned view
  snapshot as one bundle, framed after the store's definitions from the same verified inspection,
  and `beamcraft_engine::ViewSnapshot::from_bundle` rebuilds and re-validates it here. One refresh
  is one process and one inspection; only when the export fails is the definitions listing asked on
  its own. The UI never opens USD and never depends on the CLI crate (F08:
  an app may not depend on an app). The CLI is looked up next to the UI executable unless `--cli`
  names it.
- Each distinct mesh is uploaded once and drawn with one instance per element placement.
  Attached read-only references in the [R01 translation-only display profile](../../docs/design/r01-readonly-reference.md#translation-only-viewport-path-issue-137)
  use the same path and contribute to camera fit; unsupported foreign transforms are explicit
  export-view refusals, not invisible or incorrectly placed geometry.
  Coordinates are millimetres, re-centred on the engineering bounds in f64 before f32 conversion.
  Once a scene is on the device the window keeps only its origin, framing and counts, not the
  expanded vertex buffers. A refresh of an unchanged snapshot builds, uploads and draws nothing,
  and meshes the device already holds (same digest) are not expanded or sent again; uploads never
  wait for the device on the UI thread.
- The window draws the 3D image every frame. A resize rebuilds the targets, never the shader or
  the pipeline. MSAA is 4x on a hardware adapter that supports it and 1x on a software (CPU)
  adapter; `--samples 1|4` overrides it.
- Controls, `Mouse` preset (the default): the left button is reserved for selection and never
  moves the camera, middle-button drag pans, Ctrl + middle-button drag orbits, the wheel zooms
  toward the cursor. `Trackpad (macOS)` preset (Details window, from the status bar): primary drag orbits, Shift +
  primary drag or secondary drag pans, the wheel or a two-finger scroll or pinch zooms toward
  the cursor (the "Invert zoom" toggle flips the scroll direction for macOS natural scrolling;
  pinch and keys keep theirs). Both presets: `+` and `-` zoom from the keyboard, F fits, R
  reloads (`+`, `-`, F and R are ignored while a text field has the keyboard). A click selects,
  Esc clears the selection.
  Orbit drags move the front of the model with the pointer in both presets, rather than
  moving the viewpoint with the pointer.
- Live refresh: the window watches the store's committed revisions and reloads by itself when
  another process (the CLI, an agent) commits to the store it shows; `R` stays for a manual
  reload. A reload whose framing changed fits the camera, so new content comes into view.
- Native interface (egui/wgpu, entirely Rust, no HTML, JavaScript or WebView): cool neutral
  surfaces, one-point separators, compact Inter controls and cyan actions/selection replace the
  old cream panels, heavy outlines, decorative grids and hard shadows. The model library has a
  navigation sidebar, a compact model table and selected-model details with an icon-labeled Open
  action. Recent models has no duplicate header Open/New actions or search. Search (Ctrl+K)
  is available only in All models and never filters Recent models.
  Double-click or Enter opens a model. Selected models get a real geometry thumbnail
  from the CLI snapshot on a single background worker, cached for up to 20 models.
  Thumbnails have an 8,192-triangle budget; failures and larger models show an explicit message
  and Retry, not a fabricated or incomplete preview. Uninspected rows retain generic symbols.
- The model workspace has a custom draggable title bar, native window controls, a project/save-state
  header and Publish for personal workspaces; Menu, Steel, Edit, View
  and Manage ribbon tabs; resizable project-browser and inspector panes; the native 3D viewport;
  and the compact selection/snap toolbar and status line. Views are listed in the browser.
  The viewport has no floating view tabs, zoom/fit buttons, orientation triad or display-mode
  caption; mouse/wheel navigation and keyboard shortcuts remain available.
  The browser reports the loaded engine
  snapshot, not invented drawings or views. Open in Menu or Model library in Manage returns to the library
  when launched from the start page. Ctrl+K searches supported commands in a native dialog.
  Existing create/edit/property/publish commands still use the same CLI paths. Unimplemented
  tools (Bolts, Weld and floor-plan views) remain explicitly disabled.
- Manage > Rectangular grid restores regular-grid creation in the inspector: U/V coordinate
  expressions, labels, optional line limits, XY origin and rotation. Create invokes the existing
  CLI; Cancel does not write. The pinned snapshot includes grid axes for display and framing.
  Upstream grid selection, properties, local-line editing, intersection snapping and deletion
  remain available in the inspector. Grid guides can be toggled from View.
- View retains all five render modes (Wireframe, Transparent, Grey, Shaded, Selected only)
  and the upstream remappable Ctrl+1 through Ctrl+5 shortcuts. Mode changes never reload the
  model. See [render modes](../../docs/design/v16-render-modes.md).
- Resizable windows use equal 40-by-28-point title-bar buttons with centered, thin line icons,
  a distinct restore glyph when maximized, neutral hover feedback and a red close hover.
  The compact welcome window retains equal 28-point minimize/close buttons without maximize.
- The property inspector pins Apply and Reset at the bottom. Apply validates and submits through
  the existing CLI property command; Reset restores the last inspected values without writing.
  Reload re-reads current properties. The optional Command preview section shows exact arguments.
- Theme is chosen on the welcome screen, not repeated in the library or model title bars:
  **System** (default), **Light**, **Dark**. System follows live OS theme changes. The preference
  persists in the user's settings through `beamcraft set-settings --theme system|light|dark`;
  `--config-dir` also scopes appearance when supplied to the start page. Load/save errors are
  visible with Retry. `BEAMCRAFT_THEME=light|dark` is a non-persistent override; the selector is
  disabled while it is active.
- The welcome window uses a compact content-sized layout and equal minimize/close controls,
  without maximize or a subtitle. Theme changes discard a mixed-style frame so backgrounds,
  inputs and dropdowns switch together without color animation; a fresh launch restores the
  saved preference automatically.
- New model uses the CLI's `suggest-project-name` and numbered default names (`New Model`,
  `New Model 1`, etc.). Suggestions run off-thread and never overwrite a typed name; the
  CLI's final exclusive-create check remains authoritative.
- The native Menu follows the prototype's dark 232-point navigation, section headings and
  flat 42-point command rows. New/Open use the library, catalogs inspect actual model
  definitions, and unavailable settings/import/export/editor/maintenance
  actions stay disabled instead of suggesting unsupported functionality.
- The bottom toolbar has 20 compact 24-point entity controls and 13 snap/constraint controls,
  in separate dense rows on the left and right with small group separators. Glyphs are
  contributor-original Rust drawings. The eight upstream selection-class filters and six
  engine snap kinds are wired to these controls; Snap toggles snapping globally.
  Unsupported filters/constraints remain disabled with explanatory tooltips.
  Search stays available without displaying its keyboard shortcut.
- V05.1 Create panel (experimental, now the inspector of Plate, Beam and Column): a plate (width, length, thickness, optional name and material)
  or a straight beam/column (start, end, profile and material chosen from the store's definitions,
  optional name). Preview stages the exact `create-plate` / `create-member` argument list (one per line, not a shell
  command line; no geometry preview), Commit runs it in a separate process and reloads the accepted revision,
  Cancel drops it and publishes nothing; a CLI refusal is shown with its code and message. The
  parameter mapping and reply parsing are `tools.rs` (pure, unit-tested); the process is
  `command.rs`. See [the design note](../../docs/design/v05-1-ui-create-tools.md).
- `--metrics` renders offscreen without a window and prints one JSON object (adapter, scene size,
  upload, per-frame median and p95). `--synthetic` measures synthetic kernel plates for scale.
- `--perf-run PLAN` opens the window, runs a scripted measurement in it and prints one JSON report:
  `PLAN` is a comma-separated list of `load`, `idle:S`, `ui:K` (frames without a viewport change),
  `orbit:K`, `reload:K`, `select:K` (clicks onto box elements), `cancel:K` (a click, Esc while its
  lookup runs, another click), `edit:K` (creates K plates in the store through the Create panel's
  command path) and `resize:K`. Each phase reports frames, 3D draws, submits, uploads and bytes,
  pipeline and target builds, CLI processes (started, killed) and their time, selection errors,
  refresh timings and, on Linux, the CPU time of the process and of its CLI children and the
  resident set size; the exit status is 1 when a phase failed. See `src/perf.rs` and
  [the issue 26 design note](../../docs/design/issue-26-viewport-performance.md).
- The UI font is the admitted Inter, read at run time from `assets/fonts/inter`; without it the
  window runs without text. `default_fonts` is never enabled.

Linux needs a Vulkan driver and the X11 or Wayland client libraries (including
`libxkbcommon-x11`) at run time; a missing window-system library is reported, not a crash.
