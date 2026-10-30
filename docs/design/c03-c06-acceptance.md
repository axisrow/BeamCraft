# C03-C06 acceptance specification beyond the local rebase

Date: 2026-10-25. Status: **acceptance specification; the runtime is deferred after the MVP by
owner decision. The C05 rows below are executable tests as of 2026-10-27
(`apps/beamcraft-cli/tests/offline_clients.rs`), at the largest scale the default suite carries —
the per-commit verification is O(revisions), so the full 1000 + 1000 sweep stays out of it. The
authenticated coordinator rebase (C03) is executable as of 2026-10-30
(`apps/beamcraft-cli/tests/coordinator_rebase.rs`), and compensation (C06) as of 2026-10-30
(`apps/beamcraft-cli/tests/compensation.rs`). The C04 semantic-conflict workflow remains
specified only.**
Scope: fork issue #12 and fork issue #160 (upstream #331). The local rebase acceptance cases
C03-01 to C03-14 in `docs/design/c03-rebase.md` stay required unchanged. This document specifies
the remaining per-slice acceptance: the authenticated coordinator rebase (C03), the
semantic-conflict workflow (C04), retry across publication (C05) and compensation of a published
operation (C06). It completes no package and changes no implementation-plan status.

Baseline inspected: fork `master` `c1d34913871255dc2cc26b5ebcaf3b18afa044e9`. The coordinator wire
speaks only `head` and `publish` (`apps/beamcraft-cli/src/coordinator.rs`); rebase exists only as
the local `rebase-workspace` CLI operation. Everything in the tables below is therefore required
behaviour to build and verify, not a claim about current code.

## How to read the tables

Every row is a required executable acceptance case. Rows marked *coordinator* run a real
`coordinator-serve` process with real token files, over the real wire. Every case asserts content
(values, digests, operation identities, report bytes), never only an exit code. Two invariants
from issue #160 hold in every row, and each slice's test must assert them explicitly:

- **No lost writes.** After any refusal, retry, kill or recovery, every accepted operation and
  every unsent draft operation is present with its authored content. A compensation never removes
  another actor's later edit.
- **Deterministic operation outcomes.** A repeated request identity with the same payload replays
  its recorded outcome exactly once, through any entry point (local CLI or coordinator), in any
  process. A different payload under that identity is refused. Pure steps (planner, conflict
  report, compensation precondition check) give byte-stable output for equal inputs.

All adversarial input ends in a typed result or refusal under the existing caps; no hang, no
partial state, no `unwrap` (`AGENTS.md` §4). Crash-window rows kill the real process at each named
boundary, restart, and compare complete states, in the spirit of U07.

## C03 remainder: rebase through the authenticated coordinator

