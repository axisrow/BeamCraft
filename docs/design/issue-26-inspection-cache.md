# Issue 26: the cost of one inspection

Date: 2026-10-21. Status: **implemented; performance only, no reply changes.** Developed in the
fork on `bug/26-inspection-cache`, built and measured on the fork's `master` `538cbed`; the
figures were not taken again after later merges. Ported to `upstream/master` (at `236ce9b`) on
2026-10-23. Remainder of
[issue 26](https://github.com/axisrow/BeamCraft/issues/26) of the fork, finding 2: the work
inside the one inspection that every read command runs.

## Problem

Every read command (`inspect-elements`, `definitions`, `export-view`, `pick`, `export-bom`,
`export-ifc`, the numbering commands) runs one `beamcraft_cli::plate::inspect`, and every write
command evaluates the head and then the candidate again in its commit verifier. An inspection:

1. reads and hashes every layer blob twice (`RevisionStore::open`, then `Revision::layer_bytes`);
2. parses and composes the whole stage twice (`validate_layers`, then `Reader::open`; both call
   the backend's `open`);
3. re-evaluates every plate and member body with the kernel, copies and repeated parts
   included, although a body depends on its shape inputs alone;
4. for `definitions`, scans every plate for every listed definition to build `used_by`, once
   per `fit_page` attempt.

Where one `inspect-elements` spends its time on `master` (milliseconds, median of five runs; phases
from a temporary probe in the experiment binary with every change switched off, see below; the
rest is process start, output and exit):

| Phase | F1 copies | F2 distinct plates | F3 frame | F4 distinct members |
|---|---:|---:|---:|---:|
| Read and hash every layer blob (twice) | 18.7 | 18.3 | 74.3 | 43.3 |
| `validate_layers` (first parse) | 336.1 | 238.5 | 513.1 | 389.9 |
| `Reader::open` (second parse) | 226.8 | 204.6 | 411.7 | 296.8 |
| Index and decode | 20.3 | 19.6 | 53.6 | 47.6 |
| Kernel evaluation | 63.3 | 65.4 | 12,144.1 | 2,687.7 |
| Mesh comparison, snapshot, JSON | 11.3 | 11.6 | 18.6 | 20.9 |
| Rest | 16.6 | 30.1 | 28.9 | 56.1 |
| **Wall** | **693** | **588** | **13,245** | **3,543** |

Simple models spend most of an inspection parsing the stage twice; models with holes, cuts or
detailed sections spend it in the kernel, mostly on bodies that copies share.

## Decision

**Body cache (kept as decided by the owner: in-process only).**
`beamcraft_engine::query::committed::BodyCache` keeps kernel bodies by the exact bits of their
complete shape inputs:

| Body | Key (every field named, so a new field must be classified before it compiles) | Not in the key |
|---|---|---|
| Plate | contour points, thickness, hole polygons, plane cut | id, name, placement, material |
| Member | axis points, roll, plane cut, resolved `ProfileInfo` (rectangle width and height, or section family and parameter values, or custom outline and cavities; declared area) | id, kind, name, placement, material, the pinned profile digest |

* The key is an injective byte encoding: lists are length-prefixed, options and variants are
  tagged and every `f64` is its raw bits. `-0.0` and `0.0` are different keys. The P04 shape
  signatures (`beamcraft_engine::quantity`) are not reused as keys: they fold signed zeros
  (`canonical_f64_bytes`), and a contour built from `-0.0` can give other mesh bytes. No digest,
  stored or computed, stands in for an input; the profile enters by its resolved content.
* What is excluded is checked, not assumed: unit tests evaluate bodies that differ only in the
  excluded fields and require them to be equal before they may share an entry.
* Algorithm version: the cache never outlives its process, so an entry's algorithm is the
  running build's. Invalidation is by content: a changed input is a different key.
* Bounds: 32 MiB of keys and bodies (`BODY_CACHE_BYTES`, a caller may only lower it) and
  `MAX_ELEMENTS` entries; an insert that would exceed either clears the cache first, and a body
  larger than the budget is returned without being kept. A failed evaluation is never kept.
* `evaluate_committed` and `evaluate_committed_members` keep their signatures and share bodies
  within one call; `evaluate_committed_cached` and `evaluate_committed_members_cached` take a
  caller-owned cache. The per-element checks run in the same order with the same refusals.
* The CLI keeps one cache per thread behind `plate::evaluate_decoded`, so a command's read of
  the head, its commit verifier and every copy of one model share bodies.

**Reverse usage index.** `definitions` builds the material-to-plates map once from the plates in
element-id order; `used_by` is a lookup. Members are still not listed, as before.

**One parse (kept by measurement).** `Reader::open_validated` runs the `validate_layers` checks
on the very candidate it then installs, so `inspect_revision` and the commit verifier parse and
compose the stage once. The checks, their order, codes and messages (including the unwind
message) are those of `validate_layers` followed by `Reader::open`, and nothing is installed
unless every rule passed; a layer-engine test compares the two on a valid stage and on broken
definition, mesh, schema-marker and parse inputs.

**One verified read (measured and dropped).** A host method that returned the bytes
`RevisionStore::open` had already verified saved the second read and hash of every layer blob,
but the experiment below found no measurable effect, so it is not part of this change.

## Trust: what still runs on received bytes

* Every layer blob is read and its SHA-256 and size checked against the manifest.
* The structural rules (`u02-structural-v1`), the U03 index check and the decode with every
  integrity check run on every inspection; definition digests are recomputed from content.
* `verify_derived` compares the stored mesh of the received derived layer with the body,
  cached or not, and assesses the stored quantities. The cache replaces kernel calls only.
* `tests/inspection_cache.rs` warms the cache with a shape, commits a revision whose stored mesh
  for a copy of that shape is a different but structurally valid box, and requires
  `INTEGRITY_ERROR` "derived mesh differs from the kernel result" with the body served from the
  cache, from a cold thread and from the CLI.

## Experiment: every variant measured

Whether the two duplicate passes were worth removing was decided by measurement, not by
reasoning about them (owner decision). Three changes were built: the body cache (C), one parse
(P, `Reader::open_validated`) and one verified read of each layer blob (R, a host method that
returns the bytes `RevisionStore::open` had verified). One release binary carried temporary
switches (never committed) that turned each off, and all eight combinations were measured
together with the `master` binary on the fixtures F1 to F4 (see Reproduction). The combination
with every switch off matched `master` within 0.96 to 1.02 of its median, which validates the
switches. Every output was byte-identical across all variants and runs.

Machine: Linux 6.18 container, 4 vCPU Intel Xeon at 2.80 GHz, 16 GiB, rustc 1.97.0, the
workspace release profile of `538cbed`: thin LTO, one codegen unit. Those are today's `dist`
settings; today's `--release` leaves dependencies unoptimized. F1 and F2: one warm-up and 10
runs per variant; F3 and F4: one warm-up and 8 runs.

**Decision rule, fixed before the runs.** A change is kept when, in the four pairs of
combinations that differ only in it, it is at least 3 % faster with non-overlapping min-max
ranges in every pair for at least one command on one fixture; when no pair is 3 % slower with
non-overlapping ranges anywhere; and when no pair grows the median max RSS by more than 5 %.

Median wall time in milliseconds (`c`, `p` and `r` give each switch, 1 is on):

| Fixture | Command | `master` | c0p0r0 | c0p0r1 | c0p1r0 | c0p1r1 | c1p0r0 | c1p0r1 | c1p1r0 | c1p1r1 |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| F1 | `inspect-elements` | 581 | 569 | 563 | 358 | 371 | 522 | 536 | 313 | 284 |
| F1 | `definitions` | 555 | 561 | 551 | 368 | 346 | 520 | 490 | 282 | 272 |
| F1 | `export-view` | 600 | 589 | 567 | 363 | 338 | 507 | 527 | 290 | 290 |
| F1 | `copy-element` | 1643 | 1576 | 1537 | 1343 | 1377 | 1427 | 1467 | 1161 | 1186 |
| F2 | `inspect-elements` | 612 | 616 | 634 | 394 | 404 | 609 | 642 | 380 | 381 |
| F2 | `definitions` | 564 | 556 | 558 | 342 | 347 | 591 | 543 | 353 | 344 |
| F2 | `export-view` | 551 | 553 | 578 | 351 | 321 | 553 | 554 | 334 | 327 |
| F2 | `copy-element` | 1672 | 1621 | 1594 | 1345 | 1357 | 1527 | 1553 | 1325 | 1308 |
| F3 | `inspect-elements` | 12897 | 12695 | 12755 | 12412 | 12348 | 1712 | 1728 | 1300 | 1320 |
| F3 | `definitions` | 13290 | 12892 | 12580 | 12417 | 12666 | 1769 | 1690 | 1281 | 1295 |
| F3 | `export-view` | 12793 | 12738 | 13169 | 12714 | 12617 | 1781 | 1720 | 1326 | 1308 |
| F3 | `copy-element` | 27815 | 27670 | 27374 | 26968 | 26345 | 3859 | 3966 | 3369 | 3625 |
| F4 | `inspect-elements` | 3012 | 2953 | 2902 | 2748 | 2668 | 2941 | 2957 | 2899 | 2574 |
| F4 | `definitions` | 2904 | 2968 | 3032 | 2688 | 2721 | 2994 | 2987 | 2742 | 2667 |
| F4 | `export-view` | 2813 | 2728 | 2853 | 2572 | 2430 | 2845 | 2878 | 2550 | 2599 |
| F4 | `copy-element` | 6421 | 6404 | 6337 | 6178 | 6185 | 4114 | 4120 | 3801 | 3803 |

On/off ratio of the median over the four pairs of each change (a * marks a fixture and command
where every pair passed the rule):

| Fixture | Command | C (body cache) | P (one parse) | R (one read) |
|---|---|---|---|---|
| F1 | `inspect-elements` | 0.77-0.95 | 0.53-0.66 * | 0.91-1.04 |
| F1 | `definitions` | 0.77-0.93 | 0.54-0.66 * | 0.94-0.98 |
| F1 | `export-view` | 0.80-0.93 | 0.55-0.62 * | 0.93-1.04 |
| F1 | `copy-element` | 0.86-0.95 | 0.81-0.90 | 0.98-1.03 |
| F2 | `inspect-elements` | 0.94-1.01 | 0.59-0.64 * | 1.00-1.05 |
| F2 | `definitions` | 0.97-1.06 | 0.60-0.64 * | 0.92-1.02 |
| F2 | `export-view` | 0.95-1.02 | 0.55-0.63 * | 0.91-1.05 |
| F2 | `copy-element` | 0.94-0.99 | 0.83-0.87 | 0.98-1.02 |
| F3 | `inspect-elements` | 0.11-0.14 * | 0.76-0.98 | 0.99-1.02 |
| F3 | `definitions` | 0.10-0.14 * | 0.72-1.01 | 0.96-1.02 |
| F3 | `export-view` | 0.10-0.14 * | 0.74-1.00 | 0.97-1.03 |
| F3 | `copy-element` | 0.13-0.15 * | 0.87-0.98 | 0.98-1.08 |
| F4 | `inspect-elements` | 0.97-1.06 | 0.87-0.99 | 0.89-1.01 |
| F4 | `definitions` | 0.98-1.02 | 0.89-0.92 | 0.97-1.02 |
| F4 | `export-view` | 0.99-1.07 | 0.85-0.94 | 0.95-1.05 |
| F4 | `copy-element` | 0.62-0.65 * | 0.92-0.98 | 0.99-1.00 |

* **C is kept.** It cuts the frame (F3) by 85 to 90 % for every command and the write path of
  distinct members (F4) by 35 to 38 %, where the commit verifier reuses the head's bodies; with
  nothing to share (F2, F4 reads) it stays within noise and grows no max RSS beyond 1.5 %. On F1
  the kernel cost of 300 box plates is small next to parsing, so the gain is larger once P is on.
* **P is kept.** It cuts every read of the plate fixtures by 34 to 47 % and the frame by about
  24 % once C is on; no pair is slower.
* **R is dropped.** The second read and hash is at most half of the read row in the phase
  tables (about 8 to 37 ms, some 3 % of an inspection or less); no pair is clearly faster or
  slower and the medians move both ways by up to 11 %, which is the noise of this machine. It is
  not part of this change.

## Results: `master` against this change

The `master` binary (`538cbed`) against the release binary of this branch (`4604a36`, the body
cache and one parse; built from the same product code before a formatting-only test fix was
folded into that commit), on the same machine and fixtures, alternating every run: one warm-up and
15 runs per binary on F1 and F2, 10 on F3 and F4. `copy-element` runs on a fresh copy of the
fixture each time and is the write path (head evaluation plus commit verifier). This run was
noisier than the experiment, which the min-max ranges show; the medians agree with it.

| Fixture | Command | `master` median (min-max), ms | This change, ms | Change | Max RSS, MiB | Output |
|---|---|---:|---:|---:|---:|---|
| F1 copies | `inspect-elements` | 821 (607-976) | 401 (295-543) | -51 % | 37.2 / 37.0 | identical |
| F1 copies | `definitions` | 708 (606-914) | 351 (282-463) | -50 % | 37.2 / 37.0 | identical |
| F1 copies | `export-view` | 731 (588-869) | 341 (285-415) | -53 % | 36.8 / 36.8 | identical |
| F1 copies | `copy-element` | 1,920 (1,628-2,423) | 1,389 (1,229-2,185) | -28 % | 74.1 / 73.8 | identical |
| F2 distinct plates | `inspect-elements` | 754 (561-951) | 471 (388-556) | -38 % | 37.2 / 37.0 | identical |
| F2 distinct plates | `definitions` | 656 (555-876) | 400 (355-507) | -39 % | 37.2 / 37.0 | identical |
| F2 distinct plates | `export-view` | 687 (557-916) | 420 (337-579) | -39 % | 36.8 / 36.7 | identical |
| F2 distinct plates | `copy-element` | 1,791 (1,550-2,219) | 1,488 (1,339-1,992) | -17 % | 74.1 / 74.2 | identical |
| F3 frame | `inspect-elements` | 14,177 (13,005-15,207) | 1,424 (1,294-1,991) | -90 % | 70.7 / 67.9 | identical |
| F3 frame | `definitions` | 14,100 (13,022-14,965) | 1,377 (1,293-1,756) | -90 % | 70.7 / 68.0 | identical |
| F3 frame | `export-view` | 13,686 (12,929-15,479) | 1,350 (1,250-1,584) | -90 % | 70.4 / 67.6 | identical |
| F3 frame | `copy-element` | 28,201 (26,726-30,227) | 4,004 (3,892-4,799) | -86 % | 168.3 / 165.5 | identical |
| F4 distinct members | `inspect-elements` | 3,016 (2,823-3,588) | 2,801 (2,590-3,189) | -7 % | 42.9 / 41.9 | identical |
| F4 distinct members | `definitions` | 2,999 (2,719-3,209) | 2,676 (2,592-2,776) | -11 % | 43.0 / 42.0 | identical |
| F4 distinct members | `export-view` | 2,897 (2,715-3,107) | 2,718 (2,431-3,075) | -6 % | 42.6 / 41.6 | identical |
| F4 distinct members | `copy-element` | 6,560 (6,265-7,249) | 3,921 (3,689-4,208) | -40 % | 96.5 / 96.8 | identical |

CPU time follows the wall time within a few percent: every command is single-threaded. On all
four fixtures `inspect-elements` answers `LIMIT_EXCEEDED` on both binaries, with identical bytes
(see "Not changed"); its time is still the whole inspection.

Where the time goes after the change (`inspect-elements`, same probe, milliseconds):

| Phase | F1 copies | F2 distinct plates | F3 frame | F4 distinct members |
|---|---:|---:|---:|---:|
| Read and hash every layer blob (twice) | 15.7 | 15.6 | 70.5 | 43.1 |
| `Reader::open_validated` (one parse) | 218.9 | 242.2 | 429.6 | 295.0 |
| Index and decode | 18.4 | 19.3 | 53.1 | 32.1 |
| Kernel evaluation | 0.8 | 63.1 | 605.7 | 2,470.5 |
| Mesh comparison, snapshot, JSON | 9.4 | 11.5 | 18.2 | 18.8 |
| Rest | 36.9 | 34.9 | 62.8 | 35.0 |
| **Wall** | **300** | **387** | **1,240** | **2,895** |

What is left is the one parse of the stage on simple models and the kernel work on bodies that
nothing shares (F4). Within one process the cache cannot remove the latter; see below.

## Why there is no persistent cache

Each CLI call is a new process, so this cache cannot help a second call on an unchanged
revision. Reuse across calls would need state on disk:

* outside the store, because a received store must not carry entries that are trusted;
* keyed by the exact build, because a kernel change without a version bump would otherwise let
  a stale stored mesh pass the comparison that re-evaluation exists for;
* checked on read (blob digest recomputed, mesh decoded, stored inputs compared), with private
  directory checks, atomic writes and bounded eviction, and an opt-in switch for the window.

That is new local state and a new trust surface; the owner chose the in-process cache. Only a
long-lived reader process could reuse bodies across calls without on-disk state, and there is
none: U17 removed the USD worker, and the window reads through CLI processes.

## Not changed

* Each layer blob is still read and hashed twice per inspection (R above: no measurable effect).
* Six adapters read their stage with the same two-pass pattern as before: `assembly`, `bolt`,
  `level`, `numbering_approval`, `reference` and `weld`. They serve their own commands
  and are left as they are; `Reader::open_validated` is ready for them. It shares one
  implementation of the rules with `validate_layers`, so a rule added there applies to both.
* A body is still cloned into every element's record; sharing by reference would change public
  record types.
* On `538cbed` the native window ran `export-view`, `definitions` and `inspect-elements` as separate
  processes per refresh. Current `master` (the issue 26 follow-up,
  [Apibim-com/BeamCraft#159](https://github.com/Apibim-com/BeamCraft/pull/159)) runs one
  `export-view --with-definitions` per refresh and one `inspect-elements --element` per selection.
  Each of them is one inspection and pays less in the same way.
* **Found while measuring, not changed here:** on every fixture of about 300 elements,
  `inspect-elements` answers `{"outcome":"rejected","code":"LIMIT_EXCEEDED"}` (exit 2), on `master`
  as after this change. The whole inspection runs, but its JSON exceeds the read-profile
  encoder limits that `print_json` enforces, so the command cannot report a model of that size.
  On `538cbed` the window's property lookup ran this command and failed the same way; current
  `master` asks for one element (`inspect-elements --element`), which fits. Its timings above are
  still the inspection's. Paging the whole-model reply is a separate change.

## Reproduction

Fixtures are contributor-original synthetic stores made with the public CLI, with round values
chosen for this measurement (no catalogue data); they are not committed.

| Fixture | Elements | Distinct bodies | Content |
|---|---:|---:|---|
| F1 copies | 300 | 1 | one 1000 x 600 x 10 plate and 299 `copy-element` copies |
| F2 distinct plates | 300 | 300 | box plates 1000 to 1299 x 600 x 10 |
| F3 frame | 295 | 59 | see below; 2 materials and 3 profiles |
| F4 distinct members | 200 | 200 | I section (h 300, b 150, tw 7, tf 10, r 15) beams 3000 to 4990 mm |

F3: 40 columns (RHS h 200, b 200, t 8, r 16 at 3000 mm and the I section at 3600 mm, 20 each),
75 I-section beams (6000, 4500 and 3000 mm, 25 each), 10 rectangle 200 x 300 beams of 6000 mm cut
at 3000 mm, 40 base plates 400 x 400 x 20 with four round holes of 22 mm, 40 L-shaped gusset
contours (`0,0;1000,0;1000,300;400,300;400,600;0,600`) with one 100 x 100 hole, 40 stiffeners
1000 x 500 x 10 cut at 250 mm, 20 distinct plates and 30 distinct I-section beams. Every group
except the last two is one element and its copies.

```sh
cargo build --profile dist -p beamcraft-cli   # the settings measured here (--release on 538cbed)
b=./target/dist/beamcraft
s=$(mktemp -d)   # F1; the other fixtures follow the table the same way
$b create-plate --store "$s" --width 1000 --length 600 --thickness 10 \
  --new-id 019a7bf0-0001-7000-8000-000000000001 > /dev/null
for k in $(seq 1 299); do
  $b copy-element --store "$s" --id 019a7bf0-0001-7000-8000-000000000001 \
    --offset "$((k * 1100)),0,0" > /dev/null
done
# One measurement: wall time, CPU and max RSS of one process.
/usr/bin/time -v "$b" inspect-elements --store "$s" > /dev/null   # Linux
/usr/bin/time -l "$b" inspect-elements --store "$s" > /dev/null   # macOS
```

The figures above were taken with a harness that does what `/usr/bin/time` does (`wait4`:
wall, user plus system CPU, max RSS) because the container had no `/usr/bin/time`; it hashes
every output, runs one warm-up and then the stated number of runs per variant, and rotates the
order of the variants every round so that drift of the machine spreads over all of them.
