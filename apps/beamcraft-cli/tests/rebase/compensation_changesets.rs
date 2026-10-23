// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic multi-command and rebased publication fixtures, MPL-2.0.

//! C06: the changeset is one published operation; its inverse uses the actual accepted basis.

use super::*;

fn fresh(f: &Fixture, name: &str, actor: usize) {
    f.ok(&[
        "workspace-create",
        "--store",
        f.s("published"),
        "--workspace-dir",
        f.path(name).to_str().unwrap(),
        "--user-id",
        &id(actor),
    ]);
}
fn publish(f: &Fixture, name: &str) {
    f.ok(&[
        "workspace-publish",
        "--store",
        f.s("published"),
        "--workspace-dir",
        f.path(name).to_str().unwrap(),
    ]);
}
fn changeset(f: &Fixture, name: &str) {
    let package = f.ok(&[
        "changeset-create",
        "--store",
        f.s("published"),
        "--workspace-dir",
        f.path(name).to_str().unwrap(),
    ]);
    let digest = text(package.get("changeset").unwrap(), "digest");
    f.ok(&[
        "changeset-apply",
        "--store",
        f.s("published"),
        "--workspace-dir",
        f.path(name).to_str().unwrap(),
        "--changeset",
        digest,
        "--actor-id",
        &id(3),
    ]);
}
fn inverse(
    f: &Fixture,
    name: &str,
    original: &RevisionInfo,
    accepted: &RevisionInfo,
) -> (i32, Json) {
    fresh(f, "c", 3);
    f.run(&[
        "compensate-operation",
        "--store",
        f.s("published"),
        "--workspace-dir",
        f.path("c").to_str().unwrap(),
        "--source-workspace-dir",
        f.path(name).to_str().unwrap(),
        "--original-revision",
        original.revision().as_str(),
        "--op-id",
        &id(500),
        "--expected-head",
        accepted.revision().as_str(),
        "--expected-draft",
        f.head("c").revision().as_str(),
    ])
}

#[test]
fn a_complete_changeset_inverse_preserves_later_disjoint_fields_and_refuses_overlap() {
    for overlap in [false, true] {
        let f = Fixture::new(true);
        f.update("a", 10, &["--name", "original rename"]);
        f.update("a", 11, &["--thickness", "20"]);
        changeset(&f, "a");
        let original = f.head("published");
        fresh(&f, "other", 4);
        if overlap {
            f.update("other", 11, &["--thickness", "30"]);
        } else {
            f.update("other", 10, &["--translation", "12,34,56"]);
            f.update("other", 11, &["--name", "later owner"]);
        }
        publish(&f, "other");
        let accepted = f.head("published");
        let (code, result) = inverse(&f, "a", &original, &accepted);
        assert_eq!(
            f.head("published").manifest_digest(),
            accepted.manifest_digest()
        );
        if overlap {
            assert_eq!(code, 2, "{result:?}");
            assert_eq!(text(&result, "code"), "DEPENDENCY_CONFLICT");
            assert_eq!(f.head("c").sequence(), 1);
        } else {
            assert_eq!(code, 0, "{result:?}");
            changeset(&f, "c");
            let report = plate::inspect(&store(&f.path("published"))).unwrap();
            for (n, name, placement) in [
                (10, "Plate", [12.0, 34.0, 56.0]),
                (11, "later owner", [0.0; 3]),
            ] {
                let part = report
                    .plates
                    .iter()
                    .find(|p| p.authored.id == ElementId::parse(&id(n)).unwrap())
                    .unwrap();
                assert_eq!(part.authored.name, name);
                assert_eq!(part.authored.placement.translation_mm, placement);
                assert_eq!(part.quantities.volume_mm3, 5_000_000.0);
                assert_eq!(part.quantities.mass_kg, Some(39.25));
            }
            let current = store(&f.path("published")).open().unwrap();
            for entry in original
                .manifest()
                .content()
                .entries()
                .iter()
                .filter(|e| e.role() == beamcraft_host::revision::LayerRole::External)
            {
                assert_eq!(
                    current.layer_bytes(entry.name()).unwrap(),
                    fs::read(f.path("reference.usda")).unwrap()
                );
            }
        }
    }
}

#[test]
fn a_rebased_changeset_inverse_keeps_the_other_clients_accepted_name() {
    let f = Fixture::new(false);
    let prefix = f.head("b");
    f.update("a", 10, &["--name", "other accepted name"]);
    f.update("b", 10, &["--translation", "12,34,56"]);
    f.publish("a");
    let basis = f.head("published");
    assert_eq!(f.rebase(600, &basis, &f.head("b")).0, 0);
    changeset(&f, "b");
    let original = f.head("published");
    let proof = beamcraft_cli::compensation::verify_original(
        &store(&f.path("published")),
        &store(&f.path("b")),
        &f.path("b"),
        &Uuid7::parse(&id(3)).unwrap(),
        original.revision(),
    )
    .unwrap();
    assert_eq!(
        proof.before().info().manifest_digest(),
        prefix.manifest_digest()
    );
    assert_eq!(
        proof.accepted_before().info().manifest_digest(),
        basis.manifest_digest()
    );
    drop(proof);
    fresh(&f, "other", 4);
    f.update("other", 11, &["--name", "later owner"]);
    publish(&f, "other");
    let accepted = f.head("published");
    let (code, result) = inverse(&f, "b", &original, &accepted);
    assert_eq!(code, 0, "{result:?}");
    assert_eq!(
        f.head("published").manifest_digest(),
        accepted.manifest_digest()
    );
    changeset(&f, "c");
    let report = plate::inspect(&store(&f.path("published"))).unwrap();
    for (n, name) in [(10, "other accepted name"), (11, "later owner")] {
        let part = report
            .plates
            .iter()
            .find(|p| p.authored.id == ElementId::parse(&id(n)).unwrap())
            .unwrap();
        assert_eq!(part.authored.name, name);
        assert_eq!(part.authored.placement.translation_mm, [0.0; 3]);
        assert_eq!(part.quantities.volume_mm3, 5_000_000.0);
    }
}
