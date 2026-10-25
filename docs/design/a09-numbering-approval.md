# A09: numbering approval

Date: 2026-10-19. Status: **experimental vertical slice; A09 acceptance is limited as listed
below.** Branch `feat/a09-numbering-approval`, based on `origin/master` `e2d5df2`.

Plan row: A09, depends on A08 (numbering preview). The dependency on C05 (multi-user coordinator)
is lifted by the owner's decision of 2026-10-18, so this is the **single-user form**: approve the
numbering that A08 previews and store it in the model. Behaviour comes from the A08 rules, the
owner's local numbering notes (outside the repository) and the repository's own contracts; nothing
from any commercial product.

## What is implemented

```text
beamcraft approve-numbering --store ROOT --preview-token TOKEN
        [--plate-series PREFIX:START] [--beam-series PREFIX:START] [--column-series PREFIX:START]
        [--assembly-series PREFIX:START] [--tolerance MM] [--compare-name true|false]
        [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
beamcraft inspect-numbering  --store ROOT [--offset N] [--limit 1..256]
beamcraft preview-numbering ...   (A08; now also reports the stored approval and, per group, "stored")
```

| Piece | Where | What |
|---|---|---|
| Command and rules | `beamcraft_engine::numbering_approval` | `ApproveNumbering { settings, preview_token }`, `approve(parts, assemblies, command)`, `assess(parts, assemblies, stored)`, `compare_group`, `Basis`, `Mark`, `Approval::footprint`. Pure and bounded |
| Stored mark | `beamcraft_layer_engine::authored` | `Change::Mark { id, mark }`, `Mark { series, number, basis }`, the property names `MARK_SERIES`, `MARK_NUMBER`, `MARK_BASIS`, validation of loaded and authored marks, descriptor and pin changes |
| Adapter | `beamcraft_cli::numbering_approval` | `approve_numbering` (replay, facts, engine, author, footprint check, commit) and `numbering_status`; `beamcraft_cli::numbering` reads the marks with the facts, of one head |
| Commands | `numbering_cmd` | parsing and JSON only |

### Approval

`approve-numbering` takes the settings and the `preview_token` of a `preview-numbering` run (the
"pinned inputs"). It then, on the head it opens and verifies:

1. decides replay of the `op_id` from the host ledger (below);
2. reads the facts of the same head and lets the engine evaluate the preview again;
3. **refuses** a token that is no longer the proposal (`STALE_WORKSPACE`), an overlap of series and
   an empty model (`INVALID_ARGUMENT`) and a store without a revision (`NOT_FOUND`), each with
   nothing written;
4. writes the mark of every part and every assembly in **one** revision through the normal path
   (U02 authoring, footprint check, U06/U07 commit with the strict verifier). The revision is an
   ordinary undo unit: `undo-workspace` restores the earlier marks byte for byte, `redo-workspace`
   brings them back (tested).

Equivalent parts get the same mark because they are in one A08 group; non-equivalent parts get
different marks because they are in different groups; a position is taken by one group only because
an overlap is refused. Re-approving an unchanged model gives the same marks, and a copy of an
existing part joins its source's group and renumbers nothing (A08 rules).

Overlapping series are **refused**, not reported-and-partially-applied: A08 leaves the members of an
overlapping group unnumbered, and an approval that silently leaves parts unmarked is not an
approval. The user changes a series (`--beam-series B:1`), previews again and approves.

Approving what is already stored (every mark and every approval record equal to what would be
written) writes nothing and answers `numbering_approved_unchanged`. A retry of the same `op_id` with
the same settings and token answers `numbering_approved_replayed` (the revision it created, even
after later edits); the same `op_id` with other content is `OP_ID_REUSE`. The replay decision reads
the marks of the revision the ledger recorded and of the one before it, so an unrelated later
revision under another operation is not mistaken for the approval.

### Stored schema (the only layer-engine schema change of the wave)

Three optional properties on a plate, beam, column or assembly prim, in the authored layer:

