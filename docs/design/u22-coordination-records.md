# U22: vocabulary 0.2.0 coordination record schema

Date: 2026-11-01. Status: **delivered slice; store commands and scope authorization remain
U19/U20**. Branch: `ao/vocabulary-schema/feat/u22-vocabulary-02`; fork issue #362 (upstream
#228, parent epic #189).

## What this changes

Defines and tests the exact USD serialization, schema pin, typed command envelopes and
bounded parsers of the F10 0.2 coordination vocabulary before anything advertises 0.2.0:

- `beamcraft_model::coordination` — the only shapes a 0.2 record set may take:
  `SectionRecord` (SectionId, code, label, permitted modules), `ZoneRecord` (ZoneId, code,
  label), `AuthoredScope` (per-resource section/zone provenance), `ProjectMetadata`
  (code/name/organization labels), `OpeningDecision` (verdict bound to an exact proposal
  digest and reviewed host revision, closed reason codes). Every record admits itself with
  F04 codes; codes and names are labels and never authority (PC-ID-001).
- `beamcraft_layer_engine::authored::coordination` — the 0.2 mapping: authoring functions
  (`author_section`, `author_zone`, `author_decision`, `author_scope`, `author_metadata`),
  the whole-snapshot parser `read`, the per-element `authored_scope`, and the 0.2 schema pin
  (`schema_descriptor_v02`, `schema_identity_v02`, `pin_accepted_v02`, `pinned_v02`).

No `Change` variant, store format, negotiation set, CLI command or scope check is added:
U18 (explicit upgrade), U19 (workspace scope) and U20 (registries and shared commands) build
on these shapes. The frozen 0.1 `SCHEMA_DESCRIPTOR`, `pin_accepted`, `Change` enum and
`snapshot` validator are untouched; a 0.1 store keeps its contract with record content
carried inert (validated by an executable case).

## Fixed serialization (F12 section 9 gate)

Root `customLayerData` carries the 0.2 mapping digest in `bimSchemaDigest` (same key as the
0.1 line; the accepted value differs) and the optional metadata keys `bimProjectCode`,
`bimProjectName`, `bimProjectOrganization`. The coordination scopes are
`/World/Coordination/{Sections,Zones,Decisions}` (`Scope`, kind `group`). Record prims are
`SEC_<uuid7-32>`, `ZN_<uuid7-32>`, `DEC_<uuid7-32>` (`Scope`, kind `group`), one segment
below their registry, with the closed attribute sets:

| Record | Attributes (exact names) |
|--------|--------------------------|
| section | `bim:schemaVersion=string`; `bim:section:id=string(uuid7,=path)`; `bim:section:code=string(1..32)`; `bim:section:label=string(1..256)`; `bim:section:modules=string[](0..16 unique tokens)` |
| zone | `bim:schemaVersion=string`; `bim:zone:id=string(uuid7,=path)`; `bim:zone:code=string(1..32)`; `bim:zone:label=string(1..256)` |
| decision | `bim:schemaVersion=string`; `bim:decision:id=string(uuid7,=path)`; `bim:decision:proposal=string(uuid7)`; `bim:decision:proposalDigest=string(sha256)`; `bim:decision:hostRevision=string(uuid7)`; `bim:decision:verdict=token(accepted\|rejected)`; `bim:decision:reason=token(position\|size\|structural\|other)`, present exactly on a rejection |
| authored scope (per element) | `bim:sectionId=string(uuid7,optional)`; `bim:zoneId=string(uuid7,optional,section required)` |

The 0.2 pin is SHA-256 over the frozen 0.1 descriptor text followed by
`SCHEMA_DESCRIPTOR_V02_ADDITIONS`, so the digest names the complete mapping a 0.2 store
uses. `read` refuses to interpret any record unless the root pins exactly that digest
(`UNSUPPORTED_VERSION`, DV-VER-001/PC-VER-001); the 0.1 reader refuses a 0.2-pinned layer
with the same code through its existing digest check.

## Numeric budgets (F12 section 9)

sections ≤ 1024, zones ≤ 1024, decisions ≤ 4096 per project; codes 1..32 bytes
(`[A-Za-z0-9][A-Za-z0-9_-]*`); labels 1..256 bytes without control characters; at most 16
unique namespace-token modules per section; record prim names are exactly one segment below
their registry scope; registry identities and codes are unique. Refusals: unknown prim or
unknown field in a coordination scope → `INVALID_ARGUMENT`; over-count or over-size →
`LIMIT_EXCEEDED`; unparseable identity, path/identity disagreement → `INVALID_ID`;
duplicate identity or code → `DUPLICATE_ID`; wrong vocabulary line → `UNSUPPORTED_VERSION`.

## Acceptance evidence

`crates/layer-engine/tests/coordination.rs` (original cases; no existing test relabelled):

- Round-trip: metadata, two sections, one zone, an accepted and a rejected decision and an
  authored scope survive a real save (usda bytes with verified digest) and reopen (backend
  `Reader`) unchanged, twice from the same bytes; scope removal re-reads as unknown
  provenance with neither attribute stored.
- Pin: the 0.2 identity is stable, differs from the 0.1 identity, and only the 0.2 digest is
  admitted; an unpinned layer is never interpreted.
- Negative: unknown field, non-record prim, nested prim, unknown coordination scope
  (`INVALID_ARGUMENT`); over-count registry, oversized code and label (`LIMIT_EXCEEDED`);
  wrong-typed attribute, unknown verdict or reason, rejection without reason, reason on an
  acceptance, metadata without its required pair, zone without section
  (`INVALID_ARGUMENT`); non-UUIDv7 identity, path/identity disagreement (`INVALID_ID`);
  duplicate code (`DUPLICATE_ID`); unpinned and 0.1-pinned layers (`UNSUPPORTED_VERSION`).
- 0.1 invariance: an ordinary 0.1 plate layer validates unchanged, also when it carries
  inert 0.2 record content; pinning the 0.2 mapping on the 0.1 line is refused outright.

## Remaining limits

`author_*` functions write records into an authored layer but no command dispatches them
yet; registry uniqueness across registries (one UUID naming both a section and a zone) is
left to U20's registries, as PC-ID-001's typed identities do not conflict across types;
decision records are immutable evidence only — no lifecycle, transfer or proposer-side
state machine is implemented here.
