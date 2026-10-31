# Recorded test gate

Owner decision 2026-10-31, replacing the literal `cargo test --workspace` rule in fork
PR #346. Refs [issue #342](https://github.com/axisrow/BeamCraft/issues/342).
This specification becomes operational with the implementation PR, not this docs-only PR.

## Local verification

The platform-independent Rust tool `beamcraft-test-gate` provides:

```sh
cargo run --locked -p beamcraft-test-gate -- check --scope full
cargo run --locked -p beamcraft-test-gate -- check --scope cli --ui-independent-reason "..."
```

`full` is the default: non-mutating format validation, Clippy with warnings denied, all
workspace nextest targets, doctests, and documentation with `RUSTDOCFLAGS="-D warnings"`.
`cli` selects the whole CLI package for Clippy, ordinary tests, doctests and documentation;
it does not build the UI. Never use a test-name filter or the `fast` profile for a gate.

CLI-only is allowed when runtime changes are confined to CLI source/tests and the developer
explains why commands, options and responses consumed by the UI are unaffected. Shared crates,
UI, dependency manifests/lockfiles, build settings and shared contracts require `full`.
Accompanying ordinary Markdown documentation is allowed; uncertain changes require `full`.

Use cargo-nextest 0.9.149 with `--no-fail-fast`, followed by mandatory doctests with
`--no-fail-fast`. A failed ordinary test does not suppress doctests when compilation succeeded.
Record every exit status; a later successful command cannot erase an earlier failure.
No automatic retries, successful timeouts, or silent empty/incomplete runs.

Use one OS-level heavy-gate lock per user on the machine, shared across worktrees. Lock wait,
compilation, downloads, archive preparation and extraction are outside test execution budgets.
Doctest execution includes compilation of the examples after dependencies have been prepared.
Do not kill other developers' builds; developers use the common runner for heavy checks.

An eligible report requires the exact clean source snapshot to remain unchanged throughout
the gate. Store logs and a versioned JSON report under ignored build output. Record commit,
tree, base commit, OS, architecture, scope, UI-independence explanation, tool versions, selected
test count, stage exit statuses and durations. The report must explicitly say when local UI
tests were omitted. Publishing is a separate explicit operation.

## Runtime budgets

| Execution | Maximum |
| --- | --- |
| Individual nextest test | 120 seconds |
| CI test partition | 180 seconds |
| Local CLI nextest run | 300 seconds |
| Local workspace nextest run | 600 seconds |
| Doctest command after dependency preparation | 120 seconds |

Timeouts fail, terminate the affected process tree, and retain available diagnostics. Keep the
per-test and global nextest settings in versioned profiles. Changing budgets requires review;
the runner must not silently relax them on slow hardware or infer a new budget from failures.
Existing build-job watchdogs remain separate from these execution budgets.

Full UI checks fail when their required CLI or GPU adapter is missing, instead of reporting
an early successful `SKIP`. A software Vulkan adapter is sufficient on headless Linux.
Platform-conditional tests remain platform-conditional; other partitions' excluded tests do
not count as missing coverage.

## CI selection and report transport

After a successful committed-source check, an explicit publish command places the report in
a delimited machine block in the PR description, preserving the human description. PR edits,
new commits and base updates invalidate old CI selection. The tested head and base must match
the current PR; require the branch to include that base so the local source is the merge tree.
Validate report structure, successful required stages, inventory and scope before selecting OSes.

A `pull_request` event carries a description snapshot that predates the publication landing
seconds after the triggering push. Planning reads the snapshot as the fast path and, when it holds
no receipt current for the run's exact head and base, polls the live PR description for at most
two minutes before failing; a window without a current receipt still fails, and every attempt
keeps the head, base and tree binding. Master pushes and manual dispatches read the live
description directly, so they have no such startup race; a dispatch without its source `--ref`
branch runs the default branch's workflow there and can only plan through the `pr_number` input.
Publication does not itself trigger CI: the workflow reacts to `opened`, `synchronize`,
`reopened` and `ready_for_review` only, so no `cancel-in-progress` supersede follows a
publication and the first run itself goes green once the receipt lands inside its window. A
receipt published later than that window needs a manual dispatch with the `pr_number` input or a
new commit; description edits never start a run, and any later run validates the description as
it is then.

| Local report OS | CI workspace platforms |
| --- | --- |
| Linux | macOS, Windows |
| macOS | Linux, Windows |
| Windows | Linux, macOS |

CI always runs the complete workspace on its two selected platforms, even after a local
CLI-only run. That choice intentionally omits UI verification on the local OS for independent
CLI changes; it must not be described as full UI coverage on all three OSes. Architecture is
recorded; this is an OS-selection policy, not an expansion of the release target matrix.

Retain existing per-platform test partitions. Format, documentation, WASM, licence and boundary
checks must not disappear when an OS is excluded. A single `CI verdict` fails if planning or any
required check fails or is unexpectedly skipped. Missing/invalid/stale reports fail planning;
never fall back to silently skipping another OS or accepting an old green result.

On master pushes, locate the merged PR and require its tested tree to equal the actual commit
tree, supporting merge, squash and rebase integration. Manual runs need the same applicable
report. Direct pushes without one fail. For the existing no-PR upstream sync, a full local
gate may check the staged merge tree: require no unstaged/untracked source changes, verify the
tree again after the gate, and bind the report into the merge commit message. Accept it only
when the created commit has that tree and the expected merge ancestry. No PR is auto-merged.

Release builds/installers retain their existing full platform coverage. Open PRs need fresh
reports when the implementation is enabled.

## Acceptance

Exercise CLI-only selection without UI artifacts, rejection of shared changes, ordinary-test
and doctest failures, individual/global timeout process cleanup on all three OSes, empty or
missing reports, stale head/base/tree rejection, all three two-OS selections, PR merge-tree
matching, sync report binding, cross-worktree locking, and missing UI prerequisites. Use real
small subprocess fixtures for failure/timeout cases, not only command-string assertions.
