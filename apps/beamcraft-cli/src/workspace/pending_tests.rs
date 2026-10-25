// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Original synthetic models and crash fixtures, MPL-2.0; existing attribution applies.

use super::*;
use crate::{command_evidence as evidence, edit, plate, workspace};
use beamcraft_engine::footprint::Footprint;
use beamcraft_host::revision::{
    CommitPoint, ExpectedBase, LayerEntry, NoFault, RevisionDraft, Uuid7,
};
use beamcraft_layer_engine::authored::{EditTargets, rebase};
use beamcraft_model::command::{CreatePlate, ElementPatch, Placement, UpdateElement};
use beamcraft_model::id::{ElementId, OpId, ProjectId, RevisionId, WorkspaceId};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT: AtomicU64 = AtomicU64::new(0);
fn id(n: usize) -> Uuid7 {
    Uuid7::parse(&format!("019a7bf0-0000-7000-8000-{n:012x}")).unwrap()
}
fn element(n: usize) -> ElementId {
    ElementId::parse(id(n).as_str()).unwrap()
}

struct Fixture {
    root: PathBuf,
    published: RevisionStore,
    draft: RevisionStore,
}
impl Fixture {
    fn open(root: PathBuf) -> Self {
        Self {
            published: store(&root.join("published")).unwrap(),
            draft: store(&root.join("draft")).unwrap(),
            root,
        }
    }
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "beamcraft-rebase-recovery-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("published")).unwrap();
        fs::create_dir(root.join("draft")).unwrap();
        let f = Self::open(root);
        f.plate(&f.published, 10);
        workspace::create_workspace(
            &f.published,
            &f.draft,
            &f.dir(),
            &workspace::CreateWorkspace {
                workspace_id: id(2),
                user_id: id(3),
                revision_id: id(21),
                op_id: id(22),
            },
            &mut NoFault,
        )
        .unwrap();
        f.rename(30, "local");
        f.plate(&f.published, 40);
        f
    }
    fn dir(&self) -> PathBuf {
        self.root.join("draft")
    }
    fn plate(&self, store: &RevisionStore, n: usize) {
        plate::create_plate(
            store,
            &plate::CreateRequest {
                op_id: OpId::parse(id(n + 1).as_str()).unwrap(),
                revision_id: RevisionId::parse(id(n + 2).as_str()).unwrap(),
                actor: id(3),
                project: ProjectId::parse(id(1).as_str()).unwrap(),
                workspace: WorkspaceId::parse(id(4).as_str()).unwrap(),
                command: CreatePlate {
                    new_id: element(n),
                    name: format!("part {n}"),
                    contour_points_mm: vec![
                        [0.0, 0.0, 0.0],
                        [100.0, 0.0, 0.0],
                        [100.0, 200.0, 0.0],
                        [0.0, 200.0, 0.0],
                    ],
                    thickness_mm: 10.0,
                    placement: Placement::IDENTITY,
                    material_digest: None,
                    holes_mm: Vec::new(),
                    cut: None,
                },
            },
            &mut NoFault,
        )
        .unwrap();
    }
    fn rename(&self, n: usize, name: &str) {
        edit::update_element(
            &self.draft,
            &edit::UpdateRequest {
                op_id: OpId::parse(id(n + 1).as_str()).unwrap(),
                revision_id: RevisionId::parse(id(n + 2).as_str()).unwrap(),
                actor: id(3),
                project: ProjectId::parse(id(1).as_str()).unwrap(),
                workspace: WorkspaceId::parse(id(2).as_str()).unwrap(),
                command: UpdateElement {
                    target_id: element(10),
                    patch: ElementPatch {
                        name: Some(name.into()),
                        ..ElementPatch::default()
                    },
                },
            },
            &mut NoFault,
        )
        .unwrap();
    }
    fn receipt(&self, n: usize) -> RebaseReceipt {
        let old_meta = workspace::read_meta(&self.dir()).unwrap();
        let draft = self.draft.head().unwrap().unwrap();
        let operations =
            evidence::interval(&self.draft, old_meta.synced_sequence, draft.sequence()).unwrap();
        let footprint = evidence::aggregate(&self.draft, old_meta.synced_sequence, &operations)
            .unwrap()
            .footprint
            .unwrap();
        RebaseReceipt {
            op_id: OpId::parse(id(n).as_str()).unwrap(),
            project: draft.manifest().content().project().clone(),
            schema: draft.manifest().content().schema(),
            old_base_sequence: workspace::published_base(&self.published, &old_meta)
                .unwrap()
                .info()
                .sequence(),
            draft: RevisionPin::from_info(&draft),
            accepted: RevisionPin::from_info(&self.published.head().unwrap().unwrap()),
            old_meta,
            operations,
            footprint,
        }
    }
    fn candidate(&self, receipt: &RebaseReceipt) -> RevisionDraft {
        let base = workspace::published_base(&self.published, &receipt.old_meta).unwrap();
        let source = self.draft.open_at(receipt.draft.sequence).unwrap();
        let accepted = self.published.open_at(receipt.accepted.sequence).unwrap();
        let candidate = rebase::prepare(
            &plate::open_base(&plate::base_of(&base).unwrap()).unwrap(),
            &plate::open_base(&plate::base_of(&source).unwrap()).unwrap(),
            &plate::open_base(&plate::base_of(&accepted).unwrap()).unwrap(),
            &evidence::dependencies(&receipt.footprint),
            &EditTargets {
                authored: plate::AUTHORED_LAYER.into(),
                derived: plate::DERIVED_LAYER.into(),
            },
            &plate::control(),
        )
        .unwrap();
        let entries = candidate
            .assets
            .into_iter()
            .map(|asset| {
                self.draft.blobs().publish(&asset.bytes).unwrap();
                let role = accepted
                    .manifest()
                    .content()
                    .entries()
                    .iter()
                    .find(|e| e.name() == asset.name)
                    .unwrap()
                    .role();
                LayerEntry::from_bytes(&asset.name, role, &asset.bytes).unwrap()
            })
            .collect();
        let content = source.manifest().content();
        RevisionDraft {
            project: content.project().clone(),
            workspace: content.workspace().clone(),
            revision: id_from(receipt),
            op: id_from(receipt),
            workspace_generation: content.workspace_generation() + 1,
            schema: receipt.schema,
            root_layer: content.root_layer().into(),
            entries,
        }
    }
    fn commit(&self, state: &WorkspaceState, receipt: &RebaseReceipt) {
        let candidate = self.candidate(receipt);
        let pin = state.stage_rebase(receipt).unwrap();
        assert_eq!(state.stage_rebase(receipt).unwrap(), pin);
        crate::publication::commit_verified_with_evidence(
            &self.draft,
            &candidate,
            &ExpectedBase::Revision(receipt.draft.revision.clone()),
            Some(pin),
            &mut NoFault,
        )
        .unwrap();
    }
}
fn id_from(receipt: &RebaseReceipt) -> Uuid7 {
    Uuid7::parse(&receipt.op_id.to_string()).unwrap()
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn abandoned_intent_and_a_different_cas_winner_keep_original_metadata_and_unsent_work() {
    for race in [false, true] {
        let f = Fixture::new();
        let receipt = f.receipt(500);
        let state = WorkspaceState::open(&f.dir()).unwrap();
        state.stage_rebase(&receipt).unwrap();
        if race {
            f.rename(60, "other local winner");
        }
        let head = f.draft.head().unwrap().unwrap();
        drop(state);
        assert_eq!(workspace::read_meta(&f.dir()).unwrap(), receipt.old_meta);
        assert_eq!(
            f.draft.head().unwrap().unwrap().manifest_digest(),
            head.manifest_digest()
        );
        assert!(!f.dir().join(FILE).exists());
        assert_eq!(
            plate::inspect(&f.draft).unwrap().plates[0].authored.name,
            if race { "other local winner" } else { "local" }
        );
    }
}

#[test]
fn recovery_keeps_a_later_edit_and_publication_retains_both_clients() {
    let f = Fixture::new();
    let receipt = f.receipt(500);
    let state = WorkspaceState::open(&f.dir()).unwrap();
    f.commit(&state, &receipt);
    f.rename(60, "later local");
    let head = f.draft.head().unwrap().unwrap();
    drop(state);
    let meta = workspace::read_meta(&f.dir()).unwrap();
    assert_eq!(meta, next_meta(&receipt).unwrap());
    assert_eq!(
        f.draft.head().unwrap().unwrap().manifest_digest(),
        head.manifest_digest()
    );
    assert_eq!(meta.synced_sequence, receipt.old_meta.synced_sequence);
    assert_eq!(meta.rebase_sequence, receipt.draft.sequence + 1);
    let report = plate::inspect(&f.draft).unwrap();
    assert_eq!(report.plates.len(), 2);
    assert_eq!(
        report
            .plates
            .iter()
            .find(|p| p.authored.id == element(10))
            .unwrap()
            .authored
            .name,
        "later local"
    );
    assert_eq!(
        f.published.head().unwrap().unwrap().manifest_digest(),
        receipt.accepted.manifest
    );
    let published =
        workspace::publish_workspace(&f.published, &f.draft, &f.dir(), &mut NoFault).unwrap();
    assert_eq!(published.status.unsent_revisions, 0);
    assert_eq!(plate::inspect(&f.published).unwrap().plates.len(), 2);
    assert_eq!(
        plate::inspect(&f.published)
            .unwrap()
            .plates
            .iter()
            .find(|p| p.authored.id == element(10))
            .unwrap()
            .authored
            .name,
        "later local"
    );
}

#[test]
fn repeated_receipts_keep_one_dependency_union_and_all_original_operation_ids() {
    let f = Fixture::new();
    for n in 0..3 {
        let receipt = f.receipt(500 + n);
        let mut state = WorkspaceState::open(&f.dir()).unwrap();
        f.commit(&state, &receipt);
        assert_eq!(
            state.finish_rebase().unwrap().unwrap().revision_id,
            id(500 + n).to_string()
        );
        assert!(state.finish_rebase().unwrap().is_none());
        drop(state);
        let operations =
            evidence::interval(&f.draft, 1, f.draft.head().unwrap().unwrap().sequence()).unwrap();
        let union = evidence::aggregate(&f.draft, 1, &operations)
            .unwrap()
            .footprint
            .unwrap();
        assert_eq!(union, receipt.footprint);
        assert_eq!(operations.len(), n + 2);
        assert_eq!(operations[0].op_id, OpId::parse(id(31).as_str()).unwrap());
        assert_eq!(plate::inspect(&f.draft).unwrap().plates.len(), n + 2);
        f.plate(&f.published, 70 + n * 10);
    }
}

#[test]
fn malformed_or_narrowed_intents_refuse_without_advancing_metadata() {
    let f = Fixture::new();
    let mut receipt = f.receipt(500);
    let original = fs::read(f.dir().join("workspace.json")).unwrap();
    let bytes = receipt.encode().unwrap();
    for invalid in [
        b"garbage".to_vec(),
        bytes[..bytes.len() - 1].to_vec(),
        vec![b'x'; MAX_OPERATION_EVIDENCE_BYTES as usize + 1],
    ] {
        fs::write(f.dir().join(FILE), &invalid).unwrap();
        assert!(WorkspaceState::open(&f.dir()).is_err());
        assert_eq!(fs::read(f.dir().join("workspace.json")).unwrap(), original);
        assert_eq!(fs::read(f.dir().join(FILE)).unwrap(), invalid);
    }
    fs::remove_file(f.dir().join(FILE)).unwrap();
    let reads = receipt
        .footprint
        .reads()
        .iter()
        .copied()
        .collect::<Vec<_>>();
    receipt.footprint = Footprint::from_entries(&reads, &[]).unwrap();
    let state = WorkspaceState::open(&f.dir()).unwrap();
    assert_eq!(
        state.stage_rebase(&receipt).unwrap_err().code,
        ErrorCode::IntegrityError
    );
    drop(state);
    fs::write(f.dir().join(FILE), receipt.encode().unwrap()).unwrap();
    assert_eq!(
        WorkspaceState::open(&f.dir()).err().unwrap().code,
        ErrorCode::IntegrityError
    );
    assert_eq!(fs::read(f.dir().join("workspace.json")).unwrap(), original);
}

#[test]
fn a_candidate_without_the_exact_receipt_cannot_advance_the_base() {
    let f = Fixture::new();
    let receipt = f.receipt(500);
    let candidate = f.candidate(&receipt);
    let state = WorkspaceState::open(&f.dir()).unwrap();
    state.stage_rebase(&receipt).unwrap();
    crate::publication::commit_verified_with_evidence(
        &f.draft,
        &candidate,
        &ExpectedBase::Revision(receipt.draft.revision.clone()),
        None,
        &mut NoFault,
    )
    .unwrap();
    drop(state);
    assert_eq!(
        WorkspaceState::open(&f.dir()).err().unwrap().code,
        ErrorCode::IntegrityError
    );
    assert_eq!(
        WorkspaceMeta::decode(
            std::str::from_utf8(&fs::read(f.dir().join("workspace.json")).unwrap()).unwrap()
        )
        .unwrap(),
        receipt.old_meta
    );
    assert!(f.dir().join(FILE).exists());
}

fn parked(f: &Fixture) -> ! {
    fs::write(f.root.join("ready"), b"ready").unwrap();
    loop {
        std::thread::park();
    }
}

#[test]
fn rebase_crash_child() {
    let Some(root) = std::env::var_os("BEAMCRAFT_REBASE_RECOVERY_ROOT") else {
        return;
    };
    let f = Fixture::open(root.into());
    let receipt = f.receipt(500);
    let candidate = f.candidate(&receipt);
    let mut state = WorkspaceState::open(&f.dir()).unwrap();
    let phase = std::env::var("BEAMCRAFT_REBASE_RECOVERY_PHASE").unwrap();
    if phase == "blobs" {
        parked(&f);
    }
    let pin = state.stage_rebase(&receipt).unwrap();
    if phase == "intent" {
        parked(&f);
    }
    let mut hook = |point| {
        if point == CommitPoint::Switched && phase == "switch" {
            parked(&f);
        }
        Ok(())
    };
    crate::publication::commit_verified_with_evidence(
        &f.draft,
        &candidate,
        &ExpectedBase::Revision(receipt.draft.revision.clone()),
        Some(pin),
        &mut hook,
    )
    .unwrap();
    state
        .compare_and_swap(Some(&receipt.old_meta), &next_meta(&receipt).unwrap())
        .unwrap();
    parked(&f);
}

#[test]
fn killed_process_at_each_storage_boundary_recovers_one_complete_draft() {
    for phase in ["blobs", "intent", "switch", "metadata"] {
        let f = Fixture::new();
        let receipt = f.receipt(500);
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "workspace::pending::tests::rebase_crash_child",
                "--nocapture",
            ])
            .env("BEAMCRAFT_REBASE_RECOVERY_ROOT", &f.root)
            .env("BEAMCRAFT_REBASE_RECOVERY_PHASE", phase)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let until = Instant::now() + Duration::from_secs(15);
        while !f.root.join("ready").exists() && Instant::now() < until {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let reached = f.root.join("ready").exists();
        let _ = child.kill();
        child.wait().unwrap();
        assert!(reached, "{phase}");
        let meta = workspace::read_meta(&f.dir()).unwrap();
        let committed = phase == "switch" || phase == "metadata";
        assert_eq!(
            meta,
            if committed {
                next_meta(&receipt).unwrap()
            } else {
                receipt.old_meta.clone()
            }
        );
        assert_eq!(
            plate::inspect(&f.draft).unwrap().plates.len(),
            if committed { 2 } else { 1 }
        );
        assert!(!f.dir().join(FILE).exists());
        assert_eq!(
            f.published.head().unwrap().unwrap().manifest_digest(),
            receipt.accepted.manifest
        );
    }
}
