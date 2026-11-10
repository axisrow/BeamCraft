// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D03 lifecycle commands against a real source model and the immutable document journal.

use super::*;

const DRAWING: &str = "01900000-0000-7000-8000-000000000001";
const VIEW: &str = "01900000-0000-7000-8000-000000000002";
const DIMENSION: &str = "01900000-0000-7000-8000-000000000003";
const ACTOR: &str = "01900000-0000-7000-8000-000000000004";
const OP1: &str = "01900000-0000-7000-8000-000000000005";
const REV1: &str = "01900000-0000-7000-8000-000000000006";
const OP2: &str = "01900000-0000-7000-8000-000000000007";
const REV2: &str = "01900000-0000-7000-8000-000000000008";
const VERSION: &str = "01900000-0000-7000-8000-000000000009";
const OP3: &str = "01900000-0000-7000-8000-000000000010";
const REV3: &str = "01900000-0000-7000-8000-000000000011";

#[test]
fn document_cli_creates_publishes_inspects_and_lists_versions() {
    let f = Fixture::new();
    f.model();
    let document_dir = f.shared.root.join("drawing-doc");
    let document_path = document_dir.to_str().unwrap();
    let intent = [
        "--drawing-id",
        DRAWING,
        "--view-id",
        VIEW,
        "--dimension-id",
        DIMENSION,
        "--id",
        MEMBER,
        "--anchor-a",
        &format!("fid1:{MEMBER}:member:v.x0.y0.z0"),
        "--anchor-b",
        &format!("fid1:{MEMBER}:member:v.x0.y0.z1"),
    ];
    let mut create_args = vec!["--document-dir", document_path];
    create_args.extend_from_slice(&intent);
    create_args.extend_from_slice(&["--op-id", OP1, "--revision-id", REV1, "--actor-id", ACTOR]);
    let created = f.ok("create-drawing", &create_args);
    assert_eq!(text(&created, "state"), "draft");
    let expected_head = text(&created, "revision_id").to_owned();
    let replayed_create = f.ok("create-drawing", &create_args);
    assert_eq!(text(&replayed_create, "revision_id"), expected_head);
    let mut changed_create = create_args.clone();
    changed_create.extend_from_slice(&["--name", "Different title"]);
    let (exit, refusal) = f.run("create-drawing", &changed_create);
    assert_eq!(exit, 2);
    assert_eq!(text(&refusal, "code"), "OP_ID_REUSE");
    let inspected = f.ok(
        "inspect-drawing",
        &[
            "--document-dir",
            document_path,
            "--model-store",
            f.shared.root.join("store").to_str().unwrap(),
        ],
    );
    assert_eq!(text(&inspected, "freshness"), "current");
    let published = f.ok(
        "publish-drawing",
        &[
            "--document-dir",
            document_path,
            "--expected-head",
            &expected_head,
            "--op-id",
            OP2,
            "--revision-id",
            REV2,
            "--actor-id",
            ACTOR,
            "--version-id",
            VERSION,
        ],
    );
    assert_eq!(text(&published, "state"), "published");
    assert_eq!(text(&published, "version_id"), VERSION);
    let published_head = text(&published, "revision_id").to_owned();
    let mut refresh_args = vec![
        "--document-dir",
        document_path,
        "--expected-head",
        &published_head,
    ];
    refresh_args.extend_from_slice(&intent);
    refresh_args.extend_from_slice(&["--op-id", OP3, "--revision-id", REV3, "--actor-id", ACTOR]);
    let refreshed = f.ok("refresh-drawing", &refresh_args);
    assert_eq!(text(&refreshed, "state"), "draft");
    assert_eq!(text(&refreshed, "latest_published_version_id"), VERSION);
    let replayed_publish = f.ok(
        "publish-drawing",
        &[
            "--document-dir",
            document_path,
            "--expected-head",
            &expected_head,
            "--op-id",
            OP2,
            "--revision-id",
            REV2,
            "--actor-id",
            ACTOR,
            "--version-id",
            VERSION,
        ],
    );
    assert_eq!(text(&replayed_publish, "revision_id"), published_head);
    let history = f.ok(
        "inspect-drawing-versions",
        &["--document-dir", document_path],
    );
    let versions = list(&history, "versions");
    assert_eq!(versions.len(), 1);
    assert_eq!(text(&versions[0], "state"), "published");
    assert_eq!(text(&versions[0], "version_id"), VERSION);

    // Publish another version, then try the historical ID again from a fresh draft. Comparing
    // only latest_published misses this V1 -> V2 -> V1 reuse across separate CLI processes.
    let second_version = "01900000-0000-7000-8000-000000000012";
    let refreshed_head = text(&refreshed, "revision_id");
    let second_published = f.ok(
        "publish-drawing",
        &[
            "--document-dir",
            document_path,
            "--expected-head",
            &refreshed_head,
            "--op-id",
            "01900000-0000-7000-8000-000000000013",
            "--revision-id",
            "01900000-0000-7000-8000-000000000014",
            "--actor-id",
            ACTOR,
            "--version-id",
            second_version,
        ],
    );
    let second_published_head = text(&second_published, "revision_id");
    let mut next_refresh = vec![
        "--document-dir",
        document_path,
        "--expected-head",
        &second_published_head,
        "--name",
        "Changed drawing",
        "--op-id",
        "01900000-0000-7000-8000-000000000015",
        "--revision-id",
        "01900000-0000-7000-8000-000000000016",
        "--actor-id",
        ACTOR,
    ];
    next_refresh.extend_from_slice(&intent);
    let next_draft = f.ok("refresh-drawing", &next_refresh);
    let next_head = text(&next_draft, "revision_id");
    let before_history = f.ok(
        "inspect-drawing-versions",
        &["--document-dir", document_path],
    );
    for reused in [VERSION, second_version] {
        let (exit, refusal) = f.run(
            "publish-drawing",
            &[
                "--document-dir",
                document_path,
                "--expected-head",
                &next_head,
                "--op-id",
                "01900000-0000-7000-8000-000000000017",
                "--revision-id",
                "01900000-0000-7000-8000-000000000018",
                "--actor-id",
                ACTOR,
                "--version-id",
                reused,
            ],
        );
        assert_eq!(exit, 2, "historical version reuse: {refusal:?}");
        assert_eq!(text(&refusal, "code"), "DUPLICATE_ID");
        let after = f.ok("inspect-drawing", &["--document-dir", document_path]);
        assert_eq!(
            text(&after, "revision_id"),
            text(&next_draft, "revision_id")
        );
        assert_eq!(
            f.ok(
                "inspect-drawing-versions",
                &["--document-dir", document_path]
            ),
            before_history
        );
    }
    // Replaying the original accepted operation remains safe after newer publications.
    let replayed = f.ok(
        "publish-drawing",
        &[
            "--document-dir",
            document_path,
            "--expected-head",
            &expected_head,
            "--op-id",
            OP2,
            "--revision-id",
            REV2,
            "--actor-id",
            ACTOR,
            "--version-id",
            VERSION,
        ],
    );
    assert_eq!(text(&replayed, "revision_id"), published_head);
    assert_eq!(
        f.ok(
            "inspect-drawing-versions",
            &["--document-dir", document_path]
        ),
        before_history
    );
    // A refusal must not consume a revision or prevent a subsequent valid publication.
    let third = f.ok(
        "publish-drawing",
        &[
            "--document-dir",
            document_path,
            "--expected-head",
            &next_head,
            "--op-id",
            "01900000-0000-7000-8000-000000000017",
            "--revision-id",
            "01900000-0000-7000-8000-000000000018",
            "--actor-id",
            ACTOR,
            "--version-id",
            "01900000-0000-7000-8000-000000000019",
        ],
    );
    assert_eq!(text(&third, "state"), "published");
    let final_history = f.ok(
        "inspect-drawing-versions",
        &["--document-dir", document_path],
    );
    let final_versions = list(&final_history, "versions");
    assert_eq!(final_versions.len(), 3);
    assert_eq!(&final_versions[..2], list(&before_history, "versions"));
}

