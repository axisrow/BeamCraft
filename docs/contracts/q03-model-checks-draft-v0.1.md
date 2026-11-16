# Q03: model checks contract draft (`check-model`), version 0.1.0

Status: **draft.** This document is the slice 1 contract of [#634](https://github.com/axisrow/BeamCraft/issues/634)
(delivery task [#661](https://github.com/axisrow/BeamCraft/issues/661)): it defines what the
`check-model` harness checks, against what basis, with what tolerances and report semantics.
Nothing in it is implemented yet; slices 2–5 implement it and change code and this document
together. The normative research of section 3 was retrieved on 2026-11-15; unverified rules stay
outside the supported set.

The guarantee is detection within the declared scope and the supported rules. These checks do
not block export or publication, and a green report is not a structural design approval. The
engine checks the evaluated model against explicit requirements and applicable rules; intent
that cannot be inferred from geometry is recorded, never remembered by an LLM or reconstructed
from a picture.

## 1. Rules

1. **Numbers come only from recorded requirements or from pinned, cited rules.** The product
   does not invent or judge dimensions the user did not give. Where a check needs a number and
   no recorded requirement or supported rule supplies one, the result is `unknown` with the
   missing inputs named.
2. **A requirement is an input, not a measurement.** A stored required value is never replaced
   by a measured one, and no requirement is silently rewritten because another input changed.
   The sample's 10 mm corner cut stays 10 mm until the owner changes it; a 10 mm cut does not
   imply a 10 mm weld.
3. **`unknown` is a status, never a pass.** Unknown mass is not shown as zero; an unknown check
   is not shown as green and blocks nothing.
4. **Every result names its basis**: the explicit requirement, the rule reference, or the
   geometric identity it rests on. A result without a basis is refused, not shipped.
5. **Final evaluated bodies only.** Checks run on the solid after all supported operations
   (corner treatments, holes, plane cuts, fits). An uncut or unholed proxy is never tested as
   if it were the finished part.
6. **Detection only.** `check-model` reports; it does not edit the model, block export or
   publication, or auto-repair geometry. Automatic changes to cut geometry are a separate,
   still-open owner decision (recorded in `docs/plan/decisions.md`).
7. **Provenance follows `AGENTS.md` section 2**: public specifications cited by document,
   edition and clause (section 3). Nothing is taken from commercial detailing software, and no
   number enters the supported set without its retrieval recorded here.

## 2. Scope of the first delivery

### 2.1 Classification of the planned checks

Each planned check is classified by what kind of basis it may consume:

- **Geometric** — decidable from the final evaluated geometry alone, with the tolerances of
  section 7. No normative content.
- **Owner constraint** — needs a requirement recorded by the owner (sections 4–5) plus
  geometric detection. Without the recorded requirement the check is *not applicable*
  (established absence of a declared requirement), not `unknown`.
- **Normative** — needs a public rule pinned by document, edition and clause in the supported
  set (section 3). Today the supported set is empty, so a check whose remaining content is
  normative stays `unknown` unless the owner recorded an explicit requirement for it.

| Code | Classification | Basis in v0.1 |
|------|----------------|---------------|
| `PART_CLASH` | geometric | penetration beyond the product contact tolerance (section 7), including full containment; no normative content |
| `PART_WITHOUT_MATERIAL` | geometric (completeness) | a part lacks a material and its mass is unknown |
| `PART_DEGENERATE` | geometric (completeness) | established degenerate geometry state, reported with its existing `geometry_reason`; unsupported geometry is not relabelled degenerate |
| `QUANTITY_STALE` | geometric (completeness) | the existing quantity assessment reports stale stored volume or mass; the existing status and cause are reused, accepted stale snapshots distinguished from integrity failures |
| `PART_UNCONNECTED` | owner constraint + geometric | a required connection declared in joint intent is absent for an element in the declared scope; no declared scope ⇒ not applicable |
| `WELD_WITHOUT_CONTACT` | owner constraint or normative | gap at the validated local joint outside the permitted range; v0.1 checks it only against an explicit owner gap requirement — no normative permitted-gap rule is supported yet (section 3.3): a declared weld connection with neither a recorded gap requirement nor a supported rule is `unknown` (`no_supported_rule`), and with no declared connection to judge the check is `not_applicable`; unvalidated local attachment is `unknown` too |
| `CONTACT_WITHOUT_CONNECTION` | owner constraint + geometric | a contact that declared joint intent requires to be connected has no such connection; contact alone never proves a weld is needed |
| `CORNER_NOT_CUT` | owner constraint + geometric | a required corner cut is absent, undersized, or does not provide the recorded local clearance; v0.1 checks existence and size only against recorded requirements — no normative existence rule for this joint is supported (section 3.2) |
| `CORNER_OVERCUT` | owner constraint | a cut exceeds the upper edge of the recorded requirement's acceptance band (section 7.3): the explicitly recorded size is itself the upper bound #634 allows, and a recorded standalone maximum tightens it; no normative upper bound exists (section 3.2); with no recorded corner-cut requirement at all the check is `not_applicable` |
| `CORNER_WELD_CLEARANCE` | owner constraint + geometric | the final body of the required rib must not intrude into the weld fillet that passes its derived corner, judged from the weld record's recorded leg and validated path (section 4.4); located by the recorded corner-cut requirement — with none the check is `not_applicable`; the recorded leg is a model fact, not a normative limit, and no weld adequacy is validated |

A check requested but not in this table is `unknown` with reason `unsupported_rule`; it never
enters the supported set and never reports pass or fail. Unverified rules stay unknown: that is
the difference between "not checked" and "checked and fine".

### 2.2 Not in the first version

Blocking export and publication; waivers of findings; general weld-strength/design verification
and normative tables beyond the supported corner-cut rules (including normative weld leg limits
and bolt edge distances — the dependent geometric clearance check of section 4.4 is not one: it
consumes no normative table and validates no weld adequacy); a checks panel in the window;
automatic fixing or automatically changing cut geometry when related dimensions change.

## 3. Normative research: the column-base joint

The sample joint (from [#559](https://github.com/axisrow/BeamCraft/issues/559), corrected by
[#634](https://github.com/axisrow/BeamCraft/issues/634)): a base plate on a foundation, a column
standing on it, and vertical stiffener ribs between the column flanges. Each rib's **lower inner
corner** — where the rib meets both the column flange and the base plate — carries a corner cut
(clearance notch). The owner requested a **10 mm** cut for this example; that dimension is a user
input for this model, not a specified weld leg and not a universal normative dimension.

Three distinct requirements are easily conflated and must stay distinct in every rule: **weld
clearance** (space for the weld and its root opening at this joint), **access for welding**
(room to start, run and terminate the weld beyond an obstruction), and **other detailing
requirements** (drainage, corrosion, erection tolerance, restraint relief). A source may speak
to one of them and none of the others; a check names which one its basis addresses.

### 3.1 Sources and retrievals

Retrieved 2026-11-15. Each entry states what the source says, where it was retrieved, and its
applicability limit for this joint.

| # | Source, edition, clause | What it establishes | Retrieval | Applicability limit |
|---|--------------------------|---------------------|-----------|---------------------|
| S1 | **SP 16.13330.2017** "Стальные конструкции" (актуализированная редакция СНиП II-23-81*, amendments No. 1–3, No. 3 dated 2022-01-16), clause **15.5.2** (section 15.5 "Балки") | welded beams' transverse stiffeners **must have cutouts** («вырезы») so the continuous web-to-flange (поясные) welds can pass | [Public mirror of the specification text](https://meganorm.ru/mega_doc/norm_update_29032025/pravila/0/sp_16_13330_2017_svod_pravil_stalnye_konstruktsii.html) (the mirror cited by #634); full text confirmed via the [flamax.ru PDF](https://www.flamax.ru/upload/iblock/8e1/pgbdxh35dekyyl05vkliyx43vif7gdb7/SP_16.13330.2017_Stalnye_konstruktsii.pdf) and the [gktgs.ru PDF with amendments](https://gktgs.ru/assets/files/sp/sp-16-13330-2017-stalnye-konstrukcii-aktualizirovannaya-redakciya-snip-ii-23-81-s-popravkoj-s-izmeneniem-n-1-2.pdf) | **Scopes to welded beams** (web, flanges, transverse stiffeners): the cutout exists to pass the beam's flange weld. It gives **no cut dimension** and does not govern a base-plate rib, a different joint type. At most an analogy; not a normative basis for this joint in v0.1 |
| S2 | **SP 16.13330.2017**, clause **14.1.7** and **Table 38** | fillet weld leg bounds: `kf ≤ 1,2·t` (`t` = the thinner element) and `kf ≥` the Table 38 minimum by element thickness and steel strength; amendment No. 2 refines the case `t ≤ 0,6·T` | same mirrors as S1; Table 38 values interpreted in the public [buildingclub.ru summary](https://buildingclub.ru/katety-svarnyh-shvov-soglasno-sp-16-13330) | These bounds govern **the weld**, under conditions (joint type, loading, steel grade) the model does not record. They are not a corner-cut sizing formula. They also do not turn a 10 mm cut into a 10 mm weld. v0.1 does not validate weld adequacy at all (section 3.3) |
| S3 | **EN 1993-1-8:2005** (Eurocode 3, joints), **§6.2.6 "Column bases"** | design resistance of column bases: base plate bending, concrete bearing, anchor rods; stiffened components as T-stub equivalents; welds by §4 | [Full text PDF](https://www.phd.eng.br/wp-content/uploads/2015/12/en.1993.1.8.2005-1.pdf); BS EN 1993-1-8:2005 confirmed as the widely adopted edition, with a second-generation revision and national annexes behind it | A **design** code (resistance of the joint as designed). It prescribes no detailing dimension for a rib corner cut. Applicability depends on the national annex in force; not a detailing source |
| S4 | **AISC Engineering FAQs**, "8. Welding", FAQ **8.2.4** (weld access holes) | the qualitative purpose of access openings: welder access beyond an obstruction, reduced restraint for shrinkage, and avoiding the intersection of orthogonal welds | [AISC Engineering FAQs, 8. Welding](https://www.aisc.org/aisc/solutions-center/engineering-faqs/8-welding/) (retrieved 2026-11-15, quoted in full) | Justifies that **a cut may be needed** (access/clearance); states **no dimension**. Supporting guidance, not a dimensional rule |
| S5 | **AWS D1.1/D1.1M** (Structural Welding Code — Steel), clause **7.21.1** in the current edition numbering (**5.21.1** in the 2000 edition), via AISC FAQ **8.3.8** | fillet-weld fit-up: a root opening ≤ 1/16 in. is permitted without modification; > 1/16 in. up to 3/16 in. is permitted if the weld size is increased by the root opening or the required effective throat is demonstrated; for plate ≥ 3 in., 5/16 in. with suitable backing | FAQ 8.3.8 in the [AISC FAQ page](https://www.aisc.org/aisc/solutions-center/engineering-faqs/8-welding/); the older numbering visible in the public [AWS D1.1:2000 copy](https://law.resource.org/pub/us/cfr/ibr/003/aws.d1.1.2000.pdf) | **Nonzero separation is not automatically an invalid weld.** This is a *permitted-gap* regime for **weld sizing**, not a corner-cut dimension and not a contact requirement. Imperial units; edition-dependent clause numbering. The strongest candidate basis for a future `WELD_WITHOUT_CONTACT` permitted-gap rule — but it is **not pinned as a supported rule in v0.1** (section 3.3) |
| S6 | **AWS D1.1/D1.1M Table 8.1** (permitted fillet weld underrun), via AISC FAQ **8.3.4**; profiles in §7.23 | a fillet weld may underrun its nominal size by 1/16–1/8 in. (by weld size) without correction, if the undersize portion ≤ 10 % of the weld length | same AISC FAQ page | A tolerance **on the deposited weld**, measurable only against fabrication records the model does not have; not checkable from the model in v0.1 |
| S7 | **EN 1090-2** (execution of steel structures), welding fit-up tied to EN ISO 5817 quality levels by execution class | search-level evidence of a quality-level-based root-gap regime (an EXC3/quality-level-B cap of 2 mm for `t > 3 mm` per an SCI/BCSA guidance note surfaced in search) | no clause-level retrieval achieved in this slice; the guidance-note snippet carried no stable retrievable URL | **Candidate, not verified.** No clause is pinned, so EN 1090-2 contributes nothing to the supported set in v0.1. A later slice must retrieve the exact clause (or drop it) before any use |
| S8 | **AISC Design Guide 1** — 2nd ed. 2006 (Fisher & Kloiber, "Base Plate and Anchor Rod Design"), 3rd ed. retitled "Base Connection Design"; revision research in the public [AISC report by Kanvinde (2022)](https://www.aisc.org/media/1x2bmccf/aisc-lrr-2022-01_kanvinde_column-bases.pdf) | design of base plates and anchor rods: plate sizing by bearing and bending, column-to-base-plate welds, anchor rod design | editions and scope confirmed via search; the Kanvinde report is the official public document | **Design guidance** for load-carrying sizing of the joint, not a detailing code; it does not size rib corner cuts. Welds of column-to-base-plate are designed, not dimensioned for clearance here |

### 3.2 Findings of the research

1. **No retrieved public source establishes a universal corner-cut range or an automatic upper
   bound for this joint.** The proposed `K…K+5 mm` range has no normative basis and stays
   removed (#634). No source justifies declaring a 20 mm cut wrong merely because the weld leg
   is 10 mm — that comparison is only legitimate against the **recorded 10 mm requirement**.
   A clearance minimum (S5) never establishes a maximum cut.
2. **Existence of a cut** is normatively supported for a *different* joint (S1: welded beams'
   transverse stiffeners over continuous flange welds). For the base-plate rib, the justification
   is qualitative only (S4: access, restraint, avoiding orthogonal weld intersections) plus the
   owner's explicit requirement. v0.1 therefore checks cut existence and size only where an
   explicit requirement is recorded; the SP clause remains an analogy and is not loaded as a rule.
3. **Weld-leg adequacy is not validated in v0.1.** Real limits exist (S2, S6), but applying them
   needs applicability conditions — joint type, loading, steel grade, fabrication records — that
   BeamCraft does not record. A requested weld-adequacy check stays `unknown` until a slice pins
   a supported rule with those inputs; green geometric checks do not certify the weld.
4. **The applicable governing standard is a per-project choice**, not a product default:
   SP 16.13330.2017 for a Russian project, the Eurocode line (EN 1993-1-8 with its national
   annex and EN 1090-2 for execution) for a European one, AISC 360 with AWS D1.1 for a US one.
   S3/S5/S8 are design or execution sources, not interchangeable constants. The contract keeps
   the rule context explicit (section 6) instead of silently picking one.
5. **Permitted-gap regime**: the only retrieved, quotable rule of this kind is S5. It supports
   the general statements "a small nonzero gap is permissible" and "a larger gap demands a weld
   size increase or a demonstrated throat". Because BeamCraft does not model deposited weld size
   adjustments, applying S5 as a supported rule is deferred; until then `WELD_WITHOUT_CONTACT`
   runs only against explicit owner gap requirements (section 2.1).

### 3.3 Consequence for the supported set

The supported set of v0.1 is: the geometric rules and the owner-constraint rules of section 2.1.
It contains **no normative rule**. A normative rule enters the supported set only when a later
slice adds, in this contract: the pinned document, edition and clause; the joint types it
applies to; the inputs it requires; and its retrieval. Until then every check whose remaining
content is normative reports `unknown` with reason `no_supported_rule` — explicitly, and in
every report that requests it.

## 4. Owner requirements: stored inputs

### 4.1 What is recorded

Requirements are **model data**: they live in the store, are carried by revisions like authored
inputs, are created and edited only by explicit commands, and are never inferred from geometry,
from a weld, or from another requirement. Nothing in the check harness writes requirements.

A **corner-cut requirement** (the sample's case) records:

- `id` — typed UUIDv7;
- `kind` — `corner-cut`;
- `target` — the rib element UUID plus the two neighbour element UUIDs that sandwich the corner
  (for the sample: the base plate and the column). The corner itself is **derived, not stored**:
  on every run the harness re-derives the rib's lower inner corner adjacent to both neighbours
  from the final evaluated geometry (section 4.3);
- `required_mm` — the required cut size (the sample: 10.0);
- `tolerance_mm` — the stated tolerance on the comparison (section 7.3);
- `upper_bound` — `none` (the default) or an explicit recorded standalone maximum with its
  own tolerance. The recorded `required_mm` already bounds the cut in both directions: its
  acceptance band's upper edge (section 7.3) is the basis of `CORNER_OVERCUT` and its lower
  edge the basis of `CORNER_NOT_CUT` — the owner's own recorded size is the upper bound, so
  no separate and no normative maximum is needed or invented. A recorded `upper_bound`
  replaces the band's upper edge when the owner states a standalone "no more than X" limit;
- `basis` — `{ source: "owner" }` and an optional note (the sample: "owner's requested corner
  cut for this joint"); never a normative citation, because none exists for this joint
  (section 3.2);
- purpose — `clearance` (space for the joint) or `access`; the sample is `clearance`.

A **gap requirement** records, for one declared connection (section 5): permitted minimum and
maximum gap with tolerances, and basis `{ source: "owner" }`. It feeds `WELD_WITHOUT_CONTACT`
in v0.1 without any normative rule.

Other requirement kinds are refused at admission in v0.1; unknown kinds never pass silently.

### 4.2 Independence and recalculation

The 10 mm cut requirement is independent of any weld leg and of any other requirement. Changing
a weld leg recomputes **dependent checks** (a clearance check may turn red) but must not
silently change the requirement. If a requirement and a dependent check conflict, the report
explains the conflict; nothing is rewritten. Dependent requirements recomputed from recorded
inputs and the current model on every run are computed values and are labelled as such in the
report; a computed value never replaces its recorded input.

### 4.3 Targeting and ambiguity after edits

Requirements target elements by UUID and neighbours by UUID. On every run:

- **resolved** — the target element exists in the checked revision and the corner derivation
  finds exactly one candidate corner adjacent to both neighbours: the check runs;
- **unresolved** — the element is missing, or the derivation finds zero or several candidates
  (deleted rib, edited contour, relocated or replaced element): the affected finding is
  `unknown` with reason `target_unresolved` and the missing/ambiguous inputs named. The
  requirement stays recorded; the harness never auto-retargets it, never deletes it, and never
  picks a nearest corner as a repair.

A weld elsewhere between the same parts never satisfies or invalidates a corner requirement:
the requirement's target is the corner, and a distant weld does not address it.

### 4.4 The dependent weld-clearance check at the corner

The corner-cut requirement of section 4.1 locates one corner; the weld that passes it is a
model fact, and the space the cut frees for that weld is a **dependent check** (section 4.2):
changing a weld leg recomputes it, and the recorded requirement is never rewritten from it
(#723). The basis is geometric; no normative number enters it, and a green clearance never
certifies the weld.

**Which weld counts.** Among the model's welds connecting the two recorded neighbours, the
judged weld is one whose local attachment is validated (section 5.1) and whose authored path
passes the derived corner: the corner lies within the product tolerance of the path polyline.
A validated path that misses the corner is a weld elsewhere between the same parts (section
4.3): it produces no corner finding. With no qualifying weld and no unvalidated weld between
the neighbours the clearance check is `not_applicable` for that requirement — an established
absence. A weld between the neighbours whose attachment stays unvalidated (a pathless A06
weld, or a path that misses both surfaces) leaves the clearance `unknown`
(`attachment_unvalidated`) when no qualifying weld exists: the model cannot establish whether
that weld passes the corner. The independent size checks `CORNER_NOT_CUT` and
`CORNER_OVERCUT` are unaffected in every case.

**The geometric identity.** The weld fillet at the corner is rebuilt from the record: the
right-triangle fillet prism the recorded A15 weld solid is built from, with both legs equal
to the recorded size, anchored with its right angle at the derived corner and its legs along
the rib's contour-edge directions at that corner — the edges that run along the two neighbour
surfaces — and extruded along the qualifying weld's validated path. The rib's final evaluated
body must not intrude into that fillet: a proven penetration of the rib mesh into the rebuilt
fillet beyond the product tolerance (section 7.3) is the finding; face contact or separation
passes. For a right-angle chamfer of leg `c` and a fillet of leg `K` the identity reduces to
`c ≥ K`; a rounding with the same setback frees less of the fillet's triangle and can intrude
where the chamfer clears — the identity reads the final body, never the authored treatment.
The recorded `side` of the weld is named in the finding's basis; the v0.1 fillet placement
does not consume it — the sides are read from the corner's geometry, and that ceiling is
recorded (section 7.1).

**A distinguishable result.** The finding's rule token is `CORNER_WELD_CLEARANCE` and its
basis names the derived corner and the weld record, so it cannot be mistaken for the explicit
size requirement; the requirement's `required_mm` stays the recorded size (section 4.2), and
the finding reports the measured penetration depth, the contact tolerance and the recorded
weld leg.

## 5. Joint intent: declared connections

Joint intent is **model data**, recorded and edited only by explicit commands:

- `id` — typed UUIDv7;
- `scope` — the element UUIDs that form the joint (the sample: base plate, column, ribs);
- `connections` — the declared connections, each `{ a, b, kind, note? }` with element UUIDs
  `a` ≠ `b`; `kind` ∈ `weld | contact` in v0.1 (bolted kinds are refused in v0.1 and arrive
  with the fastener slices).

Intent answers only "what did the owner declare must be connected here". It never infers a
connection from contact, never infers contact from a weld, and never invents intent when it is
missing. Consequences:

- `PART_UNCONNECTED` applies to elements inside a declared scope whose declared connection has
  no connecting element (a weld for `weld`; for `contact`, physical contact within the product
  tolerance). Elements outside every declared scope are **not applicable** for this rule — an
  established absence, not an unknown. A model with no joint intent recorded has no applicable
  connectivity check at all, and the report says so in coverage.
- `CONTACT_WITHOUT_CONNECTION` applies where a declared `weld` connection's parts touch (within
  tolerance) but no weld exists between them at that contact. Contact alone never proves a weld
  is needed, so the rule never fires where no connection is declared.
- `WELD_WITHOUT_CONTACT` applies to a declared `weld` connection whose weld exists but whose
  local gap is outside a recorded permitted range (section 4.1) — or `unknown` where only a
  normative gap rule would decide (none supported, section 3.3).

### 5.1 Local attachment of welds

Finding a weld record between two element IDs does not locate it at the joint. A weld's local
attachment is **validated** against the final evaluated bodies: the weld solid must lie at the
declared contact of the two connected parts within the product tolerance. A weld with no path
is checked at its placed solid; a weld whose placed solid cannot be located at the declared
joint is **not** locally attached: the dependent checks (clearance, gap) are `unknown` with
reason `attachment_unvalidated`, while independent checks (the corner requirement, clash,
completeness) are unaffected. A path stored on the weld is a hint for the validation, not proof.

### 5.2 Recorded shape (slice 3, 2026-11-15)

Joint intent and gap requirements are stored as elements of kind `record`
(`/World/Records/REC_<uuid7>`), created only by `record-create` and removed only by
`delete-element`. An intent record carries its declared scope and its declared
connections — the pairs, one unordered pair at most once, stored per kind
(`weld | contact`). A gap requirement record names its judged declared pair with the
permitted range and the tolerance; the owner basis is fixed in v0.1 and carries no
separate field. Connection notes (section 5, `note?`) are not stored in v0.1. A model
may declare several intents: their scopes merge and their connections concatenate into
the check input. Declared ids are stored verbatim and are not required to exist at
declaration: an id that names no evaluated part reports `target_unresolved`
(section 4.3), and the record is never rewritten.

## 6. Rule context and provenance

A **rule context** names the standards a project's checks are judged against:

```
RuleContext = none | cited { document, edition, clause, purpose }
```

- `none` is the default: only geometric rules and recorded owner requirements apply.
- `cited` records a selection with its full citation. In v0.1 no cited context is consumed by
  any supported rule (section 3.3); recording one is admissible bookkeeping, and findings do
  not claim normative force from it.
- Selecting a context is an owner decision recorded with the requirements; the harness never
  defaults a national code on.

Every finding names its basis: the requirement id, the rule reference, or the geometric
identity. Every check also records its **applicability decision** — `applicable`,
`not_applicable(reason)`, or `unknown(reason)` — in the coverage of section 8. Non-applicability
must be established (a missing requirement, no declared scope, a joint type the rule does not
cover); it must never be used to hide missing inputs.

## 7. Geometric method and tolerances

### 7.1 Method

The candidate method (#634) was to decompose a prismatic part into convex pieces using extrusion
cap triangles and plane-cut half-spaces, then compare with the final closed mesh of the other
part. Slice 2 validated a simpler method that satisfies the same requirements, and the contract
below stays what any implementation must satisfy, not the algorithm:

**Validated in slice 2 (2026-11-15).** The implementation compares the two **final evaluated
closed meshes** directly (`kernel::contact`): the surface facts — strict crossing, coplanar
face contact, edge/point touch, separation, full containment — are decided by the kernel's
exact predicates and are properties of the surfaces, so they cannot depend on a triangulation
or on an artificial decomposition; there are no internal faces to offset. The measured
penetration depth samples the two meshes' vertices and per-face interior probes (exact
midpoint chains), is a deterministic lower bound of the true depth, and is invariant under
re-triangulations that keep the vertex set; a proven crossing whose overlap touches no sample
reports `unknown` (`indeterminate`), never a guess. The adversarial cases of #634 are
exercised in `kernel::contact::tests` and `steel::engine::check::tests`: face contact,
edge/point contact, separation, containment without a surface meeting, corner overlap,
rotated placements, and diagonal-flip re-triangulations. Remaining ceilings, recorded here:
depth sampling is vertex-based (a vertex-independent depth needs the intersection region),
and a point classification whose six axis rays all graze edges/vertices reports unresolved.

The requirements any implementation must satisfy:

- **Decomposition independence.** The result of `PART_CLASH`, contact and gap measurements must
  be invariant under a different valid triangulation of the same body. Artificial internal
  faces of a decomposition are not physical part boundaries: offsetting every convex piece
  inward may introduce gaps along those faces, so any tolerance use must not let the outcome
  depend on where those faces lie. Slice 2 proves this with alternate triangulations of the
  same body.
- **Contact is classified, not assumed.** Face contact, edge/point contact, proximity within
  tolerance and penetration are distinguished and reported as what they are. Clipped surface
  area inside an outward-offset volume is not automatically face-contact area.
- **Hard cases are enumerated, not absorbed**: full containment without surface intersection,
  cavities, holes, plane cuts, rotated placements, and approximation error in rounded or curved
  geometry. Geometry or method the implementation does not support reports `unknown` with the
  reason and the affected elements; bent/curved parts and holes near the joint require
  demonstrated support in slice 2, not an assumption.

**Validated in slice 3 (2026-11-15).** Local weld attachment and the local gap are measured
against the two final evaluated meshes with the kernel's exact point-to-triangle distances: a
weld is locally attached only when every authored path point lies within the product tolerance
of both surfaces — the near-threshold rule decides the band edges, and a pathless A06 weld has
no placed solid at the joint, so it is never locally attached. The local gap of an attached
weld is the smallest summed point-to-surface distance of the path points: the opening the weld
bridges at the declared contact. Both measurements are sample-based like the depth
measurement: a surface region no sample reaches can only make the true distance smaller, and
a permitted range beyond roughly twice the contact tolerance can never be satisfied by an
attached weld; both ceilings are recorded here.

**Validated in slice 4 (2026-11-16).** The corner-cut rules derive the requirement's corner
from the final evaluated geometry on every run: the candidate corners are the rib's authored
contour vertices — a treated contour keeps its sharp corner points, so the ideal corner
survives any parameterized treatment — whose position lies strictly within the contact
tolerance of both recorded neighbours. Exactly one candidate is judged; zero or several are
`unknown` (`target_unresolved`), and a contour vertex at a neighbour's contact band edge keeps
the candidate set from being established (`indeterminate`). The measured cut is the **least
leg**: the least, over the rib's evaluated mesh vertices that provably touch one neighbour,
of the distance to the other neighbour — zero where the rib surface still reaches the corner,
the setback (chamfer leg, contract section 3) where a treatment or cut removed the corner —
with the near-threshold rule and `δ = max(mesh bounds, t/2)` deciding the acceptance band
`[required − tolerance, upper edge]`, whose upper edge the recorded standalone `upper_bound`
replaces. Recorded ceilings of that reading: the leg is the least retreat of the rib surface
over the whole body, so a second approach to the neighbour pair elsewhere on the rib, without
a dual-touch contour vertex of its own, would pull the leg down — the standard two-corner
shapes (a rib flush with the neighbour face) report several candidates before the measurement
runs; and the derivation reads authored plate contours, so a rib that is not an authored plate
reports `target_unresolved`. The one unambiguous repair of `CORNER_NOT_CUT` (section 9) sets
the required chamfer at the authored vertex the derived corner maps back to, and is offered
only when that vertex exists, its treatment is absent or a chamfer, the adjacent vertices
carry no treatments, the corner really turns, and the required leg fits both adjacent edges.

**Added by #723 (2026-11-16).** The dependent clearance check (section 4.4) decides the rib's
intrusion into the rebuilt weld fillet with the kernel's contact facts: the rib's final mesh
against the rebuilt prism's sealed mesh — the same decision class as `PART_CLASH` — with the
near-threshold rule on the measured penetration. The fillet's leg directions are the rib's
authored contour-edge directions at the derived corner: the corner derivation (above) already
requires the corner to lie within the contact tolerance of both neighbours, so those edges run
along the neighbour surfaces. Recorded ceilings: the rebuilt prism follows the A15 frame rule
along the qualifying path and does not re-orient by the weld's recorded `side` (the sides are
read from the corner's geometry, section 4.4), and a path whose frames place the prism off the
joint is judged by where the prism actually lies, never by where a weld would ideally sit.

### 7.2 Two tolerance layers, kept separate

- **Numerical robustness** is the kernel's own: exact predicates for orientation and
  intersection decisions. It decides *geometric predicates* (intersecting, inside, touching the
  exact model) and is never tuned to make a physical answer come out right.
- **Mesh approximation bound** is a computed, per-body quantity: the maximum deviation of the
  evaluated mesh from the exact bounded surface (profile discretization and plane-cut
  triangulation contribute; flat prismatic faces contribute zero). It is recorded per checked
  body and reported with measurements, not replaced by a constant.

### 7.3 Physical tolerance and the near-threshold rule

The **product contact tolerance** `t` (default 0.5 mm, owner-adjustable per project) is a
product decision about what counts as touching. It is **not a normative dimension**: it makes
deterministic the distinction between contact, clearance and penetration for modelling purposes,
and it is reported with every result.

**Near-threshold rule.** A comparison of a measured quantity `m` against a bound `b` (a
requirement size, a permitted-gap end, the clash threshold) first forms the **acceptance
region** from that bound and its recorded tolerance: a recorded `tolerance_mm` widens the band
to `b ± tolerance_mm` (a required size accepts `|m − b| ≤ tolerance_mm`; a maximum accepts
`m ≤ b + tolerance_mm`; no recorded tolerance leaves the bare bound `b` as the only edge). The
uncertainty interval is `[m − δ, m + δ]`, where `δ = max(mesh_bound_of_measurement, t/2)` — the
mesh approximation and half the contact tolerance together. Then:

- pass/fail only when the interval lies **strictly inside** the acceptance region or **strictly
  outside** it — that is, `δ` is tested at the band edges of the acceptance region, and the
  recorded `tolerance_mm` therefore widens what is accepted, not just what is reported;
- `unknown` with reason `indeterminate` and the measured value reported, when the interval
  straddles a band edge — a near-threshold result never silently resolves to pass or fail, and
  its resolution must not depend on artificial decomposition boundaries.

The sample's tolerance on the 10 mm requirement is an explicit recorded `tolerance_mm`: the
acceptance region is `10 ± tolerance_mm`, and its upper edge is the overcut basis — the
20 mm-cut acceptance case reports `CORNER_OVERCUT` with measured 20 mm and required 10 mm,
strictly above the band and "subject to the recorded tolerance" in the literal sense of the
rule above. The upper edge here is the owner's recorded size, not a derived or normative
maximum (section 3.2); a recorded gap requirement's permitted ends are bounded the same way.

## 8. Findings and report semantics

### 8.1 Command and reply

`check-model --store ROOT [--element UUID] [--offset N] [--limit N]`, a read command. The reply
names the **checked revision** (the committed head at the moment the check starts; section 10)
and the scope (whole model, or the named element's subtree), then:

- `status` — `green | red | unknown` overall; red if a checked applicable rule fails, otherwise
  unknown if a requested applicable check cannot be completed, otherwise green;
- `counts` — per rule, over the **whole checked scope**, not just the returned page;
- `coverage` — every element of the scope in exactly one bucket: `checked`,
  `not_applicable(reason)`, `unknown(reason)`; plus the limitations paragraph (what the
  supported set does not cover — e.g. weld adequacy, unsupported geometry);
- `findings` — one page.

A **finding** carries: `rule`, `status` (`fail` or `unknown`), `elements` (IDs and names),
`message`, the basis (requirement id and/or rule reference, or the geometric identity),
`measured_mm` / `required_mm` where meaningful, the applicable tolerances and mesh bounds,
`missing_inputs`, and a nullable `fix` (section 9). Where several dimensions and bounds apply
(one measurement against several recorded bounds), they are a named array of
`{bound, tolerance, source}` entries, not a single collapsed number.

Pagination (`--offset`, `--limit`) affects only the `findings` page; status, counts and
coverage always cover the whole checked scope, so paging never hides a red or unknown overall
status.

### 8.2 Exit codes

`0` for green and yellow, `2` for red. A yellow (unknown-overall) exit 0 still means *not
checked*, not passed. Operational failures stay on the existing refusal path and are
distinguishable **by reply shape**: a refusal is an `outcome: "rejected"` JSON, while a red
check reply is a completed check reply — both may carry exit 2, because 2 is already the CLI's
generic refusal code. Scripted consumers branch on the reply, not on the code alone. Slices
2–5 must not repurpose other exit codes to work around this.

## 9. Optional fixes

A finding's `fix` is present only when the intended repair **and all its inputs** are
unambiguous; otherwise `fix` is `null` with an explanation and the missing inputs named.
Geometry alone never selects a material grade, a new weld size or a connection type — those
require recorded inputs the model must already contain.

- A supported repair names its command and arguments and states its effect. For
  `CORNER_NOT_CUT`, `set-plate-corner` is offered only when the appropriate authored vertex
  exists and changing its treatment produces the required final shape; contour-authored cuts
  may need contour reconstruction, and when a safe complete repair is not available the fix is
  `null`. Parameterized chamfers, contour-authored chamfers and plane cuts that produce
  equivalent final geometry receive equivalent results; a rounding with equal setbacks is **not**
  equivalent (different freed space at a right-angle corner) and must not receive the same fix.
- The workflow is: apply one fix, **recheck**, then select the next fix — commands from an
  earlier report may no longer apply. There is no promise that blindly executing every hint
  reaches green; a contact that does not require a weld never receives an unconditional
  `create-weld` fix, and a permissible weld gap passes its applicable gap rule.

## 10. Revision binding and batch behaviour

- `check-model` checks a **committed revision**: the head at the moment the check starts,
  named in the reply. No unpinned head is checked implicitly after completion; every status in
  the reply belongs to the named revision.
- **Drafts are not checked in v0.1.** Batched operations are drafted and have not published a
  revision; reading the saved head would check the wrong model. Draft checks report *not
  performed* with an explicit `unknown` reason (`draft_not_evaluated`). After a successful
  `batch-commit`, the resulting revision is checked (the slice 5 integration), not the pre-batch
  head.
- Reads inside a batch see the committed store (a #634 finding on the separate batch route): a
  `check-model` invoked inside an open batch reflects committed state, and the report states
  that the draft is not part of what was checked.

## 11. Summary in write replies (spec for slice 5)

A successful write operating on a model store gains a short `checks` summary: the checked
revision, overall status and counts. Non-negotiables:

- **Revision-specific.** The summary checks the resulting revision, not an unpinned head that
  could have moved since the command completed.
- **Budget.** The summary fits the existing `print_json` output budget (1 MiB text,
  65 536 tokens of scratch, `LIMIT_EXCEEDED` fallback in `apps/beamcraft-cli/src/report.rs`):
  counts only, never finding pages.
- **Never poisons the write.** A failure of the follow-up check must not turn an already
  successful write into a rejected write or erase its result: the summary reports the checks as
  unavailable with a reason, and the write's exit code is unchanged — the write happened.
- **Routes.** Ordinary CLI and session writes and the separate `batch-commit` route are all
  covered, with explicit handling of routes that return before `dispatch`; a wrapper around
  `dispatch` alone is insufficient.
- **Window opt-out.** The window uses the CLI for modelling edits and needs no summaries until
  it has a checks panel. The opt-out is the environment variable
  `BEAMCRAFT_CHECKS_SUMMARY=off`: set to exactly `off`, write replies omit the `checks`
  summary for that process; unset or any other value keeps summaries on.
- **Delivery note (slice 5).** The seam sits in the CLI dispatcher's tail, which both the
  one-shot process and every session command line answer through, plus an explicit attach in
  `batch-commit`. "Operating on a model store" is resolved to the registry routes whose write
  commands publish model revisions: the modelling, definition, construction and recorded-input
  routes, and the reference and numbering routes. Writes outside that set carry no summary,
  each for a stated reason: the store-level primitives (`put-blob`, `commit-revision`) manage
  blobs and revisions of arbitrary content; `export-view`, `screenshot-view`, `export`,
  `export-bom`, `schedule-export`, the changeset and IFC routes and the drawing documents
  render output files or their own documents; the named-view route stores display state and
  `add-library` writes the named library file; history, coordinator and coordination manage
  compaction, leases and descriptors; `create-project` provisions a fresh store and changes
  no modelled revision (it takes `--parent`/`--name`, no `--store`, so the summary seam does
  not reach it); and the routes that return before `dispatch` (`session`, the batch words,
  `commands`) run no model write in their own process. The summary shapes: a checked model
  reports `revision_id`, `status` (`green | red | unknown`), `counts` per rule and
  `findings_total`; a drafted write reports `status` `unknown` with `reason`
  `draft_not_evaluated` and names no revision; an unavailable check reports `status`
  `unavailable` with the reason — never silently green.

### 11.1 Whole-reply assertion audit and cost honesty

The CLI integration suites assert replies **by path** through the `at`/`text`/`list` accessors
of `apps/beamcraft-cli/tests/support/mod.rs`; there is no whole-reply equality assertion in the
support layer. Adding an optional `checks` object to write replies therefore does not
structurally break the existing suites; suites that assert help text, `COMMANDS.md` or exact
counts for a command do change and are regenerated with `BEAMCRAFT_BLESS=1` per the existing
procedure.

**Cost is unmeasured.** A full model check may be expensive; no claim about its cost (for
example, that it merely doubles command time) is established by this slice. Slice 2 measures
the check cost on the sample fixture and records the numbers here before slice 5 ships
summaries. The design constraints above (counts-only summary, checks on the resulting revision
only, opt-out) are what keep the worst case bounded; the measurement decides whether more is
needed.

**Slice 2 measurement (2026-11-15).** Sample fixture: eight 1000×500×10 plates, four stacked
in two layers so that twelve pairs penetrate (the #559 joint shape at small scale); unoptimized
debug build, Apple Silicon. Whole-command wall time of `check-model` on the committed head:
1.2–1.7 s over three runs, dominated by the strict reader's revision verification and layer
parse that every read command performs; the contact decisions themselves are in the tens of
milliseconds for this fixture (12 pairs, 12 triangles per body, box-pruned candidate pairs,
exact-predicate fallbacks far below the budget). No budget or pair cap was hit. The numbers are
a debug-build lower bound on useful work, not a release-build promise; slice 5 re-measures on
the sample fixture before shipping write-reply summaries.

**Slice 5 measurement (2026-11-16, before shipping the summaries).** Same fixture shape
(eight 1000×500×10 plates, material defined, two penetrating layers) in a debug build on
Apple Silicon, as `create-plate` wall time with and without the summary
(`BEAMCRAFT_CHECKS_SUMMARY=off`): 1.59 s with and 1.21 s without, on a store grown to nine
plates by the measured write itself. The delta — about 0.4 s — is the follow-up check of the
resulting revision and is dominated by the same strict reader and layer parse every read
command performs; the check decisions stay in the tens of milliseconds. The summary object
itself is counts only and adds under 0.7 KiB to the reply. The CLI integration suite (real
binary, nextest, debug) ran the full local gate with the summaries on; the recorded receipt
of the slice 5 PR carries the stage times, which is the honest suite-level measurement.

## 12. Sample fixture (acceptance seeds)

The acceptance fixtures follow #634's acceptance list; what slice 1 adds is the recorded shape:

- the sample fixture records the owner's 10 mm corner-cut requirement (target: rib, neighbours:
  base plate and column, `basis.source: "owner"`) and intended connections as joint intent;
  materials and weld inputs are explicitly supplied — a 10 mm cut does not imply a 10 mm weld;
- the initial column-through-plate model with missing cuts and required connections is **red**
  with the applicable `PART_CLASH`, connectivity and `CORNER_NOT_CUT` findings; a missing weld
  must not erase the independently recorded corner requirement;
- a 20 mm cut against the recorded 10 mm requirement produces `CORNER_OVERCUT`, measured 20 mm,
  required 10 mm, subject to the recorded tolerance — the recorded size is the upper bound
  (section 4.1): the fixture records no separate and no normative maximum;
- a corrected, fully specified fixture passes all applicable checks within the supported scope;
  a requested weld-adequacy check stays `unknown` (`no_supported_rule`);
- a weld elsewhere between the same parts does not trigger a corner violation; missing or
  unvalidated local attachment produces `unknown` (`attachment_unvalidated`) for dependent
  clearance checks only.

## 13. Open items for slices 2–5

1. Validate the geometric method (section 7.1) on the adversarial cases and only then fix it as
   the implementation contract.
2. Measure the check cost on the sample fixture; record the numbers in section 11.
3. Pin a normative permitted-gap rule (candidate: S5) or a weld-adequacy rule (candidates: S2,
   S6) — each with document, edition, clause, joint types, required inputs and retrieval — or
   leave them unknown; a context selection alone (section 6) never activates a rule.
4. The owner decision on automatic cut geometry (recalculation versus editing) stays open and
   is recorded in `docs/plan/decisions.md`; this contract assumes it is not granted.
