# BeamCraft CLI

The `beamcraft` binary is the headless entry point to every feature. USD is read and written in
this process by the pure-Rust layer engine (`beamcraft-layer-engine`); no other process is
started, and no command opens a socket or a service except the opt-in loopback coordinator
commands. A
corrupt or hostile layer is an ordinary refusal with a stable F04 code under the layer engine's
size and depth budgets; the project's own layers are verified by SHA-256 before they are parsed.
`commit-revision` and `open-revision` (below) publish already stored blobs and do not parse USD;
`create-plate` (P01, below) and the other modelling commands author USD and publish it.

Build offline with `cargo build --offline -p beamcraft-cli --bins`.
`beamcraft --help` describes the bounded arguments. Results are one JSON object on standard
output, with strings for generations and other lossless integers.

CLI help is owned by command family under `src/help/` (for example `member.rs`, `drawing.rs`
and `workspace.rs`). Edit that family's text and command inventory there. `src/help/mod.rs`
only composes the inventory and sections in their established order; register a new family
there without adding help text to `main.rs`. Parsing and execution stay in the existing
command adapters. No arguments, `--help` and `-h` print the same help.

Application appearance is also headless: `beamcraft show-settings [--config-dir DIR]` reports
`settings.theme`; `beamcraft set-settings --theme system|light|dark [--config-dir DIR]` saves it
without changing the author, environment or recent models. The default is `system`. Existing
version-1 settings without the optional theme field remain readable; unknown theme values are
refused without overwriting the settings.

Modelling commits also pin the admitted command's exact read/write footprint and typed request
fingerprint as operation evidence in the revision manifest. This includes unchanged copy sources,
linked parts and immutable definitions, which a state diff cannot recover. The public
`command_evidence::read` API verifies canonical bytes and operation binding; legacy absence stays
explicit. The host protects the evidence through replay, recovery and compaction. Workspace
publication and changeset application aggregate a complete, verified source interval, preserving
reads of unchanged objects. A receiver recomputes dependencies and checks the ordered source
operations and exact final layer set before publishing. The aggregate is bound to the destination
operation and immutable source-manifest or changeset digest. Operations and aggregate read/write
entries are capped at 4096, counting repeated entries before union.

Low-level `commit-revision`, workspace forks and history restoration do not fabricate command
evidence. An interval with any absent or unverified record uses conservative changeset dependencies
and publishes without precise evidence; malformed evidence still fails closed. C01 wire v1 stays
byte-compatible; the additional `mark` group uses canonical v2 without changing existing tags.
This preserves the command's actual groups (A09 approval currently declares label writes). Missing
evidence never proves independence. Publication still requires the current base; rebase is separate.

Exit codes: 0 success, 2 input or validation refusal or an unavailable command, 3 backend
unavailable, 4 outcome unknown, 124 deadline, 130 cancelled. `--timeout-ms` accepts 1..120000
(default 30000) and `--cancel-after-ms` accepts 0..120000 on the blob and revision commands
below.

`put-blob --store ROOT --file FILE` and `verify-blob --store ROOT --digest SHA256`
use U05 byte storage. They do not publish a project revision.

## U06 revision commands

```text
beamcraft commit-revision --store ROOT --project-id UUIDV7 --workspace-id UUIDV7
  --revision-id UUIDV7 --op-id UUIDV7 --generation U64 --base none|REVISION_UUIDV7
  --schema SHA256 --root-layer NAME --layers ROLE:NAME:SHA256,...
beamcraft open-revision --store ROOT
```

`ROLE` is `authored`, `derived` or `external`; every layer blob must already be in the store
(`put-blob`), and each size is taken from the verified stored bytes. The commit verifies the
expected base, every blob and the op id, then switches the root in one step; a retry of the
same op returns `revision_replayed`. `open-revision` verifies the root, its manifest and every
layer blob and prints the revision (`revision_verified`). Output is JSON with decimal strings
for `sequence`, `workspace_generation` and sizes. Errors use the existing exit codes
(`BASE_REVISION_MISMATCH`, `OP_ID_REUSE`, `INTEGRITY_ERROR`, ... exit 2). `--timeout-ms` and
`--cancel-after-ms` are checked while preparing and at every commit point before the root
switch (the root is then unchanged and the cancellation/deadline error is reported); they are
never checked after the switch, because a timeout would hide a committed revision. A retry of
a commit must repeat the same `--base`: another base under the same op id is `OP_ID_REUSE`.

