// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Real-binary checks against requirements supplied independently of the model.

#![allow(clippy::unwrap_used)]

use crate::support::{StoreFixture, at, list, text};
use beamcraft_json::{Json, Limits};
use beamcraft_model::digest::Sha256;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const FOOTING: &str = "019a7bf0-0000-7000-8000-000000000101";
const PEDESTAL: &str = "019a7bf0-0000-7000-8000-000000000102";
const BASE: &str = "019a7bf0-0000-7000-8000-000000000103";
const COLUMN: &str = "019a7bf0-0000-7000-8000-000000000104";
const RIB_A: &str = "019a7bf0-0000-7000-8000-000000000105";
const RIB_B: &str = "019a7bf0-0000-7000-8000-000000000106";
const ASSEMBLY: &str = "019a7bf0-0000-7000-8000-000000000107";
const COLUMN_TRIM: &str = "019a7bf0-0000-7000-8000-000000000108";
const RECORD_A: &str = "019a7bf0-0000-7000-8000-000000000201";
const RECORD_B: &str = "019a7bf0-0000-7000-8000-000000000202";

struct TaskFixture {
    shared: StoreFixture,
    specification: PathBuf,
    digest: String,
}

impl TaskFixture {
    fn check(&self) -> (i32, Json) {
        self.run("check-model", &[])
    }

    fn cli(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_beamcraft"));
        command
            .env("BEAMCRAFT_TASK_SPEC", &self.specification)
            .env("BEAMCRAFT_TASK_SPEC_SHA256", &self.digest)
            .env("BEAMCRAFT_CHECKS_SUMMARY", "off");
        command
    }

    fn store(&self) -> PathBuf {
        self.shared.root.join("store")
    }

    fn run(&self, word: &str, arguments: &[&str]) -> (i32, Json) {
        self.run_in(&self.store(), word, arguments)
    }

    fn run_in(&self, store: &Path, word: &str, arguments: &[&str]) -> (i32, Json) {
        let result = self
            .cli()
            .arg(word)
            .arg("--store")
            .arg(store)
            .args(arguments)
            .output()
            .unwrap();
        let json = beamcraft_json::decode(&result.stdout, &Limits::DEFAULT).unwrap();
        (result.status.code().unwrap(), json)
    }

    fn ok(&self, word: &str, arguments: &[&str]) -> Json {
        let (code, json) = self.run(word, arguments);
        assert_eq!(code, 0, "{word}: {json:?}");
        json
    }

    fn session(&self, lines: &str) -> Vec<Json> {
        let mut child = self
            .cli()
            .arg("session")
            .arg("--store")
            .arg(self.store())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(lines.as_bytes())
            .unwrap();
        let result = child.wait_with_output().unwrap();
        assert!(result.status.success(), "{result:?}");
        result
            .stdout
            .split(|b| *b == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| beamcraft_json::decode(line, &Limits::DEFAULT).unwrap())
            .collect()
    }
}

