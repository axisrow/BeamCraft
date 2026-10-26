# BeamCraft layer engine

Native, experimental U01/U02/U03 layer adapter for exactly `openusd =0.8.0`, without optional
features. This library performs no file or network I/O, starts no process, and exposes
no upstream handles. This is a desktop-native library, not a domain/WASM crate.

`Reader::open` accepts an explicit manifest of local bytes, exact digests, owned layer
names, full project/workspace identities, revision/generation, backend identity, budgets
and a cancellation/deadline control. Manifest integrity precedes parsing. Only canonical
relative asset identifiers are accepted; they never name files on the host.

The static profile supports USDA sublayers and external references with identity layer
offsets and explicit reference targets, plus a restricted USDC scalar/vector profile.
Unsupported composition and binary value encodings fail explicitly. Snapshots expose
bounded typed static attributes and contributing prim sites. They are read results,
not engineering validation or authorization. `Reader` remains read-only; the separate
`authored::Author` API accepts closed engine-produced change items. Static relationships,
including unknown names and list-edit composition, are returned explicitly.

An open is built privately and installed only after complete validation and a final
cancellation check. Failed opens leave the previous context intact. Reopen/close invalidate
all earlier contexts/cursors, including across reader instances. Pages read a frozen result
over one retained stage; they never reopen or recompose it.

Cancellation is cooperative before/after upstream synchronous calls and during wrapper
loops. Upstream allocation, panic-abort, stack overflow, and hard deadlines cannot be
contained in-process. The declared/decompressed preflight and output caps are not an
aggregate heap/work proof. The engine runs in the calling process (owner decision of
2026-10-20, U17): a refusal is an ordinary `Diagnostic`, the project's own layers are verified
by SHA-256 before they are parsed, and foreign files are read under these budgets. See [the U01 placement/containment review](../../docs/design/u01-placement-containment.md).

Tests use contributor-original synthetic USDA and binary output of those fixtures.
Same-crate binary generation is not independent-reader evidence; U04 and N03 remain open.

## Codeless authored and derived layers

`authored::SCHEMA_DESCRIPTOR` fixes the field/type mappings, units, paths and closed item
profile. `schema_identity()` returns its full SHA-256 and model version; the backend
archive/features pin remains independently checked. No schema generation, `openusd-build`,
Python, C++ plugin or new dependency is used. Elements are standard `Xform` prims with
explicit custom attributes and profile/material relationships. Transferred engine results
are standard `Mesh` children with standard points, topology, orientation and subdivision
properties, plus the U09 standard metadata: model `kind` (`/World` assembly, `/World/Steel`
group, element component, `Geometry` subcomponent), `displayName` equal to `bim:name`, an
authored `extent` (min and max of the float32 points) and one flat `normals` value per face
(`uniform` interpolation, Newell method). A rename rewrites `displayName` only; the path, the
identity and the mesh do not change. Ordinary USD readers need no BeamCraft plugin. This compatibility intention
is tested with the same Rust backend; SDK interchange has **not run**. No solid evaluation
or exact quantity computation runs here.

`Author::open` consumes a verified U01 manifest plus distinct, existing owned USDA edit
targets participating directly in the root's sublayer stack. The root must also be owned.
`prepare` consumes a pinned context, full op/actor identities, exact schema pin, bounded
immutable resources and typed changes. Host authorization, canonical F04 replay checks,
engineering validation and serialized decisions are prerequisites. It returns owned layer
bytes/digests, exact local authored opinion items, conservative semantic footprints and a
structural validation receipt while preserving the live view and generation. There is one
candidate; neither upstream handles nor raw USD setters are exposed.

`install` requires the trusted host's proven `Accepted` decision and rotates the context
once. `discard` permits only `Undecided` or proven `Rejected`. Uncertainty or cancellation
after proven acceptance retains the candidate and reports `OUTCOME_UNKNOWN`. Host claims
are not authenticated by this library. Dropping the process or library is not evidence
of rollback. Durable outcome queries, recovery, op_id replay and publication remain host
duties. Saved bytes are not an atomic U06 publication.

