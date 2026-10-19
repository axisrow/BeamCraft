# F08: module boundaries, version 0.1.0

Status: **draft 0.1.0, not frozen; K0 not passed**. Date: 2026-10-15.
Base: `29d962e790ffd21ecd9a45143f052d4b4d53d3f7` (stacked on the F06 draft).

**WS amendment, owner decision of 2026-10-16:** BeamCraft is desktop-only on Windows, Linux
and macOS. Only `beamcraft-geom`, `beamcraft-model` and `beamcraft-kernel` retain WASM
compatibility as a layering discipline. Every other crate, including protocol, USD layer engine,
host, IO, CLI, UI and later crates, is desktop-native with no WASM requirement. This amendment
changes F08 portability policy only; frozen F04/F06 contracts are not edited.

This document fixes where the code of each contract area lives, which way dependencies point,
which crates exist now and which are created later by their own tasks. It consumes the
[F04 model and command contract](model-and-commands-v0.1.md) (freeze candidate), the
[F05 worker protocol](worker-protocol-v0.1.md) and the
[F06 geometry and viewport contract](geometry-contract-v0.1.md) (both drafts) and the F07
fixtures. It changes none of their semantics. U05 now adds immutable byte storage and its pure
digest module; paths marked `planned` remain reservations created by their named tasks.

The module table in section 2 is **machine-read**. The `beamcraft-boundaries` tool and its test
(section 6) check the real workspace manifests against it, and CI derives its list of portable
crates from it. Change the table and the manifests together.

## 1. Rules

1. **One direction.** `geom` <- `model` <- everything else. A crate may depend only on crates
   with a strictly lower layer number, and only on those listed in its row. A new edge needs a
   reviewed change to the table.
2. **Kinds.**
   - `domain`: the three pure crates geom, model and kernel; builds for
     `wasm32-unknown-unknown`. No UI, operating-system,
     filesystem, process, network or async-runtime dependency, and no use of `std::fs`,
     `std::process`, `std::net`, `std::thread`, `std::env` or `std::os` outside test code. May
     depend only on other `domain` crates and on the third-party crates of section 3 (none today).
   - `native`: desktop-native library (protocol, USD layer engine, IO and later crates). No
     WASM requirement or WASM-clean dependency/source restriction. Internal edges and lower
     layers still follow the table; third-party dependencies still need the `AGENTS.md` section 5
     audit. A domain crate may never depend on a native crate.
   - `host`: native engine host. Owns process supervision, pipes, deadlines, filesystem access,
     storage, the outcome ledger and the worker client. Depends on domain crates, never the
     reverse. Third-party dependencies need the audit that `AGENTS.md` section 5 requires.
   - `app`: a binary entry point (CLI, later UI). Calls the host and the engine API. Nothing
     depends on an app.
   - `tool`: development tool, `publish = false`, never shipped. Nothing depends on a tool. The F07
    fixture tool lives in `tools/fixtures`; the boundary checker lives in `apps/boundaries` because
    F08 may write only to `crates/` and `apps/`. Moving it is a table edit plus a path move.
3. **Headless first.** Every feature is an engine API first, then a CLI command (`AGENTS.md`
   section 3). The CLI computes nothing that the engine does not expose.
4. **Contracts do not move.** Where a contract places a rule in one layer (for example F06: the
   renderer never computes engineering geometry), this document follows it.
5. **No empty crates.** A crate is created only together with real, tested code taken from a
   contract. Until then its path is documented here and nowhere else.

## 2. Module table

Columns: package; path; kind; layer; allowed internal dependencies (`-` for none); status
(`now` = a workspace member today, `planned` = created by its own task); the task that creates a
planned crate or owns a present one. Kind `domain` is reserved for the three WASM-required
crates; all future crates use a desktop-native kind (`native`, `host`, `app` or `tool`).
Dependencies are checked in `[dependencies]`,
`[dev-dependencies]`, `[build-dependencies]` and target-specific tables alike.

