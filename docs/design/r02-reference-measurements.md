# R02 reference measurements

Date: 2026-10-26. Status: **experimental vertical slice; the R02 acceptance criteria are
demonstrated for the supported profile below, and K3 is not claimed** (it also needs R03 and the
rest of G08). The command is `measure-reference`, per the F04 naming rule.

Plan row: R02 / M, depends on R01 (merged), P04 and G08. Result: measurements of a foreign
reference's composed surface as separate result objects, never written into the model and never
into the foreign content. Acceptance: units and transform respected; open or self-intersecting
meshes never exact; mass never invented. R03 (explicit source update) and W03 (persisting
measurements) are not here.

## What is implemented

```text
beamcraft measure-reference --store ROOT [--id UUID]
  verified head (U02 validator, blobs re-hashed) --> wrappers --> composed foreign surfaces
  (the display profile's walk) --engine::measure::measure_reference--> world-space triangles
  (units correction, then the full placement) --kernel (G06 analyze + measure)-->
  ReferenceMeasurement per wrapper
```

| Area | What exists |
|---|---|
| `beamcraft_engine::scene` | `foreign_surface`: the world-space vertices and triangle corners of a wrapper's composed surfaces, with the caps, face rules and coordinate bounds the view path always applied; `reference_element` is now a thin caller of it (display keeps its rotation refusal and translation instance). |
| `beamcraft_engine::measure` | `measure_reference` and the result objects `ReferenceMeasurement` / `MeshReading`: the kernel certification and measurement, the input signature, and the mass record that is `unsupported` by construction. Cancellation is a `&mut dyn FnMut(Stage) -> bool` polled by the kernel. |
| `beamcraft_commands::reference` / `reference_view` | `measure_references` (read-only over the verified head) and `reference_surfaces`, the extracted per-wrapper geometry walk shared by the view and the measurement. |
| `beamcraft-cli` | `measure-reference`: one JSON object, `--id` optional, no ids consumed, no revision created. |

## Decisions

- **Where results live.** A measurement is a reading of the head, returned to the caller and
  printed; it is not authored into the revision, and nothing is written to a foreign prim or the
  source layer. Persisting measurements as objects is W03, which can build on these result
  objects (`reference_id`, `source_digest`, `mesh_digest`).
- **Verification and tolerance policy (G08 tie-in).** The kernel's own policy, whole: the
  composed surface is certified exactly (closed, 2-manifold, outward, non-degenerate,
  self-intersection scan) before any value exists, and values are outward-rounded enclosures of
  exact integer sums. There are no body inputs, so the analysis uses the kernel's vertex-rule
  tolerance (`analyze`), not a plate or member tolerance. A budget refusal is `uncertain`, never
  an exact status. The engine read path through kernel certification and measurement is real and
  end to end (API and CLI); G08 itself stays open, its accepted scope is wider than measurements.
- **Units and transform respected.** The measurement maps every source vertex through
  `source_point_to_world`: the explicit units/up-axis correction first, then the wrapper's full
  rigid placement. Unlike the display path a rotated wrapper is measured where it stands. The
  uncorrected source is taken as stored, exactly as R01 reads it.
- **Open or self-intersecting is never exact.** `open`, `non_manifold`,
  `inconsistent_orientation`, `inward`, `degenerate`, `overlapping` and `uncertain` report the
  status, the bounds and the mesh digest, and `null` for volume, area and centroid. No partial
  or "best effort" number exists.
- **Mass never invented.** A foreign surface has no density input, and F06 section 8 gives a
  mesh from another source no engineering quantity: the mass record is `unsupported` with no
  value by construction, so the result states it instead of leaving mass silently absent.
- **Deterministic.** The measurement is a pure function of the pinned bytes, the correction and
  the placement; the command reads the verified head and writes nothing, so the same head always
  answers with byte-identical output. No wall clock and no fresh identity are involved.
- **Same supported profile as the view.** Translated foreign prims, `Cube` and
  triangle/convex-quad `Mesh` cages with explicit `subdivisionScheme = "none"`, visibility and
  activity honoured, the U01/display budgets enforced. A source outside the profile refuses the
  measurement (`UNSUPPORTED_GEOMETRY` or the specific cap code) instead of measuring a part
  silently. A wrapper with no supported surface reports `mesh: null` — no measurement, not a
  zero.
- **Input signature.** `mesh_digest` is the SHA-256 of the world-space BCMESH01 triangle blob
  the numbers come from; it changes when the source, its correction or the placement moves the
  geometry. `source_digest` and `source_version` name the pinned snapshot.

## Negative cases (tests)

| Case | Result |
|---|---|
| Open surface (two triangles, one shared edge) | status `open`, no values, bounds present |
| Two interpenetrating closed shells | status `overlapping`, no values |
| Unknown wrapper id | `NOT_FOUND`, exit 2, nothing written |
| Pentagon faces (outside the profile) | `UNSUPPORTED_GEOMETRY`, nothing written |
| Cancelled kernel call | `CANCELLED`, no measurement |
| Wrapper with no supported surface | `mesh: null` (no measurement, not a zero) |
| Over-budget geometry | `LIMIT_EXCEEDED` |

## Evidence

* `crates/engine/src/measure.rs` unit tests (9): cube values inside the exact enclosures;
  correction + quarter-turn + translation reaching volume, centroid and bounds; a shifted
  foreign prim; open; overlapping; disjoint shells certifying and summing; no-surface;
  determinism and sensitivity to a move; cancellation.
* `apps/beamcraft-cli/tests/reference_measure.rs` (6, real binary): metres/Y-up source measured
  at 6 m3 with byte-identical repeated output, a move changing the digest and bounds but not the
  volume, a rotated wrapper measured where it stands, open and overlapping statuses without
  values, mass `unsupported` everywhere, `NOT_FOUND` on an unknown id with the store listing and
  the head unchanged, id-ordered results with `--id` selection, and a profile refusal writing no
  byte.
* The R01 view tests pass unchanged over the shared `foreign_surface` core.

## Not done and limits

- **R03 is not here.** No source re-read, version comparison or update command; the pin and the
  recorded correction digest are ready for it.
- **W03 is not here.** Nothing is stored; a later task can persist these objects and make a
  changed `mesh_digest` a staleness mark.
- The profile, budgets and refusals are the R01 display profile's; raising them (larger USD
  profiles, curved surfaces, materials) is a separate decision. Foreign materials, normals and
  purposes beyond `default`/`render` are ignored exactly as for the view.
- One measurement per wrapper over its whole composed surface; per-prim or per-shell selection
  is not offered.

## Provenance

Original code and tests, MPL-2.0. The foreign models in the tests are invented text written by
the tests; nothing of any product or sample is used. Kernel mathematics as in G06 (divergence
theorem, exact integer sums, shell certification). No new dependency.
