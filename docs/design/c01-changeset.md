# C01 changeset and idempotency

Date: 2026-10-18. Status: **experimental vertical slice; C01 acceptance is limited as listed below.**
Branch `feat/c01-changeset`, based on `origin/master` `3f9f0f5`.

Plan row: C01 / M, depends on P06 and P08. Result: base, op_id, actor, dependencies, USD delta
and assets; bounded validation. Acceptance: a repeated op_id with the same payload is safe; with a
different payload it is rejected; the author string grants no rights.

## What is implemented

```text
beamcraft create-changeset  --store PUBLISHED --workspace-dir DIR [--changeset-id UUID] [--revision-id UUID] [--actor-id UUID]
beamcraft inspect-changeset --workspace-dir DIR --changeset SHA256
beamcraft apply-changeset   --store PUBLISHED --workspace-dir DIR --changeset SHA256 --actor-id UUID
```

| Area | What exists |
|---|---|
| `beamcraft_model::changeset` | `Changeset { changeset_id, project, workspace, actor, base_revision, base_manifest, revision, operations, dependencies, layers, root_layer, assets }`; canonical binary `encode`/`decode` (versioned, length-prefixed, sorted, bounded at 1 MiB, round trip enforced); `digest`; `validate`; `admit(trusted_actor, recorded) -> Apply | Replay`. `ActorId` joins the typed ids. Pure and WASM-compatible. |
| `beamcraft_host::changeset` | `RevisionStore::claim_changeset` / `changeset_claim`: an append-only, checksummed `changesets/claims.log`; the first valid line of an id is its binding. |
| `beamcraft_cli::changeset` | `create_changeset` (from a workspace: base, unsent operations, complete layer set with base digests, changed layers as assets, captured command dependencies or conservative legacy fallback), `read_changeset`, `apply_changeset` (decode, actor, replay, claim, base, source/dependency checks, blob copy and verification, commit with the strict verifier, workspace base advanced). |

## Decisions

- **What a changeset is.** A message about a USD state, never a second model: the layers stay USD
  blobs addressed by full SHA-256, and the changeset names the complete resulting layer set with
  each layer's base digest. A receiver can rebuild the revision exactly and can see which layers
  changed (the delta) without a diff format of its own.
- **Operations.** Every unsent draft revision contributes its operation id and its manifest digest
  (the content identity of its accepted result), in draft order. The order is part of the payload.
- **Accepted base versus local prefix.** The base revision id and manifest digest resolve one
  verified revision of the published store, using the current head or a bounded historical
  lookup. Its layers supply the delta base and the conservative fallback model. The local
  `synced_sequence` only identifies the already-published operation prefix; it cannot substitute
  for the accepted pin or mark later unsent work as published. Wrong, missing, compacted or corrupt
  pins and mismatched draft project/workspace/schema refuse packaging without moving either head.
- **Dependencies.** Modelling revisions pin their admitted command's exact footprint. A complete,
  verified interval contributes its union, including reads of unchanged copy sources, linked parts
  and definitions. The receiver checks every ordered source operation/result and its identity,
  recomputes dependencies and requires the exact final source layer set. It refuses omitted,
  reordered, narrowed or widened claims before publication. The destination pins the aggregate
  under its own operation id and the changeset digest; workspace publication uses the source head
  manifest digest instead. This is integrity evidence, not authentication of arbitrary local bytes.
  Both the interval and aggregate work are bounded at 4096, counting entries before union.
  Missing evidence or an unverified source revision makes the whole interval conservative;
  later malformed evidence is still an integrity failure. Legacy writes come from the difference
  between the base model and the draft head, in the F04 section 8
  groups: a changed field writes its group (an unknown field: every group), a placement change
  also writes `derived`, a changed descendant writes `derived`, created and removed elements write
  every group, new definitions are written; provenance changes alone are not changes. Reads cannot
  come from a difference (a copy reads its unchanged source, a move reads the shape), so the
  changeset reads every group of every element and every definition of the base and the head, a
  superset of any operation's reads; over 4096 entries it is `LIMIT_EXCEEDED`. Such an interval
  publishes without precise evidence. Wire v1 bytes and tags remain unchanged; `mark` (tag 7)
  requires canonical v2, and v2 without that group is rejected. Legacy v1 fallback payloads retain
  their original seven groups. This does not reinterpret a command's groups: A09 approval still
  declares label writes, and that exact footprint is retained.
