# Documentation map

Start here when you do not know which document to read. Rules first, then direction, then the
contract of the area, then what is actually built.

| Layer | Where | Answers | Nature |
|-------|-------|---------|--------|
| Rules | [`AGENTS.md`](../AGENTS.md), [`CLAUDE.md`](../CLAUDE.md) | What is forbidden; how branches, commits and PRs are made | Binding; `AGENTS.md` wins |
| Direction | [`ROADMAP.md`](../ROADMAP.md), [plan/](plan/README.md) | Stage, gates, MVP scope; owner decisions with dates ([decisions.md](plan/decisions.md)) | Changes in docs-only PRs |
| Tasks | [GitHub task catalogue](https://github.com/Apibim-com/BeamCraft/issues/474), [milestones](https://github.com/Apibim-com/BeamCraft/milestones) | Task IDs, scope, dependencies, acceptance, status and evidence | Update the authorized task issue directly |
| Contracts | [`contracts/`](contracts/) | How a thing must behave: F04 model and commands, F06 geometry, F10 vocabulary, F12 coordination, E01 IFC, F08 module boundaries | Source of truth for code |
| Architecture | [`architecture.md`](architecture.md), [`development.md`](development.md) | How the system is laid out; how to build and test | Living documents |
| Design notes | [`design/`](design/) | What a merged task really does, its decisions, what it does not do, its evidence | Dated snapshot of a slice, not a contract |
| Fixtures | [`fixtures/`](fixtures/) | Own test data and expectations | Reference data |
| Brand | [`brand/`](brand/) | Logos and their licence | Not open source |

## Which document wins

When two documents disagree, the higher one wins:

1. `AGENTS.md`, then `CLAUDE.md`.
2. A contract in `contracts/`, including its recorded amendments.
3. [`plan/decisions.md`](plan/decisions.md), newest date first.
4. The [GitHub task issues](https://github.com/Apibim-com/BeamCraft/issues/474) and `ROADMAP.md`.
5. A design note in `design/`.

A design note records what a slice did on its date. If a later contract amendment or decision
changes the rule, the note gets a dated "Update" paragraph (as in [A04](design/a04-plate-holes.md)
and [A03](design/a03-plane-cuts.md)); the old text stays as history.

## How to move from one task to the next

1. Find the task in the [GitHub catalogue](https://github.com/Apibim-com/BeamCraft/issues/474);
   read its issue for dependencies, result, acceptance, status and evidence.
2. Read the contract section it implements. If the contract is missing or contradicts the task,
   write a docs-only contract amendment first; code follows it.
3. Read the design notes of the tasks it depends on to see what already exists and what is a known
   deviation from the contract.
4. Build engine API, then CLI command, then an end-to-end test, then the UI.
5. Write or update the design note and record task status and evidence in its GitHub issue.
   Repository scope, contract and gate-verdict changes still need a docs-only PR.

## Where each kind of change goes

| Change | Document |
|--------|----------|
| A new capability of an existing task's area | An owner-authorized GitHub task issue with a stable ID and its own design note; never rewrite the old note's history |
| A new rule for model, commands or geometry | An amendment to the contract (F04, F06, ...), recorded in its log and in `decisions.md` |
| An owner choice | `plan/decisions.md`, with the date |
| A fix of a wrong or stale statement | Edit in place, or add a dated "Update" paragraph when the statement was true when written |
| Task scope, dependency, acceptance or status update | The authorized GitHub task issue; contract or owner-scope changes also need the corresponding repository amendment |
| A gate verdict or MVP scope change | `ROADMAP.md` and the dated owner decision, in a docs-only PR |

## Design note naming

`<task-id>-<topic>.md` (for example `a03-plane-cuts.md`). Notes without a task prefix are
cross-cutting evaluations or reports with a version in the name (`*-v0.1.md`).