fn prepare() -> TaskFixture {
    let shared = StoreFixture::new("task-checks-base-joint");
    // Initialize the project identity before the owner binds the specification.
    let (code, material) = shared.run("define-material", &["--density", "7850"]);
    assert_eq!(code, 0, "{material:?}");
    let material = at(&material, &["definition", "digest"]).string().unwrap();
    let (code, project) = shared.run("open-revision", &[]);
    assert_eq!(code, 0, "{project:?}");
    let part_path =
        |id: &str| Json::text(format!("/World/Steel/Parts/PRT_{}", id.replace('-', "")));
    let requirements = Json::Array(vec![
        Json::object([
            ("type", Json::text("attribute")),
            ("path", part_path(BASE)),
            ("name", Json::text("steel:plate:thicknessMm")),
            ("value", Json::uint(20)),
        ]),
        Json::object([
            ("type", Json::text("attribute")),
            ("path", part_path(COLUMN)),
            ("name", Json::text("steel:cut:operationId")),
            ("value", Json::Array(vec![Json::text(COLUMN_TRIM)])),
        ]),
    ]);
    let specification = Json::object([
        ("version", Json::uint(1)),
        ("project_id", Json::text(text(&project, "project_id"))),
        ("requirements", requirements),
        (
            "template",
            Json::object([
                ("type", Json::text("column_base_v1")),
                ("footing", Json::text(FOOTING)),
                ("pedestal", Json::text(PEDESTAL)),
                ("base_plate", Json::text(BASE)),
                ("column", Json::text(COLUMN)),
                ("rib_a", Json::text(RIB_A)),
                ("rib_b", Json::text(RIB_B)),
                ("assembly", Json::text(ASSEMBLY)),
                ("required_mm", Json::uint(10)),
                ("tolerance_mm", Json::uint(1)),
            ]),
        ),
    ]);
    let fixture = bind(shared, specification);
    let ok = |command, args: &[&str]| fixture.ok(command, args);
    let concrete = ok("define-material", &["--density", "2400"]);
    let concrete = at(&concrete, &["definition", "digest"]).string().unwrap();
    let profile = ok(
        "define-profile",
        &[
            "--family", "i_shape", "--h", "420", "--b", "200", "--tw", "10", "--tf", "16", "--r",
            "0",
        ],
    );
    let profile = at(&profile, &["definition", "digest"]).string().unwrap();

    for (id, width, start, end, name) in [
        (FOOTING, "1200", "0,0,0", "0,0,400", "Footing"),
        (PEDESTAL, "700", "0,0,400", "0,0,600", "Pedestal"),
    ] {
        let section = ok("define-profile", &["--width", width, "--height", width]);
        let section = at(&section, &["definition", "digest"]).string().unwrap();
        ok(
            "create-member",
            &[
                "--new-id",
                id,
                "--name",
                name,
                "--kind",
                "column",
                "--start",
                start,
                "--end",
                end,
                "--profile-digest",
                section,
                "--material-digest",
                concrete,
            ],
        );
    }
    ok(
        "create-plate",
        &[
            "--new-id",
            BASE,
            "--name",
            "Base plate",
            "--width",
            "460",
            "--length",
            "460",
            "--thickness",
            "20",
            "--translation",
            "-230,-230,600",
            "--material-digest",
            material,
        ],
    );
    let penetrating = ok(
        "create-member",
        &[
            "--new-id",
            COLUMN,
            "--kind",
            "column",
            "--start",
            "0,0,600",
            "--end",
            "0,0,2400",
            "--roll",
            "1.5707963267948966",
            "--profile-digest",
            profile,
            "--material-digest",
            material,
        ],
    );
    assert_eq!(
        count(at(&penetrating, &["checks"]), "PART_CLASH", "fail"),
        1
    );
    let trimmed = ok(
        "cut-by-plane",
        &[
            "--id",
            COLUMN,
            "--operation-id",
            COLUMN_TRIM,
            "--point",
            "0,0,620",
            "--normal",
            "0,0,1",
            "--keep",
            "positive",
        ],
    );
    assert_eq!(count(at(&trimmed, &["checks"]), "PART_CLASH", "fail"), 0);
    for (id, origin, orientation) in [
        (
            RIB_A,
            "0,-6,620",
            "0.7071067811865476,0,0,0.7071067811865476",
        ),
        (
            RIB_B,
            "0,6,620",
            "0,0.7071067811865476,0.7071067811865476,0",
        ),
    ] {
        ok(
            "create-plate",
            &[
                "--new-id",
                id,
                "--contour",
                "0,0;194,0;194,194",
                "--thickness",
                "10",
                "--translation",
                origin,
                "--orientation-xyzw",
                orientation,
                "--material-digest",
                material,
            ],
        );
    }
    ok(
        "create-assembly",
        &["--new-id", ASSEMBLY, "--name", "Column base"],
    );
    for id in [FOOTING, PEDESTAL, BASE, COLUMN, RIB_A, RIB_B] {
        ok(
            "attach-to-assembly",
            &["--id", id, "--assembly-id", ASSEMBLY],
        );
    }
    fixture
}

fn bind(shared: StoreFixture, specification: Json) -> TaskFixture {
    let bytes = specification.encode(&Limits::DEFAULT).unwrap();
    let digest = Sha256::digest(&bytes).unwrap().to_string();
    let specification = shared.root.join("approved-task.json");
    std::fs::write(&specification, bytes).unwrap();
    TaskFixture {
        shared,
        specification,
        digest,
    }
}

fn count(reply: &Json, rule: &str, status: &str) -> u64 {
    at(reply, &["counts", rule, status]).unsigned().unwrap()
}

