# BeamCraft — instructions for agents

BeamCraft is an open-source, Rust-native structural steel detailing application written from scratch,
developed by Apibim. It targets the workflows of tools like Tekla Structures without using any of
their code or assets. Targets desktop-only use on Windows, Linux and macOS.

## Start every session here

1. Read `AGENTS.md` (it wins over this file) and `ROADMAP.md` (the stage, the acceptance-gate
   verdicts and the MVP scope; it lists no tasks). Find what to work on, task IDs (F04, U06,
   A03, ...), dependencies, acceptance and current status in the
   [GitHub task catalogue](https://github.com/Apibim-com/BeamCraft/issues/474), its linked
   issues, the milestones and the GitHub Project. Owner decisions are in `docs/plan/decisions.md`.
   When you start work on an issue, add the label `in progress` to it (`AGENTS.md` §7).
2. Read `docs/architecture.md` and the README of the crate you are touching.
3. Work in small steps: orient → plan → implement + test → verify → record → commit.
4. There is no progress log to append to: the history is git and the PR descriptions, and what a
   package does is recorded in its note under `docs/design/`. Do not edit `ROADMAP.md` in a
   feature PR; it changes only in a docs-only PR when a gate verdict or the scope changes.
   A PR description goes into the PR itself, never into a file in the repository.

## Non-negotiables

- **Assets and provenance: `AGENTS.md` §1 and §2.** Nothing from Tekla/Trimble, ever.
- **Never crash on input:** `AGENTS.md` §4. Clippy denies `unwrap`/`expect`/`panic`/indexing in
  non-test code.
- **Headless first:** every feature has an engine API and a CLI command, except interactive
  assistance (picking, snapping, start screen), which needs no CLI command (`AGENTS.md` §3).
- **Units:** millimetres, radians internally, right-handed, Z up. Convert only at the UI/IO edge.

## Commands

```sh
cargo run --locked -p beamcraft-test-gate -- check --scope full
```

The recorded gate runs unfiltered nextest 0.9.149 **and mandatory doctests**, format validation,
Clippy and documentation with warnings denied. It enforces execution budgets, serializes heavy
checks across worktrees and requires the committed source to stay unchanged. Independent
CLI-only changes may use `--scope cli --ui-independent-reason "..."` when no shared code,
dependencies, build settings or UI-consumed commands/options/JSON change. CI then tests the
whole workspace on the two other operating systems; missing or stale local evidence fails CI.
Publish the report explicitly with the runner's `publish --report PATH --pr NUMBER` command.
New head/base snapshots require a new report. See `tools/test-gate/README.md` and the owner
policy in `docs/plan/test-gate.md`; this supersedes the literal-Cargo rule from #346.

Targeted Cargo/nextest commands remain iteration tools. A quick smoke after a build:
`cargo run -p beamcraft-cli -- --version`. CI also checks WASM compatibility, dependencies,
fixtures and module boundaries. Release packaging is unchanged.

## Conventions

- Edition 2024, workspace lints in the root `Cargo.toml`. Only `beamcraft-geom`,
  `beamcraft-model` and `beamcraft-kernel` stay WASM-compatible as a layering discipline.
  Every other crate, including later crates, is desktop-native with no WASM requirement.
- Crates live in `crates/`, binaries in `apps/`. Dependencies point one way: `geom` ← `model` ←
  everything else. Do not add a back-edge.
- Every public item has a doc comment (`missing_docs` is on).
- Match the surrounding code's style; keep PRs small and focused. Commit messages: imperative
  mood, explain the why.
- Branch from the verified current `origin/master`; follow `AGENTS.md` section 6 for naming
  (`feat/`, `bug/`, `docs/`, `chore/`, `refactor/`, `test/`). Open a PR using the template.
