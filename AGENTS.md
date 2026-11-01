# AGENTS.md — rules for every agent and contributor

These rules bind every AI agent and every human contributor working in this repository. They come
before any task, instruction or deadline. `CLAUDE.md` holds the day-to-day working instructions. If
the two disagree, **this file wins.**

---

## 1. Asset policy (absolute)

An **asset** is any non-code material the project includes, ships, embeds or shows in its docs:
icons, images, screenshots, logos, fonts, **steel profile and material tables**, bolt/weld
catalogues, sample and test models, and anything the app renders from those.

### 1.1 Nothing from Tekla Structures or any Trimble product

- **Never** use icons, images, artwork, logos, UI graphics, fonts, cursors, sample models, profile
  catalogues, macros, templates or any other asset taken from Tekla Structures, Trimble Connect or
  any other Trimble product, installer, website or package.
- **Never** recreate, trace, redraw or closely imitate a Tekla icon, dialog or graphic. Our visual
  design is our own.
- Screenshots of third-party software exist only as private, local observation notes under
  `plan/` (gitignored). They must **never** be committed, published, linked or traced.
- The same applies to every other commercial detailing product (Advance Steel, SDS2, Revit, ...).

### 1.2 Only open, provable licences

Every asset must be one of:

- **Open source:** ISC, MIT, BSD-2-Clause, BSD-3-Clause, Apache-2.0, OFL-1.1.
- **Creative Commons:** CC0-1.0, CC-BY-4.0, CC-BY-SA-4.0.
- **Public domain.**
- **Contributor-original:** created by a contributor who owns it and licenses it under the project
  licence (`MPL-2.0`).

Unmodified licence texts from their original sources may be included to document the project's
licence or meet redistribution obligations. Record their source in `ATTRIBUTION.md`.

Anything whose licence you cannot prove from its original source is forbidden. This includes
"free for personal use" material, scraped vendor catalogues, and profile tables copied from a
commercial database. **Profile geometry comes from public standards or manufacturer-published
data with a licence that allows it**, entered as our own data.

