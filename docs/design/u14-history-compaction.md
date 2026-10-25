# U14 history compaction and garbage collection

Date: 2026-10-19. Status: **experimental vertical slice; U14 acceptance is limited as listed below.**
Branch `feat/u14-history-compaction`, based on `origin/master` `e2d5df2`.

Plan row: U14 / M, depends on U06, U07 (and P06, C02 for what must not be disturbed). Result: the
revision store can be measured and shrunk. Acceptance: after a compaction the head opens, its
meshes and quantities are recomputed equal, ordinary OpenUSD readers still read the store, and an
interruption at any step leaves the old or the new state, never a mix.

## What is implemented

```text
beamcraft inspect-history   --store ROOT [--workspaces-root DIR]... [--workspace-dir DIR]...
beamcraft compact-history --store ROOT (--keep-head | --keep N) --dry-run
                          [--workspaces-root DIR]... [--workspace-dir DIR]...
                          [--ledger-tail N] [--orphan-grace-secs N]
beamcraft compact-history --store ROOT (--keep-head | --keep N) --yes
                          (--backup DIR | --no-backup)
                          (--workspaces-root DIR | --workspace-dir DIR | --no-workspaces)
                          [--ledger-tail N] [--orphan-grace-secs N]
```

| Area | What exists |
|---|---|
| `beamcraft_host::compaction` | `stats`, `plan` (read-only), `compact` (real run), `Request`, `Keep`, `Pin`, `Plan`, `Compacted`, `Blocker`, `Safeguard`, `CompactionHook` (fault-injection points), `CompactionError` with F04 codes. |
| `beamcraft_host::lease` | `StoreLease`: shared and exclusive advisory lock on `maintenance.lock` in the store root. Commits, recovery and a serving coordinator hold the shared lease; compaction needs the exclusive one. The operating system releases it when the holder exits or crashes. |
| `beamcraft_host::revision` | The compaction **floor** (below), `RevisionStore::first_sequence`, `RevisionError::Compacted` and `StoreBusy`. Every head-log reader (`head`, `open`, `info_at`, `open_at`, `sequence_of`, replay, recovery) is floor-aware. |
| `beamcraft_host::ledger` | Rotation primitives; a ledger record below the floor is a tombstone; `query_outcome` proves "not found" over the retained log. Recovery also removes abandoned `ledger/.tmp-*`. |
| `beamcraft_host::storage` | Sized blob listing, blob removal, atomically replaced files (all private to the host crate). |
| `beamcraft_cli::history_compact` | Workspace pins (`workspace.json`), the verified backup, the coordinator lease, the refusal to compact a workspace's own draft store. |
| CLI | The two commands above, JSON output and the existing exit codes (2 refused, 3 store in use). |

### The compaction floor

Records of the head log keep their sequence numbers; the head is never renumbered and keeps its
UUID and manifest. After compaction the log is the run `floor..=head`. The lowest record's manifest
still names a base that no longer exists, and that manifest is immutable (its digest is the revision's
identity), so the base cannot be rewritten. `revisions/floor` states the floor instead (sequence,
revision id, manifest digest, same canonical form as a record). With a marker the log is valid iff
the records `floor..=head` are contiguous, the record at the floor equals the marker, and the
floor's missing base is accepted. Without a marker every U06 rule applies unchanged. A build that
does not know the marker refuses the log (`TORN_ROOT`), so an old binary can never misread a
compacted store. A removed revision reads as `RevisionError::Compacted` (`NOT_FOUND`).

### What `compact-history` does

- Keeps `--keep-head` (the head) or `--keep N` (the newest N, `1..=100000`; more than the log holds
  keeps all of it; `0` and absurd counts are `INVALID_ARGUMENT`). The floor never moves down.
- Deletes only blobs that no retained revision references: manifest and layer blobs of removed
  revisions, and blobs no revision references at all (left by failed commits) once they are at
  least `--orphan-grace-secs` old (default 3600), so a blob stored a moment ago for a commit that
  has not happened yet is not taken away. Layers of retained revisions are never touched; R01
  reference entries (role `external`, the foreign layers) are carried by every revision, so they
  stay byte for byte.
- Verifies every retained revision completely before any change (record, manifest, base chain in
  the retained run, every layer blob by digest and size); a damaged or missing retained blob
  refuses the run (`INTEGRITY_ERROR`). Damage in a revision that is being removed does not.
- Rotates the outcome ledger: the records of removed revisions are dropped, the records of every
  retained revision are always kept (owner decision 2). `--ledger-tail N` keeps the records of the
  N revisions just below the floor as tombstones: such an operation id is answered `Committed` by
  the ledger and a retry is `OP_ID_REUSE` (its revision is gone). Operation ids beyond the tail are
  no longer recognised as accepted (accepted by the owner); they read as "not found", like a new
  one. Damaged ledger lines are kept as they are. A single genesis line records that the ledger no
  longer covers the early history.
- `--dry-run` computes the same plan and changes nothing, not even the lock file. It reports the
  bytes freed, the revisions removed, the record files and blobs removed, the ledger effect and
  exactly what undo history is lost: every removed revision (sequence, revision id, operation id)
  and the oldest sequence that can still be restored. It also lists `blockers`.

### Owner decisions, as implemented

1. **Undo beyond the compaction point is lost.** The dry run shows it; a real run needs `--yes`
   (without it the command is refused before the store is opened).
2. **Ledger rotation forgets old operation ids**, keeping a configurable tail
   (`--ledger-tail`, default: nothing beyond the records of retained revisions).
