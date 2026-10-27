# F12: external IFC references and report exchange, version 0.1.0

Status: **draft contract; workflow required by the owner on 2026-10-22; not implemented**.
Companion to [F12](f12-project-coordination-v0.1.md), its
[opening assignments](f12-opening-assignments-v0.1.md) and
[common Craft reading](f12-craft-read-capabilities-v0.1.md), in the same branch and PR.

**Scope update 2026-10-27:** the owner requires IFC reference loading in the MVP and separately
requires native steel IFC import through the engineering kernel into an editable project,
including KM to KMD. The latter needs its own detailed import contract; it does not make a
reference editable or change the reference rules below. This scope approval does not bring all
F12 coordination/assignment workflows into the MVP. See the [owner decision](../plan/decisions.md#2026-10-27).

## 1. External references participate in the common workflow

F12 PC-WORK-001 covers companies authoring every section, just one section, or several sections
with the rest supplied externally. This companion specifies the IFC/report part of that same
workflow, not a separate single-section product mode. Each model independently has an authored
or external-reference role; both may coexist in one project view. The common scope, revision
and assignment rules apply across all three arrangements.

The workflow must work when only one participant uses Craft. A KM company can create its own
project with an architectural IFC reference; an OV company can model services against a
structural IFC reference and send opening assignments to an external engineer as a report.
Neither counterpart needs a Craft application, module, project store, revision history or
account. A coordination project covering every company is optional.

Two source types coexist: a native Craft project pinned by project/revision/manifest, and an
external delivery pinned by its received bytes and admitted conversion. They share read-only
reference behaviour but do not pretend to have the same identities or capabilities.

## 2. One IFC delivery becomes one logical USD reference

**XR-SOURCE-001.** Retain the exact received IFC as immutable source evidence. Convert the
supported subset into an immutable USD reference root with source-to-output mappings and a
conversion report. Attach that root through an owned reference wrapper. One IFC delivery is
one logical reference, not a collection of independently editable native elements. The root
may package immutable dependent geometry resources when needed; a single giant physical USD
file is not required. Attach only after the required converted resource closure is verified.

**XR-SOURCE-002.** The conversion record pins original-byte SHA-256 and size, declared IFC
schema, converter identity/version, conversion settings, units/frame interpretation, mapping
and loss-report digests, and exact output resources. The received filename and supplier's
revision label are descriptive metadata, not sufficient version identity. A new converter or
different settings can change output for the same IFC; record that as a distinct conversion.
No determinism or complete IFC support is assumed without validation of the selected converter.

**XR-IDENTITY-001.** A local reference lineage identifies successive deliveries of the same
external model. Its persistent wrapper/reference identity is an F04 ElementId, scoped by the
receiving ProjectId; it is not a fabricated source Craft ProjectId. A source element reference
contains that lineage, the exact IFC source digest and an original IFC identifier. Preserve
IFC GlobalId where present and valid; do not reinterpret it as a BeamCraft UUIDv7. Keep one
source element's association with all of its converted geometry pieces.

GlobalId is matched only within an explicitly selected source lineage. Missing or duplicate
identifiers are diagnosed. A snapshot-local locator, such as an IFC entity number, may identify
an element within that file, but cannot establish continuity across deliveries. Geometry
similarity, names and mesh order do not silently establish identity. Ambiguous matches remain
unresolved and block dependency-sensitive approvals until explicitly reviewed.

**XR-READ-001.** The converted source stays foreign and read-only even if a Craft application
has an enabled module for its apparent kind. Conversion grants no engineering ownership.
Users author their own KM/OV content beside it; they cannot rename, move, delete, cut or override
foreign elements. Explicit wrapper placement affects the reference as a whole, not its source.
Importing as editable native objects or as owned I03 items is a separate workflow and is not
authorized by this reference conversion.

**XR-FRAME-001.** Record the source units, placements and supported georeferencing, the
conversion into the admitted USD frame, and the wrapper's placement into the local project.
Each transform applies exactly once. Unsupported or ambiguous alignment is explicit, not
guessed. Reports give units and coordinate-frame identity, including the mapping back to the
referenced source where supported; receiver coordinates must not depend on an undocumented
Craft viewport transform.

**XR-LOSS-001.** List unsupported, omitted or approximated geometry/properties and unresolved
identities. The converted model is a reference representation, not certified native engineering
geometry. Missing geometry remains a known limitation; it cannot prove clearance, absence of
clashes or a safe hole. A partial conversion can be inspected with warnings, but required
missing information prevents a task from being labelled completely validated.

## 3. Updating a received model

**XR-UPDATE-001.** Receiving a replacement IFC never overwrites the old IFC, conversion or
reference pin. The user explicitly associates the delivery with a lineage, reviews the
conversion/mapping and publishes a reference update in their own project. Original pins and
evidence remain reachable while assignments or retained history depend on them.

Changes, deletions, conversion differences, wrapper alignment changes and unresolved mappings
mark dependent assignments or engineering references for review. They do not rewrite owned
pipes, steel elements, opening proposals or prior decisions. The source snapshot in a pending
assignment remains the one actually used, even if the currently displayed reference advances.
The full `builtFrom` workflow is deferred for a decision after the MVP; this clause still
requires external source evidence for the approved reference and assignment workflows.

## 4. Reports to participants outside Craft

**XR-REPORT-001.** The existing assignment identity and proposal history remain in the sender's
Craft project. A human-readable report is another export of exact proposal versions, not a
new assignment identity or a requirement that the recipient accept a Craft exchange package.
For an external recipient, record the organization/role/contact as recipient metadata without
inventing a native ProjectId, SectionId or authenticated ActorId for that party.

The report contains the sender/project identification; full assignment identities and proposal
versions; the source IFC filename/label and exact source digest; source element identifiers
where resolved; shape, dimensions, clearance and position with units/frame; purpose; and enough
locating context (for example annotated views, levels or grids) for the recipient to review it
without Craft. Missing host identity/frame is explicitly reported and cannot silently become
an approval-ready request. A structural target and responsible reviewer are identified by the
task; possessing an architectural IFC does not make its author the structural approver.

Record the exact exported report bytes and their digest with the proposal versions it contains.
A report revision is immutable. Re-exporting or changing its layout does not create duplicate
assignments; changing the requested hole creates a new proposal version. Retain the issued
report so a later answer can be traced to what the recipient actually saw. PDF/HTML, tabular
attachments or a structured exchange standard are possible delivery formats; no codec or
external interoperability is selected or claimed here.

**XR-REPLY-001.** A response may be an external document or correspondence, not a Craft
decision file. An authorized user records the evidence and its source, the stated reviewer,
the exact assignments/proposals answered, the reviewed external model snapshot and the result.
Delivery, acknowledgement, explicit external acceptance and implementation remain separate.
Sending a report or receiving an unqualified "OK" with no resolvable proposal/basis cannot
produce a verified acceptance. Preserve uncertain evidence with unresolved applicability.

The record distinguishes a manually recorded external decision from a native authenticated
Craft decision. It follows OA-AUTH-001: local entry records evidence, not invented authority.
No parsing of a PDF, email or filename automatically grants approval. The same 150/200 mm rule
applies: a response to the old request never approves or overwrites the successor. A changed
or unknown external model basis requires revalidation or remains unresolved.

**XR-IMPL-001.** The external engineer creates accepted openings in their own authoring tool.
Craft does not execute an edit against the converted reference. Implementation can be recorded
as externally reported evidence, or checked against a later IFC delivery with resolvable
identity and geometry. These are different evidence levels; a favourable response or the mere
appearance of a hole is not a verified implementation of the exact accepted proposal.

## 5. Required acceptance cases and implementation gates

| Rule | Positive case | Required refusal or explicit limitation |
|------|---------------|----------------------------------------|
| XR-SOURCE-001/002 | One IFC yields one logical USD reference with original bytes and a pinned conversion report | Missing required output resources refuse attachment; changed settings cannot reuse old conversion identity |
| XR-IDENTITY-001 | Original GlobalId maps to every display piece and a source lineage/version | Duplicate/missing IDs or renumbered entities cannot silently match across deliveries |
| XR-READ-001 | A KM-only company models steel beside an architectural IFC reference | Enabling a matching module cannot make foreign geometry editable |
| XR-FRAME-001 | Source and report coordinates agree with independently expected positions after unit/frame conversion | Ambiguous units/georeferencing or double-applied placement cannot be reported as valid alignment |
| XR-LOSS-001 | Partial conversion lists omitted geometry/properties and still permits qualified inspection | Missing source geometry cannot count as "no clash" or validated opening clearance |
| XR-UPDATE-001 | Explicit IFC update keeps old assignment bases and flags changes | New IFC cannot overwrite old evidence or automatically move owned steel/pipes |
| XR-REPORT-001 | An OV-only company exports identifiable opening requests readable without Craft | Recipient is not required to have a Craft project/account/module; incomplete context is visible |
| XR-REPLY-001 | An authorized user links a verifiable external response to V1 and its source IFC | V1 acceptance never approves V2; ambiguous evidence stays unresolved; report delivery alone grants no approval |
| XR-IMPL-001 | External completion evidence and later observed geometry have distinct statuses | Importing a reply never cuts the converted reference or silently certifies external construction |

These are future executable requirements, not run tests. F04/R01/F10 refusals remain applicable;
no raw USD setter, unrestricted importer or automatic native-element conversion is introduced.
E04 must choose the supported IFC subset and converter with licensing/provenance and bounded
input handling; R01/R03/R05 need the reference-profile, update and property mappings. Original
IFC/report evidence retention may require a manifest extension; existing manifests are not
claimed to accept arbitrary non-USD resources. Concrete USD mappings, report format, external
decision admission, budgets and independent conversion fixtures remain implementation gates.
The owner has selected the read-only reference workflow; this does not declare its implementation
complete or change the deferred I01–I03 item workflow into the MVP.
