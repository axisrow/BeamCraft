# D01: drawing contract and FeatureIds, version 0.1.0

Status: **draft.** The constants and rules here are the ones `beamcraft-engine` and
`beamcraft-model` enforce today (`CONTRACT_VERSION = "beamcraft-drawing/1"`); change the code and
this document together.

This contract fixes how an associative drawing names the model it was captured from, how it
addresses model geometry, and when it is stale. Views, dimensions and marks are placed by hand in
later issues (D04–D07); this document only fixes the contract they must reuse. Projection output,
persistence and publication are separate operations built on top of it (first slices in D02).

## 1. Rules

1. **A dimension never addresses a triangle index.** Every reference from a drawing into the
   model is a typed, fully qualified `FeatureId` (element UUID + body kind + feature kind +
   semantic role). Renderer arrays, mesh vertex/edge/triangle indices and viewport picks are
   never valid anchor syntax anywhere in the contract.
2. **Intent is admitted, then trusted.** Untrusted intent enters through `Drawing::admit` and is
   either refused with no partial result or becomes immutable validated intent. Admission grants
   no claim that the referenced geometry exists.
3. **No cached successful values.** Resolution recomputes from the pinned source or reports an
   explicit unresolved state; a drawing never substitutes its last known geometry.
4. **A lost feature is unresolved, never silently re-bound** (D02 acceptance, enabled here):
   nothing in resolution repairs a reference by index, nearest geometry or ordering.
5. **Identity is complete.** Every drawing object identity is a full canonical UUIDv7
   (`beamcraft-model::id`). Shortened, derived or positional identity is `INVALID_ID`.
6. **Headless first.** Everything here is an engine API with a CLI adapter; the CLI computes
   nothing the engine does not expose.

## 2. Identity

| Type | Names | Ownership |
| --- | --- | --- |
| `DrawingId` | one associative drawing document | owned by the drawing, separate from the source workspace |
| `DrawingViewId` | one view owned by a drawing | unique within its document |
| `DimensionId` | one associative dimension | unique within its document |
| `DocumentVersionId` | one immutable published drawing version | distinct from the source model revision |

Drawing object UUIDs must be pairwise distinct inside one document (`DUPLICATE_ID`). Document
revision identity (the document store's own revision history) is separate from both the source
model revision and any published version.

## 3. Source pin and staleness

Lives in `beamcraft_model::drawing` (a `domain` crate, so UI-adjacent consumers can depend on it
without the engine). `SourceModelPin` is the exact captured identity of the engineering model:

- `project` — source `ProjectId`, checked against the captured manifest by the storage adapter;
- `revision` — immutable source revision UUID, not a drawing revision;
- `manifest` — SHA-256 of the complete canonical source manifest;
- `schema` — engineering-model schema digest, not a drawing schema digest;
- `generation` — exact source workspace generation.

`SourceModelPin::freshness(LiveSource)` returns one of:

| `Freshness` | Condition |
| --- | --- |
| `Current` | a verified live pin equals the captured pin in every field |
| `Stale` | a verified live pin differs in any field |
| `Unavailable` | a live context was requested but cannot be verified or opened |
| `Unknown` | no live context was supplied (a self-contained historical drawing) |

Staleness is computed from pins only. It never refreshes intent, mutates a published version, or
ranks pins against each other; the fix for a stale drawing is an explicit refresh that recaptures
the source pin while retaining object IDs and semantic references.

## 4. Semantic references

A `FeatureId` is the only addressing scheme. For the first slice a dimension's two anchors must
be qualified vertices of the drawing's selected member (`element` equal, body kind `member`,
feature kind `vertex`); anything else is refused at admission. Equal semantic endpoints are
invalid intent. Distinct endpoints that collapse only under the chosen projection stay valid and
resolve as explicitly unresolved — that is a projection outcome, not an admission error.

## 5. Admission bounds

