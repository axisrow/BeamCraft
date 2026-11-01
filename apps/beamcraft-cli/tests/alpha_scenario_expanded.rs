// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic frame and closed-form references, MPL-2.0.

//! Expanded Q01 (fork issue axisrow/BeamCraft#318, upstream issue #334): one real-binary story that extends the
//! delivered alpha scenario into the model tools, member holes, views/filters, user attributes
//! and schedules, grids/levels, invalid-input and cancellation regressions, and exact stored
//! outputs. Every command starts a fresh process; no mock storage, renderer or geometry is
//! used. The original scenario stays in `alpha_scenario.rs`;
//! `docs/design/q01-alpha-matrix.md` records what this covers and what stays open with reasons.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support::{at, text};

use beamcraft_json::{Json, Limits};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const USER: &str = "019a7bf0-0000-7000-8000-00000000aa01";
const PARTS: [&str; 5] = [
    "019a7bf0-0000-7000-8000-00000000aa10",
    "019a7bf0-0000-7000-8000-00000000aa11",
    "019a7bf0-0000-7000-8000-00000000aa12",
    "019a7bf0-0000-7000-8000-00000000aa13",
    "019a7bf0-0000-7000-8000-00000000aa14",
];
// Identities added by the expanded steps: a split half and the operation ids.
const SPLIT_HALF: &str = "019a7bf0-0000-7000-8000-00000000aa15";
const SPLIT_OP: &str = "019a7bf0-0000-7000-8000-00000000aa20";
const GRID: &str = "019a7bf0-0000-7000-8000-00000000aa21";
const LEVEL: &str = "019a7bf0-0000-7000-8000-00000000aa22";
const MEMBER_HOLE_OP: &str = "019a7bf0-0000-7000-8000-00000000aa24";

/// Closed-form volumes of the five parts before any tool touches them (as in the original
/// scenario): two columns, the cut beam, and two plates with one rectangular hole each.
const STARTING_VOLUMES: [f64; 5] = [5_000_000.0, 5_000_000.0, 4_750_000.0, 396_000.0, 396_000.0];

struct Story(PathBuf);

impl Story {
    fn new() -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("q01-expanded-{}", crate::support::boot_nonce()));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("published")).unwrap();
        fs::create_dir(root.join("empty-store")).unwrap();
        Self(root)
    }

    fn path(&self, name: &str) -> String {
        self.0.join(name).to_str().unwrap().to_owned()
    }

    /// Runs one fresh process and returns its exit code with the decoded stdout JSON.
    fn execute(&self, args: &[&str]) -> (i32, Json) {
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(args)
            .output()
            .unwrap();
        (
            output.status.code().unwrap(),
            beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap(),
        )
    }

    fn run(&self, store: &str, command: &str, args: &[&str], expected: i32) -> Json {
        let store = self.path(store);
        let mut all = vec![command, "--store", store.as_str()];
        all.extend_from_slice(args);
        let (code, json) = self.execute(&all);
        assert_eq!(code, expected, "{command} {args:?}: {json:?}");
        json
    }

    fn ok(&self, store: &str, command: &str, args: &[&str]) -> Json {
        self.run(store, command, args, 0)
    }

    fn head(&self, store: &str) -> String {
        text(&self.ok(store, "open-revision", &[]), "revision_id")
    }
}

