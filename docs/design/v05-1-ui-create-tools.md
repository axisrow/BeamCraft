# V05.1 UI create tools

Date: 2026-10-18. Status: **experimental vertical slice; V05 acceptance is not claimed.**
Branch `feat/v05-1-ui-create-tools`, based on `origin/master` `5c54c0a`.

Goal: the native UI (`apps/beamcraft-ui`) becomes a command client. A Create panel makes a plate or
a straight member with preview, cancel and commit, and a commit runs the same engine command the
CLI runs, then reloads the accepted revision.

## What is implemented

| Area | What exists |
|---|---|
| `beamcraft_ui::tools` (pure) | `PlateForm` and `MemberForm` map the typed parameters to the argument list of `create-plate` and `create-member` (`prepare`); `parse_created` reads the CLI's JSON reply into `Created` (element id, name, mesh digest, volume, revision, sequence) or into a `ToolError` (the CLI's stable code and message); `parse_definitions` reads `beamcraft definitions` into profile and material choices; `Tools` is the preview, cancel and commit state machine. No window, GPU or process, so it is unit-tested directly. |
| `beamcraft_ui::command` | Runs `beamcraft ARGS` in a separate process: stdin closed, standard output read with a 4 MiB cap, a 120 s deadline after which the process is killed (reported as `OUTCOME_UNKNOWN`), no console window on Windows. `commit` and `definitions` are the two uses. |
| `beamcraft_ui::app` | A Create panel in the left side panel: tool selector, fields, drop-downs for profile and material from the store's definitions, Preview, Commit, Cancel. Commit and every refresh run on worker threads so the window never waits for a CLI process; results are polled each frame (see decisions 7 and 8). Success starts a refresh of the accepted snapshot (the same as `R`) and of the definitions list. F and R are ignored while a text field has the keyboard. |

Commands used, exactly as a user types them:

```text
beamcraft create-plate  --store ROOT --width W --length L --thickness T [--name N] [--material-digest D]
beamcraft create-member --store ROOT --kind beam|column --start X,Y,Z --end X,Y,Z
                        --profile-digest D [--name N] [--material-digest D]
beamcraft definitions   --store ROOT
```

## Decisions

1. **The UI is a client of the CLI process, as V03.** Only the CLI links the layer engine, and F08
   forbids an app depending on an app. The commit is therefore `beamcraft create-*` in a separate
   process; the revision that results is the engine's, not a copy made in the window. The only
   new dependency edge is `beamcraft-ui` -> `beamcraft-json` (`beamcraft-protocol` at the time;
   already a workspace crate, layer 2 < 5), to decode the CLI's JSON reply with the project's bounded decoder instead of ad-hoc
   string matching. The module table in `docs/contracts/f08-module-boundaries-v0.1.md` is updated
   with it. No third-party dependency was added.
2. **The UI computes no engineering geometry.** It checks only that a number is a finite decimal
   and that required choices (profile) are made, then forwards the trimmed text. Positive sizes,
   limits, profile and material resolution, placement and the shape itself are the engine's; its
   refusal is shown as received (for example `INVALID_ARGUMENT: dimension is not positive`).
   Errors are never swallowed: a refusal, an unreadable reply, a CLI that cannot be started or a
   timeout each show a code and message in the panel, and a zero exit status cannot turn a
   rejection reply into a success.
3. **Preview is the staged command, not geometry.** A geometry preview would need either a copy
   of the kernel's layout rules in the UI (forbidden) or an engine dry-run command (it does not
   exist). Preview therefore validates the form, shows the exact argument list that Commit will
   run (one JSON-quoted argument per line; it is deliberately not a shell command line, because no
   shell quoting rule fits every argument), and stages it. Commit is enabled only while the form still produces the staged command;
   editing after the preview disables it ("preview again"). Cancel drops the staged command; since
   nothing was run, nothing is published (tested: the store still has no revision).
4. **Definitions come from the engine.** The profile and material lists are the CLI's
   `definitions` reply, never a table in the UI. Every non-material definition is offered as a
   profile; the engine decides whether a kind is valid for a member.
5. **One operation per commit.** The CLI generates the element and operation ids. Replaying an
   operation is not exposed; `Created.replayed` is parsed for completeness.
