# C06 implementation note: compensation of a published operation

Date: 2026-10-30. Status: implemented (local `compensate-workspace`, wire operation
`compensate`, client `coordinator-compensate`, acceptance C06-01..C06-07 in
`apps/beamcraft-cli/tests/compensation.rs`). This note records how the C06 slice of fork
issue #12 / #293 is built; the rows remain owned by `docs/design/c03-c06-acceptance.md`.

## What a compensation is

One ordinary, evidence-bearing draft operation that restores the target's semantic write
groups to the accepted state before the target. Nothing about accepted history changes: the
compensation commits to the workspace draft against the draft head (U06 CAS) and reaches the
accepted store only through the ordinary package-and-publish path, so every existing C01/C02
receipt, queue and expected-base rule applies unchanged.

## How the preconditions work

Two independent nets, both against the **current** published head:

1. The C04 planner runs first: the inverse operation carries the target's own footprint and
   is planned against every accepted operation after the target (renounced work contributes
   nothing; a rebase receipt contributes its whole source interval as a conservative
   superset; missing evidence is `DEPENDENCY_UNAVAILABLE`, never an empty proof). Any
   intersection is a rejected `DEPENDENCY_CONFLICT` with the same bounded report as C04,
   naming both operation identities — the later edit by another actor is named, never rolled
   back (C06-02, C06-03), and a superseded target names the operation that now owns the state
   (C06-07).
2. The layer-engine rebase materializer applies the inversion field by field and refuses when
   a field's current value is no longer the target's own result — the second net behind the
   planner, so a moved state is never silently overwritten. The candidate then passes the
   kernel/quantity verifier before the commit, exactly like a rebase candidate.

The workspace must sit on the current accepted head, and the draft must be clean
(`draft_pin.sequence == synced_sequence`): the compensation's candidate replaces the whole
draft content, so unsent work is never implicitly dropped — it is published, undone or
renounced through an explicit recorded resolution first, like the C03/C04 discipline demands
(`INVALID_ARGUMENT` names the remedy; merging the unsent delta onto the candidate like C03
is a follow-up, not an implicit loss). A moved published head after planning surfaces later
at packaging/publishing through the ordinary base checks; a compensation holds no
accepted-store lease because it never writes the accepted store.

## Identity and recovery

The compensation carries its own operation identity, bound by a durable claim fingerprint
covering the workspace, actor, operation identity, exact target pin and observed head. The
same identity replays the recorded outcome exactly once across processes and entry points
(C06-04, C06-06); a different payload under that identity is `OP_ID_REUSE`. Crash windows
are the U06/U07 commit machinery: a kill before the durable switch leaves no half state and
the retry commits once; a kill after the switch replays the recorded result (C06-05). The
compensation's own operation evidence is the target's footprint (it writes the same
semantic groups in reverse), so later rebases and plans treat it like any other command.

## Deliberate bounds

- The accepted store records no per-operation author, so "its own published operation" is
  enforced through the workspace identity chain: a compensation enters only the caller's own
  draft (the token actor must own the workspace, `PERMISSION_DENIED` otherwise, nothing
  written), and the later publication of that draft is that workspace's actor's changeset,
  which C01/C02 validate against the caller. Durable per-operation authorship would be a
  store-format change and stays out of this slice.
- The raw 4096-entry budget branches ride the existing engine caps (footprint aggregation,
  conflict report, operation interval) already proven by the C03-13/C04-09 acceptance; the
  compensation adds an interval-entry cap with the same `LIMIT_EXCEEDED` code.
- The wire answer carries `revision_id` of the compensating draft revision plus the current
  head for information; the full local revision object stays on the local command.

## Evidence

`apps/beamcraft-cli/tests/compensation.rs`: ten cases — the forward compensation published
through the real coordinator with both revisions preserved (C06-01), a delete compensation
restoring the element with its authored content (C06-01 breadth), a same-group later edit by
another actor refusing by name with B's value intact (C06-02), a disjoint later edit
surviving verbatim in accepted order (C06-02 tail), a superseded recreate target and an
unknown target and the first revision refusing typed (C06-03/C06-07), replay exactly once
and `OP_ID_REUSE` (C06-04), both crash windows with recovery and replay (C06-05), the owner
only on the wire with cross-entry-point replay (C06-06), and the unknown-field wire refusal
(C06-06/C03-R07 style).
