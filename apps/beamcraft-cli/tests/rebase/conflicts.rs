// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic conflict scenarios, MPL-2.0.

//! C04: complete diagnostics and preservation through the real CLI, in both dependency directions.

use super::*;

fn layers(f: &Fixture, name: &str) -> Vec<(String, Vec<u8>)> {
    let revision = store(&f.path(name)).open().unwrap();
    revision
        .manifest()
        .content()
        .entries()
        .iter()
        .map(|entry| {
            (
                entry.name().into(),
                revision.layer_bytes(entry.name()).unwrap(),
            )
        })
        .collect()
}

/// Reopen in fresh CLI processes, compare complete layers and metadata, and verify every reported
/// operation identity rather than just checking that some conflict message was printed.
fn preserved_conflict(f: &Fixture, target: usize, group: &str) -> Vec<Json> {
    let accepted = f.head("published");
    let draft = f.head("b");
    let before_accepted = layers(f, "published");
    let before_draft = layers(f, "b");
    let before_a = layers(f, "a");
    let metadata = fs::read(f.path("b").join("workspace.json")).unwrap();
    let (code, result) = f.rebase(500, &accepted, &draft);
    assert_eq!(code, 2, "{result:?}");
    assert_eq!(text(&result, "code"), "DEPENDENCY_CONFLICT");
    let diagnostics = result.get("diagnostics").unwrap().array().unwrap().to_vec();
    assert!(!diagnostics.is_empty());
    assert!(
        diagnostics
            .iter()
            .any(|d| text(d, "target") == id(target) && text(d, "group") == group)
    );
    for diagnostic in &diagnostics {
        assert_eq!(text(diagnostic, "target_kind"), "element");
        assert_eq!(
            text(diagnostic, "draft_op"),
            draft.manifest().content().op().as_str()
        );
        // A verified publication is one aggregate operation in accepted history. Its identity
        // differs from the original command's identity in the source workspace.
        assert_eq!(
            text(diagnostic, "accepted_op"),
            accepted.manifest().content().op().as_str()
        );
    }
    assert_eq!(f.rebase(500, &accepted, &draft), (2, result));
    assert_eq!(
        f.head("published").manifest_digest(),
        accepted.manifest_digest()
    );
    assert_eq!(f.head("b").manifest_digest(), draft.manifest_digest());
    assert_eq!(
        fs::read(f.path("b").join("workspace.json")).unwrap(),
        metadata
    );
    assert_eq!(layers(f, "published"), before_accepted);
    assert_eq!(layers(f, "b"), before_draft);
    assert_eq!(layers(f, "a"), before_a);
    assert!(!f.path("b").join("rebase.pending").exists());
    // Even a refused attempt is bound: an explicit changed payload is not the same request.
    let (code, reused) = f.rebase(500, &draft, &draft);
    assert_eq!(code, 2, "{reused:?}");
    assert_eq!(text(&reused, "code"), "OP_ID_REUSE");
    diagnostics
}

#[test]
fn accepted_copy_reads_prevent_a_draft_source_change_but_not_an_unrelated_change() {
    for same_source in [true, false] {
        let f = Fixture::new(false);
        f.ok(&[
            "copy-element",
            "--store",
            f.s("a"),
            "--id",
            &id(10),
            "--new-id",
            &id(12),
        ]);
        f.update(
            "b",
            if same_source { 10 } else { 11 },
            &["--name", "local name"],
        );
        f.publish("a");
        if same_source {
            let diagnostics = preserved_conflict(&f, 10, "label");
            assert_eq!(
                diagnostics.len(),
                1,
                "read/write directions must not duplicate a conflict"
            );
            let accepted = plate::inspect(&store(&f.path("published"))).unwrap();
            let draft = plate::inspect(&store(&f.path("b"))).unwrap();
            assert_eq!(accepted.plates.len(), 3);
            assert_eq!(draft.plates.len(), 2);
            assert_eq!(
                draft
                    .plates
                    .iter()
                    .find(|p| p.authored.id == ElementId::parse(&id(10)).unwrap())
                    .unwrap()
                    .authored
                    .name,
                "local name"
            );
        } else {
            let (code, result) = f.rebase(500, &f.head("published"), &f.head("b"));
            assert_eq!(code, 0, "{result:?}");
            f.publish("b");
            assert_eq!(
                plate::inspect(&store(&f.path("published")))
                    .unwrap()
                    .plates
                    .len(),
                3
            );
        }
    }
}

#[test]
fn accepted_weld_reads_prevent_draft_deletion_of_its_linked_part() {
    let f = Fixture::new(false);
    f.ok(&[
        "create-weld",
        "--store",
        f.s("a"),
        "--new-id",
        &id(12),
        "--main",
        &id(10),
        "--secondary",
        &id(11),
        "--size",
        "6",
        "--location",
        "workshop",
        "--side",
        "both",
        "--name",
        "Accepted weld",
    ]);
    f.ok(&["delete-element", "--store", f.s("b"), "--id", &id(10)]);
    f.publish("a");
    preserved_conflict(&f, 10, "lifecycle");
    assert_eq!(
        plate::inspect(&store(&f.path("b"))).unwrap().plates.len(),
        1
    );
    assert_eq!(
        plate::inspect(&store(&f.path("published")))
            .unwrap()
            .plates
            .len(),
        2
    );
    let accepted = f.ok(&["welds", "--store", f.s("published")]);
    let draft = f.ok(&["welds", "--store", f.s("b")]);
    assert_eq!(accepted.get("welds").unwrap().array().unwrap().len(), 1);
    assert!(draft.get("welds").unwrap().array().unwrap().is_empty());
}

#[test]
fn the_same_new_id_from_two_clients_conflicts_without_replacing_either_part() {
    let f = Fixture::new(false);
    for (name, width) in [("a", "10"), ("b", "20")] {
        f.ok(&[
            "create-plate",
            "--store",
            f.s(name),
            "--new-id",
            &id(12),
            "--name",
            name,
            "--width",
            width,
            "--length",
            "30",
            "--thickness",
            "5",
        ]);
    }
    f.publish("a");
    preserved_conflict(&f, 12, "lifecycle");
    for (name, width, label) in [("published", 10.0, "a"), ("b", 20.0, "b")] {
        let report = plate::inspect(&store(&f.path(name))).unwrap();
        let part = report
            .plates
            .iter()
            .find(|p| p.authored.id == ElementId::parse(&id(12)).unwrap())
            .unwrap();
        assert_eq!(part.authored.name, label);
        assert_eq!(part.quantities.volume_mm3, width * 30.0 * 5.0);
    }
}

#[test]
fn distinct_final_labels_report_one_complete_pair_and_preserve_both_values() {
    let f = Fixture::new(false);
    f.update("a", 10, &["--name", "accepted"]);
    f.update("b", 10, &["--name", "unsent"]);
    f.publish("a");
    let diagnostics = preserved_conflict(&f, 10, "label");
    assert_eq!(diagnostics.len(), 1);
    for (name, label) in [("published", "accepted"), ("b", "unsent")] {
        let report = plate::inspect(&store(&f.path(name))).unwrap();
        assert_eq!(
            report
                .plates
                .iter()
                .find(|p| p.authored.id == ElementId::parse(&id(10)).unwrap())
                .unwrap()
                .authored
                .name,
            label
        );
    }
}
