# In-model menu

The Menu of a model window (`App::menu_page`, `apps/beamcraft-ui/src/app/workbench.rs`) is a
full-window page with a navigation column on the left and the chosen page on the right. It lets
the user work with models without leaving the open one. The change is UI only: the store format,
the settings file, the engine contracts and the CLI surface are unchanged.

## Navigation

The rows, top to bottom: Back to model, the "Menu" title, New, Open, Save as (disabled), Open
model folder, Import, Export, Project properties, Settings, Catalogs, Editors, Logs (disabled)
and Help. Diagnose & repair, Extend and Exit BeamCraft were placeholders and are gone. New and
Open are enabled only in a window started from the library (`from_library`) and while no command
of the window runs (`other_command_running`); a window started with `--store` or `--workspace`
shows them disabled.

A sub-page has a parent (`MenuSection::parent`): Profile catalog and Material catalog belong to
Catalogs, About BeamCraft to Help. A sub-page highlights its parent's row (`MenuSection::top`)
and shows "‹ Back" before its title. `menu_back` is the one rule for "‹ Back", Esc and Cancel: a
sub-page goes to its parent, a top-level page closes the menu. Every way out of the menu goes
through `close_menu`, which also resets the New, Open and Project pages to the default page and
clears the page note, so a reopened menu never shows a stale form or stale facts.

Esc while the menu is open belongs to the menu: `App::ui` leaves it alone and `menu_page`
consumes it. Two handlers run earlier in the same frame and take it first. The command search
(Ctrl+K) closes on Esc and leaves the menu open. During an active view pick the first Esc cancels
the pick, and the next one acts on the menu; this is accepted, because the pick would otherwise
stay armed under the menu.

`menu_busy` is true only while a menu page runs a job that opens a model: a `create-project` on
New (`NewModelForm::creating`) or a remember on Open (`ModelPicker::opening`). While it is true,
Esc, "‹ Back" and `close_menu` do nothing, every navigation row (Back to model too) is disabled,
and the command search's Model library, Undo and Redo are disabled: leaving would lose the model
the job opens, and the switch that follows would drop an edit's result. The name suggestion and
the recent-list read are background reads and do not count, so Esc right after opening New or
Open never depends on thread timing. A note under the page heading (`menu_note`) shows what the
last menu action reported; it belongs to the page and is cleared when the page changes.

## New and Open

Both pages use `apps/beamcraft-ui/src/library.rs`, which the start page shares. New is the start
page's new-model form (`NewModelForm`): name, location, template and the `create-project`
command; the location starts next to the open model, and the form lives with the window, so a
typed name survives leaving the page. Open (`ModelPicker`) lists the recent models, read with
`session::recent` on a worker each time the page is entered, marks the open model "Open now" and
offers "Browse…"; a chosen model is remembered as opened through
`session::recent::remember_project`, as the start page does, and `create-project` remembers a
created one itself. Each piece runs one job at a time.

Neither page closes the menu. A created or chosen model only sets the window's `open_request`.
After the window's frame, `Shell` takes it with `App::take_open_request`, which leaves the
request in place while a command of the window still runs: the switch waits instead of dropping
a result. The new window keeps the old one's CLI, MSAA samples, actor, author, environment and
configuration directory, opens the store as the library does (no workspace, no publish target)
and takes over the appearance. Read jobs of the old window (refresh, selection, status) die with
it, which is harmless; a pending publish confirmation is not drawn while the menu is open and is
discarded.

The failure policy is narrow. `App::open` fails only without a wgpu render state or on a
renderer error; then the current model stays and its menu shows "the model could not be
opened: …". A store whose head reads but whose snapshot does not export still switches, and the
new window shows the error, as after opening from the start page.

The replaced window is not dropped at once. `Shell` keeps it in its `retired` slot and releases
its textures (`App::release`: the viewport and every view window and preview) and drops it at
the start of the next pass. egui paints only the last pass of a frame, and the output of the
pass that switched may still refer to the old textures; the next pass's output replaces it. The
"Model library" command goes through the same slot, so returning to the library no longer leaks
the viewport's texture entry. Freeing a texture twice is a no-op.

## Open model folder

`dialog::open_folder` shows the model's store directory in the system file manager: `explorer`
on Windows, `open` (Finder) on macOS and `xdg-open` elsewhere (`dialog::folder_command`), with
the absolute path as one argument and no stream attached. It does not wait: a thread named
`beamcraft-open-folder` waits for the child so Unix leaves no zombie, and the exit status is not
read, because `explorer` returns 1 even when the folder opened. A folder that is gone, a path
that cannot be resolved or a missing launcher is a note on the page; nothing panics. No crate is
added.

## Project properties

Project properties is a read-only page. Its first part comes from the store's head through
`library::project_facts`, which calls `session::project::project_info` on a worker thread named
`beamcraft-project` and writes nothing: the name, the folder, the ProjectId, the creation time
(the ProjectId's timestamp) and the last change (the head revision id's timestamp), both in UTC,
and the state (`ready`, `empty`, `missing` or the code of an unreadable store). The page polls
the read every 50 ms and shows a spinner meanwhile, so a large model never freezes the window.
Its second part is this session: the environment, the author, the units (mm) and the revision
on screen with its element count, from the pinned snapshot in the format of the Details window.
The Details window stays reachable from the status bar and the command search.

A muted sentence says that project information (number, client, building and site address,
designer, dates and custom fields) is not stored in models yet. Editable project properties are
a separate, later task; this note records no decision on where they are stored.

## No CLI command

`AGENTS.md` section 3 asks for a CLI command for every feature; the menu adds none:

- New and Open are the start screen's own flows, reached from the menu, which the section 3
  exception covers. Creating still runs the existing `create-project` command.
- Opening the model folder is an operating-system integration. An agent reads the folder
  directly and needs no command for it.
- The Project page's data is already headless: `list-project --dir <parent>` reports the same
  `project_info` reading (name, path, state, project ID, times and the head revision).

## Tests

Headless window tests drive the real `App::ui` frame: the rows and the removed entries, "‹ Back"
and the Esc walk on sub-pages, the command search taking Esc first, Esc waiting while a create
or an open runs, in-menu New and Open with "Open now", the disabled rows without the library,
the open-folder note and the Project page. `Shell` tests cover a failed switch that keeps the
model and a switch that frees the old textures one pass later. `library` tests cover the form,
the picker and the project facts of missing and empty stores; `dialog` tests cover the
file-manager command and a refused missing folder.

The code and test inputs are contributor-original MPL-2.0, with no new dependency or asset.
