# K8 acceptance evidence

This is the verification deliverable for the K8 gate verdict recorded in
[`ROADMAP.md`](../../ROADMAP.md) — "associative drawings, dimensions and marks, drafting, a
published document version", packages D01 to D08. It records the delivered scope the verdict
was taken on and the documented follow-ups that stay open as accepted boundaries. It does not
change any frozen contract or re-decide an earlier gate.

## Owner decision (2026-11-10)

The owner accepted K8 as passed on 2026-11-10. Closing the underlying cards alone would not
pass a gate; the verdict is the owner's explicit decision on the delivered scope below.

## Delivered scope

| Package | Delivered scope | Record |
|---|---|---|
| D01 drawing contract | the drawing contract with FeatureIds (`docs/contracts/d01-drawing-contract-v0.1.md`) | contract + the D01–D03 note; upstream #220 |
| D02 first projection and dimension | member projection, semantic anchors, projected dimension, explicit freshness | upstream #220 |
| D03 drawing manager and publication | headless document lifecycle, immutable published versions with archived model sources, limited SVG export | upstream #220, #409, #729 |
| D04 drawing views | placed views, per-view scales, section and detail views rendered on exports, labels, titles and title-block layout | `docs/design/d04-drawing-views.md`; the boundary record refreshed in upstream #862 |
| D05 drawing sheets | sheet catalogue, frames, title blocks filled from authored attributes, whole-sheet SVG and PDF, the sheet editor | `docs/design/d05-drawing-sheets.md`; fork PR axisrow/BeamCraft#518 |
| D06 dimensions, marks and leaders | annotation entities bound to FeatureIds; mark text from attributes; a lost feature stays unresolved, never silently re-bound | `crates/model/src/annotation.rs`, `crates/commands/src/annotation.rs`, the drawing codec |
| D07 drafting tools | drafting entities scoped to the drawing document, layers and line types | `edit-drafting`; `docs/design/d04-drawing-views.md`, `docs/design/d05-drawing-sheets.md` |
| D08 PDF and print output | the deterministic PDF page bound to the published version (upstream #729, with dash-pattern and per-version binding fixes); printer submission through the native print service — CUPS `lp` on macOS and Linux, the page on stdin, `--dry-run` exactness | fork PR axisrow/BeamCraft#563, merged 2026-11-10; the engine `print` module and the `print-drawing` command |

Cards D01–D07 are closed through the closing references of upstream #862. Card D08 (upstream
#472) carries its closing reference in upstream pull request Apibim-com/BeamCraft#863 and
closes on that merge; the owner decision of 2026-11-10 (recorded in
[`decisions.md`](decisions.md)) accepts D08's closure with CUPS-only submission.

## Accepted boundaries and documented follow-ups

The verdict accepts the delivered scope with these documented, unscheduled follow-ups:

- The printer submission is CUPS-only (macOS and Linux desktops). The Windows submission
  backend and a native print dialog are follow-ups per the 2026-11-10 owner decision; a
  platform without a submission backend refuses `BACKEND_UNAVAILABLE` honestly, and the
  Windows page itself is already the exact 1:1 PDF.
- The native drawing UI — hand placement of views and the drafting toolset inside the
  window — is tracked by the broader-drawing follow-up (upstream #333), which the same
  decision moves out of the M4 milestone.
- Section and detail views render placed views and dimension groups; titles and scales are
  carried by the sheet and view specs (D04 note).

## Provenance

No code changes; no assets, dependencies, commercial catalogue or outbound runtime access are
added.