These commands check blob integrity, not USD content: duplicate-ID, dangling-link and
U03-index checks run only through the host API's injected `ContentVerifier` (exercised in the
tests with the real layer engine). A CLI commit records `index: null`, and every command that
later reads the revision parses it under the layer engine's budgets. See the
[U06 record](../../docs/design/u06-commit-revision.md).

## U07 recovery commands

```text
beamcraft recover-revision --store ROOT --min-age-secs U64
beamcraft inspect-revision-outcome --store ROOT --op-id UUIDV7
```

Every `commit-revision` now writes a synced intent to `ledger/outcomes.log` before the root
switch and a committed record after it. `recover-revision` closes unresolved intents from the
root, removes abandoned temporary files, reports unreferenced blobs (it deletes none), verifies
the root and prints one JSON report (`recovery_complete`, or `recovery_incomplete` with exit 0
when an intent is in flight, unprovable, or the ledger contradicts the root; check `outcome`).
`--min-age-secs` is required and has no default: `0` asserts that no commit is running on the
store, a larger value leaves younger intents and temporary files alone. Running it twice changes
nothing. `inspect-revision-outcome` is read-only and prints `outcome_committed` (with `source`
`ledger` or `root`), `outcome_aborted` (with `reason`), `outcome_not_recorded` (only when the
ledger covers the whole history) or the standard `outcome_unknown` (exit 4); a ledger that the
root contradicts is `INTEGRITY_ERROR`. Not implemented: a CLI fault-injection switch (the
tests simulate the crash through the host commit hook and then run the real binary), ledger
rotation, garbage collection. See the [U07 record](../../docs/design/u07-journal-recovery.md).

## V02 view snapshot commands

`snapshot-view` and `inspect-view-changes` need no store. They call
`beamcraft-engine` and print one JSON object (same shape for errors as above, plus a
`diagnostics` entry with the view code such as `GEO_STALE_TOKEN`; exit 2 on rejection).
The scene is **synthetic** (see `inspect-elements` for committed plates), built with the real kernel
(`--plates 1..500`): every fifth element is labelled `support` and, with
`--stand-in-simplified true`, every third carries a distinct stand-in simplified mesh. The
output marks both under `stand_ins`.

```text
beamcraft snapshot-view --plates 20 [--hide plate,support] [--representation full|simplified]
  [--stand-in-simplified true|false] [--select ID,ID] [--revision SHA256] [--generation U64]
beamcraft inspect-view-changes --plates 20 --edit move-first,resize-first,add-one,remove-last
  [--since TOKEN] [--stand-in-simplified true|false] [--revision SHA256] [--generation U64]
```

`snapshot-view` reports the pin, the snapshot token (unchanged by `--hide` and
`--representation`), the projected elements with instance slot, triangle range, mesh digest,
exactness and world bounds, the display and engineering bounds, the meshes the view must load
(digest, bytes, counts) and, for `--select`, the ids and their instance slots. `inspect-view-changes`
snapshots the scene, applies the scripted edits as the next generation and prints the
`GetChanges` delta, with `new_meshes` empty for a placement-only edit. An unknown `--since`
token is `GEO_STALE_TOKEN`. No mesh bytes are exported; a consumer reads blobs by digest through
the engine API. Defaults: revision is the SHA-256 of `beamcraft-synthetic-scene`, generation 1.

## P01 first plate commands

```text
beamcraft create-plate --store ROOT --width MM --length MM --thickness MM
  [--name TEXT] [--translation X,Y,Z] [--material-digest SHA256]
  [--hole-rect "X,Y,W,H;..."] [--hole-round "X,Y,D;..."] [--hole-segments 8..256]
  [--new-id UUID] [--op-id UUID]
  [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
beamcraft inspect-elements --store ROOT
```

