---
name: view
description: Snapshot and export a BeamCraft view through the beamcraft-cli (view-snapshot, view-export), and open the model in the native viewport app. Use when the user asks for a view snapshot of a scene or store, for a view-export bundle for the native BeamCraft viewport, or to open the model in the viewport.
argument-hint: [snapshot or export, and where]
---

# Snapshot and export views with the BeamCraft CLI

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
- **Exit codes.** 0 success, 2 input or validation refusal, 3 backend unavailable,
  4 outcome unknown, 124 deadline, 130 cancelled.

## 1. Snapshot a synthetic scene (no store)

`view-snapshot` needs no store. It builds a synthetic plate scene with the real kernel
(`--plates 1..500`; every fifth element is labelled `support`) and prints one JSON object with
the pin, the snapshot token, the projected elements with instance slots and world bounds, and
the meshes a view must load:

```sh
beamcraft view-snapshot --plates 6
```

Captured output (trimmed):

```json
{"all_exact":true,"bounds":{"max":[895,85,16],"min":[0,0,0]},"element_count":6,
 "elements":[{"exact":true,"first_triangle":0,"id":"01990000-0000-7000-8000-000000000000",
   "instance":0,"kind":"plate","mesh_digest":"086d875a19a3aacd6a5c4dcd601aba1b2229640ec5f709da5095e4b0b6f63ece",
   "placement_mm":[0,0,0],"representation":"full","triangle_count":12,
   "world_bounds":{"max":[65,70,6],"min":[0,0,0]}}, "..."],
 "outcome":"view_snapshot","pin":{"generation":"1","revision":"b596f002e5ce9a09e5ab581953323894324deaa0d196e94f306cbf4120"...}}
```

`--hide plate,support`, `--representation full|simplified`, `--stand-in-simplified true|false`
and `--select ID,ID` change the picture; the snapshot token does not change with hiding or
representation. `beamcraft view-changes` scripts edits and prints the resulting delta (see
`apps/beamcraft-cli/README.md`, V02).

## 2. Export a store's view bundle

`view-export` writes the verified head's pinned view snapshot as one engine bundle - the input
of the native viewport. With `--out FILE` the file is created new and a JSON summary is printed;
with `--out -` the raw bundle goes to standard output instead:

```sh
beamcraft view-export --store STORE --out FILE.bundle
```

Captured run (the file appeared next to it):

```json
{"bytes":"1519","elements":"3","meshes":"3","outcome":"view_exported",
 "revision_id":"01a170f3-3fc0-73d9-84c4-fd3c7c20cc5f",
 "token":"vs1-d490f67825c7be9b6b9fdd2c9eb61737853bb5acb7570c02f97be57de78034db"}
```

```text
-rw-r--r-- 1 user wheel 1519 Oct 25 10:46 view.bundle
```

The destination is never overwritten: an existing `--out FILE` is a refusal, and the file is
installed only after a complete write. `--with-definitions true` and `--with-element UUID` frame
definition and element replies into the bundle, but only together with `--out -`.

## 3. Open the native viewport

The desktop app shows the accepted head of a store in a real 3D window. Build the CLI and the
app from the same checkout - the app finds the CLI next to its own executable unless `--cli`
names it - and launch the app in the background with a log file:

```sh
cd <beamcraft-checkout>
cargo build --offline -p beamcraft-cli -p beamcraft-ui --bins
target/debug/beamcraft-ui --store ROOT > /tmp/bc-ui.log 2>&1 &
```

Prove the launch by the living process and a clean log. In the window, `R` reloads the store's
head, `F` fits the view, a click selects.
What the picture shows is accepted by the person looking at it: launch evidence is not visual
acceptance.

## 4. Prove a workflow with evidence

- snapshot: `outcome` is `view_snapshot` with the element count you asked for;
- export: `outcome` is `view_exported`, and the file exists with the reported `bytes`.

A view-export bundle plus `plate-inspect` output is the complete acceptance evidence for the
"show the model headlessly" workflow. No mesh bytes are in the snapshot JSON; a consumer reads
blobs by digest through the engine API.

## Apply it to the request

Produce the BeamCraft view as described in the request - `view-snapshot` for a synthetic
scene without a store, `view-export` for the verified head of a store, §3 to open the native
viewport on it.

1. Build the `beamcraft` binary if it is not built (the command is above).
2. Quote only captured JSON as evidence (§4); for an export, confirm the file exists with the
   reported `bytes`.
