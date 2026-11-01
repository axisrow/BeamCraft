// SPDX-License-Identifier: MPL-2.0

//! D05 sheets through the real CLI: a document laid on a supported sheet renders the same
//! output bytes every time, the sheet survives the wire and every unsupported size or
//! malformed request refuses without committing.

use super::*;

const DRAWING: &str = "01900000-0000-7000-8000-000000000321";
const VIEW: &str = "01900000-0000-7000-8000-000000000322";
const DIMENSION: &str = "01900000-0000-7000-8000-000000000323";
const ACTOR: &str = "01900000-0000-7000-8000-000000000324";
const OP_CREATE: &str = "01900000-0000-7000-8000-000000000325";
const REV_CREATE: &str = "01900000-0000-7000-8000-000000000326";
const OP_VIEWS: &str = "01900000-0000-7000-8000-000000000327";
const REV_VIEWS: &str = "01900000-0000-7000-8000-000000000328";
const TOP: &str = "01900000-0000-7000-8000-000000000329";
const OP_SHEET: &str = "01900000-0000-7000-8000-000000000330";
const REV_SHEET: &str = "01900000-0000-7000-8000-000000000331";
const OP_SHEET2: &str = "01900000-0000-7000-8000-000000000332";
const REV_SHEET2: &str = "01900000-0000-7000-8000-000000000333";
const OP_REMOVE: &str = "01900000-0000-7000-8000-000000000334";
const REV_REMOVE: &str = "01900000-0000-7000-8000-000000000335";
const OP_REMOVE2: &str = "01900000-0000-7000-8000-000000000336";
const REV_REMOVE2: &str = "01900000-0000-7000-8000-000000000337";
const VERSION: &str = "01900000-0000-7000-8000-000000000338";
const VERSION2: &str = "01900000-0000-7000-8000-000000000339";
const VERSION3: &str = "01900000-0000-7000-8000-000000000340";

/// Standard document arguments of this suite; `extra` carries the per-case sheet options.
fn base_args(document_dir: &str, command: &str, head: Option<&str>) -> Vec<String> {
    let mut args = vec![
        command.to_owned(),
        "--document-dir".to_owned(),
        document_dir.to_owned(),
    ];
    if let Some(head) = head {
        args.extend(["--expected-head".to_owned(), head.to_owned()]);
    }
    args.extend([
        "--op-id".to_owned(),
        "unused".to_owned(),
        "--revision-id".to_owned(),
        "unused".to_owned(),
        "--actor-id".to_owned(),
        ACTOR.to_owned(),
    ]);
    args
}

fn create_args(document_dir: &str) -> Vec<String> {
    let args = vec![
        "create-drawing".to_owned(),
        "--document-dir".to_owned(),
        document_dir.to_owned(),
        "--drawing-id".to_owned(),
        DRAWING.to_owned(),
        "--view-id".to_owned(),
        VIEW.to_owned(),
        "--dimension-id".to_owned(),
        DIMENSION.to_owned(),
        "--id".to_owned(),
        MEMBER.to_owned(),
        "--anchor-a".to_owned(),
        format!("fid1:{MEMBER}:member:v.x0.y0.z0"),
        "--anchor-b".to_owned(),
        format!("fid1:{MEMBER}:member:v.x0.y0.z1"),
        "--origin".to_owned(),
        "30,40".to_owned(),
        "--op-id".to_owned(),
        OP_CREATE.to_owned(),
        "--revision-id".to_owned(),
        REV_CREATE.to_owned(),
        "--actor-id".to_owned(),
        ACTOR.to_owned(),
    ];
    args
}

/// Runs an `edit-*` command with fresh op/revision ids substituted into `base_args`.
fn edit(
    f: &Fixture,
    command: &str,
    document_dir: &str,
    head: &str,
    op: &str,
    revision: &str,
    extra: &[&str],
) -> (i32, Json) {
    let mut args = base_args(document_dir, command, Some(head));
    let op_at = args.iter().position(|value| value == "--op-id").unwrap() + 1;
    args[op_at] = op.to_owned();
    let rev_at = args
        .iter()
        .position(|value| value == "--revision-id")
        .unwrap()
        + 1;
    args[rev_at] = revision.to_owned();
    args.extend(extra.iter().map(|value| (*value).to_owned()));
    f.run(
        &args[0],
        &args[1..].iter().map(String::as_str).collect::<Vec<_>>(),
    )
}

fn sheet_field(json: &Json, key: &str) -> String {
    text(at(json, &["sheet"]), key)
}

