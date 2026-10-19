# V06 UI element properties and publication

Date: 2026-10-19. Status: **experimental vertical slice; V06 acceptance is not claimed.**
Branch `feat/v06-properties-ui`, based on `origin/master` `11a21397`.

Goal: the native UI (`apps/beamcraft-ui`) shows the properties of a selected element and the
publication state of a personal workspace, and every change it makes is the same engine command
the CLI runs, staged and committed like the V05.1 create tools.

## What is implemented

| Area | What exists |
|---|---|
| `beamcraft_ui::props` (pure) | `Properties` and `parse_element` read one element's record out of `beamcraft plate-inspect` (name, kind, dimensions, profile and material with their display names, placement, volume and mass with their statuses); `parse_pick` reads the first hit's element id out of `beamcraft pick`; `UpdateForm` maps the typed patch to the exact `update-element` argument list (an untouched field sends no option); `Props` is the preview, cancel and commit state machine of the properties panel; `parse_workspace_state` and `parse_history` read `workspace-status` and `workspace-history` into `WorkspacePanel`; `workspace_args` / `history_args` / `pick_args` / `inspect_args` are the argument lists exactly as a user types them. No window, GPU or process, so it is unit-tested directly. |
| `beamcraft_ui::command` | Runs the queries and changes in a separate process (the V05.1 runner): `select` (one `pick`, then one `plate-inspect`), `properties`, `run_change` (`update-element`, `workspace-publish`, `workspace-undo`, `workspace-redo`), `workspace_state`, `workspace_history`. |
| `beamcraft_ui::app` | A Properties panel in the left side panel and a Workspace panel when the window edits a personal draft (`--store PUBLISHED --workspace DIR`; the model shown is the draft head). A click in the viewport (not a drag) picks: the camera ray is display logic (`Camera::pick_ray`, unit-tested as the inverse of the projection), and the selection is the engine's `pick` answer, refreshed like every other panel by the same worker reload. Property edits stage the exact `update-element` argument list and commit like the create tools; one change command runs at a time across all panels. Publish, Undo and Redo run their workspace commands directly (they take no form) and reload; a moved published head is shown before it is attempted. |

Commands used, exactly as a user types them:

```text
beamcraft pick             --store ROOT --origin X,Y,Z --direction X,Y,Z --targets face
beamcraft plate-inspect    --store ROOT
beamcraft update-element   --store ROOT --id UUID [--name N] [--material-digest D|none]
                           [--thickness MM] [--translation X,Y,Z]
beamcraft workspace-status --store PUBLISHED --workspace-dir DIR
beamcraft workspace-history --workspace-dir DIR
beamcraft workspace-publish --store PUBLISHED --workspace-dir DIR
beamcraft workspace-undo   --workspace-dir DIR
beamcraft workspace-redo   --workspace-dir DIR
```

## Decisions

