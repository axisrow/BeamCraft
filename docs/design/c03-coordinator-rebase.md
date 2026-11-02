# C03 implementation note: rebase through the authenticated coordinator

Date: 2026-10-30. Status: implemented (wire operation `rebase`, client command
`coordinator-rebase`, acceptance C03-R01..C03-R08 in
`apps/beamcraft-cli/tests/coordinator_rebase.rs`). This note records how the remaining C03
slice of fork issue axisrow/BeamCraft#12 and fork issue axisrow/BeamCraft#290 reached the wire.
It changes no acceptance
specification and
no implementation-plan status; the local semantics stay owned by `docs/design/c03-rebase.md`
and the R rows by `docs/design/c03-c06-acceptance.md`.

## What runs where

All engineering and storage decisions stay in `workspace::rebase::rebase_workspace`; the wire
adds transport, authentication and serialization only, like C02 `publish` before it:

- The `rebase` wire operation (`crates/commands/src/coordinator.rs`) accepts exactly the
  fields `workspace`, `op_id`, `expected_head`, `expected_draft`; anything else is
  `UNSUPPORTED_PROPERTY` before any state is read, and an unknown operation stays
  `UNSUPPORTED_COMMAND`. The handler opens the named workspace under the workspaces root
  (the same containment check as `publish`) and calls `rebase_workspace` with the
  **authenticated actor** derived from the token; a request cannot name an actor, so the
  "trusted actor only" rule of C03-R02 is the C02 rule applied to the new operation. The
  workspace-ownership check inside `rebase_workspace` (`PERMISSION_DENIED` before any claim
  or commit) is the second half of that row.
- The answer mirrors the local `rebase-workspace` result shape — `workspace_rebased` /
  `workspace_rebase_replayed` with `revision_id`, `base_revision`, `base_manifest`,
  `retained_operations`; a conflicted plan is a rejected `DEPENDENCY_CONFLICT` carrying the
  same bounded `diagnostics` array as the local command — plus `current_head` /
  `current_head_status`, which are informational exactly as in `publish`'s answer. A rebase
  never moves the published head; the client packages and calls `publish` explicitly, and a
  stale `expected_head` there is the ordinary `BASE_REVISION_MISMATCH` with the current head
  (C03-R04's forward path: rebase again, repackage under a new changeset identity, publish).
- The single request queue of C02 serializes concurrent rebases of one workspace
  (C03-R05): the metadata CAS inside `rebase_workspace` then admits exactly one merge; a
  repeated identity replays the recorded result, a stale expected draft gets the typed
  mismatch, and never two merged draft heads exist.
- Idempotency and crash windows (C03-R06) reuse the store-recorded outcome and the
  `RebaseReceipt`: a request identity committed under any entry point replays under every
  entry point, so a lost answer or a killed server is finished exactly once by retrying the
  same identity (C03-R08 covers the local CLI plus wire pair explicitly).
- Bounded wire behaviour (C03-R07) is the C02 transport plus the field check; nothing is
  written on any refusal before the rebase claims its intent.

## Deliberate bounds

- The wire answer carries the merged draft head as `revision_id` only, not the full local
  revision object; clients that want the complete summary read their own draft through the
  ordinary local commands. The recorded result of an identity is comparable across entry
  points by that revision id.
- The conflict report travels in planner order and is byte-stable for equal inputs; the
  acceptance test asserts the wire report equals the local command's report for the same
  inputs.
- `docs/design/c03-coordinator-rebase.md` intentionally does not extend the R tables;
  `c03-c06-acceptance.md` remains their home.

## Evidence

`apps/beamcraft-cli/tests/coordinator_rebase.rs`: eight cases against a real
`serve-coordinator` process with real token files — R01 (typed merge, immovable published
head, explicit publish, conflict branch equal to the local report, byte-stable replay),
R02 (stranger token and claimed actor field refused with nothing written, changeset stays
with its actor), R03 (authentication before any state read: unknown token, short token,
group-readable token file), R04 (interleaved publications, stale-head refusal carries the
current head, accepted history holds every edit exactly once), R05 (concurrent rebases of
one workspace: same identity = one merge plus one replay; distinct identities = one merge
plus one typed mismatch), R06 (client killed before the answer, server killed in flight:
complete states only, the identity finishes exactly once, no unsent edit lost), R07
(unknown operation, unknown field, oversized frame, wrong protocol — nothing written), R08
(one identity across the local CLI and the wire).
