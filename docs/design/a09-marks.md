# A09 approved element marks

Date: 2026-10-20. Status: **experimental vertical slice; A09 acceptance is limited as listed
below.**
Branch `feat/a09-marks`, based on the verified current `origin/master`.

Issue row: A09 (the second half of issue #7's A08/A09) / L. Result: stable, human-approved marks
on plates, members, assemblies and welds, so outputs (BOM, drawings, NC later) can reference
them. Acceptance: API/CLI assign, approve and list end to end; an approved mark is immutable and
a shape edit withdraws its approval; mark storage reads back from ordinary USD.

The A08 half of the issue is the merged read-only numbering preview (`numbering-preview`,
`docs/design/a08-numbering-preview.md`): it proposes series and stores nothing — "approving
numbers is A09". This note is that A09 half: the marks a person assigns and approves. A later
step may feed the preview's proposal to `number-assign`; the storage and the state machine below
do not depend on it.

## What is implemented

```text
beamcraft number-assign --store ROOT --id UUID (--mark TEXT | --revoke true) [--op-id UUID ...]
beamcraft number-approve --store ROOT --id UUID [--op-id UUID ...]
beamcraft marks --store ROOT
```

| Area | What exists |
|---|---|
| `beamcraft_model::command` | `AssignMark { target_id, mark: Option<String> }` (`Some` assigns or replaces the value, `None` revokes the approval), `ApproveMark { target_id }`, `MarkStatus` (`assigned`, `approved`), each with a versioned content fingerprint. A mark is 1 to 64 UTF-8 bytes without control characters. |
| `beamcraft_engine::marks` | `Numbering` (the mark, status and markable kind of every model element) decides assign, revoke, approve and the withdrawal of an approval under a shape-input edit; returns the footprint. |
| `beamcraft_engine::footprint` | `Group::Mark` (a new semantic group beside the F04 section 8 groups: the numbering approval of an element) and `Footprint::for_mark`. |
| `beamcraft_layer_engine::authored` | `bim:mark=string` and `bim:markStatus=token(assigned\|approved)` on the element prim (present iff a mark exists), `Change::Mark`, structural rule `A08-V01` (value bounds, status pairing, kinds, uniqueness across the model). Layers pinned to the A07.2 descriptor stay accepted. |
| `beamcraft_cli::marks` | `assign` (with revoke), `approve`, `marks`: replay by `op_id` decided from the operation's own revision, `OP_ID_REUSE`, `NOT_FOUND`; the layer engine's observed writes are checked against the footprint; commit through U06/U07 with the strict verifier. |
| `update-element` | A thickness change (today the only shape-input edit) of an element whose mark is `approved` commits the withdrawal in the same batch: the mark value is kept, the status falls back to `assigned`. |

## Decisions

1. **A mark is a human-authored string, not a generated series.** The value is assigned by a
   person and only a person approves it; the engine never numbers anything by itself and never
   suggests a value. This keeps the slice free of numbering-series semantics (prefixes, direction
   of increase, per-series counters), which a later task can add over the same storage without a
   schema change. Direction conventions of a series are therefore not fixed here.
2. **One mark per element, one namespace per model.** Any two marked elements of a model must
   carry different values, whatever their kind, so a mark identifies an element in every output
   without qualification. This is stricter than domain practice (part and assembly marks usually
   live in separate series) and is the cheaper rule to validate; a per-kind namespace would need a
   scope field that no current output consumes.
3. **Markable kinds** are `plate`, `beam`, `column`, `assembly` and `connection` (a weld): the
   fabricated or fabricated-assembly objects a BOM can list. A level is not fabricated and a
   reference is foreign, so neither carries a mark. Bolts do not exist as elements yet; the kind
   list grows with the vocabulary.
4. **"Approved" means frozen value, not frozen element.** An approval vouches for the mark
   *value* of that element: with status `approved` a different value can no longer be assigned
   over (`DEPENDENCY_CONFLICT`, "revoke first"), while the identical value is the
   `INVALID_ARGUMENT` no-change, and the approval is withdrawn explicitly
   (`number-assign --revoke`; on a non-approved element that is `INVALID_ARGUMENT`). Everything
   else about the element stays editable exactly as before; deleting a marked element removes
   its mark with it.
5. **A shape-input edit re-approves (issue wording: "fail or re-approve").** An edit that changes
   what the mark identifies — today `thickness_mm`, the only shape-input edit; later contour,
   axis, roll, profile — commits `markStatus=assigned` in the same batch and keeps the value, so
   outputs keep referencing the element while the approval waits for a human again. The value is
   never dropped silently and the edit is never refused: the mark says who the part is, not that
   its geometry was already fabricated. Label, material, placement and level edits do not touch
   the approval.
6. **The record lives with its source.** `bim:mark` and `bim:markStatus` are two optional
   attributes of the element prim in the owned authored layer, like the level elevation. No new
   element, no relationship, no second namespace; an ordinary USD reader sees both values.
7. **A copy does not inherit the mark.** A copy is a different part with a fresh identity; two
   elements with one mark would break the single namespace, and marks are human decisions, so the
   copy starts unmarked (P05 already copies only the authored inputs the mark is not part of).
8. **Footprint.** `Group::Mark` is its own semantic group: assigning, approving, revoking or
   withdrawing writes only the target's mark group and reads its lifecycle and mark group, so a
   mark edit conflicts with another command on the same element and with nothing else. The
   withdrawal of a shape edit extends `UpdateElement`'s footprint by exactly that entry.
9. **Replay is decided from the operation's revision** (as in A06 and D2). The host ledger
   stores no request bytes (the stored canonical request of F05 replaces this), so the command
   kind is recognised from the before-and-after signature the recorded revision left on the
   target's mark: an assign leaves `(value, assigned)` over an absent or different assigned
   mark, a revoke moves `(value, approved)` back to `(value, assigned)`, an approval moves
   `assigned` to `approved` — whatever the head holds now. One collision is accepted and
   documented: the thickness withdrawal of `update-element` leaves the same approved-to-assigned
   pair a revoke leaves, so a revoke retried under the withdrawal's `op_id` answers replayed
   (a real revoke at that point is `INVALID_ARGUMENT`); only the stored canonical request of F05
   tells the two apart.
10. **Damaged records fail closed.** A stored layer whose `bim:markStatus` disagrees with
    `bim:mark`, whose value exceeds the bounds or that repeats a value is refused by the
    structural validator before any command reads or replays over it (rule `A08-V01`).

## Requirement coverage

BC-ASM-002 ("assembly marks/numbering", listed as uncovered by A01) is covered **partially**: an
assembly carries a mark a human approves, exactly like a part. Main-part semantics, sub-assembly
types and welded/bolted assembly rules stay uncovered. Bolts have no mark yet (no element kind).
BOM, drawing and NC outputs do not consume marks yet (E03, D01–D03, DSTV are later); `marks`
reports what they will reference.

Provenance: the storage pattern follows the repository's D1/D2 record-with-source decision and
F04 section 8 footprints; the approval state machine is this issue's own definition. No external
product material was used.

## Not done

- No generated numbering series, prefixes or direction conventions; no renumber command.
- No mark on bolts (no element kind), no main part, no per-kind mark namespaces.
- No output consumes marks yet (BOM, drawings, NC).
- Undo/redo of a mark change is the P07 layer-level undo, not a mark-specific operation.
- `update-element` cannot edit contour, axis, roll or profile yet; when those patches arrive,
  each must withdraw an approved mark like the thickness change does (the engine decision already
  exists; each call site must extend its footprint and author the status rewrite).

## Proposed Roadmap entry

`2026-10-20 A09: approved element marks (number-assign, number-approve, marks; bim:mark and
bim:markStatus on the element, Group::Mark, the engine's Numbering decisions and the withdrawal
of an approval under a shape-input edit) on top of the merged A08 numbering preview;
experimental slice.`
