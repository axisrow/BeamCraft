# A14.2 fastener geometry

Date: 2026-10-27. Status: **experimental slice; A14 acceptance (holes in all parts, conflicts as
in A05) stays open and is claimed by A14.3 and A14.4.**
Branch `feat/a14-2-fastener-geometry`, stacked on `feat/a14-bolt-groups` (A14.1).

Plan row: A14.2 / M (slice of A14, issue axisrow/BeamCraft#194), contract
`docs/contracts/f04-bolt-fastener-amendment-draft-v0.1.md` section 3. Depends on A14.1 (the
pinned definition values); U11 instancing is what makes the group cheap later, and the group
itself is A14.3.

## What is implemented

`beamcraft_engine::fastener` builds the prototype of one fastener of a pinned definition:

```text
fastener::prototype(&FastenerSpec, length_mm) -> FastenerPrototype
```

| Area | What exists |
|---|---|
| Stack | From the bearing face along `+z`: the optional washer under the head, the hex head, the shank over the chosen standard length, then — for a bolt — the washer under the nut and the nut ending at `shank end - thread_protrusion`, leaving the protrusion as bare shank beyond the nut. The standard length is the shank length under the head and covers grip, the washer under the nut, the nut and the protrusion (bolt amendment section 4); the layout is the part-free exploded stack, the grip space between the head and the washer under the nut is where the connected parts sit, and A14.3 places the group from the offsets (`length - washer - nut - protrusion` is the grip) without re-deriving it. Each [`FastenerComponent`] carries its kind, its own sealed mesh, its exact volume and its `z_start`/`z_end` offsets. A length that cannot cover the definition's own nut, washer and protrusion is refused. |
| Shapes | The head and nut are exact regular hexagons across the flats; the shank and the washer faces are regular polygons on the 0.05 mm chord rule with a vertex count that is a multiple of four (F06 A-2 D-2), floored at 8 and capped at 1020 like the kernel's own curves; the washer is the outer polygon with a round `Cavity::circle` hole. Threads are not modelled; edges are not chamfered. |
| Meshes | Every component is a separate `SolidMesh` sealed by the kernel (G05), so colour, quantities and the BOM read them one by one. The builder takes no group size: a group of `n` fasteners builds this one prototype once and places it `n` times (U11 instancing at the group), and the same definition builds byte-identical meshes (digest-checked in the tests). |
| Quantities | Exact polygon volumes per component and for the whole fastener; the count of every component kind; the mass from the definition's declared `mass_each_kg` with source `declared`, else `unknown` — the definition carries no material density and none is inferred. The `computed` source of the amendment needs a density source the definition does not have yet; it joins when the owner decides where the fastener's material density lives. |

Acceptance of the card: volumes against the analytic values hold in the unit tests (hex prisms
exact; discs and the annulus within the chord bound); a group of `n` shares one mesh holds by
construction and by the determinism test; the mass source is shown as a stable token
(`declared`/`unknown`).

## Decisions

- **No CLI command in this slice.** The amendment's command table has no A14.2 command: the
  geometry surfaces through `CreateBoltGroup` (A14.3), which stores the prototype and its
  quantities with the group. Headless-first is satisfied at the slice boundary by the engine API
  and its tests, as the card's acceptance (property-level) is.
- **Mass without a declared value stays unknown.** The amendment computes mass "from volume and
  the density of the definition's material", but a fastener has no material in the model and the
  definition carries no density field (section 2 lists none). Inventing one would break the
  no-inferred-values rule; the `Computed` source is named and reserved.
- **Stack layout is data, not baked placement.** Components are extruded locally and carry their
  offsets; the group's placement (A14.3) transforms the prototype once, not per component.

## Limits

- Round parts beyond the plain washer hole (oversize, slots, blind) are post-MVP (O-B3).
- The prototype is not stored yet: A14.3 authors the group and its prototype into the revision.

## Tests

`crates/engine/src/fastener.rs`: analytic volumes within the chord rule (amendment section 9
case 1); the stack order, offsets and the protrusion (washer 0–3, head 3–13, shank 13–73,
washer under the nut 50.2–53.2, nut 53.2–68 for the case definition at length 60 — the nut ends
at `shank end - protrusion`); a screw has no nut and a bare bolt no washers; the mass
source is declared or unknown; only a standard length builds; the prototype is deterministic, so
a group of `n` shares one mesh.
