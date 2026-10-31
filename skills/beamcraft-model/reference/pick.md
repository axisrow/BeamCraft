# Resolve features with the BeamCraft CLI

The feature commands (`list-feature`, `lookup-feature`, `element-edges`) are read-only engine queries over the verified head's view snapshot. There is
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

## Apply it to the request

Resolve the coordinate or element as described in the request.

1. Build the `beamcraft` binary if it is not built (the command is above).
2. Use `list-feature`/`lookup-feature` for semantic feature addresses. There is no `pick` or
   `snap` command: give coordinates and ids directly.
3. Quote only captured JSON as evidence, and remember that feature ids exist for canonical box
   bodies only.
