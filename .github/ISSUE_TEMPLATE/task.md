---
name: Implementation task
about: Define one task or numbered slice with dependencies and verifiable acceptance
---

New task scope requires the code owner's approval.

**Task ID and title** (keep the existing ID; use a numbered slice for a large task)

**Milestone / scope** (current MVP, deferred after the MVP, or awaiting an owner decision)

**Depends on** (link task issues and contract sections; name unresolved owner decisions)

**Result** (the externally observable outcome)

**Acceptance** (positive and negative cases, refusals, unknown/stale/unsupported states)

**Delivery status and evidence** (open, blocked, merged, accepted, done, deferred or later;
link integrated commits, PRs, reproducible checks and remaining limits)

**Size and slices** (S/M estimate; split large tasks before implementation; the parent stays
the acceptance gate)

Runtime features need an engine API, a CLI command and an end-to-end test before UI (interactive
assistance such as picking, snapping and the start screen needs no CLI command, `AGENTS.md` §3).
Follow `AGENTS.md` for provenance, dependencies and never-crash rules. A merged slice does not
accept the whole task. Link the implementing PR with `Fixes #N` only for the scope it completes.
