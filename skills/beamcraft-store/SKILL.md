---
name: beamcraft-store
description: Create, open and verify a BeamCraft store through the beamcraft-cli (project-create, revision-open, plate-inspect). Use when the user asks to create a BeamCraft project/store, check that a store is intact, or list what a store contains.
---

# Open and verify a BeamCraft store

A BeamCraft store is a directory of verified USD revisions. Everything goes through the
`beamcraft` binary; there is no server and nothing to configure.

## Build the binary

The plugin ships no binary. Build the CLI once from a BeamCraft checkout; every example below
calls it as `beamcraft`.

```sh
cd <beamcraft-checkout>
cargo build --offline -p beamcraft-cli --bins
<beamcraft-checkout>/target/debug/beamcraft --version
```

## House rules

- **Units.** Millimetres and radians, right-handed, Z up.
- **Real runs only.** Run the real binary for every step and quote the JSON you captured. Never
  invent, paraphrase or simulate output.
- **Never crashes.** Every refusal is one JSON object with a stable `code` and `message` and
  exit code 2; treat it as the normal error channel.
- **Reads are safe.** `revision-open` and `plate-inspect` are read-only and take no `--op-id`.
- **Exit codes.** 0 success, 2 input or validation refusal, 3 backend unavailable,
  4 outcome unknown, 124 deadline, 130 cancelled.

## 1. Create a store

`project-create` makes the folder `DIR/NAME` and commits an empty first revision into it. That
folder is the `--store ROOT` of every other command.

```sh
beamcraft project-create --parent DIR --name demo-store
```

Captured output (trimmed):

```json
{"actor_id":"01a170ef-ea50-7060-8bbf-49a2080a0dab","outcome":"project_created",
 "project":{"created_ms":1792896133713,"name":"demo-store",
  "path":"/tmp/bc-plugin-demo/demo-store","project_id":"01a170ef-ea51-7482-a219-459443501b53",
  "revision":{"revision_id":"01a170ef-ea51-7548-aa68-9971fe8d7071","sequence":"1"}},
 "recent_recorded":true}
```

The first run also writes a settings profile (the `actor_id` above) that later commands reuse as
the author when `--actor-id` is not given. Refused when the folder exists.

## 2. Verify a store

`revision-open` verifies the root pointer, its manifest and every layer blob, and prints the
head revision:

```sh
beamcraft revision-open --store STORE
```

Captured output (trimmed):

```json
{"index":{"prim_count":"14","structure_digest":"f6b68369e4ca4ef4b9d3f49080b42fecb3d0148e177fb3a9aecb905b2f758f82"},
 "layers":[{"digest":"58da2630463f31b14ede9386a26e663705083a9f40ff213aa5e5cd517080dc53",
   "name":"authored.usda","role":"authored","size":"5011"}, "..." ],
 "outcome":"revision_verified","project_id":"01a170ef-ea51-7482-a219-459443501b53",
 "revision_id":"01a170f3-3fc0-73d9-84c4-fd3c7c20cc5f","sequence":"6","workspace_generation":"6"}
```

`outcome` is `revision_verified` (exit 0) or a refusal such as `INTEGRITY_ERROR` (exit 2). A
truncated, corrupt or hostile store is an ordinary refusal, never a crash.

## 3. List what the store holds

`plate-inspect` opens the head revision with full verification and lists plates and members
(counts first, then every element with its placement, quantities and signatures):

```sh
beamcraft plate-inspect --store STORE [--element UUID]
```

Captured output for a store with one rotated beam, one straight beam and one plate (trimmed to
the first member; the same object lists `plate_count`, `plates` and `member_count`, `members`):

```json
{"member_count":2,"members":[{"axis_points_mm":[[0,0,0],[6000,0,0]],
  "element_id":"01a170f2-6df7-72f9-903d-f5cc74956a9b","kind":"beam","material_name":"S355",
  "name":"B1","placement":{"orientation_xyzw":[0,0,0.7071067811865476,0.7071067811865476],
  "translation_mm":[0,0,0]},"profile_name":"RECT 200x300",
  "quantities":{"mass":{"kg":2826,"status":"current"},"volume":{"mm3":360000000,"status":"current"}},
  "section_mm":{"height":300,"width":200}, "..."], "plate_count":1, "..." }
```

`--element UUID` filters to one element and keeps the whole-revision counts. Stored volume and
mass are re-verified on open; `stored_quantities.freshness` is `current`, `stale` or `missing`,
and a stale record refuses later publication.

## 4. Prove a workflow with evidence

A claim about a store is proven by captured output, not by a description:

- creation: `project_created` with the store path;
- integrity: `revision_verified` with the head `sequence`;
- content: `plate-inspect` listing the element ids and quantities you expect.

Raw byte storage (`blob-put`/`blob-verify`) and revision recovery
(`revision-recover`/`revision-outcome`) exist as well; see `apps/beamcraft-cli/README.md`
(U05/U06/U07) before touching them.