`--hole-rect` and `--hole-round` (A04, see the [design note](../../docs/design/a04-plate-holes.md))
author through holes together with the plate: axis-aligned rectangles from the corner `X,Y`, and
regular polygons (default 32 sides) inscribed in the circle of centre `X,Y` and diameter `D`.
Only the clockwise polygons are stored, so a "round" hole is that polygon and its volume is exact for
it; the kernel cuts them (strictly inside the outline, clear of each other, otherwise
`INVALID_ARGUMENT`) and the volume is the certified enclosure's midpoint. At most 64 holes.

`create-plate` builds the origin rectangle contour `(0,0,0),(w,0,0),(w,l,0),(0,l,0)`, runs the
engine's `CreatePlate` (F04 admission, kernel mesh and volume, declared footprint), authors the
model layer and the derived layer as standard USDA with the U02 layer engine, checks that the layer
engine's writes are inside the engine's footprint, and commits through U06/U07 with a verifier that
runs the U02 structural check, the U03 index and a kernel re-evaluation of every plate in the
layers. Output is one JSON object: element id, prim path, dimensions, volume (mm3 and m3, status,
algorithm), mass (kg, density and status: `current` only when `--material-digest` pins a material
that has a density, otherwise `unknown` with no value), the pinned material digest, mesh digest,
revision id, sequence, manifest digest,
generation, layer digests and the index record, and the footprint. Ids that are not given are
generated (UUIDv7, strictly increasing, safe inside one millisecond). A retry of the same
`--op-id` with the same `--new-id` and arguments is `plate_replayed` (nothing written); other
content (including another material) under that op id is `OP_ID_REUSE`; an owned `--new-id` is
`DUPLICATE_ID`. A lost race is
`BASE_REVISION_MISMATCH`. `--store` must be an existing absolute directory; the first call
creates project and workspace ids (or takes them from the flags), later calls reuse the head's.

`inspect-elements` opens the head revision with full verification in a fresh process, re-reads the
layers with a fresh reader, re-evaluates every plate and checks its committed mesh against the
kernel's, and prints the plates, the pinned snapshot token, the element table and the meshes a
viewer must load (`synthetic: false`).

`--contour "X,Y[,Z];..."` accepts two or three components per point (omitted Z is zero).
Plate geometry supports simple counter-clockwise contours in `z=0` and exactly rectangular
contours in other planes. A framed rectangle extrudes along its ordered normal; holes on this
new path are unsupported. Nonplanarity beyond the scaled band, clockwise winding and touching
are `INVALID_ARGUMENT`. Valid near-planar contours and other unsupported-plane polygons without
operations are saved as authored-only plates: `inspect-elements` reports `validity: valid`,
`geometry_status: unsupported`, null mesh/key/bounds and null volume/mass with unsupported status.
They remain in the plate list and inspection snapshot without a render instance. Replays create
no revision and deletion works. Editing/copying these plates and geometry-dependent numbering,
BOM, IFC and binary view export return `DEPENDENCY_UNAVAILABLE`; none silently drops the element.
Rotated element placements remain `UNSUPPORTED_GEOMETRY` at command application. The layer engine runs in
process by the U17 decision. Workspace and undo commands have their own sections below; a full
F04 envelope/JSON command adapter remains absent. Quantities are printed, not stored in USD
(the pinned schema has none).
A library target (`beamcraft_cli::plate`) exposes the same path with a commit hook for fault
injection in tests. `tests/independent_reader.rs` and `tests/reference_reader.rs` re-read the
saved layers with the independent `beamcraft-usd-oracle` process — its own parser, composition
and vector algebra, sharing no code with the layer engine — and cross-check the closed forms.
The gate runs in every `cargo test`, locally and in CI.
`tests/f04_cases.rs` and `tests/f06_cases.rs`
read the frozen contract fixtures and report their projections, deviations and omissions. See
the [P01 design note](../../docs/design/p01-first-plate.md) and the current
[K1 evidence and case inventory](../../docs/design/k1-contract-acceptance.md).

