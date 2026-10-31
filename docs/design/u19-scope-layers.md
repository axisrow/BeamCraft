# U19: authored section and zone scopes, the active section

Date: 2026-11-01. Status: **delivered slice; the F12 registries, the 0.2 record wiring and
the store commands remain U20/#364**. Branch: `ao/beamcraft-169/feat/u19-scope-layers`;
fork issue #363 (upstream #230, parent epic #189). Contracts:
[f10-domain-vocabulary-v0.2](../contracts/f10-domain-vocabulary-v0.2.md) (DV-SCOPE-001..004),
[f12-project-coordination-v0.1](../contracts/f12-project-coordination-v0.1.md) section 3.

## What this changes

A project can now be created **sectioned**: `create-project --code 2401-DEMO --sections
KM,K1:KMD,K2:KMD` stores one authored layer per scope, named
`<code>[_<zone>]_<section>.usda` after the architecture's layer plan, plus the shared
`<code>_Grids.usda` (grids and levels) and the one derived layer. The root composes the
authored scope layers first and the derived layer last. Classic stores (the single
`authored.usda`) are untouched and never gain authority from their names (DV-SCOPE-005).

A workspace is authorized **once**, at `create-workspace --scope-code --scope-section
[--scope-zone] [--scope-shared true]`: one active section, one zone partition (element
creation needs one unambiguous write layer; a wider set is refused until element commands
name their zone), and optionally the separately authorized shared grids/levels scope. The
authorization is recorded in `workspace.json` (format v3) and validated against the
store's layout — both the scope's layer and the shared layer must already participate.

Every edit command derives its routing again from trusted inputs (the verified manifest
head and the workspace metadata), never from a client claim:

- `beamcraft_engine::scope` — the pure vocabulary: code rules (ASCII alphanumerics and
  `-`, so a code can enter a file name), the `<code>[_<zone>]_<section>.usda` composition
  and `WorkspaceScope`.
- `beamcraft_layer_engine::authored::EditScope` — the admitted `{write_layer, allowed,
  shared, shared_authorized}` of one authoring session, pinned to the edit target at
  `Author::open`. The admission check in `validate::changes` refuses (`DV-SCOPE-002`)
  any change to an element whose strongest provenance lies outside `allowed` — a stronger
  opinion over another section's prim is still an edit — treats unknown provenance as no
  permission, and answers common layers and the attribute schema only to authorized shared
  commands (`DV-SCOPE-004`). Creates land in `write_layer`, which must be the edit target.
- Publication (`publish-workspace`) re-derives the scope from the manifests: the unsent
  work may change only the authorized layers, the derived layer, external sources and the
  root's units/schema pins; the root's `subLayers` list must be byte-identical
  (`DV-SCOPE-001`/`DV-SCOPE-003`). Rebase and compensation materialize into the
  authorized layer.

Both proof projects author KM **and** KMD through the same steel module: one shared store
(`2401-DEMO`, sections KM, K1:KMD, K2:KMD) and one independent KMD-only store
(`2402-DEMO`). `tests/u19_scopes.rs` runs DV-SCOPE-001..004 positive and negative
end to end; the layer engine carries the same rules as unit cases.

## The shared-scope grant handle (for T01.5)

The handle T01.5 needs exists and is explicit: `create-workspace --scope-shared true` is
the trusted-context grant of the shared grids/levels scope; grid and level commands route
through it, and a workspace without the grant is refused with `PERMISSION_DENIED` /
`DV-SCOPE-004`. Deliberate limits of this slice, recorded so they are decisions and not
holes:

- The grant is given only at workspace creation; there is no command to widen, narrow or
  revoke a scope yet (a scope change means a new workspace). T01.5's rights work can add
  the explicit grant/revoke command over the same recorded shape.
- One shared layer per project (`<code>_Grids.usda`, unzoned). Per-zone grids layers
  (`<code>_<zone>_Grids.usda`) are a layout extension the naming rule already admits; the
  routing and the shared authorization then become per-layer instead of per-project.
- Read-only pinned coordination grids are coordination-project composition (U21/#364);
  this slice scopes only the project's own store, where foreign content is already
  `FOREIGN_READ_ONLY` by ownership.

## Deliberate limits

- Codes are labels (`2401-DEMO`, `KM`, `K1`): the section and zone registries with their
  UUIDv7 identities are U20/#364 on top of U22's `SectionRecord`/`ZoneRecord`; the
  workspace metadata carries the code until then, and the layer-participation check is
  what makes a mislabelled scope unusable rather than authoritative.
- Definitions (materials, profiles) and user-attribute schemas are shared data: on a
  sectioned store both are refused this slice (`--environment` seeding with them), until
  the shared definition scope arrives with the registry work. Members of a sectioned
  project are created without pinned definitions, as plates are.
- A workspace with an authorized scope cannot have it changed; a sectioned store cannot be
  edited without a workspace. Both refusals are `PERMISSION_DENIED` / `DV-SCOPE-001`.