| Property | USD type | Meaning |
|---|---|---|
| `steel:numbering:series` | `string` | series prefix, at most 16 of `A-Z a-z 0-9 _ -`, may be empty |
| `steel:numbering:number` | `int` | number in the series, `0..=2147483647` |
| `steel:numbering:basis` | `string` | the approval record, printable ASCII, at most 512 bytes |

All three or none; never on a weld (`connection`), a `level` or a `reference` wrapper. A partial or
mistyped record, a number outside the range, a prefix outside the character set and an oversize
record are refused when authored and when loaded (the structural validator, `inspect-elements`,
`inspect-numbering`, `preview-numbering` and `approve-numbering` all refuse them; never read as
"unnumbered"). The record is the **approval basis**: the settings (the four series, the tolerance
as exact bits, whether names are compared) and the preview token, e.g.
`BCA1 plate=PL:1 beam=B:1 column=C:1 assembly=A:1 tol=3f50624dd2f1a9fc name=0 token=<sha256>`; it is
stored with every mark so the model alone says what was approved.

Descriptor: one new line (`numbering=...`) and `Mark` in the `changes=` list. The schema digest
changes; every earlier pin stays accepted: the pre-A09 mapping is derived from the current
descriptor by removing exactly those two additions (`pre_a09_descriptor()`), the older pins derive
from that, and the pinned digest of master `e2d5df2`
(`fdaf8904...3865b`) is a test. A store pinned to it stays editable and is repinned on its next
edit (test). `Change::Mark` uses the **label** footprint group (reads lifecycle and label, writes
label): it conflicts with a rename of the element and another approval, and with nothing derived.
Nothing derived depends on a mark: an approval changes `authored.usda` only (tested; the derived
layer digest and every mesh digest are unchanged). Copying an element builds it from its authored
inputs, so a copy has **no** mark until the next approval.

**Merge-conflict risk (for the owner):** `crates/layer-engine/src/authored.rs` (the descriptor,
`pin_accepted` and its tests), `authored/edit.rs` and `authored/validate.rs` change here, and
`Change` gains a variant. Any open branch that changes the descriptor (a new element kind or
property) conflicts textually in `SCHEMA_DESCRIPTOR`, in `pin_accepted` and in the pin tests, and
semantically needs its own "pre" descriptor derived the same way. Merge this before or rebase after;
do not both change the digest without ordering.

### Invalidation and the status query

`inspect-numbering` (and the `approval` object of `preview-numbering`) answers `not_approved`,
`current` or `stale`. `assess` recomputes the preview **with the settings stored in the basis** and
compares its token with the stored one:

