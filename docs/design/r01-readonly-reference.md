# R01 read-only USD reference

## Update 2026-10-27: a reference is displayed, not validated

By the owner's instruction following the N08 run on a third-party Omniverse scene (#478): a
reference exists to be looked at and snapped to, so only its geometry matters and no property
needs support. A foreign (non-owned) layer is therefore admitted without being validated. The
project's own layers keep the strict profile.

For a foreign layer the reader no longer refuses, and instead leaves out of the snapshot:

- properties whose value the snapshot cannot hold (asset paths, dictionaries, list-ops, unknown
  types, numbers beyond the scalar budget, non-finite numbers); a `matrix4d` and a float `quatf`
  are now held (an array of 16 and of 4 numbers), so a placement stays readable;
- encodings and metadata of any kind (USDC value types and compression, `apiSchemas`,
  `customData`, `instanceable`, time samples), single-quoted and triple-quoted strings, relationships
  that are not defined, `interpolation` and `displayName` of another type;
- arcs the profile cannot use. Payloads, variants, inherits, specializes, relocates, sublayers
  and references to assets that were not supplied, URLs and escaping identifiers are inert: the
  composer drops the arc and composes the rest of the prim. The resolver is still a virtual byte
  map, so nothing is opened or fetched, and `check_source_path` still refuses a URL or device path
  given as the `--source` argument. What an inert arc would have contributed is absent, so a scene
  that is a root of payloads to other files shows only what its own layer holds; multi-file
  sources are not supported;
- composition diagnostics raised by a foreign layer's own arcs. A diagnostic raised by an owned
  layer, such as a wrapper whose source was not supplied, still refuses the stage;
- a prim path deeper than 16 levels (up to 128 are admitted).

The decoded-size estimate of a foreign USDC layer is 16 bytes per table entry or array scalar
instead of 1,024, so the size of a real model is bounded by its memory use and not by the old
estimate. The byte, prim, value, array and snapshot ceilings of the previous update still bind, and
a layer the upstream parser cannot decode (an 18 MB sample with a non-UTF-8 token table is one)
is `INTEGRITY_ERROR`.

Parsing and composing a foreign source now run on one short-lived scoped thread with 256 MiB of
reserved stack. The upstream text parser recurses once per escape sequence in a string literal, so
a 50 KB string full of escapes (an Omniverse baked preview) overflowed the 1 MiB stack of a
Windows main thread: an abort on untrusted input. The upstream stage is no longer retained after
the snapshot is built.

A reply of `attach-reference` and `inspect-reference` lists at most 512 foreign prims per wrapper
(`foreign_prims_truncated` says when more exist; `foreign_prim_count` is always the total): a real
scene composes tens of thousands of prims and the reply is bounded by the JSON token ceiling.

The display no longer refuses a source. `crates/commands/src/reference_view/display.rs` applies the
full USD transform stack of every ancestor (translate, scale, `rotateX/Y/Z` and the six
`rotateXYZ` orders, `orient`, `transform`, `!invert!`, `!resetXformStack!`), draws any `Mesh` as its
polygon cage whatever its `subdivisionScheme` (holes and creases are ignored), fan-triangulates a
polygon of more than four corners or a concave or non-planar quad, bakes the wrapper rotation
and the explicit units correction into the drawn vertices, and leaves out what it cannot draw (other
gprims, a mesh without usable points or faces, a surface that cannot be placed or lies beyond
the coordinate bound, anything over the 262,144-vertex and 524,288-triangle blob budgets). The
picture is an approximation, never a quantity: the R02 measurement keeps the strict
translated Cube and triangle/convex-quad profile and refuses what the display approximates, so
a drawn reference may still have no measurement. Without a units correction a source in centimetres is
drawn at one hundredth of its size; the correction stays explicit (`correct-reference-units`).

## Update 2026-10-27: reference input budgets

By the owner's instruction following the N08 external-scene check (#478), R01 now admits
sources up to **32 MiB**, matching the host blob cap. Source inspection uses `Budgets::MODEL`
instead of the original `FIRST_SLICE` profile: 262,144 prims, 1,572,864 scalars per array and the
existing decoded/snapshot ceilings. Raw USD string/token values have an explicit tightenable
64 KiB ceiling; paths, identifiers, engineering labels and the dedicated library/drawing
profiles retain their tighter limits. The complete project still has its 64 MiB and eight-layer
ceilings. The original small-input profile remains available to callers that select it explicitly.

