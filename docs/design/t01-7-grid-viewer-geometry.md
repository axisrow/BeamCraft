# T01.7: derived USD grid guides

The engine tessellates each finite line into a closed sixteen-sided tube of diameter 50 mm,
centred on local Z=0. The disconnected tubes share one standard USD Mesh below the grid's
`Geometry` child; the authored grid frame supplies its XY translation and Z rotation. Point,
polygon and index buffers are bounded by the model's 400-line limit. A geometry-only digest
excludes names, labels, identities and inherited placement.

Grid creation/copy and line/family edits supply the engine result through the existing validated
mesh change into the derived layer. Grid-family edits declare derived writes. The layer engine
sets `purpose=guide` for grid results and refuses engineering quantity records. Stored guides
must be Mesh children with guide purpose, no element identity and no mass/volume properties;
standard mesh buffers, extents and normals use the existing validator. Legacy grids with no
derived guide remain readable. Authoritative grid attributes never move into the derived layer.

The guide has no engineering body or FeatureIds. Picking, bounds, BOM, numbering and IFC continue
to use the grid's engineering exclusion. Relabelling leaves the derived bytes unchanged; local
geometry edits rebuild the guide; frame edits are inherited. Deletion removes both layers and
workspace undo/redo uses the existing atomic revision mechanism. USD text labels are not authored.

Tests check tube radius/endpoints and label/frame-invariant geometry digests independently, real
CLI exclusion and edit/delete behaviour, and reopening the complete standard-USD layer set with
OpenUSD when its Python oracle is configured. No interactive external viewer is assumed available.
This slice follows #421 and the defaults proposed in #415; it does not complete the T01 parent gate.

Contributor-original MPL-2.0. No dependencies or assets added.
