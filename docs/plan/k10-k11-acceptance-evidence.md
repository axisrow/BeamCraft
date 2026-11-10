# K10 and K11 acceptance evidence

This is the verification deliverable for the K10 and K11 gate verdicts recorded in
[`ROADMAP.md`](../../ROADMAP.md). It records the delivered scope the verdict was taken on and
the documented follow-ups that stay open as accepted boundaries. It does not change any frozen
contract or re-decide an earlier gate.

## Owner decision (2026-11-09)

The owner accepted K10 and K11 as passed on 2026-11-09. Closing the underlying cards alone
would not pass a gate; the verdict is the owner's explicit decision on the delivered scope
below, and the M3 milestone closes on the same decision.

## K10 — model tools, views, filters, cut plane, render modes, colour by filter, selection and snap panels

All K10 packages are closed. The cards and their delivered scope:

| Package | Delivered scope | Card(s) |
|---|---|---|
| T01 grids | grid creation and editing, member binding, pinned grids and rights (T01.5), the magnetic grid mode (T01.8) with the GR-N22 guard that refuses shape-input and placement writes of a bound element | #533, #461, #463 |
| T02 construction objects | construction objects as snap targets | #443 |
| T03 model tools | copy, move, rotate, mirror and arrays | #444 |
| T04 numeric input | numeric input and direction lock | #445 |
| V11 selection | bottom snap panel and object-type selection filter | #439 |
| V12 named views | named views, several windows, work area | #447 |
| V13 projection | orthographic and perspective switch | #440 |
| V14 filters | filters | #448 |
| V15 cut plane | cut plane | #449 |
| V16 render modes | five render modes on Ctrl+1 to Ctrl+5 | #441 |
| V17 colour | colour by filter and class | #450 |

The milestone record is the upstream milestone "M2: tools and views" (18 closed cards, none
open). The per-package delivery history is git and the merged pull requests that carry the
closing references; cards T01.5 (#461) and T01.8 (#463) extend T01 and stay inside this gate.

## K11 — user attributes and schedules

| Package | Delivered scope | Card | Delivered by |
|---|---|---|---|
| I04 typed user attributes | user attributes on any element, stored in USD under `/World/Definitions/Attributes`; an unknown attribute is shown, never dropped | #531 | #748 |
| I05 schedules | schedules from user attributes over the same verified read as the BOM export; unknown and stale values are shown honestly, the export is byte-for-byte reproducible | #532 | #750 |

Beyond the two cards, the stored schedule templates required by the owner decision of
2026-10-27 (templates as shared, owned model USD entities with revisions, footprints, commit
and undo) were completed on 2026-11-09 in fork PR
[axisrow/BeamCraft#550](https://github.com/axisrow/BeamCraft/pull/550), merged in the fork the
same day and delivered upstream through the regular delta-wave flow. The #532 acceptance
itself was satisfied by #750.

## Accepted boundaries and documented follow-ups

The verdict accepts the delivered scope with these documented, unscheduled follow-ups:

- I01–I03 (the item contract and GLB/IFC mesh-item reading) stay deferred until after the MVP,
  as `ROADMAP.md` records.
- The schedule follow-ups documented in
  [`docs/design/i05-schedules.md`](../design/i05-schedules.md): grand-total rows, unit
  conversion, assembly-level schedules, filter parity with V14, and rebase support for stored
  definitions.
- The grids contract `docs/contracts/t01-project-grids-draft-v0.1.md` stays a draft (v0.1);
  its GR-N22 acceptance wording now matches the delivered guard.

## Provenance

No code changes; no assets, dependencies, commercial catalogue or outbound runtime access are
added.
