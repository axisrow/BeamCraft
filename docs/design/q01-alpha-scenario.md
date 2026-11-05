# Q01 alpha scenario

One documented command runs the whole alpha story — the original five-part detailing story and
its expanded story — on Windows, Linux or macOS:

```sh
cargo nextest run --locked -p beamcraft-cli --test cli alpha_scenario
```

The `alpha_scenario` substring filter selects both stories of the package's `cli` integration
target: `alpha_scenario.rs` (the original) and `alpha_scenario_expanded.rs` (the expanded one,
which includes the original frame as its opening and runs the curved/poly members, the
bent/lofted plates, the rotate/mirror/array tool story, the bolt-group story, the model-data
tool story, the contour plate story, the part class story, the cut lifecycle story, the
fits-and-combine story, the hole-record story, the storey assignment story and the renumbering
story each in their
own published store as its further tests). The
expected exit is 0. On success the
stories themselves are silent; the runner prints one result line per test. A failing step
aborts its story with the exact command and its expected and actual JSON in the assert panic
message. If cargo-nextest is not installed, the libtest
equivalent on a clean checkout is:

```sh
cargo test --locked -p beamcraft-cli --test cli -- alpha_scenario
```

The same substring runs both stories there too. Setting `BEAMCRAFT_Q01_KEEP=1` (PowerShell:
`$env:BEAMCRAFT_Q01_KEEP = '1'`) keeps both stories' stores, workspaces, exports and drawing
documents for inspection, and each story then prints its absolute artifact directory path on
stderr when it ends (`Q01 artifacts:` for the original story, `Q01 expanded artifacts:` for
the expanded one). A runner captures a passing test's output, which would hide those paths,
so run the nextest command with `--no-capture`, or the libtest form as
`cargo test --locked -p beamcraft-cli --test cli -- --nocapture alpha_scenario` (the flag
goes after the `--`, before the filter; a second `--` would leave capture on),
whenever KEEP is set. This invocation
is the acceptance run of the two stories, not the recorded test gate: the gate
(`tools/test-gate`) remains the workspace-wide evidence behind a PR. Every item of the expanded
acceptance is traced to its executable check or recorded manual result in
[q01-alpha-matrix.md](q01-alpha-matrix.md).

## The original five-part story

Its test runs the real CLI in a new process for every step, against a fresh published store and
personal workspace. Its invented dimensions and expected values are contributor-original test
data under MPL-2.0, covered by `ATTRIBUTION.md`. No commercial section catalogue is used.

The story defines a 100 by 50 mm rectangular section and steel density of 7850 kg/m³, forks a
workspace, creates two 1000 mm columns and a 1000 mm top beam cut to 950 mm, and creates two
200 by 200 by 10 mm base plates with 20 by 20 mm rectangular holes. It groups all five parts
in an assembly, adds a two-bolt group and a workshop weld, previews and approves numbering,
publishes, exports the BOM and experimental IFC, and reopens the accepted result.

Assertions cover each command's exit status and bounded JSON, five independent closed-form
volumes, total mass of 122.0047 kg, draft isolation, refusal without a revision, six approved
marks (five parts and their assembly), publication status, reopened meshes and quantities,
deterministic BOM bytes with exactly five unique parts, IFC entity counts, and publication retry
without another revision. Bolts and welds are records; the current bolt slice does not generate
holes automatically. The holes in this story are explicitly authored plate holes.

## Published drawing slice

The same entry point also creates and publishes a drawing of the first column with semantic
anchors. It saves `drawing-version.json`, reopens the document and its archived model source,
and checks exact publication replies and the single retained version. A subsequent draft rename
is isolated until publication; after publication the drawing reports `stale` and still refers to
the original source. Retrying its publication returns the original version bytes.

This exercises the existing minimal D03 slice only. It does not cover drawing sheets, broader
views, PDF/print, or drawing native UI. The retained final model has the same five parts with the
first column renamed. The original BOM remains the output
of the earlier numbered source revision.