<!-- module-table:begin -->
| Package | Path | Kind | Layer | Allowed dependencies | Status | Owner task |
|---------|------|------|-------|----------------------|--------|------------|
| beamcraft-geom | crates/geom | domain | 0 | - | now | G01 |
| beamcraft-model | crates/model | domain | 1 | beamcraft-geom | now | P01 |
| beamcraft-protocol | crates/protocol | native | 2 | beamcraft-model | now | U01 |
| beamcraft-layer-engine | crates/layer-engine | host | 2 | beamcraft-model | now | U01/U02/U03; separate process, owner decision 2026-10-16 |
| beamcraft-kernel | crates/kernel | domain | 2 | beamcraft-geom, beamcraft-model | now | G02 |
| beamcraft-ifc | crates/ifc | native | 2 | beamcraft-model | now | E02 (IFC4 STEP writer and structural reader; std only, no file system) |
| beamcraft-engine | crates/engine | native | 3 | beamcraft-geom, beamcraft-model, beamcraft-kernel | now | V02 (view snapshot and query slice); P01 (`CreatePlate` application, footprints, transactions, committed-plate queries) |
| beamcraft-host | crates/host | host | 4 | beamcraft-model, beamcraft-protocol, beamcraft-engine | now | U05 |
| beamcraft-cli | apps/beamcraft-cli | app | 5 | beamcraft-model, beamcraft-engine, beamcraft-host, beamcraft-protocol, beamcraft-layer-engine, beamcraft-ifc | now | U04 amendment: CLI and thin Rust worker binaries; E02 (`export-ifc` adapter) |
| beamcraft-ui | apps/beamcraft-ui | app | 5 | beamcraft-model, beamcraft-protocol, beamcraft-engine, beamcraft-host | now | V03 (window and viewport; reads the accepted snapshot from the CLI in a separate process); V05.1 (create tools; decodes the CLI's JSON replies with the protocol crate); V05.2 (edit tools; same edges) |
| beamcraft-fixtures | tools/fixtures | tool | 9 | - | now | F07 |
| beamcraft-boundaries | apps/boundaries | tool | 9 | - | now | F08 |
<!-- module-table:end -->

No package has a third-party dependency today: `Cargo.lock` holds project-owned packages only.

## 3. Third-party crates allowed in domain crates

A domain crate may use a third-party crate only if it is listed here with the reason, after the
`AGENTS.md` section 5 review (permissive licence, pure Rust, `wasm32-unknown-unknown`, no
network, no OS access). The list is empty: no domain crate has a third-party dependency, and
a test fails if one appears.

<!-- third-party:begin -->
| Crate | Reason |
|-------|--------|
<!-- third-party:end -->

## 4. Where each contract area lives

"Now" means present and tested in this change; "planned" means a path reserved here and created
by the named task with real code. Allowed dependencies are those of the owning row in section 2.

| Contract area | Contract | Exact path | Status |
|---------------|----------|------------|--------|
| Rigid transform, vectors, quaternion, tolerance constants, canonical f64 encoding for keys | F06 s.2, 4, 7 | `beamcraft_geom` (modules `vec`, `quat`, `tolerance`) | planned, G01. Today `geom` holds `Point3` only |
| Kernel input and result, diagnostics, `GEO_` codes and their F04 mapping | F06 s.2, 3 | `beamcraft_kernel::{input, result, diagnostic}` | planned, G01 |
| Analytic box builders, self-checks, cancellation points | F06 s.4, 6, 10 | `beamcraft_kernel::build` | planned, G01 |
| Mesh blob builder, validator, face table, digest rule | F06 s.6 | `beamcraft_kernel::mesh` (plate builder, G02); generic blob/buffer validator `beamcraft_kernel::trimesh` (G06) | planned, G01 |
| FeatureId (`fid1`) | F06 s.5 | `beamcraft_kernel::feature_id` (uses `ElementId` from `model::id`) | planned, G01 |
| Three dependency keys, mass signature, invalidation | F06 s.7 | `beamcraft_kernel::keys` | planned, G01 |
| Quantities and statuses | F06 s.8, F04 s.5 | `beamcraft_kernel::quantity` | planned, G01 |
| Pick and feature geometry for one body | F06 s.9 | `beamcraft_kernel::pick` | planned, G01 |
| Pinned queries (`ListElements`, `GetMeshSnapshot`, `LookupFeature`, `GetChanges`, `QueryBounds`), snapshot tokens | F06 s.9 | `beamcraft_engine::query` | **now, partial**, V02: `ListElements`, mesh blobs by digest, world `QueryBounds`, `GetChanges` over a bounded history, snapshot tokens, selection. `LookupFeature` and `Pick` planned (G04); dependency keys planned. P01 adds `query::committed`: authored plates of a committed revision become kernel-evaluated records and the pinned snapshot input |
| View settings (element-kind filter, representation selector), read-time projection, instance/triangle id tables | architecture "Part representations and view filters", F06 s.9 | `beamcraft_engine::presentation` | **now**, V02 |
| Presentation key preimage | F06 s.7 | `beamcraft_engine::presentation` (V01 may move it to the UI if it needs no engine state; never into the kernel) | planned, V01 |
| Identity (UUIDv7 `ElementId`), units, input limits | F04 s.2, 3 | `beamcraft_model::{id, limits}` | **now**, P01 (`Uuid7`, typed ids, deterministic UUIDv7 generator with injected clock and entropy; F04 section 3 caps) |
| SHA-256 digest type and hasher, for F04 resources, F06 keys and F05 buffers | F04 s.2, F06 s.6, 7, F05 s.3 | `beamcraft_model::digest` | **now**, U05 (full digest, incremental FIPS 180-4 SHA-256) |
| F04 command and envelope types | F04 s.6, 7 | `beamcraft_model::command` | **now, partial**, P01: `CreatePlate` arguments, `Placement`, label/contour/placement admission, typed `CommandError`, argument content fingerprint. The envelope and the other commands are planned |
| F04 stable error codes | F04 s.8 | `beamcraft_model::error::ErrorCode` | **now** (23 codes, tested) |
| Bounded JSON decoder and RFC 8785 canonical bytes of the validated envelope | F04 s.3, F05 s.6 | `beamcraft_model::{json, canonical}` | planned, P01 |
| Command application, footprints, transactions, replay decisions | F04 s.8, F05 s.7 | `beamcraft_engine::{command, footprint, transaction}` | **now, partial**, P01: `CreatePlate` only (origin rectangle), the Create footprint, op_id replay/reuse decision; no canonical F05 bytes |
| Production UUIDv7 source (system clock, per-process OS-seeded entropy; uniqueness, not secrecy) | F04 s.2 | `beamcraft_host::ids` | **now**, P01 |
| P01 adapter: engine command to U02 authoring to U06/U07 commit, and reopen through U01/U03 to the engine query | AGENTS s.3 | `apps/beamcraft-cli` (`beamcraft_cli::plate`, in-process layer engine) | **now**, P01 (experimental; no process isolation) |
| Frame header, binary prefix, bundle budget | F05 s.3, 10 | `beamcraft_protocol::frame` | **now** (tested, bounded) |
| Negotiable limits and their maxima | F05 s.4, 10 | `beamcraft_protocol::limits` | **now** |
| Transport `IPC_` codes and their F04 mapping | F05 s.9 | `beamcraft_protocol::code::TransportCode` | **now** |
| Bounded JSON and closed read-profile messages (pure, no I/O); mutation/event schemas remain unavailable | F05 s.3-7 | `beamcraft_protocol::{json, message}` | U04 amendment: experimental Rust read profile |
| Worker supervision, pipes, deadlines, restart limits, diagnostic drain; shared command dispatcher | F05 s.2, 8 | `beamcraft_host::{wire, worker, dispatch}` | U04 amendment: experimental native read client |
| Immutable byte storage, publish-if-absent and verified retrieval | F04 s.2, U05 | `beamcraft_host::storage` | **now**, U05 |
| Revision manifest, atomic root (head log), expected-base commit, verified reader, commit hook seam | F04 s.2, 6, 8, U06 | `beamcraft_host::revision` | **now**, U06 (experimental; content verifier is injected, host links no layer engine); personal workspace remains planned, U07 |
| Durable outcome ledger (write-ahead intent, committed/aborted records), recovery of an unfinished commit, host-ledger QueryOutcome | F05 s.7, 8, U07 | `beamcraft_host::ledger` | **now**, U07 (experimental; personal workspaces, collaboration, ledger rotation and garbage collection are not implemented) |
| Rust USD read wrapper (no host edge; separate process, owner decision 2026-10-16; process containment still requires implementation) | F11 s.9, LEW-01/02/03/04/07 | `beamcraft_layer_engine` | experimental native read slice, U01 |
| USD worker (thin Python/pxr) | F05 s.1 | `workers/usd/`, not a Cargo package | planned, U01 |
| USD worker (thin Rust wrapper, separate process; no new package) | F11 s.9, U04 owner topology decision | `apps/beamcraft-cli/src/bin/beamcraft-usd-worker.rs` | U04 amendment: experimental read worker; SDK fallback remains unavailable |
| CLI | AGENTS s.3, F06 s.9 | `apps/beamcraft-cli` | U04 amendment: version, experimental open/read and byte-storage commands; U06: `revision-commit`, `revision-open`; V02: `view-snapshot` and `view-changes` over a synthetic kernel plate scene; P01: `create-plate` and `plate-inspect` over a store (the CLI package also has a library target, `beamcraft_cli`, for its tests) |
| Read-only foreign reference: source pin, units interpretation, attach/move/correct commands, write refusal; wiring of file read, layer engine and host | F04 s.1, F10 `reference`, LEW-01/04 | `beamcraft_model::reference`, `beamcraft_engine::reference`, `beamcraft_layer_engine::authored` (`Change::AttachReference`, `read_references`, `inspect_foreign_layer`), `apps/beamcraft-cli` (`beamcraft_cli::{reference, source_file}`) | **now**, R01 (experimental; single self-contained layer, in-process layer engine; no new crate and no new edge) |
| Numbering preview: grouping rules, series, overlaps, staleness token | owner numbering notes (outside the repository), A08 | `beamcraft_engine::numbering` (rules), `beamcraft_cli::numbering` (adapter), `numbering-preview` | **now**, A08 (experimental; read-only, no stored numbers) |
| IFC4 export: GlobalId, STEP lexical rules, entity templates, bounded sink, structural reader for tests | E01 s.3, 11, 15 | `beamcraft_ifc::{guid, step, model, verify}` and `write_ifc` | **now**, E02 (experimental; std only, no I/O of its own; `verify` is a test harness, not an importer) |
| Export adapter: committed revision to `ExportModel`, report, atomic output | E01 s.4, 8, 13, 15.4 | `apps/beamcraft-cli` (`beamcraft_cli::export`), `beamcraft_host::output` | **now**, E02 (the contract proposed `beamcraft_engine::export`; the adapter sits with the other P/A-slice adapters and the engine gains no edge to `beamcraft-ifc`) |
| Native UI and viewport | F06 s.9, V-tasks | `apps/beamcraft-ui` | **now, partial**, V03: window, orbit camera, instanced offscreen viewport of the accepted snapshot (received as `beamcraft_engine::query::bundle` from `beamcraft view-export`), headless baseline metrics. Selection, snapping and tools are V04/V05 |

The F05 sections 3 and 10 also require SHA-256 verification of attachments. The protocol crate
checks the declared lengths, counts, indices and prefix fields and exposes the claimed digest; the
streaming digest comparison can now use `model::digest`; protocol session integration remains U01.

## 5. Crates created now, and why

- **`beamcraft-protocol` (`crates/protocol`, native library).** Created because it holds real, tested code
  taken directly from F05 sections 3, 4, 9 and 10: the 16-byte header and 44-byte binary prefix
  with bounded encode and decode, the 14 negotiable limits with their maxima and the
  per-field minimum, a bundle size budget and the 16 transport codes with their F04 mapping. It
  depends on `model` only for `ErrorCode`, and has no I/O. Every decoder validates before it
  trusts a length, uses checked arithmetic, never slices by an input-derived position and has a
  regression test for each rejected input class.
- **`beamcraft_model::error::ErrorCode`** (a module in the existing crate, not a new crate): the
  23 stable codes of F04 section 8, shared by F05 and, later, F06.
- **`beamcraft_model::digest`**, added by U05: dependency-free, incremental SHA-256 with
  a bounded length counter, full lowercase `ContentDigest` and independent vector tests.
- **`beamcraft-host` (`crates/host`, native host)**, created by U05 with real storage code:
  `BlobStore` publishes full digest filenames without replacement, compares complete existing
  bytes and verifies every read. Only `model` is used today; allowed edges are unchanged.
  The F05 32 MiB bundle budget bounds each blob and returned buffer. The root namespace must
  be protected from hostile concurrent filesystem mutation: safe portable std containment checks
  cannot close directory replacement races. See the storage module's documented platform limits.
- **`beamcraft-engine` (`crates/engine`, native library)**, created by V02 with the real, tested
  view snapshot and query contract: a pinned `ViewSnapshot` built from sealed kernel plate
  results, snapshot tokens, `GetChanges` deltas, view settings (kind filter and representation
  selector) applied as a read-time projection, ids/instances/triangles tables and a selection API.
  It depends on `geom`, `model` and `kernel`, adds no third-party dependency and has no UI,
  renderer or I/O code. Command application, authored elements, dependency keys, `LookupFeature`
  and `Pick` remain planned (G04). P01 added `CreatePlate` application, footprints, the op_id
  transaction decision and the committed-plate queries; `ElementId` now comes from
  `beamcraft_model::id` (re-exported by `beamcraft_engine::element`).
- **`beamcraft-boundaries` (`apps/boundaries`, tool).** Created because it holds the real, tested
  checker of section 6. It is a development tool with no dependencies; it is not part of the
  product.

**`beamcraft-kernel` (`crates/kernel`, domain)** is created by G02 with the tested
dimension-based rectangular plate builder, bounded staged diagnostics, local feature roles,
self-checks and fixed BCMESH01 encoding. It depends only on `geom` and `model`.
G06 adds the generic BCMESH01 blob and buffer validator (`trimesh`), shell topology
and orientation certification (`topology`), mesh measurements and F06 section 8 status
helpers (`measure`), a bounded box index (`spatial`) and their typed verdicts
(`verdict`, `enclosure`), still inside this crate and without new dependencies.
G09 adds the bounded pilot `ops` module (plane cut and through hole over certified solids,
with typed refusals); it is not a general boolean kernel, and the A03/A04 commands, keys and
queries stay with their tasks.
The full contour/placement request adapter, keys, engine-level quantity
integration and the `LookupFeature`/`Pick` queries remain planned under the section 4
paths.

V03 created `apps/beamcraft-ui` with its first real code (window, camera, renderer, snapshot
loading). It does not depend on `beamcraft-cli` (an app may not depend on an app): it runs the
`beamcraft` binary as a separate process and rebuilds the snapshot with the engine.

## 6. Mechanical enforcement

`cargo test --workspace` runs the tests of `apps/boundaries`; CI also runs
`cargo run -p beamcraft-boundaries -- check`. The checks read the workspace and member
`Cargo.toml` files and this document, using only the standard library and no network:

1. every workspace member has a `now` row with the same path, and every `now` row is a member;
2. package names and paths in the table are unique, every dependency named in a row exists in the
   table and has a lower layer;
3. every internal dependency of a member (in any dependency table) is listed in its row. A
   dependency is checked by its real package name: `package = "..."` is resolved whether written
   inline, dotted (`x.package = "p"`, `x.version = "1"`) or in a sub-table, and an inherited
   `workspace = true` entry is resolved against the root `[workspace.dependencies]`;
4. a `domain` crate depends on no `native`, `host`, `app` or `tool` crate, and on no third-party crate that
   is not in section 3;
5. no package depends on an `app` or a `tool`, and `tool` packages set `publish = false`;
6. source of a `domain` crate does not mention `std::fs`, `std::process`, `std::net`,
   `std::thread`, `std::env` or `std::os` (also inside a `use std::{...}` group, including
   `use std::{fs as f}`). Comments and string literals are ignored, and only items annotated
   `#[cfg(test)]` are exempt, wherever they are declared. An inner `#![cfg(test)]` exempts the
   whole file only at file level before any item; inside `mod tests { ... }` it exempts only that
   module. Code placed after a test module is still scanned;
7. every CI job that mentions `wasm32-unknown-unknown` calls
   `cargo run -p beamcraft-boundaries -- portable` (the three `now` `domain` rows) and selects no
   package by hand: continuation lines are joined, and any `-p`, `-pNAME` or `--package`
   argument other than the tool's own `cargo run -p beamcraft-boundaries` is rejected, whether
   in a variable assignment, a continuation or a separate run line;
8. the loader fails closed: a symbolic link or Windows reparse point at a member, along its path,
   at its manifest or inside its `src`, a manifest that is not a regular file, and a source
   directory nested deeper than eight levels are errors, never silently skipped.

Known limits: the scan is lexical, so a `#[cfg_attr(...)]`-generated or macro-generated path is
not seen, and an alias such as `use std as s;` is not followed. The manifest reader is line
based: a multi-line inline table is not read, and an inherited key that the root does not define
keeps its key as the name (Cargo itself rejects such a manifest). The wasm check and code review
cover what a lexical scan cannot.

The failing paths are proved by tests on an in-memory model (a synthetic manifest set), so no
broken manifest is committed. Under the WS amendment, the WASM check lists exactly
`beamcraft-geom`, `beamcraft-model` and `beamcraft-kernel`. Tests prove that a domain crate
cannot acquire a native-only dependency and that desktop-native crates may use native APIs and
audited dependencies without joining the WASM list. Real-workspace and CLI tests assert the
exact list so an accidental scope expansion fails.

## 7. Amendments proposed to other contracts

WS proposes a separate F06 section 4 clarification: target conformance, including `wasm32`,
applies to geom/model/kernel only; engine/query/viewport consumers are desktop-native. The
frozen F06 text is unchanged. Two earlier clarifications remain proposed for F05:

- F05 section 11, row F08: the "canonical-byte comparison" obligation is placed at
  `beamcraft_model::canonical` (pure) with the host as its only caller; it is not implemented yet.
- F05 section 12, open item "U02/F08 must publish the schema-specific change and snapshot item
  fields": F08 leaves this to U02. The message schemas will live in
  `beamcraft_protocol::message`, and the item fields in `beamcraft_engine`, not in the host.

## 8. Provenance

Contributor-original BeamCraft text, MPL-2.0, derived from F04, F05, F06 and the repository
[architecture](../architecture.md) and [development guide](../development.md). No third-party
material is used. The code it describes is original and implements only byte layouts, numbers and
code tables stated in F04 section 8 and F05 sections 3, 4, 9 and 10, plus U05 byte storage.
The original SHA-256 implementation follows public FIPS PUB 180-4 (August 2015), sections
4.1.2, 4.2.2, 5 and 6.2 (https://doi.org/10.6028/NIST.FIPS.180-4); no implementation was copied.
Padding-boundary expected digests were independently computed with Windows .NET SHA256.
The separate fixture-tool SHA-256 remains untouched; consolidating it is outside U05.