6. **Outcome unknown is not failure.** On `OUTCOME_UNKNOWN` (exit 4 or the deadline) the panel says
   the result is unknown and reloads, so the viewport shows whatever was accepted.
7. **Refresh is off the UI thread.** `source::reload` runs the snapshot export, the engine rebuild,
   the scene build and the `definitions` query on a worker; only the GPU upload happens on the UI
   thread when the result is polled. A refresh requested while one runs is queued, so the last
   one reads the store after the change that asked for it. The snapshot export runs under the
   same 120 s deadline as the create commands (`source::load_within`, built on
   `command::run_capped`; its output cap stays the engine's bundle limit, which a 4 MiB cap would
   break), so a hung CLI ends in a typed error (`export-view did not finish in time`) shown in the
   panel instead of an endless refresh.
8. **Two statuses, never one.** The result of the last Preview/Cancel/Commit and the error of the
   last definitions refresh are separate fields and both are shown, so a refresh failure (which
   also clears the now stale choices) is never overwritten by a success message, including after
   an unknown outcome. `Tools::apply_definitions` holds that rule.
9. **Bounded input.** Every text editor, including the six Start/End coordinates, is capped at
   `MAX_TEXT` characters before layout.

## Not done (not claimed)

- No geometry or bounding-box preview in the viewport (decision 3); no graphical placement:
  start and end points are typed, not picked (V04 picking and snaps exist in the engine and CLI but
  are not wired into the window).
- No plate contour field: this branch's `create-plate` has no contour option (rectangles and holes
  only); plate holes (`--hole-rect`, `--hole-round`) are not in the panel.
- Member roll, translation, section-family profile choices beyond what `definitions` lists, and
  defining new profiles or materials from the window (`define-profile`, `define-material`) are not
  in the panel; an empty store needs those definitions made with the CLI first.
- No edit, move, copy, delete, undo or workspace commands in the window; every commit goes to the
  published store head (the same as running the CLI without a workspace).
- The window has been started on Windows and stays up, but the click path (typing, Preview,
  Commit) was not exercised by an automated GUI test; the logic under it is the tested `tools` and
  `command` modules, and the panel only wires widgets to them. Not run on Linux or macOS.
- Experimental: no properties or publication UI (V06).

## Evidence

- Unit tests in `apps/beamcraft-ui/src/tools.rs` (13, including the definitions-refresh rule): exact argument lists for plate and member
  (including name and material, trimming, the displayed argument list round-trips names with quotes, backslashes, shell characters and newlines), rejection of
  empty, non-numeric and non-finite numbers and an over-long name before any process runs,
  forwarding of engineering rules to the engine (a negative thickness is not judged by the UI),
  reading of created, stale-volume, refusal, unknown-outcome and garbage replies (a forged code is
  not echoed, a zero exit cannot hide a rejection), splitting of definitions, and the state machine
  (commit only what was previewed and only while unchanged, no second commit while busy, cancel).
- `apps/beamcraft-ui/tests/create_tools.rs` (6, real `beamcraft` binary, no GPU needed):
  - a plate made through the tool mapping and `command::commit` equals the same plate made by
    hand in a second store: same `mesh_digest`, same volume (5 000 000 mm3 for 1000 x 500 x 10),
    same volume algorithm, same world bounds, and the viewport reload pins the manifest digest the
    command reported (one element, one mesh); a preview that is cancelled leaves the store without
    any revision;
  - a column from the store's own profile and material (chosen through `command::definitions`)
    equals the hand-made one: same mesh digest, volume (180 000 000 mm3), mass, kind, profile and
    material digests, bounds, and reload;
  - an engine refusal (width -5, and a well-formed profile digest the store lacks) reaches the
    caller with the CLI's own code (`INVALID_ARGUMENT`, `NOT_FOUND`) and message, identical to the
    hand run, and publishes nothing;
  - a missing CLI is `CLI_UNAVAILABLE`, not a hang;
  - a snapshot export with a zero deadline is stopped and reported as `did not finish in time`;
    a missing CLI is `CLI_UNAVAILABLE`;
  - `source::reload` run on a spawned thread: on a store without a revision the snapshot and the
    definitions each fail on their own with `NOT_FOUND`; with a revision both succeed.
- The window (`beamcraft-ui --store ROOT`) was started on Windows (adapter not recorded) and stayed
  up; it was then stopped.
