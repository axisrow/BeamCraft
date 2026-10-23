// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic compensation models, MPL-2.0.

//! C06.2: semantic inverses through the actual CLI, preserving another user's later work.

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
fn compensate(
    f: &Fixture,
    original: &RevisionInfo,
    accepted: &RevisionInfo,
    draft: &RevisionInfo,
) -> (i32, Json) {
    f.run(&[
        "compensate-operation",
        "--store",
        f.s("published"),
        "--workspace-dir",
        f.path("c").to_str().unwrap(),
        "--source-workspace-dir",
        f.s("a"),
        "--original-revision",
        original.revision().as_str(),
        "--op-id",
        &id(500),
        "--expected-head",
        accepted.revision().as_str(),
        "--expected-draft",
        draft.revision().as_str(),
    ])
}

#[test]
fn supported_inverses_keep_another_users_later_fields_until_explicit_publication() {
    for case in ["rename", "thickness", "create", "delete"] {
        let f = Fixture::new(case == "rename");
        match case {
            "create" => {
                f.ok(&[
                    "create-plate",
                    "--store",
                    f.s("a"),
                    "--new-id",
                    &id(12),
                    "--width",
                    "10",
                    "--length",
                    "20",
                    "--thickness",
                    "3",
                ]);
            }
            "delete" => {
                f.ok(&["delete-element", "--store", f.s("a"), "--id", &id(10)]);
            }
            "thickness" => f.update("a", 10, &["--thickness", "20"]),
            _ => f.update("a", 10, &["--name", "original accepted rename"]),
        }
        f.publish("a");
        let original = f.head("published");
        fresh(&f, "other", 4);
        f.update(
            "other",
            11,
            &[
                "--name",
                "another user's later name",
                "--translation",
                "5,6,7",
            ],
        );
        if case == "rename" {
            f.update("other", 10, &["--translation", "100,200,300"]);
        }
        publish(&f, "other");
        let accepted = f.head("published");
        fresh(&f, "c", 3);
        let draft = f.head("c");
        let metadata = fs::read(f.path("c").join("workspace.json")).unwrap();
        let (code, result) = compensate(&f, &original, &accepted, &draft);
        assert_eq!(code, 0, "{case}: {result:?}");
        assert_eq!(text(&result, "outcome"), "operation_compensated");
        let candidate = f.head("c");
        assert_eq!(candidate.sequence(), draft.sequence() + 1);
        assert_eq!(
            f.head("published").manifest_digest(),
            accepted.manifest_digest()
        );
        assert_eq!(
            fs::read(f.path("c").join("workspace.json")).unwrap(),
            metadata
        );
        let (code, replay) = compensate(&f, &original, &accepted, &draft);
        assert_eq!(code, 0, "{replay:?}");
        assert_eq!(text(&replay, "outcome"), "compensation_replayed");
        assert_eq!(f.head("c").manifest_digest(), candidate.manifest_digest());
        publish(&f, "c");
        let report = plate::inspect(&store(&f.path("published"))).unwrap();
        assert_eq!(report.plates.len(), 2, "{case}");
        for (n, name, placement) in [
            (
                10,
                "Plate",
                if case == "rename" {
                    [100.0, 200.0, 300.0]
                } else {
                    [0.0; 3]
                },
            ),
            (11, "another user's later name", [5.0, 6.0, 7.0]),
        ] {
            let part = report
                .plates
                .iter()
                .find(|p| p.authored.id == ElementId::parse(&id(n)).unwrap())
                .unwrap();
            assert_eq!(part.authored.name, name, "{case}");
            assert_eq!(part.authored.placement.translation_mm, placement, "{case}");
            assert_eq!(part.quantities.volume_mm3, 5_000_000.0);
            assert_eq!(part.quantities.mass_kg, Some(39.25));
        }
        let current = store(&f.path("published")).open().unwrap();
        let history = store(&f.path("published"))
            .open_at(original.sequence())
            .unwrap();
        assert_eq!(history.info().manifest_digest(), original.manifest_digest());
        for entry in original
            .manifest()
            .content()
            .entries()
            .iter()
            .filter(|e| e.role() == beamcraft_host::revision::LayerRole::External)
        {
            assert_eq!(
                current.layer_bytes(entry.name()).unwrap(),
                history.layer_bytes(entry.name()).unwrap()
            );
        }
    }
}

#[test]
fn later_writes_reverted_writes_reads_and_links_prevent_automatic_compensation() {
    for case in ["overlap", "reverted", "copy", "link"] {
        let f = Fixture::new(false);
        if case == "link" {
            f.ok(&[
                "create-plate",
                "--store",
                f.s("a"),
                "--new-id",
                &id(12),
                "--width",
                "10",
                "--length",
                "20",
                "--thickness",
                "3",
            ]);
        } else {
            f.update("a", 10, &["--name", "original"]);
        }
        f.publish("a");
        let original = f.head("published");
        fresh(&f, "other", 4);
        match case {
            "link" => {
                f.ok(&[
                    "create-weld",
                    "--store",
                    f.path("other").to_str().unwrap(),
                    "--new-id",
                    &id(13),
                    "--main",
                    &id(12),
                    "--secondary",
                    &id(11),
                    "--name",
                    "later user's weld",
                    "--size",
                    "6",
                    "--location",
                    "workshop",
                    "--side",
                    "both",
                ]);
            }
            "copy" => {
                f.ok(&[
                    "copy-element",
                    "--store",
                    f.path("other").to_str().unwrap(),
                    "--id",
                    &id(10),
                    "--new-id",
                    &id(12),
                ]);
            }
            _ => {
                f.update("other", 10, &["--name", "later"]);
                if case == "reverted" {
                    f.update("other", 10, &["--name", "original"]);
                }
            }
        }
        publish(&f, "other");
        let accepted = f.head("published");
        fresh(&f, "c", 3);
        let draft = f.head("c");
        let metadata = fs::read(f.path("c").join("workspace.json")).unwrap();
        let (code, result) = compensate(&f, &original, &accepted, &draft);
        assert_eq!(code, 2, "{case}: {result:?}");
        assert_eq!(text(&result, "code"), "DEPENDENCY_CONFLICT");
        let diagnostics = result.get("diagnostics").unwrap().array().unwrap();
        assert!(diagnostics.iter().any(|d| text(d, "target")
            == id(if case == "link" { 12 } else { 10 })
            && text(d, "group") == if case == "link" { "lifecycle" } else { "label" }));
        assert_eq!(compensate(&f, &original, &accepted, &draft), (2, result));
        assert_eq!(
            f.head("published").manifest_digest(),
            accepted.manifest_digest()
        );
        assert_eq!(f.head("c").manifest_digest(), draft.manifest_digest());
        assert_eq!(
            fs::read(f.path("c").join("workspace.json")).unwrap(),
            metadata
        );
        assert!(!f.path("c").join("rebase.pending").exists());
    }
}
