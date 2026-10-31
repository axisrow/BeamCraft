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
  object or sibling placed-view UUID (checked in both replacement directions);
- placed views and drawn objects together exceeding the one-page intent
  (`MAX_INTENT_PAGE_ITEMS`, 64 prims), even when each collection is under its own cap.

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

## 6a. Drawn objects (D07 slice 1)

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

Admission refuses more than `MAX_DRAFTING_ENTITIES` (60) objects, any entity UUID equal to the
drawing/view/dimension UUID, and any pair of equal entity UUIDs. Coordinates are exact paper
millimetres; angles are radians. In the intent USD the entities are the canonical ordinal
children `E1..En` of `/Drawing/Drafting` (`drawing:id`, `drawing:kind`,
`drawing:geometry double[]`); the canonical re-encode comparison refuses any other topology.

## 6b. Placed views (D04 slice 1)

Beside the primary view, a document may hold up to `MAX_VIEWS` (16) placed views
(`DrawingSpec.views`), each a full `ViewSpec` with its own front/top/right basis, scale
denominator and paper origin under the same bounds as the primary view. Nothing is placed
automatically: a created or refreshed document carries only the views its caller named, and
placement changes only through the whole-document intent operation `Drawing::with_views`
(the CLI `edit-views`), never as a side effect of model or drafting edits.

Identity discipline mirrors the drawn objects: placed-view UUIDs are pairwise distinct and
distinct from the drawing/view/dimension and drawn-object UUIDs, in both replacement
directions — `with_views` is cross-checked against existing drafting and `with_drafting`
against existing views.

Each placed view resolves from the document's exact pinned snapshot through
`projection::resolve_view`: a view outside the document refuses, a snapshot whose pin
disagrees refuses before geometry, and missing/unsupported geometry stays explicitly
unresolved. Freshness is unchanged — every placed view shares the document's source pin, so
a model change marks all views stale together and an explicit refresh re-binds them.

The closed canonical intent carries the views as the optional `/Drawing/Views` scope with
canonical ordinal children `V1..Vn` (`drawing:id`, `drawing:plane`, `drawing:scale`,
`drawing:origin double[]`). The whole intent must fit one closed reader page:
`MAX_INTENT_PAGE_ITEMS` (64) prims — the three fixed document prims, one optional scope per
non-empty collection, and every placed view and drawn object. `edit-views` follows the same
replay discipline as `edit-drafting`: no model store on the command line, and the request
fingerprint binds the incoming placement, so an exact replay returns the same revision and a
changed placement under the same operation id refuses.

## 7. Module map

- `beamcraft-model::id` — drawing object identities (typed UUIDv7).
- `beamcraft-model::drawing` — source pin, live-source and freshness states.
- `beamcraft-engine::drawing` — admission (`Drawing::admit`), view/dimension specs, placed
  views (`views`), drawn objects (`drafting`), re-export of the pin and freshness states;
  `projection` (including per-view `resolve_view`), `svg`, `pdf` and `print` submodules. `pdf`
  renders one published version as stroke-only PDF 1.4 (no font is embedded or referenced);
  `print` is the print pipeline shape: resolved strokes laid out 1:1, centred on a fixed
  A4–A0 sheet, refusing instead of rescaling.
- `apps/beamcraft-cli` — `preview-drawing`, `create-drawing`, `refresh-drawing`,
  `edit-drafting`, `edit-views`, `publish-drawing`, `inspect-drawing`,
  `inspect-drawing-versions`, `export-drawing`, and the drawing document store; it computes
  nothing the engine does not expose.