The CLI regression covers a source over 64 KiB with 303 prims, a 2,048-value array and a
48,854-byte foreign string. It checks reopening, unchanged source bytes/digest and viewport
export. An oversized sparse source is
refused before publication and leaves the accepted revision unchanged; model admission checks
the inclusive source-size boundary. Reader tests round-trip USDA/USDC strings at their exact
text cap, reject cap-plus-one, and exercise a tighter caller-selected text budget.
This does not establish hard containment of the upstream parser or support for multi-file
scenes, payloads, variants or textures. The historical
first-slice limits below describe the original delivery and are superseded by this update.

Date: 2026-10-17. Status: **experimental vertical slice; the R01 acceptance criteria are demonstrated
for the supported profile below, and K3 is not claimed** (it also needs R02, R03 and G08).
Branch `feat/r01-readonly-reference`, based on `origin/master` at `43026e2`.

Plan row: R01 / M, depends on P01 (merged). Result: an attach/reference wrapper for a foreign USD
file, a snapshot and version of the source, and a ban on edits and overrides of foreign prims.
Acceptance: the API and CLI refuse a write to the source file and to a foreign prim; saving keeps
the foreign bytes (content-addressed, verified); the wrapper transform is stored apart from the
foreign content. Plan amendments of revision 1.3: the units and up-axis correction is an explicit
operation on our reference container, never an edit of the foreign layer; a network asset path is a
negative case refused before any resolution or I/O. Measurements are R02 and a source update is
R03; neither is here.

## What is implemented

```text
beamcraft attach-reference --store ROOT --source FILE [--name T] [--translation X,Y,Z] [--rotation-z-deg D]
  path text --model::check_source_path (no I/O)--> bounded read of FILE (read only)
  --layer engine inspect_foreign_layer--> SourcePin: SHA-256, size, format, defaultPrim, declared units
  --engine::reference::attach--> admission + footprint
  --layer engine (U02) Change::AttachReference--> wrapper in authored.usda; the foreign bytes are a
      non-owned layer of the stage, returned as Prepared.retained
  --host (U06/U07)--> blobs, verifier (U02 structure, no owned opinion on foreign content, U03 index,
      kernel re-evaluation of plates and members), root switch; the source is an `external` entry
beamcraft move-reference          --store ROOT --id UUID [--translation X,Y,Z] [--rotation-z-deg D]
beamcraft correct-reference-units --store ROOT --id UUID (--meters-per-unit F --up-axis Y|Z | --clear true)
beamcraft write-reference         --store ROOT --id UUID (--target source | --prim /Default/Path [--attribute N])
beamcraft inspect-reference       --store ROOT
```

| Area | What exists |
|---|---|
| `beamcraft_model::reference` | `check_source_path` (URL schemes and network or device paths are `PERMISSION_DENIED` on the text alone), `SourcePin` (digest, size, format, default prim, version, label, declared units), `UnitsInterpretation` (metres per unit, `UpAxis` Y or Z), commands `AttachReference`, `MoveReference`, `CorrectReferenceUnits`, `WriteForeign` with admission, caps and fingerprints. |
| `beamcraft_engine::reference` | `attach`, `move_reference`, `correct_units` (admission, footprints, fingerprints), `write_foreign` whose success type is `Infallible`, `ReferenceRecord::units_status` and `source_point_to_world` (the transform R02 must use), `Footprint::for_attach`. |
| `beamcraft_layer_engine` | `Change::AttachReference`, `Change::ReferenceUnits`; `Prepared.retained` (foreign layers, byte for byte); `authored::read_references`; `inspect_foreign_layer`; `Reader::refuse_layer_write`; the wrapper validator, including a spec-level scan of every owned layer; the index treats the wrapper as owned although its `Source` descendant is foreign; the schema descriptor lists the wrapper. |
| `beamcraft_cli` | `reference` (wiring), `reference_cmd` (the five commands), `source_file` (the bounded file read of `put-blob`, now shared), host ownership by manifest role in `plate::open_request`, retained foreign layers in `publish_revision`. |

## The shape of a wrapper

```text
/World/Steel/E_<id>             owned  kind = reference, group; label, rigid placement, source pin
  /Correction                   owned  the explicit units / up-axis interpretation (optional)
    /Source                     typeless def, references = @external/<sha256>.usda@</DefaultPrim>
      ...                       foreign: composed from the pinned layer, never written
```