fn record(fixture: &TaskFixture, id: &str, target: &str, size: &str) -> Json {
    fixture.ok(
        "record-create",
        &[
            "--new-id",
            id,
            "--kind",
            "corner_cut",
            "--name",
            "Corner requirement",
            "--target",
            target,
            "--neighbours",
            &format!("{BASE},{COLUMN}"),
            "--required-mm",
            size,
            "--tolerance-mm",
            "1",
            "--purpose",
            "clearance",
        ],
    )
}

fn missing_records(report: &Json) -> usize {
    list(report.get("task").unwrap(), "findings")
        .iter()
        .filter(|f| text(f, "code") == "TASK_CORNER_RECORD_MISSING")
        .count()
}

fn export(fixture: &TaskFixture, name: &str, out: &Path) -> Json {
    fixture.ok(name, &["--out", out.to_str().unwrap()])
}

#[test]
fn an_approved_task_reports_omissions_and_keeps_writes_and_exports_available() {
    let fixture = prepare();
    let (code, missing) = fixture.check();
    assert_eq!(code, 2, "{missing:?}");
    assert_eq!(missing_records(&missing), 2);
    assert_eq!(count(&missing, "CORNER_NOT_CUT", "fail"), 2);
    assert_eq!(text(at(&missing, &["task"]), "sha256"), fixture.digest);
    let bad_revision = text(&missing, "revision_id");

    let remove = fixture.ok("delete-element", &["--id", RIB_B]);
    let checks = at(&remove, &["checks"]);
    assert_eq!(text(checks, "status"), "red");
    assert!(
        list(at(checks, &["task"]), "findings")
            .iter()
            .any(|row| text(row, "code") == "TASK_MODEL_MISMATCH" && text(row, "target") == RIB_B)
    );
    fixture.ok("undo-revision", &[]);

    let destination = fixture.shared.root.join("unchecked.ifc");
    let report_path = fixture.shared.root.join("export-report.json");
    let exported = fixture.ok(
        "export-ifc",
        &[
            "--out",
            destination.to_str().unwrap(),
            "--report",
            report_path.to_str().unwrap(),
        ],
    );
    assert!(destination.is_file());
    let checks = at(&exported, &["checks"]);
    assert_eq!(text(checks, "status"), "red");
    assert_eq!(
        text(checks, "revision_id"),
        text(&fixture.check().1, "revision_id")
    );
    assert_eq!(missing_records(checks), 2);
    let export_report =
        beamcraft_json::decode(&std::fs::read(report_path).unwrap(), &Limits::DEFAULT).unwrap();
    assert_eq!(checks, at(&export_report, &["checks"]));

    let stream = fixture
        .cli()
        .arg("export-view")
        .arg("--store")
        .arg(fixture.store())
        .args(["--with-definitions", "true", "--out", "-"])
        .output()
        .unwrap();
    assert!(stream.status.success());
    assert!(stream.stdout.starts_with(b"BCVXFRM1"));
    let stream_report = beamcraft_json::decode(&stream.stderr, &Limits::DEFAULT).unwrap();
    assert_eq!(text(&stream_report, "outcome"), "task_checks");
    assert_eq!(at(&stream_report, &["checks"]), checks);

    let only_cut = fixture.ok(
        "update-element",
        &["--id", RIB_B, "--corners", "1=chamfer:10"],
    );
    let checks = at(&only_cut, &["checks"]);
    assert_eq!(text(checks, "status"), "red");
    assert_eq!(missing_records(checks), 2);
    assert_eq!(count(checks, "CORNER_NOT_CUT", "fail"), 1);
    fixture.ok("undo-revision", &[]);

    let changed = fixture.ok(
        "update-element",
        &["--id", RIB_A, "--corners", "1=chamfer:20"],
    );
    assert_eq!(
        count(at(&changed, &["checks"]), "CORNER_OVERCUT", "fail"),
        1
    );
    let wrong = record(&fixture, RECORD_A, RIB_A, "20");
    let checks = at(&wrong, &["checks"]);
    assert_eq!(text(checks, "status"), "red");
    assert!(
        list(at(checks, &["task"]), "findings")
            .iter()
            .any(|row| text(row, "code") == "TASK_CORNER_RECORD_MISMATCH"
                && text(row, "target") == RIB_A)
    );
    assert!(
        list(checks, "findings")
            .iter()
            .any(|row| text(row, "rule") == "CORNER_OVERCUT"
                && at(row, &["required_mm"]).float() == Ok(10.0))
    );

    fixture.ok("undo-revision", &[]);
    record(&fixture, RECORD_A, RIB_A, "10");
    let only_record = record(&fixture, RECORD_B, RIB_B, "10");
    let checks = at(&only_record, &["checks"]);
    assert_eq!(text(checks, "status"), "red");
    assert_eq!(missing_records(checks), 0);
    assert_eq!(count(checks, "CORNER_NOT_CUT", "fail"), 1);

    let lines = format!(
        "batch-open\ncheck-model --store {}\nupdate-element --store {} --id {RIB_A} --corners 1=chamfer:10\n\
        update-element --store {} --id {RIB_B} --corners 1=chamfer:10\nbatch-commit\n",
        fixture.store().display(),
        fixture.store().display(),
        fixture.store().display()
    );
    let replies = fixture.session(&lines);
    assert_eq!(replies.len(), 6);
    assert_eq!(text(replies.first().unwrap(), "outcome"), "session_ready");
    let read = replies.get(2).unwrap();
    assert_eq!(text(read, "status"), "red");
    assert_eq!(
        text(at(read, &["draft_checks"]), "reason"),
        "draft_not_evaluated"
    );
    assert_eq!(
        text(at(replies.get(3).unwrap(), &["checks"]), "reason"),
        "draft_not_evaluated"
    );
    let checks = at(replies.last().unwrap(), &["checks"]);
    assert_eq!(text(checks, "status"), "green", "{checks:?}");
    assert_eq!(fixture.check().0, 0);
    let (_, partial) = fixture.run("check-model", &["--element", RIB_A]);
    assert_eq!(text(&partial, "status"), "unknown");
    assert_eq!(
        text(at(&partial, &["task"]), "reason"),
        "partial_geometry_scope"
    );

    let (code, pinned) = fixture.run("check-model", &["--revision", &bad_revision]);
    assert_eq!(code, 2);
    assert_eq!(text(&pinned, "revision_id"), bad_revision);
    assert_eq!(missing_records(&pinned), 2);

    let ambiguous = fixture.ok(
        "update-element",
        &["--id", RIB_A, "--translation", "0,-5,620"],
    );
    let checks = at(&ambiguous, &["checks"]);
    assert_eq!(text(checks, "status"), "unknown", "{checks:?}");
    assert_eq!(count(checks, "CORNER_NOT_CUT", "unknown"), 1);

    let workspace = fixture.shared.root.join("draft");
    let created = fixture.ok(
        "create-workspace",
        &["--workspace-dir", workspace.to_str().unwrap()],
    );
    assert_eq!(text(at(&created, &["checks"]), "status"), "unknown");
    let (code, changed) = fixture.run_in(
        &workspace,
        "set-plate-corner",
        &["--id", RIB_B, "--vertex", "1", "--corner", "sharp"],
    );
    assert_eq!(code, 0, "{changed:?}");
    assert_eq!(text(at(&changed, &["checks"]), "status"), "red");
    let published = fixture.ok(
        "publish-workspace",
        &["--workspace-dir", workspace.to_str().unwrap()],
    );
    let checks = at(&published, &["checks"]);
    assert_eq!(text(checks, "status"), "red");
    assert_eq!(count(checks, "CORNER_NOT_CUT", "fail"), 1);
    assert_eq!(
        text(checks, "revision_id"),
        text(at(&published, &["revision"]), "revision_id")
    );
}

