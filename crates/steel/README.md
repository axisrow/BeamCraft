# steel

Structural steel: the discipline for plates, members, holes, bends, welds, bolts and fasteners.

The crate holds four parts, one for each layer it extends. The platform crates
(`model`, `kernel`, `engine`, `layer-engine`) do not depend on it.

| Module | What it holds |
|---|---|
| `model` | The commands that create and edit plates, members, holes, bends, welds and bolts and their admission (`command`); copy, array and update commands; section families (`section`); fastener definitions (`fastener`); profile definitions (`definition`); part instances (`instance`). |
| `kernel` | The steel bodies: plate contours, straight members from a section, ruled and lofted plates, fasteners, member holes, sweeps. |
| `engine` | The plate and member engines (`command`, `member`), cuts, corners, bends, lofted plates, fabrication, welds, bolts, edits (`update`), copies, transforms, quantities, the committed-plate reader, the snap axes of members, and `part`: one committed part (`PartRecord`) whose copy, array, patch, evaluation and quantity record choose the plate or member operation, so an adapter never names the kind. |
| `layer` | The steel half of the layer engine: the hooks behind `Shape::Discipline` and `Change::Discipline` (kind, attributes, bounds and the writes of a steel element, and the steel edits), read-back validation and definition read-back through the installable discipline registry (`authored::register`, which the apps fill by calling `steel::layer::install()` at start), and the facade of free constructors the apps build steel shapes and changes with. |

The integration suites are modules of one test binary, `tests/steel.rs`. Among them,
`g11_accuracy` runs the plate, member and fastener builders against the G11 accuracy fixtures
(`docs/fixtures/g11`, exact rational references, the F06 comparison rule).

What every discipline shares stays in the platform: identities, digests, error codes, limits,
grids, levels, assemblies, the box primitive and its demo scenes (`kernel::build`,
`engine::synthetic`), queries, scenes, drawings, views, filters and schedules.

The dependency reversal (#795) landed: the platform names no steel type, and no platform crate
depends on this crate. `layer-engine` authors and validates elements through the discipline
hooks and answers its read-back questions through the registry, so only the apps depend on
steel. Stored layers are byte-identical — the `steel:*` attributes, the prim paths and the
schema descriptor are unchanged — so stores written before the reversal read back. How the code
was separated is in `docs/design/platform-and-steel.md`.