## P02 profile and material definitions

```text
beamcraft define-material --store ROOT [--density KG_M3] [--grade TEXT] [--standard TEXT]
  [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
beamcraft define-profile --store ROOT --width MM --height MM [--designation TEXT] [--standard TEXT]
  [--area-mm2 N] [--mass-per-meter-kg N] [--surface-per-meter-m2 N] [same id options]
beamcraft definitions --store ROOT
```

P02.2: a profile may declare its catalogue identity and section numbers, and a material its grade
and standard. They are optional, stay unknown when absent and are part of the definition's digest
(a definition without them keeps its earlier digest). Labels follow the element-label rule;
numbers are finite, greater than zero and capped. For the rectangle family the declared area and
surface per metre must agree with `width * height` and the perimeter within 0.1 percent
(`INVALID_ARGUMENT` otherwise); the declared mass per metre is stored as declared and is checked
against area times density only when a member quantity uses it (P04). A grade never selects a
density. The numbers are plain USD attributes (`steel:profile:areaMm2`, `steel:profile:massPerMeterKg`,
`steel:profile:surfacePerMeterM2`, `steel:profile:designation`, `steel:profile:standard`,
`bim:materialGrade`, `bim:materialStandard`).

`define-material` and `define-profile` publish one immutable definition as the next revision: a
`/World/Definitions/Materials/MAT_<sha256>` or
`/World/Definitions/Profiles/PRF_<sha256>` prim in the model layer, with units `mm` and schema version
`0.1.0`. The digest is the full SHA-256 of the canonical isolated definition layer that the layer
engine writes, so it pins the values, the units and the version together; the output is
`definition_created` with that digest, or `definition_exists` (no new revision, nothing written)
when the head already holds the same content. A material without `--density` keeps an unknown
density: no default is selected, no value is stored, and plates that pin it report mass `unknown`.
Density (greater than zero, at most 1,000,000 kg/m3) and rectangle dimensions (0.001 to 1,000,000
mm) are admitted with the F04 codes (`NON_FINITE_NUMBER`, `LIMIT_EXCEEDED`, `INVALID_ARGUMENT`)
before anything is written. `definitions` opens the verified head in a fresh process, re-admits
every definition, recomputes its digest from the stored values (a layer edited after publication is
`INTEGRITY_ERROR`) and lists digest, kind, values and the plates that pin it. A pinned digest that
is not in the model is `NOT_FOUND`; one that names a profile is `INVALID_ARGUMENT`.

Limits: no catalogue, designation lookup or environment default; the physical material carries no display appearance (a separate reference and signature
later); definitions are not an F04 command, so there is no `op_id` replay for them and publication
is idempotent by content digest; in-process layer engine as above. See the
[design note](../../docs/design/p02-definitions.md).

## P03 straight member

### Associative projection preview (D02.1)

```text
beamcraft preview-drawing --store ROOT --id MEMBER_UUIDV7
  --drawing-id UUIDV7 --view-id UUIDV7 --dimension-id UUIDV7
  --anchor-a QUALIFIED_VERTEX_ID --anchor-b QUALIFIED_VERTEX_ID
  [--plane front|top|right] [--scale 100] [--origin X,Y] [--offset MM]
  [--decimals 0..3] [--name TEXT]
  [--intent-out NEW_FILE.usda]
```

The engine projects twelve named edges of a real canonical rectangular member and measures the
projected interval between the selected member's two semantic vertices. Front uses model X/Z,
top X/Y and right Y/Z. Paper geometry is in millimetres; scale is a denominator (default 1:100),
and the dimension label reports projected model millimetres before scaling. This is a wireframe
with coincident/hidden edges retained, not hidden-line removal or a true 3D measurement.

