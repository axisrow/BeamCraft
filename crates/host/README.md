# BeamCraft native host

U05 implements `beamcraft_host::storage` using the standard library and the portable
`beamcraft_model::digest` module. No third-party dependency is used.

`settings::Settings` also owns the per-user appearance preference (`System`, `Light`, `Dark`).
It defaults to the system and is encoded as `theme` in the bounded settings JSON, never in a
model. Version-1 files without this optional field retain their previous behavior; malformed or
unknown values are refused.

Public API:

- `BlobStore::new(root)` / `with_limit(root, cap)` bind one existing absolute directory.
- `publish(&[u8])` returns `(ContentDigest, Published::{Created, AlreadyPresent})`.
- `read(&ContentDigest)` returns completely verified bytes.
- `contains(&ContentDigest)` verifies presence; corruption is an error.
- `verify(&ContentDigest)` verifies and discards the returned capped buffer.
- `StorageError::code()` maps safe, typed failures to F04 `ErrorCode`.
- `MAX_BLOB_BYTES` is 33,554,432 (32 MiB; the layer engine sizes one layer by it).

The root argument is host configuration, not a resource identifier. No resource API accepts
paths or URIs. Files use `blobs/<two lowercase hex digits>/<64 lowercase hex digits>`.

Publication flushes and syncs a temporary file inside the store before a non-replacing hard
link. There is no replacing rename fallback. Filesystems lacking hard links return an error.
Existing files are never rewritten; all existing bytes are compared, even after digest
verification. Cryptographic addressing does not prove collisions impossible. Temporary cleanup
is attempted on errors; a crash can leave a temporary file. Cleanup or verification errors after
installation can report failure with the blob already installed; retry is safe.

The authorized root and its namespace must be protected against hostile concurrent filesystem
mutation. Static links/reparse points, non-regular blobs and containment violations are rejected,
but portable safe std lacks directory-relative no-follow traversal to close directory replacement
races. Windows blob opens additionally use `FILE_FLAG_OPEN_REPARSE_POINT`. A writer allowed to
change stored bytes can still change them after a verified read; readers receive their verified
snapshot. Neither this API nor SHA-256 authenticates a hostile writer.

No directory crash durability or garbage collection is implemented in `storage`;
revision publication is the separate `revision` module below and recovery is the `ledger` module (U07, below); the CLI
byte commands are U04.

SHA-256 is original code implementing FIPS PUB 180-4 (August 2015), sections 4.1.2, 4.2.2,
5 and 6.2: https://doi.org/10.6028/NIST.FIPS.180-4. NIST examples and .NET-derived boundary
vectors test it independently. The older fixture-tool hasher remains separate and untouched.

## U06 revision commit

`beamcraft_host::revision` commits and reads one complete revision; see the
[design record](../../docs/design/u06-commit-revision.md). A revision is a canonical
`Manifest` blob (identities, base, schema digest, optional verified U03 index record, sorted
layer entries with full digests and sizes) stored through `BlobStore`. The root is an
append-only head log `revisions/<20-digit sequence>`: each record is a complete synced file
installed by a non-replacing hard link, so the root switches in one step and the link is also
the compare-and-swap for the expected base.

- `RevisionStore::commit(&draft, &expected)` / `commit_with(.., verifier, hook)`.
- `RevisionStore::open()` verifies record, manifest, base link and every layer blob and
  refuses missing, corrupt, resized or torn state with `RevisionError` (`code()` maps to F04).
- `RevisionStore::info_at(sequence)` / `open_at(sequence)` read one earlier (or the current)
  revision by its 1-based log sequence with the same checks; zero or past the head is
  `NoRevision`. Used by the workspace undo (P07) to restore an earlier state as a new revision.
- `ContentVerifier` is the injected content gate (layer engine, U03 evidence). It receives a
  `VerifyRequest` (layers plus the draft's root layer, schema, project, workspace, generation
  and base pin) and its `IndexEvidence` must name that root and schema. `CommitHook`
  marks `Checked`, `IntentRecorded`, `BlobsVerified`, `ManifestStored`, `RecordStaged`,
  `Switched`, `Recorded`: abort before `Switched` leaves the root unchanged, a failure at
  `Switched` is `OutcomeUnknown`; a hook that unwinds simulates a crash (U07 fault injection).