`Source` carries the arc and nothing else: no type, kind, property or child of ours. A type or an
`xformOp` there would be a stronger opinion on the foreign root prim, that is an override. The
placement lives on the wrapper and the correction one level down, so moving a source or re-reading
its units never writes a foreign prim, and the foreign layer is not among the layers a save
rewrites.

## Decisions

- **Ownership is the host's choice, never the bytes'.** The revision manifest has an `external`
  role (U06). The adapter opens the stage with every `external` layer non-owned. A save keeps those
  entries (`Prepared.retained`), re-publishing a blob is a compare, and the host re-hashes every
  blob and compares its size on each open. Layer names derive from the digest
  (`external/<sha256>.usda`), so identical bytes share one layer and one blob.
- **Four independent barriers against a write.** (1) No authoring path writes below `Source`.
  (2) Every candidate and every committed revision is scanned at the spec level: in any owned layer
  the `Source` spec may hold only `specifier` and `references`, and no spec may exist below it or
  on it as a property (`FOREIGN_READ_ONLY`). (3) The composed provenance of every prim below
  `Source` must be foreign only, with the pinned digest, layer name and default prim (an owned
  opinion that composition would hide behind a stronger foreign one is caught by (2)). (4)
  `write_foreign` refuses in the engine and the layer engine's `refuse_write` /
  `refuse_layer_write` must agree for every target that exists, or the answer is
  `INTEGRITY_ERROR`.
- **Explicit units correction.** The source declares what it declares and the pin records that;
  absence is recorded as absence (USD's fallback of 0.01 m, Y up is not the project's). Reading it
  as something else is `correct-reference-units`, stored on the wrapper's `Correction` prim as the
  statement (metres per unit, up axis), the digest of the source it was made for, and the
  operations it implies: `xformOp:scale:bimUnits` = metres per unit x 1000 (omitted when 1) and
  `xformOp:rotateX:bimUpAxis` = +90 degrees for Y up. A hand-edited scale without the statement is
  refused. `units_status` says `matches_project`, `needs_correction`, `undeclared`, `corrected` or
  `corrected_differs_from_declared`. The correction is recorded with the source version so that R03
  must re-confirm it when the source changes (a statement made for another digest is
  `INTEGRITY_ERROR`).
- **Network paths.** `check_source_path` runs before the store, the file or a resolver is touched
  (the CLI runs it before opening the store root). Inside a source a URL or escaping asset
  identifier is inert (see the 2026-10-27 update): the stage resolver is a virtual byte map that
  cannot open a file or a socket.
- **Footprints.** For a reference the F04 group `shape_inputs` means the source pin and its units
  interpretation. Attach writes lifecycle, label, shape inputs and placement; a move writes
  placement and derived (world measurements); a correction writes shape inputs and derived. No
  footprint names the foreign layer: it is never a target.
- **Identity.** The wrapper is an element of kind `reference` (F10 reserves the kind) at the
  ordinary element path, so duplicate detection, the U03 index and the footprint machinery apply.
  F10 `DV-KIND-004` (authoring a reserved kind is rejected) needs an amendment that admits this
  one; F04 section 1 defers the attachment commands to R01. Proposed, not edited here.
- **Stored placement is a full rigid pose**; the CLI offers a plan rotation about Z. The
  translation-only viewport path below supports identity wrapper rotation only; storing a rotated
  reference is still valid R01 data, but exporting its view is an explicit refusal.
- **Schema pin.** The U02 descriptor lists the wrapper, so its digest changes. Layers pinned to the
  P02.2 digest contain nothing the new mapping refuses and are accepted next to the earlier
  previous mapping; the next edit rewrites the pin.

## Translation-only viewport path (issue #137)

`export-view` now includes supported foreign surfaces in the ordinary engine view bundle. The
adapter reads the already verified composed prims (no second file open or resolver); the engine's
`scene::reference_element` combines a wrapper's visible surfaces into one `reference` element and
a bounded BCMESH01 display mesh. The existing UI instancing/upload path draws it in both full and
simplified modes, and it contributes to scene counts, bounds and camera fit. No new dependency or
bundle format is needed.

Supported profile:

- `Cube` (centred at the source origin, default size 2) and polygonal `Mesh` cages made of triangles
  or convex quads planar within 1e-6 mm. Groups may be `Xform`, `Scope` or typeless. Multiple
  surfaces are combined under the wrapper identity; identical representations share a mesh.