#[test]
fn an_unreadable_or_changed_approved_specification_cannot_pass() {
    let fixture = prepare();
    std::fs::write(&fixture.specification, b"{}").unwrap();
    let (code, report) = fixture.check();
    assert_eq!(code, 0);
    assert_eq!(text(&report, "status"), "unknown");
    assert_eq!(text(at(&report, &["task"]), "code"), "INTEGRITY_ERROR");

    let command = fixture.ok("update-element", &["--id", RIB_A, "--name", "Rib A"]);
    assert_eq!(text(at(&command, &["checks"]), "status"), "unknown");
    let destination = fixture.shared.root.join("unknown.ifc");
    let exported = export(&fixture, "export-ifc", &destination);
    assert!(destination.is_file());
    assert_eq!(text(at(&exported, &["checks"]), "status"), "unknown");
}

#[test]
fn a_script_delivers_additional_findings_from_the_same_revision() {
    let shared = StoreFixture::new("task-checks-pagination");
    let (code, seeded) = shared.run(
        "create-plate",
        &[
            "--new-id",
            BASE,
            "--width",
            "10",
            "--length",
            "10",
            "--thickness",
            "10",
        ],
    );
    assert_eq!(code, 0, "{seeded:?}");
    let (_, project) = shared.run("open-revision", &[]);
    let fixture = bind(
        shared,
        Json::object([
            ("version", Json::uint(1)),
            ("project_id", Json::text(text(&project, "project_id"))),
            (
                "requirements",
                Json::Array(vec![Json::object([
                    ("type", Json::text("element")),
                    ("id", Json::text(BASE)),
                    ("kind", Json::text("plate")),
                ])]),
            ),
        ]),
    );
    let store = fixture.store();
    let script = Json::Array(vec![Json::Array(
        [
            "array-linear",
            "--store",
            store.to_str().unwrap(),
            "--id",
            BASE,
            "--count",
            "35",
            "--direction",
            "1,0,0",
            "--spacing",
            "30",
        ]
        .into_iter()
        .map(Json::text)
        .collect(),
    )]);
    let path = fixture.shared.root.join("array.json");
    std::fs::write(&path, script.encode(&Limits::DEFAULT).unwrap()).unwrap();
    let output = fixture
        .cli()
        .arg("run-script")
        .arg("--store")
        .arg(&store)
        .arg("--file")
        .arg(path)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let result = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
    let row = list(&result, "replies").first().unwrap();
    assert_eq!(row.get("exit").unwrap().unsigned(), Ok(0));
    let checks = at(row, &["reply", "checks"]);
    let revision = text(checks, "revision_id");
    assert_eq!(text(checks, "status"), "red");
    assert_eq!(checks.get("next_offset").unwrap().unsigned(), Ok(32));
    assert_eq!(list(checks, "findings").len(), 32);

    let supplemental = beamcraft_json::decode(&output.stderr, &Limits::DEFAULT).unwrap();
    assert_eq!(text(&supplemental, "outcome"), "task_checks");
    let page = at(&supplemental, &["checks"]);
    assert_eq!(text(page, "revision_id"), revision);
    assert_eq!(text(page, "task_sha256"), fixture.digest);
    assert_eq!(text(page, "status"), "red");
    assert_eq!(page.get("offset").unwrap().unsigned(), Ok(32));
    assert_eq!(page.get("next_offset").unwrap(), &Json::Null);
    let (code, report) = fixture.run("check-model", &["--revision", &revision, "--offset", "32"]);
    assert_eq!(code, 2);
    assert_eq!(list(page, "findings"), list(&report, "findings"));
}

