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
    let document_dir = f.root.join("drawing-doc");
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
    let created = f.ok("drawing-create", &create_args);
    assert_eq!(text(&created, "state"), "draft");
    let expected_head = text(&created, "revision_id").to_owned();
    let replayed_create = f.ok("drawing-create", &create_args);
    assert_eq!(text(&replayed_create, "revision_id"), expected_head);
    let mut changed_create = create_args.clone();
    changed_create.extend_from_slice(&["--name", "Different title"]);
    let (exit, refusal) = f.run("drawing-create", &changed_create);
    assert_eq!(exit, 2);
    assert_eq!(text(&refusal, "code"), "OP_ID_REUSE");
    let inspected = f.ok(
        "drawing-inspect",
        &[
            "--document-dir",
            document_path,
            "--model-store",
            f.root.join("store").to_str().unwrap(),
        ],
    );
    assert_eq!(text(&inspected, "freshness"), "current");
    let published = f.ok(
        "drawing-publish",
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
    let refreshed = f.ok("drawing-refresh", &refresh_args);
    assert_eq!(text(&refreshed, "state"), "draft");
    assert_eq!(text(&refreshed, "latest_published_version_id"), VERSION);
    let replayed_publish = f.ok(
        "drawing-publish",
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
    let history = f.ok("drawing-versions", &["--document-dir", document_path]);
    let versions = list(&history, "versions");
    assert_eq!(versions.len(), 1);
    assert_eq!(text(&versions[0], "state"), "published");
    assert_eq!(text(&versions[0], "version_id"), VERSION);
}
