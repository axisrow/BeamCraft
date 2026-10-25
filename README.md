<p align="center">
  <img src="docs/brand/beamcraft-logo.svg" alt="The BeamCraft logo: an isometric steel I-beam on a blueprint grid" width="192">
</p>

<h1 align="center">BeamCraft™</h1>

<p align="center">
  <b>An open-source structural steel detailing workbench, written from scratch in Rust.</b><br>
  Model, detail and document steel structures in a fast, native app, built from public standards.<br>
  Windows · Linux · macOS
</p>

<p align="center">
  <a href="LICENSE">License: MPL-2.0</a>
  <img alt="Written in Rust" src="https://img.shields.io/badge/written%20in-Rust-0a7563">
  <img alt="Status: pre-alpha" src="https://img.shields.io/badge/status-pre--alpha-d9822b">
  <img alt="No account, no telemetry" src="https://img.shields.io/badge/no%20account-no%20telemetry-0a7563">
</p>

<p align="center">
  <a href="https://github.com/Apibim-com/BeamCraft/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/Apibim-com/BeamCraft/actions/workflows/ci.yml/badge.svg"></a>
</p>

<br>

<!-- Screenshots go here once the app is ready:
<p align="center">
  <img src="docs/images/beamcraft-viewer.png" alt="BeamCraft" width="100%">
</p>
-->

> [!NOTE]
> **BeamCraft is pre-alpha.** There is no usable application yet; this repository currently holds
> the project scaffold, the rules and the CI that the first crates are built on.
> Windows and Linux are the initial CI targets. macOS remains a desktop development goal.

## Why BeamCraft

Steel detailing software is expensive, closed and tied to one vendor's file formats. BeamCraft aims
to be an open, scriptable alternative with a native, fast core:

- **Fast.** An original Rust engineering core and a planned native GPU-accelerated UI.
- **Open formats first.** USD is the authoritative project format, read and written by an
  original Rust layer engine. IFC export and DSTV NC are later milestones.
- **Scriptable by design.** Every feature is reachable headlessly from the CLI, so scripts and AI
  agents can drive it as easily as a person can.
- **Yours.** No account, no telemetry, no cloud. It works offline.

## Independent

BeamCraft targets the workflows of professional steel detailing tools such as Tekla Structures, but
it is **not** derived from any of them: it is written from scratch with its own logic, and its
engineering rules come from public standards (IFC, DSTV, EN 1993, AISC, ...) or are our own. No
code, assets or profile databases are taken from any commercial product. The rules are in
[`AGENTS.md`](AGENTS.md).

BeamCraft is an independent project. It is not affiliated with, endorsed by or sponsored by Trimble
Inc. *Tekla* and *Tekla Structures* are trademarks of Trimble Inc.; they are named here only to say
which workflows BeamCraft is comparable to.

## Get started

You need a recent stable [Rust toolchain](https://rustup.rs) (1.96 or newer).

```sh
git clone https://github.com/Apibim-com/BeamCraft
cd BeamCraft
cargo run -p beamcraft-cli -- --version
cargo test --workspace
```

More in [`docs/development.md`](docs/development.md) and [`docs/architecture.md`](docs/architecture.md).

## Drive BeamCraft from Claude Code

This repository is also a Claude Code plugin. It teaches Claude Code to run the `beamcraft`
binary: create, update and inspect members and plates, open and verify a store, snapshot and
export views, and pick features at coordinates - and to open the result in the native viewport
app. There is no MCP server and no background process: commands drive the CLI for modelling,
and `/beamcraft:open` only launches the viewport app on the store.

```sh
claude plugin install beamcraft --marketplace axisrow/BeamCraft
```

or from a local checkout instead of GitHub:

```sh
claude plugin install beamcraft --marketplace /path/to/BeamCraft
```

The commands are `/beamcraft:model`, `/beamcraft:store`, `/beamcraft:view`, `/beamcraft:pick`
and `/beamcraft:open`, which launches the native viewport app on a store; the same workflows
are also available as skills that Claude picks up from the request.
See [`.claude-plugin/plugin.json`](.claude-plugin/plugin.json) and the
`skills/` guides, which embed real captured CLI output as their acceptance evidence.

## What's next

First, establish model and geometry contracts plus original fixtures. Then demonstrate
headless plate creation and atomic saving of a complete revision that ordinary OpenUSD can open.
See [`ROADMAP.md`](ROADMAP.md) for acceptance gates.

## Contributing

Humans and AI agents are both welcome. Read [`CONTRIBUTING.md`](CONTRIBUTING.md) and
[`AGENTS.md`](AGENTS.md) first; the provenance rules are strict.

## License

BeamCraft's original source code and documentation are licensed under the Mozilla Public License,
version 2.0 ([`LICENSE`](LICENSE)). Contributions to that code and documentation are accepted under
`MPL-2.0`. Separately licensed third-party dependencies and assets retain their own licences.

When distributing executable versions, make the corresponding MPL-covered source available and
tell recipients where to obtain it, as required by MPL-2.0 section 3.2. The source repository is
[Apibim-com/BeamCraft](https://github.com/Apibim-com/BeamCraft); source must correspond to the
distributed version, including any modifications.

Copyright © 2026 [Apibim](https://github.com/Apibim-com).

## Trademark

**BeamCraft™** and the BeamCraft logos are trademarks of Apibim; registration is pending. The
MPL-2.0 licence does not grant rights to the name or logos. You may use the name to refer to
the official project; forks and modified versions must not present themselves as BeamCraft and must
remove the logos. See [`docs/brand/LICENSE-brand.txt`](docs/brand/LICENSE-brand.txt).
