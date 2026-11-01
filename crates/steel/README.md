# steel

Structural steel: the discipline for plates, members, holes, bends, welds, bolts and fasteners.

The crate holds three parts, one for each layer it extends. The platform crates below it
(`model`, `kernel`, `engine`) do not depend on it.

| Module | What it holds |
|---|---|
| `model` | The commands that create and edit plates, members, holes, bends, welds and bolts and their admission (`command`); copy, array and update commands; section families (`section`); fastener definitions (`fastener`); profile definitions (`definition`); part instances (`instance`). |
| `kernel` | The steel bodies: plate contours, straight members from a section, ruled and lofted plates, fasteners, member holes, sweeps. |
| `engine` | The plate and member engines (`command`, `member`), cuts, corners, bends, lofted plates, fabrication, welds, bolts, edits (`update`), copies, transforms, quantities, the committed-plate reader and the snap axes of members. |

What every discipline shares stays in the platform: identities, digests, error codes, limits,
grids, levels, assemblies, the box primitive and its demo scenes (`kernel::build`,
`engine::synthetic`), queries, scenes, drawings, views, filters and schedules.

`layer-engine` and the CLI still branch on the kind of element, so they depend on this crate. The
element interface that removes that is tracked in #795; until then the crate is kind `native` in
`docs/contracts/f08-module-boundaries-v0.1.md`, and it becomes `discipline` when the interface
lands. How the code was separated is in `docs/design/platform-and-steel.md`.
