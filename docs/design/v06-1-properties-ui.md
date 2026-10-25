# V06.1 properties editor and Publish in the native UI

Date: 2026-10-19. Status: **experimental vertical slice (reduced V06); V06 acceptance is not claimed.**
Branch `feat/v06-1-properties-ui`, based on `origin/master` `e2d5df2`.

Goal: the window shows the selected plate or straight member's properties, edits the ones the
engine supports through the same command the CLI runs (`update-element`), says whether what it
shows is current, stale or unknown, and publishes a personal workspace through `publish-workspace`.
The UI stays a thin client: no engineering value is computed here.

## What is implemented

| Area | What exists |
|---|---|
| `beamcraft_ui::props` (pure) | `Details` and `parse_details` (one element of the `inspect-elements` reply, with the revision it was read from); `State` (`Current`, `Stale`, `Unknown`); `Properties` (the reading, the form and the preview, commit and cancel state machine); `PropertyEdit` (the one `update-element` argument list). No window, GPU or process |
| `beamcraft_ui::publish` (pure) | `publish_args` / `parse_published` (`publish-workspace`), `status_args` / `parse_status` (`inspect-workspace`), `Publisher` (request, confirm, run, result; a refusal is only recorded) |
| `beamcraft_ui::command` | `element_details`, `commit_property`, `publish`, `workspace_status`; `Selected` now carries the details read with the pick |
| `beamcraft_ui::tools` | `Definitions::head`: the head revision named by the `definitions` reply that every reload already runs, which is how the window learns the head's sequence and digest |
| `beamcraft_ui::app` | A Properties panel (read-only rows, editable fields, Preview, Commit, Cancel, Re-read) and a Publish panel with a confirm dialog |
| `beamcraft-ui --workspace DIR --published ROOT` | The workspace does not record where it was forked from, so Publish needs the published store named; without both, Publish is off and says why |

Commands used, exactly as a user types them:

```text
beamcraft inspect-elements     --store ROOT                                   (the properties of the selection)
beamcraft definitions       --store ROOT                                   (materials; the head revision)
beamcraft update-element    --store ROOT --id UUID [--name N] [--material-digest D|none]
                            [--thickness MM] [--translation X,Y,Z]         (only the changed fields)
beamcraft inspect-workspace  --store PUBLISHED --workspace-dir DIR
beamcraft publish-workspace --store PUBLISHED --workspace-dir DIR
```

## Editable properties

`update-element` supports a label, a material reference (a definition digest or `none`), a plate's
thickness, an absolute translation and a level. The panel edits the first four. Everything else
the engine reports is shown read-only: a plate's width and length, a member's profile, section and
axis points, the level, volume, mass and density.

## Decisions

1. **Same command as the CLI.** The panel builds `update-element` from the form and sends only
   the fields that differ from the reading (an unchanged field is not re-sent, so a stale value
   can never overwrite a newer one by accident). The preview is the exact argument list; Commit
   runs only what was previewed and only while the form still produces it; Cancel runs nothing.
   `tests/properties.rs` proves the saved result equals the CLI's: the same edit made by the
   panel on one store and typed by hand on an identical store leaves the whole element identical
   (properties, quantities, mesh digest) for a plate and a member.
2. **Current, stale, unknown.** Each reading carries the revision it was read from. The head is
   the revision the last `definitions` reply named (that command already runs on every reload).
   *Current*: the reading is the head's, or newer than the last head seen (the view has not
   reloaded yet). *Stale*: the revision differs from the head's and is not newer (the head moved
   on, or has another digest at the same sequence). *Unknown*: the properties could not be read
   (the element is not a plate or straight member, the reply is over the control-message limit
   or unreadable, the CLI failed); the panel then shows no value and says why. If the head itself
   could not be read the reading is shown as current with a note that the head was not checked.
3. **A stale or unknown reading is never edited.** Preview and Commit are refused until Re-read
   makes it current; a staged edit also stops being committable when the head moves under it.
4. **Unknown values are labelled, never guessed.** A quantity the reply does not carry is
   `unknown (not reported)`; one with a status other than `current` is shown with the engine's
   status; a mass the engine could not compute (for example no material) is shown as
   `not available (engine status: ...)`. The editable fields, the revision and the kind are
   required in the reply: without them the whole reading is unknown.
5. **Early refusal is input hygiene only.** A typed value must be a finite number and not beyond
   the engine's published limits (`DIMENSION_CAP_MM`, `COORDINATE_CAP_MM`); a label is capped at
   the UI's text limit and must not be empty; a thickness must also be above zero. The limits are the
   model crate's own constants, not copies. Everything else (an unknown material, a patch the
   engine refuses) is the engine's refusal, shown with its code and message, and publishes
   nothing.
6. **Publish is `publish-workspace`.** The dialog names the command and states that the engine
   refuses a moved published head or an unchanged draft and that there is no merge. Only Publish
   in the dialog runs anything; Esc or Cancel closes it. The result or the refusal
   (`BASE_REVISION_MISMATCH`, `INVALID_ARGUMENT`, ...) is displayed and nothing else happens: no
   conflict resolution (C03 to C06 are deferred). The panel also shows `inspect-workspace` as the
   engine reports it.
7. **One writer at a time.** Create, edit, property, undo and redo and Publish never run
   concurrently; each refuses while another command is in flight.

## Not done (not claimed)

* Not editable: a plate's width, length and contour, a member's profile and axis, the level
  assignment, holes. `update-element` has no field for the first three (a new engine command
  would be needed; none was added), and the level needs a level picker that is not built. They are
  shown read-only.
* No multi-selection; no property editing of welds, assemblies or references; no drag handles,
  rotate or mirror.
* No conflict resolution, rebase or retry on a refused publication (C03 to C06).
* The material list is the first page of `definitions` (64 entries); a larger store lists only
  those, as the Create panel already does. A material pinned by an element outside the page is
  shown as `unlisted definition`.
* A stale reading is detected when the window next reloads (an edit made by this window, `R`, or
  Re-read); it does not poll the store.
* The properties are read from `inspect-elements` of the whole revision, whose reply is bounded by
  the control-message limits; for a very large model the reading is unknown (decision 2).
* The click path (mouse, panel buttons, the dialog) was not exercised by an automated GUI test;
  the logic under it is the tested `props`, `publish` and `command` modules and the panel only
  wires widgets to them. The window has not been started in this environment (no display).

## Evidence

* Unit tests in `apps/beamcraft-ui/src/props.rs` (9): reading a plate and a member; unknown
  values labelled; a reply missing an editable field or the revision is unreadable, a missing
  element is `NOT_FOUND`; the current, stale and unknown states including a head newer, older or
  unknown; only changed fields in the command; non-numbers, NaN, infinities, zero and negative
  thicknesses, over-limit and over-long values refused; a stale or unknown reading never edited;
  preview, commit and cancel including edit-after-preview, head-moved-after-preview and busy.
* Unit tests in `apps/beamcraft-ui/src/publish.rs` (6): the commands; publishing needs a
  workspace and its published store; only a confirmed dialog runs; replies and forged outcomes;
  a refusal is displayed with the engine's code; the status.
* `apps/beamcraft-ui/tests/properties.rs` (4, real `beamcraft` binary, no GPU): the panel's edit
  equals `update-element` by hand for a plate (all four properties and clearing the material) and
  a member; a reading goes stale when the head moves and is not edited; an unreadable element is
  unknown and an engine refusal carries the CLI's code and message and publishes nothing;
  Publish refused for nothing to publish, succeeding with the published element equal to the
  workspace's, and refused with `BASE_REVISION_MISMATCH` after the published head moved, the
  published head unchanged.
