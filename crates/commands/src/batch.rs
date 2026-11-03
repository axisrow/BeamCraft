// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The single commit of a draft batch (`I424`): the session calls it at `batch-commit` and it
//! publishes the draft's accumulated layer set as one revision through U06/U07 — the batch's
//! own operation id as the retry key, the union footprint plus one binding per carried
//! command as the pinned evidence, and the base pinned at `batch-open` as the expected root.
//! A moved head, a budget overrun or any refusal fails the batch whole; nothing partial is
//! ever published (`F04`).

use session::Refusal;
use session::draft::{self, Draft, DraftCommand};
use session::pipeline::{ROOT_LAYER, digest_of, host_uuid};

use crate::publication::commit_verified_with_evidence;
use beamcraft_host::revision::{
    CommitHook, Committed, LayerEntry, LayerRole, RevisionDraft, RevisionStore, Uuid7,
};
use beamcraft_layer_engine::authored::SchemaIdentity;
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::RevisionId;

/// The digest that identifies one batch attempt's request: a digest over the carried
/// commands' operation ids and request fingerprints, in batch order. An identical command
/// list under the same batch operation id reproduces it, so the whole batch replays by its
/// own id; any different content under a committed batch id is `OP_ID_REUSE`.
pub fn batch_fingerprint(commands: &[DraftCommand]) -> Result<ContentDigest, Refusal> {
    let mut text = Vec::new();
    text.extend_from_slice(b"beamcraft-batch-request 1");
    for command in commands {
        text.extend_from_slice(command.op.to_string().as_bytes());
        text.extend_from_slice(command.fingerprint.as_bytes());
    }
    digest_of(&text)
}

/// What the session passes to [`commit_batch`].
pub struct BatchCommit<'a> {
    /// The store the batch publishes into.
    pub store: &'a RevisionStore,
    /// The batch operation id: the whole batch's retry key.
    pub op: Uuid7,
    /// The revision id the batch revision takes.
    pub revision_id: &'a RevisionId,
    /// The admitted schema identity, as any command carries.
    pub schema: SchemaIdentity,
    /// Project and workspace of the pinned base.
    pub project: &'a str,
    /// Workspace the batch revision publishes under.
    pub workspace: &'a str,
    /// Workspace generation of the pinned base.
    pub base_generation: u64,
}

/// What one batch commit produced.
#[derive(Debug)]
pub struct BatchCommitted {
    /// The committed (or replayed) batch revision.
    pub committed: Committed,
    /// How many commands the batch carried.
    pub commands: usize,
}

/// Ends the open draft and publishes it as one revision. The commit checks the base pinned
/// at `batch-open` exactly as any U06 commit does, so a head that moved mid-batch fails the
/// whole batch instead of rebasing it silently. An empty draft is refused.
///
/// The carried commands stay replayable without their own ledger records: the batch
/// revision's evidence carries one binding per command (operation id to request
/// fingerprint), and the command-level replay lookups consult those bindings.
pub fn commit_batch(
    batch: BatchCommit<'_>,
    hook: &mut dyn CommitHook,
) -> Result<BatchCommitted, Refusal> {
    let state = draft::take()
        .ok_or_else(|| Refusal::new(ErrorCode::InvalidArgument, "no draft batch is open"))?;
    if state.commands.is_empty() {
        return Err(Refusal::new(
            ErrorCode::InvalidArgument,
            "the batch holds no commands",
        ));
    }
    let committed = publish_state(&batch, &state, hook)?;
    Ok(BatchCommitted {
        commands: state.commands.len(),
        committed,
    })
}

