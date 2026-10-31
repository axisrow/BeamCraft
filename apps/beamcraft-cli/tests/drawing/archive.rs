// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D03 source capture uses real committed USD, maintenance locks and actual history compaction.

use super::*;
use beamcraft_cli::drawing::archive::{MANIFEST_RESOURCE, SourceArchive};
use beamcraft_cli::drawing::document::store::expected_base;
use beamcraft_cli::drawing::document::{DocumentStore, DocumentWrite, State};
use beamcraft_engine::drawing::{
    DimensionSpec, Drawing, DrawingSpec, Plane, SourceModelPin, ViewSpec,
};
use beamcraft_engine::query::feature::FeatureId;
use beamcraft_host::lease::StoreLease;
use beamcraft_host::revision::{
    CommitOutcome, Content, ExpectedBase, LayerEntry, Manifest, RevisionDraft, RevisionStore,
};
use beamcraft_host::storage::BlobStore;
use beamcraft_layer_engine::Asset;
use beamcraft_model::digest::{ContentDigest, Sha256};
use beamcraft_model::id::{ProjectId, RevisionId};

const DRAWING: &str = "01900000-0000-7000-8000-000000000001";
const VIEW: &str = "01900000-0000-7000-8000-000000000002";
const DIMENSION: &str = "01900000-0000-7000-8000-000000000003";
const ACTOR: &str = "01900000-0000-7000-8000-000000000004";
const OP1: &str = "01900000-0000-7000-8000-000000000005";
const REV1: &str = "01900000-0000-7000-8000-000000000006";
const OP2: &str = "01900000-0000-7000-8000-000000000007";
const REV2: &str = "01900000-0000-7000-8000-000000000008";
const VERSION: &str = "01900000-0000-7000-8000-000000000009";

fn drawing(source: SourceModelPin) -> Drawing {
    Drawing::admit(DrawingSpec {
        id: beamcraft_model::id::DrawingId::parse(DRAWING).unwrap(),
        name: "Member drawing".to_owned(),
        source,
        member: beamcraft_model::id::ElementId::parse(MEMBER).unwrap(),
        view: ViewSpec {
            id: beamcraft_model::id::DrawingViewId::parse(VIEW).unwrap(),
            plane: Plane::Front,
            scale_denominator: 100.0,
            paper_origin_mm: [0.0, 0.0],
        },
        dimension: DimensionSpec {
            id: beamcraft_model::id::DimensionId::parse(DIMENSION).unwrap(),
            anchors: [
                FeatureId::parse(&format!("fid1:{MEMBER}:member:v.x0.y0.z0")).unwrap(),
                FeatureId::parse(&format!("fid1:{MEMBER}:member:v.x0.y0.z1")).unwrap(),
            ],
            paper_offset_mm: 10.0,
            decimal_places: 0,
        },
        drafting: Vec::new(),
        layers: Vec::new(),
        views: Vec::new(),
        sections: Vec::new(),
        details: Vec::new(),
        sheet: None,
    })
    .unwrap()
}

fn actor() -> beamcraft_model::id::ActorId {
    beamcraft_model::id::ActorId::parse(ACTOR).unwrap()
}

#[test]
fn captured_model_and_foreign_source_survive_live_edit_compaction_and_removal() {
    let f = Fixture::new();
    f.model();
    let foreign = f.shared.root.join("site.usda");
    fs::write(&foreign, SITE).unwrap();
    f.ok("attach-reference", &["--source", foreign.to_str().unwrap()]);
    let root = f.shared.root.join("store");
    let archive = SourceArchive::capture(&root).unwrap();
    let pin = archive.pin();
    assert_eq!(pin.revision.to_string(), f.head());
    let before = archive.inspect().unwrap();
    assert_eq!(before.members.len(), 1);
    assert_eq!(before.plates.len(), 1);
    assert_eq!(
        before.members[0].authored.placement.translation_mm,
        [0.0, 2000.0, 0.0]
    );
    assert!(
        archive
            .resources()
            .iter()
            .any(|r| r.name.starts_with("model/external/"))
    );
    // Capture owns and drops all its maintenance handles. Its return must not leave compaction
    // blocked by a cached lock descriptor, even in this same long-lived process.
    {
        let blobs = BlobStore::new(&root).unwrap();
        let _exclusive = StoreLease::exclusive(&blobs).unwrap();
    }
    f.ok(
        "update-element",
        &["--id", MEMBER, "--translation", "1,2,3"],
    );
    assert_ne!(pin.revision.to_string(), f.head());
    f.ok(
        "compact-history",
        &["--keep-head", "--yes", "--no-backup", "--no-workspaces"],
    );
    {
        let store = RevisionStore::new(BlobStore::new(&root).unwrap());
        assert!(store.open_at(before.revision.sequence).is_err());
    }
    // Reopen from retained resources only; neither the live model nor foreign filesystem source
    // participates. Removing the whole model is stronger than a retained-head-only compaction.
    fs::remove_dir_all(&root).unwrap();
    fs::remove_file(foreign).unwrap();
    let reopened = SourceArchive::verify(pin, archive.resources().to_vec()).unwrap();
    let old = reopened.inspect().unwrap();
    assert_eq!(old.revision, before.revision);
    assert_eq!(old.snapshot.token(), before.snapshot.token());
    assert_eq!(
        old.members[0].authored.placement.translation_mm,
        [0.0, 2000.0, 0.0]
    );
}

