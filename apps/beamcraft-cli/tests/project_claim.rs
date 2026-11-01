// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic fixtures, MPL-2.0.

//! D1 regression: an explicit project claim is compared with the base a revision is built on, not
//! with an earlier read of the store. A writer that initialises the store between a caller's
//! early check and its base loading must not make the caller commit into another project.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_cli::level::{LevelContext, create_level, levels};
use beamcraft_host::{
    ids::IdSource,
    revision::{NoFault, RevisionStore, Uuid7},
    storage::BlobStore,
};
use beamcraft_model::command::CreateLevel;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId, ProjectId, RevisionId, WorkspaceId};
use session::pipeline::{claim_project, ensure_project};
use std::path::PathBuf;

const PROJECT_A: &str = "019a7bf0-0000-7000-8000-00000000e001";
const PROJECT_B: &str = "019a7bf0-0000-7000-8000-00000000e002";

fn context(project: &str) -> LevelContext {
    let mut ids = IdSource::system();
    LevelContext {
        op_id: OpId::from_uuid(ids.next_id().unwrap()),
        revision_id: RevisionId::from_uuid(ids.next_id().unwrap()),
        actor: Uuid7::parse(&ids.next_id().unwrap().to_string()).unwrap(),
        project: ProjectId::parse(project).unwrap(),
        workspace: WorkspaceId::from_uuid(ids.next_id().unwrap()),
    }
}

fn level(name: &str) -> CreateLevel {
    let mut ids = IdSource::system();
    CreateLevel {
        new_id: ElementId::from_uuid(ids.next_id().unwrap()),
        name: name.into(),
        elevation_mm: 0.0,
    }
}

fn store_dir(tag: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("d1-claim-tests")
        .join(format!("{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_store_initialised_after_the_early_check_is_not_adopted_by_a_claimed_project() {
    let dir = store_dir("race");
    let store = RevisionStore::new(BlobStore::new(&dir).unwrap());
    let b = ProjectId::parse(PROJECT_B).unwrap();
    // Command B names project B and its early check passes: the store has no revision yet.
    ensure_project(&store, &b).unwrap();
    let claim = claim_project(b);
    // Another writer initialises the store with project A before B loads its base.
    let writer_dir = dir.clone();
    std::thread::spawn(move || {
        let other = RevisionStore::new(BlobStore::new(&writer_dir).unwrap());
        create_level(
            &other,
            &context(PROJECT_A),
            &level("A's level"),
            &mut NoFault,
        )
        .unwrap();
    })
    .join()
    .unwrap();
    // B must not commit into A's project: typed refusal, nothing written.
    let refused = create_level(
        &store,
        &context(PROJECT_B),
        &level("B's level"),
        &mut NoFault,
    )
    .unwrap_err();
    assert_eq!(refused.code, ErrorCode::InvalidArgument);
    let head = store.open().unwrap();
    assert_eq!(head.manifest().content().project().as_str(), PROJECT_A);
    assert_eq!(head.info().sequence(), 1);
    // The claim binds base loading, reads included: end it before reading as another caller.
    drop(claim);
    assert_eq!(levels(&store).unwrap().len(), 1);
    // The same project is accepted, and a caller without a claim takes the store's project as
    // before.
    let a = claim_project(ProjectId::parse(PROJECT_A).unwrap());
    create_level(&store, &context(PROJECT_A), &level("second"), &mut NoFault).unwrap();
    drop(a);
    create_level(
        &store,
        &context(PROJECT_B),
        &level("unclaimed"),
        &mut NoFault,
    )
    .unwrap();
    let head = store.open().unwrap();
    assert_eq!(head.manifest().content().project().as_str(), PROJECT_A);
    assert_eq!(levels(&store).unwrap().len(), 3);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_claim_on_an_empty_store_records_the_claimed_project_and_claims_nest() {
    let dir = store_dir("empty");
    let store = RevisionStore::new(BlobStore::new(&dir).unwrap());
    let outer = claim_project(ProjectId::parse(PROJECT_A).unwrap());
    {
        let _inner = claim_project(ProjectId::parse(PROJECT_B).unwrap());
        // The inner claim wins while it is held, even over the context's own project.
        create_level(&store, &context(PROJECT_A), &level("first"), &mut NoFault).unwrap();
    }
    assert_eq!(
        store
            .open()
            .unwrap()
            .manifest()
            .content()
            .project()
            .as_str(),
        PROJECT_B
    );
    // The outer claim is restored when the inner one ends, and now disagrees with the store.
    let refused =
        create_level(&store, &context(PROJECT_A), &level("second"), &mut NoFault).unwrap_err();
    assert_eq!(refused.code, ErrorCode::InvalidArgument);
    drop(outer);
    let _ = std::fs::remove_dir_all(dir);
}
