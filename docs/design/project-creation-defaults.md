# Model creation defaults

> **2026-10-31:** the CLI commands `suggest-project-name`, `list-recent` and `add-recent` were
> removed (`AGENTS.md` section 3). The engine functions behind them remain and serve the start page.

The start page suggests `New Model`, then the first available `New Model N` in the selected
parent folder. Files, directories and links occupy names. Opening the form or changing its
location asks `suggest-project-name`; the query creates nothing. A manually edited name stays
unchanged when the location changes.

The default creation uses `create-project --name "New Model" --unique-name true`. It reserves
the first available directory atomically and tries the next suffix on a creation collision.
This remains safe if another process takes the suggested name before Create is pressed.
Search is bounded to 10,000 candidates. Explicit CLI names retain their ordinary duplicate-name
refusal unless the caller requests the unique-name option. No existing file is overwritten.

`list-recent` removes confirmed missing model folders from saved settings. An unreadable model
is kept, and unchanged listings do not rewrite settings. Other profile settings are preserved.

Actual-binary tests cover suggestions without writes, numbered creation, occupied files,
concurrent creation, explicit-name refusal, persisted recent cleanup, and UI argument mapping.
The code and synthetic test inputs are contributor-original MPL-2.0, with no new dependency
or asset.
