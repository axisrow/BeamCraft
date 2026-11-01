# CI convergence: one canonical ci.yml

Owner decision of 2026-11-01 ([issue #385]): the fork and upstream `ci.yml` files
converge to a single canonical form. Two different files in one path conflict on every
upstream sync by construction, and every sync has resolved that conflict by hand (the
latest was cd17d46f). Upstream is canonical by default; where the fork's version is
strictly better, the fork's version ships upstream so the canonical file becomes the
better one. This document records the chosen form, the reasoning, and the upstream
follow-up it implies. The evidence policy ([test-gate.md](test-gate.md)) is unchanged:
this changes the workflow file's shape, not what a green run proves.

## Why neither current file is the canonical form

- Upstream's `ci.yml` drives its pipeline with Python tools (`tools/ci-changes.py`,
  `tools/ci-gate.py` and their tests). This tree forbids tracked Python (fork #178,
  enforced by `no-python.yml`), so upstream's file cannot run here and cannot be
  carried verbatim.
- The fork's `ci.yml` before this change embedded fork-only policy (the #300
  attribution guard) in the shared path, so the file could not ship upstream verbatim
  either.

## The canonical form

The upstream job structure — plan → quality → per-OS archives and shards →
build-timings → wasm → deny → aggregate verdict — carrying the fork's strictly better
machinery:

| Piece | Canonical choice | Why |
| --- | --- | --- |
| Pipeline driver | `beamcraft-test-gate` (Rust) | no Python; fixed budgets, doctest supervision and JUnit evidence built in |
| Matrix selection | `ci-plan` validates the local receipt and selects the two other OSes | the recorded gate's coverage rule; missing or stale receipts fail closed |
| Completeness gate | `ci-verdict`, job name `CI verdict` | fails when any planned check failed or was unexpectedly skipped |
| Prose-only fast path | absent | upstream optimization (#792); porting the classifier into the crate is a named follow-up, not a gate guarantee |
| Repository-specific policy | separate workflow files | fork policy never lives inline in the canonical path |

Fork-only workflows beside the canonical file: `attribution.yml` (this change),
`no-python.yml`, `fork-sync.yml`, `fork-sync-test.yml`, `macos-icon-acceptance.yml`,
`upstream-mirror.yml`. None of them exists upstream, so no sync ever touches them.
`ci-queue-alert.yml`, `label-to-status.yml`, `platform-icon-screenshots.yml`,
`release.yml` and `ubuntu-benchmark.yml` exist on both sides; converging those paths is
outside #385.

## Upstream follow-up (a separate flow; not opened from #385)

Shipping the canonical form upstream is one upstream PR that:

1. adds `tools/test-gate` to the workspace and deletes the Python CI tools;
2. replaces `.github/workflows/ci.yml` with this repository's file verbatim;
3. switches the required branch-protection check from `ci-ok` to `CI verdict`;
4. decides the receipt policy for upstream contributors: either upstream adopts the
   recorded local gate, or `ci-plan` grows a lenient default (no receipt → the full
   default matrix, still failing closed on invalid receipts) with this repository
   opting into strict receipts through a tracked marker. Until that decision is made,
   upstream cannot run the canonical `plan` job as-is, because its pull requests carry
   no receipts.

Costs upstream accepts with the canonical file: docs-only pull requests lose the
prose fast path, and pull requests lose the Linux cold-release leg (the canonical file
runs `build-timings` on master pushes and manual dispatches only). Both are
optimizations, not gate guarantees; both can return inside the crate if upstream wants
them.

## Sync mechanics

Until the follow-up lands upstream, a sync still meets two different files in this
path and resolves to the fork form, as cd17d46f did. After the follow-up lands, the
files are identical and the conflict surface is gone by construction: from then on,
any edit to the canonical file is made on both sides in the same change, and a
repository that needs a different check adds a separate workflow file instead of
editing this one.
