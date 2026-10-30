# V15 cut plane

Per-window clip planes and a work-area box, the V15 slice. The cut is a display filter: it
hides part of the model in one window's image and never changes the model, its revisions or
any measurement. A later saved-views slice (V12) can store this record per named view; the
record type lives in the engine and knows nothing about view registries.

## Result

A cut is `beamcraft_engine::view_clipping::Clipping` (`crates/engine/src/view_clipping.rs`):
at most [`MAX_PLANES = 8`] user half-spaces plus an optional `WorkArea` box. Every plane is a
point (millimetres) and a nonzero normal, normalized on admission; the visible side faces the
normal. `effective()` expands the record to at most [`MAX_EFFECTIVE = 16`] half-spaces (user
planes, then the box's six inward faces). Construction is validated: finite numbers, nonzero
normals, `min <= max` per box axis, the model coordinate cap.

`visible_elements(&ViewSnapshot, &[ClipPlane])` is the conservative read-time mask: a body is
hidden only when its whole world bounds lie beyond a plane (with a relative epsilon), so
partially intersecting bodies keep their original meshes. The snapshot is only read — token,
bounds and entries are unchanged, and quantities never see the cut.

## Headless command

`beamcraft clip-view --plates 1..500 (--cut px,py,pz,nx,ny,nz;... | --area
minx,miny,minz,maxx,maxy,maxz) [--stand-in-simplified true|false] [--revision SHA256]
[--generation U64]` builds the synthetic scene, reports the effective planes and the visible
ids, and re-builds the scene after masking to assert `model_unchanged` (the token of the
rebuilt snapshot equals the masked one; the pin carries no cut). The snapshot token equals the
uncut `snapshot-view` token of the same pin: a cut is not revision data. Rejections carry the
F04 code and the view wire code like every view command.

## Renderer clipping

The window applies the cut as a fragment discard. `viewport.wgsl` grew a second uniform,
`Clip` (a plane count plus up to 16 half-spaces as `(normal.xyz, -normal·point)`), bound at
group 0, binding 1, fragment stage; every fragment entry point calls `cut(o.world)` and a
fragment outside any half-space is discarded, so bodies are cut through smoothly in the image
without touching meshes or uploads. `beamcraft_ui::cut_plane::ClipGpu` packs the record into
the uniform bytes, offsetting plane points by the scene origin (the shader's world is the
scene re-centred on `origin_mm`; cut coordinates stay model millimetres). The ghost placement
preview draws through the same shaders, so a cut covers it too.

## Window controls

The View tab has a Cut plane tool; the inspector shows `beamcraft_ui::cut_plane::Form`: one
user plane stated as an axis plane with offset (XY/XZ/YZ) or a point and normal, an optional
work-area box, and Apply/Remove buttons. Applying fills the window's `cut` state and the next
frame redraws; removing restores the whole model. The panel reports the applied plane count
and the effective half-space count. The state is per window and session-only until saved
views arrive; Esc and reloads do not clear it, and switching tools does not touch it.

## Non-destruction evidence

`visible_elements` leaves the snapshot byte-identical (asserted in the engine test), the CLI
rebuild asserts `model_unchanged`, and the UI render test reads the frame back: an uncut row
of plates, the same row cut at the centre (fewer lit pixels, straddling bodies kept), an empty
work area (nothing), and the rebuilt snapshot token equal to the displayed one. Display and
engine mask agree on the same cut.
