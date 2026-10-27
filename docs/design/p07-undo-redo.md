# P07 undo and redo of the draft

Date: 2026-10-18. Status: **experimental vertical slice; P07 acceptance is limited as listed below.**
Branch `feat/p07-undo-redo`, stacked on `feat/p06-workspace` (PR #68). Merge order:
#63, #64, #65, #66, #67, #68, then this PR.

Plan row: P07 / M, depends on P06. Result: durable undo and redo of the draft; restored identity,
links and quantities. Acceptance: undo of a copy and of a change is atomic; the stacks survive a
restart.

## What is implemented

```text
beamcraft undo-workspace    --workspace-dir DIR [--revision-id UUID] [--op-id UUID]
beamcraft redo-workspace    --workspace-dir DIR [--revision-id UUID] [--op-id UUID]
beamcraft inspect-workspace-history --workspace-dir DIR
```

| Area | What exists |
|---|---|
| `beamcraft_host::revision` | `RevisionStore::info_at(sequence)` and `open_at(sequence)`: an earlier revision by log sequence with the record, manifest, base-link and blob checks of `open`. Test in `crates/host/tests/revision.rs`. |
| `beamcraft_cli::history` | `timeline` (derives the stacks), `step` (undo or redo), the `undo.log` intent file. |
| CLI | The three commands above. They need no published store. |

## Decisions

- One accepted command is one draft revision (F04 section on undo units), so a revision is an undo
  unit and no command needs to know about undo. A copy or a change made by any command is undoable.
- Undo does not rewrite history. It commits a **new** revision whose layers are byte for byte those
  of the state before the last unit: the whole state (authored, derived, stored quantities) returns
  in one verified commit, or nothing changes. The log stays linear, so publish, status and every
  verifier work unchanged. Redo commits the layers of the state that was undone.
- The stacks are derived, not stored: walking the draft log, a revision listed in `undo.log` as an
  undo or redo moves an entry between the stacks; a resolution revision (also listed there, C04
  follow-up) and a rebase candidate clear both stacks as undo boundaries; any other revision is an
  edit that pushes the previous state and clears redo. An intent is appended and synced before its
  commit, and a revision is classified by its identity, so a crash between the intent and the
  commit leaves the derivation consistent (an intent without a revision is ignored).
- A step pins the moved unit's recorded footprint as its own operation evidence (C04 follow-up),
  so a plain undo or redo keeps the draft's command interval complete; a source without usable
  evidence keeps the step evidence-less, and later planning refuses the interval conservatively.
- The commit is expected against the draft head, so an edit that lands concurrently wins and the
  step fails with `BASE_REVISION_MISMATCH` without writing a revision.
- The same `--op-id` replays a committed step, finishes an intent that has no revision (when the
  head is still the one it was recorded against), and is `OP_ID_REUSE` for the other direction or
  for an intent recorded against an older head.
- Nothing to undo (the fork point is the floor) or redo is `INVALID_ARGUMENT`.

## Evidence

`apps/beamcraft-cli/tests/undo.rs` (4 end-to-end tests, each call a fresh process): undo and redo of
a copy (the copy's id, name and mesh digest return; layers equal the earlier revision's layers),
undo of a thickness change (thickness, volume, mass and `current` freshness restored), a new edit
discards redo, undo stops at the fork, the published store is untouched; an undone draft publishes
the restored state; replay, `OP_ID_REUSE`, a crash between intent and commit finished by the same
operation, a stale intent refused; bad arguments and a damaged log.

## Not done

- Undo applies to the draft only; there is no undo of a publish (the published store is linear and
  verified; a corrective change is a new commit).
- Derivation reads the whole draft log (at most 4096 revisions) and `undo.log` is capped at
  256 KiB; a longer history needs a stored checkpoint.
- Restored relationships: elements author no relationships to each other yet, so only definition
  links are restored (they are part of the restored layers).
- Layer processing is in-process (LEW-03), as everywhere.
