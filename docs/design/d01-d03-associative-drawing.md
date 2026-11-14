# D01–D03: first associative drawing

Scope: fork #13 / upstream #127. This is acceptance and storage design before implementation.
`d1-d2-project-level.md` describes project/level metadata, a different task. The drawing plan is
D01 contract/FeatureIds, D02 projection/dimension, then D03 manager/version/publication. Each
package keeps its own focused PR and may split into named subtasks.

Baseline: verified fork `master` `df02970` (2026-10-23). Existing V04 qualifies F06 semantic box
roles as `fid1:<full ElementId>:member:<role>`; `lookup_feature` resolves them against a pinned
`ViewSnapshot`. A rectangle member keeps the certified canonical box builder. I-sections, holes,
cuts and other non-box topology do not acquire fictitious box FeatureIds. Drawing references
must reuse that rule, rather than a renderer buffer, mesh/triangle index or nearest geometry.

## First vertical slice

One owned straight rectangular member, one orthographic wireframe view and one aligned linear
dimension between two qualified vertex FeatureIds. Both endpoints belong to the selected member:
its own `member` or `profile` vertices, or the corners of its plane-cut caps
(`cut:v.p<n>.…`, [cut cap corner amendment](../contracts/f06-cut-cap-corners-amendment-v0.1.md)),
so a cut or fitted member's kept length can be dimensioned.
The dimension's value is its projected distance in model millimetres; paper scale changes its
drawn size, not the reported model-unit value. A view that collapses the anchors has an unresolved
dimension, not a successful zero measurement. The contract explicitly identifies the projection
plane; it does not label a foreshortened projection as a true three-dimensional length.

Initial planes are exact front/top/right axis-aligned orthographic bases. Default scale is 1:100;
the caller chooses the plane, anchors and paper dimension offset. General camera views, hidden-line
removal, assemblies, sections/details, multi-view sheets, drafting and PDF/printer integration
remain their plan packages. Unsupported source geometry is explicit and never a bounding-box
substitute. An initial SVG is a self-contained vector proof/export of this limited drawing.

Projection geometry comes from the committed model's strictly verified kernel-derived snapshot.
The adapter opens one exact model revision, validates its USD/index/content and recomputes/checks
the body's mesh and quantities through the existing inspection path. The engine resolves named
edges/vertices and projects those coordinates in f64. A UI camera or its buffers are not the
source of a dimension. Geometry resolution is pure engine work; persistence is host/application
work. No new dependency, external service or engineering-model format is needed.

## D01 contract and identity

Use full typed UUIDv7 identities for the drawing, its view, dimension and immutable document
version. Preserve drawing/view/dimension identities through refresh; a publication gets a new
version. Clock order grants no authority. A feature reference includes its full owner and rule/
body/role identity, parsed by the existing engine `FeatureId` API. A reference must be a member
vertex in this slice; numeric triangle/vertex/instance addresses and malformed/shortened IDs
refuse before a document is committed.

The authored document contains its identity/name, exact source project/revision UUID/manifest
digest/schema/generation, one view (plane/scale/origin), and one dimension (two references,
paper offset and numeric format). Source identity is separate from document revision identity.
Only validated/admitted engine documents enter the writer; a storage DTO grants no semantic
authority. Untrusted saved documents are revalidated through the same contract before use.

Geometry states are `resolved` or `unresolved` with a typed reason and retained reference. A
missing owner, changed body/topology, missing role or coincident projected anchors has no cached
successful numeric value. No index repair, silent relink to a copy or nearest-face fallback is
allowed. The existing owner's unchanged semantic roles survive translation, rename and supported
dimension edits; a copy has a new ElementId and new qualified FeatureIds.

Freshness compares the exact source pin with the current model when one is supplied: current,
stale or unavailable. Without a live model context it is unknown. Stale is not the same as an
unresolved reference: an immutable published version remains resolved at its own archived source
even when the live model changes. An explicit refresh creates a draft against a newly selected
source pin and resolves the same references there. Repair/rebinding requires explicit caller
input and never changes an already published version.

Marks must remain distinguishable from drawing dimensions and model names. This slice prints
no fabricated approved position number; the source pin retains any A09 approval state. D06 adds
manual marks/leaders and their chosen attribute bindings. Unavailable/stale numbering is never
represented as a valid mark by the drawing API.

## USD document and immutable source archive

The document is ordinary USD, with a separate drawing schema/descriptor. It does not add drawing
primitives to the engineering model or alter its schema. Use a distinct document store directory
with the existing content-addressed blobs, revision CAS, outcome ledger and recovery. Its project
is the source project; its document/workspace identity is the drawing's identity. Model commands
must not mistake that directory/schema for an engineering workspace.

The authored drawing layer stores intent and source pins. A derived drawing layer stores the
verified 2D projection/dimension results and their source/algorithm signature. Standard linear
USD curves in the XY plane can represent the first wireframe and original dimension strokes.
Derived numbers are checked against engine resolution before publication; they are not a second
independent source of geometric truth. A model schema parser never silently accepts a drawing
schema, or vice versa.

