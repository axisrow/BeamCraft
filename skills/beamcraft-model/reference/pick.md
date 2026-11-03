# Pick and resolve features with the BeamCraft CLI

All four commands are read-only engine queries over the verified head's view snapshot. There is
no server; the `beamcraft` binary is the only contract.

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
- **An index is never an address.** Feature ids (`fid1:<element>:<kind>:<role>`) exist for
  canonical box bodies only; array positions are not addresses. Unsupported bodies (I-sections,
  holed plates, triangle meshes) refuse with `GEO_FEATURE_NOT_FOUND` or are listed as `skipped`.
- **Exit codes.** 0 success, 2 input or validation refusal, 3 backend unavailable,
  4 outcome unknown, 124 deadline, 130 cancelled. An empty hit list is a normal exit 0 result.

## 1. Enumerate an element's features

```sh
beamcraft list-feature --store STORE --id ELEMENT_UUID
```

Captured output (trimmed; a canonical box has 6 faces, 12 edges, 8 vertices):

```json
{"element_id":"01a170f2-6df7-72f9-903d-f5cc74956a9b","elements":3,
 "features":[{"feature_id":"fid1:01a170f2-6df7-72f9-903d-f5cc74956a9b:member:f.x0","kind":"face"},
   "...",{"feature_id":"fid1:01a170f2-6df7-72f9-903d-f5cc74956a9b:member:v.x1.y1.z1","kind":"vertex"}]}
```

## 2. Look a feature up

```sh
beamcraft lookup-feature --store STORE --feature-id FID1_ADDRESS
```

Captured run of the far top vertex of an unplaced member (axis (0,0,0) to (3000,0,0), identity
placement). `local` is the element frame and `world` adds the placement, so the two agree here:

```json
{"element_id":"6a7b8c9d-0e1f-7a2b-8c3d-4e5f6a7b8c9d","elements":4,
 "feature_id":"fid1:6a7b8c9d-0e1f-7a2b-8c3d-4e5f6a7b8c9d:member:v.x1.y1.z1","feature_kind":"vertex",
 "local":{"position":[3000,100,150]},"local_id":"member:v.x1.y1.z1",
 "outcome":"feature","revision_id":"01a17111-d7f3-707c-a9c2-bdd8396ebcdc",
 "world":{"position":[3000,100,150]}}
```

A move preserves addresses and changes the pin and coordinates. A member's placement rotation
is baked into its authored mesh, so for a placed member `local` is that rotated mesh frame; the
translation is never in `local` - `world` is `local` plus the placement translation. Treat
`world` as the coordinate that matters and verify with a fresh run.

## 3. Pick at a coordinate

Two forms. A ray (`--origin` + `--direction`, optional `--t-max`) or a point
(`--point` + `--radius`); `--targets face,edge,vertex` selects what may hit:

```sh
beamcraft pick --store STORE --origin 1500,0,1000 --direction 0,0,-1 --targets face,edge,vertex
beamcraft pick --store STORE --point 1500,0,150 --radius 20 --targets face
```

Captured ray run against an unplaced member's top face:

```json
{"elements":4,"hits":[{"back_facing":false,"boundary":false,"distance_mm":0,
  "element_id":"6a7b8c9d-0e1f-7a2b-8c3d-4e5f6a7b8c9d",
  "feature_id":"fid1:6a7b8c9d-0e1f-7a2b-8c3d-4e5f6a7b8c9d:member:f.y1","feature_kind":"face",
  "normal_world":[0,0,1],"point_world_mm":[1500,0,150],"t_mm":850}],
 "outcome":"picked","revision_id":"01a17111-d7f3-707c-a9c2-bdd8396ebcdc","truncated":false}
```

For a member, the role letters follow the F06 member frame, not the world or mesh axes of the
placed element: `x` is width, `y` is height and `z` is the axis (the module documentation of
`crates/engine/src/member.rs`: width along `X_m`, height along `Y_m`, F06 sections 2, 4 and 6).
On this unrotated member (axis along world +X) the +height face (world +Z) is `f.y1` and the
axis end cap is `f.z1`. Reproduce the letter-to-normal map with three lookups - `f.y1` resolves
to normal `[0,0,1]`, `f.z1` to normal `[1,0,0]`, `f.x1` to normal `[0,1,0]`:

```sh
beamcraft lookup-feature --store STORE --feature-id "fid1:ELEMENT_UUID:member:f.y1"
beamcraft lookup-feature --store STORE --feature-id "fid1:ELEMENT_UUID:member:f.z1"
beamcraft lookup-feature --store STORE --feature-id "fid1:ELEMENT_UUID:member:f.x1"
```

