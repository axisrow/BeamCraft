// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D03 export of one exact published version: deterministic, bound to its archived source
//! snapshot, and non-destructive to the model, the document and the destination.

use super::*;

const DRAWING: &str = "01900000-0000-7000-8000-000000000101";
const VIEW: &str = "01900000-0000-7000-8000-000000000102";
const DIMENSION: &str = "01900000-0000-7000-8000-000000000103";
const ACTOR: &str = "01900000-0000-7000-8000-000000000104";
const VERSION: &str = "01900000-0000-7000-8000-000000000105";
const UNUSED: &str = "01900000-0000-7000-8000-000000000106";
const OP1: &str = "01900000-0000-7000-8000-000000000107";
const REV1: &str = "01900000-0000-7000-8000-000000000108";
const OP2: &str = "01900000-0000-7000-8000-000000000109";
const REV2: &str = "01900000-0000-7000-8000-000000000110";
const OP3: &str = "01900000-0000-7000-8000-000000000111";
const REV3: &str = "01900000-0000-7000-8000-000000000112";
const OP4: &str = "01900000-0000-7000-8000-000000000113";
const REV4: &str = "01900000-0000-7000-8000-000000000114";

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
    .map(str::to_owned)
    .into()
}

fn argv(options: &[String]) -> Vec<&str> {
    options.iter().map(String::as_str).collect()
}

fn create(f: &Fixture, document_dir: &str) -> String {
    let mut options = vec![
        "--document-dir".to_owned(),
        document_dir.to_owned(),
        "--op-id".to_owned(),
        OP1.to_owned(),
        "--revision-id".to_owned(),
        REV1.to_owned(),
        "--actor-id".to_owned(),
        ACTOR.to_owned(),
    ];
    options.extend(intent());
    let created = f.ok("create-drawing", &argv(&options));
    text(&created, "revision_id")
}

fn publish(f: &Fixture, document_dir: &str, expected_head: &str) -> (i32, Json) {
    f.run(
        "publish-drawing",
        &argv(&[
            "--document-dir".to_owned(),
            document_dir.to_owned(),
            "--expected-head".to_owned(),
            expected_head.to_owned(),
            "--op-id".to_owned(),
            OP2.to_owned(),
            "--revision-id".to_owned(),
            REV2.to_owned(),
            "--actor-id".to_owned(),
            ACTOR.to_owned(),
            "--version-id".to_owned(),
            VERSION.to_owned(),
        ]),
    )
}

fn export(f: &Fixture, document_dir: &str, out: &std::path::Path) -> (i32, Json) {
    f.run(
        "export-drawing",
        &argv(&[
            "--document-dir".to_owned(),
            document_dir.to_owned(),
            "--version-id".to_owned(),
            VERSION.to_owned(),
            "--out".to_owned(),
            out.to_str().unwrap_or_default().to_owned(),
        ]),
    )
}

fn model_store(f: &Fixture) -> String {
    f.root.join("store").to_str().unwrap().to_owned()
}

