# V05.2 UI edit tools

Date: 2026-10-19. Status: **experimental vertical slice; V05 acceptance is not claimed.**
Branch `feat/v05-2-ui-edit`, based on `origin/master` `11a2139`.

Goal: the native window selects an element with the mouse and edits it through the same engine
commands the CLI runs: move, copy, delete, undo and redo. Property editing is V06.

## What is implemented

| Area | What exists |
|---|---|
| `beamcraft_ui::edit` (pure) | `ndc_of` and `ray_through` (a clicked pixel to a world ray: the inverse of `View::project`, shifted back by the scene origin in f64); `pick_args` / `parse_pick` (the CLI `pick`: nearest face hit only); `inspect_args` / `parse_element_info` (name, kind and translation of one element from `inspect-elements`); `Editor` (the selection, the move and copy forms and the preview, commit and cancel state machine); `Edit` / `parse_edited` (the argument list and the reply of one edit). No window, GPU or process, so it is unit-tested directly |
| `beamcraft_ui::command` | `select_at` (runs `pick`, then looks the hit element up), `element_info`, `commit_edit` (runs an edit in a separate process under the V05.1 deadline and output cap) |
| `beamcraft_ui::app` | A click on the view selects (a drag still orbits); the selection is drawn as the projected box of the engine's world bounds of the element; an Edit panel with Move, Copy and Delete (preview, commit, cancel, as the Create panel) and Undo and Redo buttons; Esc clears the selection. Lookups and commits run on worker threads and a success reloads the accepted revision |
| `beamcraft-ui --workspace DIR` | opens the window on a personal workspace (a store made by `create-workspace`). It is also the store every command and the viewport use, so edits are draft revisions of the workspace; Undo and Redo act on it |

Commands used, exactly as a user types them:

```text
beamcraft pick            --store ROOT --origin X,Y,Z --direction X,Y,Z --targets face --max-hits 1
beamcraft inspect-elements   --store ROOT                                  (details of the picked element)
beamcraft update-element  --store ROOT --id UUID --translation X,Y,Z
beamcraft copy-element    --store ROOT --id UUID --offset X,Y,Z
beamcraft delete-element  --store ROOT --id UUID
beamcraft undo-workspace  --workspace-dir DIR
beamcraft redo-workspace  --workspace-dir DIR
```

## Decisions

1. **The engine picks, the UI only aims.** The UI turns the click into a ray (camera math, display
   only, like the existing projection) and the engine's `pick` (V04) decides what was hit. A click
   on empty space clears the selection. Picking by id lists plates and members only (the kinds
   with a body); assemblies, welds and levels are not selectable in the view.
2. **A move is an absolute position.** `update-element --translation` is absolute. The form is
   pre-filled with the element's present translation as the engine reports it, and the user edits
   it; the UI adds and subtracts nothing. A copy takes an offset because `copy-element --offset`
   is one. Rotation is kept by the engine (P08).
3. **Preview is the staged command, as in V05.1.** Move, copy and delete are staged and shown as
   the exact argument list; Commit runs only what was previewed and only while the forms still
   produce it; editing after the preview disables Commit; Cancel runs nothing. Undo and redo run
   at once: each is itself undoable.
4. **Undo and redo need a workspace.** The published store has no undo (P07 works on a personal
   workspace's revision log). Opened with `--store`, the window disables both buttons and says
   why; opened with `--workspace DIR`, edits are draft revisions and undo and redo restore them.
   `--store` and `--workspace` together are a usage error (a workspace is a store).
5. **Nothing is computed.** The UI checks only that a typed number is finite and that something
   is selected. Limits (a translation beyond the coordinate cap is `LIMIT_EXCEEDED`), existence,
   dependencies (a welded part cannot be deleted: `DEPENDENCY_CONFLICT`) and "nothing to undo"
   are the engine's, shown with its code and message, and a refused edit publishes nothing.
6. **Selection follows the model.** After a successful edit the window reloads and re-reads the
   selected element (it may have moved, or an undo may have removed it, in which case the
   selection is cleared). A delete clears the selection. If the details cannot be read the element
   stays selected with an empty move form, never a guessed one.
7. **Late results never win.** Every lookup carries an epoch; Esc and every accepted edit move
   the epoch on and drop the receiver in flight, so a late `pick` or details reply can neither
   bring back a cleared selection nor put pre-edit coordinates into the move form
   (`beamcraft_ui::edit::Epoch`). A details lookup after an edit that fails for any reason other
   than `NOT_FOUND` keeps the element selected with an empty move form (never the old position);
   `NOT_FOUND` clears the selection.
8. **Switching the action drops the preview.** A staged Delete cannot be committed from the Move
   panel: `Editor::set_action` cancels the staged edit when the action changes.
9. **The highlight is the engine's bounds.** The box comes from `ViewSnapshot::element_bounds`;
   the renderer is unchanged.

## Not done (not claimed)

* No multi-selection, box selection or selection of edges and vertices; no snapping or dragging in
  the view (`snap` exists in the CLI, V04, but is not wired); a move is typed.
* No property editing (V06), no rotate, no mirror, no array copy, no keyboard Delete (a delete is
  always previewed and committed).
* Selection details come from `inspect-elements`, whose reply is bounded by the control-message
  limits; for a very large model the details lookup fails and the element is selected without
  them (decision 6).
* Undo and redo exist only in a workspace; publishing a workspace is `beamcraft publish-workspace`.
* The click path (mouse, panel buttons) was not exercised by an automated GUI test; the logic under
  it is the tested `edit` and `command` modules, and the panel only wires widgets to them. The window
  has not been started in this environment (no display).

## Evidence

* Unit tests in `apps/beamcraft-ui/src/edit.rs` (14, including the epoch rule and the action switch): pixel to device coordinates inside the view
  only; the ray through a projected point hits that point (for a rotated camera and a large scene
  origin); exact `pick` arguments; pick, inspection and edit replies (accepted, forged outcome of
  another command, rejection that a zero exit cannot hide, crash without JSON); the argument lists
  of move, copy, delete, undo and redo; undo and redo refused without a workspace; number checks;
  the preview, commit and cancel rules; direct undo and redo not while busy.
* `apps/beamcraft-ui/tests/edit_tools.rs` (7, real `beamcraft` binary, no GPU): a computed click on
  the middle of each plate's top face selects that plate through the real `pick` and
  `inspect-elements` and a click on empty space selects nothing; a move, a copy and a delete made by the tools leave the
  store in exactly the state of the same commands by hand (translation, mesh digest, world
  bounds); a cancelled preview leaves the head unchanged; an engine refusal (a translation of
  1e12, a delete of a welded part) reaches the panel with the CLI's code and message and publishes
  nothing; in a workspace, undo restores the earlier state, redo the edit, a surplus undo is the
  engine's refusal, and the published store's head and state never change; the window binary
  refuses `--store` with `--workspace`.
