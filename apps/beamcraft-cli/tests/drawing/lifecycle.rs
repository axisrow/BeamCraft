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