- Ordered `xformOp:translate` operations (including suffixes) on foreign prims accumulate through
  the foreign hierarchy. Operations absent from `xformOpOrder` are inactive. Wrapper rotation must
  be identity. General rotation, scale, matrices, inverse operations and reset stacks are refused,
  not silently ignored. The R01 **explicit** units/up-axis correction is applied to local display
  vertices before wrapper translation; declared units alone still change nothing.
- Inactive or invisible subtrees contribute no surface; `inherited` visibility cannot reveal an
  invisible ancestor. Mesh handedness controls winding. Geometry uses the UI's existing flat
  shading and palette, not source materials, normals or textures.

This is a polygon-cage view, not subdivision evaluation, a closed-shell certificate or a quantity.
Subdivision schemes other than an explicit `none` are outside this profile: USD defaults an absent
`scheme` to catmullClark, so a mesh that does not declare `none` is refused rather than drawn as an
unsubdivided cage. Holes, creases, other prim types, non-default/render
purposes, larger polygons and concave/nonplanar quads are outside this profile. A malformed or
unsupported representation refuses the **entire view export** with a stable code before an output
file is created. Attachment, reference inspection and non-view model operations remain available;
`Inspection::reference_view_error` records the display refusal without invalidating R01 data.
Empty and invisible groups have no representation. References remain skipped by engineering
picking; R02/R03, full transforms and foreign authoring are not added.

Regression evidence: `crates/engine/tests/reference_scene.rs` covers attach-to-snapshot, combined
surfaces, bounds, units correction, winding, bundle round-trip, kind hiding, pick exclusion, moves
without mesh changes, and malformed/over-budget inputs. `apps/beamcraft-cli/tests/reference.rs`
covers real attachment-to-view export, hierarchical translations, visibility, shared sources,
coexistence with steel, correction and explicit unsupported-transform refusals without breaking
inspection. `apps/beamcraft-ui/tests/viewport.rs` loads an attached reference through the CLI,
asserts buffers/bounds and verifies lit pixels with a native offscreen render (the GPU step reports
a skip when no adapter is available). No kittest dependency is added: actual pixel readback tests
the viewport renderer directly.