#[test]
fn export_of_one_published_version_is_deterministic_snapshot_bound_and_non_destructive() {
    let f = Fixture::new();
    f.model();
    let document_dir = f.root.join("drawing-doc").to_str().unwrap().to_owned();
    let head = create(&f, &document_dir);
    let (code, published) = publish(&f, &document_dir, &head);
    assert_eq!(code, 0, "{published:?}");
    assert_eq!(text(&published, "state"), "published");

    // The expected numbers come from the model inputs: the 3000 mm member axis spans 30 mm on
    // paper at 1:100 from (0, 2000, 0) to (3000, 2000, 0); the centred 50 mm section puts the
    // anchored height face at world z -25 -> paper y -0.25, and the +10 mm offset line at
    // paper y 9.75.
    let out = f.root.join("version.svg");
    let (code, exported) = export(&f, &document_dir, &out);
    assert_eq!(code, 0, "{exported:?}");
    assert_eq!(text(&exported, "version_id"), VERSION);
    let bytes = fs::read(&out).unwrap();
    let svg = std::str::from_utf8(&bytes).unwrap();
    assert!(svg.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
    assert!(svg.ends_with("</svg>\n"));
    assert!(svg.contains(&format!("version={VERSION}")));
    assert!(svg.contains("label_mm=3000"));
    assert!(svg.contains("L 30.0000 9.7500"));
    for hostile in [
        "<script",
        "<image",
        "<text",
        "<foreignObject",
        "href",
        "url(",
        "@import",
    ] {
        assert!(!svg.contains(hostile));
    }

    // A fresh CLI process republishes the identical bytes for the identical version; each
    // successful export needs a fresh destination by the create-new rule.
    let again = f.root.join("version-again.svg");
    let (code, exported) = export(&f, &document_dir, &again);
    assert_eq!(code, 0, "{exported:?}");
    assert_eq!(fs::read(&again).unwrap(), bytes);

    // The export is bound to the archived snapshot: a later live edit changes freshness,
    // never the bytes.
    f.ok(
        "update-element",
        &["--id", MEMBER, "--translation", "500,2000,0"],
    );
    let after_edit = f.root.join("version-after-edit.svg");
    let (code, exported) = export(&f, &document_dir, &after_edit);
    assert_eq!(code, 0, "{exported:?}");
    assert_eq!(fs::read(&after_edit).unwrap(), bytes);
    let inspected = f.ok(
        "inspect-drawing",
        &[
            "--document-dir",
            &document_dir,
            "--model-store",
            &model_store(&f),
        ],
    );
    assert_eq!(text(&inspected, "freshness"), "stale");

    // Refusals change nothing: an unknown version creates no file, an existing destination is
    // never opened, and the document head stays where it was.
    let before = f.ok("inspect-drawing", &["--document-dir", &document_dir]);
    let missing = f.root.join("missing.svg");
    let (code, refusal) = f.run(
        "export-drawing",
        &argv(&[
            "--document-dir".to_owned(),
            document_dir.clone(),
            "--version-id".to_owned(),
            UNUSED.to_owned(),
            "--out".to_owned(),
            missing.to_str().unwrap().to_owned(),
        ]),
    );
    assert_eq!(code, 2, "{refusal:?}");
    assert_eq!(text(&refusal, "code"), "NOT_FOUND");
    assert!(!missing.exists());
    let (code, refusal) = export(&f, &document_dir, &out);
    assert_eq!(code, 2, "{refusal:?}");
    assert_eq!(text(&refusal, "code"), "INVALID_ARGUMENT");
    assert_eq!(fs::read(&out).unwrap(), bytes);
    assert_eq!(
        f.ok("inspect-drawing", &["--document-dir", &document_dir]),
        before
    );
}

#[test]
fn export_pdf_prints_one_published_version_onto_a_deterministic_sheet() {
    let f = Fixture::new();
    f.model();
    let document_dir = f.root.join("drawing-doc").to_str().unwrap().to_owned();
    let head = create(&f, &document_dir);
    let (code, published) = publish(&f, &document_dir, &head);
    assert_eq!(code, 0, "{published:?}");

    let out = f.root.join("version.pdf");
    let (code, exported) = f.run(
        "export-drawing",
        &argv(&[
            "--document-dir".to_owned(),
            document_dir.clone(),
            "--version-id".to_owned(),
            VERSION.to_owned(),
            "--format".to_owned(),
            "pdf".to_owned(),
            "--sheet".to_owned(),
            "a4".to_owned(),
            "--out".to_owned(),
            out.to_str().unwrap().to_owned(),
        ]),
    );
    assert_eq!(code, 0, "{exported:?}");
    assert_eq!(text(&exported, "format"), "pdf");
    assert_eq!(text(&exported, "version_id"), VERSION);
    let bytes = fs::read(&out).unwrap();
    let pdf = std::str::from_utf8(&bytes).unwrap();
    assert!(pdf.starts_with("%PDF-1.4\n"));
    assert!(pdf.ends_with("%%EOF\n"));
    assert!(pdf.contains("/MediaBox [0 0 595.2756 841.8898]"));
    // Fonts and symbols are our own: no font object exists, the label glyphs are strokes.
    assert!(!pdf.contains("/Font"));
    assert!(pdf.contains(&format!("version={VERSION} sheet=a4")));

    // A fresh CLI process prints the identical bytes for the identical version.
    let again = f.root.join("version-again.pdf");
    let (code, exported) = f.run(
        "export-drawing",
        &argv(&[
            "--document-dir".to_owned(),
            document_dir.clone(),
            "--version-id".to_owned(),
            VERSION.to_owned(),
            "--format".to_owned(),
            "pdf".to_owned(),
            "--sheet".to_owned(),
            "a4".to_owned(),
            "--out".to_owned(),
            again.to_str().unwrap().to_owned(),
        ]),
    );
    assert_eq!(code, 0, "{exported:?}");
    assert_eq!(fs::read(&again).unwrap(), bytes);

    // Refusals change nothing: an unknown sheet or format creates no file.
    let refusals = [
        (
            "refused-sheet",
            vec![("--format", "pdf"), ("--sheet", "letter")],
        ),
        ("refused-format", vec![("--format", "jpeg")]),
    ];
    for (name, pairs) in refusals {
        let refused = f.root.join(name);
        let mut options = vec![
            "--document-dir".to_owned(),
            document_dir.clone(),
            "--version-id".to_owned(),
            VERSION.to_owned(),
            "--out".to_owned(),
            refused.to_str().unwrap().to_owned(),
        ];
        options.extend(
            pairs
                .into_iter()
                .flat_map(|(key, value)| [key.to_owned(), value.to_owned()]),
        );
        let (code, refusal) = f.run("export-drawing", &argv(&options));
        assert_eq!(code, 2, "{refusal:?}");
        assert_eq!(text(&refusal, "code"), "INVALID_ARGUMENT");
        assert!(!refused.exists());
    }
}

#[test]
fn publication_refuses_an_unresolved_draft_and_keeps_the_document_head() {
    let f = Fixture::new();
    f.model();
    let document_dir = f.root.join("drawing-doc").to_str().unwrap().to_owned();
    let head = create(&f, &document_dir);

    // The retained references never relink: after the deletion the draft resolves as
    // explicitly unresolved, which is a valid draft but never a publishable one.
    f.ok("delete-element", &["--id", MEMBER]);
    let mut refresh = vec![
        "--document-dir".to_owned(),
        document_dir.clone(),
        "--expected-head".to_owned(),
        head.clone(),
        "--op-id".to_owned(),
        OP3.to_owned(),
        "--revision-id".to_owned(),
        REV3.to_owned(),
        "--actor-id".to_owned(),
        ACTOR.to_owned(),
    ];
    refresh.extend(intent());
    let refreshed = f.ok("refresh-drawing", &argv(&refresh));
    assert_eq!(text(&refreshed, "state"), "draft");
    let refreshed_head = text(&refreshed, "revision_id");

    let (code, refusal) = publish(&f, &document_dir, &refreshed_head);
    assert_eq!(code, 2, "{refusal:?}");
    assert_eq!(text(&refusal, "code"), "INVALID_ARGUMENT");
    let inspected = f.ok("inspect-drawing", &["--document-dir", &document_dir]);
    assert_eq!(text(&inspected, "revision_id"), refreshed_head);
    assert_eq!(text(&inspected, "state"), "draft");
    let versions = f.ok(
        "inspect-drawing-versions",
        &["--document-dir", &document_dir],
    );
    assert_eq!(list(&versions, "versions").len(), 0);

    // Nothing was published, so nothing exports under that version id.
    let out = f.root.join("never.svg");
    let (code, refusal) = export(&f, &document_dir, &out);
    assert_eq!(code, 2, "{refusal:?}");
    assert_eq!(text(&refusal, "code"), "NOT_FOUND");
    assert!(!out.exists());

    // The refused publication did not consume the operation: restoring the geometry and
    // publishing from the same head succeeds.
    let profile = text(
        at(
            &f.ok("define-profile", &["--width", "100", "--height", "50"]),
            &["definition"],
        ),
        "digest",
    );
    f.ok(
        "create-member",
        &[
            "--profile-digest",
            &profile,
            "--start",
            "0,0,0",
            "--end",
            "3000,0,0",
            "--translation",
            "0,2000,0",
            "--new-id",
            MEMBER,
        ],
    );
    let mut retry = vec![
        "--document-dir".to_owned(),
        document_dir.clone(),
        "--expected-head".to_owned(),
        refreshed_head,
        "--op-id".to_owned(),
        OP4.to_owned(),
        "--revision-id".to_owned(),
        REV4.to_owned(),
        "--actor-id".to_owned(),
        ACTOR.to_owned(),
    ];
    retry.extend(intent());
    let redraft = f.ok("refresh-drawing", &argv(&retry));
    let (code, published) = publish(&f, &document_dir, &text(&redraft, "revision_id"));
    assert_eq!(code, 0, "{published:?}");
    assert_eq!(text(&published, "version_id"), VERSION);
}
