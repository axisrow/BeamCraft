# V06 UI element properties and publication

> **2026-10-31:** the CLI commands `pick` and `snap` described here were removed (interactive
> assistance needs no CLI command, `AGENTS.md` section 3). The engine queries `pick` and `snap`
> remain and serve the window; the command lines below are historical.

Date: 2026-10-19; re-based on the merged V05.2 edit tools 2026-10-20. Status: **experimental
vertical slice; V06 acceptance is not claimed.**
Branch `feat/v06-properties-ui`; the slice was first based on `origin/master` `11a21397`, then
merged forward onto the master that contains V05.2 (`a001460`), because V05.2 had landed the
same click selection this slice first built.

Goal: the native UI (`apps/beamcraft-ui`) shows the full properties of the selected element and
the publication state of a personal workspace, and every change it makes is the same engine
command the CLI runs, staged and committed like the V05.1 create tools.

## What is implemented

| Area | What exists |
|---|---|
| `beamcraft_ui::props` (pure) | `Properties` and `parse_element` read one element's full record out of `beamcraft plate-inspect` (name, kind, dimensions, profile and material with their display names, placement, volume and mass with their statuses) — the panel-shaped superset of the edit tools' `ElementInfo`; `UpdateForm` maps the typed property patch to the exact `update-element` argument list (an untouched field sends no option; the position is not in the form — the edit panel's Move owns it); `Props` is the preview, cancel and commit state machine of the properties panel; `parse_change` reads the reply of `update-element` and `workspace-publish`; `parse_workspace_state` and `parse_history` read `workspace-status` and `workspace-history` into `WorkspacePanel`; `workspace_args` / `draft_args` are the argument lists exactly as a user types them. No window, GPU or process, so it is unit-tested directly. |
| `beamcraft_ui::command` | `properties` (the full report of one element, under the edit tools' 32 MiB inspection cap), `run_change`, `workspace_state`, `workspace_history`. Selection stays V05.2's `select_at`; a pick now also fetches the full properties of what it hit. |
| `beamcraft_ui::app` | The V05.2 window (click selection with the epoch rule, selection box, Edit panel with Move/Copy/Delete and Undo/Redo, `--workspace`) gains two panels. **Properties**: the full report of the selection, refreshed with every reload and pick; beneath it a typed patch (name, material keep/none/set, thickness for plates) staged and committed like the create tools; the position is shown read-only (Move edits it). **Workspace** (publication mode): draft and published heads, unsent revisions, whether the published head is still the base, and Publish (`workspace-publish`, fast-forward only) — Undo and Redo remain the edit panel's. One change command runs at a time across the create, edit, update and publish panels. |

Commands used, exactly as a user types them:

```text
beamcraft pick              --store ROOT --origin X,Y,Z --direction X,Y,Z --targets face --max-hits 1
beamcraft plate-inspect     --store ROOT
beamcraft update-element    --store ROOT --id UUID [--name N] [--material-digest D|none] [--thickness MM]
beamcraft workspace-status  --store PUBLISHED --workspace-dir DIR
beamcraft workspace-history --workspace-dir DIR
beamcraft workspace-publish --store PUBLISHED --workspace-dir DIR
```

## Decisions

1. **The slice builds on the merged V05.2, not beside it.** Selection, the click ray, the epoch
   rule, the selection box and the Move/Copy/Delete/Undo/Redo panel are V05.2's and are reused
   unchanged. This slice's first version had its own ray and pick mapping; it was deleted in the
   merge (`camera::pick_ray` and `props::pick_args`/`parse_pick` are gone, `edit::ray_through`
   and `edit::parse_pick` are the one implementation). A pick now additionally fetches the full
   properties of what it hit, so the properties panel is alive from the first click.
2. **The properties are a report, not a form binding.** Every value shown (including volume and
   mass with their `current`/`unknown` statuses and the profile and material display names) is
   what `plate-inspect` reported, and a refresh replaces it wholesale. An element that is no
   longer in the store clears the property view and says so instead of showing stale values; the
   edit panel's own selection semantics are untouched.
3. **An edit is a typed patch, exactly as typed.** `update-element` applies only the options it
   receives, so an untouched field sends no option: an empty name does not rename, "keep current"
   sends no material option, "none" sends `--material-digest none`. The staged argument list is
   shown one JSON-quoted argument per line (the create tools' display rule), Commit runs only
   while the form still produces it, and selecting another element starts from an empty form.
   The position is deliberately not in this form: a second way to move the element next to the
   edit panel's Move would be two fronts on one command.
4. **Publication mode amends V05.2's flag rule with a reason.** V05.2 decided `--store` and
   `--workspace` together are a usage error, because the workspace is the store. The publication
   panel needs the published store too, and nothing records its path in the workspace (the
   workspace metadata has no back-pointer), so V06 re-allows the pair: `--workspace DIR` alone
   behaves exactly as V05.2 (no publication panel), `--store PUBLISHED --workspace DIR` opens
   the same window with the panel. The published head moving is stated in the panel
   (fast-forward only, no merge) before Publish is attempted; a refused publish shows the
   engine's `BASE_REVISION_MISMATCH`.
5. **One change command at a time.** The create, edit, property-patch and publish panels share
   one busy gate, so the store never sees two concurrent change commands from this window; a
   refusal (`OP_ID_REUSE`, `BASE_REVISION_MISMATCH`, ...) is shown as received. Queries (pick,
   inspect, refresh) are not gated.
6. **Outcome unknown is not failure.** As in V05.1/V05.2: on `OUTCOME_UNKNOWN` the panel says
   the result is unknown, reloads and re-reads the selection, so every panel shows whatever was
   accepted. Each panel keeps its own note, and a refresh problem never overwrites a command
   result.

## Not done (not claimed)

- No highlight change for the properties panel (the edit panel's selection box is the one
  highlight); no multi-selection; no rotate, level, roll, profile or hole edits — the typed
  patch covers what `update-element` can change and the slice does not widen the engine.
- One `plate-inspect` run feeds the editor and a second feeds the properties panel (two reads
  per selection); one lookup serving both is a planned simplification, kept out of this slice to
  leave V05.2's tested API untouched.
- Publication covers status and publish only: no changesets, coordinator or conflict flow
  (C01–C02 stay CLI-only), and no create-workspace UI.
- The window has not been exercised by an automated GUI test; the logic under it is the tested
  `props`/`command`/`source`/`edit` modules, and the panels only wire widgets to them.
- Experimental: V06 acceptance is not claimed.

## Evidence

- Unit tests in `apps/beamcraft-ui/src/props.rs` (8): a plate and a member record are read with
  their dimensions, names, statuses and translations; a missing element is `None` and a broken
  reply is an error; a change reply carries outcome, element and sequence, and a refusal keeps
  its code; the update form maps exactly the changed properties (rename, material
  set/clear/keep, thickness; negative sizes are forwarded, bad numbers are refused before
  anything runs, no change at all is refused); the update state machine commits only what was
  previewed, only for the selected element, and blocks a second commit while busy; the workspace
  status and history replies are read; the workspace argument lists are exactly what a user
  types.
- `apps/beamcraft-ui/tests/properties.rs` (3, real `beamcraft` binary, no GPU needed):
  - pick and lookup report what the CLI reports: the panel's properties equal the engine's
    (name, kind, dimensions, volume 5 000 000 mm3, mass 39.25 kg, both `current`, the material's
    display name); a ray through the plate's world-bounds centre selects it and a ray away
    selects nothing; a store without a revision refuses with `NOT_FOUND`; a missing element is
    `None`;
  - an update through the form equals the same `update-element` by hand in a second store (name,
    thickness, translation, volume 10 000 000 mm3, mesh digest), the properties refresh shows
    the new values, an engine refusal (`INVALID_ARGUMENT`, thickness -5) changes nothing, and
    the staging machine refuses a stale preview;
  - the publication panel follows the real draft: a fresh fork has nothing unsent and no units,
    the panel's own update makes one unsent revision and one undo unit, publish empties the
    unsent count and equals the published head, a second publish of the unchanged draft is the
    same request and replays, undo and redo restore and reapply the edit while the published
    store keeps revision sequence 3 the whole time, and one whole window refresh reads the draft
    and the panel with the same result.
- The merged V05.1 and V05.2 suites still pass (`create_tools.rs` and `edit_tools.rs` were only
  adjusted to the grown `source::reload` signature and the publication-mode flags; their store
  behaviour tests are unchanged).

## Proposed Roadmap entry

`2026-10-19 V06: UI element properties and publication (apps/beamcraft-ui, on the merged V05.2 selection and edit tools: properties panel reading plate-inspect (name, kind, dimensions, profile, material, volume/mass with statuses), property edits staged as the exact update-element argument list (name, material, thickness; position stays the edit panel's Move), and a publication panel with workspace-status/-history state and fast-forward-only publish, enabled by --store PUBLISHED --workspace DIR; pure mapping and reply parsing in props.rs, end-to-end tests equal to the CLI by hand); experimental, no GUI click-path test, publication covers status and publish only.`
