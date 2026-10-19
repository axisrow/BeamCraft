# A08: numbering preview

Date: 2026-10-19. Status: **experimental vertical slice; A08 acceptance is limited as listed
below.** Branch `feat/a08-numbering-preview`, based on `origin/master` `11a2139`.

Plan row: A08, depends on A01 (assemblies), P04 (quantities and update) and P05 (copy). Result:
a read-only preview that groups equivalent parts, proposes number series and shows overlaps.
**Nothing is approved and nothing is written to the model**; approving numbers is A09.

## What is implemented

```text
beamcraft numbering-preview --store ROOT [--plate-series PREFIX:START] [--beam-series PREFIX:START]
        [--column-series PREFIX:START] [--assembly-series PREFIX:START] [--tolerance MM]
        [--compare-name true|false] [--previous TOKEN] [--offset N] [--limit 1..256]
```

| Piece | Where | What |
|---|---|---|
| Rules | `beamcraft_engine::numbering` | `preview(parts, assemblies, settings)`: pure, bounded, deterministic. Part facts and assembly facts in, groups, positions, statuses and a token out |
| Adapter | `beamcraft_cli::numbering` | reads plates, members and assemblies of the verified head (`plate::inspect`, `assembly::assemblies`) into facts; the two readers are checked to describe the same revision (read again up to three times if the head moved) |
| Command | `numbering-preview` | parsing and JSON only; groups are paged (`--offset`, `--limit`, default 64, at most 256; at most 32 members listed per group) |

Rules (own wording, from the owner's local numbering notes `24-numbering.md` and
`08-numbering-assemblies.md`, which stay outside the repository):

* **Equivalent parts** have the same kind, the same pinned profile digest, the same pinned
  material digest, lengths within the tolerance (plate: width, length, thickness; member: axis
  length), a roll within 1e-9 rad (an angle is never compared with the length tolerance) and the
  same holes, compared in the part's own frame. A hole is its ring, kept with its edges:
  counter-clockwise, started at its smallest vertex, so the same ring started elsewhere or wound
  the other way is the same hole while two rings through the same vertices in another order are
  different holes; the rings of a plate are then sorted and compared point by point within the
  tolerance. Placement, level and
  assembly membership never matter. The name is compared only with `--compare-name true`.
* **The oldest part is the reference** (the smallest ElementId, a UUIDv7, so the earliest). A part
  joins the first earlier group of its kind whose reference it matches; groups are numbered in the
  order of their references, so a copy of an existing part joins its group and renumbers nothing.
* **Series.** Each part kind has a series, prefix and start number (defaults `PL:1`, `B:1`,
  `C:1`); a position is the prefix followed by `start + index`. Assemblies are grouped by the
  multiset of the groups of every part they hold, nested assemblies included, and marked
  `PREFIX-NUMBER` (default `A:1`). Prefixes are letters, digits and `_` (an assembly prefix may
  also hold `-`), at most 16 characters; a start number is at most 1,000,000,000.
* **Overlaps.** If two groups would get the same position, the first in series order (plates,
  beams, columns, assemblies; inside a series by reference) is `new`; the other is `overlapping`,
  names the position that took it in `overlaps_with`, and its members are counted as unnumbered.
* **Staleness.** `preview_token` is a SHA-256 over the settings and the groups with their
  positions, statuses and members, i.e. over exactly what the proposal says. `--previous TOKEN`
  returns `stale: true` when the present proposal differs. A thickness, material, profile, length
  or hole edit, a copy and a delete change it; a move does not, and a rename does not unless the
  name is compared and the rename splits or joins a group.
* The tolerance defaults to 0.001 mm (the F04 length floor), in the engine and in the command:
  sub-micron differences between axis lengths computed from different coordinates are
  floating-point noise, not different parts.
* **Work bound.** Candidates for a part are looked up by kind, profile, material, the name when
  compared, the counts of dimensions, angles and holes, and the range of its first dimension, so
  unrelated groups are never compared. A model that puts tens of thousands of different parts
  into one such bucket (equal first dimension) exceeds 50,000,000 comparisons and is refused with
  `INVALID_ARGUMENT` instead of running without bound. Assembly signatures are looked up by their
  SHA-256, not scanned.

## Decisions (for the owner)

1. **The kind is part of a part's identity.** A beam and a column of the same profile and length
   get different series and numbers. The notes make part series a property of the part type and
   list orientation as an optional comparison; this is the simple reading.
2. **A preview is a function of the revision, not of stored numbers.** No position is persisted
   (that is A09), so every group is `new` or `overlapping`; the statuses "keeps number" and
   "changes number", the "compare to old / keep / take new" options, "renumber all", "re-use old
   numbers" and the manual "change number" commands need stored numbers and are not here.
3. **Staleness is "the proposal would differ"**, not "the model changed". A move or an unrelated
   rename leaves it current.
4. **The preview reads two views of one head** (plates and assemblies come from two existing
   readers). It re-reads when the head moved between them and fails with
   `DEPENDENCY_UNAVAILABLE` after three attempts. Folding both into one reader would touch code
   that other open work edits.

## Requirement coverage

Tests: 11 unit tests in `beamcraft_engine::numbering` and 10 end-to-end tests through the real
binary (`apps/beamcraft-cli/tests/numbering.rs`): identical parts share a number and the oldest is
the reference; a different material, a hole and a different thickness or length split groups; a
copy joins its source's group and renumbers nothing; overlapping series leave the later group
unnumbered; the tolerance and the name setting; assembly marks; paging; every bad option is
`INVALID_ARGUMENT`; an empty model; the store (head and layer digests) is unchanged after previews;
a thickness edit, a material edit, a copy and a delete make a preview stale while a move and a
rename do not.

## Not done

* Approval, stored numbers, modified-part options, change/clear/renumber commands, history log
  (A09 and later). Weld numbering, standard-part model, family and control numbers, design groups.
* Finish, surface treatment, welds, assembly phase and UDA comparisons (the model has none of them).
* Deactivated elements are not skipped (lifecycle is read by the E02 adapter, not by this one).
* No viewport selection sync or preview table UI.
* The tolerance on holes is applied after the rings are put in canonical form and sorted, so two
  rings whose smallest vertices differ by less than the tolerance in different leading digits
  could be started at different vertices and not match.
* The "main part" of an assembly and its mark when sub-assemblies differ are not modelled.

## Proposed Roadmap entry

`2026-10-19 A08: numbering preview (beamcraft_engine::numbering, beamcraft_cli::numbering, CLI numbering-preview): groups equivalent plates, beams and columns by kind, profile, material, dimensions within a tolerance and holes, proposes series per kind and assembly marks, shows overlapping series and a staleness token; read-only, nothing is approved or stored (A09); experimental slice.`