`Drawing::admit` refuses, with no partial result:

- duplicate drawing/view/dimension UUIDs;
- labels violating the model's bounded label rule;
- nonfinite or out-of-range numbers: paper coordinates and dimension offsets within
  ±`MAX_PAPER_MM` (10 000 mm), scale denominators 1 through `MAX_SCALE_DENOMINATOR` (10 000);
- more than three decimal places in a dimension label;
- anchors that are not distinct qualified vertices of the selected member;
- placed views violating the paper bounds above, colliding with any document-object, drawn
  object or sibling paper-object UUID (checked in both replacement directions);
- section cuts that are nonfinite or beyond the model coordinate cap, and details whose
  region centre or radius is nonfinite, non-positive or beyond the paper cap, or whose
  parent is not the primary view or a placed view;
- paper objects (placed views, sections, details, drawn objects and layers) together
  exceeding the one-page intent (`MAX_INTENT_PAGE_ITEMS`, 64 prims), even when each
  collection is under its own cap;
- more than `MAX_LAYERS` (16) layers, a layer name violating the bounded label rule, or layer
  UUIDs colliding with any other document object;
- a drawn object naming a layer the same admitted or replaced set does not define;
- any drawn number (coordinate, radius, height, rotation, scale, offset) whose canonical
  decimal form exceeds `MAX_NUMBER_TEXT_BYTES` (24) bytes — a pathological binary64 would
  otherwise overflow the document byte cap at encode time.

## 6. Resolution and unresolved states

Projection (`beamcraft_engine::drawing::projection`, algorithm
`drawing-orthographic-box/1;collapse-mm=0.001`) resolves intent against a strictly inspected
snapshot of the pinned source. Body wireframe and dimension each resolve, or fail with exactly
one explicit reason:

| `UnresolvedReason` | Meaning |
| --- | --- |
| `missing_owner` | selected owner was deleted or is absent |
| `unsupported_body` | owner is not a member or no longer has canonical box topology |
| `missing_feature` | a retained semantic role no longer resolves; never repaired |
| `collapsed_projection` | distinct anchors project below the 0.001 mm geometric floor |

There is no fabricated bounding-box fallback and no fabricated position mark in a label.

## 6a. Drawn objects (D07 slices 1–2)

A document also owns drawn paper-space objects (`drafting`). They are contributor-authored
geometry, never projections of the model, and they hold no model reference of any kind: editing
them reads and mutates only the document. Each is a typed UUIDv7 (`DraftingEntityId`) plus one
of:

| Kind | Payload | Bounds |
| --- | --- | --- |
| `line` | two distinct points | points within ±`MAX_PAPER_MM` |
| `polyline` | 2–16 points, no zero-length segment | points within ±`MAX_PAPER_MM` |
| `arc` | centre, radius > 0, start/end radians, nonzero sweep (magnitude ≤ one turn) | radius ≤ `MAX_PAPER_MM` |
| `circle` | centre, radius > 0 | radius ≤ `MAX_PAPER_MM` |
| `rectangle` | two opposite corners, positive width and height | corners within ±`MAX_PAPER_MM` |
| `text` | anchor, height > 0, rotation (radians, magnitude ≤ one turn); `drawing:text` string | anchor and height within ±`MAX_PAPER_MM`; text non-empty, no control characters, ≤ 128 bytes |
| `hatch` | closed outline of 3–16 vertices (closing segment implicit), `drawing:pattern` = `solid`\|`diagonal`\|`crosshatch` | vertices within ±`MAX_PAPER_MM`, positive-length segments |
| `symbol` | anchor, scale > 0; `drawing:symbol` = `north`\|`section`\|`detail`\|`elevation` | anchor and scale within ±`MAX_PAPER_MM` |
| `free_dimension` | two distinct points, signed offset, 0–3 decimal places | points and offset within ±`MAX_PAPER_MM` |