The JSON includes the exact source project/revision/manifest/schema/generation, document object
IDs, retained anchor IDs, paper geometry and resolution states. A deleted owner, unsupported
body or anchors that project to less than 0.001 mm are explicitly unresolved with no numeric
dimension value. All output coordinates/offsets are capped at 10000 paper mm; scale is 1..10000.
The command is read-only and does not yet save/publish a drawing or export a document version.

`--intent-out` optionally creates a new ordinary USDA intent file after complete admission,
source inspection and projection. It never overwrites a destination and reports `intent_digest`.
The separate `beamcraft-drawing-usd/1` schema records intent and exact source pins, without copying
the model into it. This standalone intent is not a published version or a self-contained source
archive. The library `drawing::codec::{encode, decode}` uses admitted engine intent and bounded
USD reading/re-admission (16 KiB, three prims, canonical closed fields); source verification is
still a separate prerequisite. Historical publication/archive and derived geometry follow in D03.

The D03.1 prerequisite `drawing::archive::SourceArchive` captures a source revision under the
source store's shared maintenance guard. It retains exact `source-manifest` provenance and all
USD bytes under `model/`, outside drawing composition. Verification binds every source-pin field,
layer name/role/size/digest and the existing strict USD/schema/index/kernel inspection. Reopening
restores original virtual names and needs no live model or foreign filesystem source. Existing
model layer/count/total-byte budgets apply before capture. The document commands below retain
these resources; an in-memory archive alone is not a published document.

The D02.2 storage prerequisite `drawing::derived::{root, encode, verify}` composes authored
intent with ordinary linear `BasisCurves` in paper XY. Standard point3f display curves accompany
exact double3 coordinates and a double model-mm dimension value. A source/algorithm signature
binds derived data; verification recomputes the engine projection and compares exact bytes.
Unresolved geometry stores a reason without a successful value. The root composes only drawing
USD, never source/provenance resources. This API is used by the document-store commands;
`preview-drawing` continues to expose the same engine geometry headlessly.

### Drawing document lifecycle (D03.3)

`create-drawing --store MODEL --document-dir DOCUMENT` takes the same drawing/member/view/
dimension IDs, anchors and settings as `preview-drawing`, plus required `--op-id`,
`--revision-id` and `--actor-id`. It creates a dedicated document store and archives the exact
verified model. `refresh-drawing` takes those options and `--expected-head REVISION_UUID`,
creating a new draft against that document head. Neither command publishes automatically.

`publish-drawing --document-dir DOCUMENT --expected-head REVISION_UUID --op-id UUID
--revision-id UUID --actor-id UUID --version-id UUID` publishes the current draft as an immutable
version. A new publication must use a version ID that has never been published in this document;
reusing either the latest or an older ID is `DUPLICATE_ID` without a new revision. The exact
original operation still replays after later publications and leaves the current head intact.

Publication verifies the complete document history one revision at a time, bounded by the host
revision cap and a verification deadline. A shared maintenance lease protects that check through
the candidate CAS; ordinary writers remain subject to the head CAS. Missing or corrupt history
refuses publication. In particular, a compacted document history is `DEPENDENCY_UNAVAILABLE`:
restore the complete history before publishing again. Retained head inspection still works.

`inspect-drawing --document-dir DOCUMENT [--model-store MODEL]` reports the current document
and freshness (`current`, `stale`, `unavailable` or `unknown`). `inspect-drawing-versions --document-dir
DOCUMENT` lists retained published revisions. Archived models remain outside drawing composition.
This slice does not implement drawing UI, PDF/print, or the full D03 acceptance gate.

### Semantic feature addresses (D01.1)

```text
beamcraft list-feature --store ROOT --id ELEMENT_UUIDV7
beamcraft lookup-feature --store ROOT --feature-id QUALIFIED_FEATURE_ID
```

