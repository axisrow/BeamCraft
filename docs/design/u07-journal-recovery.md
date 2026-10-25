# U07 journal and recovery

Date: 2026-10-16. Status: **experimental engine API plus headless commands; a durable outcome
ledger, a write-ahead intent in the U06 commit, recovery of an unfinished commit and fault-injection
proofs are implemented. Personal workspaces, collaboration, ledger rotation and garbage collection
are not.** Branch `feat/u07-journal-recovery`, stacked on `feat/u06-commit-revision` (PR #57);
retarget to `master` after #57 merges.

Plan row: U07 / M, depends on U06. Result: consistent recording of intent/commit and recovery of
an unfinished save. Acceptance: fault injection before/after root replacement opens the old or the
new complete snapshot; recovery is idempotent.

## What is implemented

`beamcraft_host::ledger` (pure `std`, no new dependency, no `unsafe`, bounded):

| Piece | Behaviour |
|---|---|
| Ledger file | `ledger/outcomes.log`, append-only. One closed canonical text line per record, ending in a SHA-256 of the line's other bytes. Records: `genesis` (head sequence that existed before the ledger), `intent` (op, project, revision, sequence, base, content fingerprint, creation second, a per-attempt identity, layer blob digests), `committed` (op, revision, sequence, manifest digest), `aborted` (op, identity of the intent it closes, reason `cancelled` / `rejected` / `lost-race` / `recovered`). A parsed record is re-encoded and must equal the input. |
| Strict reading | The whole file is read under a cap (a new attempt is admitted below 16 MiB, 131,072 lines and 4,096 open intents, reserving room to close every open one; the reader accepts 1 MiB, 8,192 lines and 1,024 intents more for racing appenders and closing records; lines under 8 KiB). An unterminated final fragment is a torn append and is ignored. A complete line that is not a valid record is counted and ignored. Two different accepted records for one op are `Inconsistent`. Nothing ignored is ever acted on. |
| Commit order | validate, replay, base -> **intent appended and synced** -> blobs verified, manifest blob stored, record staged -> **root switch** (the U06 non-replacing link) -> **committed appended and synced**. A failure before the switch closes the attempt as `aborted`. If the intent cannot be written the commit fails with the root untouched. If the committed record cannot be written after the switch the result is `OUTCOME_UNKNOWN`, never success and never rollback. |
| `CommitHook` | Two points were added: `IntentRecorded` (after the intent) and `Recorded` (after the committed record, observational). A hook that unwinds simulates a crash: no error handling of the commit runs afterwards. |
| QueryOutcome | `RevisionStore::query_outcome(op)` is read-only and answers from the ledger **cross-checked against the root**: `Committed` (with source `Ledger` or `Root`), `Aborted`, `NotFound`, `Unknown` (`OUTCOME_UNKNOWN`). A ledger `Committed` that the root does not hold is `INTEGRITY_ERROR`, not an acceptance. |
| Recovery | `RevisionStore::recover(RecoveryPolicy)` closes each unresolved intent from the root: if the head log holds the intent's revision (same id, op and content fingerprint) at the intent's sequence it appends `committed`; if the root has not reached that sequence or holds another revision there it appends `aborted`; if the root cannot be verified the intent stays open and is reported. It removes abandoned `.tmp-*` (revision record) and `.publish-*` (blob) temporaries, reports every ledger `committed` record that the root contradicts (wrong revision, manifest or operation at its sequence, an unreadable or unreached sequence; all records are checked, not only recent ones), and reports unreferenced blobs without deleting any. `open_recovered` recovers and then opens the root. |
| Replay and reuse | The ledger recognises an op id beyond the head log's 64-record look-back (`Replayed`; the revision-id scan of U06 does the same for an identical retry). While an attempt is unresolved or committed, a different content or expected base under the same op id is `OP_ID_REUSE`; an attempt closed as not accepted leaves the id free. Each attempt has its own identity, so closing one attempt never closes another identical one. A replay of an op whose ledger record is missing writes the missing `committed` record. |

CLI (`beamcraft`): `recover-revision --store ROOT --min-age-secs N` and
`inspect-revision-outcome --store ROOT --op-id UUID`, machine-readable JSON, exit codes of the existing
table. Every `commit-revision` goes through the ledger.

## Decisions to review

1. **The root is the authority; the ledger restates it.** The head log decides what the root is.
   The ledger records what was attempted and what was proven, and recovery appends only what the
   root proves. There is no state in which the ledger overrides the root.
2. **Intent before everything.** The intent is written after the cheap checks and before blob
   verification, manifest storage and staging, so every on-disk trace of an attempt has an intent.
   Consequently the intent cannot carry the manifest digest (it depends on the verifier's index);
   it carries the content fingerprint (SHA-256 of the canonical manifest of the draft with
   sequence 1, no base, no index), the revision id and the sequence. Recovery identifies "the
   root points at this intent" by reading the head-log record at the intent's sequence and
   comparing revision id, op id and fingerprint; the manifest digest is in the `committed` record.
3. **No lock file.** A crash cannot leave anything that blocks later writers. Concurrency rests on
   the U06 link (the CAS) and on idempotent appends. Two concurrent recoveries may both append the
   same closing record; the reader treats a repeated identical record as one, and `committed`
   wins over any `aborted` for the same op.
4. **Recovery cannot see a live committer.** An intent is closed, and a temporary file removed,
   only if it is at least `min_age_secs` old (`RecoveryPolicy::live`). `quiescent()` (zero) asserts
   that no commit is running on the store. There is no `Default`; the CLI requires the flag. The
   committer re-checks, just before the link, that recovery has not closed its intent, which
   narrows the race (a recovery with `quiescent()` against a running committer) but does not
   remove it. A wrong clock can only make recovery act early or late; it never accepts anything
   unproven.
5. **`NotFound` is a proof, so it is rare.** It is returned only when the ledger began before any
   revision existed (`genesis 0`), has no damaged line, accounts (by `committed` records or open
   intents) for every sequence of the root, and has no record of the op. A store with revisions
   that predate the ledger, a ledger with an ignored damaged line and a ledger cut at a record
   boundary answer `Unknown` for an unrecorded op. `Aborted` is likewise only answered when the
   root does not hold the op (a retry may have switched it with its own intent lost) and no line
   was ignored. Absence of a record is never reported as "not applied" otherwise.
6. **Op id binding.** An op id is bound to its whole request (content and expected base, F04
   section 6) while an attempt is unresolved or committed; another content or base is
   `OP_ID_REUSE`. A closed (`aborted`) attempt had no effect, so a corrected retry under the same id is allowed,
   as in U06 before the ledger. This is the one place where the ledger could be stricter; it is
   not, to keep U06's behaviour for a rejected-then-fixed command.
7. **`inspect-revision-outcome` is the QueryOutcome.** The ledger answers it; U17 removed the older
   `query-outcome` command, which could only return `OUTCOME_UNKNOWN`.

## Crash points and what recovery yields

The tests simulate a crash by unwinding from the real commit hook at each point, so exactly what
was synced before it remains, with a fresh handle, real files and no storage mock. For each point
and with or without a previous revision they check that a fresh reader sees one complete snapshot
before any recovery, that `query_outcome` claims no more than is proven, that recovery yields the
old or the new complete snapshot, that the ledger then agrees with the root for every revision,
that a second and third recovery change nothing (the ledger bytes are identical), and that a retry
of the same op id is `Replayed` if the root switched and `Created` exactly once otherwise.

| Crash at | Ledger | Root | Before recovery | Recovery |
|---|---|---|---|---|
| `Checked` | no trace | old | `NotFound` | nothing to do |
| `IntentRecorded`, `BlobsVerified`, `ManifestStored`, `RecordStaged` | intent | old | `Unknown` | `aborted` (manifest and layer blobs stay, reported unreferenced) |
| `Switched` | intent | **new** | `Committed` (source root) | `committed` |
| `Recorded` | intent, committed | new | `Committed` (ledger) | nothing to do |

A torn ledger tail is tested at every byte offset of the last record, both a `committed` record
(root new: recovery rebuilds it) and an `intent` record (root old: no complete intent means no
switch, so the answer is `NotFound`; the whole intent is `Unknown` until recovery aborts it).

## Guarantees and non-guarantees

* Promised: the intent is on stable storage (file `sync_all`) before the root switch; the root
  always opens as one complete revision (U06); a torn, truncated, garbage or flipped record is
  ignored and counted; recovery is idempotent and leaves nothing that blocks writers; outcomes
  never claim acceptance that the root does not hold; no blob is deleted.
* Not promised: **directory-entry crash durability.** Files are synced, directories are not. After
  power loss the ledger file's directory entry or the root record's link may be missing although
  a commit was reported; the root is then still a complete earlier revision, and a ledger record
  that outlives its root record is reported (`disagreements`, `INTEGRITY_ERROR`) instead of
  trusted. Protection from a hostile writer to the store directory: the per-record SHA-256 is an
  integrity check against accidents, not authentication; whoever can write the file can forge a
  line, and the answers are still cross-checked against the root.
* Not implemented: ledger rotation or compaction (a full ledger refuses new attempts with
  `LIMIT_EXCEEDED` before writing, with room kept to close every open intent; about 20,000
  commits fit). The length check and the append are not atomic across processes, so racing
  appenders can overshoot the admission limit; the headroom absorbs that, and beyond it the
  ledger fails closed (every read is `LIMIT_EXCEEDED`) instead of being partially trusted;
  garbage collection of blobs (needs
  retained revisions and drafts accounted for); personal workspaces and preserved drafts across
  restart; multi-client collaboration; a CLI switch that injects faults (the E2E tests produce the
  interrupted commit through the host hook and then run the real binary).

## Provenance and limits

Original code and fixtures; formats derived from F04 section 8 and the plan, and
the head-log design of U06. No asset, no third-party dependency, no Tekla/Trimble material.
Caps: record line 8 KiB; a new attempt is admitted only below 16 MiB, 131,072 lines and 4,096 open
intents, with room reserved to close every open intent; the reader and closing records accept
1 MiB, 8,192 lines and 1,024 open intents more (headroom for racing appenders). 64 layer digests
per intent, unreferenced-blob scan 4,096 revisions and 262,144 blobs (otherwise "not computed"),
1,024 digests per report (the CLI prints 256; the total is exact). Linux/macOS, wasm32 and
`cargo deny` were not run (not installed here).