Resolve a hit's id with `lookup-feature` instead of reading geometry out of the letter; the
frame still needs documenting in the CLI (follow-up issue #147).

The captured letter-to-normal map, quoted from real runs against the member above (all four
outputs are from the same head, unedited apart from line wrapping). The ray onto the +width
face (world normal `[0,1,0]`) returns `f.x1`, not `f.y1`:

```json
{"elements":4,"hits":[{"back_facing":false,"boundary":false,"distance_mm":0,
  "element_id":"6a7b8c9d-0e1f-7a2b-8c3d-4e5f6a7b8c9d",
  "feature_id":"fid1:6a7b8c9d-0e1f-7a2b-8c3d-4e5f6a7b8c9d:member:f.x1","feature_kind":"face",
  "normal_world":[0,0.9999999999999999,0],"point_world_mm":[1500,100,0],"t_mm":900}],
 "outcome":"picked","revision_id":"01a17111-d7f3-707c-a9c2-bdd8396ebcdc","truncated":false}
```

```json
{"feature_id":"fid1:6a7b8c9d-0e1f-7a2b-8c3d-4e5f6a7b8c9d:member:f.x1",
 "local":{"corners":[[0,100,-150],[0,100,150],[3000,100,150],[3000,100,-150]],
  "normal":[0,0.9999999999999999,0],"vertex_ids":["fid1:6a7b8c9d-0e1f-7a2b-8c3d-4e5f6a7b8c9d:member:v.x1.y0.z0",
  "fid1:6a7b8c9d-0e1f-7a2b-8c3d-4e5f6a7b8c9d:member:v.x1.y1.z0","fid1:6a7b8c9d-0e1f-7a2b-8c3d-4e5f6a7b8c9d:member:v.x1.y1.z1",
  "fid1:6a7b8c9d-0e1f-7a2b-8c3d-4e5f6a7b8c9d:member:v.x1.y0.z1"]},"local_id":"member:f.x1","outcome":"feature",
 "world":{"corners":[[0,100,-150],[0,100,150],[3000,100,150],[3000,100,-150]],
  "normal":[0,0.9999999999999999,0]}}
```

```json
{"feature_id":"fid1:6a7b8c9d-0e1f-7a2b-8c3d-4e5f6a7b8c9d:member:f.y1",
 "local":{"corners":[[0,-100,150],[3000,-100,150],[3000,100,150],[0,100,150]],
  "normal":[0,0,1],"vertex_ids":["fid1:6a7b8c9d-0e1f-7a2b-8c3d-4e5f6a7b8c9d:member:v.x0.y1.z0",
  "fid1:6a7b8c9d-0e1f-7a2b-8c3d-4e5f6a7b8c9d:member:v.x0.y1.z1","fid1:6a7b8c9d-0e1f-7a2b-8c3d-4e5f6a7b8c9d:member:v.x1.y1.z1",
  "fid1:6a7b8c9d-0e1f-7a2b-8c3d-4e5f6a7b8c9d:member:v.x1.y1.z0"]},"local_id":"member:f.y1","outcome":"feature",
 "world":{"corners":[[0,-100,150],[3000,-100,150],[3000,100,150],[0,100,150]],"normal":[0,0,1]}}
```

```json
{"feature_id":"fid1:6a7b8c9d-0e1f-7a2b-8c3d-4e5f6a7b8c9d:member:f.z1",
 "local":{"corners":[[3000,-100,-150],[3000,100,-150],[3000,100,150],[3000,-100,150]],
  "normal":[1,0,0],"vertex_ids":["fid1:6a7b8c9d-0e1f-7a2b-8c3d-4e5f6a7b8c9d:member:v.x0.y0.z1",
  "fid1:6a7b8c9d-0e1f-7a2b-8c3d-4e5f6a7b8c9d:member:v.x1.y0.z1","fid1:6a7b8c9d-0e1f-7a2b-8c3d-4e5f6a7b8c9d:member:v.x1.y1.z1",
  "fid1:6a7b8c9d-0e1f-7a2b-8c3d-4e5f6a7b8c9d:member:v.x0.y1.z1"]},"local_id":"member:f.z1","outcome":"feature",
 "world":{"corners":[[3000,-100,-150],[3000,100,-150],[3000,100,150],[3000,-100,150]],"normal":[1,0,0]}}
```

Note the self-consistency in the captures: each face's `vertex_ids` carry the same letter as
the face (`f.z1` holds the four `v.*.z1` vertices, `f.x1` the four `v.x1.*`), and those
vertices sit on the world axes the section frame predicts - `v.x1.*` at world `y = +100`
(width), `v.*.z1` at world `x = 3000` (axis end).

Pass `--radius` for the point form (and `--include-backfaces true` if needed). A point inside a
solid or a ray into space is a normal, empty result:

```json
{"elements":3,"hits":[],"outcome":"picked","revision_id":"01a170f3-3fc0-73d9-84c4-fd3c7c20cc5f","truncated":false}
```

## 4. Snap to a point, axis or vertex

```sh
beamcraft snap --store STORE --point X,Y,Z [--radius MM] [--kinds vertex,axis_end,axis] [--max-hits 1..64]
```

Captured run (14 mm off the rotated member's axis, explicit radius; the axis candidate is the
nearest point on the member axis, here mid-span):

```json
{"candidates":[{"axis_parameter":0.5,"distance_mm":14.142135623731424,
  "element_id":"01a170f2-6df7-72f9-903d-f5cc74956a9b","feature_id":null,"kind":"axis",
  "point_world_mm":[-0.0000000000006679101716144942,3000.0000000000005,0]}],
 "elements":3,"outcome":"snapped","revision_id":"01a170f4-e90f-77f0-a8ff-f08edb824d32","truncated":false}
```

Pass `--radius` explicitly: with the default the same point 14 mm away found nothing.

## Apply it to the request

Resolve the coordinate or element as described in the request.

1. Build the `beamcraft` binary if it is not built (the command is above).
2. Use `list-feature`/`lookup-feature` for semantic feature addresses, `pick` for a ray or a
   point, `snap` for the nearest vertex, axis end or axis point (pass `--radius`).
3. Quote only captured JSON as evidence; an empty hit list is a normal result, and feature ids
   exist for canonical box bodies only.