`list-feature` opens and strictly verifies the committed model, then enumerates the selected
canonical box body's 6 face, 12 edge and 8 vertex addresses. Each full `fid1` address includes
the owning ElementId and semantic role; array positions are not addresses. The result includes
the model revision UUID, manifest digest, generation and snapshot token. Both queries are
read-only. A move preserves addresses and changes the pin/coordinates; a copy has new addresses.
Missing owners and unsupported non-box bodies (including I-sections and holed plates) refuse
with `GEO_FEATURE_NOT_FOUND`. Triangle, numeric vertex and instance addresses refuse with
`GEO_TRIANGLE_ADDRESS_FORBIDDEN`. This query supplies associative references; it does not yet
create or publish a drawing.

### Create a member

```text
beamcraft create-member --store ROOT --start X,Y,Z --end X,Y,Z --profile-digest SHA256
  [--kind beam|column] [--roll RAD] [--material-digest SHA256] [--name TEXT]
  [--translation X,Y,Z] [--orientation-xyzw X,Y,Z,W] [--new-id UUID] [--op-id UUID] [--revision-id UUID] [--actor-id UUID]
  [--project-id UUID] [--workspace-id UUID]
```

`create-member` publishes one beam or column whose rectangle section comes from a profile
definition published earlier (`define-profile`); the optional material works as for plates. The
engine resolves both from the head (`NOT_FOUND` for an absent digest, `INVALID_ARGUMENT` for the
wrong kind), the kernel builds the box in the F06 member frame (section centred on the axis, the
default frame of F06 section 4 plus the explicit roll), and the commit runs the same verifier as
plates, which now re-evaluates members too. Output: element id, kind, profile and material digests,
section, axis points, roll, placement, volume and mass (as for plates), mesh digest, revision and
footprint (the profile and material are reads). `inspect-elements` lists `plates` and `members` and
puts both in the one pinned snapshot. Axis points may differ in all three coordinates: horizontal,
vertical and inclined beams use the same command, with no separate 3D flag. Coordinates are in
millimetres in a right-handed system with Z up; `--roll` rotates the section about the axis in
radians. An axis along `+X` gives width along `Y` and height along `Z` before roll. Optional rigid
placement (`--translation`, `--orientation-xyzw`) transforms the member without changing its local
mesh or quantities; the default orientation is the identity quaternion `0,0,0,1`.

For example, after defining a profile, create an inclined beam with a 13000 mm axis:

```sh
beamcraft define-profile --store ROOT --width 200 --height 300
# Use the returned definition.digest as PROFILE_SHA256.
beamcraft create-member --store ROOT --kind beam --profile-digest PROFILE_SHA256 \
  --start 100,200,300 --end 3100,4200,12300
```
See the [design note](../../docs/design/p03-straight-member.md).

## P04 update-element and stored quantities

