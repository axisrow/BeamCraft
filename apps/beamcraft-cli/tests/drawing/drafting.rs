// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D07 slice 1: drawn paper objects live in the document only. The edit command takes no model
//! store at all, published versions survive later edits unchanged, and a refusal keeps the head.

use super::*;

const DRAWING: &str = "01900000-0000-7000-8000-000000000201";
const VIEW: &str = "01900000-0000-7000-8000-000000000202";
const DIMENSION: &str = "01900000-0000-7000-8000-000000000203";
const ACTOR: &str = "01900000-0000-7000-8000-000000000204";
const OP_CREATE: &str = "01900000-0000-7000-8000-000000000205";
const REV_CREATE: &str = "01900000-0000-7000-8000-000000000206";
const OP_EDIT: &str = "01900000-0000-7000-8000-000000000207";
const REV_EDIT: &str = "01900000-0000-7000-8000-000000000208";
const OP_PUBLISH: &str = "01900000-0000-7000-8000-000000000209";
const REV_PUBLISH: &str = "01900000-0000-7000-8000-000000000210";
const OP_AFTER: &str = "01900000-0000-7000-8000-000000000211";
const REV_AFTER: &str = "01900000-0000-7000-8000-000000000212";
const VERSION: &str = "01900000-0000-7000-8000-000000000213";
const LINE: &str = "01900000-0000-7000-8000-000000000214";
const CIRCLE: &str = "01900000-0000-7000-8000-000000000215";
const ARC: &str = "01900000-0000-7000-8000-000000000216";
const RECT: &str = "01900000-0000-7000-8000-000000000217";

fn intent() -> Vec<String> {
    [
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
    ]
    .iter()
    .map(|value| (*value).to_owned())
    .collect()
}

fn model_store(f: &Fixture) -> String {
    f.shared.root.join("store").to_str().unwrap().to_owned()
}

fn argv(options: &[String]) -> Vec<&str> {
    options.iter().map(String::as_str).collect()
}

/// Runs `edit-drafting` with no `--store` argument: the drawn objects must be reachable from the
/// document alone, so the harness that injects the model store is deliberately bypassed.
fn edit(
    document_dir: &str,
    expected_head: &str,
    op: &str,
    revision: &str,
    entities: &[String],
) -> (i32, Json) {
    let mut args = vec![
        "edit-drafting".to_owned(),
        "--document-dir".to_owned(),
        document_dir.to_owned(),
        "--expected-head".to_owned(),
        expected_head.to_owned(),
        "--op-id".to_owned(),
        op.to_owned(),
        "--revision-id".to_owned(),
        revision.to_owned(),
        "--actor-id".to_owned(),
        ACTOR.to_owned(),
    ];
    args.extend(entities.iter().cloned());
    let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .args(&args)
        .output()
        .unwrap();
    let body = beamcraft_json::decode(&output.stdout, &beamcraft_json::Limits::DEFAULT).unwrap();
    (output.status.code().unwrap(), body)
}