#[test]
fn document_revision_is_cas_verified_and_operation_replay_survives_a_later_edit() {
    let f = Fixture::new();
    f.model();
    let model_root = f.shared.root.join("store");
    let first_source = SourceArchive::capture(&model_root).unwrap();
    let first_drawing = drawing(first_source.pin());
    let documents = DocumentStore::create(&f.shared.root.join("documents")).unwrap();
    let first_op = beamcraft_model::id::OpId::parse(OP1).unwrap();
    let first_revision = beamcraft_model::id::Uuid7::parse(REV1).unwrap();
    let (outcome, first) = documents
        .commit(
            &ExpectedBase::Initial,
            DocumentWrite {
                revision: first_revision,
                operation: first_op,
                actor: actor(),
                drawing: &first_drawing,
                source: &first_source,
                state: State::Draft,
                version: None,
                latest_published: None,
            },
        )
        .unwrap();
    assert_eq!(outcome, CommitOutcome::Created);
    assert_eq!(first.state, State::Draft);
    assert_eq!(first.drawing.spec().source, first_source.pin());
    assert_eq!(documents.head().unwrap().info, first.info);

    let (replayed, prior) = documents
        .commit(
            &ExpectedBase::Initial,
            DocumentWrite {
                revision: first_revision,
                operation: first_op,
                actor: actor(),
                drawing: &first_drawing,
                source: &first_source,
                state: State::Draft,
                version: None,
                latest_published: None,
            },
        )
        .unwrap();
    assert_eq!(replayed, CommitOutcome::Replayed);
    assert_eq!(prior.info, first.info);

    f.ok(
        "update-element",
        &["--id", MEMBER, "--translation", "4,5,6"],
    );
    let second_source = SourceArchive::capture(&model_root).unwrap();
    let second_drawing = drawing(second_source.pin());
    let second_op = beamcraft_model::id::OpId::parse(OP2).unwrap();
    let second_revision = beamcraft_model::id::Uuid7::parse(REV2).unwrap();
    let base = expected_base(Some(&first.info)).unwrap();
    let (_, second) = documents
        .commit(
            &base,
            DocumentWrite {
                revision: second_revision,
                operation: second_op,
                actor: actor(),
                drawing: &second_drawing,
                source: &second_source,
                state: State::Draft,
                version: None,
                latest_published: None,
            },
        )
        .unwrap();
    assert_ne!(first.info.revision(), second.info.revision());

    // The exact first operation is answered from the ledger after the head has moved.
    assert_eq!(
        documents.query(&first_op).unwrap().unwrap().info,
        first.info
    );
    assert!(
        documents
            .commit(
                &ExpectedBase::Initial,
                DocumentWrite {
                    revision: first_revision,
                    operation: first_op,
                    actor: actor(),
                    drawing: &second_drawing,
                    source: &second_source,
                    state: State::Draft,
                    version: None,
                    latest_published: None,
                },
            )
            .is_err()
    );
    assert_eq!(documents.head().unwrap().info, second.info);
}