Changes cover create, delete, label, contour, thickness, axis, roll, profile, material, placement
and transferred meshes. A delete removes the whole element prim from the authored layer and its
result prims from the derived layer (every group is written; both layers must own it, and a
delete cannot share a batch with another change of the element); that nothing links to it is
proved separately with `Author::check_delete`. Optional roll distinguishes value, declaration without default,
block and property removal. A material distinguishes one pinned target, an explicit empty
list and removal; omitting a change leaves it untouched. Every contributing source and
ancestor must be owned, including sources contributed through references. Unsupported
arcs retain U01's explicit refusal. Shape edits remove exclusively owned derived meshes
before replacement; mixed-layer geometry fails rather than revealing a weaker stale result.
Unknown admitted metadata, attributes, relationships, specs and sublayer order survive
editing; untouched layers retain their exact bytes. No flatten operation is used.

`authored::rebase::prepare` builds an isolated snapshot from a base, a local source and an
accepted snapshot. It transfers only closed-schema source deltas covered by verified writes,
merges child membership and retains accepted opinions and untouched asset bytes. Every input
and the output pass structural/index checks; foreign contributions, unknown source deltas,
overlapping groups and deltas outside the two edit targets refuse. References retain their
dedicated wrapper writer. The caller must first prove independence using all command reads and
intervening history, then run the kernel verifier before a durable commit. This API performs no
publication, metadata transition, recovery or request replay and does not complete C03.

Resources are contributor/engine-supplied rectangular profiles or optional-density
materials. A batch may consist of resources only (no element change): it publishes the
definitions, declares each newly written one as a write of its digest, and rewrites nothing
that already exists. Their digest commits the canonical isolated USD definition layer produced by
`Resource::bytes`, excluding the eventual embedding path/digest label. Existing known
definition values are rehashed before use. Unknown fields remain inert, outside that
closed resource identity; this is not a catalogue or a hash of arbitrary embedded specs.

U01 ceilings only tighten. Additional input accounting caps a batch at 1 MiB, 4096 items
and 4096 total semantic footprint entries, with the smaller stage caps applying. Points,
mesh arrays and labels are counted before scanning/copying. Retained candidates, exported
layers and typed opinions are bounded by these source/snapshot/value caps. These are
wrapper bounds, not a proof of aggregate upstream allocation or preemptive cancellation.

