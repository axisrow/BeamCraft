# P09 validation layer

Date: 2026-11-03. Base: `a5089be0` (verified `origin/master`). Branch
`ao/beamcraft-195/feat/p09-validation`. Status: acceptance evidence for the shared validator;
the validator itself is existing delivered code and was not duplicated.

## One validator, two doors

The task's "one validator (dangling links, duplicates, module rules) for client and post-merge"
is one rule stack enforced at two doors. It already existed in code at the audited base; the
remaining P09 work (upstream #494 audit) was the complete rule set and its acceptance.

The rule stack, in enforcement order:

1. **Backend open** (`layer-engine/src/backend.rs`): bounded read; every `bim:id` prim must
   already satisfy path ≡ identity (`DV-BASE-001`); reserved leaf shapes; provenance capture.
2. **Structural snapshot** (`layer-engine/src/authored/validate.rs::snapshot`): the
   `u02-structural-v1` profile (`U02-V01`–`V06`) plus the vocabulary rules `A01-V01` (part-of
   kinds, cardinality, self-link, cycles), `A16-V01` (main part), `A06-V01` (weld),
   `A05-V01` (bolt group), `D2-V01` (levels), `T01-V01` (grids), with `DV-REL-001`–`007` rule
   ids, A09 marks, I04 attribute schema and the derived-mesh rules (`U02-V06`).
3. **Identity index** (`layer-engine/src/index.rs`): duplicate identities (`DV-BASE-002`),
   generic dangling relationship targets (`DV-REL-001`), short/property-path targets,
   provenance and completeness (`DEPENDENCY_UNAVAILABLE`, `INTEGRITY_ERROR`).
4. **Module re-admission and derived verification** (`crates/steel/src/layer/read.rs`,
   `commands::plate::decode`/`evaluate_decoded`/`verify_derived`): steel records (welds, bolt
   groups, holes, cuts, corners, marks, quantities) re-admitted from the stored prims, and the
   committed derived layer proven to match a fresh evaluation.

The two doors run this same stack:

- **Client door** — every authoring command opens the live model through the snapshot,
  admits its batch through the change rules (`input_bounds`, normalized duplicate keys,
  `validate::changes`, scope), and checks relationship preconditions such as
  `Author::check_delete`; the prepared candidate is validated as a whole before publication.
- **Post-merge door** — every commit and every `publish-workspace`/`apply-changeset` runs the
  host's `ContentVerifier` (`commands::plate::PlateVerifier`): `open_validated` (steps 1–2 over
  the complete merged layer set), `element_index` + `verify_index` (step 3), then `decode`,
  `evaluate_decoded` and `verify_derived` (step 4). The client's own commit passes through this
  door too, so a rule added to the stack applies to both (see the `validated` doc comment in
  `authored.rs`).

The read door (U17) is separate by design: bytes committed through the raw transport command
`commit-revision` carry no content verification, and every reading command refuses them
(`hostile_usd.rs`). The post-merge door catches them at the next verified merge.

## Acceptance: each rule has a positive and a negative case

Door-level evidence is `apps/beamcraft-cli/tests/validation_layer.rs` (real binary, both
doors); the per-rule engine-level evidence lives beside the rules.