- `beamcraft commit-revision` / `open-revision` are the headless commands.

`RevisionStore::commit_with_evidence` additionally pins one opaque operation-evidence blob by
full SHA-256 and byte size (at most 1 MiB). A pin uses canonical manifest version 2; commits
without one retain exactly version 1 bytes, and new readers support both. Evidence participates
in operation/revision replay identity, is verified on commit/open/historical read, remains
protected by unresolved intents and recovery reports, and is retained by compaction with its
revision. `Revision::operation_evidence()` re-verifies the bytes and returns explicit `None`
for old revisions. The host does not interpret the evidence or treat its presence as semantic
proof: command codecs, dependency verification and rebase remain application/engine work. It
is not another USD layer and is never given to the USD content verifier. Older binaries refuse
version 2, as they should; no existing stored version 1 revision is rewritten.

Not promised: directory-entry crash durability, garbage collection, protection from a hostile
writer to the store directory. Not implemented: personal workspaces, collaboration, a CLI
verifier.

## U07 journal and recovery

`beamcraft_host::ledger` adds a durable append-only outcome ledger (`ledger/outcomes.log`,
checksummed text lines) and recovery; see the [design record](../../docs/design/u07-journal-recovery.md).
`RevisionStore::commit_with` appends a synced `Intent` before the root switch and a `Committed`
record after it.

- `RevisionStore::query_outcome(&op)` is the host-ledger QueryOutcome: `Outcome::Committed`,
  `Aborted`, `NotFound` or `Unknown` (`OUTCOME_UNKNOWN`). Read-only; never fabricates acceptance.
- `RevisionStore::recover(RecoveryPolicy)` closes unresolved intents from the root (the root is the
  authority), removes abandoned `.tmp-*`/`.publish-*` files and reports unreferenced blobs
  without deleting any. Idempotent; no lock file. `open_recovered` recovers and then opens.
- `beamcraft recover-revision --min-age-secs N` and `inspect-revision-outcome --op-id UUID` are the
  headless commands.

Not promised: directory-entry crash durability (files are synced, directories are not),
protection from a hostile writer (the per-record SHA-256 is not authentication). Not
implemented: ledger rotation (a full ledger refuses commits with `LIMIT_EXCEEDED`), garbage
collection, personal workspaces, collaboration.

## Control

`beamcraft_host::control` holds what a CLI host operation shares: `HostError` (an F04 code, a
fixed safe message and the stable CLI exit code), `Cancellation` and `Control` (one deadline
across the operation, 30 s by default, 1..120000 ms when given). The host starts no process.

## P01 identity source

`beamcraft_host::ids::IdSource` feeds the pure `beamcraft_model::id::UuidV7Generator` with the system clock and per-process entropy from `std::hash::RandomState` (OS-seeded by `std`, no new dependency). Identities from one source are strictly increasing and complete, also inside one millisecond; across processes uniqueness rests on 62 random bits and on the existence check at creation. The entropy is not documented as cryptographic and no identity is a secret or a capability. `IdSource::with_sources` injects both inputs.

## U14 history compaction

`beamcraft_host::compaction` measures a store (`stats`) and shrinks it (`plan` is read-only, `compact` is the real run): it keeps the newest N revisions under their original sequence numbers and ids, deletes the blobs no retained revision references, and rotates the outcome ledger. The head log gains a floor marker (`revisions/floor`) and `RevisionStore::first_sequence`; a removed revision reads as `RevisionError::Compacted`. `beamcraft_host::lease::StoreLease` is the advisory store lock: commits, recovery and a serving coordinator hold it shared, compaction exclusively. A real run needs explicit confirmation and a safeguard (a backup or an explicit decision to go without) and is refused for an unresolved intent, a held store and a declared workspace based on a revision it would remove. Every step is a `CompactionPoint` for fault injection. See the [design note](../../docs/design/u14-history-compaction.md).
