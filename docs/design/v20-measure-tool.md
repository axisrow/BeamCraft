# V20 interactive measure tool (point to point)

Date: 2026-11-14. Status: **proposed; nothing implemented.** A planning note for fork issue #561
(upstream [Apibim-com#861](https://github.com/Apibim-com/BeamCraft/issues/861)): the ribbon's
Measure button is a stub (`apps/beamcraft-ui/src/app/workbench.rs`, `tool_button(ui, Icon::Measure,
"Measure", false, false)`), and no plan row specifies the tool it advertises. This note proposes
the minimal point-to-point tool; the plan row **V20** lands with it in `docs/plan/tasks.md`.
Owner acceptance of this note precedes any implementation, and this note's PR changes no code.

## What the plan has for measurements today

- **G06** (merged) — kernel topology/volume measurement; internal math, not a user tool.
- **R02** `measure-reference` (experimental slice merged; row deferred) — kernel measurement of a
  foreign reference's composed surface, printed as JSON by the CLI.
- **W03** (open) — saved measurement objects that go stale on shape change.

None of these specifies the interactive viewport measure — pick two points in the 3D view, read
the distance. This note fills that gap for **point to point only**.

## Scope

In scope, as two slices of one task (V20 is the acceptance gate):

1. **V20.1 — headless.** A read-only engine function and a CLI command that answer the distance
   between two world points as JSON.
2. **V20.2 — viewport.** Two-point selection in the native view on the window's existing snaps,
   a live rubber-band and readout, and the ribbon button wired to the same engine function.

Out of scope, kept as follow-ups (none is started by V20):

- **Point-to-object and object-to-object distance.** "The distance to a body" needs its own
  contract (nearest-point semantics on holes and concave bodies) before it can be specified.
- **Saved measurements** — W03, unchanged: persistence, references, staleness on shape change.
- **Chained polylines, angles, areas.**
- **Drawing dimensions.** #623's dimensioning concern stays on the D side; this tool places
  nothing on a sheet.
- **Foreign reference surfaces as first-class endpoints.** The tool measures whatever points the
  snap pipeline already resolves (V11 snap kinds include reference snaps); no new
  reference-measurement surface is added here — R02 stays separate.

## Engine API (slice V20.1)

One pure function beside the existing `measure_reference` in `beamcraft_engine::measure`:

```text
point_distance(from_mm: [f64; 3], to_mm: [f64; 3]) -> Result<PointDistance, ViewError>

PointDistance { from_mm, to_mm, delta_mm: [f64; 3], distance_mm: f64 }
```

- **Reading, not a command.** It reads no model, no snapshot and no revision; it is a pure
  function of its two arguments. Resolution of FeatureIds is **not** duplicated here: callers
  resolve an id to a world point through the existing read-only
  `beamcraft_engine::query::feature::lookup_feature` (already used by `lookup-feature` and
  `pick`) and then call this function. The CLI and the UI therefore run the same engine path.
- **Arithmetic, no certification.** The distance is the Euclidean norm of `to − from` in world
  millimetres, computed in binary64. Unlike R02, which certifies composed surfaces through the
  kernel before any value exists, a point pair always has a distance: no tolerance band, no
  outward rounding, no `uncertain` state. The only refusals are non-finite inputs
  (`GEO_NON_FINITE`), so a NaN can never print as a measurement.
- **Deterministic.** Same arguments, byte-identical result — the R02 determinism decision, which
  here costs nothing because there is no model input at all.

## CLI (slice V20.1)

```text
beamcraft measure-points --store ROOT --from X,Y,Z|FEATURE-ID --to X,Y,Z|FEATURE-ID
```

Named after `measure-reference`, per the F04 verb-first rule. One JSON object, exit 0:

```json
{
  "outcome": "point_distance",
  "revision": { "…the verified head, as the other read-only commands print it…" },
  "from":  { "point_mm": [0.0, 0.0, 0.0], "feature_id": null },
  "to":    { "point_mm": [3000.0, 0.0, 4000.0], "feature_id": "fid1:…:plate:v.x1.y0.z1" },
  "delta_mm": [3000.0, 0.0, 4000.0],
  "distance_mm": 5000.0
}
```

- **`--store` is required even for two plain coordinates.** One contract: the command answers
  from the verified head's pinned snapshot exactly as `pick` and `lookup-feature` do, the
  `revision` names the head the answer is bound to, and the same head with the same inputs
  prints byte-identical output. No revision is created; nothing is written.
- **Endpoint forms, in any mix.** Coordinates `X,Y,Z` in world millimetres (the format pick and
  snap already print), or a **vertex-role FeatureId** resolved through `lookup_feature` to its
  world position. `point_mm` always carries the resolved world point; `feature_id` echoes the id
  when one was given, `null` otherwise. Edge and face roles are refused: fid1 gives them no
  single representative point, and inventing a face-centre convention is not this task's job.
  (An edge length or a point-to-face distance is a follow-up.) Member axis ends are reachable
  as coordinates today — the snap query prints them as points — and vertex roles cover the
  canonical member corners, including the `cut:v…` cap-corner forms of #623 once that lands.

Negative cases (F04 `code` + F06 reason, exit 2, nothing written):

| Case | Answer |
|------|--------|
| Non-finite coordinate | `GEO_NON_FINITE` |
| Missing or unparsable `--from`/`--to` | `VIEW_ARGUMENT_INVALID` |
| Malformed FeatureId | `GEO_FEATURE_ID_INVALID` |
| FeatureId absent from the head | `GEO_FEATURE_NOT_FOUND` |
| Edge or face role as an endpoint | `GEO_PICK_INPUT_INVALID` (not a point input) |

## Viewport and ribbon (slice V20.2)

- **The button stops being a stub.** Enabled exactly when the Fit model button next to it is
  enabled — a pane with an open store exists — and carrying the active state while the tool
  runs. No third state, no hiding.
- **Two clicks, the window's snaps.** Clicking Measure puts the active pane into the two-point
  measure mode. Each click consumes the window's existing snap resolution — the V04 snap query
  with the kinds and switches of the V11 snap panel; the tool adds no snapping of its own and no
  new snap kind. A click that resolves no point does not advance the tool.
- **Live feedback.** After the first point, the pane draws a session-only rubber-band segment to
  the current snap point with the live distance, computed per frame by the same
  `point_distance` function.
- **Readout.** The second point puts the distance in the status line, in the status line's
  millimetre formatting (`PointText`'s convention); it stays until the next user action. Esc,
  a right-click or picking any other tool cancels or leaves the mode at any stage.
- **Nothing is stored.** The measurement is a reading: no model change, no views-layer object,
  no revision. Persistence is W03's follow-up and stays out.
- **References.** A point resolved on a foreign reference's surface is an ordinary point; the
  tool special-cases nothing.

## Relationship to open work (this note edits no contract)

- **#622 (pick-ray height residue in snapped points).** The measure path consumes snap and pick
  outputs as plain coordinates and proposes no change to `query::pick`, its entry points or its
  codes. If #622 lands first, its cleaned points are simply what the tool reads; nothing in V20
  depends on it.
- **#623 (cut cap corner FeatureIds).** V20 resolves vertex-role FeatureIds only, through
  `lookup_feature` exactly as it stands; the `cut:v…` forms #623 adds resolve with no
  measure-side change and no grammar, parse or naming edit. Conversely, #623's drawing
  dimensioning does not depend on this tool.

Both are separate PRs; their code and frozen contracts are not touched by the V20 work.

## Acceptance (parent gate V20)

The same two points answer with the same distance through the engine API, the CLI and the UI
readout; measuring never changes the model, the head or the snapshot token; the ribbon button is
enabled only with an open pane and runs the same engine function as the CLI.

## Provenance

Original design, MPL-2.0, written for this note from the repository's own contracts (F04
command naming, F06 feature and pick surfaces, the headless-first rule of `AGENTS.md` §3). No
product documentation, code or assets used; no new dependency; no crate-boundary change.
