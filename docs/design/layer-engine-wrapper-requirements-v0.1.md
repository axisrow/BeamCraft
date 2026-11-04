# F11: layer-engine wrapper requirements, version 0.1.0

Date: 2026-10-16. Status: **planned acceptance requirements; no wrapper implemented here**.
The owner selected variant C (pure-Rust `openusd` behind the BeamCraft wrapper, `docs/plan/decisions.md`).
These duties close gaps found by the N01 spike (its report is removed).
They do not amend F04/F06 or admit dependencies. The engine runs in the calling process (owner
decision of 2026-10-20, U17). U01-U04 implement the duties; N03 supplies platform and available-oracle evidence (delivered 2026-11-05: [the differential report](usd-sdk-differential.md), acceptance pending owner review).

## Boundary and evidence rule

Expose only BeamCraft identities, typed values, contexts, candidate tokens and bounded errors.
Keep `openusd` types, writable handles and raw path setters private. The host retains
engineering authorization, validation, serialized acceptance, publication and durable outcomes.
Each requirement below needs original positive, negative and boundary tests through the real
wrapper. An upstream claim, spike stub or same-crate round trip alone is insufficient.

| ID | Required behaviour | Acceptance evidence | N01 trace |
|----|--------------------|---------------------|-----------|
| LEW-01 | Refuse every mutation of a foreign prim, including stronger opinions in owned layers, relationships, lifecycle edits and diff replay. Derive ownership from every contributing composition source and the host-selected owned-layer set; require an owned edit target and full matching UUID/path/`bim:id`. Missing provenance cannot grant write rights. Expose no bypass handle. | A raw-crate foreign override succeeds in a control fixture; every wrapper route returns `FOREIGN_READ_ONLY`, with unchanged owned/foreign bytes and generation. Cover reference, variant, payload, inherit and specialize paths; unavailable provenance yields a safe dependency failure. An owned attachment may move without editing its foreign child. | Section 3, finding 5, C-G04: crate permits a local foreign override; spike wrapper rejects only the tested reference slice. |
| LEW-02 | Validate numeric values before exposing engineering inputs/results or preparing owned opinions. Reject NaN, infinity and overflowing numeric text with `NON_FINITE_NUMBER`; enforce F04 caps and F06 finite/range/scale validation in their written order. Preserve unknown content inertly without interpreting it as engineering data. | Reproduce `1e999999999` becoming infinity in the raw crate, then reject it through open/read and edit boundaries. Test NaN/negative infinity, finite cap excess, zero/negative dimensions, inclusive minima and cap neighbours. Verify F06 stage precedence, no clamping/healing, no partial result or generation change. | Section 5, finding 3 and `numeric-overflow.usda`, C-G08: successful parse does not imply finite engineering input. |
| LEW-03 | Bound bytes, decompression, declared counts, arrays, properties, prims, traversal/depth, resolved layers/bytes, cumulative allocation and work before trusting input. Use checked arithmetic, cycle detection, deadlines/cancellation and bounded diagnostics. Opaque parser allocation must be bounded or the affected input/profile remains unavailable. | Publish an explicit reviewed stage-budget profile. Exercise each cap below/at/above, malicious count arithmetic, nesting/cycles and decompression expansion. Demonstrate enforcement before allocation/dispatch; process caps alone are insufficient. Run hostile tests under enforced platform containment with zero panic/crash/hang/escape; retain minimal regressions for findings. | Section 5, findings 6-9, C-G08: 64 KiB corpus was cheap; depth 64 accepted, huge-count input did not test allocation, Windows-only supervision. |
| LEW-04 | Verify complete manifest/digest integrity before parsing admitted assets; resolve only host-authorized contained local bytes. Reject missing/mismatched content and path escape; no model-triggered network, plugins, scripts or executables. A parser accepting a prefix cannot certify a complete revision. | Mutate/truncate trusted USDA/USDC while retaining the expected digest and require `INTEGRITY_ERROR` before stage exposure. Test empty valid layers separately from missing revision content, path aliases/escapes and cumulative resolution. Verify foreign bytes remain identical and no external action occurs. | Section 5, finding 4 and unrun resolver/cycle tests: valid-prefix truncations and 28 USDC header corruptions were accepted; section 3 resolver was trusted-fixture-only. |
| LEW-05 | Save only validated owned authored layers from an isolated candidate; retain unknown opinions, metadata, relationships and layer structure. Fixed identities, values, composition context, backend pin and serialization profile must yield deterministic own bytes/digests. Export never constitutes host acceptance or root publication. | Run 20 saves with fixed IDs and seeded insertion permutations; reopen from fresh state and compare exact authored binary64 values, unknown fields, relationships, bytes and SHA-256. N03 compares native-platform digests and independent-reader results; cross-engine bytes need not match. Failure preserves the live view and all source bytes. | Sections 2/7, C-G05/G09: tested unknown fields survived, one second write matched within one build; reordered repeats/platform digests were not run. |
| LEW-06 | Translate supported upstream diffs into typed authored absence/block/value/removal and normalized `(full UUID, attribute)` keys. Reject unsupported diff forms. Derive F04 semantic read/write/relationship footprints in the engine; key inequality alone cannot prove independence. Pin base/generation/op_id; preserve rejected intent, idempotency and stale/gap invalidation. | Test same-field collision, independent label/placement, distinct shape-field edits sharing the shape-input read group, lifecycle/delete versus dependent read, missing incoming-link index and unsupported diffs. Replay at least 100 deltas; duplicate op_id changes no generation, stale/gap requires resnapshot, rejected local edits survive. No direct replay bypasses LEW-01/02 or host acceptance. | Section 4, C-G06: only field-set capture/replay and key intersection were shown; footprints, typed absence, replay-once, gaps/staleness and 100-delta sequence were not run. |
| LEW-07 | Prepare one isolated candidate against a complete pinned context; only the host records acceptance. Install after proven acceptance; discard only before decision or after proven rejection. Invalidate stale views/cursors/indexes on context changes. Return bounded safe errors and preserve uncertainty after loss; no automatic mutation retry or silent SDK fallback. | Inject failure/cancel before prepare, after prepare, around host recording and before/after install. Assert unchanged live state before acceptance, one generation increment on acceptance, rejection-preserving discard, and host-ledger QueryOutcome available without backend. A backend acknowledgement never proves acceptance; unresolved outcome is `OUTCOME_UNKNOWN`. Full durable publication/recovery tests remain U06/U07. | Sections 4/8 and "What BeamCraft would have to write on top", C-G06/G07/G10: host acceptance/recovery and independent-reader evidence were absent. |

