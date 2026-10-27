# M5 CLI delivery and acceptance

Approved scope: all engine/API and CLI capabilities of M1–M4 required by Q01,
followed by headless Q02 rehearsal on Windows, Linux and macOS. No automatic merges.
The initial verified master is `36ca241`; re-read master and open PRs before every slice.
Task cards alone are not delivery evidence. No UI/GPU acceptance is claimed.

## Reuse existing work

| Source | Reuse; do not reimplement |
| --- | --- |
| #262 | Workflow permission correction and actionlint CI; integrate first |
| #238–#243, #247, #248, #250, #252–#254 | T02/T03, personal and constructed views, clipping, I04, V14/V16/V17 |
| #251 | A12.1 thickness side |
| #257 | P12 create/edit polybeam, quantities and G12 cap feature correction |
| #259 → #260 | A14.1 definitions/libraries → A14.2 component geometry |
| #255 | V11 engine pick filters and CLI `pick --exclude`; UI remains separate |
| #249 | V13 native projection switch; no duplicate UI work |
| #237, #261 | Drawing-card status and M2 CLI acceptance documents |
| #256, #258 | Adjacent collaboration/history changes; preserve regressions without extending MVP |
| Master #223, #216, #211 | G12 sweep; A15 fillet weld slice; A16 main part, nesting and explode |
| Master #217, #213, #244 | A03/A04 creation records; D08 PDF/print pipeline |
| Existing Q01/Q02 | Frame story, drawing reopen/staleness, package layout and release workflow |

## Reviewable candidates and continuing work

These are candidate slices, not acceptance of the complete M5 CLI gate.

| PR | Delivered slice / integration note |
| --- | --- |
| #264 → #265 | A03 cut record update, removal and identity/ordinal retirement |
| #266 → #267 | A04 hole record update, removal and shared retirement history |
| #268 | Original cut creation retries use immutable accepted evidence |
| #270 | P13 circular member creation, quantities, persistence and point replacement; path-aware numbering remains |
| #274 | Reuse the existing P14 section-placement candidate; do not duplicate |
| #275 → #281 | Reuse A12.2; #281 adds complete corner-list replacement/clear, linked groups and undo |
| #279 | Repair D07 drafting test compilation and the G13 Clippy admission guard; supersedes #280 |

Verified master advanced to `b2ee4d9`: it includes A12.1, P12, G13 ruled-surface slice,
D07 drawn-paper-object slice, V11, A14.1 and adjacent history fixes. A14.2 #260 and A14.3
placement #269 merged into prerequisite branches: check master ancestry before treating
these as master delivery. The separate native V16 #276 does not accept the headless M2 gate.

A local alternate A12.2 implementation was retained only for comparison; it has no duplicate
PR. Continue from #275. Vertex/edge contour editing and P-Degenerate creation/repair remain
outstanding. #281 is a prerequisite and does not close A12.3. M2 integration, I05, remaining
geometry/connection/drawing capabilities and Q01/Q02 acceptance remain as listed below.

Every published candidate is ready for review under the owner's explicit non-draft instruction.
Full workspace or hosted checks that are running are recorded as pending in their PRs; a
successful predecessor run does not establish a later head. No automatic PR merge is authorized.

## Remaining implementation groups

1. A03/A04 record update/removal and recovery of invalid authored elements. Existing create
   commands and schemas are retained. One revision/undo per command, immutable retry evidence.
2. P13/P14, remaining A12 corner/contour editing; G13 and A10/A11; A13 cuts/fit/chamfer/
   split/combine. Use the existing kernel and operation record contracts and reject uncertified bodies.
3. A14.3 placement/editing, G14.1/A04.2 member holes, A14.4 generated holes. Use the accepted
   amendments. No G14.2 expansion, inferred fastener density or foreign catalogue data.
4. Audit the implemented A15/A16 against the scenario matrix and implement only uncovered
   requirements, including workshop-weld assembly behavior. Never recreate the existing commands.
5. Integrate M2 with new geometry. I05: owned USD schedule templates, one V14 evaluator,
   typed columns/grouping/count/sum/min/max, deterministic CSV/HTML and explicit unknown/stale.
6. D04–D07: explicit views/sections/details, A0–A4 sheets, associative dimensions/marks/leaders,
   drafting primitives/text/hatching/symbols/layers/line types; extend D08 for complete sheets.
7. Q01: a standalone real-CLI scenario runner shared with integration tests and packaged-binary
   acceptance. Full capability matrix plus the combined frame; no direct USD writes in the driver.
8. Q02: safe reproducible packaging, license/native-dependency inventory, SHA-256 manifests and
   extracted-package scenarios on all three OS targets. Manual workflow artifacts only, no release.

Each numbered group is split into focused task/slice PRs; it is not a single implementation PR.
Existing open implementations are dependencies, not permission to overwrite concurrent work.

## Acceptance

Every required behavior maps to an engine test and a real CLI process test: success, invalid input,
resource limits, replay after later edits, atomic refusal, undo/redo and restart. New geometry needs
analytic/enclosed references, certification, saved-data and quantity checks. Views/drawing filters
and clipping must use the same pinned source. Missing required scenarios never count as passed.

Run fmt check, all-target workspace Clippy, full workspace tests and doctests, docs with warnings
 denied, pure-crate WASM, dependencies, boundaries, fixtures and actionlint. Inspect workflow runs
as well as check-runs; zero-job validation failures are failures. Record exact heads and results.

Only an integrated master can accept the complete CLI gate. A stacked candidate is identified
as a candidate. Shared-model templates use inline typed filter rules with attribute UUIDs, not
personal-filter references. Personal view parameters are resolved and frozen into drawing sources.
Q02 packages retain the existing UI binary, but headless smoke does not accept the native window.