| Rule family | Positive | Negative |
|---|---|---|
| Identity (`DV-BASE-001`) | `validation_layer::positive_model_passes_both_doors`; `layer_index.rs::reader_index_resolves_ids_provenance_and_typed_relationships` | `layer_index.rs::ids_are_full_uuidv7_and_same_millisecond_ids_are_never_shortened`, `layer_index.rs::duplicate_malformed_misplaced_and_mismatched_identities_are_rejected`; post-merge duplicate identity → `INVALID_ID` (`validation_layer::duplicates_are_refused_by_both_doors`) |
| Duplicates (`DV-BASE-002`) | positive model; `layer_authored.rs::twenty_seeded_permutations_produce_identical_owned_bytes_and_digests` | client create with an existing id → `DUPLICATE_ID` (`validation_layer::duplicates_are_refused_by_both_doors`); duplicate change key and index duplicate: `layer_authored.rs::duplicate_keys_wrong_kind_bad_mesh_and_missing_or_forged_definitions_fail`, `layer_index.rs::duplicate_malformed_misplaced_and_mismatched_identities_are_rejected` |
| Dangling links (`DV-REL-001`) | positive model; `layer_index.rs::reader_index_resolves_ids_provenance_and_typed_relationships` | client: delete of a welded or level-assigned part, attach to an absent assembly → `DEPENDENCY_CONFLICT`/`NOT_FOUND`; post-merge: absent part-of target → `DANGLING_RELATIONSHIP` "part-of target unavailable" (`validation_layer::dangling_links_are_refused_by_both_doors`); index: `layer_index.rs::dangling_short_and_unsupported_targets_of_owned_prims_are_rejected` |
| Kind, cardinality, cycles (`DV-REL-003`–`007`) | positive model (weld, level assignment, nested assembly) | same-part weld, weld to an assembly, assembly on a level → refusals; part-of cycle → `DEPENDENCY_CONFLICT` (`validation_layer::module_rules_are_refused_by_the_client_door`); engine: `engine/src/assembly.rs::kinds_cardinality_self_links_and_cycles_are_refused`; snapshot: `part_of`/`main_part`/`level_links`/`connects` negatives in `layer_authored.rs` (`wrong_weld_inputs_targets_and_loaded_records_are_refused`, vocabulary tests) |
| Structural profile (`U02-V01`–`V06`) | `layer_authored.rs::author_install_reopen_standard_mesh_relationships_and_exact_unknown_opinions` and the positive halves of the U02 suite | `layer_authored.rs::mismatched_schema_pin_corruption_and_nonfinite_never_change_live_state`, `layer_authored.rs::validator_rejects_schema_markers_changed_definition_bytes_and_loaded_bad_mesh`, `layer_authored.rs::duplicate_keys_wrong_kind_bad_mesh_and_missing_or_forged_definitions_fail` |
| Vocabulary rules (`A01`/`A16`/`A06`/`A05`/`D2`/`T01`-`V01`) | positive model; package suites `assemblies.rs`, `welds.rs`, `bolt_groups.rs`, `levels.rs`, `grids.rs` | each package suite's refusal cases; snapshot negatives in `layer_authored.rs` |
| Module re-admission and derived truth | `plate.rs` P01 end to end; `verify_derived` strict on every commit | `hostile_usd.rs` (stable codes, never a panic or a hang); steel record refusals in `crates/steel/tests/` |
| Bounds and budgets | positive model | label beyond the 256-byte bound → `INVALID_ARGUMENT` (`validation_layer::violations_are_deterministic_and_bounded`); `layer_index.rs::index_budgets_and_cancellation_fail_closed`; batch caps in `validate::input_bounds` |
| Determinism | `twenty_seeded_permutations_...` (authoring); index resolves in fixed path order regardless of delivery order | same dangling record with the prim order permuted → identical code and message (`validation_layer::violations_are_deterministic_and_bounded`); `layer_index.rs::tampered_pages_cannot_forge_identity_provenance_or_bounds` |

## Limits

- The F10 rule-id propagation gaps (refusals that carry no `DV-` id yet, such as some
  `input_bounds` paths and the CLI domain-diagnostic adapter's requirements) remain tracked in
  upstream #550 and are deliberately untouched here.
- `commit-revision` stays a raw transport door (U17): it verifies blob digests only, and the
  post-merge door plus the read door refuse violations it may have let in.
- The post-merge duplicate-identity shape is refused by the path ≡ identity rule of the
  backend-open step (`INVALID_ID`); the index's own `DUPLICATE_ID` verdict guards the direct
  page-delivery door, because two composed prims with one identity cannot both satisfy
  path ≡ identity.