Every retained document version owns an immutable archive of the exact model manifest and all
its USD layer bytes under a separate resource namespace in the document store. The manifest is
host provenance metadata, not a second engineering model. Source layer names/digests/roles/sizes,
project/schema and recorded revision must match that manifest. The drawing reader constructs a
separate source-model open request with the original virtual names and validates it normally.
Document-layer parsing does not compose or reinterpret provenance metadata as geometry.

The source store's maintenance guard protects capture against compaction. Ordinary model commits
may continue: an explicitly captured revision is immutable and a document publication is bound
to that snapshot. Later live-head changes affect reported freshness, not the captured version.
After capture, model history compaction cannot remove a retained document's own source archive.
Document history destruction is not an implicit side effect of refresh or model compaction.

The document's own revision store retains draft/published state and the latest published version
reference. Publish requires a fully resolved draft and its verified archive; it creates a new
immutable version with exact content/request binding. The same operation and arguments replay
the historical version after later edits; another request under that identity refuses. Concurrent
document writes use an exact expected document head. Crash/unknown outcomes use actual committed
history and explicit host recovery, not a guessed successful version. A refused update preserves
the old complete document and engineering-model head.

## Headless surface and export

Expose stable feature enumeration as an engine query and CLI, so clients obtain full semantic
addresses from a pinned real model rather than inventing indices. Build the document/admission,
projection and resolution engine APIs before the application/CLI adapter. The final CLI surface
needs creation/inspection, explicit refresh or reference changes, publication/version listing
and export of one exact document version. It reports source pins, state and typed unresolved
reasons. Required operation/document identities and expected heads are bounded, canonical inputs.

SVG export uses the exact published version, f64 projection, model-unit dimension values and
paper-unit geometry. Its strokes, arrowheads and numeric glyphs are contributor-original vector
geometry, with no embedded external font, image, script, URL or commercial drawing asset. Escape
all textual metadata. Output is deterministic for identical version bytes, created without
overwriting an existing file, and never mutates the model/document during export. PDF and native
print are D08, not simulated by naming SVG as PDF.

Keep one view/dimension for the first slice. Bound document bytes, reference lengths, strokes,
versions/history, copied layer bytes, output size and numerical ranges before allocation/work.
Reject nonfinite values, invalid scales and unsafe paths; no unsafe, unchecked input indexing or
production panic. Reuse existing model/stage caps instead of raising them for a drawing. All
fixtures, glyphs and symbols are original MPL-2.0 with existing fixture attribution.

## Executable acceptance before completion

| Case | Required observable result |
|---|---|
| D01-01 identity | Full typed drawing/view/dimension/version IDs round-trip; short, wrong-version and duplicate document-object IDs refuse |
| D01-02 semantic addresses | Enumerate/resolve the real rectangle member's qualified features; bare triangle/vertex/buffer indices refuse; no fabricated features for non-box bodies |
| D01-03 round trip | Authored USD round-trips exact source pins, view and dimension references; unsupported schema and malformed/oversized records refuse |
| D01-04 reference continuity | Move/rename/supported dimensional edit preserves semantic IDs; copy has new qualified IDs; no repair by index |
| D02-01 real projection | Create a straight rectangle member through the real CLI; one view has expected projected geometry from the committed kernel-derived body |
| D02-02 measured dimension | A 1000 mm source interval reports the explicit closed-form projected distance; paper scaling does not change the model-unit label |
| D02-03 unresolved | Deletion, topology loss, wrong body/owner and collapsed projection retain references with typed unresolved state and no old numeric success |
| D02-04 determinism | Identical pin/spec gives identical output; nonfinite/range/cancellation/budget input refuses without partial state |
| D03-01 publication | Publish one resolved draft as an immutable document version bound to exact model revision/digest/schema/generation |
| D03-02 stale versus old version | Change the model: published bytes/export remain unchanged and live freshness becomes stale; explicit refresh produces a new draft/version with newly resolved geometry |
| D03-03 source archive | Compact old model history in an isolated fixture; the retained published document still verifies/exports from its exact archived source |
| D03-04 replay/race/crash | Fresh-process same-request replay creates no second version; changed payload refuses; a competing expected-head writer wins once; killed-process recovery yields an old or new complete document |
| D03-05 export/reader | Reopen/parse published drawing USD independently; parse SVG as XML and check explicit geometry/dimension numbers, source/version metadata, no external resource and deterministic bytes |
| D03-06 non-destructive refusal | Unresolved publication, corrupt archive/pins, wrong project/schema and existing export destination refuse with unchanged document/model heads and destination bytes |

Required workspace/platform/portable/licence checks apply to each named slice. Passing a pure
projection test, a synthetic viewport or this design document does not complete parent #13.
