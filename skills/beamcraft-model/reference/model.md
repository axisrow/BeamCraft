# Model members and plates with the BeamCraft CLI

Wrap only the `beamcraft` binary. There is no MCP server and no daemon: the CLI is the single
contract, and every claim below was captured from real runs of it.

## Build the binary

The plugin ships no binary. Build the CLI once from a BeamCraft checkout; every example below
calls it as `beamcraft`.

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
  arguments replays (`member_replayed`, `plate_replayed`, `element_replayed`, no new revision);
  the same id with different arguments is refused (`OP_ID_REUSE`).
- **Exit codes.** 0 success, 2 input or validation refusal, 3 backend unavailable,
  4 outcome unknown, 124 deadline, 130 cancelled.

## Generate ids

Every id (`--op-id`, `--new-id`, `--revision-id`, `--actor-id`, ...) must be a canonical
lowercase UUIDv7. Anything else is refused with `INVALID_ID` (exit 2) - `uuidgen` alone (v4) is
not accepted. Generate a fresh id for every use; never reuse one value for two different ids:

```sh
uuidgen | tr 'A-Z' 'a-z' | awk '{print substr($1,1,14) "7" substr($1,16,4) "b" substr($1,21)}'
```

Follow the sibling `store.md` reference if no store exists yet. A definition must be published
before an element can pin it.

## 1. Publish definitions (once per store)

```sh
beamcraft define-profile --store STORE --width 200 --height 300 --name "RECT 200x300"
beamcraft define-material --store STORE --density 7850 --name "S355"
```

Captured `define-profile` output (trimmed): the `digest` is what `create-member` pins.

```json
{"definition":{"digest":"c560510a9be9957df194b0c6f8d482e6d132755e8efb3992ed759d04e959d0ad",
  "display_name":"RECT 200x300","height_mm":300,"kind":"rectangle","units":"mm","width_mm":200},
 "outcome":"definition_created", "revision":{"revision_id":"01a170f0-01ce-775a-9112-5230d475c650",
  "sequence":"2"}}
```

A definition already in the head is not rewritten. `--width/--height` is a rectangle section;
`--family i_shape|channel|angle|rhs|chs|...` builds parametric sections (see
`apps/beamcraft-cli/README.md`, P02). `--material NAME`/`--profile NAME` select by display name
or designation instead of a digest; an ambiguous name is refused with every candidate listed,
never guessed.

## 2. Create a member

```sh
beamcraft create-member --store STORE --start X,Y,Z --end X,Y,Z --profile-digest SHA256 \
  [--kind beam|column] [--roll RAD] [--material-digest SHA256] [--name TEXT] \
  [--translation X,Y,Z] [--orientation-xyzw X,Y,Z,W] [--new-id UUIDv7] [--op-id UUIDv7]
```

A rotation is `--orientation-xyzw X,Y,Z,W`: a placement quaternion in x,y,z,w order, normalized
and validated. The default is the identity `0,0,0,1`. This captured run creates a 6 m beam
rotated 90 degrees about Z (`0,0,0.7071067811865476,0.7071067811865476`):

```sh
beamcraft create-member --store STORE --start 0,0,0 --end 6000,0,0 \
  --profile-digest c560510a9be9957df194b0c6f8d482e6d132755e8efb3992ed759d04e959d0ad \
  --material-digest 3751bf4b3e9a6db1fe5acdb363e5d2ff91111f757baf5c83a90409a0a1be0b25 \
  --orientation-xyzw 0,0,0.7071067811865476,0.7071067811865476 --name "B1" --op-id UUIDv7
```

Captured output (trimmed):

```json
{"member":{"axis_points_mm":[[0,0,0],[6000,0,0]],"element_id":"01a170f2-6df7-72f9-903d-f5cc74956a9b",
  "kind":"beam","mesh_digest":"24bcb94ab5f533abbb9f9b3c150546cd935578c83f418ab60592e792fabb0eb0",
  "name":"B1","placement":{"orientation_xyzw":[0,0,0.7071067811865476,0.7071067811865476],
  "translation_mm":[0,0,0]},"section_mm":{"height":300,"width":200},"triangles":12,"vertices":8,
  "quantities":{"mass":{"kg":2826,"status":"current"},"volume":{"mm3":360000000,"status":"current"}}},
 "op_id":"f8705155-efee-7c47-bc76-c89364187cfc","outcome":"member_created",
 "revision":{"revision_id":"01a170f2-6df8-7551-8693-dc64683f3792","sequence":"4"}}
```

The rotation is rigid: the local mesh and quantities are unchanged; only the placement turns
(world vertex v.x1.y1.z1 of this member sits at (-100, 6000, 150), verified below by
`feature-lookup`). Axis points define the local layout: an axis along +X gives width along Y and
height along Z; `--roll` rolls the section about the axis before the quaternion maps it.

