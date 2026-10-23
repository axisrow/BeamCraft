// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D03 source capture uses real committed USD, maintenance locks and actual history compaction.

use super::*;
use beamcraft_cli::drawing::archive::{MANIFEST_RESOURCE, SourceArchive};
use beamcraft_host::lease::StoreLease;
use beamcraft_host::revision::{Content, LayerEntry, Manifest, RevisionDraft, RevisionStore};
use beamcraft_host::storage::BlobStore;
use beamcraft_layer_engine::Asset;
use beamcraft_model::digest::{ContentDigest, Sha256};
use beamcraft_model::id::{ProjectId, RevisionId};

#[test]
fn captured_model_and_foreign_source_survive_live_edit_compaction_and_removal() {
    let f = Fixture::new();
    f.model();
    let foreign = f.root.join("site.usda");
    fs::write(&foreign, SITE).unwrap();
    f.ok("reference-attach", &["--source", foreign.to_str().unwrap()]);
    let root = f.root.join("store");
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
        "history-compact",
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
fn capture_refuses_active_maintenance_and_untrusted_archive_pins_resources_and_bytes() {
    let f = Fixture::new();
    f.model();
    let root = f.root.join("store");
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
    let archive = SourceArchive::capture(&f.root.join("store")).unwrap();
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