* **stale** after a change of profile, material, dimensions, holes or plate outline (everything A08
  compares), a new, copied or deleted part, or a regrouped assembly, because the numbering the model
  would get now differs from the approved one (A08 decision 3: stale means "the proposal would
  differ"). The report names the first 32 elements that differ and counts the unmarked ones.
* **not stale** after a pure move (position is no fact of the preview, so a moved identical part
  stays in its group), a rename (unless names were compared when approving), a camera, view or
  selection change (none of them is in the model or in the preview), a read, or a change of
  definitions nothing uses.
* stale too when the stored marks come from different approvals, an approval record cannot be read
  or a stored mark differs from the approved numbering (a record edited outside BeamCraft). A damaged
  *structure* is an error, not a stale report.

`preview-numbering` adds, per group, `stored`: `none`, `same`, `different` or `mixed`, comparing
the marks stored on its members with the proposed position. The proposal itself is still a
function of the revision's facts and never of stored numbers (A08 decision 2 stands).

## Interface for read-only consumers (IFC export E02, BOM E03)

Not changed in this task. A consumer reads the three properties of the element prim (through the
layer engine reader's `Prim.properties`, or any OpenUSD reader: `pxr` reads them, tested) and
composes the mark as `series` followed by `number`, with a `-` between them for an assembly;
`beamcraft_engine::numbering_approval::Mark::text(kind)` is the one definition. A consumer that must
not print an out-of-date mark reads `numbering_status` (`assess`) first and treats anything but
`current` as "numbering not approved for the current model". The IFC mapping proposed by E01 for a
position number (a property set entry) is E02's decision.

## Decisions (for the owner)

1. **Refuse on overlap** instead of approving the numbered part and leaving the rest unmarked.
2. **Stale-token error is `STALE_WORKSPACE`**; no F04 code names "preview out of date" more closely.
3. **The mark lives on the element** (three properties) and the approval basis is repeated in each
   mark, instead of a record in the root layer or a new element kind. It keeps the change inside the
   existing authored-layer pattern, makes the model self-describing, and costs about 250 bytes per
   element.
4. **Stale means "the proposal would differ"**, consistent with A08, so a deleted part also makes
   the approval stale although no remaining mark is wrong. Re-approving then gives the same marks
   unless a group's reference part (its oldest member) left.
5. **Re-approval recomputes from the A08 rules.** Numbers are stable while nothing relevant
   changes and after a copy; they can shift after an edit that splits a group or removes a group's
   reference part. Keeping or re-using old numbers is a modified-part option and is not done.

## Requirement coverage

Tests (all through real code, the CLI tests through the binary):

* Engine, 11 unit tests in `numbering_approval`: shared and different marks, assembly mark text,
  stale token, overlap, empty model, invalid settings, re-approval and a copy, the approval record
  round trip and its rejects, stale on every fabrication fact and not on a rename, damaged records,
  group comparison, footprint and its bound.
* Layer engine, 3 integration tests and 1 pin test: marks authored and replaced as a whole, label
  group only, rejected values, wrong kinds, missing and duplicate targets, loaded records tampered
  (wrong type, range, characters, partial), the pre-A09 pin stays editable and is repinned; every
  earlier pin is still accepted.
* CLI end to end, 7 tests in `apps/beamcraft-cli/tests/numbering_approval.rs`: approval stores the
  marks (10 marks, 7 groups, authored layer only, derived layer and mesh digests unchanged, the
  properties in the authored layer text, `inspect-elements` unchanged), read back through
  `inspect-numbering` and `preview-numbering` and through the OpenUSD reference reader `pxr` when
  installed locally (the test prints SKIP otherwise; CI has no `pxr`); stale token, overlap, empty
  model and a store without a revision refused with the layers unchanged; approving twice
  (`unchanged`), same `op_id` replay and reuse; move, rename and reads keep it `current` while
  thickness, material, copy and delete make it `stale`; undo and redo of an approval in a
  workspace; bad and oversize arguments, and four kinds of damaged stored marks refused by every
  reader.

## Not done

* Renumber, clear and delete-marks commands; modified-part options ("keep", "take new", "re-use old
  numbers"); the history log of numbering; assignment of a mark to a part by hand. Re-approval
  after a model change is the only way to change marks.
* Numbering of welds, bolts and standard parts; family and control numbers; design groups.
* Deactivated elements are not skipped (as in A08; lifecycle is read by the E02 adapter).
* No UI, no viewport selection sync, no multi-user coordinator (C05).
* `inspect-elements` does not list the mark (its source is outside this task's paths); the marks are
  read with `inspect-numbering`.
* One approval writes at most about 1,360 marks: the declared footprint of F04 section 8 is bounded
  at 4096 entries and one numbered element costs three. A larger model is refused with
  `LIMIT_EXCEEDED` and nothing is written. Raising it needs a footprint constructor for a whole-model
  label write (`beamcraft_engine::footprint`, outside this task's paths).
* An approval records the head it was computed from but not who approved it; the actor is the
  revision's `createdBy`.

## Contract amendments proposed (not edited, F04 is frozen 0.1.0)

* `docs/contracts/f08-module-boundaries-v0.1.md` row "Numbering preview": add the `approve-numbering`
  and `inspect-numbering` commands, `beamcraft_engine::numbering_approval` and
  `beamcraft_cli::numbering_approval`, and replace "no stored numbers" by "approved marks stored as
  `steel:numbering:*` properties (A09, experimental)".
* `docs/contracts/f04-model-and-commands-v0.1.md` section 9 names approved-number invalidation as open:
  this note is the A09 resolution (stale = the proposal would differ; move, rename without name
  comparison and view changes never invalidate); section 4's change list gains `Mark`.
