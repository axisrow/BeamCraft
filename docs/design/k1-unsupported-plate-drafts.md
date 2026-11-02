# K1 authored-only plates with unsupported geometry

Follow-up within issue #171 and PR #174, on the owner's requested existing
`bug/k1-plate-contour-frame` branch. It implements the valid-authored/unsupported-evaluation
distinction of F04 section 3 (Element-local frames) and section 5, including frozen
F04-POS-28 / F06-POS-56. No contract, fixture, schema pin or roadmap gate is changed.

## State and persistence

`beamcraft_engine::plate_draft` admits a plate as either a complete evaluated result or a typed
`UnsupportedPlate`. Only a contour that passes the complete bounded kernel admission can enter
the latter state. Numeric uncertainty, nonplanarity outside the scaled band, wrong winding,
intersections, invalid arguments, duplicate IDs and missing definitions remain failures.
This slice retains unsupported-plane contours without operations and with identity element
rotation. Operations and P10 placements are not converted into accepted unsupported input.

`beamcraft_commands::plate_draft::create_plate_draft` publishes the authored-only result through the
existing layer author, footprint check, complete revision manifest, atomic host commit and
content verifier. The USD contains the unchanged authored contour, thickness, placement and
material pin, without a Geometry prim, mesh, quantity values or signatures. Geometry status is
recomputed from those authored inputs on every verified read, not trusted from a persisted flag.
Ordinary OpenUSD opens the layers without BeamCraft or a schema plugin.

The existing evaluated-body engine and application `create_plate` APIs retain their original
return types and refusal-before-mutation behaviour. New callers that accept authored-only
results use the explicit draft API; the CLI's existing `create-plate` command uses that API.
Replay retains the original revision and unchanged authored inputs; reuse with different
arguments is refused. Unsupported result paths verify an existing head before reporting or
publishing. The content verifier rejects even malformed Geometry specs, mesh descendants under
another name and stored derived-value opinions on an unsupported element.

## Inspection and consumer limits

`Inspection.unsupported_plates` explicitly accompanies its evaluated plate list. The JSON
`plates` list contains both, with the original inputs and `validity: valid`,
`geometry_status: unsupported`, `geometry_code: UNSUPPORTED_GEOMETRY`, stable reason and null
mesh, shape key and world bounds. Volume and mass have unsupported status and null values and
signatures. The inspection snapshot lists the identity with no render instance or mesh; its
geometry snapshot contains only supported meshes. No bounding-box estimate or zero replaces a
missing engineering value.

Deletion is supported. Editing, copying, numbering, BOM/IFC export and binary viewport bundle
export currently require supported geometry and return `DEPENDENCY_UNAVAILABLE` rather than
misreporting a missing element or silently excluding it. Existing supported plates can coexist
with unsupported plates and retain their meshes and quantities. Editing/repair and richer
geometry-free viewport/report protocols need later implementation; this slice does not claim
their F04/F06 acceptance.

## Verification

The new CLI near-planar regression failed with `UNSUPPORTED_GEOMETRY` before command integration.
Engine tests distinguish authored acceptance from evaluated-body refusal and cover invalid,
nonfinite, duplicate, winding, placement, replay and reuse cases. The unchanged frozen F04
driver now uses authored admission: all 37 decidable plate projections match, with five context
skips and no declared deviations. F06-POS-56 additionally checks accepted unchanged authored
input while retaining unsupported evaluation; its complete diagnostic protocol remains partial.

Real CLI tests save, reopen in a fresh process, replay without a revision, coexist with a
supported plate, delete, and check that unsupported geometry consumers refuse without writing
output. A low-level revision test injects partial derived specs and proves they are refused.
The independent OpenUSD 0.25.11 oracle reads the exact original contour, asserts no Geometry,
mesh descendant, key or quantity value, and runs compliance checks. All six oracle tests passed
locally with the required-reader flag; the unsupported case had no compliance errors, failed
checks or warnings. Full workspace/CI results belong to the PR, with no gate pass inferred here.

## Provenance

Contributor-original Rust, Python test logic and synthetic numeric inputs, MPL-2.0. No dependency,
download, SDK installation or outbound runtime network access is added. Generated synthetic
models are covered by the existing test-data entry in `ATTRIBUTION.md`.
