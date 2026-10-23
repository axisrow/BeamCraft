// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic publication/proof fixtures, MPL-2.0.

//! C06.1: real published before/after images, ownership and proof refusal.

use super::*;
use beamcraft_cli::compensation::verify_original;
use beamcraft_host::lease::StoreLease;

#[test]
fn the_original_publication_proves_both_images_after_the_source_advances() {
    let f = Fixture::new(false);
    let before = f.head("a");
    f.update("a", 10, &["--name", "original accepted rename"]);
    let original = f.head("a");
    f.publish("a");
    let accepted = f.head("published");
    assert_eq!(original.revision(), accepted.revision());
    assert_ne!(
        original.manifest().content().op(),
        accepted.manifest().content().op()
    );
    f.update("a", 11, &["--name", "later unsent edit"]);
    let source_head = f.head("a");
    let metadata = fs::read(f.path("a").join("workspace.json")).unwrap();
    let published = store(&f.path("published"));
    let source = store(&f.path("a"));
    let proof = verify_original(
        &published,
        &source,
        &f.path("a"),
        &Uuid7::parse(&id(3)).unwrap(),
        original.revision(),
    )
    .unwrap();
    assert_eq!(
        proof.before().info().manifest_digest(),
        before.manifest_digest()
    );
    assert_eq!(
        proof.after().info().manifest_digest(),
        original.manifest_digest()
    );
    assert_eq!(
        proof.published().info().manifest_digest(),
        accepted.manifest_digest()
    );
    assert_eq!(
        proof.command().op_id.to_string(),
        original.manifest().content().op().as_str()
    );
    assert_eq!(proof.command().footprint.writes().len(), 1);
    assert!(
        StoreLease::exclusive(published.blobs()).is_err(),
        "proof pins accepted history against collection"
    );
    assert!(
        StoreLease::exclusive(source.blobs()).is_err(),
        "proof pins the source images too"
    );
    assert_eq!(f.head("a").manifest_digest(), source_head.manifest_digest());
    assert_eq!(
        fs::read(f.path("a").join("workspace.json")).unwrap(),
        metadata
    );
    drop(proof);
    assert!(StoreLease::exclusive(published.blobs()).is_ok());
    assert!(StoreLease::exclusive(source.blobs()).is_ok());
}

#[test]
fn wrong_actor_wrong_source_and_unpublished_results_never_become_original_proof() {
    let f = Fixture::new(false);
    f.update("a", 10, &["--name", "accepted"]);
    let original = f.head("a");
    f.publish("a");
    f.update("b", 11, &["--name", "unpublished"]);
    let draft = f.head("b");
    let published = store(&f.path("published"));
    let source = store(&f.path("a"));
    assert_eq!(
        verify_original(
            &published,
            &source,
            &f.path("a"),
            &Uuid7::parse(&id(4)).unwrap(),
            original.revision()
        )
        .err()
        .unwrap()
        .code,
        ErrorCode::PermissionDenied
    );
    assert!(
        verify_original(
            &published,
            &store(&f.path("b")),
            &f.path("b"),
            &Uuid7::parse(&id(3)).unwrap(),
            original.revision()
        )
        .is_err()
    );
    assert!(
        verify_original(
            &published,
            &store(&f.path("b")),
            &f.path("b"),
            &Uuid7::parse(&id(3)).unwrap(),
            draft.revision()
        )
        .is_err()
    );
    assert!(
        verify_original(
            &published,
            &source,
            &f.path("b"),
            &Uuid7::parse(&id(3)).unwrap(),
            original.revision()
        )
        .is_err()
    );
    assert_eq!(f.head("b").manifest_digest(), draft.manifest_digest());
}

#[test]
fn a_publication_containing_another_parts_change_cannot_claim_a_single_commands_inverse() {
    let f = Fixture::new(false);
    f.update("a", 11, &["--name", "another accepted edit"]);
    f.update("a", 10, &["--name", "last command"]);
    let original = f.head("a");
    f.publish("a");
    let accepted = f.head("published");
    assert_eq!(
        verify_original(
            &store(&f.path("published")),
            &store(&f.path("a")),
            &f.path("a"),
            &Uuid7::parse(&id(3)).unwrap(),
            original.revision()
        )
        .err()
        .unwrap()
        .code,
        ErrorCode::DependencyUnavailable
    );
    assert_eq!(
        f.head("published").manifest_digest(),
        accepted.manifest_digest()
    );
    assert_eq!(f.head("a").manifest_digest(), original.manifest_digest());
}