A `text`, `hatch` or `symbol` entity additionally carries exactly one string payload attribute —
`drawing:text`, `drawing:pattern` or `drawing:symbol` — required by its kind; every other kind
carries none. A free dimension measures the drawn paper interval only: it binds no model feature,
resolves nothing through the pinned source and never becomes stale through the model. The
document-wide text volume shares the intent byte cap below, so long labels are limited by
`MAX_INTENT_BYTES`, not by a separate budget.

## 6a-1. Layers and line types (D07 slice 3)

Every drawn object names the document layer it sits on (`drawing:layer`) and its stroke pattern
(`drawing:linetype` = `continuous`|`dashed`|`dotted`|`dashdot`, `LineType`). A document holds up
to `MAX_LAYERS` (16) layers (`LayerSpec`), each a typed UUIDv7, a name under the bounded label
rule, and a `visible` flag. A layer carries no model semantics and no geometry of its own:
hiding one is a paper-visibility filter (`Drawing::visible_drafting`) and never touches the
stored objects or the engineering model.

Admission refuses a drawn object whose layer the same replacement does not define, so no
admitted state ever holds a dangling layer reference. `Drawing::with_drafting` replaces layers
and drawn objects as one atomic intent operation. Layer UUIDs are pairwise distinct and distinct
from the document, placed-view and drawn-object UUIDs. Layers occupy the same one-page intent
budget as every other paper object: the page formula counts one optional `/Drawing/Layers` scope
plus each layer.

In the intent USD the layers are the canonical ordinal children `L1..Ln` of `/Drawing/Layers`
(`drawing:id`, `drawing:name`, `drawing:visible`); each drawn entity additionally carries
`drawing:layer` and `drawing:linetype`. The canonical re-encode comparison refuses any other
topology.

Admission refuses more than `MAX_DRAFTING_ENTITIES` (60) objects, any entity UUID equal to the
drawing/view/dimension UUID, and any pair of equal entity UUIDs. Coordinates are exact paper
millimetres; angles are radians. The intent byte cap (`MAX_INTENT_BYTES`, 131 072) holds the
densest admitted page: even fifty-eight sixteen-point hatches on the longest bounded canonical
coordinates encode within it, so an admitted document never refuses at encode time. In the intent USD the entities are the canonical ordinal
children `E1..En` of `/Drawing/Drafting` (`drawing:id`, `drawing:kind`,
`drawing:geometry double[]`); the canonical re-encode comparison refuses any other topology.

## 6b. Placed views, sections and details (D04 slices 1–2)

Beside the primary view, a document may hold up to `MAX_VIEWS` (16) placed views
(`DrawingSpec.views`), each a full `ViewSpec` with its own front/top/right basis, scale
denominator and paper origin under the same bounds as the primary view. Nothing is placed
automatically: a created or refreshed document carries only the views its caller named, and
placement changes only through the whole-document intent operations `Drawing::with_views`,
`with_sections` and `with_details` (the CLI `edit-views`, which replaces all three collections
in one operation), never as a side effect of model or drafting edits.

Identity discipline mirrors the drawn objects: every paper object's UUID — placed view,
section, detail or drawn object — is pairwise distinct and distinct from the
drawing/view/dimension UUIDs, in both replacement directions.

A **section view** (`SectionSpec`) is a cut plane perpendicular to its basis's depth axis
(Front looks along Y, Top along Z, Right along X) at `cut_mm`, with `side` picking the visible
half-space. It resolves through `projection::resolve_section` to the visible half of the
member wireframe — crossing edges end exactly on the cut — plus the exact cut contour (the
edge/plane crossings in boundary order, empty when the cut misses the member box). Axis cuts
only: arbitrary cut planes are outside this contract, exactly as arbitrary cameras are.