3. **The head keeps its UUID and sequence number.** No renumbering (the floor marker).
4. **A backup is required.** `--backup DIR` copies the whole store while the exclusive lease is
   held and verifies the copy (same files and sizes, control files byte for byte, the copy's head
   opens with every blob verified); the destination must be absolute, absent or empty, and neither
   inside nor around the store. `--no-backup` must be given explicitly, together with `--yes`.
   A failed backup removes the partial copy and changes nothing.

### Refusals (nothing is changed, the dry run lists them as `blockers`)

| Situation | How it is found | Code |
|---|---|---|
| Unresolved intent in the ledger | U07 ledger scan: open intents. Run `recover-revision` first. | `DEPENDENCY_CONFLICT` |
| Active coordinator, a running commit, a recovery or another compaction | The exclusive store lease is not granted. `serve-coordinator` holds the shared lease for as long as it serves; it refuses to start while a compaction runs. | `BACKEND_UNAVAILABLE` |
| A live workspace is based on a revision that would be removed | `workspace.json` of the declared workspaces: the base revision's log sequence is below the new floor. | `DEPENDENCY_CONFLICT` |
| A workspace is declared but unreadable | It cannot be protected. | `BACKEND_UNAVAILABLE` |
| `--store` is a workspace's draft store | `workspace.json` in the store root; the draft's undo history lives there. | `INVALID_ARGUMENT` |
| Damaged or missing retained blob, torn log or ledger | Full verification before the first change. | `INTEGRITY_ERROR` |
| No confirmation, no backup decision, workspaces not declared, both or neither of the exclusive options | Argument checks before the store is opened. | `INVALID_ARGUMENT` |

A workspace whose base revision is not in the log (another project, or already compacted) is
listed with `base_sequence: null` and does not block: there is nothing to protect.

### Crash safety

The steps, in order: stage the floor marker, **install it with one atomic rename (the commit
point)**, remove the records below the floor, replace the ledger (staged, then one rename), delete
the unreferenced blobs, reopen the head. Before the commit point the store is exactly the old one.
After it the store is the new one: records below the floor are ignored by readers, a ledger record
below the floor is a tombstone, a blob nobody references is harmless. So an interruption at any
step leaves a store whose head opens and whose retained revisions are complete, and running the
same command again finishes the work and then reports nothing to do. A repeated run changes no byte.
Abandoned `.tmp-*` files are removed by `recover-revision`.

## Evidence

- `crates/host/tests/compaction.rs` (18 tests, real filesystem, no mocks): statistics; a dry run
  leaves every byte untouched and predicts the real run exactly; refusal without confirmation;
  `keep-head` and `keep N` (head id, sequence and manifest unchanged, exactly the retained blobs
  left, the foreign layer byte for byte, the next commit chains on the compacted head); `0` and
  absurd counts; workspace base, unfinished intent, active lease (and commits and recovery refused
  under the exclusive lease); damaged and missing retained blobs, an unreadable removed revision;
  the orphan grace period; ledger rotation, tombstones and "not found" after rotation;
  idempotence (a repeat changes no byte); a failed backup; **a crash injected at every one of the
  23 steps** (staging, commit point, each record, ledger, each blob): the head is identical, the old
  history reads entirely before the commit point and the new one after it, recovery reports no
  disagreement, a rerun reaches the same final records and blobs, and the store takes a new commit;
  torn, forged and gapped floor markers; leftover records below the floor.
- `apps/beamcraft-cli/tests/history_compact.rs` (8 end-to-end tests, one process per call, real
  plates and a real foreign reference): statistics against the files on disk; the dry run is
  byte-identical and lists the lost undo points; every refusal leaves the store byte-identical; a
  real compaction with a verified backup keeps `open-revision`, `inspect-elements` (meshes and
  quantities) and `inspect-reference` equal and the foreign blob unchanged, then accepts a new
  plate and a repeated run changes nothing; a live workspace pins its base; an unfinished intent
  (made by killing the real commit hook) blocks until `recover-revision`; a running
  `serve-coordinator` process blocks and then releases; a damaged retained blob refuses.
- The existing host and CLI suites pass unchanged, except for the documented change that an
  operation absent from a compacted ledger is "not found" (U07 semantics are identical for a store
  that was never compacted).

## Provenance

Original code. Behaviour follows the repository's U06/U07 design (head log, ledger, recovery),
P06 (workspace metadata), C02 (coordinator) and the owner decisions above; the store-lock and
rename primitives are `std` only (`File::try_lock`, `fs::rename`). No asset, no new dependency,
no network access, no Tekla/Trimble material, no third-party code.

## Not done

- **Retention policy.** Proposals such as "keep the last 50 revisions plus the newest of each day"
  are not implemented and nothing compacts automatically; the operator chooses `--keep`. (N = 50
  is only a proposal.)
- **U15** per-element derived layers, a UI, tags or milestones beyond what exists.
- Directory-entry crash durability (as in U06/U07): a power loss can lose a rename.
- The lock is advisory and local: a process that ignores it (a raw file copy, an older build that
  does not take the shared lease) is not stopped, and a network share may not honour it.
- A blob stored by `put-blob` and not yet committed is only protected by the grace period.
- Workspaces are found only where the operator says (`--workspaces-root`, `--workspace-dir`); there
  is no registry in the store. A compaction does not touch a workspace's own draft store.
- Commands that read an earlier revision of a published store by sequence (for example the replay
  check of `create-level` or `create-weld` at the floor revision, `apply-changeset` replays of
  compacted operations) report `NOT_FOUND` for a removed one; this slice does not change them.
- The OpenUSD reference reader (`pxr`) is not installed here. The layers are never rewritten, so
  their bytes are those that were read before; the optional `pxr` oracle tests were not run.
- Linux, macOS, wasm32 and `cargo deny` were not run here (CI covers them). `File::try_lock` is
  Rust 1.89 and later; the workspace requires 1.96.