```text
beamcraft update-element --store ROOT --id UUID
  [--name TEXT] [--material-digest HEX|none] [--thickness MM]
  [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

`update-element` edits one plate or member and writes only the groups that change: a rename
touches the authored layer alone, a material change rewrites the stored mass record at the same
mesh, a thickness change rebuilds the mesh and volume (plates only). `--material-digest none`
clears the material, so the mass becomes unknown. An empty patch is `INVALID_ARGUMENT`. Volume
and mass are stored beside the derived mesh with their own signatures; `inspect-elements` reports
`stored_quantities` as `current`, `stale` or `missing`, and the publish-time verifier refuses a
stale record with `INTEGRITY_ERROR`. See the [design note](../../docs/design/p04-quantities-update.md).
`--translation X,Y,Z` (P08) moves the element: it sets the absolute placement translation and keeps
the rotation, rewrites the authored layer alone and never rebuilds the body, its mesh or its stored
quantities.

## P08 move and delete

```text
beamcraft update-element --store ROOT --id UUID --translation X,Y,Z [ids as above]
beamcraft delete-element --store ROOT --id UUID
  [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

A move is the placement patch of `UpdateElement`: the footprint writes the placement and the
derived invalidation group only, the derived layer is byte for byte unchanged and the stored
volume and mass stay `current`. `delete-element` removes one plate or member from both layers and
nothing else (the other elements, the unknown content and the definitions it pinned stay). It is
refused with `DEPENDENCY_CONFLICT` while another element links to it (no cascade, no dangling
link), `NOT_FOUND` for an unknown element, and the same `--op-id` is a replay or `OP_ID_REUSE`.
See the [design note](../../docs/design/p08-move-delete.md).

## P05 copy-element

```text
beamcraft copy-element --store ROOT --id SOURCE_UUID [--name TEXT] [--offset X,Y,Z] [--new-id UUID]
  [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

`copy-element` creates one new plate or member with a fresh id and the source's authored inputs;
the label defaults to the source's and `--offset` is added to the placement translation. The body
is element-local, so the copy has the same mesh digest, volume and mass; the pinned profile and
material are shared by digest. The source is only read (the footprint says so and is checked).
Editing the copy later rebuilds only the copy. A reused identity is `DUPLICATE_ID`, an unknown
source `NOT_FOUND`, and the same `--op-id` is a replay or `OP_ID_REUSE`. See the
[design note](../../docs/design/p05-copy-element.md).

## P06 personal workspaces

```text
beamcraft create-workspace  --store PUBLISHED --workspace-dir DIR [--user-id UUID] [--workspace-id UUID]
  [--revision-id UUID] [--op-id UUID]
beamcraft inspect-workspace  --store PUBLISHED --workspace-dir DIR
beamcraft publish-workspace --store PUBLISHED --workspace-dir DIR
```

A workspace is a directory that is its own revision store, forked from the published head. Every
edit command (`create-plate`, `update-element`, `copy-element`, ...) works on it with
`--store DIR`; the published store is untouched until `publish-workspace`. Two workspaces of one
user are two directories and stay independent. `inspect-workspace` reports the unsent draft
revisions and whether the base is still the published head. `publish-workspace` (the published revision id is the draft head's id, so an interrupted publish is retried as the same request) is a fast-forward
only: a moved published head is `BASE_REVISION_MISMATCH`, a fresh fork is `INVALID_ARGUMENT`, and
repeating a finished or interrupted publish is a replay. There is no authentication and no merge.
See the [design note](../../docs/design/p06-workspace.md).

`workspace::WorkspaceState` serializes metadata transitions with an OS lease and a complete
expected-snapshot comparison. Reads and base updates return `BACKEND_UNAVAILABLE` while another
holder is transitioning; a stale metadata writer gets `BASE_REVISION_MISMATCH`. Ordinary local
model edits still use the draft revision CAS. The metadata guard also blocks compaction while
held. Workspace/user identity and the published local prefix cannot be rolled back, and legacy
v1 metadata bytes stay unchanged. This primitive does not implement rebase or its recovery.

A nonzero `rebase_sequence` uses canonical metadata v2 and identifies the earliest source endpoint
publication may send. Older payloads and a boundary beyond the draft head are refused. Status
exposes that sequence; snapshot undo stops there, while later edits remain undo/redo units. The
bounded `workspace::rebase_receipt` codec binds original metadata, source/accepted pins, ordered
operations and dependency evidence. It does not itself produce or recover a runtime rebase.

`WorkspaceState::stage_rebase` syncs `rebase.pending` before the candidate CAS; the caller must
first verify/pin the accepted basis, prove independence and materialize the candidate. Every state
open resolves that intent before exposing metadata. Recovery verifies the original source union,
the exact candidate slot/base, strict-verifier index and receipt pin; it advances metadata once
without classifying retained commands or later edits as published. A missing/other CAS winner
abandons the intent without changing metadata. Corrupt or unavailable evidence fails closed.
As with other store writes, the complete file is synced; directory-entry durability is not promised.
Status, packaging, direct publication and undo retain the state guard across their snapshots.
Receipt aggregation checks ordered prefixes and prior bases, retaining the original command IDs
without counting administrative copies of their dependencies again. This is recovery plumbing;
an explicit rebase command and its complete acceptance remain separate.

`rebase-workspace --store PUBLISHED --workspace-dir DRAFT --op-id UUID --expected-head UUID
--expected-draft UUID [--actor-id UUID]` explicitly retains independent unsent work over a newer
accepted basis. It pins publication while planning/committing the personal draft, verifies all
intervening command evidence, materializes in isolation and runs the kernel/quantity verifier.
It never publishes automatically. Missing proof refuses; semantic conflicts return complete
target/group and operation diagnostics. Unknown/reference-wrapper deltas retain their dedicated
writers and refuse this materializer. The local actor defaults to the recorded workspace owner;
this is trusted CLI context, not authentication. The authenticated coordinator rebase endpoint
is separate. Retrying the same ID/pins returns its original revision, preserving later edits;
another payload under that ID is refused, including after a failed attempt.

## P07 undo and redo

```text
beamcraft undo-workspace    --workspace-dir DIR [--revision-id UUID] [--op-id UUID]
beamcraft redo-workspace    --workspace-dir DIR [--revision-id UUID] [--op-id UUID]
beamcraft inspect-workspace-history --workspace-dir DIR
```

Every accepted command is one revision of the draft and therefore one undo unit. `undo-workspace`
commits a **new** verified revision whose layers are those of the state before the last unit, so
the element, its mesh and its stored quantities come back together; `redo-workspace` restores the
undone state. The stacks are derived from the revision log and the append-only `undo.log` (an edit
after an undo discards the redo stack), so they survive a restart and a crash between an intent
and its commit; the same `--op-id` replays or finishes a step. Nothing to undo or redo is
`INVALID_ARGUMENT`; undo does not reach before the fork and does not touch the published store.
See the [design note](../../docs/design/p07-undo-redo.md).

## R01 read-only references

```text
beamcraft attach-reference        --store ROOT --source FILE [--name TEXT] [--translation X,Y,Z] [--rotation-z-deg DEG]
  [--new-id UUID] [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
beamcraft move-reference          --store ROOT --id UUID [--translation X,Y,Z] [--rotation-z-deg DEG] [same ids]
beamcraft correct-reference-units --store ROOT --id UUID (--meters-per-unit F --up-axis Y|Z | --clear true) [same ids]
beamcraft write-reference         --store ROOT --id UUID (--target source | --prim /DefaultPrim/Path [--attribute NAME])
beamcraft inspect-reference       --store ROOT
```

`attach-reference` reads a local USDA or USDC layer (at most 32 MiB, one layer with a
`defaultPrim`, under the model-profile stage budgets) and publishes
a revision in which its bytes are an
`external` layer named by their SHA-256, composed below an owned wrapper element. The file is only
read; its path is never stored (only its file name, as a label). A URL, a network share or a device
path (any `scheme:` of two or more characters, or a path starting with two slashes or two
backslashes) is `PERMISSION_DENIED` on the text alone,
before the store or the file system is touched. The layer is displayed, not validated: values and
encodings it uses that the snapshot cannot hold, and arcs to other assets (payloads, variants,
sublayers, references, URLs), are left out and never resolved, so a source made of payloads to other
files shows only what its own layer holds. The wrapper has its own placement
(`move-reference`, absolute, stored apart from the foreign content) and, only when the owner asks,
an explicit units and up-axis interpretation (`correct-reference-units`; `--clear true` removes
it). The source keeps declaring what it declares; `units_status` in the output says whether it
matches the project (millimetres, Z up), `needs_correction`, is `undeclared`, or is `corrected`.
Without a correction the source is taken as stored.

`write-reference` has no success path: the source layer and every prim of the source (existing or
not, including an override a layer of ours would author) answer `FOREIGN_READ_ONLY`, exit 2, and
nothing is written. `inspect-reference` opens the verified head (every blob re-hashed) and lists
the wrappers, their pinned source, the foreign prims and the external layers. Every other command
(`create-plate`, `update-element`, `publish-workspace`, `undo-workspace`, ...) carries the external
layer into its revision unchanged. See the [design note](../../docs/design/r01-readonly-reference.md).

U16 uses the [category-based prim paths](../../docs/architecture.md#prim-hierarchy-and-naming-rule)
in saved layers and `prim_path` output. Stores with earlier paths or schema pins are refused;
there is no migration during this pre-alpha compatibility break.