/// Publishes the draft's accumulated state as the batch revision.
fn publish_state(
    batch: &BatchCommit<'_>,
    state: &Draft,
    hook: &mut dyn CommitHook,
) -> Result<Committed, Refusal> {
    let fingerprint = batch_fingerprint(&state.commands)?;
    let bindings = state
        .commands
        .iter()
        .map(|command| (command.op, command.fingerprint))
        .collect::<Vec<_>>();
    let pin = crate::command_evidence::store(
        batch.store,
        &batch.op,
        fingerprint,
        &state.union,
        &bindings,
    )?;
    // The draft carried the layer bytes in memory only; publication stores them now, at the
    // one commit, never between the brackets.
    for layer in &state.layers {
        batch
            .store
            .blobs()
            .publish(&layer.bytes)
            .map_err(|e| Refusal::new(e.code(), "layer blob could not be stored"))?;
    }
    // The draft's layer set mixes owned layers with the base's foreign sources; the external
    // names of the pinned base tell them apart.
    let entries = state
        .layers
        .iter()
        .map(|layer| {
            let role = if state.base.external.contains(&layer.name) {
                LayerRole::External
            } else {
                role_of(&layer.name)
            };
            LayerEntry::from_bytes(&layer.name, role, &layer.bytes)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let generation = batch.base_generation.checked_add(1).ok_or(Refusal::new(
        ErrorCode::LimitExceeded,
        "generation exhausted",
    ))?;
    let draft = RevisionDraft {
        project: host_uuid(batch.project)?,
        workspace: host_uuid(batch.workspace)?,
        revision: host_uuid(&batch.revision_id.to_string())?,
        op: batch.op.clone(),
        workspace_generation: generation,
        schema: batch.schema.digest,
        root_layer: ROOT_LAYER.into(),
        entries,
    };
    let expected = state.base.expected.clone();
    commit_verified_with_evidence(batch.store, &draft, &expected, Some(pin), hook)
}

fn role_of(name: &str) -> LayerRole {
    if name == session::pipeline::DERIVED_LAYER {
        LayerRole::Derived
    } else {
        LayerRole::Authored
    }
}

/// Whether a command route may run inside an open draft batch: the modelling, definition and
/// The workspace words of the modelling route commit through the direct materializer funnel
/// (fork, publish, undo, redo), not through `publish_revision`, so admitting them between
/// the brackets would let them write the store behind the draft's back. They are refused by
/// name whatever their route says.
const WORKSPACE_WORDS: &[&str] = &[
    "create-workspace",
    "publish-workspace",
    "undo-workspace",
    "redo-workspace",
    "inspect-workspace",
    "inspect-workspace-history",
    "workspace",
];

/// Whether a command may run inside an open draft batch: the modelling, definition and
/// construction routes whose write publications route through the draft, with the workspace
/// words excluded by name — they commit directly and would escape the batch. Everything
/// else (store-level revisions, workspaces, compaction, exports) is refused between the
/// brackets, so no command can escape a batch by writing the store behind the draft's back.
/// Reads of these routes answer from the committed store; a read never sees the draft.
pub fn batchable(route: crate::registry::Route, word: &str) -> bool {
    use crate::registry::Route;
    if WORKSPACE_WORDS.contains(&word) {
        return false;
    }
    matches!(
        route,
        Route::Plate
            | Route::WorkPlane
            | Route::Grid
            | Route::Level
            | Route::Construction
            | Route::Weld
            | Route::Bolt
            | Route::Attribute
            | Route::Annotation
            | Route::Assembly
            | Route::Library
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

    use super::*;
    use crate::plate::{CreateRequest, create_plate, inspect};
    use beamcraft_engine::footprint::Footprint;
    use beamcraft_host::ids::IdSource;
    use beamcraft_host::revision::{NoFault, RevisionStore as Store, Uuid7};
    use beamcraft_host::storage::BlobStore;
    use beamcraft_layer_engine::authored::schema_identity;
    use beamcraft_model::command::Placement;
    use beamcraft_model::id::{ElementId, OpId, ProjectId, RevisionId, WorkspaceId};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};
    use steel::model::command::CreatePlate;

    static NEXT: AtomicU32 = AtomicU32::new(0);

    fn command(op: &str, fingerprint: &str) -> DraftCommand {
        DraftCommand {
            op: OpId::parse(op).unwrap(),
            fingerprint: fingerprint.parse().unwrap(),
            footprint: Footprint::default(),
        }
    }

    #[test]
    fn workspace_words_never_admit_even_on_a_modelling_route() {
        use crate::registry::Route;
        assert!(batchable(Route::Plate, "create-plate"));
        assert!(!batchable(Route::Plate, "publish-workspace"));
        assert!(!batchable(Route::Plate, "undo-workspace"));
        assert!(!batchable(Route::Plate, "create-workspace"));
        assert!(!batchable(Route::Plate, "inspect-workspace"));
        assert!(!batchable(Route::Coordinator, "create-plate"));
    }

    #[test]
    fn the_batch_fingerprint_follows_the_command_list() {
        let one = batch_fingerprint(&[command(
            "01900000-0000-7000-8000-00000000000a",
            "0100000000000000000000000000000000000000000000000000000000000000",
        )])
        .unwrap();
        let same = batch_fingerprint(&[command(
            "01900000-0000-7000-8000-00000000000a",
            "0100000000000000000000000000000000000000000000000000000000000000",
        )])
        .unwrap();
        let other = batch_fingerprint(&[command(
            "01900000-0000-7000-8000-00000000000a",
            "0200000000000000000000000000000000000000000000000000000000000000",
        )])
        .unwrap();
        assert_eq!(one, same);
        assert_ne!(one, other);
    }

    #[test]
    fn an_empty_list_never_matches_a_command_list() {
        let empty = batch_fingerprint(&[]).unwrap();
        let one = batch_fingerprint(&[command(
            "01900000-0000-7000-8000-00000000000a",
            "0100000000000000000000000000000000000000000000000000000000000000",
        )])
        .unwrap();
        assert_ne!(empty, one);
    }

    fn fresh() -> (PathBuf, Store) {
        let root = std::env::temp_dir().join(format!(
            "beamcraft-batch-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let store = Store::new(BlobStore::new(root.clone()).unwrap());
        (root, store)
    }

    fn fresh_id(ids: &mut IdSource) -> String {
        ids.next_id().unwrap().to_string()
    }

    fn request(ids: &mut IdSource, thickness: f64) -> CreateRequest {
        CreateRequest {
            op_id: OpId::parse(&fresh_id(ids)).unwrap(),
            revision_id: RevisionId::parse(&fresh_id(ids)).unwrap(),
            actor: Uuid7::parse(&fresh_id(ids)).unwrap(),
            project: ProjectId::from_uuid(ids.next_id().unwrap()),
            workspace: WorkspaceId::from_uuid(ids.next_id().unwrap()),
            command: CreatePlate {
                new_id: ElementId::parse(&fresh_id(ids)).unwrap(),
                name: "Batch plate".into(),
                class: 0,
                contour_points_mm: vec![
                    [0.0, 0.0, 0.0],
                    [400.0, 0.0, 0.0],
                    [400.0, 200.0, 0.0],
                    [0.0, 200.0, 0.0],
                ],
                thickness_mm: thickness,
                thickness_side: None,
                corners: Vec::new(),
                placement: Placement::IDENTITY,
                material_digest: None,
                holes_mm: Vec::new(),
            },
        }
    }

    #[test]
    fn a_batch_of_two_plates_commits_one_revision_and_replays_per_command() {
        let (_root, store) = fresh();
        let mut ids = IdSource::system();
        // The first revision lands outside any batch, so a batch has a head to pin.
        let first = request(&mut ids, 8.0);
        create_plate(&store, &first, &mut NoFault).unwrap();
        let before = inspect(&store).unwrap().revision;
        let (base, base_sequence) = {
            let head = store.open().unwrap();
            let sequence = head.info().sequence();
            let (_, pinned) =
                session::pipeline::load_base(&store, &first.project, &first.workspace).unwrap();
            (pinned, sequence)
        };
        let project = base.project.clone();
        let workspace = base.workspace.clone();
        let base_generation = base.generation;
        let guard = draft::hold(base, base_sequence).unwrap();
        // Two plates are admitted into the draft: each reports a drafted preview and the
        // store keeps its one revision with one plate.
        let second = request(&mut ids, 10.0);
        let created = create_plate(&store, &second.clone(), &mut NoFault).unwrap();
        assert!(!created.replayed);
        let third = request(&mut ids, 12.0);
        create_plate(&store, &third.clone(), &mut NoFault).unwrap();
        // The guard stays held: commit_batch itself ends the batch by taking its state.
        let unchanged = inspect(&store).unwrap();
        assert_eq!(unchanged.revision, before);
        assert_eq!(unchanged.plates.len(), 1);
        // The batch commits one revision carrying both plates.
        let mut fresh_ids = IdSource::system();
        let batch_op = OpId::parse(&fresh_ids.next_id().unwrap().to_string()).unwrap();
        let batch_revision = RevisionId::parse(&fresh_ids.next_id().unwrap().to_string()).unwrap();
        let schema = schema_identity().unwrap();
        let done = commit_batch(
            BatchCommit {
                store: &store,
                op: Uuid7::parse(&batch_op.to_string()).unwrap(),
                revision_id: &batch_revision,
                schema,
                project: &project,
                workspace: &workspace,
                base_generation,
            },
            &mut NoFault,
        )
        .unwrap();
        drop(guard);
        assert_eq!(done.commands, 2);
        let after = inspect(&store).unwrap();
        assert_eq!(after.plates.len(), 3);
        assert_eq!(
            after.revision.sequence,
            before.sequence.checked_add(1).unwrap()
        );
        // Each carried command replays like a standalone operation: the same request under
        // its own op id is answered from the batch revision, a different one is reuse.
        let replay = create_plate(&store, &second, &mut NoFault).unwrap();
        assert!(replay.replayed);
        let reuse = CreateRequest {
            command: CreatePlate {
                thickness_mm: 99.0,
                ..third.command.clone()
            },
            ..third
        };
        let error = create_plate(&store, &reuse, &mut NoFault).unwrap_err();
        assert_eq!(error.code, ErrorCode::OpIdReuse);
    }
}