`docs/design/c03-rebase.md` defines the rebase semantics and the local acceptance. The remaining
C03 scope (issue #160) is reaching the same semantics through the coordinator, where the
token-derived actor is the only trusted actor; a claimed author field grants no authority, as
C02 established for `publish`.

| Case | Required observable assertions |
|---|---|
| C03-R01 rebase operation on the wire *coordinator* | A coordinator rebase request (workspace name, expected published head, request identity) performs the C03-01..C03-14 semantics end to end: the answer carries the same typed result as the local operation and the new draft head; the published head is unchanged by a rebase, and the client still needs an explicit `publish` to move it |
| C03-R02 trusted actor only *coordinator* | The authenticated client's actor is the rebase actor. A request naming another actor field, or an authenticated actor that does not match the workspace's user identity, is refused with nothing written; the answer does not depend on the claimed field. The resulting changeset keeps the workspace actor as author, and a stranger's token cannot publish it |
| C03-R03 authentication before any state read *coordinator* | Unknown token, short token and (Unix) a group/world-readable token file are refused before any workspace state is read; head and both drafts are unchanged afterwards |
| C03-R04 interleaved publication *coordinator* | From one base: A publishes; B rebases through the coordinator; a third publication moves the head before B publishes. B's publish with the stale expected head is `BASE_REVISION_MISMATCH` carrying the current head; B repeats rebase, then publish, and succeeds. The store ends with A's and B's edits in accepted order, each exactly once; no accepted head is overwritten |
| C03-R05 concurrent rebase of one workspace *coordinator* | Two concurrent rebase requests with the same workspace and base: the metadata CAS admits exactly one merge; the other replays the recorded result or receives a typed refusal. Killing the loser at any point leaves the winner's outcome and never two different merged draft heads |
| C03-R06 coordinator crash windows *coordinator* | Kill the client after the server accepts the request but before the answer, and kill the server between draft commit and answer: retry with the same request identity replays the recorded outcome exactly once; the draft is either the old complete draft or the complete merged draft, and unsent operations are never lost |
| C03-R07 bounded wire behaviour *coordinator* | Oversized frame, wrong protocol, unknown operation or field, full queue: the C02 typed codes (`LIMIT_EXCEEDED`, `UNSUPPORTED_*`, `BACKEND_UNAVAILABLE`) with nothing written; loopback-only binding and off-by-default remain as in C02 |
| C03-R08 identity across entry points | The same rebase request identity submitted once through the local CLI and once through the coordinator returns the recorded result; a second merge under that identity never happens |

## C04 semantic-conflict workflow

Plan row C04: same property, atomic groups, read dependencies, delete/link. Acceptance: a
conflict keeps both sides; no silent server-wins. The conflict report is data produced by the
pure planner: for each conflict, both operation identities, the intersecting target/group pairs
and the kind (write-write, read-write, lifecycle), in a deterministic order.

| Case | Required observable assertions |
|---|---|
| C04-01 write-write, same group | A and B change the same field of the same element; A publishes; B's rebase returns a typed conflict naming both operations and the target/group. B's draft keeps B's value, A's published value is unchanged, and after B's explicit resolution the final published value is the predicted one |
| C04-02 atomic shape inputs | Thickness versus contour on one element yields one conflict on the atomic shape inputs, not separately resolvable property conflicts; no half-applied shape is ever produced |
| C04-03 read-write dependency | A changes a group of the copy source; B's copy rebase conflicts even though B's written fields are disjoint from A's change (the read side of C03-04) |
| C04-04 delete/link lifecycle | A deletes X while B adds a weld linking X (or B moved X): a lifecycle conflict; both sides are kept and no dangling link is created or silently dropped |
| C04-05 no silent server-wins | With an unresolved conflict recorded, status, packaging, local publication and coordinator `publish` all refuse the draft until an explicit resolution exists; each refusal names the pending conflicts and replays identically on every call |
| C04-06 reverted intervening edit | An intervening accepted edit that conflicts and is later reverted (final value equals the old base) still conflicts: final-value equality never hides accepted history (conflict side of C03-05) |
| C04-07 crash windows | Kill before and after the planner at every boundary: a conflict rebase writes nothing, so after restart the same report is produced byte-stably; neither head nor either draft moved; re-reading the draft proves no half-applied plan exists |
| C04-08 two clients resolve differently | B and C fork from one base and hit the same published conflict; B explicitly discards its own operation, C re-edits. Each resolution is a new operation identity; both final states are the predicted ones and independent of resolution timing |
| C04-09 bounded reports | Conflicts at the 4096 cap produce the complete report; cap plus one is `LIMIT_EXCEEDED`; a report is never silently truncated |
| C04-10 explicit discard is durable | Discarding a named own unsent operation is a recorded operation with its own identity: a crash during discard and a restart leave it done or not done exactly once; the discarded operation never reappears, the other side is untouched |

## C05 retry and two offline clients

Plan row C05: harness for 1000 + 1000 IDs, retries, crash windows and two workspaces.
Acceptance: no path collisions; the accepted snapshot is readable by OpenUSD.

| Case | Required observable assertions |
|---|---|
| C05-01 1000 + 1000 offline | Two workspaces create 1000 elements each against one base; publish A, rebase B, publish B; the store reopens with 2000 elements with the expected values; element and operation IDs are distinct across workspaces; the final accepted revision opens in the ordinary OpenUSD distribution (additional evidence, as C03-14) and in the independent reader |
| C05-02 retry path is explicit and bounded | After `BASE_REVISION_MISMATCH` the only forward path is rebase, repackage under a new changeset identity, publish with the observed head; a retry that skips rebase keeps failing; a bounded retry budget ends in a typed result and never spins unboundedly |
| C05-03 duplicate retries publish once | The same changeset identity resubmitted after a crash publishes exactly once; every later submission replays the recorded answer whatever the current head is; a different payload under that identity is refused |
| C05-04 interleaved publications with retries | A and B both fail and retry: per base exactly one publication is admitted (the coordinator's single queue and the store's expected-base check); every loser's data stays in its draft; the accepted order is total and matches the queue order; no accepted head is overwritten |
| C05-05 crash windows in the retry path | Kill during packaging, between claim binding and commit, and between commit and workspace-base advance: restart yields, per C01/U06/U07, a changeset that is fully bound or unbound, published or not, and a workspace base that moved or did not — never a half state; retries after recovery replay instead of re-publishing |
| C05-06 forced collisions | Two workspaces forced to reuse the same element or operation IDs are refused at apply with typed diagnostics; nothing is merged, renamed or dropped; layer blobs stay addressed by SHA-256 and cannot collide |
| C05-07 bounded failure surface | Oversized changeset (beyond the 1 MiB cap), corrupt blob, wrong project/schema, unknown version: typed refusals with nothing written, extending C03-13 to the C05 harness |

## C06 compensation of a published operation

Plan row C06: a compensating command with current preconditions. Acceptance: a later edit by
someone else is never rolled back. Compensation is a new forward operation: it never rewrites or
removes accepted history, and its preconditions are checked against the current head, not the
original base.

| Case | Required observable assertions |
|---|---|
| C06-01 simple compensation | A compensates its own published edit with no intervening edits: a new forward operation reverses exactly that effect; element IDs, paths and the history of both the original and the compensation are preserved; reopen shows the compensated state |
| C06-02 later edit by another actor | A publishes a move of X, then B publishes a move of X, then A requests compensation of its own move: the current-placement precondition fails and the request is refused with a typed diagnostic, or an explicitly re-planned compensation is applied as a new operation; either way B's edit survives verbatim and no other actor's operation is rolled back |
| C06-03 lifecycle preconditions | A published delete of X; accepted history contains another actor's operation referencing X: compensation names the conflicting accepted operations and refuses; nothing is rolled back |
| C06-04 compensation idempotency | The compensation request carries its own operation identity: replay returns the recorded outcome exactly once and never compensates twice; a different payload under that identity is refused (`OP_ID_REUSE`) |
| C06-05 crash windows | Kill between precondition planning and commit, and between commit and publication: recovery per U06/U07 leaves the compensation fully applied or not applied; replay after recovery yields one outcome; the original operation is neither lost nor applied twice |
| C06-06 through the coordinator *coordinator* | Only the original actor's token (or direct store access) may request a compensation; another client's token receives `PERMISSION_DENIED` with nothing written; interleaved with another publication, the expected-head and single-queue rules of C02/C03 hold |
| C06-07 stale or over-budget targets | Compensating an operation whose effect later accepted operations already changed or compensated refuses with a typed diagnostic naming the current owner of the state; a target beyond the 4096-entry caps refuses with `LIMIT_EXCEEDED` |

## What this document does not complete

Each slice still needs its own focused implementation PR per fork issue #12 (one C-package per
PR), with these cases as executable tests through the real CLI and coordinator, all existing
C01/C02 evidence and the platform CI staying green, and bounded input/error handling preserved.
The plan statuses in `docs/plan/tasks.md` change only in docs-only PRs after owner acceptance.
A planner unit test or this document alone completes nothing.
