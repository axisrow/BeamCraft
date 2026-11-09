// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D08 print submission over one exact published version: a dry run reports the exact
//! `lp` invocation and byte count and submits nothing; a real submission goes through the
//! native print service with the page on stdin, and a missing or rejecting service is
//! `BackendUnavailable`, never a crash. The live model is never read.

use super::*;

const DRAWING: &str = "01900000-0000-7000-8000-000000000121";
const VIEW: &str = "01900000-0000-7000-8000-000000000122";
const DIMENSION: &str = "01900000-0000-7000-8000-000000000123";
const ACTOR: &str = "01900000-0000-7000-8000-000000000124";
const VERSION: &str = "01900000-0000-7000-8000-000000000125";
const UNUSED: &str = "01900000-0000-7000-8000-000000000126";
const OP1: &str = "01900000-0000-7000-8000-000000000127";
const REV1: &str = "01900000-0000-7000-8000-000000000128";
const OP2: &str = "01900000-0000-7000-8000-000000000129";
const REV2: &str = "01900000-0000-7000-8000-00000000012a";

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

/// One fixture with its model member and one published drawing version; the document
/// lives in the fixture root under `print-doc`.
fn published() -> (Fixture, String) {
    let f = Fixture::new();
    f.model();
    let document_dir = f.shared.root.join("print-doc").to_str().unwrap().to_owned();
    let mut create = vec![
        "--document-dir".to_owned(),
        document_dir.clone(),
        "--op-id".to_owned(),
        OP1.to_owned(),
        "--revision-id".to_owned(),
        REV1.to_owned(),
        "--actor-id".to_owned(),
        ACTOR.to_owned(),
    ];
    create.extend(intent());
    let head = text(&f.ok("create-drawing", &argv(&create)), "revision_id");
    let (code, reply) = f.run(
        "publish-drawing",
        &argv(&[
            "--document-dir".to_owned(),
            document_dir.clone(),
            "--expected-head".to_owned(),
            head,
            "--op-id".to_owned(),
            OP2.to_owned(),
            "--revision-id".to_owned(),
            REV2.to_owned(),
            "--actor-id".to_owned(),
            ACTOR.to_owned(),
            "--version-id".to_owned(),
            VERSION.to_owned(),
        ]),
    );
    assert_eq!(code, 0, "{reply:?}");
    assert_eq!(text(&reply, "state"), "published");
    (f, document_dir)
}

fn print_args(document_dir: &str, extra: &[String]) -> Vec<String> {
    let mut args = vec![
        "--document-dir".to_owned(),
        document_dir.to_owned(),
        "--version-id".to_owned(),
        VERSION.to_owned(),
    ];
    args.extend_from_slice(extra);
    args
}

#[test]
fn a_dry_run_reports_the_exact_invocation_and_replays_it() {
    let (f, document_dir) = published();
    let (code, reply) = f.run(
        "print-drawing",
        &argv(&print_args(
            &document_dir,
            &["--dry-run".to_owned(), "true".to_owned()],
        )),
    );
    assert_eq!(code, 0, "{reply:?}");
    assert_eq!(text(&reply, "outcome"), "drawing_print");
    assert_eq!(at(&reply, &["dry_run"]), &Json::Bool(true));
    assert_eq!(text(&reply, "version_id"), VERSION);
    assert_eq!(at(&reply, &["printer"]), &Json::Null);
    // The exact invocation for the default queue: `lp -J <drawing name (version)>`.
    assert_eq!(
        at(&reply, &["arguments"]),
        &Json::Array(vec![
            Json::text("lp"),
            Json::text("-J"),
            Json::text(format!("Member drawing ({VERSION})")),
        ])
    );
    // The dry run carries no request line: nothing was submitted.
    assert!(
        reply.get("request").is_err(),
        "a dry run must not carry a submission request"
    );
    // An exact replay reports the identical invocation and byte count.
    let (code, again) = f.run(
        "print-drawing",
        &argv(&print_args(
            &document_dir,
            &["--dry-run".to_owned(), "true".to_owned()],
        )),
    );
    assert_eq!(code, 0, "{again:?}");
    assert_eq!(again, reply);

    // A named queue lands in the invocation as plain arguments.
    let (code, named) = f.run(
        "print-drawing",
        &argv(&print_args(
            &document_dir,
            &[
                "--printer".to_owned(),
                "Prints-R-Us".to_owned(),
                "--dry-run".to_owned(),
                "true".to_owned(),
            ],
        )),
    );
    assert_eq!(code, 0, "{named:?}");
    assert_eq!(text(&named, "printer"), "Prints-R-Us");
    assert_eq!(
        at(&named, &["arguments"]),
        &Json::Array(vec![
            Json::text("lp"),
            Json::text("-d"),
            Json::text("Prints-R-Us"),
            Json::text("-J"),
            Json::text(format!("Member drawing ({VERSION})")),
        ])
    );
}

#[test]
fn refusals_stay_honest_and_submit_nothing() {
    let (f, document_dir) = published();
    // A queue name that reads as an option refuses before any submission, also on a dry run.
    let (code, refused) = f.run(
        "print-drawing",
        &argv(&print_args(
            &document_dir,
            &[
                "--printer".to_owned(),
                "-evil".to_owned(),
                "--dry-run".to_owned(),
                "true".to_owned(),
            ],
        )),
    );
    assert_eq!(code, 2, "{refused:?}");
    assert_eq!(text(&refused, "code"), "INVALID_ARGUMENT");
    // A broken --dry-run value refuses.
    let (code, refused) = f.run(
        "print-drawing",
        &argv(&print_args(
            &document_dir,
            &["--dry-run".to_owned(), "maybe".to_owned()],
        )),
    );
    assert_eq!(code, 2, "{refused:?}");
    assert_eq!(text(&refused, "code"), "INVALID_ARGUMENT");
    // An unknown version refuses and never reaches the print service.
    let (code, refused) = f.run(
        "print-drawing",
        &argv(&[
            "--document-dir".to_owned(),
            document_dir.clone(),
            "--version-id".to_owned(),
            UNUSED.to_owned(),
            "--dry-run".to_owned(),
            "true".to_owned(),
        ]),
    );
    assert_eq!(code, 2, "{refused:?}");
    assert_eq!(text(&refused, "code"), "NOT_FOUND");

    // A queue that does not exist refuses the submission with the backend code; the same
    // outcome whether the platform has no print service or the service rejects the job.
    let (code, refused) = f.run(
        "print-drawing",
        &argv(&print_args(
            &document_dir,
            &["--printer".to_owned(), "beamcraft-no-such-queue".to_owned()],
        )),
    );
    assert_eq!(code, 3, "{refused:?}");
    assert_eq!(text(&refused, "code"), "BACKEND_UNAVAILABLE");
}