1. **Selection is the engine's pick, not a UI guess.** The click only computes the ray the
   viewport looks through (`Camera::pick_ray`, the inverse of the CPU projection, unit-tested by
   round trip; the scene origin is added back so the ray is in the store's millimetres). The hit
   is the engine's `pick` answer; the properties are the engine's `plate-inspect` answer. Nothing
   about shapes, hits or features is computed or cached in the window.
2. **The properties are a report, not a form binding.** Every value shown (including volume and
   mass with their `current`/`unknown` statuses and the profile and material display names) is
   what `plate-inspect` reported, and a refresh replaces it wholesale. An element that is no
   longer in the store clears the selection and says so instead of showing stale values.
3. **An edit is a typed patch, exactly as typed.** `update-element` applies only the options it
   receives, so an untouched field sends no option: an empty name does not rename, "keep current"
   sends no material option, "none" sends `--material-digest none`, and an all-empty translation
   does not move. The staged argument list is shown one JSON-quoted argument per line (the
   create tools' display rule), Commit runs only while the form still produces it, and selecting
   another element starts from an empty form so a patch never crosses elements.
4. **One change command at a time.** The create, update, publish, undo and redo panels share one
   busy gate, so the store never sees two concurrent change commands from this window; a
   refusal (`BASE_REVISION_MISMATCH`, `OP_ID_REUSE`, ...) is shown as received. Queries
   (pick, inspect, refresh) are not gated.
5. **The workspace panel reports before it acts.** The panel shows the draft and published heads,
   unsent revisions, the undo and redo units and whether the published head is still the base;
   Undo and Redo are enabled only while a unit exists and Publish only while something is
   unsent. A moved head is stated (fast-forward only, no merge) before Publish is attempted.
   The commands take no form, so there is nothing to stage: the button is the command a user
   types.
6. **The draft is the model the window edits.** With `--workspace DIR`, every create, update and
   pick runs against the draft store and the viewport shows the draft head; the published store
   appears only as the publication target. Without the option the window behaves as before.
7. **Outcome unknown is not failure.** As in V05.1: on `OUTCOME_UNKNOWN` the panel says the
   result is unknown and reloads, so the viewport and the panels show whatever was accepted.
   Each panel keeps its own note, and a refresh problem never overwrites a command result.

## Not done (not claimed)

- No highlight of the selected element in the viewport; the panel names what is selected.
- No thickness edit for members and no level, roll, profile or hole edits: `update-element` does
  not change them (a member's thickness is its profile) and this slice does not widen the engine.
- No copy, delete, weld, assembly, level or reference UI; no geometry preview of an update; no
  multi-element selection.
- Publication covers a personal workspace's own buttons only: no changesets, coordinator or
  conflict flow (C01–C02 stay CLI-only), and no create-workspace UI (the workspace is created
  with the CLI, then opened with `--workspace`).
- The window has not been exercised by an automated GUI test; the logic under it is the tested
  `props`/`command`/`source` modules and the camera ray, and the panels only wire widgets to
  them. The click path (pick, edit, publish) has not been run on any platform in a real window
  in this slice.
- Experimental: V06 acceptance is not claimed.

## Evidence

- Unit tests in `apps/beamcraft-ui/src/props.rs` (10): a pick reply names the first hit or
  nothing (a refusal and garbage stay errors); a plate and a member record are read with their
  dimensions, names, statuses and translations; a missing element is `None` and a broken reply is
  an error; a change reply carries outcome, element and sequence, and a refusal keeps its code;
  the update form maps exactly the changed properties (rename, material set/clear/keep,
  thickness, absolute translation; negative sizes are forwarded, bad numbers are refused before
  anything runs, no change at all is refused); the update state machine commits only what was
  previewed, only for the selected element, and blocks a second commit while busy; the workspace
  status and history replies are read; the argument lists are exactly what a user types.
- Unit tests in `apps/beamcraft-ui/src/camera.rs` (1 new): a pick ray is the inverse of the
  projection (round trip over a grid of screen points) and a unit vector.
- `apps/beamcraft-ui/tests/properties.rs` (3, real `beamcraft` binary, no GPU needed):
  - pick and lookup report what the CLI reports: the panel's properties equal the engine's
    (name, kind, dimensions, volume 5 000 000 mm3, mass 39.25 kg, both `current`, the material's
    display name); a ray through the plate's world-bounds centre hits it and a ray away misses;
    a store without a revision refuses the pick with `NOT_FOUND`; a missing element is `None`;
  - an update through the form equals the same `update-element` by hand in a second store (name,
    thickness, translation, volume 10 000 000 mm3, mesh digest), the properties refresh shows
    the new values, an engine refusal (`INVALID_ARGUMENT`, thickness -5) changes nothing, and the
    staging machine refuses a stale preview;
  - the publication panel follows the real draft: a fresh fork has nothing unsent and no units,
    the panel's own update makes one unsent revision and one undo unit, publish empties the
    unsent count and equals the published head, a second publish of the unchanged draft is the
    same request and replays, undo restores the published state and redo brings the edit back
    (the published store keeps revision sequence 3 the whole time), and one whole window refresh
    reads the draft and the panel with the same result.
- The earlier V05.1 suites still pass unchanged (`create_tools.rs` was only adjusted to the
  `source::reload` signature, which grew the selection and workspace inputs).

## Proposed Roadmap entry

`2026-10-19 V06: UI element properties and publication (apps/beamcraft-ui: click-to-pick through the CLI's pick query, properties panel reading plate-inspect (name, kind, dimensions, profile, material, volume/mass with statuses), edits staged as the exact update-element argument list and committed like the create tools, and a personal workspace panel with status, undo, redo and fast-forward-only publish through workspace-status/-history/-publish/-undo/-redo; pure mapping and reply parsing in props.rs, end-to-end tests equal to the CLI by hand); experimental, no viewport highlight, GUI click path not exercised in a real window.`
