# C03: rebase of independent workspace edits

Status: design and executable-acceptance specification; runtime implementation is not present.
Scope: C03 from fork issue #12 / upstream #126. C04 conflict-resolution UI, C05 offline stress
and C06 compensation retain their own packages and PRs. This document does not complete any
of those packages or change the deferred implementation-plan statuses.

Baseline inspected: fork `master` `dae17846453b0e18d58b9fad55f3acbd6c22100d`.

## Existing behavior and constraints

`workspace-publish` and `changeset-apply` require the published head to equal the recorded base.
`coordinator-publish` also requires its expected head to equal that changeset's base. The tests
in `workspace.rs` and `coordinator.rs` already prove that one concurrent publisher succeeds and
the other keeps its draft with `BASE_REVISION_MISMATCH`. This is the existing safety behavior,
not a bug to remove by ignoring the expected-base check.

The C01 payload contains complete resulting USD layers and dependencies. Captured command
evidence supplies exact dependencies for complete verified intervals; legacy intervals retain
`changeset.rs::footprint`, which reads every group of every element and definition in the
base/head. A USD difference cannot prove a read did not occur: copying an unchanged source or
moving an element may depend on unchanged shape/definitions. Rebase must not silently turn
legacy conservative dependencies into a narrower read set.

`workspace.json` stores a published base identity and a local `synced_sequence`. C01 resolves
the exact published revision/manifest pin as its layer/footprint base, independently of the local
prefix. An unavailable or corrupt pin refuses packaging. Using the old local prefix as the base,
or setting that prefix to the merged draft to classify unsent work as published, is incorrect.
Runtime rebase still needs the durable metadata transition and recovery specified below.

## Public operation

An explicit `rebase-workspace --store PUBLISHED --workspace-dir DRAFT --expected-head REVISION`
operation calls a pure engine planner, then the application/storage adapter. It never publishes
automatically, starts a service or widens a bind address. The existing token-derived actor is
required when reached through the coordinator. A claimed author field grants no authority.

The planner receives verified pins for the old published base, current published head and draft
head, plus the ordered verified operation footprints. It returns either a bounded rebase plan
or a typed refusal with the conflicting target/group pairs. No planner result changes storage.
The application applies a successful plan in isolation, re-evaluates the touched bodies,
quantities and numbering freshness, validates the complete USD candidate, and commits a new
personal draft through U06/U07. Both published head and original draft remain immutable inputs.

The caller must supply/reuse a rebase request identity. Repeating the same request returns its
recorded result; another payload under that identity is `OP_ID_REUSE`. Rebasing changes a
changeset's base and therefore its canonical payload: a resulting changeset needs a new identity.
An old claimed changeset ID remains bound even if its original receipt was a base mismatch.

## Independence and preserved content

Compare each unsent operation with all accepted operations between the old and new bases, in
their verified revision order. The unit is F04 section 8's target plus semantic group, not a
property name or the layer file containing it. Writes intersecting the other operation's reads
or writes refuse automatic rebase. Lifecycle, linked elements and pinned definitions participate
in the same comparison. Conflicts keep both original versions; C03 does not choose a winner.

Examples required of the planner:

| Draft edit | Accepted edit | Automatic result |
|---|---|---|
| Rename part A | Rename unrelated part B | Keep both names |
| Move A | Rename A, with disjoint verified semantic footprints | Keep both fields and current derived state |
| Change A thickness | Change A contour | Conflict on atomic shape inputs |
| Copy A | Delete or change a source group the copy read | Conflict on the dependency |
| Add a weld linking A | Delete A | Conflict on lifecycle/relationship dependencies |
| Rename A | Rename A, including coincident final text | Conflict; identical text does not erase distinct operation intent |

Record and verify precise engine footprints with each newly accepted draft operation before
using them for rebase. An operation lacking this evidence keeps its conservative footprint and
can be refused as not provably independent; it must never be reclassified from a visible diff.
Unknown fields/groups and unhandled content refuse automatic rebase. Retain the current layer
engine's foreign-content and footprint guards: foreign source bytes are not merged or rewritten.

For each accepted independent edit, preserve element IDs, paths, pinned definitions, operation
IDs/order and untouched authored opinions. Recompute affected derived data through the normal
engine API, including freshness of numbering. Do not copy one entire authored or derived layer
over another: independent edits may share both files.

## Durable base and recovery

`WorkspaceState` holds a separate exclusive metadata lease and a shared maintenance lease. Its
compare-and-swap requires the complete expected metadata snapshot before staging a replacement;
workspace/user identity and the published local prefix cannot move backwards. Existing metadata
reads and writes use this guard. A holder can still commit a draft through the ordinary revision
CAS, so concurrent model edits remain possible and must participate in the rebase checks below.
The OS releases the lease after a killed process; it does not itself recover a rebase or make a
draft-head change and metadata finalization atomic.