#[test]
fn document_publication_creates_an_immutable_version_and_stale_cas_loses() {
    let f = Fixture::new();
    f.model();
    let source = SourceArchive::capture(&f.shared.root.join("store")).unwrap();
    let drawing = drawing(source.pin());
    let documents = DocumentStore::create(&f.shared.root.join("documents")).unwrap();
    let (_, draft) = documents
        .commit(
            &ExpectedBase::Initial,
            DocumentWrite {
                revision: beamcraft_model::id::Uuid7::parse(REV1).unwrap(),
                operation: beamcraft_model::id::OpId::parse(OP1).unwrap(),
                actor: actor(),
                drawing: &drawing,
                source: &source,
                state: State::Draft,
                version: None,
                latest_published: None,
            },
        )
        .unwrap();
    let base = expected_base(Some(&draft.info)).unwrap();
    let version = beamcraft_model::id::DocumentVersionId::parse(VERSION).unwrap();
    let published_op = beamcraft_model::id::OpId::parse(OP2).unwrap();
    let (_, published) = documents
        .commit(
            &base,
            DocumentWrite {
                revision: beamcraft_model::id::Uuid7::parse(REV2).unwrap(),
                operation: published_op,
                actor: actor(),
                drawing: &drawing,
                source: &source,
                state: State::Published,
                version: Some(version),
                latest_published: Some(version),
            },
        )
        .unwrap();
    assert_eq!(published.version, Some(version));
    assert_eq!(published.latest_published, Some(version));
    assert_eq!(
        documents.query(&published_op).unwrap().unwrap().version,
        Some(version)
    );
    assert!(
        documents
            .commit(
                &base,
                DocumentWrite {
                    revision: beamcraft_model::id::Uuid7::parse(
                        "01900000-0000-7000-8000-000000000010"
                    )
                    .unwrap(),
                    operation: beamcraft_model::id::OpId::parse(
                        "01900000-0000-7000-8000-000000000011"
                    )
                    .unwrap(),
                    actor: actor(),
                    drawing: &drawing,
                    source: &source,
                    state: State::Draft,
                    version: None,
                    latest_published: None,
                },
            )
            .is_err()
    );
    assert_eq!(documents.head().unwrap().version, Some(version));
}

#[test]
fn publication_refuses_incomplete_history_and_active_maintenance_without_a_revision() {
    use beamcraft_model::error::ErrorCode;
    use beamcraft_model::id::{DocumentVersionId, OpId, Uuid7};

    for damage in ["compacted", "missing", "busy"] {
        let f = Fixture::new();
        f.model();
        let source = SourceArchive::capture(&f.shared.root.join("store")).unwrap();
        let drawing = drawing(source.pin());
        let root = f.shared.root.join("documents");
        let documents = DocumentStore::create(&root).unwrap();
        let first_version = DocumentVersionId::parse(VERSION).unwrap();
        let write = |base: &ExpectedBase, n: u8, state, version, latest_published| {
            documents.commit(
                base,
                DocumentWrite {
                    revision: Uuid7::parse(&format!("01900000-0000-7000-8000-{n:012x}")).unwrap(),
                    operation: OpId::parse(&format!("01900000-0000-7000-8000-{:012x}", n + 1))
                        .unwrap(),
                    actor: actor(),
                    drawing: &drawing,
                    source: &source,
                    state,
                    version,
                    latest_published,
                },
            )
        };
        let (_, draft) = write(&ExpectedBase::Initial, 20, State::Draft, None, None).unwrap();
        let (_, published) = write(
            &expected_base(Some(&draft.info)).unwrap(),
            22,
            State::Published,
            Some(first_version),
            Some(first_version),
        )
        .unwrap();
        let (_, current) = write(
            &expected_base(Some(&published.info)).unwrap(),
            24,
            State::Draft,
            None,
            Some(first_version),
        )
        .unwrap();
        let blobs = BlobStore::new(&root).unwrap();
        let maintenance = if damage == "busy" {
            Some(StoreLease::exclusive(&blobs).unwrap())
        } else {
            None
        };
        let expected_code = match damage {
            "compacted" => {
                let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
                    .args([
                        "compact-history",
                        "--store",
                        root.to_str().unwrap(),
                        "--keep-head",
                        "--yes",
                        "--no-backup",
                        "--no-workspaces",
                    ])
                    .output()
                    .unwrap();
                assert!(output.status.success(), "{:?}", output);
                assert!(matches!(
                    RevisionStore::new(blobs.clone()).open_at(published.info.sequence()),
                    Err(beamcraft_host::revision::RevisionError::Compacted)
                ));
                ErrorCode::DependencyUnavailable
            }
            "missing" => {
                let digest = published
                    .info
                    .manifest()
                    .content()
                    .entries()
                    .iter()
                    .find(|entry| entry.name() == "root.usda")
                    .unwrap()
                    .digest()
                    .to_string();
                fs::remove_file(root.join("blobs").join(&digest[..2]).join(&digest)).unwrap();
                ErrorCode::IntegrityError
            }
            _ => ErrorCode::BackendUnavailable,
        };
        // Neither a previously used nor a fresh ID earns a uniqueness proof from missing
        // history. A conflicting maintenance lease must also refuse before any commit.
        for version in [first_version, DocumentVersionId::parse(COPY).unwrap()] {
            let error = write(
                &expected_base(Some(&current.info)).unwrap(),
                26,
                State::Published,
                Some(version),
                Some(version),
            )
            .unwrap_err();
            assert_eq!(error.code, expected_code, "{damage}");
            assert_eq!(documents.head().unwrap().info, current.info);
            assert!(
                documents
                    .query(&OpId::parse("01900000-0000-7000-8000-00000000001b").unwrap())
                    .unwrap()
                    .is_none()
            );
        }
        drop(maintenance);
    }
}

