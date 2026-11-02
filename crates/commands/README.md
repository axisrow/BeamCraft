# beamcraft-commands

The application layer of BeamCraft. Each module carries out one family of commands against a
project:

- it reads the verified head;
- it calls the engine and the discipline;
- it writes layers through the layer engine and commits through the host;
- it returns a typed result or a refusal with a stable F04 code.

It parses no command line and prints nothing. The `beamcraft` CLI (`apps/beamcraft-cli`) reads
the arguments, calls these modules and renders their results as JSON, and the window can call the
same modules.

Until #208 this code was the library target of `beamcraft-cli`. It moved here unchanged, so its
modules, functions and refusals are those the CLI has always run.

F08 kind `service`, layer 7. It depends on the platform crates (`model`, `json`, `engine`,
`host`, `layer-engine`, `ifc`, `session`) and on the `steel` discipline. Only an app or a tool may
depend on it.

| Area | Modules |
|------|---------|
| Store, scope and inspection | `plate` (scope, revision, inspection and the plate commands), `plate_draft`, `source_file`, `publication` |
| Steel parts | `member`, `copy`, `array`, `edit`, `delete`, `hole`, `cut`, `bend`, `loft`, `bolt`, `weld` |
| Definitions and libraries | `definition`, `library` |
| Platform elements | `grid`, `level`, `construction`, `annotation`, `assembly`, `attribute`, `reference`, `reference_view` |
| Numbering | `numbering`, `numbering_approval` |
| Outputs | `drawing`, `export`, `bom`, `schedules`, `filters` |
| Workspaces and collaboration | `workspace` (pending, rebase, resolve, compensation), `history`, `history_compact`, `changeset`, `coordinator`, `command_evidence`, `project` |
| Command inventory | `registry`: every command with its family, access, parameters and adapter route |

**Isolated verification.** Run `cargo test -p beamcraft-commands` for this crate's unit tests.
The end-to-end suites that run the real binary on real stores live with the CLI:
`cargo test -p beamcraft-cli --test cli`.

**Adding a command.** Put the work in a module here, behind a typed request and result. Then
register it in `registry.rs`. Its argument parsing, help and rendering go in the CLI adapter
(`apps/beamcraft-cli/src/*_cmd.rs`, `src/help/`).
