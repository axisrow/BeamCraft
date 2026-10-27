# P15 part classes

Contributor-original BeamCraft code and documentation, MPL-2.0.

Plates, beams and columns have an authored int32 `steel:class` attribute. Creation defaults to
zero; `create-plate`, `create-member` and `update-element` accept `--class`. Inspection returns
`class`; creation forms and the property editor expose the same integer. Copies preserve it.
Other element categories cannot be assigned a part class.

The engine's typed create/patch APIs carry the class. It participates in command identity and
replay detection, but not body-cache inputs or engineering quantities. A class-only update writes
the label/metadata footprint group and authored layer; it leaves the mesh and quantities alone.
The class can feed the V14 filter and V17 representation rule APIs without becoming a material.

`beamcraft_engine::class_palette::DEFAULT_CLASS_PALETTE` is the contributor-original default
environment preset; `class_colour` also accepts an environment's own RGB entries. No third-party
palette was consulted. The preset's provenance is in `ATTRIBUTION.md`.

The layer schema descriptor gains `steel:class`; as with other pre-alpha schema extensions,
stores pinned to the prior descriptor require migration before they can be opened with this
schema. This task does not supply a migration or silently replace a store's schema identity.
