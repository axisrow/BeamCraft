# session

Platform state for the applications. It is what the command line and the window both need and
that is neither storage (`beamcraft-host`) nor a discipline (steel today, concrete or MEP later).
It carries no `beamcraft-` prefix because nothing in it is specific to steel.

Both apps call it in their own process; every function is bounded and returns a typed error.

| Module | What it does |
|---|---|
| `settings` | The per-user `settings.json`: author profile, appearance (`System`, `Light`, `Dark`), the environment chosen at start, the models folder and the recent list. A missing file is the default settings; a damaged, oversized or foreign-version file is refused and never overwritten. `Settings::update` reads, changes and saves under one lock of the process, so two threads cannot lose each other's edit. `BEAMCRAFT_CONFIG_DIR` overrides the directory. |
| `project` | Project folders as people see them: portable-name check, free-name suggestion (`next_project_name`), listing, and reading a head without verifying every blob. Creating a project is a command and stays in the CLI. |
| `environment` | The environment layers (`environment.usda`): admission of their declarations, the built-in and user folders, and the list with shadowing and refusals. Loading the libraries an environment lists is a command and stays in the CLI. |
| `recent` | The recent list over the settings and the folders: a vanished project is dropped on read, and only a project with a readable head is remembered. |
| `foreign` | One read-only, self-contained USD layer from outside the project (a library or an environment file), opened under fixed budgets. |

Dependencies point down only: `beamcraft-model`, `beamcraft-json`, `beamcraft-host` and
`beamcraft-layer-engine` (F08, `docs/contracts/f08-module-boundaries-v0.1.md`).

The command registry and a long-lived session over a verified revision are planned for this
crate; see `docs/architecture.md`.