/// A refresh recaptures the source pin and re-authors the primary intent, but the drawn
/// objects and their layers are the document's own (D07) and survive it, while the placed
/// views drop: a refreshed document carries only the views its caller named (d01 §3, §6b).
#[test]
fn a_refresh_carries_drawn_objects_and_layers_and_drops_placements() {
    let f = Fixture::new();
    f.model();
    let document_dir = f.shared.root.join("refresh-doc");
    let document_path = document_dir.to_str().unwrap();
    let layer = "01900000-0000-7000-8000-000000000301";
    let text_id = "01900000-0000-7000-8000-000000000302";
    let circle = "01900000-0000-7000-8000-000000000303";
    let line = "01900000-0000-7000-8000-000000000304";
    let plan = "01900000-0000-7000-8000-000000000305";
    let anchor_a = format!("fid1:{MEMBER}:member:v.x0.y0.z0");
    let anchor_b = format!("fid1:{MEMBER}:member:v.x0.y0.z1");
    let intent = [
        "--drawing-id",
        DRAWING,
        "--view-id",
        VIEW,
        "--dimension-id",
        DIMENSION,
        "--id",
        MEMBER,
        "--anchor-a",
        anchor_a.as_str(),
        "--anchor-b",
        anchor_b.as_str(),
        "--origin",
        "20,60",
        "--scale",
        "25",
        "--view-title",
        "BEAM B1",
    ];
    let mut create = vec!["--document-dir", document_path];
    create.extend_from_slice(&intent);
    create.extend_from_slice(&["--op-id", OP1, "--revision-id", REV1, "--actor-id", ACTOR]);
    let mut head = text(&f.ok("create-drawing", &create), "revision_id");
    let drafted = f.ok(
        "edit-drafting",
        &[
            "--document-dir",
            document_path,
            "--expected-head",
            &head,
            "--op-id",
            "01900000-0000-7000-8000-000000000306",
            "--revision-id",
            "01900000-0000-7000-8000-000000000307",
            "--actor-id",
            ACTOR,
            "--layer",
            &format!("{layer}:Annot:on"),
            "--drafting",
            &format!("{text_id}:text:20,20,6,0:{layer}:continuous:FIELD WELD ALL ROUND"),
            "--drafting",
            &format!("{circle}:circle:200,25,8:{layer}:continuous"),
            "--drafting",
            &format!("{line}:line:15,15,190,15:{layer}:dashed"),
        ],
    );
    head = text(&drafted, "revision_id");
    let placed = f.ok(
        "edit-views",
        &[
            "--document-dir",
            document_path,
            "--expected-head",
            &head,
            "--op-id",
            "01900000-0000-7000-8000-000000000308",
            "--revision-id",
            "01900000-0000-7000-8000-000000000309",
            "--actor-id",
            ACTOR,
            "--placed-view",
            &format!("{plan}:top:20:30,250:PLAN"),
        ],
    );
    head = text(&placed, "revision_id");
    let seen = f.ok("inspect-drawing", &["--document-dir", document_path]);
    assert_eq!(list(&seen, "drafting").len(), 3);
    assert_eq!(list(&seen, "layers").len(), 1);
    assert_eq!(list(&seen, "placed_views").len(), 1);

    // The refresh re-authors the primary intent (a new scale and title take effect) and the
    // reply already reports the carried paper families and the dropped placements.
    let mut refresh_args = vec!["--document-dir", document_path, "--expected-head", &head];
    refresh_args.extend_from_slice(&intent[..intent.len() - 6]);
    refresh_args.extend_from_slice(&[
        "--origin",
        "20,60",
        "--scale",
        "40",
        "--view-title",
        "BEAM B1 REV 2",
        "--op-id",
        "01900000-0000-7000-8000-00000000030a",
        "--revision-id",
        "01900000-0000-7000-8000-00000000030b",
        "--actor-id",
        ACTOR,
    ]);
    let refreshed = f.ok("refresh-drawing", &refresh_args);
    assert_eq!(text(&refreshed, "state"), "draft");
    assert_eq!(list(&refreshed, "drafting").len(), 3);
    assert_eq!(list(&refreshed, "layers").len(), 1);
    assert_eq!(list(&refreshed, "placed_views").len(), 0);
    assert_eq!(text(at(&refreshed, &["view"]), "title"), "BEAM B1 REV 2");
    let document = f.ok("inspect-drawing", &["--document-dir", document_path]);
    assert_eq!(list(&document, "drafting").len(), 3);
    assert_eq!(list(&document, "layers").len(), 1);
    assert_eq!(list(&document, "placed_views").len(), 0);
    let kinds: Vec<String> = list(&document, "drafting")
        .iter()
        .map(|entity| text(entity, "kind"))
        .collect();
    assert_eq!(kinds, ["text", "circle", "line"]);
    assert_eq!(text(&document, "name"), "Member drawing");
}
