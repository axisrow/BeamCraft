# U12: representation levels

Date: 2026-11-03. Status: **delivered slice**. Upstream issue [#484](https://github.com/Apibim-com/BeamCraft/issues/484),
owner decision 2026-11-03: bring the CLI to 100%). Depends on U09 and V02 (delivered
historically); U10 payload loading is PostMVP and is not pulled in.

## What this changes

The derived layer authors the representation levels of a committed steel part beside the full
body, so a viewer can switch the standard `purpose` and every non-full level is marked inexact
(owner decision of 2026-10-16: own element representations use the standard `purpose` —
Full `render`, Simplified `proxy`, Axis `guide`; engineering reads Full only):

| Level | Prim | `purpose` | Body |
|-------|------|-----------|------|
| Full | `<part>/Geometry` | `render` | cuts and holes applied; the quantity record lives here and only here |
| Reduced | `<part>/Reduced` | `default` | cuts kept, holes dropped |
| Simplified | `<part>/Proxy` | `proxy` | no cuts, no holes (the gross body) |
| Axis | `<part>/Axis` | `guide` | the member's element-local axis points, one linear `BasisCurves` polyline |

`Reduced` and `Proxy` are ordinary `Mesh` prims in the same encoding as `Geometry` (points,
face arrays, `extent`, uniform `normals`, their own `bim:shapeKey`). Every level except the
full one carries `bim:exact = false`; the full body stores nothing, `true` being the default.
A level never carries a quantity record: quantities, BOM schedules and snapping read the full
body only, and the marker tells an external reader the same.

## Where the bodies come from

Each level body is one extra kernel evaluation of the same authored record with the
display-only operations cleared (holes for `Reduced`; holes and cuts for `Proxy`). The
evaluation runs per mesh change (`part_mesh_change`), is bounded by the same caps as the full
mesh, and can never refuse a command: a level the kernel does not admit, or one whose body
repeats another level, is simply not authored.

A level prim is authored only where its body differs from the levels it would otherwise
repeat. A plate without holes and cuts carries only `Geometry`; a holed, uncut plate carries
`Reduced` (which equals the gross body, so `Proxy` adds nothing); a holed and cut plate
carries both. This keeps the layer bytes within the recorded budgets — plain USD has no
payloads (U10, PostMVP) to deduplicate identical bodies across levels.

A member always authors its `Axis` guide from `bim:axisPointsMm`. A plate has no axis data in
storage and authors none. A mesh change replaces the level set as a whole: a level the new
body does not carry is erased wholesale, never left stale beside the fresh full mesh. The
`prims` and `values` model-profile ceilings were raised to six prims and 128 fields per
element for the same 65,536-element view cap; every other ceiling is unchanged.

## Deliberate limits, recorded so they are decisions and not holes

- **Lofted plates (A11) author no levels.** Their authored record is not a re-evaluable plate
  shape in the engine facade this slice; their full body remains the only geometry.
- **Plates have no Axis.** Storage has no plate axis; inventing one (a centroid marker, a
  mid-plane outline) is a later decision.
- **No payload or instancing (U10/U11).** Levels are plain prims; the payload move is the
  PostMVP slice that owns heavy-geometry loading.
- **No view or scene wiring.** View settings, the snapshot scene and `presentation.rs`
  continue to load the full body only; switching representations in a viewport is view state
  (the owner decision binds it to saved views), owned by the views slices, not by storage.
- **No CLI listing of levels.** The levels are USD content any viewer reads; a reporting
  surface rides with the views work that consumes them. The acceptance is verified through
  `create-plate`/`cut-by-plane`/`create-member` and the saved layer text
  (`apps/beamcraft-cli/tests/representation_levels.rs`) plus the layer-engine authoring
  regressions in `crates/steel/tests/layer_authored.rs`.
- An axis guide is a display curve: float32 points, no shape key, no engineering record, and
  the structural reader refuses an `Axis` under a plate or under anything but a beam or a
  column.