#[test]
fn unsupported_requirements_and_missing_approval_stay_visible() {
    let shared = StoreFixture::new("task-checks-unsupported");
    let (code, material) = shared.run("define-material", &["--density", "7850"]);
    assert_eq!(code, 0);
    let (_, project) = shared.run("open-revision", &[]);
    let (_, standalone) = shared.run("check-model", &[]);
    assert_eq!(text(&standalone, "status"), "green");
    assert_eq!(
        text(at(&standalone, &["task"]), "reason"),
        "task_approval_missing"
    );
    let fixture = bind(
        shared,
        Json::object([
            ("version", Json::uint(1)),
            ("project_id", Json::text(text(&project, "project_id"))),
            (
                "requirements",
                Json::Array(vec![Json::object([
                    ("type", Json::text("required_check")),
                    ("target", Json::text(BASE)),
                    ("check", Json::text("computed structural capacity")),
                ])]),
            ),
        ]),
    );
    let written = fixture.ok(
        "create-plate",
        &[
            "--new-id",
            BASE,
            "--width",
            "20",
            "--length",
            "20",
            "--thickness",
            "5",
            "--material-digest",
            at(&material, &["definition", "digest"]).string().unwrap(),
        ],
    );
    let checks = at(&written, &["checks"]);
    assert_eq!(text(checks, "status"), "unknown");
    let finding = list(at(checks, &["task"]), "findings").first().unwrap();
    assert_eq!(text(finding, "code"), "TASK_CHECK_UNSUPPORTED");
    assert_eq!(text(finding, "target"), BASE);
}