#[test]
fn drawn_objects_live_in_the_document_only_and_published_versions_survive_edits() {
    let f = Fixture::new();
    f.model();
    let document_dir = f
        .shared
        .root
        .join("drawing-doc")
        .to_str()
        .unwrap()
        .to_owned();
    let mut create = vec![
        "--document-dir".to_owned(),
        document_dir.clone(),
        "--op-id".to_owned(),
        OP_CREATE.to_owned(),
        "--revision-id".to_owned(),
        REV_CREATE.to_owned(),
        "--actor-id".to_owned(),
        ACTOR.to_owned(),
    ];
    create.extend(intent());
    let head = text(&f.ok("create-drawing", &argv(&create)), "revision_id");

    // The empty set becomes three drawn objects without any model store on the command line.
    let (code, edited) = edit(
        &document_dir,
        &head,
        OP_EDIT,
        REV_EDIT,
        &[
            "--drafting".to_owned(),
            format!("{LINE}:line:1.5,-2,30.25,4"),
            "--drafting".to_owned(),
            format!("{CIRCLE}:circle:10,10,2.5"),
            "--drafting".to_owned(),
            format!("{ARC}:arc:0,0,5,0,1.25"),
        ],
    );
    assert_eq!(code, 0, "{edited:?}");
    let edited_head = text(&edited, "revision_id");
    assert_ne!(edited_head, head);
    let inspected = f.ok(
        "inspect-drawing",
        &["--document-dir", document_dir.as_str()],
    );
    let drawn = list(&inspected, "drafting");
    assert_eq!(drawn.len(), 3);
    assert_eq!(text(&drawn[0], "id"), LINE);
    assert_eq!(text(&drawn[0], "kind"), "line");
    assert_eq!(text(&drawn[1], "id"), CIRCLE);
    assert_eq!(text(&drawn[1], "kind"), "circle");
    assert_eq!(text(&drawn[2], "id"), ARC);
    assert_eq!(text(&drawn[2], "kind"), "arc");
    // The source pin never moved, so the live model is still exactly current.
    assert_eq!(
        text(
            &f.ok(
                "inspect-drawing",
                &[
                    "--document-dir",
                    document_dir.as_str(),
                    "--model-store",
                    model_store(&f).as_str(),
                ]
            ),
            "freshness"
        ),
        "current"
    );

    // A degenerate rectangle refuses and the document head stays exactly where it was.
    let (code, refused) = edit(
        &document_dir,
        &edited_head,
        OP_AFTER,
        REV_AFTER,
        &["--drafting".to_owned(), format!("{RECT}:rectangle:0,0,5,0")],
    );
    assert_eq!(code, 2, "{refused:?}");
    assert_eq!(
        text(
            &f.ok(
                "inspect-drawing",
                &["--document-dir", document_dir.as_str()]
            ),
            "revision_id"
        ),
        edited_head
    );

    // Publication binds one immutable version; a later edit lands as a fresh draft on top.
    let (code, published) = f.run(
        "publish-drawing",
        &argv(&[
            "--document-dir".to_owned(),
            document_dir.clone(),
            "--expected-head".to_owned(),
            edited_head.clone(),
            "--op-id".to_owned(),
            OP_PUBLISH.to_owned(),
            "--revision-id".to_owned(),
            REV_PUBLISH.to_owned(),
            "--actor-id".to_owned(),
            ACTOR.to_owned(),
            "--version-id".to_owned(),
            VERSION.to_owned(),
        ]),
    );
    assert_eq!(code, 0, "{published:?}");
    let published_head = text(&published, "revision_id");
    let (code, after) = edit(
        &document_dir,
        &published_head,
        OP_AFTER,
        REV_AFTER,
        &["--drafting".to_owned(), format!("{RECT}:rectangle:0,0,5,7")],
    );
    assert_eq!(code, 0, "{after:?}");
    let after_head = text(&after, "revision_id");
    let inspected = f.ok(
        "inspect-drawing",
        &["--document-dir", document_dir.as_str()],
    );
    assert_eq!(text(&inspected, "state"), "draft");
    assert_eq!(text(&inspected, "revision_id"), after_head);
    assert_eq!(text(&inspected, "latest_published_version_id"), VERSION);
    assert_eq!(list(&inspected, "drafting").len(), 1);

    // The published version still exports exactly as it did before the edit.
    let before_out = f.shared.root.join("before-edit.svg");
    let (code, exported) = f.run(
        "export-drawing",
        &argv(&[
            "--document-dir".to_owned(),
            document_dir.clone(),
            "--version-id".to_owned(),
            VERSION.to_owned(),
            "--out".to_owned(),
            before_out.to_str().unwrap_or_default().to_owned(),
        ]),
    );
    assert_eq!(code, 0, "{exported:?}");
    let after_out = f.shared.root.join("after-edit.svg");
    let (code, exported_again) = f.run(
        "export-drawing",
        &argv(&[
            "--document-dir".to_owned(),
            document_dir,
            "--version-id".to_owned(),
            VERSION.to_owned(),
            "--out".to_owned(),
            after_out.to_str().unwrap_or_default().to_owned(),
        ]),
    );
    assert_eq!(code, 0, "{exported_again:?}");
    assert_eq!(
        fs::read(&before_out).unwrap(),
        fs::read(&after_out).unwrap()
    );
}

#[test]
fn drafting_replay_binds_the_incoming_geometry_even_after_later_edits() {
    let f = Fixture::new();
    f.model();
    let document_dir = f
        .shared
        .root
        .join("replay-doc")
        .to_str()
        .unwrap()
        .to_owned();
    let mut create = vec![
        "--document-dir".to_owned(),
        document_dir.clone(),
        "--op-id".to_owned(),
        OP_CREATE.to_owned(),
        "--revision-id".to_owned(),
        REV_CREATE.to_owned(),
        "--actor-id".to_owned(),
        ACTOR.to_owned(),
    ];
    create.extend(intent());
    let head = text(&f.ok("create-drawing", &argv(&create)), "revision_id");
    let original = vec!["--drafting".to_owned(), format!("{LINE}:line:0,0,10,0")];
    let changed = vec!["--drafting".to_owned(), format!("{LINE}:line:0,0,20,0")];
    let (code, accepted) = edit(&document_dir, &head, OP_EDIT, REV_EDIT, &original);
    assert_eq!(code, 0, "{accepted:?}");
    let edited_head = text(&accepted, "revision_id");
    let (code, replayed) = edit(&document_dir, &head, OP_EDIT, REV_EDIT, &original);
    assert_eq!(code, 0, "{replayed:?}");
    assert_eq!(replayed, accepted);
    let (code, refused) = edit(&document_dir, &head, OP_EDIT, REV_EDIT, &changed);
    assert_eq!(code, 2, "{refused:?}");
    assert_eq!(text(&refused, "code"), "OP_ID_REUSE");
    assert_eq!(
        text(
            &f.ok("inspect-drawing", &["--document-dir", &document_dir]),
            "revision_id"
        ),
        edited_head
    );

    let (code, later) = edit(&document_dir, &edited_head, OP_AFTER, REV_AFTER, &changed);
    assert_eq!(code, 0, "{later:?}");
    let later_head = text(&later, "revision_id");
    let (code, replayed) = edit(&document_dir, &head, OP_EDIT, REV_EDIT, &original);
    assert_eq!(code, 0, "{replayed:?}");
    assert_eq!(replayed, accepted);
    for entities in [&changed[..], &[][..]] {
        let (code, refused) = edit(&document_dir, &head, OP_EDIT, REV_EDIT, entities);
        assert_eq!(code, 2, "{refused:?}");
        assert_eq!(text(&refused, "code"), "OP_ID_REUSE");
    }
    assert_eq!(
        text(
            &f.ok("inspect-drawing", &["--document-dir", &document_dir]),
            "revision_id"
        ),
        later_head
    );
}
