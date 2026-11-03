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

Deletion is supported. Copying, hole drilling, numbering, BOM/IFC export and binary viewport
bundle export currently require supported geometry and return `DEPENDENCY_UNAVAILABLE` rather
than misreporting a missing element or silently excluding it. Existing supported plates can
coexist with unsupported plates and retain their meshes and quantities.

## P-Degenerate: the degenerate class and repair (A-2 D-7)

Since amendment A-2 D-7 (decision of 2026-10-19, task P-Degenerate, fork issue #422) a wrongly
built contour is retained as well, as the degenerate class of `UnsupportedPlate`
(`degenerate: true`): clockwise winding, a zero area vector, an off-band planarity, a zero
projected edge or a broken topology — every `INVALID_ARGUMENT` fault the contour admission
reports, minus an edge below the geometric floor, which the authored layer's own storage floor
refuses, so it keeps refusing with its precise message. The stable reason tokens
(`contour_winding_clockwise`, `contour_area_vector_zero`, `contour_outside_planarity_band`,
`contour_projection_zero_edge`, `contour_bad_topology`) are matched against the admission's own
message vocabulary in `kernel::plate_contour::degenerate_reason`; the frozen F04 fixture cases
that predicted a refusal (NEG-28, NEG-29, NEG-30, NEG-38) are declared deviations in the K1
admission driver, superseded by the amendment, not edited.

Editing is implemented for retained plates: `update-element` patches the authored record,
reclassifies the result once, and either repairs the element (the kernel evaluates the body,
the mesh and quantity records are stored, the quantities read `current`) or keeps it
authored-only; a hard refusal of the patched state still refuses the whole command, so nothing
is dropped or half-written. An update that would degrade an evaluated plate into a wrongly
built one is therefore refused with the kernel's message and the head keeps its derived data.
The repair input is a whole-contour replacement (`update-element --contour`); the A12.3 contour
edits work on a retained plate as well, so a single vertex move can repair a stored fault.
Shared-definition edits (material or shape with instances beside the target) refuse: the group
cannot share a body the kernel does not evaluate. The update result type distinguishes the
classes (`UpdatedPart::Evaluated` / `UpdatedPart::AuthoredOnly`); the JSON labels a retained
plate `validity: degenerate`, `geometry_status: degenerate`, `geometry_code:
INVALID_ARGUMENT` for the degenerate class and keeps the K1 labels for the unsupported class.
Members keep their hard refusals: a member draft state (retention of wrongly built paths and
profiles) is not implemented and remains later work.

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
