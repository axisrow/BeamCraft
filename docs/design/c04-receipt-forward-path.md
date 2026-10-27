# C04 follow-up: resolution receipts versus undo and redo

Date: 2026-10-27. Status: implemented; fork issue axisrow/BeamCraft#176 (the fork tracking twin
of the C04 slice under #12 / #160). Baseline inspected: fork `master` `36ca2416` (C04.2 in
PR #177). This note records how the two typed-refusal windows between `resolve-workspace`
receipts and the P07 undo/redo machinery got a typed forward path. It changes no acceptance
specification and no implementation-plan status.

## Window 1: undo/redo across a resolution revision

The undo stacks classified a resolution revision as an ordinary edit, so `undo-workspace` could
restore the renounced layer state while the discard receipt kept excluding those operations; the
next rebase then refused with an integrity error (`source delta exceeds its verified writes`).

A resolution revision is now an **undo boundary**, derived the same way as `rebase_sequence` in
`history::timeline`: `resolve-workspace` appends a `resolved <revision>` marker to `undo.log`
before its commit (an entry without its revision is ignored, so the crash window keeps the
P07 intent semantics), and the derivation clears both stacks at the marked revision. Neither
direction crosses the boundary: undo cannot restore renounced layers, and a pending redo cannot
resurrect them. Edits after the boundary stay undo/redo units. Detection reads only `undo.log`,
never evidence blobs per revision. A receipt committed before this change has no marker and
keeps the old (refused-later) behaviour.

## Window 2: a renounced range that contains undo noise

P07 committed undo and redo revisions without operation evidence, so a renounced range that
contained one left the interval incomplete: the resolve succeeded while the later rebase refused
`DEPENDENCY_UNAVAILABLE` — a fix that was not one.

Both halves of the fix are in:

- **Undo and redo revisions carry real evidence.** A step inherits the recorded footprint of the
  unit it moves (undo: the revision at `target + 1`; redo: the revision at `target`), pinned as
  the step's own `CapturedFootprint` with a fingerprint that binds direction, restored sequence
  and source sequence. The inherited footprints participate in rebase planning like any command's.
  A source without usable evidence (legacy history) keeps the step evidence-less; the interval
  then stays incomplete and later planning refuses it conservatively.
- **The resolve refuses an incomplete renounced range up front.** When the range from the
  discarded operation through the head contains a revision without command evidence, the resolve
  is `INVALID_ARGUMENT` before anything is written, instead of recording a receipt that cannot
  keep its promise.

## Evidence

`apps/beamcraft-cli/tests/resolve.rs`: `a_resolution_revision_is_an_undo_boundary` (both
directions refused at the boundary, later edits undoable and stopping at it, a pending redo
cannot resurrect renounced work), `a_renounced_range_across_undo_and_redo_stays_verifiable`
(edit + undo + redo renounced; typed conflict before, rebase proceeds after, merged draft holds
the accepted state), and the evidence-less-range refusal with a synthetic legacy revision in
`mid_interval_renunciation_works_and_unsuitable_targets_are_refused`.
`apps/beamcraft-cli/tests/rebase.rs`: `a_plain_undo_carries_evidence_rebase_proceeds_and_a_stranger_is_refused`
replaces the old `DEPENDENCY_UNAVAILABLE` expectation with the merged-content assertions.