`workspace::rebase_receipt` supplies the bounded canonical evidence format: original metadata,
project/schema, old accepted-base sequence, pinned draft and new accepted head, ordered unsent
operation/result identities and retained dependencies. Its request fingerprint covers every
input coordinate and binds the embedded command-footprint record to the rebase operation. The
candidate revision uses that operation identity. Operation count and footprint entries are each
capped at 4096; the whole record fits the host's one-MiB evidence cap. Lengths, decimal values,
field order and re-encoding are strict. A codec round trip proves no ancestry, authority or
correct model: the adapter must recompute dependencies and verify the actual committed candidate
before using its receipt. No runtime producer or recovery path is implemented by the codec.

Metadata keeps the accepted base pin, already-published operation prefix and latest local rebase
sequence separate. Zero rebase sequence preserves exact v1 bytes; nonzero uses strict canonical
v2 with one additional `rebase_sequence` field. Unsupported, redundant or noncanonical v2 is
refused, and the metadata CAS cannot move the boundary backwards. C01 packaging still uses the
accepted pin for layer bases. A received changeset cannot end before the rebase boundary, even
when its earlier source interval and output are otherwise valid. Status, packaging and publication
refuse a boundary beyond the draft head. Snapshot undo clears its older stacks at the boundary;
later edits remain undo/redo units, while the combined basis cannot be restored to an old snapshot.
These guards do not produce a rebase; compensation of earlier accepted work remains C06.

Use an idempotent rebase intent containing the three original verified pins, source operation
identities, request fingerprint and candidate identity. Stage and verify immutable candidate
blobs before the intent can affect the head. Commit the merged draft once against the exact
old draft head. Finalize metadata from the committed result, rather than materializing the new
published base as an intermediate visible draft that would hide unsent work.

Every workspace entry point that interprets base/unsent state must resolve a pending intent
before returning status or starting another rebase/publication. Recovery checks the store's
actual committed outcome and exact payload; a torn intent or an unknown result is never reported
as successful rebase. A committed candidate with unfinished metadata can be finalized on retry.
An unrelated winning commit is preserved and causes a typed refusal/replan. A later draft edit
must remain unsent when recovery finalizes its ancestor; use bounded verified history, not an
unbounded scan or a guessed sequence. Original revision blobs remain available throughout.

The published store has its own compare-and-swap. If it moves after planning, refuse/retry using
the newly observed pin; never overwrite that head or report the stale plan as current. A rebase
request authorizes no subsequent publication to a later, different head.

The host's `PublicationLease` supports this boundary: ordinary committers hold its shared gate
and retain concurrent root CAS behavior; an exclusive holder pins one store's head while it
commits a different store. Acquire it before reading the accepted pin and retain it through the
draft CAS. The guard also holds a shared maintenance lease, so a serving coordinator is compatible
and compaction is excluded. A conflicting writer/pinner receives `BACKEND_UNAVAILABLE` before a
commit intent; reads and edits of another draft remain available. This primitive does not itself
plan, merge or recover a rebase, and the runtime still needs to use it with the metadata guard.

## Executable acceptance required before completion

These are required tests to implement, not claims that they already pass:

| Case | Required observable assertions |
|---|---|
| C03-01 two independent clients | Fork A/B from one base; publish A; rebase B; reopen B with both edits; published A stays unchanged until B explicitly publishes; final reopen retains both edits |
| C03-02 shared layer, separate groups | Rename A / move A with verified disjoint footprints; check authored fields, geometry, quantities and freshness, not only an exit code |
| C03-03 atomic input conflict | Thickness/contour conflict returns stable target/group diagnostics and changes neither head nor either draft |
| C03-04 indirect dependency | Copy/change-source and weld/delete cases refuse even when changed property names differ |
| C03-05 intervening revisions | Traverse multiple verified accepted changes, including an intermediate conflicting edit later reverted; final-value equality does not hide it |
| C03-06 absent/forged footprint | Missing, truncated, narrowed or mismatched footprint evidence cannot make a dependent edit pass |
| C03-07 repeated request | Same identity/payload replays exactly once across processes; changed payload is reuse; old changeset binding remains unchanged |
| C03-08 concurrent head changes | Move each store's head between plan and commit; exactly the valid CAS winner is retained; no accepted edit is overwritten |
| C03-09 crash windows | Kill before/after intent, blob staging, draft switch and metadata finalization; restart/retry yields the old complete draft or the complete merged draft, with every original unsent edit retained |
| C03-10 later draft edit | Commit another local edit after the rebase candidate, interrupt finalization and recover; retain it and count it as unsent |
| C03-11 resumed publication | Package a changeset after rebase; its layer base digests match the new accepted base; apply and replay through the real coordinator under token authentication |
| C03-12 foreign content | Attach a synthetic reference before branching; source SHA-256 and foreign opinions remain unchanged through rebase and publication |
| C03-13 budgets/hostile data | At-cap and cap-plus-one inputs for operations, dependencies, bytes and history; wrong project/actor/schema, corrupt base and unknown metadata version refuse without partial state |
| C03-14 independent reader | Materialize the final accepted revision; the independent reader agrees with the explicit expected parts/placements/quantities; ordinary OpenUSD is additional interoperability evidence |

Keep existing C01/C02 replay, token, loopback, concurrent-publication, footprint and storage
tests passing. Required workspace checks and platform CI apply. A passing planner unit test or
this design document alone does not complete C03 or parent issue #12.
