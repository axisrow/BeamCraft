# V05.2 UI move and delete tools

Date: 2026-10-19. Status: **experimental vertical slice; V05 acceptance is not claimed.**
Branch `ao/extend-the-native/feat/v05-2-move-delete` (the `feat/v05-2-move-delete` slice), based on `origin/master` `11a2139`.

Goal: the native UI grows from create-only to editing. The window lists the accepted revision's
elements for selection, and a selection is moved or deleted with the same engine commands the CLI
user types (`update-element --translation`, `delete-element`), then the accepted revision is
reloaded.

## What is implemented

| Area | What exists |
|---|---|
| `beamcraft_ui::tools` (pure) | `ToolKind` gains `Move` and `Delete`; `ElementRow` is one listed element (id, kind token, placement); `Tools::select` stores the selection and pre-fills the move form with the snapshot's placement; `prepare` produces the exact `update-element` / `delete-element` argument list of the selection; `parse_updated` and `parse_deleted` read the CLI's JSON reply or its refusal; `EditResult` carries whichever answer a committed tool produced, with the panel's note (`summary`). The create tools of V05.1 are unchanged in behaviour (their reply struct now shares the `Revision` reader). |
| `beamcraft_ui::command` | `run_tool` runs a previewed command and parses its reply by the tool's kind, on the same capped, deadline-bounded process runner as before. |
| `beamcraft_ui::app` | An Elements list at the top of the side panel (one row per snapshot element, kind and id, click to select); the tool row grows Move and Delete tabs; the move form takes the new absolute translation X/Y/Z; the delete tab states what Commit removes; selection, staged command and the two-status rule work exactly as in V05.1. A committed move, delete or create reloads the accepted revision and the definitions. A reload drops a selection the new revision no longer lists. |

Commands used, exactly as a user types them:

```text
beamcraft update-element --store ROOT --id UUID --translation X,Y,Z
beamcraft delete-element --store ROOT --id UUID
```

## Decisions

1. **The same client pattern as V05.1.** The commit is the CLI command in a separate process;
   the revision that results is the engine's. No new dependency, no new process plumbing — the
   edit commands are two more argument lists through the same runner and reply decoders.
2. **Selection is from the accepted snapshot, not from the viewport.** V04's picking exists in
   the engine and CLI but is not wired into the window; the smallest honest selection is the
   Elements list built from the same `view-export` snapshot the viewport draws, so what the user
   clicks is exactly what the engine reported (id, kind, placement). Click-to-pick in the 3D
   view stays future work.
3. **Move is the engine's semantics: an absolute translation, rotation kept.** The form shows
   the new placement translation and is pre-filled with the snapshot's placement, so a move is
   typically one edited coordinate. The UI adds no delta arithmetic and re-derives no geometry;
   it forwards the text, and the engine places the element. A snapshot read just before a
   foreign change could be stale; the preview shows the exact `--translation` that Commit will
   send, so what is sent is never in doubt.
4. **Delete is preview-gated.** The staged preview shows the exact `delete-element` arguments
   before Commit exists as enabled, and the tab states that Commit removes the whole element and
   that this window has no undo. The engine still refuses a delete that would leave a link
   behind (`DEPENDENCY_CONFLICT`), and that refusal is shown as received.
5. **One state machine for all four tools.** Preview, commit gating ("commit only what was
   previewed and only while unchanged"), cancel, busy and the two-status rule are the V05.1
   logic unchanged; a staged command stops being committable the moment the selection, the move
   text or any form field produces a different argument list. One command runs at a time.
6. **The selection follows the accepted revision.** After each reload a selection the new
   revision no longer lists is dropped (`retain_selection`), so a delete or a foreign change
   cannot leave the panel addressing an element that is not there. A staged command of another
   selection is invalidated by the same comparison rule.
7. **Bounded input.** The move fields are capped at `MAX_TEXT` like every other editor; a
   non-finite or non-numeric translation is refused before any process runs, while a negative
   translation is forwarded (absolute placements may be negative; that is the engine's to
   judge).

## Not done (not claimed)

- No click-to-pick in the 3D view (decision 2); selection is the list only. No graphical move
  handle, no snap-driven placement.
- No undo/redo, no workspace commands, no copy, rename, material, thickness or level editing in
  the window; `update-element` is used only for `--translation`. Every commit still goes to the
  published store head.
- No multi-select; one element at a time.
- The window has not been exercised by an automated GUI test; the logic under it is the tested
  `tools` and `command` modules. Not run on Windows or Linux in this slice.
- Experimental: no properties or publication UI (V06); V05 acceptance is not claimed.

## Evidence

- Unit tests in `apps/beamcraft-ui/src/tools.rs` (17, four new in this slice): the exact argument lists of the move and
  the delete (selection required, numbers checked before any process runs, negative translation
  forwarded), selection pre-fill and re-fill, the staged move invalidated by a changed selection
  or text, `retain_selection`, reading of updated and deleted replies (current volume, replay
  outcomes, a null kind on a delete replay, missing fields, wrong-command replies, refusals kept
  with code and message and never hidden by a zero exit), and the success notes; the V05.1 tests
  pass unchanged in behaviour over the shared `Revision` reader.
- `apps/beamcraft-ui/tests/edit_tools.rs` (2, real `beamcraft` binary, no GPU needed):
  - a plate created by hand in two stores, then moved through selection + tool mapping +
    `run_tool`: the accepted revision's rows carry the engine's id, kind and placement; the move
    equals the hand-run `update-element` (same mesh digest, same volume 5 000 000 mm3, same
    placement 500,0,250); the reload pins the manifest digest the move reported and lists the
    new placement; the spent staged command is no longer committable; the delete then equals the
    hand-run `delete-element` (kind `plate`, sequence 3), both stores end with no elements, and
    the deleted element is deselected;
  - a move or delete of a well-formed id the store does not have reaches the caller with the
    CLI's own `NOT_FOUND` and message, identical to the hand run, and publishes nothing (the
    revision digest, the element count and the placement are unchanged).
- The create-tools end-to-end tests still pass over the shared reply reader.

## Proposed Roadmap entry

`2026-10-19 V05.2: UI move and delete tools (apps/beamcraft-ui: Elements list from the accepted snapshot for selection, move to an absolute translation and delete of a selected element through the CLI's update-element / delete-element in a separate process with preview, cancel and reload; selection follows the accepted revision, end-to-end test equal to the CLI by hand); experimental, no click-to-pick or graphical move.`