Acceptance evidence of the refusals (exit 2 in both cases):

```json
{"code":"INVALID_ARGUMENT","message":"quaternion squared norm is not within 1e-6 of one","outcome":"rejected"}
```
from `--orientation-xyzw 0,0,1,1`, and

```json
{"code":"NOT_FOUND","message":"profile definition is not in the model","outcome":"rejected"}
```
from an unknown `--profile-digest`.

## 3. Create a plate

```sh
beamcraft create-plate --store STORE (--width MM --length MM | --contour X,Y[,Z];...) \
  --thickness MM [--name TEXT] [--translation X,Y,Z] [--material-digest SHA256] \
  [--hole-rect X,Y,W,H[;...]] [--hole-round X,Y,D[;...]] [--new-id UUIDv7] [--op-id UUIDv7]
```

Captured output (trimmed):

```json
{"op_id":"3b1f8a20-9c4d-7e2a-b1f0-5d6e7a8b9c0d","outcome":"plate_created",
 "plate":{"dimensions_mm":{"length":600,"thickness":12,"width":400},
  "element_id":"01a170f3-3fbf-7575-bc79-c912f587633f","hole_count":0,"kind":"plate","name":"PL1",
  "placement":{"orientation_xyzw":[0,0,0,1],"translation_mm":[0,6000,0]},
  "quantities":{"mass":{"kg":22.608,"status":"current"},"volume":{"mm3":2880000,"status":"current"}}},
 "revision":{"revision_id":"01a170f3-3fc0-73d9-84c4-fd3c7c20cc5f","sequence":"6"}}
```

## 4. Replay is the retry contract

Rerunning the exact same `create-plate`/`create-member` command with the same `--op-id` (and the
same `--new-id` if given) must produce the original result again, writing nothing:

```json
{"op_id":"12018679-aebd-7b5c-bf62-83290b85bfec","outcome":"member_replayed",
 "revision":{"revision_id":"01a170f2-d5a0-72ea-b212-d4cbf8b3cd7a","sequence":"5"}}
```

The same id with **different** arguments is refused:

```json
{"code":"OP_ID_REUSE","message":"operation id was accepted with different content","outcome":"rejected"}
```

(That refusal is what a retry without `--new-id` produces: the generated element id would differ.)
So: one fresh `--op-id` per logical operation, repeated byte for byte on every retry.

## 5. Update an element

```sh
beamcraft update-element --store STORE --id UUID [--name TEXT] [--material-digest SHA256|none]
  [--thickness MM] [--translation X,Y,Z] [--orientation-xyzw X,Y,Z,W|none] [--op-id UUIDv7]
```

`--translation` is the **absolute** placement translation (rotation kept); a move rewrites only
the placement and never rebuilds the body. Captured rename + move of the second member
(trimmed): note `mesh_digest` is unchanged and only `authored.usda` changed.

```json
{"changed_layers":["authored.usda"],
 "effect":{"level":false,"material":false,"name":true,"orientation":false,"placement":true,"shape":false},
 "element":{"element_id":"12018679-aebd-7b5c-bf62-83290b85bfec","name":"B2-gable",
  "placement":{"orientation_xyzw":[0,0,0,1],"translation_mm":[6000,4000,0]},
  "mesh_digest":"85ca6b03a0dfbf2f834683583017debaeb413f3334740bab06435d71a765c160"},
 "op_id":"5d3e8c42-bf6a-7d4c-9b32-7f8e9a0b1c2d","outcome":"element_updated",
 "revision":{"revision_id":"01a170f4-e90f-77f0-a8ff-f08edb824d32","sequence":"7"}}
```

An empty patch is `INVALID_ARGUMENT`. On a member, `--orientation-xyzw` replaces the stored
quaternion (absolute; `none` resets to identity) and rebuilds the rotated body; `--thickness` is
plates only.

## 6. Inspect what you built

`beamcraft plate-inspect --store STORE` opens and verifies the head revision and lists plates
and members with quantities; `--element UUID` lists only that element. Full documentation and
captured output: the sibling `store.md` reference. Always prove a modelling change by inspecting
afterwards and reporting element ids, the new `sequence` and the quantities from the real output.

`copy-element` and `delete-element` also exist (see `apps/beamcraft-cli/README.md`, P05/P08).

## Gaps in the CLI today (do not work around)

- No CLI command generates a UUIDv7; callers need the shell recipe above. A future
  `beamcraft id-new` would remove it.

## Apply it to the request

Model BeamCraft elements exactly as described in the request.

1. Build the `beamcraft` binary if it is not built (the command is above).
2. Ensure a store exists - follow the sibling `store.md` reference when the user has not named one.
3. Run the real binary for every step; quote only captured JSON as evidence.
4. Prove the result: run `plate-inspect` afterwards and report the element ids, the new
   revision `sequence` and the quantities from the real output.