A **detail view** (`DetailSpec`) magnifies one parent view's circular region
(`center_paper_mm`, `radius_mm` in the parent's paper millimetres) at the detail's own scale.
The parent must be the primary view or a placed view; details of sections or of other details
are outside this slice's contract. It resolves through `projection::resolve_detail` to the
parent wireframe mapped by the scales' ratio and clipped to the region, keeping the parent's
semantic feature addresses.

Each derived view resolves from the document's exact pinned snapshot: an object outside the
document refuses, a snapshot whose pin disagrees refuses before geometry, and
missing/unsupported geometry stays explicitly unresolved. Freshness is unchanged — every
paper object shares the document's source pin, so a model change marks all views stale
together and an explicit refresh re-binds them.

The closed canonical intent carries placed views as the optional `/Drawing/Views` scope
(`V1..Vn`: `drawing:id`, `drawing:plane`, `drawing:scale`, `drawing:origin double[]`),
sections as `/Drawing/Sections` (`S1..Sn`: plus `drawing:cut`, `drawing:side`) and details as
`/Drawing/Details` (`D1..Dn`: plus `drawing:parent`, `drawing:center double[]`,
`drawing:radius`). The whole intent must fit one closed reader page: `MAX_INTENT_PAGE_ITEMS`
(64) prims — the three fixed document prims, one optional scope per non-empty collection, and
every paper object. `edit-views` follows the same replay discipline as `edit-drafting`: no
model store on the command line, and the request fingerprint binds the incoming placement of
all three collections, so an exact replay returns the same revision and a changed placement
under the same operation id refuses.

## 6c. Sheets (D05)

A document may be laid on one sheet (`DrawingSpec.sheet`, `beamcraft_model::sheet`): a fixed
catalogue size (`a4`–`a0`, portrait; any other size refuses, nothing is approximated), a
uniform frame inset (`SheetFrame`, at least 1 mm and strictly inside the sheet) and two
authored title block fields (`TitleBlock.project`, `TitleBlock.author`, both under the
bounded label rule). The sheet holds no UUID: it is document furniture, not a document
object, so it adds no identity constraints — only one more optional scope to the one-page
budget. Placement changes only through `Drawing::with_sheet` / `without_sheet` (the CLI
`edit-sheet`, same replay discipline and no model store).

The whole-sheet export (`drawing::sheet::export_svg`) renders one exact published version as
deterministic stroke-only SVG: frame, title block (contributor-original stroke glyphs, no
font or embedded text), every resolved placed view and the primary view's dimension groups.
Anything unresolved refuses; every stroke must lie inside the frame — content that does not
fit refuses instead of being rescaled or clipped. A document without a sheet keeps the
loose-paper export of section 6, and `--format pdf` defaults its print sheet to the
document's sheet size.

## 7. Module map

- `beamcraft-model::id` — drawing object identities (typed UUIDv7).
- `beamcraft-model::drawing` — source pin, live-source and freshness states.
- `beamcraft-model::sheet` — sheet entities: the fixed A4–A0 catalogue, frame and title
  block, and their admission.
- `beamcraft-engine::drawing` — admission (`Drawing::admit`), view/dimension specs, placed
  views (`views`), drawn objects (`drafting`), the sheet (`sheet`), re-export of the pin and
  freshness states; `projection` (including per-view `resolve_view`), `svg`, `pdf`, `print`
  and `sheet` submodules. `pdf` renders one published version as stroke-only PDF 1.4 (no
  font is embedded or referenced); `print` is the print pipeline shape: resolved strokes laid
  out 1:1, centred on a fixed A4–A0 sheet, refusing instead of rescaling; `sheet` is the
  whole-sheet renderer of section 6c.
- `apps/beamcraft-cli` — `preview-drawing`, `create-drawing`, `refresh-drawing`,
  `edit-drafting`, `edit-views`, `edit-sheet`, `publish-drawing`, `inspect-drawing`,
  `inspect-drawing-versions`, `export-drawing`, and the drawing document store; it computes
  nothing the engine does not expose.
