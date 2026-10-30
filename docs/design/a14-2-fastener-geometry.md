# A14.2 fastener geometry

Date: 2026-10-30. Status: **experimental slice; A14 acceptance (holes in all parts, conflicts
as in A05) stays open and is claimed by A14.3 and A14.4.**
Branch `ao/beamcraft-133/feat-a14-fastener-geometry`, based on the master of 2026-10-30.

Plan row: A14.2 / M (slice of A14, issue axisrow/BeamCraft#310), contract
`docs/contracts/f04-bolt-fastener-amendment-draft-v0.1.md` section 3. Depends on A14.1 (the
`FastenerSpec` values) and nothing else; U11 (instancing) stays open and is not needed for the
prototype itself.

## What is implemented

One fastener prototype: head, shank, optional nut and washers as separate closed meshes, built
from the kernel's own G05 extrusions — no new kernel operation:

```text
beamcraft fastener-geometry --fastener-library FILE --designation TEXT --length MM
    [--count N] [--density KG_M3] [--washer-under-head true|false] [--washer-under-nut true|false]
```

| Area | What exists |
|---|---|
| `beamcraft_kernel::fastener` | `build(spec, length, washer switches)`: the components in stack order from the bearing face (`z = 0`, the shank along +Z) — head washer, head, shank, nut, nut washer — each a sealed [`Extrusion`] with its exact volume and mesh digest, its stack offsets and, for a round part, the chord-rule provenance. A hex head and nut are extrusions of a regular hexagon across flats; the shank is an extrusion of a regular polygon of the nominal diameter; a washer is an extrusion of a polygon with a circular cavity. Round parts follow the round-hole rule of F06 A-2 D-2: chord deviation at most 0.05 mm, vertex count a multiple of four, on the kernel's deterministic trigonometry. Threads are not modelled; edges are not chamfered. The definition is re-admitted and the washer switches follow its rules, so a refused build produces nothing. |
| `beamcraft_engine::fastener` | `prototype` (the build with kernel refusals mapped to command fields) and `quantities(spec, build, count, density)`: per-component counts (`n` for every component), per-piece and group volumes, and the mass with its source — `mass_each_kg` of the definition shown as `declared` (never split into components), volume and density shown as `computed`, unknown density shown as unknown. The group size follows the A05 cap (512). |
| `beamcraft_cli` | `fastener-geometry`: resolves one library entry, builds the prototype and prints the per-component rows (mesh sizes, mesh digests, stack offsets, approximation provenance, volumes, masses), the group totals and the `mass_source` token. Reads the library, never a store. |

## Decisions

- **The group totals are the union of the components.** The shank starts at the under-head
  plane and runs through the head, so summing the component volumes would count their overlap
  twice. The per-component rows keep the volumes of their separate closed meshes; the group
  totals subtract the shank-section area over `min(head_height_mm, length_mm)` — the
  definition guarantees the head clears the shank, so the overlap is exactly that prism.
- **One prototype per group without U11.** The build returns one prototype regardless of the
  group size; `quantities` only multiplies counts, volumes and masses. A group of `n` shares
  that one mesh (identical inputs give identical sealed meshes and digests); placing it `n`
  times in the view is U11's, and A14.3 wires the placements.
- **The prototype frame is the stack itself.** The bearing face is the plane `z = 0` and the
  shank runs towards +Z, so a group command (A14.3) places the whole prototype with one rigid
  transform and the component offsets travel with it.
- **A declared mass is never split.** `mass_each_kg` is the mass of one whole fastener; per-
  component masses exist only as computed volumes times a known density. With no density the
  mass stays unknown, as for every material without a density.
- **The command reads a library, not a definition digest.** A14.3 resolves and pins the
  definition in its transaction; until then the library file is the only source, exactly as
  `show-library` reads it.

## Limits

- No group element, no placement records, no holes: A14.3 adds `CreateBoltGroup`/
  `UpdateBoltGroup` over this prototype, A14.4 the holes.
- The length is any admitted millimetre value; picking the smallest standard length (`length_
  mode = auto`) is A14.3's rule.
- `HeadKind` has only `Hex`, as in A14.1; a round head adds one outline, not a mechanism.

## Tests

`beamcraft_kernel::fastener`: hex volumes against the analytic hexagon; shank and washer
volumes against the exact polygon values and inside the chord-rule circle bound; the stack
offsets from the bearing face; a screw has no nut and no washer under a nut; the switch rules
and the length codes name their fields; identical inputs build identical prototypes;
`beamcraft_engine::fastener`: counts and volumes scale with the group size, declared mass
wins over density and is never split, computed mass is the volume times the density, an
unknown density leaves the mass unknown, the group cap and the density codes;
`tests/fastener_geometry.rs` (end to end through the binary): one prototype shared by a group
of 3 (the two washers carry the same mesh digest), five components of the washered bolt, the
analytic head volume, the chord-rule records, the declared-mass variant, a screw without a
nut, unknown designations, bad lengths and counts refused.