## Native UI runbook

Keep the generated stores and output files for the manual portion (the header's entry point
with `--no-capture`, so both artifact paths print):

```sh
BEAMCRAFT_Q01_KEEP=1 cargo nextest run --locked -p beamcraft-cli --test cli --no-capture alpha_scenario
```

In PowerShell, set `$env:BEAMCRAFT_Q01_KEEP = '1'` first. The run prints two absolute artifact
directories. Set `ROOT` to the `Q01 artifacts:` path of the original story and `ROOTX` to the
`Q01 expanded artifacts:` path of the expanded one. The four steps below run on the original
artifacts; the extended cells run on the expanded ones, which carry the grid and level their
steps need:

```sh
cargo run --locked -p beamcraft-ui -- --workspace "$ROOT/draft" --published "$ROOT/published"
```

1. Confirm five visible parts, orbit and zoom, and inspect the two columns, the cut beam and the
   two holed plates. Record OS, GPU/backend, commit and any failed selection or unreadable field.
2. In Create, select Member, section `Q01-RECT`, material `Q01-steel`, start `0,500,0` and end
   `1000,500,0`. Preview, then Cancel. Reload: there must still be five parts and no new draft.
3. Repeat Preview and Commit. Confirm a sixth visible part, its coordinates, volume
   5,000,000 mm³ and mass 39.25 kg, and one unsent draft revision in Publish. Record stale
   status as a failure, even when a restart repairs it (fork issue #64).
4. Undo and confirm the five-part scene returns. Reopen the same workspace and confirm it
   persists. Do not infer a pass from a screenshot or from the automated CLI story alone.

The UI portion is a manual runbook, not an automated or completed OS/GPU acceptance claim.
Clean up only the printed artifact directory when finished.

## Extended native UI cells

The cells below extend the manual portion to the newer UI surfaces: the selection switches
(V11), the render modes (V16), the cut plane (V15), named views and view panes (V12), the
colour sets (V17) and the sheet editor (D05). They run on the expanded artifacts, which carry
the grid and level the steps need. Every cell stays open until the owner runs it and accepts
each visual result by eye. Observed results are recorded separately from these instructions —
in the acceptance traceability of [q01-alpha-matrix.md](q01-alpha-matrix.md) and on upstream
issue #334 — never inside this file; the per-cell checklists are filled in that record. All
labels, shortcuts and limits are read from the source; a discrepancy against the running app
is a recorded failure, not a reason to improvise.

```sh
cargo run --locked -p beamcraft-ui -- --workspace "$ROOTX/draft" --published "$ROOTX/published"
```

Record the environment first: OS and version, the GPU/backend line of the `Details` window
(bottom bar → `Details`), the commit and the date. Keep `Details` open — its `Revision …` and
`Elements:` lines are the "model unchanged" evidence of every cell. The draft the cells open
holds ten parts: the seven of the numbered frame (two members named `column`, the beam
trimmed to 950 mm, `Split column`, two `Base plate` plates, one copied part) plus the three
slice-D members `Q01-D oblique hole beam` and `Q01-D tube hole beam` (both along x, origin
at 0,0,0) and `Q01-D sloped wall beam` (along y), which the story appends to the draft after
the published store was forked — only `$ROOTX/published` keeps the seven-part state. The
draft also carries the grid `Q01 grid` (U lines at x = 0 and x = 1000, a
V line at y = 0), the level `Q01 level` at elevation 1000, the assembly `Q01 expanded frame`
and a weld record. Write the exact `Elements:` count down; the cells use it as their
baseline. The colour-set and shortcut files live in the per-user configuration directory
(macOS: `~/Library/Application Support/BeamCraft`); to keep the real configuration clean,
launch with `BEAMCRAFT_CONFIG_DIR=/tmp/q01-config` and put the colour-set file of cell V17
there. Recommended cell order: V11 → V16 → V15 → V12 → V17 → D05; each cell ends by clearing
its own display state. A crash, refusal or mismatch is recorded verbatim and stops its cell —
a crash is never "try again and it worked".

### V11 — selection switches and snap panel (open until accepted)

**Precondition.** The UI is open on `$ROOTX/draft`; the ten draft parts are visible; `Details` shows
the baseline revision and element count; the bottom bar shows the entity strip on the left
and the snap strip on the right (each button names itself on hover).

**Steps.**

1. Click a column in the viewport. Write down the status-bar text (it must change from
   `Nothing selected` to `1 selected · <kind> <name>`) and that the right inspector switches
   to `Properties` with the element's fields.
2. Turn **off** `Select parts` (the button loses its highlight). Click the same column:
   `Nothing selected` is expected. Turn the switch back on and click again — selection must
   work again.
3. Turn **off** `Select grids`, click a grid line of `Q01 grid`: `Nothing selected` is
   expected. Turn the switch back on and click the line again — the status bar must show the
   grid selected and the inspector switches to grid properties. Write the exact text down.
4. With `Select parts` on, click a part once more and confirm selection still works with
   `Select assemblies`, `Select bolts`, `Select welds`, `Select holes` and `Select items`
   off. Recorded fact, not a failure: bolts, welds, holes and items have no pickable
   geometry in the model yet, so those switches currently select nothing extra. Write down
   that this matches the app.
5. Snap panel: toggle the master `Snap` switch (far right), then `Snap grid intersections`,
   `Snap endpoints`, `Snap midpoints`, `Orthogonal constraint` and `Snap nearest` one by
   one. After each toggle write down only that the highlight followed the click and the
   scene did not change. Leave every snap switch on.
6. Press `R` (reload). The same ten parts, the same `Revision …` prefix and the same
   `Elements:` count as the baseline.

**Checklist** (fill the recorded results in the separate record, not here).

- [ ] Step 1: a click selects exactly one part, status text recorded — pass / fail
- [ ] Step 2: the parts switch gates part picking and nothing else — pass / fail
- [ ] Step 3: the grids switch gates grid picking — pass / fail
- [ ] Step 4: the other switches do not interfere; the reserved-switch fact matches — pass / fail
- [ ] Step 5: the snap toggles are display state only — pass / fail
- [ ] Step 6: reload proves no switch touched the model — pass / fail
- [ ] Owner accepted (visual): yes / no — date

### V16 — five render modes on shortcuts (open until accepted)

**Precondition.** Nothing selected, no cut, no colour overlay. Click once on empty viewport
space so no text field has focus: the shortcuts are ignored while a text field owns the
keyboard.

**Steps.** After each press write down the `Details` line `Render mode: <token>` and one
line on what changed on screen.

1. `Ctrl+1` — `wireframe`: parts render as edges, no filled surfaces.
2. `Ctrl+2` — `transparent`: parts become see-through.
3. `Ctrl+3` — `grey`: material colours disappear, everything uniform grey.
4. `Ctrl+4` — `shaded`: the default appearance returns.
5. Click a column, then `Ctrl+5` — `selected`: only the selected column remains visible.
   `Ctrl+4` back, then clear the selection (record how).
6. Ribbon cross-check: tab `View` → group `Display` → `Grey`. The button shows its active
   state and `Details` flips to `grey` without the keyboard.
7. Write down which mode the app started in (expected `shaded`).

**Checklist.**

- [ ] Steps 1–4: each shortcut switches the mode; token and visuals match — pass / fail
- [ ] Step 5: `selected` shows only the selection — pass / fail
- [ ] Step 6: the ribbon button and the shortcut drive one state — pass / fail
- [ ] Step 7: start mode recorded — pass / fail
- [ ] Owner accepted (visual): yes / no — date

### V15 — view cut plane (open until accepted)

**Precondition.** The ten draft parts visible, `Shaded` mode, no cut applied. The model on
disk must not change in this cell.

**Steps.**

1. Tab `View` → group `Cut` → `Cut plane`. The inspector switches to `Cut plane` and reads
   "The cut plane hides part of the model in this window only…". Write down the
   `No cut: the whole model is visible.` line.
2. Expand `Cut plane`, keep `XY (cut below)`, set `Model axis offset (mm)` to `100`, click
   `Apply cut`. Write down the confirmation (expected `Cut applied: 1 user plane(s),
   work-area box off, 1 effective half-space(s).`). On screen everything below z = 100 mm is
   cut away; write down what remains.
3. Offset `950`, apply again: the cut moves up to the beam. Write the change down.
4. Mode `Point and normal`, point `500,0,1000`, normal `0,0,1`, apply: the visible
   half-space faces +Z at beam level. Write down what remains visible.
5. Switch the mode back to `XY (cut below)` — the offset field still holds `100` — tick
   `Work-area box`, set `Min X,Y,Z (mm)` to `-500,-500,-100` and `Max X,Y,Z (mm)` to
   `600,500,1100`, apply. Write down the confirmation (expected `Cut applied: 1 user
   plane(s), work-area box on, 7 effective half-space(s).` — with the box on, its six faces
   join the user plane). On screen only what lies inside the box and at or above z = 100 mm
   remains: the left column above the cut and the part of the beams inside the box; the
   plates and the low slice-D beams disappear.
6. `Remove cut`: `Cut removed: the whole model is visible.`, all ten parts back.
7. Press `R`; `Details` still shows the baseline revision and element count.

**Checklist.**

- [ ] Step 2: the XY cut applies with the exact confirmation text — pass / fail
- [ ] Step 3: moving the offset moves the cut — pass / fail
- [ ] Step 4: point and normal honour the typed plane — pass / fail
- [ ] Step 5: the work-area box combines with the plane — pass / fail
- [ ] Steps 6–7: removal restores everything; model unchanged — pass / fail
- [ ] Owner accepted (visual): yes / no — date

### V12 — named views, view panes, projection, arrangement (open until accepted)

**Precondition.** No cut applied (V15 cleaned up). The sidecar `$ROOTX/draft.views.json`
must not exist yet; if it does, write that down — the cell then starts with views present.

**Steps.**

1. Save the current camera: with the free 3D pane active, the inspector shows the free-view
   panel. Type `Home` into `Name` and click `Save as view` (tooltip: "Keep the camera as it
   is now as a saved view"). Write down where the view appears (project browser, folder and
   name).
2. Orbit far away and zoom. Open `Home` from the project browser — the inspector shows the
   view's form. Click `Apply`: the pane returns to the saved pose. Write down how you
   verified the return (visually and through the form's camera values).
3. Projection: in the same form set `Projection` to `Orthographic`, `Apply` — the pane
   visibly flattens. Back to `Perspective`, `Apply`. Write down both looks and the form
   values.
4. Plane views: tab `View` → group `Views` → `Create view` → `Plan (XY plane)`. The
   inspector opens the new-view form; type a name, keep the defaults, `Apply` — a top-down
   pane appears. Repeat with `Elevation (XZ plane)` for a front view. With a pane active,
   try the `Direction` menu (e.g. `Top`): the pane turns to that direction. Write the pane
   titles down and that `Details` counts never changed.
5. Windows: group `Windows` → `New view window` opens an extra pane. Orbit in it; the other
   panes must not move. Keep opening panes until the button disables and write the maximum
   count down. Then `Arrange`: try `Tile horizontal`, `Tile vertical`, `Cascade`,
   `Maximize / restore` and write down what each does.
6. Close the extra panes. `Details` still shows the baseline revision and element count.
7. Persistence: quit the app; the sibling file `$ROOTX/draft.views.json` must exist.
   Relaunch; `Home` and the plane views are in the project browser again; open `Home` once
   more. Write the list down.

**Checklist.**

- [ ] Steps 1–2: `Save as view` and `Apply` reproduce the pose — pass / fail
- [ ] Step 3: projection is a form property applied on `Apply` — pass / fail
- [ ] Step 4: plan and elevation views build via `Create view`; `Direction` orients — pass / fail
- [ ] Step 5: extra panes are independent; `Arrange` works; the maximum is recorded — pass / fail
- [ ] Steps 6–7: sidecar exists; views survive the restart; model unchanged — pass / fail
- [ ] Owner accepted (visual): yes / no — date

The display crop (work-area box) is covered by cell V15; the view surface has no separate
work-area fields.

### V17 — colour by filter/class sets (open until accepted)

**Precondition.** The app is closed. A colour-sets file in the configuration directory
(`colour-sets.json`, or `$BEAMCRAFT_CONFIG_DIR/colour-sets.json`) with exactly this content
(the rules are the V14 rule JSON, evaluated by the same engine `query-elements` uses):

```json
{
  "version": 1,
  "sets": [
    {
      "name": "Q01 check",
      "rules": [
        { "rule": { "field": "name", "op": "eq", "value": "column" },      "colour": [220, 60, 50],  "transparency": 0.0 },
        { "rule": { "field": "name", "op": "eq", "value": "Base plate" },  "colour": [60, 120, 220], "transparency": 0.0 }
      ]
    }
  ]
}
```

Launch the UI. Baseline: the ten draft parts in material colours; `Details` revision and
counts noted.

**Steps.**

1. Tab `View` → group `Colour` → `Colour`. The inspector switches to `Colour` with a set
   combo. Write down the sets it lists: the built-in `Part classes` and `Q01 check`.
2. Pick `Part classes`, click `Show colours`. The parts recolour by class (all story parts
   carry the same class, so one palette colour is expected across them — write down which).
   The legend shows `Part classes: N element(s) coloured` and one swatch per rule. Write N
   and the swatch count down.
3. `Hide colours`: material colours return, the legend disappears.
4. Pick `Q01 check`, `Show colours`. The two `column` members turn red, the two
   `Base plate` plates turn blue, everything else keeps its material colour; the legend
   shows the two rules with their swatches. Write the legend lines and the count down.
5. With the overlay applied, orbit and select a coloured column: picking and properties
   still work.
6. `Hide colours`, press `R`, `Details` shows the baseline. Quit and read the store:

   ```sh
   cargo run --locked -p beamcraft-cli -- inspect-elements --store "$ROOTX/draft"
   ```

   The part list and volumes match the baseline: the overlay never reached the store.
7. Invalid-file regression: quit, replace the colour-sets file with `{` (one broken byte),
   relaunch, open the `Colour` inspector. A muted note must report the file was refused and
   the preset kept (wording starts `the colour-sets file is not valid…`); the combo still
   lists `Part classes` and applying it still works. Restore the good file afterwards.

**Checklist.**

- [ ] Step 1: both sets listed — pass / fail
- [ ] Step 2: the built-in class set applies with legend and count — pass / fail
- [ ] Steps 3–4: the custom set colours exactly the matching elements — pass / fail
- [ ] Step 5: the overlay does not interfere with picking — pass / fail
- [ ] Step 6: reload and the CLI read prove the store is untouched — pass / fail
- [ ] Step 7: the broken file is reported, defaults kept, nothing overwritten — pass / fail
- [ ] Owner accepted (visual): yes / no — date

### D05 — sheet editor (open until accepted)

**Precondition A — lay a sheet and a placed view on the original-story document (CLI,
executed by the owner).** The story leaves `$ROOT/drawing` published but stale (the model
moved on after publication, by design; sheet edits address the document's own head, not the
model source). Generate five fresh UUIDs (`uuidgen`); below they are `U1…U5`.

1. Read the document head:

   ```sh
   cargo run --locked -p beamcraft-cli -- inspect-drawing --document-dir "$ROOT/drawing"
   ```

   Write `revision_id` down as `HEAD` and the primary view's `origin` down.

2. Lay the sheet:

   ```sh
   cargo run --locked -p beamcraft-cli -- edit-sheet --document-dir "$ROOT/drawing" \
     --expected-head HEAD --op-id U1 --revision-id U2 --actor-id U3 \
     --sheet-size a4 --sheet-frame 10 --sheet-project "Q01" --sheet-author "<your initials>"
   ```

   `--sheet-frame` takes the frame inset in millimetres. Write the reply's new
   `revision_id` down as `HEAD2` and the sheet fields down. A refusal here is a recorded
   failure — do not work around it.

3. Place one extra view (`V` is the fourth fresh UUID; the spec is
   `id:plane:scale:x,y[:title]`):

   ```sh
   cargo run --locked -p beamcraft-cli -- edit-views --document-dir "$ROOT/drawing" \
     --expected-head HEAD2 --op-id U4 --revision-id U5 --actor-id U3 \
     --placed-view "V:top:100:120,180:Placed top"
   ```

4. Re-run `inspect-drawing`: `placed_views` holds one entry at origin `120,180`; the
   primary view's origin is unchanged.

*Shortcut.* The expanded document `$ROOTX/drawing` already carries a sheet (a3, 10 mm
frame, title block `Q01 alpha frame` / `Q01 expanded story`) and placed, section and detail
views. Load it directly and skip the CLI steps; the UI steps apply unchanged (more markers,
same gestures; record the actual sheet size line).

**Precondition B.** Relaunch the UI (any store — the editor reads the document by path).
Baseline evidence: `Details` counts.

**Steps.**

1. Inspector → `Sheet editor` collapsing header → `Open sheet editor`. Paste the document
   directory and click `Load`.
2. Write down the header line (document name, sheet size, frame inset) and the paper
   preview: sheet, frame, title block (project and author fields), one marker per view —
   the primary view and every placed view.
3. Alignment snap: drag a placed marker so its X lands within half a millimetre of another
   view's X axis, and drop — it snaps exactly onto that axis (verify numerically in step
   5). Write down what you saw.
4. Frame constraint: drag a placed marker toward the sheet edge — it stops at the frame,
   never outside it, on a 0.1 mm grid. The **primary** marker must not move at all (the
   primary view is placed by the document, not here). Write both down.
5. `Apply placements`, wait for the note. Re-run the `inspect-drawing` command and write
   the moved origin down: it equals the dropped position (snapped and constrained). The
   primary view's origin is unchanged.
6. Move a marker again, click `Reset`: it jumps back to the last applied position.
7. Quit the UI; the `inspect-elements` baseline still matches — the sheet edits live in the
   document, never in the model.

**Checklist.**

- [ ] Precondition A (or the shortcut): sheet set, placed views recorded — pass / fail
- [ ] Step 2: paper, frame, title block and markers render — pass / fail
- [ ] Step 3: dropping near another view's axis snaps exactly — pass / fail
- [ ] Step 4: frame constraint and 0.1 mm grid hold; the primary marker is not draggable — pass / fail
- [ ] Step 5: `Apply placements` writes the moved origin; primary unchanged — pass / fail
- [ ] Step 6: `Reset` discards unapplied moves — pass / fail
- [ ] Step 7: model store untouched — pass / fail
- [ ] Owner accepted (visual): yes / no — date

The extended cells are part of the same manual runbook: they are not an executed OS/GPU
acceptance claim, and no cell is accepted until the owner says so.

## Acceptance boundary

This implements the original fork issue #9 story and the frame/BOM story of upstream #123.
It does not declare gate K7 passed. The expanded Q01 plan now also depends on modelling,
views/filters, attributes/schedules and drawings (K9–K11 and K8); those packages retain their
own acceptance and are not replaced by this test. IFC export remains experimental and outside
the current MVP; entity counts are not independent IFC schema or geometry certification.