#[test]
fn a_sized_sheet_reproduces_its_output_and_travels_the_wire() {
    let f = Fixture::new();
    f.model();
    let document_dir = f.shared.root.join("sheet-doc").to_str().unwrap().to_owned();
    // A document is created already laid on a supported sheet.
    let mut created = create_args(&document_dir);
    created.extend(
        [
            "--sheet-size",
            "a3",
            "--sheet-project",
            "Bridge",
            "--sheet-author",
            "Drafter",
        ]
        .iter()
        .map(|value| (*value).to_owned()),
    );
    let created = f.ok(
        &created[0],
        &created[1..].iter().map(String::as_str).collect::<Vec<_>>(),
    );
    assert_eq!(sheet_field(&created, "size"), "a3");
    assert_eq!(sheet_field(&created, "project"), "Bridge");
    assert_eq!(text(&created, "name"), "Member drawing");
    assert_eq!(text(at(&created, &["view"]), "id"), VIEW);

    // One placed view beside the primary view.
    let head = text(&created, "revision_id");
    let (_, placed) = edit(
        &f,
        "edit-views",
        &document_dir,
        &head,
        OP_VIEWS,
        REV_VIEWS,
        &["--placed-view", &format!("{TOP}:top:100:200,60")],
    );
    let views_head = text(&placed, "revision_id");

    // The published version renders the same bytes on every run, in a fresh process each
    // time: the whole sheet is reproducible output.
    let (_, published) = edit(
        &f,
        "publish-drawing",
        &document_dir,
        &views_head,
        OP_SHEET,
        REV_SHEET,
        &["--version-id", VERSION],
    );
    let views_head = text(&published, "revision_id");
    let first_out = f.shared.root.join("sheet-one.svg");
    let second_out = f.shared.root.join("sheet-two.svg");
    for out in [&first_out, &second_out] {
        f.ok(
            "export-drawing",
            &[
                "--document-dir",
                &document_dir,
                "--version-id",
                VERSION,
                "--out",
                out.to_str().unwrap(),
            ],
        );
    }
    let first = fs::read(&first_out).unwrap();
    let second = fs::read(&second_out).unwrap();
    assert_eq!(first, second);
    let text_out = String::from_utf8(first).unwrap();
    assert!(text_out.starts_with("<?xml version=\"1.0\""));
    assert!(text_out.contains("sheet=a3"));
    assert!(text_out.contains("class=\"frame\""));
    assert!(text_out.contains("class=\"titleblock\""));
    assert!(text_out.contains(&format!("data-view=\"{TOP}\"")));
    // The title block text is contributor-original strokes; no font text is embedded.
    assert!(!text_out.contains("<text"));

    // The pdf print honours the document's sheet size without --sheet.
    let pdf_out = f.shared.root.join("sheet.pdf");
    f.ok(
        "export-drawing",
        &[
            "--document-dir",
            &document_dir,
            "--version-id",
            VERSION,
            "--format",
            "pdf",
            "--out",
            pdf_out.to_str().unwrap(),
        ],
    );
    let pdf = String::from_utf8(fs::read(&pdf_out).unwrap()).unwrap();
    assert!(pdf.contains("/MediaBox [0 0 841.8898 1190.5512]"));
    assert!(pdf.contains("sheet=a3"));

    // Replacing the sheet is a whole-document operation with the usual replay discipline.
    let (_, replaced) = edit(
        &f,
        "edit-sheet",
        &document_dir,
        &views_head,
        OP_SHEET2,
        REV_SHEET2,
        &[
            "--sheet-size",
            "a2",
            "--sheet-project",
            "Harbour",
            "--sheet-author",
            "Drafter",
        ],
    );
    let replaced_head = text(&replaced, "revision_id");
    assert_eq!(sheet_field(&replaced, "size"), "a2");
    let (_, replayed) = edit(
        &f,
        "edit-sheet",
        &document_dir,
        &views_head,
        OP_SHEET2,
        REV_SHEET2,
        &[
            "--sheet-size",
            "a2",
            "--sheet-project",
            "Harbour",
            "--sheet-author",
            "Drafter",
        ],
    );
    assert_eq!(text(&replayed, "revision_id"), replaced_head);

    // A changed sheet publishes a different, still reproducible version.
    let (_, republished) = edit(
        &f,
        "publish-drawing",
        &document_dir,
        &replaced_head,
        OP_REMOVE,
        REV_REMOVE,
        &["--version-id", VERSION2],
    );
    let third_out = f.shared.root.join("sheet-three.svg");
    f.ok(
        "export-drawing",
        &[
            "--document-dir",
            &document_dir,
            "--version-id",
            VERSION2,
            "--out",
            third_out.to_str().unwrap(),
        ],
    );
    let third = fs::read(&third_out).unwrap();
    assert_ne!(third, second);
    assert!(String::from_utf8(third).unwrap().contains("sheet=a2"));

    // Taking the sheet off returns the document to the loose-paper export.
    let (_, removed) = edit(
        &f,
        "edit-sheet",
        &document_dir,
        &text(&republished, "revision_id"),
        OP_REMOVE2,
        REV_REMOVE2,
        &["--sheet-remove", "true"],
    );
    assert!(
        at(&removed, &["sheet"]).string().is_err(),
        "the sheet field is null after removal"
    );
    let (_, loose_published) = edit(
        &f,
        "publish-drawing",
        &document_dir,
        &text(&removed, "revision_id"),
        "01900000-0000-7000-8000-000000000341",
        "01900000-0000-7000-8000-000000000342",
        &["--version-id", VERSION3],
    );
    let _ = loose_published;
    let loose_out = f.shared.root.join("loose.svg");
    f.ok(
        "export-drawing",
        &[
            "--document-dir",
            &document_dir,
            "--version-id",
            VERSION3,
            "--out",
            loose_out.to_str().unwrap(),
        ],
    );
    let loose = String::from_utf8(fs::read(&loose_out).unwrap()).unwrap();
    assert!(!loose.contains("class=\"frame\""));
}

