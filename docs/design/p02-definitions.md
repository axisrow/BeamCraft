# P02 profile and material definitions

Date: 2026-10-17. Status: **experimental vertical slice; P02 acceptance is limited as listed below.**
Branch `feat/p02-definitions`, based on `origin/master` at `3dc1f47` (P01 and U06 merged).

Plan row: P02 / M, depends on P01 and U05. Result: a synthetic parametric profile and an
engineering material as immutable USD assets, referenced by pinned digest. Acceptance: the material
is separate from any shader; digest and units are pinned; an unknown density stays unknown.

## What is implemented

```text
beamcraft define-material --store ROOT [--density KG_M3]
beamcraft define-profile  --store ROOT --width MM --height MM
  model::definition (F04 caps) --> layer engine Resource (canonical definition layer, SHA-256)
  --> U02 resource-only batch --> authored.usda: /World/Definitions/D_<sha256>
  --> host U06/U07 commit with the verifier (index, structural check, definition re-hash, plates)
beamcraft create-plate ... --material-digest SHA256
  engine resolves the pinned digest (NOT_FOUND / INVALID_ARGUMENT) --> mass = volume * density
beamcraft definitions --store ROOT   (fresh process: verified head, every definition re-admitted)
```

| Area | What exists |
|---|---|
| `beamcraft_model::definition` | `MaterialSpec` and `RectangleProfileSpec`: F04 section 3 admission with stable codes (finite, density at most 1,000,000 kg/m3 and above zero, section dimensions from the 0.001 mm floor to 1,000,000 mm). |
| `beamcraft_model::command` | `CreatePlate.material_digest` is admitted (the P01 `UNSUPPORTED_PROPERTY` refusal is gone) and is part of the content fingerprint, so another material under one `op_id` is `OP_ID_REUSE`. |
| `beamcraft_engine` | `MaterialInfo`, `MaterialLookup`, `no_materials`; `AuthoredPlate.material`; `Quantities.{mass_kg, density_kg_m3}`; mass by the kernel's `analytic_mass_kg` (`fl(fl(V_mm3 * density) / 1e9)`, F06 section 2). The density is re-admitted from the committed layer before it makes a mass. `Footprint::for_create` reads the pinned definition and never writes it; `Footprint::for_define` writes only the definition. |
| `beamcraft_layer_engine::authored` | A batch of resources only is accepted (an empty batch is still rejected), each newly written definition is a declared write of its digest, existing definitions are never rewritten. |
| `beamcraft_cli::definition` | `define` (admit, resource, head read, idempotency by digest, author, footprint check, commit), `decode_definitions` (path digest, `bim:contentDigest` label, units, version, values and the recomputed digest must agree), `material_lookup`. |
| `beamcraft_cli::plate` | The verifier and `inspect` now resolve materials from the committed definitions; `load_base`/`publish_revision` are shared by `create_plate` and `define`. |

## Decisions

1. **Definitions are published by an adapter operation, not an F04 command.** F04 section 7 lists
   no definition command and the U02 contract calls the resources contributor/engine-supplied.
   Identity is the content digest, so publication is idempotent without an `op_id` replay record:
   `definition_exists` means the head already held the same bytes and no revision was written. If
   the owner wants a logical `DefineMaterial` command, it needs an F04 amendment.
2. **The digest is the layer engine's.** It is the SHA-256 of the canonical isolated definition
   layer (`Resource::bytes`), which includes units `mm`, schema version `0.1.0`, kind and values
   (and, since A07.2, the optional display name).
   Changing any of them is a different identity. `bim:contentDigest` in the prim is only a label
   that the reader checks against the path and the recomputed bytes.
3. **Unknown stays unknown.** A material published without `--density` has no density attribute.
   A plate that pins it, or no material at all, reports mass `unknown` with no value.
4. **Physical material only.** The layers contain no shader, colour or binding; the test checks the
   saved text. The display reference and its signature are a separate later reference (F04
   section 4).
5. **Profiles are published and verified, not consumed.** Only `CreatePlate` exists, and a plate
   needs no profile (F04 section 4). The straight member (P03) will pin `bim:profile` by the same
   digest. A digest that names a profile where a material is required is `INVALID_ARGUMENT`.
6. **Resolution order.** Arguments, duplicate id, then the pinned material (so a missing definition
   is `NOT_FOUND` before any geometry runs), then geometry (F04 section 8).

## Not done (not claimed)

* Mass is returned and listed, **not stored in USD**: the U02 schema still has no quantity
  attributes, and there is no signature (P04 owns invalidation and signatures).
* No logical `DefineMaterial`/`DefineProfile` F04 command and no JSON envelope; no replay record.
* No profile consumer (P03), no catalogue or designation lookup, no environment default.
* Updating or clearing the material of an existing plate (`UpdateElement`, P04) is not implemented.
* The layer engine runs in the CLI process (LEW-03 containment open), as in P01.
* A definition-only revision is a new generation of the implicit personal workspace; workspaces,
  drafts and undo (P06, P07) do not exist.

## Evidence

* `cargo test -p beamcraft-cli --test cli -- definitions::` (real binary, six tests): created and existing
  definitions, the digest equal to the layer engine's `Resource` digest, closed-form mass
  `0.005 m3 * 7850 kg/m3 = 39.25 kg` and `13.5 kg` at 2700, unknown density with no value, the same
  mesh digest for every material, F04 codes for hostile densities and sizes, `NOT_FOUND` and
  `INVALID_ARGUMENT` for wrong digests with no revision written, replay and `OP_ID_REUSE` with a
  material, fresh-process `inspect-elements` and `definitions`, and plain-text checks of the saved
  layer (units, version, kind, density, the `bim:material` link, no shader).
* `beamcraft_cli` unit tests: a definition whose stored density, units, label or kind differs from
  its path digest is `INTEGRITY_ERROR`; a hostile density is `LIMIT_EXCEEDED`; unknown kinds and
  descendant or upper-case paths stay inert.
* pxr oracle (`tests/pxr_oracle.rs`, local and opt-in): `usd-core` 25.11 in CPython 3.11 reads
  the definitions, the `bim:material` target, units, version, kind, label and density, derives the
  mass from its own mesh volume and the stored density, and `UsdUtils.ComplianceChecker` reports no
  errors, failed checks or warnings for a revision with two materials, a profile and three plates.
  CI has no pxr: a skipped run is not evidence.
* Engine, model and layer-engine unit and integration tests for admission, footprints (a
  publication conflicts with a plate that pins the digest, never with another), the resource-only
  batch and the mass rules.
