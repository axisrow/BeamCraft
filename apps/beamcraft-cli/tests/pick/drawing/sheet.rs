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
const SECTION: &str = "01900000-0000-7000-8000-000000000348";
const DETAIL: &str = "01900000-0000-7000-8000-000000000349";
const LAYER: &str = "01900000-0000-7000-8000-000000000350";
const NOTE: &str = "01900000-0000-7000-8000-000000000351";
const LEADER: &str = "01900000-0000-7000-8000-000000000352";
const RING: &str = "01900000-0000-7000-8000-000000000353";
const BOX: &str = "01900000-0000-7000-8000-000000000354";
const OP_DRAFT: &str = "01900000-0000-7000-8000-000000000355";
const REV_DRAFT: &str = "01900000-0000-7000-8000-000000000356";
const OP_PUBLISH: &str = "01900000-0000-7000-8000-000000000357";
const REV_PUBLISH: &str = "01900000-0000-7000-8000-000000000358";
const VERSION4: &str = "01900000-0000-7000-8000-000000000359";

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

#[test]
fn the_whole_sheet_reaches_the_svg_and_the_pdf_alike() {
    // Findings 1-3 of docs/visual-bug-evidence-2026-11-04: the sheet SVG rendered the
    // Y-up paper unflipped into Y-down SVG space (mirrored glyphs, title block on top),
    // the PDF of a sheet document dropped the frame, the title block and every view but
    // the primary, and the drawn objects on a visible layer never reached either surface.
    let f = Fixture::new();
    f.model();
    let document_dir = f
        .shared
        .root
        .join("whole-sheet-doc")
        .to_str()
        .unwrap()
        .to_owned();
    let mut created = create_args(&document_dir);
    created.extend(
        [
            "--sheet-size",
            "a3",
            "--sheet-project",
            "NORTH WAREHOUSE EXTENSION PHASE TWO 2026",
            "--sheet-author",
            "J. Smith",
        ]
        .iter()
        .map(|value| (*value).to_owned()),
    );
    let created = f.ok(
        &created[0],
        &created[1..].iter().map(String::as_str).collect::<Vec<_>>(),
    );
    let head = text(&created, "revision_id");

    // A placed view, a section and a detail, then four drawn objects on a visible layer,
    // exactly the objects the evidence reproducer drafts.
    let mut placements = Vec::new();
    for (flag, value) in [
        // The fixture member sits at model y 2000, so the placements keep every view
        // inside the A3 frame: the plan view at 1:20 needs 100 mm of paper y for the
        // model depth, the section at 1:20 needs 20 and the detail magnifies 2x.
        ("--placed-view", format!("{TOP}:top:20:40,230:PLAN")),
        (
            "--placed-section",
            format!("{SECTION}:right:1500:positive:100:240,250:SECTION A-A"),
        ),
        (
            "--placed-detail",
            format!("{DETAIL}:{VIEW}:45,40:5:50:220,330:DETAIL B"),
        ),
    ] {
        placements.push(flag.to_owned());
        placements.push(value);
    }
    let (code, placed) = edit(
        &f,
        "edit-views",
        &document_dir,
        &head,
        OP_VIEWS,
        REV_VIEWS,
        &placements.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    assert_eq!(code, 0, "{placed:?}");
    let placed_head = text(&placed, "revision_id");

    let mut drafts = vec!["--layer".to_owned(), format!("{LAYER}:Notes:on")];
    for entity in [
        format!("{NOTE}:text:30,80,5,0:{LAYER}:continuous:MARK B-1 (2 PCS)"),
        format!("{LEADER}:line:60,95,90,150:{LAYER}:continuous"),
        format!("{RING}:circle:200,110,10:{LAYER}:dashed"),
        format!("{BOX}:rectangle:25,70,120,95:{LAYER}:continuous"),
    ] {
        drafts.push("--drafting".to_owned());
        drafts.push(entity);
    }
    let (code, drafted) = edit(
        &f,
        "edit-drafting",
        &document_dir,
        &placed_head,
        OP_DRAFT,
        REV_DRAFT,
        &drafts.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    assert_eq!(code, 0, "{drafted:?}");
    let (_, published) = edit(
        &f,
        "publish-drawing",
        &document_dir,
        &text(&drafted, "revision_id"),
        OP_PUBLISH,
        REV_PUBLISH,
        &["--version-id", VERSION4],
    );
    assert_eq!(text(&published, "version_id"), VERSION4);

    // The SVG: the whole sheet, upright, with every drawn object and its line type.
    let svg_out = f.shared.root.join("whole-sheet.svg");
    f.ok(
        "export-drawing",
        &[
            "--document-dir",
            &document_dir,
            "--version-id",
            VERSION4,
            "--out",
            svg_out.to_str().unwrap(),
        ],
    );
    let svg = String::from_utf8(fs::read(&svg_out).unwrap()).unwrap();
    assert!(svg.contains("class=\"frame\""));
    assert!(svg.contains("class=\"titleblock\""));
    for id in [TOP, SECTION, DETAIL] {
        assert!(svg.contains(&format!("data-view=\"{id}\"")), "{id}");
    }
    for id in [NOTE, LEADER, RING, BOX] {
        assert!(svg.contains(&format!("data-drafting=\"{id}\"")), "{id}");
    }
    assert!(
        svg.contains(&format!(
            "data-drafting=\"{RING}\" stroke-dasharray=\"3 3\""
        )),
        "the dashed circle keeps its line type"
    );
    // Upright: the frame's bottom edge lands at svg y 410 (420 mm minus the 10 mm
    // inset), the top edge at y 10; the unflipped writer drew them the other way round.
    assert!(
        svg.contains("M 10.0000 410.0000 L 287.0000 410.0000"),
        "{svg}"
    );
    // The frame's sides run into each other, so the second edge continues the first
    // polyline instead of opening its own subpath (#490 finding 1).
    assert!(svg.contains("L 287.0000 10.0000 L 10.0000 10.0000"));
    // The drafted note's first glyph stroke starts at its anchor (30, 80); the 5 mm
    // glyphs rise to paper y 85, flipped to svg y 335.
    assert!(
        svg.contains("M 30.0000 340.0000 L 30.0000 335.0000"),
        "note anchor"
    );

    // The PDF renders the same sheet: A3 paper, the title block, the placed views and
    // the drawn objects. The primary-only print never drew any of these.
    let pdf_out = f.shared.root.join("whole-sheet.pdf");
    f.ok(
        "export-drawing",
        &[
            "--document-dir",
            &document_dir,
            "--version-id",
            VERSION4,
            "--format",
            "pdf",
            "--out",
            pdf_out.to_str().unwrap(),
        ],
    );
    let pdf = String::from_utf8(fs::read(&pdf_out).unwrap()).unwrap();
    assert!(pdf.contains("/MediaBox [0 0 841.8898 1190.5512]"));
    assert!(pdf.contains("sheet=a3"));
    // The title block's left edge runs from paper (127, 10) mm -> (360.0000, 28.3465) pt.
    assert!(pdf.contains("360.0000 28.3465 m"), "title block left edge");
    // The leader line reaches the page: (60, 95) mm -> pt.
    assert!(pdf.contains("170.0787 269.2913 m"), "leader endpoint");
    // The drafted note's first glyph stroke starts at its anchor (30, 80) mm -> pt.
    assert!(pdf.contains("85.0394 226.7717 m"), "note anchor");
    // The dashed circle paints with its authored line type: a 3 mm dash in points.
    assert!(
        pdf.contains("[8.5039 8.5039] 0 d"),
        "dashed circle dash pattern"
    );
}

/// Issue 546: a placed detail whose annotation lands inside the A4 title block refuses both
/// surfaces with `LIMIT_EXCEEDED` and writes nothing; placed clear of the block, the same
/// detail exports.
#[test]
fn a_detail_over_the_title_block_refuses_both_surfaces() {
    let f = Fixture::new();
    f.model();
    let dir = f
        .shared
        .root
        .join("sheet-block-doc")
        .to_str()
        .unwrap()
        .to_owned();
    let mut args = create_args(&dir);
    let origin_at = args.iter().position(|value| value == "--origin").unwrap() + 1;
    args[origin_at] = "50,230".to_owned();
    args.extend(
        [
            "--scale",
            "40",
            "--plane",
            "front",
            "--sheet-size",
            "a4",
            "--sheet-project",
            "Validate 271",
            "--sheet-author",
            "Worker 271",
        ]
        .iter()
        .map(|value| (*value).to_owned()),
    );
    let created = f.ok(
        &args[0],
        &args[1..].iter().map(String::as_str).collect::<Vec<_>>(),
    );
    let low = format!("{DETAIL}:{VIEW}:120,230:5:5:110,60:DET-1");
    let (code, placed) = edit(
        &f,
        "edit-views",
        &dir,
        &text(&created, "revision_id"),
        "01900000-0000-7000-8000-000000000361",
        "01900000-0000-7000-8000-000000000362",
        &["--placed-detail", &low],
    );
    assert_eq!(code, 0, "{placed:?}");
    let (code, published) = edit(
        &f,
        "publish-drawing",
        &dir,
        &text(&placed, "revision_id"),
        "01900000-0000-7000-8000-000000000363",
        "01900000-0000-7000-8000-000000000364",
        &["--version-id", "01900000-0000-7000-8000-000000000365"],
    );
    assert_eq!(code, 0, "{published:?}");
    for format in ["svg", "pdf"] {
        let out = f.shared.root.join(format!("block.{format}"));
        let (code, refusal) = f.run(
            "export-drawing",
            &[
                "--document-dir",
                &dir,
                "--version-id",
                "01900000-0000-7000-8000-000000000365",
                "--format",
                format,
                "--out",
                out.to_str().unwrap(),
            ],
        );
        assert_eq!(code, 2, "{format}: {refusal:?}");
        assert_eq!(text(&refusal, "code"), "LIMIT_EXCEEDED");
        assert!(!out.exists(), "a refused export writes no file");
    }
    let clear = format!("{DETAIL}:{VIEW}:120,230:5:5:110,150:DET-1");
    let (code, moved) = edit(
        &f,
        "edit-views",
        &dir,
        &text(&published, "revision_id"),
        "01900000-0000-7000-8000-000000000366",
        "01900000-0000-7000-8000-000000000367",
        &["--placed-detail", &clear],
    );
    assert_eq!(code, 0, "{moved:?}");
    let (code, republished) = edit(
        &f,
        "publish-drawing",
        &dir,
        &text(&moved, "revision_id"),
        "01900000-0000-7000-8000-000000000368",
        "01900000-0000-7000-8000-000000000369",
        &["--version-id", "01900000-0000-7000-8000-000000000370"],
    );
    assert_eq!(code, 0, "{republished:?}");
    let out = f.shared.root.join("clear.pdf");
    let (code, saved) = f.run(
        "export-drawing",
        &[
            "--document-dir",
            &dir,
            "--version-id",
            "01900000-0000-7000-8000-000000000370",
            "--format",
            "pdf",
            "--out",
            out.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "{saved:?}");
    assert!(out.exists());
}