#[test]
fn capture_refuses_active_maintenance_and_untrusted_archive_pins_resources_and_bytes() {
    let f = Fixture::new();
    f.model();
    let root = f.shared.root.join("store");
    let head = f.head();
    {
        let blobs = BlobStore::new(&root).unwrap();
        let _exclusive = StoreLease::exclusive(&blobs).unwrap();
        assert!(SourceArchive::capture(&root).is_err());
    }
    let archive = SourceArchive::capture(&root).unwrap();
    let pin = archive.pin();
    for field in 0..5 {
        let mut wrong = pin;
        match field {
            0 => wrong.project = ProjectId::parse(COPY).unwrap(),
            1 => wrong.revision = RevisionId::parse(COPY).unwrap(),
            2 => wrong.manifest = ContentDigest::from_bytes([9; 32]),
            3 => wrong.schema = ContentDigest::from_bytes([9; 32]),
            _ => wrong.generation += 1,
        }
        assert!(SourceArchive::verify(wrong, archive.resources().to_vec()).is_err());
    }
    for case in 0..6 {
        let mut resources = archive.resources().to_vec();
        match case {
            0 => {
                resources.pop();
            }
            1 => resources.push(resources[0].clone()),
            2 => {
                resources[1].bytes.push(b' ');
            }
            3 => {
                resources[1].name = "model/missing.usda".into();
            }
            4 => {
                resources[1].bytes.push(b' ');
                resources[1].digest = Sha256::digest(&resources[1].bytes).unwrap();
            }
            _ => {
                resources[1].name = "m".repeat(beamcraft_host::revision::MAX_NAME_BYTES + 1);
            }
        }
        assert!(SourceArchive::verify(pin, resources).is_err());
    }
    assert_eq!(f.head(), head);
}

#[test]
fn a_self_consistent_rehashed_archive_with_stale_kernel_geometry_is_refused() {
    let f = Fixture::new();
    f.model();
    let archive = SourceArchive::capture(&f.shared.root.join("store")).unwrap();
    let mut resources = archive.resources().to_vec();
    let source = resources
        .iter_mut()
        .find(|r| r.name == "model/authored.usda")
        .unwrap();
    let original = String::from_utf8(source.bytes.clone()).unwrap();
    assert!(original.contains("double steel:plate:thicknessMm = 10"));
    source.bytes = original
        .replacen(
            "double steel:plate:thicknessMm = 10",
            "double steel:plate:thicknessMm = 11",
            1,
        )
        .into_bytes();
    source.digest = Sha256::digest(&source.bytes).unwrap();
    let provenance = resources
        .iter()
        .find(|r| r.name == MANIFEST_RESOURCE)
        .unwrap();
    let manifest = Manifest::parse(&provenance.bytes).unwrap();
    let c = manifest.content();
    let entries = c
        .entries()
        .iter()
        .map(|e| {
            let asset = resources
                .iter()
                .find(|r| r.name == format!("model/{}", e.name()))
                .unwrap();
            LayerEntry::from_bytes(e.name(), e.role(), &asset.bytes).unwrap()
        })
        .collect();
    let content = Content::from_draft(&RevisionDraft {
        project: c.project().clone(),
        workspace: c.workspace().clone(),
        revision: c.revision().clone(),
        op: c.op().clone(),
        workspace_generation: c.workspace_generation(),
        schema: c.schema(),
        root_layer: c.root_layer().to_owned(),
        entries,
    })
    .unwrap();
    let changed = Manifest::assemble(
        content,
        manifest.sequence(),
        manifest.base().cloned(),
        manifest.index(),
    )
    .unwrap();
    let bytes = changed.encode();
    let digest = Sha256::digest(&bytes).unwrap();
    let slot = resources
        .iter_mut()
        .find(|r| r.name == MANIFEST_RESOURCE)
        .unwrap();
    *slot = Asset {
        name: MANIFEST_RESOURCE.into(),
        digest,
        bytes,
    };
    let mut pin = archive.pin();
    pin.manifest = digest;
    let error = SourceArchive::verify(pin, resources).unwrap_err();
    assert_eq!(error.code.as_str(), "INTEGRITY_ERROR");
}