#[test]
fn a_changeset_publication_uses_its_exact_payload_and_original_source_result() {
    let f = Fixture::new(false);
    f.update("a", 10, &["--name", "accepted through C01"]);
    let original = f.head("a");
    let package = f.ok(&[
        "changeset-create",
        "--store",
        f.s("published"),
        "--workspace-dir",
        f.s("a"),
    ]);
    let digest = text(package.get("changeset").unwrap(), "digest");
    f.ok(&[
        "changeset-apply",
        "--store",
        f.s("published"),
        "--workspace-dir",
        f.s("a"),
        "--actor-id",
        &id(3),
        "--changeset",
        digest,
    ]);
    let accepted = f.head("published");
    assert!(
        verify_original(
            &store(&f.path("published")),
            &store(&f.path("a")),
            &f.path("a"),
            &Uuid7::parse(&id(3)).unwrap(),
            original.revision()
        )
        .is_err()
    );
    let proof = verify_original(
        &store(&f.path("published")),
        &store(&f.path("a")),
        &f.path("a"),
        &Uuid7::parse(&id(3)).unwrap(),
        accepted.revision(),
    )
    .unwrap();
    assert_eq!(
        proof.after().info().manifest_digest(),
        original.manifest_digest()
    );
    assert_eq!(
        proof.published().info().manifest_digest(),
        accepted.manifest_digest()
    );
    drop(proof);
    let prefix: String = digest.chars().take(2).collect();
    let payload = f.path("a").join("blobs").join(prefix).join(digest);
    fs::write(&payload, b"damaged canonical changeset").unwrap();
    assert!(
        verify_original(
            &store(&f.path("published")),
            &store(&f.path("a")),
            &f.path("a"),
            &Uuid7::parse(&id(3)).unwrap(),
            accepted.revision()
        )
        .is_err()
    );
    assert_eq!(fs::read(payload).unwrap(), b"damaged canonical changeset");
    assert_eq!(
        f.head("published").manifest_digest(),
        accepted.manifest_digest()
    );
}

#[test]
fn a_complete_changeset_proves_its_whole_interval_and_accepted_basis() {
    let f = Fixture::new(false);
    let prefix = f.head("a");
    let base = f.head("published");
    f.update("a", 11, &["--name", "first"]);
    f.update("a", 10, &["--name", "second"]);
    let source = f.head("a");
    let package = f.ok(&[
        "changeset-create",
        "--store",
        f.s("published"),
        "--workspace-dir",
        f.s("a"),
    ]);
    let digest = text(package.get("changeset").unwrap(), "digest");
    f.ok(&[
        "changeset-apply",
        "--store",
        f.s("published"),
        "--workspace-dir",
        f.s("a"),
        "--actor-id",
        &id(3),
        "--changeset",
        digest,
    ]);
    let accepted = f.head("published");
    let proof = verify_original(
        &store(&f.path("published")),
        &store(&f.path("a")),
        &f.path("a"),
        &Uuid7::parse(&id(3)).unwrap(),
        accepted.revision(),
    )
    .unwrap();
    assert_eq!(
        proof.before().info().manifest_digest(),
        prefix.manifest_digest()
    );
    assert_eq!(
        proof.after().info().manifest_digest(),
        source.manifest_digest()
    );
    assert_eq!(
        proof.accepted_before().info().manifest_digest(),
        base.manifest_digest()
    );
    assert_ne!(
        proof.before().info().manifest_digest(),
        proof.accepted_before().info().manifest_digest()
    );
    assert_eq!(proof.command().footprint.writes().len(), 2);
    assert_eq!(
        proof.command().op_id.to_string(),
        accepted.manifest().content().op().as_str()
    );
    assert_eq!(
        f.head("published").manifest_digest(),
        accepted.manifest_digest()
    );
}