## Numeric and execution constraints

F06 sections 2, 4, 7 and 10 supply operation order, scale derivation, encoding and existing limits.
`eps_geom=max(0.001 mm, 2^-22*L)` uses element-local body size; test comparison and pick bands
remain absolute as specified. Never substitute upstream math, a guessed tolerance or a changed
validation order. Small fixed-size f64 types are BeamCraft-owned in `geom`; no glam/nalgebra,
general orientation or quaternion is added. Plates stay as drawn and members roll only about
local Z. Frozen-contract alignment is an integration prerequisite, proposed in the PR only.

F11 section 7's 16 MiB/layer, 64 MiB/resolved bytes, 10000 prims, 100000 properties,
1000000 array scalars, depth 32 and 128 resolved layers are **experimental proposals**, not a
production allowance. U01 must review and test actual parser/stage budgets without widening any
F04/F06 maximum. The smaller applicable cap always wins.

In-process execution keeps every safety obligation of LEW-03: a refusal is an ordinary
`Diagnostic`, never a panic, and allocation and work stay charged against the budgets before
they happen. The project's own layers are verified by SHA-256 before they are parsed.

## Provenance and completion

Contributor-original requirements, MPL-2.0, derived from the owner card, the cited N01 synthetic
findings and repository F04/F06/F08 responsibilities. No external code, schema, test corpus,
model or asset is included. All LEW requirements are unimplemented acceptance gates in this
record; N01 first-slice evidence passes none of them in full.
