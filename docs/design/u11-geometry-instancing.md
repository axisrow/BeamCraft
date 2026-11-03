# U11 instancing of shared geometry

Contributor-original BeamCraft code and documentation, MPL-2.0.

U11 delivers the USD instancing contract left open after #497: **an `instanceable` reference to an
immutable geometry asset**, with the acceptance "1000 copies give one prototype; editing one copy
publishes a new blob only for it". The P11 shared *authored* definitions stay unchanged; this card
changes the *derived* storage form.

## Form

Each distinct derived body is stored once as an immutable, content-addressed asset prim in the
derived layer:

```text
/World/Definitions/Geometry/GEO_<sha256>     def Mesh: points, faceVertexCounts,
                                             faceVertexIndices, subdivisionScheme, orientation,
                                             extent, normals (uniform), bim:shapeKey
/World/Steel/Parts/PRT_<uuid>/Geometry       def Mesh (instanceable = true, kind = subcomponent)
                                             { references = </World/Definitions/Geometry/GEO_...> }
                                             { per-element quantity record, guide purpose }
```

* The asset name is the full SHA-256 of the canonical asset content: the engine shape key followed
  by the exact stored `f32`/`i32` buffers (`authored::mesh_asset_digest`). The name always
  recomputes from the content it labels, and structural validation enforces it — a layer whose
  `GEO_` label does not recompose is refused (`InvalidId`).
* The asset prim carries no element identity, no model kind and no quantity record. It is never
  rewritten: a mesh transfer skips an existing spec with the same content-addressed name
  (idempotent, like resource definitions), and deleting the last referent keeps the asset, like
  every pinned definition.
* The element's own prim is an `instanceable` reference. Equal bodies — the copies of one part, a
  whole P11 instance group — reference the same asset and compose one prototype in ordinary USD;
  per-element data (the quantity record, a grid guide's `purpose`) stays a local opinion on the
  referencing prim and composes over the shared buffers.

## Acceptance

* 1000 copies: ten array rounds of 100 in `layer_instancing::a_thousand_copies_share_one_prototype`
  produce one `GEO_` asset and exactly 1000 `instanceable` wrappers; the stage opens, composes and
  validates within the model budgets. The independent oracle (`beamcraft-usd-oracle`) composes the
  internal reference and checks the instanceable form on every committed revision it reads.
* Editing one copy: a new body hashes to a new name, so exactly one new asset prim is written for
  the edited body; the shared asset's spec is carried verbatim and the other copies' wrappers are
  untouched (`a_new_body_publishes_one_new_asset_and_never_rewrites_the_shared_one`). The derived
  layer of a revision is one blob, so the container blob is rewritten; no geometry content of any
  other copy changes inside it.

## Profile and compatibility

* The U01 static read profile admits the `instanceable` metadata (a boolean) and *internal*
  reference arcs (empty asset path, absolute target prim path) on owned layers; external arcs
  keep their existing admission, and offsets, custom data, payloads, inherits, specializes and
  variants stay refused. An internal arc adds no manifest dependency; a dangling target fails in
  the composer like any other unresolvable owned opinion.
* The composed snapshot exposes `instanceable` as a new `Prim` field.
* The schema descriptor changes (the new mapping lines), so the pin changes and stores written by
  earlier pre-alpha schemas are refused without migration, per the U16 rule.
* C03 rebase classifies asset-prim deltas as derived writes of the content digest
  (`DependencyTarget::Definition`, group `derived`), and a transfer's footprint/evidence carries
  that digest, so a rebase admits the shared prim's delta but refuses any delta outside a
  verified transfer. Existing assets are immutable; a tampered one fails the name recomposition
  check before it can rebased into a candidate.

## Limits

* Wrapper-local quantity opinions compose per instance; ordinary USD tools may therefore keep more
  than one runtime prototype between elements whose quantity records differ — the geometry itself
  is still shared, and BeamCraft's own reader composes per prim regardless.
* There is no garbage collection of unreferenced assets; like definitions, they stay for the
  revision's lifetime. Compaction (U14) may revisit this.
* `beamcraft-usd-oracle` composes one internal reference hop per prim; SDK interchange (U04/N03)
  remains unverified, as before.
* The U12 representation levels (`Reduced`, `Proxy`, `Axis`) stay inline, per-element prims:
  they are authored only where their body differs from the full one, so this slice takes no
  part in the U11 asset sharing. Asset-ifying a level whose body repeats across copies is a
  possible follow-up.