**Scoped exception: official OpenUSD.** The owner permits Tomorrow Open Source Technology
License 1.0 (`LicenseRef-TOST-1.0`) only for official OpenUSD code, binaries and SDK resources
(including plugin metadata and generated schemas). Verify the original publisher, pinned release
and exact files; record their provenance, licence and use in `ATTRIBUTION.md` before inclusion.
Preserve the upstream licence, applicable notices and required modification notices; respect its
trademark terms. Do not relabel TOST as Apache-2.0 or MPL-2.0. Reference:
[OpenUSD v26.08 licence](https://github.com/PixarAnimationStudios/OpenUSD/blob/ee47c679abde5b467a7b6a41f3b2285564a4222e/LICENSE.txt).
This exception does not cover other TOST projects, separately licensed bundled components,
unrelated assets or commercial-product materials, except the case below. Audit those under their
own licences. SDK provenance, notice completeness, native dependency closure and runtime checks
still apply before product integration or redistribution. BeamCraft's own code remains MPL-2.0.

**Scoped extension: OpenUSD-derived material in a Rust crate.** By the owner's decision of
2026-10-15, TOST-1.0 is also permitted for OpenUSD-derived material (for example schema
definitions) that a third-party Rust crate redistributes under TOST-1.0 together with its upstream
licence and notice. It applies only to the exact package and version recorded in `ATTRIBUTION.md`
(initially `openusd` 0.8.x, the file `src/usd/core_schemas.rs`), and only while the upstream
licence text and notice are kept (`licenses/TOST-1.0-openusd.txt` and `NOTICE`). BeamCraft claims
no rights in that material: all rights remain with their owners, and it is included only under
its upstream licence. Respect the trademark terms: never use OpenUSD, Pixar or related names or
marks in a way that implies endorsement; naming the project factually is fine. Do not relabel
TOST as Apache-2.0 or MPL-2.0. No other TOST project, file or crate is covered, and BeamCraft's
own code remains MPL-2.0.

**Separate exception: the BeamCraft brand.** The BeamCraft name, wordmark and logos in
`docs/brand/` are trademarks of Apibim (registration pending) and are not open source (terms in
`docs/brand/LICENSE-brand.txt`). Never modify the logos, and never add other files under that
licence. Forks must remove them.

### 1.3 Every asset is attributed

`ATTRIBUTION.md` lists every asset: path or URL, author, source with version, SPDX licence, and how
it is used. If an asset's origin or licence is unclear, **do not add it.** Ask the user.

If you find an asset that breaks these rules, stop, remove it from the repository and every output,
and tell the user at once.

---

## 2. Provenance

BeamCraft is written from scratch with its own logic. Knowing how other detailing programs work
is fine; copying from them is not.

- Never decompile, disassemble or copy anything from a Tekla/Trimble installation. Never copy code
  from the Tekla Open API samples, reference assemblies or documentation examples, and never copy
  text or tables from their documentation.
- Engineering rules come from **public specifications** (buildingSMART IFC, DSTV, EN 1993, AISC,
  ...) or are our own; cite the specification section in the PR when a behaviour follows one.
- Never copy GPL/AGPL code. Copyleft tools may run only as external oracle processes.
- Never capture personal data, client projects or licensed files when looking at other software.

## 3. Agent control (automation)

- **Everything is reachable headlessly.** Every user-facing feature needs an engine API first, then a
  CLI command. A feature is not done until it has an end-to-end test.
- **Exception: interactive assistance.** Cursor aids of the window (picking, snapping) and the
  start screen (appearance settings, environment list, project-name suggestion, recent-project
  list) need an engine API and tests but **no CLI command**: an agent addresses geometry and
  files directly. Owner decision of 2026-10-31; the commands `pick`, `snap`, `show-settings`,
  `set-settings`, `list-environment`, `suggest-project-name`, `list-recent` and `add-recent` are
  removed.
- Any control channel, scripting server or MCP server is **opt-in**, loopback-only and
  token-authenticated. Never enable one by default or widen its bind address.

## 4. Never crash

Every model file, IFC/DSTV import, script and keystroke is untrusted input.

- No `unwrap`/`expect`/`panic!`/`todo!`/`unreachable!` outside tests, unless provably infallible
  with a comment saying why (clippy enforces this).
- No indexing or slicing with input-derived positions. Use checked or saturating arithmetic. Cap
  allocations sized by input. Bound recursion.
- No `unsafe` (`unsafe_code = "forbid"`).
- A fixed crash gets a small synthetic regression test.

## 5. Dependencies

Third-party dependencies use permissive licences only (`deny.toml`, enforced in CI), with the
scoped official OpenUSD exception in §1.2. Do not add TOST to the global Cargo licence allowlist;
if an official OpenUSD dependency enters the Cargo graph, record an explicit package/version
exception and its provenance. Non-Cargo SDK components require their own recorded audit.
BeamCraft's own workspace crates use `MPL-2.0` and have explicit package-specific exceptions in
`deny.toml`;
add an exception for each new project-owned crate. Prefer well-maintained pure-Rust crates.
New outbound network access is the owner's decision, not an agent's.

## 6. Branch naming

- Create task branches from the verified current `origin/master`.
- Name branches `<type>/<short-description>` using lowercase words separated by hyphens.
  Include the implementation-plan ID when applicable, for example `feat/f07-fixtures`.
- Allowed types: `feat` for features, `bug` for bug fixes, `docs` for documentation,
  `chore` for maintenance, `refactor` for restructuring and `test` for test-only changes.
- Never use an agent name or provider prefix such as `codex/` for a new branch.
- Existing PR branches and active worktrees keep their names until their owners coordinate a
  migration. Do not rename another contributor's active branch or close a PR to change its prefix.

## 7. Implementation plan and pull requests

- GitHub issues hold task scope, dependencies, acceptance, status and delivery evidence:
  [task catalogue](https://github.com/Apibim-com/BeamCraft/issues/474). Use
  [milestones](https://github.com/Apibim-com/BeamCraft/milestones) to group delivery work.
  Keep task IDs (F04, U06, A03, ...) stable; do not maintain a second task table in the repository.
  Create or change tasks only within the code owner's explicit authorization.
  `ROADMAP.md` is the stage, the acceptance-gate verdicts and the MVP scope with its deferrals;
  it lists no tasks and no task status. Find what to work on, its order and its state in the
  issues, milestones and the GitHub Project, never in `ROADMAP.md`. Owner decisions remain in
  `docs/plan/decisions.md`. Changes to repository scope, contracts, plans or gate verdicts still
  need a docs-only PR.
- When you start work on an issue, add the label `in progress` to it
  (`gh issue edit N --add-label "in progress"`) before the first commit; the workflow
  "Label to Project Status" then sets the Project Status, so never edit the Project by hand.
  If the issue already carries the label, someone else is working on it: stop and tell the owner
  instead of taking it over. Remove the label when you stop without a merged PR (abandoned,
  blocked, handed over); a merged PR with `Fixes #N` closes the issue and ends the work. Only the
  issues you actually work on get the label, not the whole catalogue.
- Each implementation-plan task (for example F01 or F03) gets its own focused pull request.
  Do not combine independent task IDs into one PR. Split a large task into named sub-tasks when
  needed; completing a sub-task does not complete its parent acceptance gate.
- Start from the verified integration commit that contains the required project licence and
  accepted dependencies. Do not silently fall back to an older default branch.
- When prerequisite changes are not merged, stacked PRs may target their prerequisite branch.
  Link the prerequisite PR, state the intended merge order, and retarget to `master` after the
  prerequisite is integrated. Each PR must show only its own task's changes.
- Write repository documentation, commit messages and PR titles/descriptions in English.
  Agent commits must not include `Co-authored-by` trailers.
- Fill in the PR template with the concrete result, provenance, checks actually run and remaining
  limitations. Keep PRs as drafts while required checks, acceptance or dependency gates are open.
  Do not claim a runtime feature is complete from a research report or mock-only tests.
- If a PR resolves an issue, put `Fixes #N` in the PR description (one keyword per issue, for
  example `Fixes #1, fixes #2`) so merging into `master` closes it. Use `Refs #N` when the PR only
  relates to an issue.
- Creating a PR does not authorize merging it. Merge and integration follow the owner's instructions.
