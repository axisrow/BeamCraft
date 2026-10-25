---
name: model
description: Drive BeamCraft steel detailing through the beamcraft-cli - create, update and inspect members and plates (define-profile, define-material, create-member, create-plate, update-element, plate-inspect), create and verify a store (project-create, revision-open), pick and resolve coordinates (pick, feature-list, feature-lookup, snap), and snapshot or export views (view-snapshot, view-export). Use when the user asks to model, edit, move or list elements in a BeamCraft store, to check or create a store, to snap or resolve a coordinate or feature address, or for a view snapshot or export. The slash command /beamcraft:open launches the native viewport app separately.
argument-hint: [what to do, and where]
---

# Drive BeamCraft through the CLI

One entry point for every BeamCraft task: model members and plates, create and verify a store,
pick and resolve features, snapshot and export views. Wrap only the `beamcraft` binary. There is
no MCP server and no daemon: the CLI is the single contract, and every reference below embeds
real captured output from it as acceptance evidence.

## Build the binary

The plugin ships no binary. Build the CLI once from a BeamCraft checkout; every example calls it
as `beamcraft`.

```sh
cd <beamcraft-checkout>
cargo build --offline -p beamcraft-cli --bins
<beamcraft-checkout>/target/debug/beamcraft --version
```

## House rules

- **Units.** Millimetres and radians, right-handed, Z up. Never convert inside a CLI argument.
- **Real runs only.** Run the real binary for every step and quote the JSON you captured. Never
  invent, paraphrase or simulate output. "It works" means you captured the JSON.
- **Never crashes.** The CLI does not panic on input. Every refusal is one JSON object with a
  stable `code` and `message` and exit code 2; treat it as the normal error channel and read it.
- **`--op-id` on every write.** Repeat the same id on a retry: the same id with the same
  arguments replays; the same id with different arguments is refused (`OP_ID_REUSE`).
- **Exit codes.** 0 success, 2 input or validation refusal, 3 backend unavailable,
  4 outcome unknown, 124 deadline, 130 cancelled.

## References - follow the one the task needs

- `reference/store.md` - `project-create`, `revision-open`, `plate-inspect`: create a store,
  verify its integrity, list what it holds.
- `reference/model.md` - `define-profile`, `define-material`, `create-member`, `create-plate`,
  `update-element`: publish definitions and model or edit elements.
- `reference/pick.md` - `pick`, `feature-list`, `feature-lookup`, `snap`: resolve coordinates,
  feature addresses and snaps.
- `reference/view.md` - `view-snapshot`, `view-export`: snapshot a scene, export a view bundle,
  open the native viewport.

## Apply it to the request

1. Build the `beamcraft` binary if it is not built (the command is above).
2. Ensure a store exists - `reference/store.md` shows `project-create` - and read the reference
   file for the task.
3. Run the real binary for every step; quote only captured JSON as evidence, and prove a write
   by inspecting afterwards (element ids, the new revision `sequence`, quantities).