`validate_layers` is the offline `u02-structural-v1` checker. Its six fixed rules cover
U01 admission; schema markers/known types; UUID/path/required inputs; pinned definition
links; scalar caps, scaled length minima and rigid pose; and standard mesh buffer structure.
Newer element schemas/kinds/modes remain inert and cannot be edited. This checker is separate
from external `usdchecker` (**not run**, no offline executable found), the F10 module-rule
catalogue and the F06 engineering validator. It does not certify contour topology,
planarity, support, mesh solidness, exact quantities, F06 result freshness or signatures.
Those remain engine prerequisites. See [the U02 validation report](../../docs/design/u02-validation.md).
`Reader::open_validated` runs the same checker on the candidate it then installs, so a reader
that needs both (the CLI's inspection and its commit verifier) parses and composes the stage
once instead of twice; its refusals, their order, codes and messages are those of
`validate_layers` followed by `Reader::open` (issue 26).

## Read-only foreign references (R01)

`Change::AttachReference` authors an owned wrapper of kind `reference` (element path, `group` kind)
around a foreign layer that is already a **non-owned** manifest entry with exactly the pinned
digest and size. Below the wrapper are an owned `Correction` prim (the optional units and up-axis
interpretation, `Change::ReferenceUnits`) and a typeless `Source` prim that holds the reference arc
and nothing else; everything composed below `Source` is foreign. Moving (`Change::Placement`) or
renaming (`Change::Label`) a wrapper does not touch a foreign prim. `Prepared.retained` returns the
non-owned layers byte for byte; they are never among `Prepared.layers`.

The read-only rule is enforced where the layers are read, not only where they are written:
`authored::validate_layers`, `Author::open` and `Author::prepare` scan the specs of every owned
layer, and an owned spec at `Source` may hold only `specifier` and `references`, with no spec
below it (`FOREIGN_READ_ONLY`). The composed provenance of every prim below `Source` must be
foreign only, with the pinned digest, layer name and default prim. `Reader::refuse_layer_write`
answers `FOREIGN_READ_ONLY` for a layer the host did not select as owned. `inspect_foreign_layer`
examines a candidate source (format, default prim, declared units, up axis) under the same byte,
field and budget rules as every stage asset, without composing or resolving it; a URL or escaping
identifier in it is `PERMISSION_DENIED`, an arc outside the static profile
`UNSUPPORTED_PROPERTY`, another local asset `DEPENDENCY_UNAVAILABLE`. `authored::read_references`
reads the wrappers back (after the same validation). A foreign prim that carries an element
identity (`bim:id`, or a reserved element prefix) makes the stage fail with `INVALID_ID`. The schema
descriptor lists the wrapper; pre-U16 schema pins are refused. See the
[R01 design note](../../docs/design/r01-readonly-reference.md).

## UUID and relationship index (U03)

`index::ElementIndex` is a reconstructible lookup built only from the complete pinned snapshot
of one context: `Reader::element_index` / `Author::element_index` page the frozen result through
`IndexBuilder`, which also accepts caller-held pages and rejects mixed contexts, gaps, replays,
cap-plus-one pages and any item whose identity, path or provenance fails re-validation. The key
is the full 128-bit UUIDv7 (`ElementId`); same-millisecond IDs are never shortened or merged and
prefix, upper-case, unhyphenated, wrong-version/variant text is `INVALID_ID`. Element prims must
follow the category folders and three-letter prefixes of the [prim naming rule](../../docs/architecture.md#prim-hierarchy-and-naming-rule), with the full 32-digit UUIDv7 with agreeing snapshot identity and `bim:id`; duplicates are
`DUPLICATE_ID`. Labels are never identity: rename keeps ID, path and the label-free
`structure_digest`. Entries carry lifecycle (`bim:isDeactivated`, absent means false), the USD
`active` flag, composition provenance and an ownership verdict that includes foreign ancestors
and descendants. Every relationship target is resolved to an element, an immutable definition, a
plain prim or (foreign sources and partial indexes only) an `Unresolved` entry; an owned dangling
target is `DANGLING_RELATIONSHIP`, a short/malformed element or definition target `INVALID_ID`.
Outgoing and incoming queries, `delete_blockers`, `dependents`, deactivated-target reporting and
`copy_plan` (fresh ID, self-link remap, shared definitions and level, dropped assembly membership
and connections for copied parts, refusal of other unsupported or descendant-carried links) are
pure reads of the index. Links to a descendant prim count as
dependencies of the containing element.

The index is not authority. A partial index answers `DEPENDENCY_UNAVAILABLE` for every
relationship query and cannot prove absence. `Author::authorize_write`, `check_delete` and
`certify_dependent_reads` first call `verify_index`, which requires the index's own context to
be live (`STALE_WORKSPACE` after any install/reopen), complete, and equal to a fresh rebuild
(`INTEGRITY_ERROR` for a forged or tampered page set). The grant names the owned authored layer
but is not host authorization or F04/F06 validation. Not implemented: a CLI/host command that
reports the index, a persisted
index, copy/delete commands (P05/P08), typed element-to-element relationship records and their
vocabulary rules (F10/vocabulary section 5), and USDC relationship evidence (the restricted binary
profile refuses relationship specs and U02 element layers). See [the U03 report](../../docs/design/u03-index.md).

U16 makes an intentional pre-alpha compatibility break: no old-layout store or schema pin is
accepted or migrated. Definitions live under `Definitions/Materials/MAT_<sha256>` or
`Definitions/Profiles/PRF_<sha256>`; levels and reference wrappers live outside `/World/Steel`.
See the [U16 design note](../../docs/design/u16-prim-naming.md).


## F10 vocabulary alignment (#148)

Deactivation is `bool bim:isDeactivated` (absent means false); a present declaration must have
an actual boolean value. USD `active` remains separate. Connections author the required token
`steel:connection:type` (`weld` or `bolt_group`); conflicting property families are rejected.
The schema digest includes these changes. Earlier schema pins are refused, with no implicit migration.
Domain diagnostics retain their `DV-*` rule IDs through CLI error responses; transport and
geometry failures retain their own error namespaces.

The executable logical-case subset is `DVC-BASE-004-P/N` and `DVC-REL-011-P/N` in
`apps/beamcraft-cli/tests/f10_vocabulary.rs`. These cases drive the real USD reader, index and copy planner and compare exact
rule IDs and F04 codes. The remaining logical cases are not claimed as executable F10 acceptance
by this suite. Additional authoring and CLI tests cover connection types, assembly-free copies,
level retention, unchanged source relationships and undo/redo.