- **Idempotency (project-wide).** A changeset id names one payload forever. The receiver binds the
  id to the payload digest durably *before* it publishes anything, so a crash after the binding
  resumes on retry, a retry of the same payload is a replay (the ledger proves the commit), and
  any other payload under the id is `OP_ID_REUSE` with nothing written — also when the first
  receipt was refused (for example `BASE_REVISION_MISMATCH`). This is stricter than F04's
  draft rule (an aborted op_id is free again) on purpose: a refused changeset is rebuilt under a
  new id, and an id that once named a payload can never be confused with another. The published
  revision's operation id is the changeset id, so the U06 commit binds it to the exact content
  as a second line of defence. A replay is reported only when the revision committed under the id
  is exactly this changeset's publication (revision id, base, root layer, every layer's name,
  role, digest and size); an id already committed for another operation is `OP_ID_REUSE` before
  anything is claimed. After the claim, a moved head is re-checked against the ledger, so two
  identical concurrent receipts both return the published result.
- **Workspace update after publication.** Metadata is validated before publishing; if the update
  of `workspace.json` still fails afterwards, the result is a successful publication with
  `workspace_synced: false` (never a rejection of published work), and a retry repairs it.
- **Authority.** `admit` checks the claimed actor against the trusted caller *before* the replay
  decision (F04 section 8 order), so a mismatched caller learns nothing about an id. The CLI's
  `--actor-id` stands for the trusted context until C02 adds tokens.
- **Concurrency of claims.** One synced append per claim and no lock file, as in the U07 ledger;
  racing receivers re-read and the earliest line decides (tested with eight threads). Records are
  decoded line by line, so a damaged line hides nothing after it. Size admission is not atomic
  across writers, so readers accept a 1 MiB headroom beyond the 16 MiB admission cap.
- **No merge.** The base must be the published head (revision id and manifest digest), otherwise
  `BASE_REVISION_MISMATCH`; rebase is C03, conflicts C04.

## Evidence

`apps/beamcraft-cli/tests/changeset.rs` (end-to-end tests through the binary): a changeset of two
renames carries the base, both operations, the label/lifecycle footprint, one changed layer and one
asset; the digest is deterministic; inspect shows the same; apply publishes revision and op ids as
declared and moves the workspace base; a retry replays with nothing written. The caller must be the
claimed actor (also for a forged author and after binding); another payload under the id is
`OP_ID_REUSE`. A moved head is `BASE_REVISION_MISMATCH` on every retry, the id stays bound, and the
draft stays unsent. Garbage, a non-canonical trailing byte, a future version and an unknown digest
are refused with nothing published. `crates/host/tests/changeset_claims.rs` covers reopen and an
eight-thread race. `beamcraft_model::changeset` unit tests cover the round trip, every prefix,
single-byte flips, caps before allocation, ordering, duplicates and the admission matrix.
`apps/beamcraft-cli/tests/publication_evidence.rs` covers both publication paths, precise rename
and copy dependencies, forged dependencies/source intervals/output, explicit legacy fallback,
malformed evidence after a missing record, numbering and raw aggregate work limits. Coordinator
tests verify that the accepted publication retains the same exact footprint and request digest.
`apps/beamcraft-cli/tests/changeset_base.rs` constructs an advanced accepted pin with an unchanged
local prefix, then checks exact delta bases, retained unsent operations and successful publication
of both clients' fields. This boundary fixture does not implement runtime rebase. It also covers
stale historical bases, wrong or corrupt pins, another project's pin and compaction removing a
base while its old local copy remains readable.

## Not done

- No rebase (C03) or semantic conflict resolution (C04); a moved head is refused. C02's opt-in,
  authenticated coordinator uses the same changeset receiver.
- Legacy source intervals still have conservative dependencies; absence never proves independence.
- Changesets travel inside the workspace store (the receiver reads the workspace's blobs); there is
  no transport or bundle format yet.
- Claims are never garbage-collected; the claims file is capped at 16 MiB.