impl Drop for Story {
    fn drop(&mut self) {
        if std::env::var_os("BEAMCRAFT_Q01_KEEP").is_some() {
            eprintln!("Q01 expanded artifacts: {}", self.0.display());
        } else {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1e-8 * expected.abs().max(1.0),
        "actual {actual}, expected {expected}"
    );
}

/// (element_id, volume mm3, mesh_digest) of every part, sorted by id.
fn parts(inspection: &Json) -> Vec<(String, f64, String)> {
    let mut all = Vec::new();
    for key in ["plates", "members"] {
        for part in inspection.get(key).unwrap().array().unwrap() {
            all.push((
                text(part, "element_id"),
                at(part, &["quantities", "volume", "mm3"]).float().unwrap(),
                text(part, "mesh_digest"),
            ));
        }
    }
    all.sort_by(|a, b| a.0.cmp(&b.0));
    all
}

fn volume_of(parts: &[(String, f64, String)], id: &str) -> f64 {
    parts
        .iter()
        .find(|(element, _, _)| element == id)
        .expect("part is present in the inspection")
        .1
}

#[test]
fn expanded_alpha_runs_tools_views_regressions_and_exact_outputs() {
    let s = Story::new();
    s.ok(
        "published",
        "define-material",
        &["--grade", "Q01-steel", "--density", "7850"],
    );
    s.ok(
        "published",
        "define-profile",
        &[
            "--designation",
            "Q01-RECT",
            "--width",
            "100",
            "--height",
            "50",
        ],
    );
    s.ok(
        "published",
        "create-workspace",
        &["--workspace-dir", &s.path("draft"), "--user-id", USER],
    );

    // The same synthetic frame as the delivered scenario: two columns and a beam, the beam
    // trimmed by an owned plane cut, two base plates with holes.
    for (id, kind, start, end) in [
        (PARTS[0], "column", "0,0,0", "0,0,1000"),
        (PARTS[1], "column", "1000,0,0", "1000,0,1000"),
        (PARTS[2], "beam", "0,0,1000", "1000,0,1000"),
    ] {
        s.ok(
            "draft",
            "create-member",
            &[
                "--new-id",
                id,
                "--kind",
                kind,
                "--start",
                start,
                "--end",
                end,
                "--profile",
                "Q01-RECT",
                "--material",
                "Q01-steel",
                "--name",
                kind,
            ],
        );
    }
    s.ok(
        "draft",
        "cut-by-plane",
        &[
            "--id",
            PARTS[2],
            "--operation-id",
            "019a7bf0-0000-7000-8000-00000000aa17",
            "--point",
            "950,0,1000",
            "--normal",
            "1,0,0",
            "--keep",
            "negative",
        ],
    );
    for (id, translation) in [(PARTS[3], "-100,-100,0"), (PARTS[4], "900,-100,0")] {
        s.ok(
            "draft",
            "create-plate",
            &[
                "--new-id",
                id,
                "--width",
                "200",
                "--length",
                "200",
                "--thickness",
                "10",
                "--hole-rect",
                "150,150,20,20",
                "--translation",
                translation,
                "--material",
                "Q01-steel",
                "--name",
                "Base plate",
            ],
        );
    }
    let before = parts(&s.ok("draft", "inspect-elements", &[]));
    assert_eq!(before.len(), 5);
    for (id, volume) in PARTS.into_iter().zip(STARTING_VOLUMES) {
        close(volume_of(&before, id), volume);
    }

    // Model tools (split, copy, chamfer) change exactly the parts they name. The split cuts
    // the second column at mid height, so the halves sum to the closed-form column volume.
    s.ok(
        "draft",
        "split-part",
        &[
            "--id",
            PARTS[1],
            "--new-id",
            SPLIT_HALF,
            "--new-name",
            "Split column",
            "--operation-id",
            SPLIT_OP,
            "--point",
            "1000,0,500",
            "--normal",
            "0,0,1",
            "--keep",
            "positive",
        ],
    );
    let after_split = parts(&s.ok("draft", "inspect-elements", &[]));
    assert_eq!(after_split.len(), 6);
    close(
        volume_of(&after_split, PARTS[1]) + volume_of(&after_split, SPLIT_HALF),
        STARTING_VOLUMES[1],
    );
    let copied = s.ok(
        "draft",
        "copy-element",
        &[
            "--id",
            PARTS[4],
            "--offset",
            "0,300,0",
            "--name",
            "Copied plate",
        ],
    );
    let copy_id = text(at(&copied, &["element"]), "element_id");
    let after_copy = parts(&s.ok("draft", "inspect-elements", &[]));
    assert_eq!(after_copy.len(), 7);
    close(
        volume_of(&after_copy, copy_id.as_str()),
        volume_of(&after_copy, PARTS[4]),
    );
    s.ok(
        "draft",
        "chamfer-part",
        &[
            "--id",
            PARTS[3],
            "--operation-id",
            "019a7bf0-0000-7000-8000-00000000aa23",
            "--vertex-index",
            "0",
            "--first-distance",
            "15",
            "--second-distance",
            "15",
        ],
    );
    let after_chamfer = parts(&s.ok("draft", "inspect-elements", &[]));
    assert_eq!(after_chamfer.len(), 7);
    assert!(
        volume_of(&after_chamfer, PARTS[3]) < volume_of(&after_split, PARTS[3]),
        "chamfer must remove material"
    );
    for id in [PARTS[0], PARTS[1], PARTS[2], PARTS[4], SPLIT_HALF] {
        close(volume_of(&after_chamfer, id), volume_of(&after_split, id));
    }
    close(
        volume_of(&after_chamfer, copy_id.as_str()),
        volume_of(&after_copy, copy_id.as_str()),
    );

    // Undo returns the workspace across one operation; redo restores it. Workspace commands
    // address the workspace folder directly, without a store argument.
    let undo_json = s.execute(&["undo-workspace", "--workspace-dir", &s.path("draft")]);
    assert_eq!(undo_json.0, 0);
    assert_eq!(text(&undo_json.1, "outcome"), "workspace_undone");
    let after_undo = parts(&s.ok("draft", "inspect-elements", &[]));
    assert_eq!(after_undo.len(), 7);
    close(
        volume_of(&after_undo, PARTS[3]),
        volume_of(&after_split, PARTS[3]),
    );
    let redo_json = s.execute(&["redo-workspace", "--workspace-dir", &s.path("draft")]);
    assert_eq!(redo_json.0, 0);
    assert_eq!(text(&redo_json.1, "outcome"), "workspace_redone");
    let after_redo = parts(&s.ok("draft", "inspect-elements", &[]));
    assert_eq!(after_redo.len(), 7);
    close(
        volume_of(&after_redo, PARTS[3]),
        volume_of(&after_chamfer, PARTS[3]),
    );

    // Member holes (A04.2, the G14.1 transverse slice): a through round hole along the
    // section width (u) direction at the section centre, half way up the first column. The
    // stored polygon is the geometry: d = 20 takes the regular 32-gon of F06 section 2, so
    // the hole removes its area times the 100 mm width from the gross column volume.
    let holed = s.ok(
        "draft",
        "add-hole",
        &[
            "--id",
            PARTS[0],
            "--operation-id",
            MEMBER_HOLE_OP,
            "--axis-point",
            "0,0,500",
            "--axis-direction",
            "1,0,0",
            "--diameter",
            "20",
        ],
    );
    assert_eq!(text(&holed, "outcome"), "hole_added");
    let holed_member = at(&holed, &["element"]);
    let hole_records = at(holed_member, &["hole_records"]).array().unwrap();
    assert_eq!(hole_records.len(), 1);
    assert_eq!(text(&hole_records[0], "extent"), "through");
    let hole_area = 1600.0 * (std::f64::consts::PI / 16.0).sin();
    close(
        at(holed_member, &["quantities", "volume", "mm3"])
            .float()
            .unwrap(),
        5_000_000.0 - hole_area * 100.0,
    );
    // A transverse coordinate outside the section leaves no fit and writes nothing.
    let head_before_hole = s.head("draft");
    let refused_hole = s.run(
        "draft",
        "add-hole",
        &[
            "--id",
            PARTS[0],
            "--operation-id",
            "019a7bf0-0000-7000-8000-00000000aa25",
            "--axis-point",
            "0,500,500",
            "--axis-direction",
            "1,0,0",
            "--diameter",
            "20",
        ],
        2,
    );
    assert_eq!(text(&refused_hole, "code"), "INVALID_ARGUMENT");
    assert_eq!(s.head("draft"), head_before_hole);

    // User attributes (I04): definitions are project data, values ride on any element, and
    // an undefined name or an off-choice value is refused, never stored silently.
    s.ok(
        "draft",
        "attribute-define",
        &[
            "--name",
            "Paint_status",
            "--type",
            "choice",
            "--options",
            "New,Done",
        ],
    );
    s.ok(
        "draft",
        "attribute-define",
        &["--name", "Weight", "--type", "number", "--unit", "kg"],
    );
    assert_eq!(
        s.ok("draft", "attributes", &[])
            .get("attributes")
            .unwrap()
            .array()
            .unwrap()
            .len(),
        2
    );
    for (id, paint, weight) in [(PARTS[0], "New", "12.5"), (PARTS[3], "Done", "8.25")] {
        let set = s.ok(
            "draft",
            "attribute-set",
            &["--element", id, "--name", "Paint_status", "--value", paint],
        );
        assert_eq!(text(&set, "outcome"), "attribute_set");
        s.ok(
            "draft",
            "attribute-set",
            &["--element", id, "--name", "Weight", "--value", weight],
        );
    }
    let readback = s.ok("draft", "attribute-get", &["--element", PARTS[0]]);
    let entries = at(&readback, &["attributes"]).array().unwrap();
    assert_eq!(entries.len(), 2);
    for entry in entries.iter() {
        let name = text(entry, "name");
        if name == "Paint_status" {
            assert!(entry.get("defined").unwrap().boolean().unwrap());
            assert_eq!(text(entry, "value"), "New");
        } else {
            assert_eq!(name, "Weight");
            assert!(entry.get("defined").unwrap().boolean().unwrap());
            close(entry.get("value").unwrap().float().unwrap(), 12.5);
        }
    }
    let refused_attribute = s.run(
        "draft",
        "attribute-set",
        &["--element", PARTS[0], "--name", "Nope", "--value", "x"],
        2,
    );
    assert_eq!(text(&refused_attribute, "code"), "NOT_FOUND");
    let refused_choice = s.run(
        "draft",
        "attribute-set",
        &[
            "--element",
            PARTS[0],
            "--name",
            "Paint_status",
            "--value",
            "painted",
        ],
        2,
    );
    assert_eq!(text(&refused_choice, "code"), "INVALID_ARGUMENT");

    // Filters (V14): typed rules over the real frame through the engine evaluator. A query
    // never writes, a missing value never satisfies ne, and an unknown attribute refuses
    // instead of matching silently.
    let pinned_head = s.head("draft");
    fs::write(
        s.path("rule-plates.json"),
        r#"{"field":"type","op":"eq","value":"plate"}"#,
    )
    .unwrap();
    let plates = s.ok(
        "draft",
        "query-elements",
        &["--rule", &s.path("rule-plates.json")],
    );
    let plate_ids: Vec<String> = plates
        .get("elements")
        .unwrap()
        .array()
        .unwrap()
        .iter()
        .map(|e| e.string().unwrap().to_owned())
        .collect();
    assert_eq!(plate_ids.len(), 3);
    for id in [PARTS[3], PARTS[4]] {
        assert!(plate_ids.iter().any(|found| found == id), "{id}");
    }
    assert!(plate_ids.iter().any(|found| found == copy_id.as_str()));
    fs::write(
        s.path("rule-paint.json"),
        r#"{"field":"attribute","attribute":"Paint_status","op":"ne","value":"Done"}"#,
    )
    .unwrap();
    let painted = s.ok(
        "draft",
        "query-elements",
        &["--rule", &s.path("rule-paint.json")],
    );
    let painted_ids: Vec<String> = painted
        .get("elements")
        .unwrap()
        .array()
        .unwrap()
        .iter()
        .map(|e| e.string().unwrap().to_owned())
        .collect();
    assert_eq!(painted_ids, vec![PARTS[0].to_owned()]);
    fs::write(
        s.path("rule-combined.json"),
        r#"{"all":[{"field":"type","op":"eq","value":"column"},{"any":[{"field":"attribute","attribute":"Paint_status","op":"eq","value":"New"},{"field":"name","op":"contains","value":"no-such-name"}]}]}"#,
    )
    .unwrap();
    let combined = s.ok(
        "draft",
        "query-elements",
        &["--rule", &s.path("rule-combined.json")],
    );
    let combined_ids: Vec<String> = combined
        .get("elements")
        .unwrap()
        .array()
        .unwrap()
        .iter()
        .map(|e| e.string().unwrap().to_owned())
        .collect();
    assert_eq!(combined_ids, vec![PARTS[0].to_owned()]);
    fs::write(
        s.path("rule-unknown.json"),
        r#"{"field":"attribute","attribute":"Missing","op":"eq","value":1}"#,
    )
    .unwrap();
    s.run(
        "draft",
        "query-elements",
        &["--rule", &s.path("rule-unknown.json")],
        2,
    );
    assert_eq!(s.head("draft"), pinned_head);

    // The draft reference for the publication round trip: the frame as it stands now,
    // including the member hole; attribute edits below never touch a mesh.
    let final_parts = parts(&s.ok("draft", "inspect-elements", &[]));

    // Grids and levels are storey model data the real store carries (T01/D2).
    s.ok(
        "draft",
        "create-grid",
        &[
            "--name",
            "Q01 grid",
            "--new-id",
            GRID,
            "--u-expression",
            "0 1000",
            "--v-expression",
            "0",
        ],
    );
    assert_eq!(
        s.ok("draft", "list-grids", &[])
            .get("grids")
            .unwrap()
            .array()
            .unwrap()
            .len(),
        1
    );
    s.ok(
        "draft",
        "create-level",
        &[
            "--name",
            "Q01 level",
            "--new-id",
            LEVEL,
            "--elevation",
            "1000",
        ],
    );
    assert_eq!(
        s.ok("draft", "levels", &[])
            .get("levels")
            .unwrap()
            .array()
            .unwrap()
            .len(),
        1
    );

    // Assembly, bolts and welds complete the frame (the original scenario keeps the details).
    let assembly = text(
        &s.ok(
            "draft",
            "create-assembly",
            &["--name", "Q01 expanded frame"],
        ),
        "element_id",
    );
    for id in [
        PARTS[0],
        PARTS[1],
        PARTS[2],
        PARTS[3],
        PARTS[4],
        SPLIT_HALF,
        copy_id.as_str(),
    ] {
        s.ok(
            "draft",
            "attach-to-assembly",
            &["--id", id, "--assembly-id", &assembly],
        );
    }
    s.ok(
        "draft",
        "create-weld",
        &[
            "--name",
            "Q01 weld",
            "--main",
            PARTS[4],
            "--secondary",
            PARTS[1],
            "--size",
            "6",
            "--location",
            "workshop",
            "--side",
            "both",
        ],
    );

    // Numbering and publication; the view bundle and BOM are then checked against the
    // published head.
    let token = text(&s.ok("draft", "preview-numbering", &[]), "preview_token");
    let approved = s.ok("draft", "approve-numbering", &["--preview-token", &token]);
    assert_eq!(approved.get("marks").unwrap().unsigned().unwrap(), 8);
    let published = s.ok(
        "published",
        "publish-workspace",
        &["--workspace-dir", &s.path("draft")],
    );
    assert_eq!(text(&published, "outcome"), "workspace_published");
    let accepted_head = s.head("published");

    // Reopen: every part keeps its exact volume and mesh across the publication round trip.
    let reopened = parts(&s.ok("published", "inspect-elements", &[]));
    assert_eq!(reopened.len(), final_parts.len());
    for (id, volume, digest) in &final_parts {
        let found = reopened.iter().find(|(other, _, _)| other == id).unwrap();
        close(found.1, *volume);
        assert_eq!(found.2, *digest, "{id} mesh must survive the round trip");
    }

    // Views: the synthetic scene stands in for P01 (recorded in the matrix), so the kind
    // filter is exercised there; the store itself exports its pinned view bundle (V03).
    let snapshot = s.execute(&["snapshot-view", "--plates", "10", "--hide", "support"]);
    assert_eq!(snapshot.0, 0);
    assert_eq!(text(&snapshot.1, "outcome"), "view_snapshot");
    assert_eq!(
        at(&snapshot.1, &["settings", "hidden_kinds"])
            .array()
            .unwrap()
            .len(),
        1
    );
    let view_args = ["--out", &s.path("view-1.bin")];
    s.ok("published", "export-view", &view_args);
    s.ok(
        "published",
        "export-view",
        &["--out", &s.path("view-2.bin")],
    );
    let first_bundle = fs::read(s.path("view-1.bin")).unwrap();
    assert!(!first_bundle.is_empty());
    assert_eq!(
        fs::read(s.path("view-2.bin")).unwrap(),
        first_bundle,
        "the view bundle must be deterministic for one head"
    );
    // A repeat export into the same file must refuse instead of overwriting the stored view.
    let refused_export = s.run("published", "export-view", &view_args, 2);
    assert_eq!(text(&refused_export, "code"), "INVALID_ARGUMENT");
    assert_eq!(fs::read(s.path("view-1.bin")).unwrap(), first_bundle);

    // Invalid-file regressions: each refusal leaves both heads intact.
    let missing_blob = s.run(
        "published",
        "put-blob",
        &["--file", &s.path("no-such-file")],
        2,
    );
    assert_eq!(text(&missing_blob, "outcome"), "rejected");
    let empty = s.run("empty-store", "open-revision", &[], 2);
    assert_eq!(text(&empty, "outcome"), "rejected");
    assert_eq!(s.head("published"), accepted_head);

    // Cancellation regression: an interrupted command answers "cancelled" and commits nothing;
    // recovery then reports a clean store (the full crash path lives in tests/recovery.rs).
    let cancelled = s.run(
        "published",
        "put-blob",
        &["--file", &s.path("view-1.bin"), "--cancel-after-ms", "0"],
        130,
    );
    assert_eq!(text(&cancelled, "outcome"), "cancelled");
    let recovered = s.ok("published", "recover-revision", &["--min-age-secs", "0"]);
    assert_eq!(text(&recovered, "outcome"), "recovery_complete");
    assert_eq!(s.head("published"), accepted_head);

    // Exact stored outputs: the BOM and its schedule summary reproduce byte for byte.
    for (name, summary) in [
        ("bom.csv", "bom-summary-1.csv"),
        ("bom-again.csv", "bom-summary-2.csv"),
    ] {
        s.ok(
            "published",
            "export-bom",
            &["--out", &s.path(name), "--summary", &s.path(summary)],
        );
    }
    assert_eq!(
        fs::read(s.path("bom.csv")).unwrap(),
        fs::read(s.path("bom-again.csv")).unwrap()
    );
    assert_eq!(
        fs::read(s.path("bom-summary-1.csv")).unwrap(),
        fs::read(s.path("bom-summary-2.csv")).unwrap()
    );

    // Schedules (I05): the same pinned head renders a typed template — attribute columns,
    // group totals that keep unknown cells visible, and HTML — byte-identically on repeat.
    // Four members share Q01-RECT and three plates share the plate profile, so two groups.
    fs::write(
        s.path("schedule.json"),
        r#"{"version":1,"columns":[{"column":"mark"},{"column":"profile"},{"attribute":"Paint_status"},{"attribute":"Weight","total":true}],"group_by":["profile"],"sort_by":["mark"]}"#,
    )
    .unwrap();
    let template_path = s.path("schedule.json");
    let schedule_out = s.path("schedule-1.csv");
    let schedule_groups = s.path("schedule-groups-1.csv");
    let schedule_html = s.path("schedule-1.html");
    let schedule_args = [
        "--template",
        template_path.as_str(),
        "--out",
        schedule_out.as_str(),
        "--summary",
        schedule_groups.as_str(),
        "--html",
        schedule_html.as_str(),
    ];
    let scheduled = s.ok("published", "schedule-export", &schedule_args);
    assert_eq!(text(&scheduled, "outcome"), "schedule_exported");
    assert_eq!(scheduled.get("elements").unwrap().unsigned().unwrap(), 7);
    assert_eq!(scheduled.get("groups").unwrap().unsigned().unwrap(), 2);
    assert_eq!(
        at(&scheduled, &["unknown_columns"]).array().unwrap().len(),
        0
    );
    let elements_csv = fs::read_to_string(s.path("schedule-1.csv")).unwrap();
    assert_eq!(elements_csv.lines().count(), 8);
    assert_eq!(elements_csv.matches(",New,").count(), 1);
    assert_eq!(elements_csv.matches(",Done,").count(), 1);
    let groups_csv = fs::read_to_string(s.path("schedule-groups-1.csv")).unwrap();
    let group_lines: Vec<&str> = groups_csv.lines().collect();
    assert_eq!(group_lines.len(), 3);
    assert_eq!(group_lines[0], "profile,count,Weight,Weight_unknown");
    assert_eq!(
        group_lines
            .iter()
            .find(|line| line.starts_with("Q01-RECT,"))
            .unwrap(),
        &"Q01-RECT,4,12.5,3"
    );
    let plate_group = group_lines
        .iter()
        .find(|line| !line.starts_with("profile,") && !line.starts_with("Q01-RECT,"))
        .unwrap();
    assert!(plate_group.ends_with(",3,8.25,2"), "{plate_group:?}");
    let page = fs::read_to_string(s.path("schedule-1.html")).unwrap();
    assert!(
        page.contains("<h2>Elements</h2>")
            && page.contains("<h2>Groups</h2>")
            && page.contains("<th>Paint_status</th>"),
        "{page}"
    );
    s.ok(
        "published",
        "schedule-export",
        &[
            "--template",
            s.path("schedule.json").as_str(),
            "--out",
            s.path("schedule-2.csv").as_str(),
            "--summary",
            s.path("schedule-groups-2.csv").as_str(),
            "--html",
            s.path("schedule-2.html").as_str(),
        ],
    );
    for (first, second) in [
        ("schedule-1.csv", "schedule-2.csv"),
        ("schedule-groups-1.csv", "schedule-groups-2.csv"),
        ("schedule-1.html", "schedule-2.html"),
    ] {
        assert_eq!(
            fs::read(s.path(first)).unwrap(),
            fs::read(s.path(second)).unwrap(),
            "{first} must reproduce byte for byte"
        );
    }
    // An unknown attribute column stays in the schedule, renders empty and is reported;
    // a filter narrows the rows through the engine's own evaluator.
    fs::write(
        s.path("schedule-ghost.json"),
        r#"{"version":1,"columns":[{"column":"mark"},{"attribute":"Ghost"}]}"#,
    )
    .unwrap();
    let ghost = s.ok(
        "published",
        "schedule-export",
        &[
            "--template",
            s.path("schedule-ghost.json").as_str(),
            "--out",
            s.path("schedule-ghost.csv").as_str(),
        ],
    );
    assert_eq!(
        at(&ghost, &["unknown_columns"])
            .array()
            .unwrap()
            .first()
            .unwrap()
            .string()
            .unwrap(),
        "Ghost"
    );
    assert_eq!(
        fs::read_to_string(s.path("schedule-ghost.csv"))
            .unwrap()
            .lines()
            .next()
            .unwrap(),
        "mark,Ghost"
    );
    fs::write(
        s.path("schedule-beams.json"),
        r#"{"version":1,"columns":[{"column":"kind"}],"filter":{"key":"kind","op":"eq","value":"beam"}}"#,
    )
    .unwrap();
    let beams = s.ok(
        "published",
        "schedule-export",
        &[
            "--template",
            s.path("schedule-beams.json").as_str(),
            "--out",
            s.path("schedule-beams.csv").as_str(),
        ],
    );
    assert_eq!(beams.get("elements").unwrap().unsigned().unwrap(), 1);
    let beams_csv = fs::read_to_string(s.path("schedule-beams.csv")).unwrap();
    assert_eq!(beams_csv.lines().count(), 2);
    assert_eq!(beams_csv.lines().nth(1).unwrap(), "beam");

    // The full drawing document surface over the numbered frame (the stale/version cycle
    // lives in the original scenario; this checks the fresh path end to end and then grows
    // the document: D04 placed/section/detail views with per-view titles and scales, a D05
    // sheet, D06/D07 drawn marks and leaders on owned layers, and D08 PDF export of each
    // exact published version).
    let id = |suffix: u16| format!("019a7bf0-0000-7000-8000-{suffix:012}");
    let drawing = id(0xaa30);
    let view = id(0xaa31);
    let dimension = id(0xaa32);
    let create_op = id(0xaa33);
    let create_revision = id(0xaa34);
    let publish_op = id(0xaa35);
    let publish_revision = id(0xaa36);
    let version = id(0xaa37);
    let placed = id(0xaa38);
    let section = id(0xaa39);
    let detail = id(0xaa3a);
    let views_op = id(0xaa3b);
    let views_revision = id(0xaa3c);
    let sheet_op = id(0xaa3d);
    let sheet_revision = id(0xaa3e);
    let annotations = id(0xaa3f);
    let construction = id(0xaa40);
    let leader = id(0xaa41);
    let mark = id(0xaa42);
    let free = id(0xaa43);
    let hidden = id(0xaa44);
    let drafting_op = id(0xaa45);
    let drafting_revision = id(0xaa46);
    let publish_two_op = id(0xaa47);
    let publish_two_revision = id(0xaa48);
    let version_two = id(0xaa49);
    let anchor_a = format!("fid1:{}:member:v.x0.y0.z0", PARTS[0]);
    let anchor_b = format!("fid1:{}:member:v.x0.y0.z1", PARTS[0]);
    let created = s.ok(
        "published",
        "create-drawing",
        &[
            "--document-dir",
            &s.path("drawing"),
            "--drawing-id",
            &drawing,
            "--view-id",
            &view,
            "--dimension-id",
            &dimension,
            "--id",
            PARTS[0],
            "--anchor-a",
            &anchor_a,
            "--anchor-b",
            &anchor_b,
            "--view-title",
            "COLUMN A",
            "--scale",
            "20",
            "--op-id",
            &create_op,
            "--revision-id",
            &create_revision,
            "--actor-id",
            USER,
        ],
    );
    assert_eq!(text(&created, "state"), "draft");
    assert_eq!(text(&created, "source_revision_id"), accepted_head);
    assert_eq!(
        at(&created, &["view", "title"]).string().unwrap(),
        "COLUMN A"
    );
    close(
        at(&created, &["view", "scale_denominator"])
            .float()
            .unwrap(),
        20.0,
    );
    let published_drawing = s.ok(
        "published",
        "publish-drawing",
        &[
            "--document-dir",
            &s.path("drawing"),
            "--expected-head",
            &create_revision,
            "--op-id",
            &publish_op,
            "--revision-id",
            &publish_revision,
            "--actor-id",
            USER,
            "--version-id",
            &version,
        ],
    );
    assert_eq!(text(&published_drawing, "state"), "published");
    assert_eq!(
        text(
            &s.ok(
                "published",
                "inspect-drawing",
                &[
                    "--document-dir",
                    &s.path("drawing"),
                    "--model-store",
                    &s.path("published")
                ]
            ),
            "freshness"
        ),
        "current"
    );

    // D04: placed, section and detail views join the primary view, each with its own title
    // and display scale. Placement belongs to the document; the model is never re-read.
    let placed_views = s.ok(
        "published",
        "edit-views",
        &[
            "--document-dir",
            &s.path("drawing"),
            "--expected-head",
            &publish_revision,
            "--op-id",
            &views_op,
            "--revision-id",
            &views_revision,
            "--actor-id",
            USER,
            "--placed-view",
            &format!("{placed}:top:10:140,70:TOP VIEW"),
            "--placed-section",
            &format!("{section}:right:0:positive:5:200,70:SECTION A-A"),
            "--placed-detail",
            &format!("{detail}:{placed}:25,35:15:2:260,70:DETAIL B"),
        ],
    );
    let placed_list = at(&placed_views, &["placed_views"]).array().unwrap();
    assert_eq!(placed_list.len(), 1);
    assert_eq!(text(&placed_list[0], "id"), placed);
    assert_eq!(text(&placed_list[0], "title"), "TOP VIEW");
    close(
        at(&placed_list[0], &["scale_denominator"]).float().unwrap(),
        10.0,
    );
    let section_list = at(&placed_views, &["placed_sections"]).array().unwrap();
    assert_eq!(section_list.len(), 1);
    assert_eq!(text(&section_list[0], "id"), section);
    assert_eq!(text(&section_list[0], "plane"), "right");
    assert_eq!(text(&section_list[0], "side"), "positive");
    assert_eq!(text(&section_list[0], "title"), "SECTION A-A");
    close(at(&section_list[0], &["cut_mm"]).float().unwrap(), 0.0);
    close(
        at(&section_list[0], &["scale_denominator"])
            .float()
            .unwrap(),
        5.0,
    );
    let detail_list = at(&placed_views, &["placed_details"]).array().unwrap();
    assert_eq!(detail_list.len(), 1);
    assert_eq!(text(&detail_list[0], "id"), detail);
    assert_eq!(text(&detail_list[0], "parent"), placed);
    assert_eq!(text(&detail_list[0], "title"), "DETAIL B");
    close(at(&detail_list[0], &["radius_mm"]).float().unwrap(), 15.0);
    close(
        at(&detail_list[0], &["scale_denominator"]).float().unwrap(),
        2.0,
    );

    // D05: the document lands on an a3 sheet with a title block; every export surface
    // renders this fixed sheet from here on.
    let sheet = s.ok(
        "published",
        "edit-sheet",
        &[
            "--document-dir",
            &s.path("drawing"),
            "--expected-head",
            &views_revision,
            "--op-id",
            &sheet_op,
            "--revision-id",
            &sheet_revision,
            "--actor-id",
            USER,
            "--sheet-size",
            "a3",
            "--sheet-frame",
            "10",
            "--sheet-project",
            "Q01 alpha frame",
            "--sheet-author",
            "Q01 expanded story",
        ],
    );
    assert_eq!(at(&sheet, &["sheet", "size"]).string().unwrap(), "a3");
    assert_eq!(
        at(&sheet, &["sheet", "project"]).string().unwrap(),
        "Q01 alpha frame"
    );

    // D06/D07: drawn marks, a leader and a free dimension join document-owned layers; the
    // circle joins a hidden layer, so the paper-visible subset excludes it.
    let drafting = s.ok(
        "published",
        "edit-drafting",
        &[
            "--document-dir",
            &s.path("drawing"),
            "--expected-head",
            &sheet_revision,
            "--op-id",
            &drafting_op,
            "--revision-id",
            &drafting_revision,
            "--actor-id",
            USER,
            "--layer",
            &format!("{annotations}:Q01 annotations:on"),
            "--layer",
            &format!("{construction}:Q01 construction:off"),
            "--drafting",
            &format!("{leader}:line:240,70,252,78:{annotations}:continuous"),
            "--drafting",
            &format!("{mark}:text:240,84,4,0:{annotations}:continuous:MARK P/1(1)"),
            "--drafting",
            &format!("{free}:free_dimension:0,0,1000,0,15,1:{annotations}:dashdot"),
            "--drafting",
            &format!("{hidden}:circle:290,70,8:{construction}:dashed"),
        ],
    );
    let layer_list = at(&drafting, &["layers"]).array().unwrap();
    assert_eq!(layer_list.len(), 2);
    assert_eq!(text(&layer_list[0], "name"), "Q01 annotations");
    assert_eq!(text(&layer_list[1], "name"), "Q01 construction");
    assert!(!at(&layer_list[1], &["visible"]).boolean().unwrap());
    let drawn = at(&drafting, &["drafting"]).array().unwrap();
    assert_eq!(drawn.len(), 4);
    assert_eq!(
        at(&drafting, &["visible_drafting"]).array().unwrap().len(),
        3,
        "the hidden layer must hold its object out of the paper-visible subset"
    );
    assert_eq!(
        text(
            &s.ok(
                "published",
                "inspect-drawing",
                &[
                    "--document-dir",
                    &s.path("drawing"),
                    "--model-store",
                    &s.path("published")
                ]
            ),
            "freshness"
        ),
        "current",
        "document edits must keep the pinned source revision"
    );

    // Second publication: the grown document becomes version two, and both versions stay
    // listed and exportable from their own archived source.
    let published_two = s.ok(
        "published",
        "publish-drawing",
        &[
            "--document-dir",
            &s.path("drawing"),
            "--expected-head",
            &drafting_revision,
            "--op-id",
            &publish_two_op,
            "--revision-id",
            &publish_two_revision,
            "--actor-id",
            USER,
            "--version-id",
            &version_two,
        ],
    );
    assert_eq!(text(&published_two, "state"), "published");
    assert_eq!(text(&published_two, "version_id"), version_two);
    let version_list = s.ok(
        "published",
        "inspect-drawing-versions",
        &["--document-dir", &s.path("drawing")],
    );
    assert_eq!(at(&version_list, &["versions"]).array().unwrap().len(), 2);

    // D08: the PDF of one exact published version is byte-deterministic across repeats, and
    // each version renders its own stored document.
    s.ok(
        "published",
        "export-drawing",
        &[
            "--document-dir",
            &s.path("drawing"),
            "--version-id",
            &version_two,
            "--out",
            &s.path("drawing-v2.pdf"),
            "--format",
            "pdf",
        ],
    );
    s.ok(
        "published",
        "export-drawing",
        &[
            "--document-dir",
            &s.path("drawing"),
            "--version-id",
            &version_two,
            "--out",
            &s.path("drawing-v2-again.pdf"),
            "--format",
            "pdf",
        ],
    );
    let second = fs::read(s.path("drawing-v2.pdf")).unwrap();
    assert!(second.starts_with(b"%PDF-"));
    assert_eq!(
        fs::read(s.path("drawing-v2-again.pdf")).unwrap(),
        second,
        "the PDF must be deterministic for one version"
    );
    s.ok(
        "published",
        "export-drawing",
        &[
            "--document-dir",
            &s.path("drawing"),
            "--version-id",
            &version,
            "--out",
            &s.path("drawing-v1.pdf"),
            "--format",
            "pdf",
        ],
    );
    let first = fs::read(s.path("drawing-v1.pdf")).unwrap();
    assert!(first.starts_with(b"%PDF-"));
    assert_ne!(
        first, second,
        "each version must render its own stored document"
    );

    // D04.3: view titles and display scales in a second document, end to end. The primary
    // view carries --view-title on create and on refresh, the edit-views specs carry the
    // optional :title segment (one placed view stays unannotated), the sheet export
    // annotates every view with its title and 1:N scale, inspect-drawing reports the title
    // fields, and the stored intent binds the titles to the closed descriptor through its
    // schema identity.
    let titled_drawing = id(0xaa50);
    let titled_view = id(0xaa51);
    let titled_dimension = id(0xaa52);
    let titled_create_op = id(0xaa53);
    let titled_create_revision = id(0xaa54);
    let titled_refresh_op = id(0xaa55);
    let titled_refresh_revision = id(0xaa56);
    let plan_view = id(0xaa57);
    let bare_view = id(0xaa58);
    let titled_section = id(0xaa59);
    let titled_detail = id(0xaa5a);
    let titled_views_op = id(0xaa5b);
    let titled_views_revision = id(0xaa5c);
    let titled_sheet_op = id(0xaa5d);
    let titled_sheet_revision = id(0xaa5e);
    let titled_publish_op = id(0xaa5f);
    let titled_publish_revision = id(0xaa60);
    let titled_version = id(0xaa61);
    let refused_op = id(0xaa62);
    let refused_revision = id(0xaa63);
    let titled_dir = s.path("drawing-titled");
    // A title over the bounded label cap refuses before anything is stored, so the
    // document directory is not even created.
    let long_title = "A".repeat(257);
    let refused_title = s.run(
        "published",
        "create-drawing",
        &[
            "--document-dir",
            &titled_dir,
            "--drawing-id",
            &titled_drawing,
            "--view-id",
            &titled_view,
            "--dimension-id",
            &titled_dimension,
            "--id",
            PARTS[0],
            "--anchor-a",
            &anchor_a,
            "--anchor-b",
            &anchor_b,
            "--view-title",
            long_title.as_str(),
            "--op-id",
            &refused_op,
            "--revision-id",
            &refused_revision,
            "--actor-id",
            USER,
        ],
        2,
    );
    assert_eq!(text(&refused_title, "code"), "INVALID_ARGUMENT");
    let titled_create = s.ok(
        "published",
        "create-drawing",
        &[
            "--document-dir",
            &titled_dir,
            "--drawing-id",
            &titled_drawing,
            "--view-id",
            &titled_view,
            "--dimension-id",
            &titled_dimension,
            "--id",
            PARTS[0],
            "--anchor-a",
            &anchor_a,
            "--anchor-b",
            &anchor_b,
            "--view-title",
            "COLUMN A",
            "--scale",
            "20",
            "--origin",
            "45,60",
            "--op-id",
            &titled_create_op,
            "--revision-id",
            &titled_create_revision,
            "--actor-id",
            USER,
        ],
    );
    assert_eq!(text(&titled_create, "state"), "draft");
    assert_eq!(text(at(&titled_create, &["view"]), "title"), "COLUMN A");
    close(
        at(&titled_create, &["view", "scale_denominator"])
            .float()
            .unwrap(),
        20.0,
    );

    // Refresh re-authors the primary view annotation from the same archived source; the
    // exact repeat is a replay that returns the stored revision byte for byte.
    let refresh_args = [
        "--document-dir",
        &titled_dir,
        "--expected-head",
        &titled_create_revision,
        "--drawing-id",
        &titled_drawing,
        "--view-id",
        &titled_view,
        "--dimension-id",
        &titled_dimension,
        "--id",
        PARTS[0],
        "--anchor-a",
        &anchor_a,
        "--anchor-b",
        &anchor_b,
        "--view-title",
        "COLUMN A REV 2",
        "--scale",
        "25",
        "--origin",
        "45,60",
        "--op-id",
        &titled_refresh_op,
        "--revision-id",
        &titled_refresh_revision,
        "--actor-id",
        USER,
    ];
    let titled_refresh = s.ok("published", "refresh-drawing", &refresh_args);
    assert_eq!(
        text(at(&titled_refresh, &["view"]), "title"),
        "COLUMN A REV 2"
    );
    close(
        at(&titled_refresh, &["view", "scale_denominator"])
            .float()
            .unwrap(),
        25.0,
    );
    let refreshed_again = s.ok("published", "refresh-drawing", &refresh_args);
    assert_eq!(
        refreshed_again.encode(&Limits::DEFAULT).unwrap(),
        titled_refresh.encode(&Limits::DEFAULT).unwrap(),
        "the refresh replay must return the stored revision unchanged"
    );

    // One edit places the titled family: a placed view whose title is the whole remainder
    // of its spec (colons and spaces included), an unannotated placed view, and a titled
    // section and detail; the exact repeat replays byte for byte.
    let plan_spec = format!("{plan_view}:top:10:140,70:PLAN: east");
    let bare_spec = format!("{bare_view}:top:25:140,110");
    let section_spec = format!("{titled_section}:right:50:positive:5:200,70:SECTION A-A");
    let detail_spec = format!("{titled_detail}:{plan_view}:145,72:5:2:260,70:DETAIL 1");
    let views_args = [
        "--document-dir",
        &titled_dir,
        "--expected-head",
        &titled_refresh_revision,
        "--op-id",
        &titled_views_op,
        "--revision-id",
        &titled_views_revision,
        "--actor-id",
        USER,
        "--placed-view",
        plan_spec.as_str(),
        "--placed-view",
        bare_spec.as_str(),
        "--placed-section",
        section_spec.as_str(),
        "--placed-detail",
        detail_spec.as_str(),
    ];
    let placed_family = s.ok("published", "edit-views", &views_args);
    let placed_list = at(&placed_family, &["placed_views"]).array().unwrap();
    assert_eq!(placed_list.len(), 2);
    assert_eq!(text(&placed_list[0], "title"), "PLAN: east");
    close(
        at(&placed_list[0], &["scale_denominator"]).float().unwrap(),
        10.0,
    );
    assert_eq!(
        placed_list[1].get("title").unwrap(),
        &Json::Null,
        "an absent title segment is the unannotated view"
    );
    close(
        at(&placed_list[1], &["scale_denominator"]).float().unwrap(),
        25.0,
    );
    let section_list = at(&placed_family, &["placed_sections"]).array().unwrap();
    assert_eq!(text(&section_list[0], "title"), "SECTION A-A");
    close(at(&section_list[0], &["cut_mm"]).float().unwrap(), 50.0);
    close(
        at(&section_list[0], &["scale_denominator"])
            .float()
            .unwrap(),
        5.0,
    );
    let detail_list = at(&placed_family, &["placed_details"]).array().unwrap();
    assert_eq!(text(&detail_list[0], "title"), "DETAIL 1");
    assert_eq!(text(&detail_list[0], "parent"), plan_view);
    let placed_again = s.ok("published", "edit-views", &views_args);
    assert_eq!(
        placed_again.encode(&Limits::DEFAULT).unwrap(),
        placed_family.encode(&Limits::DEFAULT).unwrap(),
        "the edit-views replay must return the stored revision unchanged"
    );

    // The annotated document lands on an a3 sheet and publishes as one version; its sheet
    // render annotates all five views and repeats byte-identically.
    s.ok(
        "published",
        "edit-sheet",
        &[
            "--document-dir",
            &titled_dir,
            "--expected-head",
            &titled_views_revision,
            "--op-id",
            &titled_sheet_op,
            "--revision-id",
            &titled_sheet_revision,
            "--actor-id",
            USER,
            "--sheet-size",
            "a3",
            "--sheet-project",
            "Q01 titled sheet",
            "--sheet-author",
            "Q01 expanded story",
        ],
    );
    s.ok(
        "published",
        "publish-drawing",
        &[
            "--document-dir",
            &titled_dir,
            "--expected-head",
            &titled_sheet_revision,
            "--op-id",
            &titled_publish_op,
            "--revision-id",
            &titled_publish_revision,
            "--actor-id",
            USER,
            "--version-id",
            &titled_version,
        ],
    );
    for name in ["titled-sheet-1.svg", "titled-sheet-2.svg"] {
        s.ok(
            "published",
            "export-drawing",
            &[
                "--document-dir",
                &titled_dir,
                "--version-id",
                &titled_version,
                "--out",
                &s.path(name),
                "--format",
                "svg",
            ],
        );
    }
    let sheet_svg = fs::read(s.path("titled-sheet-1.svg")).unwrap();
    assert_eq!(
        fs::read(s.path("titled-sheet-2.svg")).unwrap(),
        sheet_svg,
        "the sheet render must be deterministic for one version"
    );
    let sheet_svg = String::from_utf8(sheet_svg).unwrap();
    // One annotation line per view: the primary, both placed views, the section and the
    // detail. Sections and details render their own geometry, not only the annotation, and
    // the refreshed primary scale rides in the render metadata.
    assert_eq!(sheet_svg.matches("class=\"viewtitle\"").count(), 5);
    for view_id in [
        titled_view.as_str(),
        plan_view.as_str(),
        bare_view.as_str(),
        titled_section.as_str(),
        titled_detail.as_str(),
    ] {
        assert!(
            sheet_svg.contains(&format!("data-view=\"{view_id}\"")),
            "{view_id}"
        );
    }
    assert!(
        sheet_svg.contains(&format!("class=\"body\" data-view=\"{titled_section}\"")),
        "the section renders its own sheet geometry"
    );
    assert!(
        sheet_svg.contains(&format!("class=\"body\" data-view=\"{titled_detail}\"")),
        "the detail renders its own sheet geometry"
    );
    assert!(
        sheet_svg.contains("scale_denominator=25"),
        "the refreshed primary scale annotates the render metadata"
    );

    // inspect-drawing reports every title field, including the null of the unannotated view.
    let inspected_titles = s.ok(
        "published",
        "inspect-drawing",
        &[
            "--document-dir",
            &titled_dir,
            "--model-store",
            &s.path("published"),
        ],
    );
    assert_eq!(text(&inspected_titles, "freshness"), "current");
    assert_eq!(
        text(at(&inspected_titles, &["view"]), "title"),
        "COLUMN A REV 2"
    );
    let reported_views = at(&inspected_titles, &["placed_views"]).array().unwrap();
    assert_eq!(text(&reported_views[0], "title"), "PLAN: east");
    assert_eq!(reported_views[1].get("title").unwrap(), &Json::Null);
    assert_eq!(
        text(
            &at(&inspected_titles, &["placed_sections"]).array().unwrap()[0],
            "title"
        ),
        "SECTION A-A"
    );
    assert_eq!(
        text(
            &at(&inspected_titles, &["placed_details"]).array().unwrap()[0],
            "title"
        ),
        "DETAIL 1"
    );

    // The stored intent carries each title beside its view scope and binds the document to
    // the closed descriptor through its schema identity; the unannotated view authors no
    // title line, so an untitled document keeps its byte-stable form.
    let documents =
        beamcraft_cli::drawing::document::store::DocumentStore::open(Path::new(&titled_dir))
            .unwrap();
    let stored = documents.head().unwrap();
    assert_eq!(stored.version.unwrap().to_string(), titled_version);
    let intent_entry = stored
        .info
        .manifest()
        .content()
        .entries()
        .iter()
        .find(|entry| entry.name() == beamcraft_cli::drawing::codec::INTENT_LAYER)
        .unwrap();
    let blobs = beamcraft_host::storage::BlobStore::new(Path::new(&titled_dir)).unwrap();
    let intent = String::from_utf8(blobs.read(&intent_entry.digest()).unwrap()).unwrap();
    assert!(beamcraft_cli::drawing::codec::SCHEMA_DESCRIPTOR.contains("view-title=drawing:title"));
    assert!(intent.contains(&format!(
        "custom string drawing:schema = \"{}\"",
        beamcraft_cli::drawing::codec::schema_identity().unwrap()
    )));
    assert_eq!(intent.matches("custom string drawing:title = ").count(), 4);
    for title in ["COLUMN A REV 2", "PLAN: east", "SECTION A-A", "DETAIL 1"] {
        assert!(
            intent.contains(&format!("custom string drawing:title = \"{title}\"")),
            "{title}"
        );
    }
}
