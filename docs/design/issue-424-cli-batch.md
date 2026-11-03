# CLI session / batch mode — phase 1 measurements and design

Date: 2026-11-03. Status: **phase 1 draft for owner review — measurements and a design
proposal only; nothing is implemented.** Fork card #424 (upstream Apibim-com#766, slice 6
of #753). Per the upstream card, the design note precedes any code; implementation starts
only after the owner approves the design.

## Measured baseline

Method: hyperfine against a release build of `origin/master` `a5089be0` (Apple Silicon
macOS). Store: `create-project` + one i-shape profile (`define-profile`) + members added
one `create-member` per run. Warmup 5 (reads) / 3 (writes); 50 runs for reads, 30 for
writes. Wall clock, medians:

| What | Command | Median | Mean | Min–Max |
|---|---|---|---|---|
| Process start, no store | `beamcraft --version` | 2.1 ms | 2.4 ms | 1.5–16 ms |
| Store verification, 3 revisions | `open-revision` | 3.7 ms | 4.0 ms | 2.2–22 ms |
| Store verification, 33 revisions | `open-revision` | 5.3 ms | 5.7 ms | 4.3–14 ms |
| Open + decode + evaluate, no commit | `inspect-elements` (33 elements) | 103 ms | 104 ms | 102–117 ms |
| One modelling command, whole run | `create-member` (30 runs) | 525 ms | 525 ms | 266–755 ms |

Reading of the numbers:

- **Process start and store verification are not the cost.** Start is ~2 ms and the full
  head verification (`RevisionStore::open`: root record, manifest, base link, every layer
  blob's digest and size, operation evidence) adds ~3–5 ms. Together ~6 ms of the 525 ms.
- **The cost is a full model re-evaluation plus a full revision commit on every command.**
  `create-member` opens the head, decodes it and re-evaluates every plate and member in the
  kernel (~103 ms at 33 elements), then authors, publishes layer blobs and commits one
  revision through the U06/U07 ledger (the remaining ~420 ms).
- **Both terms grow with the model.** Over the 30 write runs the model grew from 4 to 33
  members and the per-command wall time grew from ~266 ms to ~755 ms. A hundred-command
  build through the current one-process-per-command model therefore costs minutes, and the
  per-command price rises as the model fills.

## Design proposal (nothing implemented; owner decides)

### Session semantics

A session is one `beamcraft` process that holds one opened store and executes commands one
at a time:

```text
beamcraft session --store ROOT
```

- One command invocation per stdin line, exactly the option grammar used today; one JSON
  result object per stdout line. No new dispatch path: the session routes each line through
  the same registry and adapters, so refusals, help rules and identity options are
  unchanged.
- The process is started by the agent itself; its stdin/stdout is a pipe with no bind
  address, no listener and no network. `AGENTS.md` §3 ("any control channel is opt-in,
  loopback-only and token-authenticated") is satisfied by construction — there is no
  channel to bind or discover; whether a token is still wanted is open question Q1.
- Strictly opt-in: without `session` the binary behaves exactly as today
  (one command per process). The mode ends at EOF on stdin.
- The store contract does not change: every command still carries its five identity
  options, still refuses identically, still produces the same JSON.

### What a batch of commands means for the journal and revisions

Three shapes were considered:

- **(a) Transport only.** Every command commits its own revision exactly as today; the
  session merely amortizes process start. The measurements say this alone is not worth a
  mode: it saves the ~6 ms start+verify out of 525 ms per command.
- **(b) One revision per batch.** Commands accumulate as in-memory draft layers; a single
  U06 commit publishes one revision containing all of them. The manifest's operation
  evidence already aggregates per-command read/write footprints (capped at 4096 entries —
  a longer batch is refused, not truncated). The U07 outcome ledger records one
  intent/committed pair per batch attempt, keyed by a batch op id; per-command op ids keep
  their bindings inside the evidence, so `inspect-revision-outcome` still answers per
  command.
- **(c) Explicit batch bracketing.** One revision per command remains the default; an
  explicit `batch-open` … `batch-commit` pair marks commands that must land atomically as
  one revision.

Proposal: **(c), with (b) as the mechanics of a bracketed batch.** Revisions stay the
publication boundary; nothing between the brackets touches the disk, so an interrupted
batch leaves no trace and recovery has nothing new to recover. Unbracketed session
commands commit exactly as today.

Concurrency: today every command re-resolves the head, so a second writer (another CLI
process, the coordinator) moving the head between commands is absorbed as the new base. A
bracketed batch must instead pin its expected base: at `batch-commit` the base is checked
as U06 does today and a moved head fails the whole batch rather than silently rebasing it.
(Auto-rebase onto the moved head is open question Q4.)

### Refusal and replay behavior

- **Refusal inside a bracketed batch:** the refused command's JSON is emitted and the
  batch is abandoned whole — nothing is applied, the store keeps the pre-batch revision,
  and the session stays alive and ready for a new batch. A partial batch is never
  published, so the F04 all-or-nothing rule holds by construction. Whether instead to
  continue past refusals and report per-command outcomes is open question Q3.
- **Replay:** unchanged per-command semantics (F04 §6, U07): an identical request under
  the same op id replays; different content under a committed op id is `OP_ID_REUSE`. A
  bracketed batch additionally replays as a whole by its batch op id: the same command
  list against the same expected base returns the already-committed batch revision. The
  batch attempt follows U07 exactly — intent appended and synced before any work is
  visible, `committed` after the root switch, `aborted` for a failure before it, and
  `OUTCOME_UNKNOWN` resolved by `recover-revision` or an identical retry.
- **Timeouts and cancellation:** the existing `--timeout-ms` / `--cancel-after-ms`
  semantics apply to the batch attempt; after the root switch a timeout is
  `OUTCOME_UNKNOWN`, never a fabricated failure.

### What a session may not do

No new identity source, no bypassed refusal, no relaxed evidence, no changed budgets. No
listening socket and no daemon in this proposal; the opt-in loopback coordinator (already
shipped separately) is unaffected.

### Expected gain, honestly

With (a) the measured saving is ~6 ms per command. With (b)/(c) the saving per additional
command in a batch is the full re-evaluation plus the commit plus the start — at this
store's size roughly 0.5 s of the 0.53 s per command, growing with the model. On the
measured store a 100-command batch saves on the order of tens of seconds versus one
process per command, and the gap widens as the model grows. (Holding the decoded model
across commands to make authoring incremental is a further step and is not proposed here.)

## Open questions for the owner

1. **§3 token:** is explicit opt-in invocation of a stdin-pipe session enough, or must the
   session still require a token (and where would it come from)?
2. **Batch default:** (c) explicit bracketing only, or (b) one-revision-per-batch as the
   session default?
3. **Refusal policy:** stop-on-first-refusal abandons the batch (proposed) vs continue and
   report per-command outcomes at `batch-commit`.
4. **Head moved mid-batch:** fail the batch on base mismatch (proposed) vs auto-rebase the
   remaining commands onto the moved head.
5. **Output shape:** full per-command JSON on stdout (proposed) or a compact line protocol.

## Provenance

Measurements and this note are original work. No Tekla/Trimble material, no new
dependency, no code change.
