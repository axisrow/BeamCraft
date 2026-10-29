# BeamCraft — instructions for agents

BeamCraft is an open-source, Rust-native structural steel detailing application written from scratch,
developed by Apibim. It targets the workflows of tools like Tekla Structures without using any of
their code or assets. Targets desktop-only use on Windows, Linux and macOS.

## Start every session here

1. Read `AGENTS.md` (it wins over this file) and `ROADMAP.md` (current stage and next tasks).
   Find task IDs (F04, U06, A03, ...), dependencies, acceptance and current status in the
   [GitHub task catalogue](https://github.com/Apibim-com/BeamCraft/issues/474) and its linked
   issues. Milestones group delivery work; owner decisions are in `docs/plan/decisions.md`.
2. Read `docs/architecture.md` and the README of the crate you are touching.
3. Work in small steps: orient → plan → implement + test → verify → record → commit.
4. There is no progress log to append to: the history is git and the PR descriptions, and what a
   package does is recorded in its note under `docs/design/`. Do not edit `ROADMAP.md` in a
   feature PR; it changes only in a docs-only PR when a gate status or the scope changes.
   A PR description goes into the PR itself, never into a file in the repository.

## Non-negotiables

- **Assets and provenance: `AGENTS.md` §1 and §2.** Nothing from Tekla/Trimble, ever.
- **Never crash on input:** `AGENTS.md` §4. Clippy denies `unwrap`/`expect`/`panic`/indexing in
  non-test code.
- **Headless first:** every feature has an engine API and a CLI command (`AGENTS.md` §3).
- **Units:** millimetres, radians internally, right-handed, Z up. Convert only at the UI/IO edge.

## Commands

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
cargo test --workspace --doc
cargo run -p beamcraft-cli -- --version
```

`cargo nextest` is the local test runner, matching CI; install it once with
`cargo install cargo-nextest --locked`. nextest never runs doctests, so they stay a separate
line. `cargo test --workspace` runs the same full set in one command.

CI (`.github/workflows/ci.yml`) runs the same on Windows and Linux, plus a wasm32 check of the
three pure crates (`beamcraft-geom`, `beamcraft-model`, `beamcraft-kernel`) and `cargo deny`.
A change is not done until these pass. The full list, with tools and PowerShell notes, is in
`docs/development.md`.

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