USD semantics: [Cube size](https://openusd.org/release/api/class_usd_geom_cube.html#details),
[Xformable operation order](https://openusd.org/release/api/class_usd_geom_xformable.html#details),
and [Mesh polygon topology](https://openusd.org/release/api/class_usd_geom_mesh.html#details).
All implementation and generated test models are contributor-original, MPL-2.0; the existing
generated-fixture entry in `ATTRIBUTION.md` covers the test sources.

## Negative cases (tests)

| Case | Result |
|---|---|
| `http`, `https`, `ftp`, `file`, `s3`, `omniverse`, `anon`, `data` scheme; `\\server\share`, `//server/share`; padded and upper-case forms | `PERMISSION_DENIED`, store root not touched (a non-existent store root gives the same answer and is not created) |
| Source file naming a URL in a reference, payload or sublayer | attached; the arc is inert, nothing is resolved (the 2026-10-27 update) |
| Write to the source layer, to a foreign prim, to an attribute of one, to a prim that does not exist yet below the root | `FOREIGN_READ_ONLY`, exit 2, store listing and head unchanged |
| Path outside the source namespace, unknown wrapper, malformed path or attribute | `NOT_FOUND`, `NOT_FOUND`, `INVALID_ARGUMENT` |
| Hand-edited layer: override, new child, property, type or kind opinion on `Source`, in authored, derived or root layer | `FOREIGN_READ_ONLY` at validation (the commit verifier uses it) |
| Hand-edited scale, pin that names other bytes, correction made for another digest, extra prim under the wrapper, arc removed | `INVALID_ARGUMENT`, `INTEGRITY_ERROR`, `INTEGRITY_ERROR`, `INVALID_ARGUMENT`, `DEPENDENCY_UNAVAILABLE` |
| Source layer owned, absent, wrong size; duplicate wrapper id | `FOREIGN_READ_ONLY`, `DEPENDENCY_UNAVAILABLE`, `INTEGRITY_ERROR`, `DUPLICATE_ID` |
| No default prim, undefined default prim, up axis X, non-finite or zero units, element-like prim name, over 65536 bytes, not a layer, truncated, usdz | `UNSUPPORTED_PROPERTY`, `NOT_FOUND`, `UNSUPPORTED_PROPERTY`, `NON_FINITE_NUMBER` / `INVALID_ARGUMENT`, `INVALID_ID`, `LIMIT_EXCEEDED`, `INTEGRITY_ERROR`, `INTEGRITY_ERROR`, `UNSUPPORTED_PROPERTY`; no revision created |
| Corrupted stored foreign blob | `INTEGRITY_ERROR` on every open |
| Units: NaN, infinity, zero, negative, above 1e6, up axis X, missing values | `NON_FINITE_NUMBER`, `INVALID_ARGUMENT`, `LIMIT_EXCEEDED`, `INVALID_ARGUMENT` |
| Same op id again; same op id with other content; used wrapper id | replay, `OP_ID_REUSE`, `DUPLICATE_ID` |

## Evidence

* `crates/model` unit tests: path classification (14 URL / network / device forms, local forms
  that must pass), units bounds in F04 order, pin rules, fingerprints.
* `crates/engine` unit tests: admission, footprints, move and correction effects, units status,
  `source_point_to_world`, and that `write_foreign` is a refusal for every input.
* `crates/layer-engine/tests/reference.rs` (real `openusd` 0.8.0, 14 tests): composition below the
  wrapper with provenance and the foreign type showing through; foreign bytes retained and never
  among the saved layers; deterministic saves; move, rename and correction leave the foreign bytes
  alone; ten hostile layer edits refused; pin and shape faults; a binary source; inspection of
  unsafe sources; pre-R01 pinned layers still editable.
* `apps/beamcraft-cli/tests/reference.rs` (real binary, 13 tests): the end-to-end results above,
  including that the original file is only read (bytes and modification time), the stored blob is
  the file's bytes under its SHA-256, the path of the original is stored nowhere, later saves of
  other commands carry the layer unchanged, a workspace forks, publishes and undoes with it, a
  revision whose authored layer was edited as plain text (a type, a child or a property on the
  foreign root) is `FOREIGN_READ_ONLY` for `inspect-reference` and for the next `create-plate`,
  and a sixth distinct source exceeds the eight-layer stage budget (`LIMIT_EXCEEDED`, nothing
  stored).
* `apps/beamcraft-cli/tests/reference_pxr.rs` (opt-in, local): `pxr` 0.25.11 composes the model
  below the wrapper, places all eight corners of the foreign block within 2e-12 mm of the engine
  function and of the closed form (metres and Y up corrected, a quarter turn about Z, a
  translation), finds the foreign layer byte-identical to the file, finds no owned spec below the
  arc holder, and `UsdUtils.ComplianceChecker` reports no error, failed check or warning. Skipped
  without `pxr`; a skip is not evidence. Not the F03-audited 26.8 release.

## Not done and limits

- **Size and shape of the source.** One self-contained USDA or restricted USDC layer of at most
  65536 bytes. These are the experimental U01 stage budgets (65536 bytes per layer, 262144 in
  total, 8 layers, 256 prims); a real CAD export exceeds them. No sublayer, payload, variant set,
  inherit, specialize, instanceable, time sample, asset-valued attribute (textures) or `matrix4d`.
  Raising the budgets needs the LEW-03 containment evidence. The 2026-10-27 updates supersede this
  bullet except for multi-file sources, which remain unsupported.
- **A foreign prim may not carry an element identity.** A foreign `bim:id`, or a prim named `E_...`
  without one, makes the stage fail with `INVALID_ID`: the identity namespace is ours.
- **Parser containment.** The layer engine runs in this process (as in P01); a hostile foreign file
  is parsed here under the wrapper's bounds, not under OS containment.
- **No detach, no source update, no measurement, no annotation objects.** Removing a wrapper is
  not a command (a workspace undo removes it); R02 measures with `source_point_to_world`, R03
  updates the source (the version counter and the stored statement are ready for it), W03 stores
  measurements.
- Replay of an operation id is decided from the head state, as in P04 to P08.
- Windows only; `cargo deny` and the wasm32 check of the three pure crates were not run (tool and
  target not installed). USDC sources are covered by layer-engine tests only (no CLI or `pxr`
  run). Older stores pinned before P02.2 are not covered.

## Provenance

Original code and tests, MPL-2.0. USD behaviour from the public OpenUSD documentation of
references, composition, `xformOp` suffixes, `metersPerUnit` and `upAxis`; F04 sections 1, 3, 8;
the F10 `reference` kind; the U01 wrapper requirements LEW-01 and LEW-04. The foreign models in the
tests are invented text written by the tests; nothing of the NVIDIA, Autodesk or any other sample
is used or reproduced, and no third-party model, asset or code is added (`ATTRIBUTION.md` lists
the oracle scripts only). No new dependency.