#[test]
fn unsupported_sizes_and_malformed_sheet_requests_refuse() {
    let f = Fixture::new();
    f.model();
    let document_dir = f
        .shared
        .root
        .join("sheet-refused")
        .to_str()
        .unwrap()
        .to_owned();
    // An unsupported catalogue size refuses at creation.
    let mut bad = create_args(&document_dir);
    bad.extend(
        [
            "--sheet-size",
            "b4",
            "--sheet-project",
            "X",
            "--sheet-author",
            "Y",
        ]
        .iter()
        .map(|value| (*value).to_owned()),
    );
    let (code, refusal) = f.run(
        &bad[0],
        &bad[1..].iter().map(String::as_str).collect::<Vec<_>>(),
    );
    assert_eq!(code, 2, "{refusal:?}");
    assert_eq!(text(&refusal, "code"), "INVALID_ARGUMENT");

    // The refused creation committed nothing, so a clean document still starts here.
    let clean = create_args(&document_dir);
    let created = f.ok(
        &clean[0],
        &clean[1..].iter().map(String::as_str).collect::<Vec<_>>(),
    );
    let head = text(&created, "revision_id");
    for (extra, expected_code) in [
        (
            vec![
                "--sheet-size",
                "letter",
                "--sheet-project",
                "X",
                "--sheet-author",
                "Y",
            ],
            "INVALID_ARGUMENT",
        ),
        (vec!["--sheet-size", "a4"], "INVALID_ARGUMENT"),
        (
            vec![
                "--sheet-size",
                "a4",
                "--sheet-project",
                "X",
                "--sheet-author",
                "Y",
                "--sheet-remove",
                "true",
            ],
            "INVALID_ARGUMENT",
        ),
        (vec!["--sheet-remove", "maybe"], "INVALID_ARGUMENT"),
        (vec![], "INVALID_ARGUMENT"),
    ] {
        let (code, refusal) = edit(
            &f,
            "edit-sheet",
            &document_dir,
            &head,
            "01900000-0000-7000-8000-000000000343",
            "01900000-0000-7000-8000-000000000344",
            &extra,
        );
        assert_eq!(code, 2, "{extra:?}: {refusal:?}");
        assert_eq!(text(&refusal, "code"), expected_code);
    }

    // Content that does not fit the frame refuses instead of being clipped. The primary
    // view sits at paper origin (0,0), under and left of any frame inset.
    let tight_dir = f
        .shared
        .root
        .join("sheet-tight-doc")
        .to_str()
        .unwrap()
        .to_owned();
    let mut tight = create_args(&tight_dir);
    let origin_at = tight.iter().position(|value| value == "--origin").unwrap() + 1;
    tight[origin_at] = "0,0".to_owned();
    tight.extend(
        [
            "--sheet-size",
            "a4",
            "--sheet-project",
            "X",
            "--sheet-author",
            "Y",
        ]
        .iter()
        .map(|value| (*value).to_owned()),
    );
    let tight_created = f.ok(
        &tight[0],
        &tight[1..].iter().map(String::as_str).collect::<Vec<_>>(),
    );
    let (_, published) = edit(
        &f,
        "publish-drawing",
        &tight_dir,
        &text(&tight_created, "revision_id"),
        "01900000-0000-7000-8000-000000000345",
        "01900000-0000-7000-8000-000000000346",
        &["--version-id", "01900000-0000-7000-8000-000000000347"],
    );
    let _ = published;
    let out = f.shared.root.join("tight.svg");
    let (code, refusal) = f.run(
        "export-drawing",
        &[
            "--document-dir",
            &tight_dir,
            "--version-id",
            "01900000-0000-7000-8000-000000000347",
            "--out",
            out.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 2, "{refusal:?}");
    assert_eq!(text(&refusal, "code"), "LIMIT_EXCEEDED");
    assert!(!out.exists(), "a refused export writes no file");
}
