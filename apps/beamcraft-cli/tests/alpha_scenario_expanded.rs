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
// Q01 slice D (the delivered G14.2 surface): the oblique/sloped members and their hole
// operation ids.
const OBLIQUE_MEMBER: &str = "019a7bf0-0000-7000-8000-00000000aa70";
const OBLIQUE_HOLE_OP: &str = "019a7bf0-0000-7000-8000-00000000aa71";
const SLOPED_MEMBER: &str = "019a7bf0-0000-7000-8000-00000000aa72";
const SLOPED_HOLE_OP: &str = "019a7bf0-0000-7000-8000-00000000aa73";
const AXIAL_HOLE_OP: &str = "019a7bf0-0000-7000-8000-00000000aa76";
const SECOND_HOLE_OP: &str = "019a7bf0-0000-7000-8000-00000000aa77";
// The tube remainder: the CHS member and its hole operation id.
const TUBE_MEMBER: &str = "019a7bf0-0000-7000-8000-00000000aa78";
const TUBE_HOLE_OP: &str = "019a7bf0-0000-7000-8000-00000000aa79";

/// Closed-form volumes of the five parts before any tool touches them (as in the original
/// scenario): two columns, the cut beam, and two plates with one rectangular hole each.
const STARTING_VOLUMES: [f64; 5] = [5_000_000.0, 5_000_000.0, 4_750_000.0, 396_000.0, 396_000.0];

/// The shipped synthetic fastener library (contributor-original, MPL-2.0).
const SYNTHETIC_FASTENERS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/libraries/synthetic-fasteners.usda"
);

/// The measured quantity drop of one connected plate under its group's two generated
/// washered d 12 holes (the engine suites certify the hole geometry; the story pins the
/// executed drop).
const BOLT_DROP_MM3: f64 = 2_838.81;

struct Story(PathBuf);

impl Story {
    fn new() -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("q01-expanded-{}", crate::support::boot_nonce()));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("published")).unwrap();
        fs::create_dir(root.join("curved-published")).unwrap();
        fs::create_dir(root.join("plates-published")).unwrap();
        fs::create_dir(root.join("tools-published")).unwrap();
        fs::create_dir(root.join("bolts-published")).unwrap();
        fs::create_dir(root.join("data-published")).unwrap();
        fs::create_dir(root.join("contour-published")).unwrap();
        fs::create_dir(root.join("classes-published")).unwrap();
        fs::create_dir(root.join("cuts-published")).unwrap();
        fs::create_dir(root.join("fits-published")).unwrap();
        fs::create_dir(root.join("holes-published")).unwrap();
        fs::create_dir(root.join("storeys-published")).unwrap();
        fs::create_dir(root.join("numbering-published")).unwrap();
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

fn numbers(json: &Json) -> Vec<f64> {
    json.array()
        .unwrap()
        .iter()
        .map(|v| v.float().unwrap())
        .collect()
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

    // Member holes (A04.2, the G14.2 transverse slice). The no-fit refusal runs on the
    // plain member first: a transverse coordinate outside the section leaves no fit and
    // writes nothing (the axis point stays mid-member — at the member's end the end-cap
    // clearance would refuse first), and the successful hole after it proves that.
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
            "0,500,250",
            "--axis-direction",
            "1,0,0",
            "--diameter",
            "20",
        ],
        2,
    );
    assert_eq!(text(&refused_hole, "code"), "INVALID_ARGUMENT");
    assert_eq!(s.head("draft"), head_before_hole);
    // The through round hole along the section width (u) direction at the section centre,
    // half way up the first column. The stored polygon is the geometry: d = 20 takes the
    // regular 32-gon of F06 section 2, so the hole removes its area times the 100 mm width
    // from the gross column volume.
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

    // Named views (V12): the storey plan, a grid-axis elevation and a free orbit become
    // named views in the W01 camera prim layer beside the store. A view is display state:
    // the draft head and the layer bytes stay identical across every read and every
    // refusal, and each provenance-bearing save captures the draft revision it was taken
    // against.
    let views_file = s.path("story-views.usda");
    let draft_head = s.head("draft");
    let save_named = |args: &[&str]| -> Json {
        let mut all = vec!["--file", views_file.as_str()];
        all.extend_from_slice(args);
        s.ok("draft", "save-view", &all)
    };
    let plan = save_named(&[
        "--name",
        "Q01 plan",
        "--parallel",
        "xy",
        "--offset",
        "1000",
        "--author",
        "Q01 story",
        "--comment",
        "the storey plan",
    ]);
    assert_eq!(text(&plan, "outcome"), "view_saved");
    let plan_view = &at(&plan, &["views"]).array().unwrap()[0];
    assert_eq!(text(plan_view, "name"), "Q01 plan");
    assert_eq!(
        numbers(at(plan_view, &["plane", "origin_mm"])),
        [0.0, 0.0, 1000.0]
    );
    let stamp = at(plan_view, &["provenance"]);
    assert_eq!(text(stamp, "author"), "Q01 story");
    assert_eq!(text(stamp, "comment"), "the storey plan");
    assert_eq!(text(stamp, "model_revision"), draft_head);
    assert!(at(stamp, &["created_at_ms"]).unsigned().unwrap() > 1_700_000_000_000);
    let axis = save_named(&[
        "--name",
        "Q01 axis 2",
        "--grid",
        "Q01 grid",
        "--axis",
        "2",
        "--author",
        "Q01 story",
    ]);
    let axis_view = &at(&axis, &["views"]).array().unwrap()[0];
    assert_eq!(text(axis_view, "name"), "Q01 axis 2");
    assert_eq!(
        numbers(at(axis_view, &["plane", "origin_mm"])),
        [1000.0, -2000.0, 0.0],
        "the elevation stands on the second U axis: its plane origin sits on the x = 1000 line"
    );
    let level_plan = save_named(&["--name", "Q01 level plan", "--level", "Q01 level"]);
    let level_view = &at(&level_plan, &["views"]).array().unwrap()[0];
    assert_eq!(text(level_view, "name"), "Q01 level plan");
    assert_eq!(
        numbers(at(level_view, &["plane", "origin_mm"])),
        [0.0, 0.0, 1000.0],
        "the level plan is the model XY plane at the level elevation"
    );
    assert!(matches!(at(level_view, &["provenance"]), Json::Null));
    let orbit = save_named(&["--name", "Q01 3d", "--yaw", "0.7", "--pitch", "0.85"]);
    let orbit_view = &at(&orbit, &["views"]).array().unwrap()[0];
    assert_eq!(text(orbit_view, "name"), "Q01 3d");
    assert_eq!(text(orbit_view, "projection"), "perspective");
    let listed = s.ok("draft", "list-views", &["--file", &views_file]);
    assert_eq!(at(&listed, &["view_count"]).unsigned().unwrap(), 4);
    let mut names: Vec<String> = at(&listed, &["views"])
        .array()
        .unwrap()
        .iter()
        .map(|v| text(v, "name"))
        .collect();
    names.sort();
    assert_eq!(
        names,
        ["Q01 3d", "Q01 axis 2", "Q01 level plan", "Q01 plan"]
    );
    assert_eq!(at(&listed, &["folders"]).array().unwrap().len(), 0);
    let layer_bytes = fs::read(&views_file).unwrap();
    let applied = s.ok(
        "draft",
        "apply-view",
        &["--file", &views_file, "--view", "q01 axis 2"],
    );
    assert_eq!(text(&applied, "outcome"), "view_applied");
    assert_eq!(
        numbers(at(&applied, &["camera_plane", "origin_mm"])),
        [1000.0, -2000.0, 0.0]
    );
    assert_eq!(text(at(&applied, &["provenance"]), "author"), "Q01 story");
    assert_eq!(
        text(at(&applied, &["provenance"]), "model_revision"),
        draft_head
    );
    let duplicate = s.run(
        "draft",
        "save-view",
        &[
            "--file",
            &views_file,
            "--name",
            "Q01 PLAN",
            "--parallel",
            "xy",
        ],
        2,
    );
    assert_eq!(text(&duplicate, "outcome"), "rejected");
    let missing = s.run(
        "draft",
        "apply-view",
        &["--file", &views_file, "--view", "no such view"],
        2,
    );
    assert_eq!(text(&missing, "outcome"), "rejected");
    assert_eq!(
        fs::read(&views_file).unwrap(),
        layer_bytes,
        "reads and refusals never rewrite the layer"
    );
    assert_eq!(s.head("draft"), draft_head, "views never write the model");

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

    // The named views survive the publication round trip: the camera prim layer is
    // store-independent display state, so the published head lists the same four views,
    // restores the exact camera, and the captured provenance still names the draft
    // revision the view was taken against.
    assert_eq!(
        s.ok("published", "list-views", &["--file", &views_file])
            .get("view_count")
            .unwrap()
            .unsigned()
            .unwrap(),
        4
    );
    let restored = s.ok(
        "published",
        "apply-view",
        &["--file", &views_file, "--view", "Q01 plan"],
    );
    assert_eq!(text(&restored, "outcome"), "view_applied");
    assert_eq!(
        numbers(at(&restored, &["camera_plane", "origin_mm"])),
        [0.0, 0.0, 1000.0]
    );
    assert_eq!(
        text(at(&restored, &["provenance"]), "model_revision"),
        draft_head
    );
    assert_eq!(
        fs::read(&views_file).unwrap(),
        layer_bytes,
        "publication and reads never rewrite the layer"
    );
    let removed = s.ok(
        "published",
        "remove-view",
        &["--file", &views_file, "--view", "Q01 3d"],
    );
    assert_eq!(text(&removed, "outcome"), "view_removed");
    assert_eq!(text(&removed, "name"), "Q01 3d");
    assert_eq!(at(&removed, &["view_count"]).unsigned().unwrap(), 3);
    let gone = s.run(
        "published",
        "apply-view",
        &["--file", &views_file, "--view", "Q01 3d"],
        2,
    );
    assert_eq!(text(&gone, "outcome"), "rejected");
    assert_eq!(
        s.head("published"),
        accepted_head,
        "views never write the model"
    );

    // Views: the kind filter over the real frame is exercised through query-elements above;
    // snapshot-view still runs on its synthetic kernel plate scene because P01 does not
    // exist (recorded in the matrix; no acceptance item traces to it), and the store
    // itself exports its pinned view bundle (V03).
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
            // The sheet export renders the whole fixed sheet, so the primary view must
            // sit inside the A3 frame: the 1000 mm column at 1:20 spans 50 mm of paper
            // y from this origin, clear of the 10 mm inset.
            "--origin",
            "40,60",
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
            &format!("{free}:free_dimension:40,60,90,60,15,1:{annotations}:dashdot"),
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
        beamcraft_commands::drawing::document::store::DocumentStore::open(Path::new(&titled_dir))
            .unwrap();
    let stored = documents.head().unwrap();
    assert_eq!(stored.version.unwrap().to_string(), titled_version);
    let intent_entry = stored
        .info
        .manifest()
        .content()
        .entries()
        .iter()
        .find(|entry| entry.name() == beamcraft_commands::drawing::codec::INTENT_LAYER)
        .unwrap();
    let blobs = beamcraft_host::storage::BlobStore::new(Path::new(&titled_dir)).unwrap();
    let intent = String::from_utf8(blobs.read(&intent_entry.digest()).unwrap()).unwrap();
    assert!(
        beamcraft_commands::drawing::codec::SCHEMA_DESCRIPTOR.contains("view-title=drawing:title")
    );
    assert!(intent.contains(&format!(
        "custom string drawing:schema = \"{}\"",
        beamcraft_commands::drawing::codec::schema_identity().unwrap()
    )));
    assert_eq!(intent.matches("custom string drawing:title = ").count(), 4);
    for title in ["COLUMN A REV 2", "PLAN: east", "SECTION A-A", "DETAIL 1"] {
        assert!(
            intent.contains(&format!("custom string drawing:title = \"{title}\"")),
            "{title}"
        );
    }

    // Member holes, oblique and sloped walls (Q01 slice D: the delivered G14.2
    // surface). Each case gets its own member based at the world origin: the replay maps
    // the record's member-frame axis point into element coordinates without the member's
    // own origin, so a member moved off the origin refuses the hole as not fitting (a
    // delivered-surface limitation recorded in the matrix), and a second hole record on one
    // member is the standing BodyNotPrism refusal the slice-4 delivery records. The stored
    // polygon stays the geometry: d = 20 takes the regular 32-gon of F06 section 2, the
    // same hole_area as the perpendicular hole above, and the volume tolerances below are
    // the certified enclosures' own width bounds.
    let outline = |points: [(f64, f64); 4]| {
        points
            .iter()
            .map(|(x, y)| format!("{x},{y}"))
            .collect::<Vec<_>>()
            .join(";")
    };
    let square = [(-50.0, -50.0), (50.0, -50.0), (50.0, 50.0), (-50.0, 50.0)];
    s.ok(
        "draft",
        "define-profile",
        &[
            "--family",
            "custom",
            "--outer",
            &outline(square),
            "--designation",
            "Q01-D-SQUARE",
        ],
    );
    // The same square rotated by 20 degrees: every wall is one sloped single-strip chain,
    // the merge-walk case of the G14.2 slice 3.
    let (cos20, sin20) = (20.0_f64.to_radians().cos(), 20.0_f64.to_radians().sin());
    let rotated = square.map(|(u, v)| (u * cos20 - v * sin20, u * sin20 + v * cos20));
    s.ok(
        "draft",
        "define-profile",
        &[
            "--family",
            "custom",
            "--outer",
            &outline(rotated),
            "--designation",
            "Q01-D-SLOPED",
        ],
    );
    s.ok(
        "draft",
        "create-member",
        &[
            "--new-id",
            OBLIQUE_MEMBER,
            "--kind",
            "beam",
            "--start",
            "0,0,0",
            "--end",
            "1000,0,0",
            "--profile",
            "Q01-D-SQUARE",
            "--material",
            "Q01-steel",
            "--name",
            "Q01-D oblique hole beam",
        ],
    );
    s.ok(
        "draft",
        "create-member",
        &[
            "--new-id",
            SLOPED_MEMBER,
            "--kind",
            "beam",
            "--start",
            "0,0,0",
            "--end",
            "0,1000,0",
            "--profile",
            "Q01-D-SLOPED",
            "--material",
            "Q01-steel",
            "--name",
            "Q01-D sloped wall beam",
        ],
    );
    let before_d = parts(&s.ok("draft", "inspect-elements", &[]));

    // An oblique, non-unit direction through the flat walls of the square: (12,5,0) names
    // the direction, the walls perpendicular to u are 100 apart and |n.d| = 12/13, so the
    // cut removes the 32-gon area times 100 * 13 / 12.
    let oblique = s.ok(
        "draft",
        "add-hole",
        &[
            "--id",
            OBLIQUE_MEMBER,
            "--operation-id",
            OBLIQUE_HOLE_OP,
            "--axis-point",
            "0,0,500",
            "--axis-direction",
            "12,5,0",
            "--diameter",
            "20",
        ],
    );
    assert_eq!(text(&oblique, "outcome"), "hole_added");
    let oblique_element = at(&oblique, &["element"]);
    let oblique_records = at(oblique_element, &["hole_records"]).array().unwrap();
    assert_eq!(oblique_records.len(), 1);
    assert_eq!(text(&oblique_records[0], "extent"), "through");
    let oblique_volume = at(oblique_element, &["quantities", "volume", "mm3"])
        .float()
        .unwrap();
    let oblique_removed = volume_of(&before_d, OBLIQUE_MEMBER) - oblique_volume;
    assert!(
        (oblique_removed - hole_area * 100.0 * 13.0 / 12.0).abs() <= 1.0,
        "oblique removed {oblique_removed}"
    );
    let oblique_digest = text(oblique_element, "mesh_digest");

    // A sloped wall: the rotated square's walls tilt 20 degrees against the cut, the two
    // crossed walls stay parallel 100 apart, so the removed column is the footprint area
    // times 100 / cos(20 degrees) — the moment certification of the merge walk.
    let sloped = s.ok(
        "draft",
        "add-hole",
        &[
            "--id",
            SLOPED_MEMBER,
            "--operation-id",
            SLOPED_HOLE_OP,
            "--axis-point",
            "0,0,500",
            "--axis-direction",
            "0,1,0",
            "--diameter",
            "20",
        ],
    );
    assert_eq!(text(&sloped, "outcome"), "hole_added");
    let sloped_element = at(&sloped, &["element"]);
    let sloped_records = at(sloped_element, &["hole_records"]).array().unwrap();
    assert_eq!(sloped_records.len(), 1);
    let sloped_volume = at(sloped_element, &["quantities", "volume", "mm3"])
        .float()
        .unwrap();
    let sloped_removed = volume_of(&before_d, SLOPED_MEMBER) - sloped_volume;
    assert!(
        (sloped_removed - hole_area * 100.0 / cos20).abs() <= 1.0,
        "sloped removed {sloped_removed}"
    );
    let sloped_digest = text(sloped_element, "mesh_digest");

    // An axial direction has no section-plane part: refused at admission, nothing written
    // (the sloped member's one record below proves the refusal wrote no second record).
    let axial = s.run(
        "draft",
        "add-hole",
        &[
            "--id",
            SLOPED_MEMBER,
            "--operation-id",
            AXIAL_HOLE_OP,
            "--axis-point",
            "0,0,500",
            "--axis-direction",
            "0,0,1",
            "--diameter",
            "20",
        ],
        2,
    );
    assert_eq!(text(&axial, "code"), "UNSUPPORTED_GEOMETRY");

    // A second record on one member is the standing BodyNotPrism limitation of the G14.2
    // delivery: the first hole's opening ring vertices sit on their wall planes only up to
    // rounding, so the replay refuses (a direction and position that fit a virgin member)
    // and the member keeps its last valid state.
    let second = s.run(
        "draft",
        "add-hole",
        &[
            "--id",
            OBLIQUE_MEMBER,
            "--operation-id",
            SECOND_HOLE_OP,
            "--axis-point",
            "0,30,500",
            "--axis-direction",
            "1,0,0",
            "--diameter",
            "20",
        ],
        2,
    );
    assert_eq!(text(&second, "code"), "UNSUPPORTED_GEOMETRY");

    // The identical oblique command replays the recorded revision, not a second cut.
    let retry = s.ok(
        "draft",
        "add-hole",
        &[
            "--id",
            OBLIQUE_MEMBER,
            "--operation-id",
            OBLIQUE_HOLE_OP,
            "--axis-point",
            "0,0,500",
            "--axis-direction",
            "12,5,0",
            "--diameter",
            "20",
        ],
    );
    assert_eq!(text(&retry, "outcome"), "hole_add_replayed");
    assert_eq!(
        text(at(&retry, &["revision"]), "revision_id"),
        text(at(&oblique, &["revision"]), "revision_id")
    );

    // Fresh processes read both members back with the exact volumes and digests the
    // add-hole replies reported (the refused second record changed neither), and a second
    // read is byte-identical.
    let d_read = parts(&s.ok("draft", "inspect-elements", &[]));
    let d_repeat = parts(&s.ok("draft", "inspect-elements", &[]));
    for (id, volume, digest) in [
        (OBLIQUE_MEMBER, oblique_volume, oblique_digest.as_str()),
        (SLOPED_MEMBER, sloped_volume, sloped_digest.as_str()),
    ] {
        let found = d_read
            .iter()
            .find(|(other, _, _)| other == id)
            .expect("the G14.2 member is present");
        close(found.1, volume);
        assert_eq!(found.2, digest, "{id} mesh must survive the reopen");
        let again = d_repeat
            .iter()
            .find(|(other, _, _)| other == id)
            .expect("the G14.2 member is present");
        assert_eq!(found, again, "{id} must read byte-identically");
    }

    // The faceted-tube hole (Q01 slice D remainder): a CHS member takes a radial
    // through hole whose drop certifies within the engine's own `2 * A * t` bound, and the
    // gross stays the nominal `area x length` across it (the plate rule: a gross that
    // ignores the hole, a net that does not). The old CLI path reported the faceted mesh
    // volume as the net, a constant ~10,980 mm3 past the bound on this very section.
    s.ok(
        "draft",
        "define-profile",
        &[
            "--family",
            "chs",
            "--d",
            "114",
            "--t",
            "5",
            "--designation",
            "Q01-D-CHS",
        ],
    );
    let tube = s.ok(
        "draft",
        "create-member",
        &[
            "--new-id",
            TUBE_MEMBER,
            "--kind",
            "beam",
            "--start",
            "0,0,0",
            "--end",
            "1000,0,0",
            "--profile",
            "Q01-D-CHS",
            "--material",
            "Q01-steel",
            "--name",
            "Q01-D tube hole beam",
        ],
    );
    let tube_gross = at(&tube, &["member", "quantities", "volume", "mm3"])
        .float()
        .unwrap();
    let tube_holed = s.ok(
        "draft",
        "add-hole",
        &[
            "--id",
            TUBE_MEMBER,
            "--operation-id",
            TUBE_HOLE_OP,
            "--axis-point",
            "0,0,500",
            "--axis-direction",
            "1,0,0",
            "--diameter",
            "20",
        ],
    );
    assert_eq!(text(&tube_holed, "outcome"), "hole_added");
    let tube_element = at(&tube_holed, &["element"]);
    let tube_net = at(tube_element, &["quantities", "volume", "mm3"])
        .float()
        .unwrap();
    // The analytic bound of the smooth tube: the d = 20 hole circle through both 5 mm walls.
    let tube_bound = 2.0 * std::f64::consts::PI * 100.0 * 5.0;
    let tube_drop = tube_gross - tube_net;
    assert!(
        tube_drop > 0.0 && tube_drop <= tube_bound,
        "tube drop {tube_drop} past bound {tube_bound}"
    );
    close(
        at(tube_element, &["quantities", "volume_gross_mm3"])
            .float()
            .unwrap(),
        tube_gross,
    );
    let tube_digest = text(tube_element, "mesh_digest");
    // The tube hole survives the reopen with the same figures (the fresh-reader check).
    let tube_read = parts(&s.ok("draft", "inspect-elements", &[]));
    let tube_found = tube_read
        .iter()
        .find(|(other, _, _)| other == TUBE_MEMBER)
        .expect("the tube member is present");
    close(tube_found.1, tube_net);
    assert_eq!(
        tube_found.2, tube_digest,
        "tube mesh must survive the reopen"
    );
}

/// The curved and poly story (P12/P13): its own published store, so the numbered frame
/// above keeps the exact part counts the manual runbook records, and its own test, so
/// the documented entry point still runs both and the per-test budget stays honest.
#[test]
fn curved_and_poly_members_run_their_own_published_store() {
    let s = Story::new();
    // The store numbers a poly member, but refuses once the curved member exists
    // (`UNSUPPORTED_GEOMETRY`, a delivered-surface limit), so the curved story cannot
    // join the numbered frame yet. The mitred polybeam quantity drops the corners out
    // exactly; the arc quantity is the exact analytic sweep, only the mesh facets.
    s.ok(
        "curved-published",
        "define-profile",
        &[
            "--designation",
            "Q01-ARC",
            "--width",
            "100",
            "--height",
            "100",
        ],
    );
    s.ok(
        "curved-published",
        "define-material",
        &["--grade", "Q01-steel", "--density", "7850"],
    );
    s.ok(
        "curved-published",
        "create-workspace",
        &[
            "--workspace-dir",
            &s.path("curved-draft"),
            "--user-id",
            USER,
        ],
    );
    let poly = s.ok(
        "curved-draft",
        "create-polybeam",
        &[
            "--points",
            "0,0,0;2000,0,0;2000,3000,0",
            "--profile",
            "Q01-ARC",
            "--material",
            "Q01-steel",
            "--name",
            "Q01 poly gutter",
        ],
    );
    assert_eq!(text(&poly, "outcome"), "polybeam_created");
    let poly_member = at(&poly, &["member"]);
    assert_eq!(text(poly_member, "kind"), "beam");
    assert_eq!(
        at(poly_member, &["section_mm", "width"]).float().unwrap(),
        100.0
    );
    assert_eq!(
        at(poly_member, &["section_mm", "height"]).float().unwrap(),
        100.0
    );
    // The mitres drop out of the quantity: area x polyline length, exactly.
    let poly_volume = at(poly_member, &["quantities", "volume", "mm3"])
        .float()
        .unwrap();
    close(poly_volume, 50_000_000.0);
    close(
        at(poly_member, &["quantities", "mass", "kg"])
            .float()
            .unwrap(),
        392.5,
    );
    assert_eq!(
        at(poly_member, &["axis_points_mm"]).array().unwrap().len(),
        3
    );
    let poly_id = text(poly_member, "element_id");
    let poly_digest = text(poly_member, "mesh_digest");
    s.ok("curved-draft", "preview-numbering", &[]);

    // The radius-mode semicircle: the axis length and the quantity are the exact arc
    // values (area x arc length); only the stored mesh facets the sweep.
    let arc = s.ok(
        "curved-draft",
        "create-curved-beam",
        &[
            "--start",
            "1000,0,0",
            "--end",
            "-1000,0,0",
            "--radius",
            "1000",
            "--normal",
            "0,0,1",
            "--profile",
            "Q01-ARC",
            "--material",
            "Q01-steel",
            "--name",
            "Q01 arc hanger",
        ],
    );
    let arc_member = at(&arc, &["member"]);
    assert_eq!(text(arc_member, "path_type"), "arc");
    let arc_length = 1000.0 * std::f64::consts::PI;
    close(
        at(arc_member, &["axis_length_mm"]).float().unwrap(),
        arc_length,
    );
    assert!(at(arc_member, &["arc_deviation_mm"]).float().unwrap() <= 0.05);
    let arc_volume = at(arc_member, &["quantities", "volume", "mm3"])
        .float()
        .unwrap();
    close(arc_volume, 10_000.0 * arc_length);
    let arc_id = text(arc_member, "element_id");
    let arc_digest = text(arc_member, "mesh_digest");

    let curved_head = s.head("curved-draft");
    let degenerate = s.run(
        "curved-draft",
        "create-polybeam",
        &["--points", "0,0,0;2000,0,0", "--profile", "Q01-ARC"],
        2,
    );
    assert_eq!(text(&degenerate, "outcome"), "rejected");
    let unnumbered = s.run("curved-draft", "preview-numbering", &[], 2);
    assert_eq!(text(&unnumbered, "code"), "UNSUPPORTED_GEOMETRY");
    assert_eq!(s.head("curved-draft"), curved_head);

    // The arc creation undoes and redoes as one command.
    let undo = s.execute(&["undo-workspace", "--workspace-dir", &s.path("curved-draft")]);
    assert_eq!(undo.0, 0);
    assert_eq!(text(&undo.1, "outcome"), "workspace_undone");
    assert_eq!(
        parts(&s.ok("curved-draft", "inspect-elements", &[])).len(),
        1
    );
    let redo = s.execute(&["redo-workspace", "--workspace-dir", &s.path("curved-draft")]);
    assert_eq!(redo.0, 0);
    assert_eq!(text(&redo.1, "outcome"), "workspace_redone");

    let curved_published = s.ok(
        "curved-published",
        "publish-workspace",
        &["--workspace-dir", &s.path("curved-draft")],
    );
    assert_eq!(text(&curved_published, "outcome"), "workspace_published");
    let reopened_curved = parts(&s.ok("curved-published", "inspect-elements", &[]));
    assert_eq!(reopened_curved.len(), 2);
    for (id, volume, digest, label) in [
        (poly_id.as_str(), poly_volume, poly_digest.as_str(), "poly"),
        (arc_id.as_str(), arc_volume, arc_digest.as_str(), "arc"),
    ] {
        let found = reopened_curved
            .iter()
            .find(|(other, _, _)| other == id)
            .unwrap_or_else(|| panic!("{label} member is present"));
        close(found.1, volume);
        assert_eq!(found.2, digest, "{label} mesh must survive the round trip");
    }
}

/// The bent and lofted plate story (A10/A11): its own published store, so the numbered
/// frame keeps the counts the manual runbook records, and its own test for the per-test
/// budget. The lofted plate is the certified ruled loft with the exact analytic volume;
/// the fold keeps the blank's `t x area` volume within the certified bound and the
/// unfold stays the authored contour.
#[test]
fn bent_and_lofted_plates_run_their_own_published_store() {
    let s = Story::new();
    // The project's steel seeds the store head; the workspace forks from it.
    s.ok(
        "plates-published",
        "define-material",
        &["--grade", "Q01-steel", "--density", "7850"],
    );
    s.ok(
        "plates-published",
        "create-workspace",
        &[
            "--workspace-dir",
            &s.path("plates-draft"),
            "--user-id",
            USER,
        ],
    );

    // The lofted plate between two translated contours: the oblique prism of the square,
    // with the exact analytic volume in the reply and in the stored quantities.
    let lofted = s.ok(
        "plates-draft",
        "create-lofted-plate",
        &[
            "--name",
            "Q01 lofted transition",
            "--new-id",
            "019a7bf0-0000-7000-8000-00000000aa90",
            "--start",
            "0,0,0;200,0,0;200,100,0;0,100,0",
            "--end",
            "0,0,50;200,0,50;200,100,50;0,100,50",
        ],
    );
    assert_eq!(text(&lofted, "outcome"), "lofted_plate_created");
    let lofted_id = text(at(&lofted, &["plate"]), "id");
    assert!(
        (at(&lofted, &["geometry", "volume_mm3"]).float().unwrap() - 1_000_000.0).abs() < 1e-6,
        "the lofted oblique prism volume"
    );
    let stored_lofts = s.ok("plates-draft", "inspect-lofted-plates", &[]);
    assert_eq!(text(&stored_lofts, "outcome"), "lofted_plates");
    let lofts = at(&stored_lofts, &["plates"]).array().unwrap();
    assert_eq!(lofts.len(), 1);
    assert_eq!(text(&lofts[0], "id"), lofted_id);
    assert_eq!(text(&lofts[0], "volume_status"), "current");
    assert!(
        (at(&lofts[0], &["volume_mm3"]).float().unwrap() - 1_000_000.0).abs() < 1e-6,
        "the stored lofted quantity"
    );

    // The folded plate: one right-angle fold on radius 8 keeps the volume at the blank's
    // t x area within the certified bound, the outline becomes the unfold contour, and
    // the bend record carries its parameters.
    let flat = s.ok(
        "plates-draft",
        "create-plate",
        &[
            "--width",
            "1000",
            "--length",
            "2000",
            "--thickness",
            "2",
            "--name",
            "Q01 folded bracket",
        ],
    );
    let flat_id = text(at(&flat, &["plate"]), "element_id");
    let fold_args = [
        "--id",
        flat_id.as_str(),
        "--operation-id",
        "019a7bf0-0000-7000-8000-00000000aa91",
        "--op-id",
        "019a7bf0-0000-7000-8000-00000000aa92",
        "--kind",
        "fold",
        "--point-a",
        "0,1000,0",
        "--point-b",
        "1000,1000,0",
        "--radius",
        "8",
        "--angle-rad",
        "1.5707963267948966",
    ];
    let bent = s.ok("plates-draft", "bend-plate", &fold_args);
    assert_eq!(text(&bent, "outcome"), "bend_applied");
    let inspection = s.ok("plates-draft", "inspect-elements", &[]);
    let bent_parts = parts(&inspection);
    assert_eq!(
        bent_parts.len(),
        1,
        "the loft lists separately from the plates"
    );
    let bent_found = bent_parts
        .iter()
        .find(|(element, _, _)| element == flat_id.as_str())
        .expect("the folded plate is present");
    let bent_volume = bent_found.1;
    assert!(
        (bent_volume - 4_000_000.0).abs() <= 8_000.0,
        "bent volume {bent_volume} past the fold bound"
    );
    let bent_digest = bent_found.2.clone();
    let folded_entry = at(&inspection, &["plates"])
        .array()
        .unwrap()
        .iter()
        .find(|p| text(p, "element_id") == flat_id)
        .expect("the folded plate entry is present");
    assert_eq!(text(folded_entry, "outline"), "contour");
    let bends = at(folded_entry, &["bends"]).array().unwrap();
    assert_eq!(bends.len(), 1);
    assert_eq!(text(&bends[0], "bend"), "fold");

    let plates_head = s.head("plates-draft");
    let replay = s.ok("plates-draft", "bend-plate", &fold_args);
    assert_eq!(text(&replay, "outcome"), "bend_replayed");
    assert_eq!(
        s.head("plates-draft"),
        plates_head,
        "the replay writes nothing"
    );
    let overlapping = s.run(
        "plates-draft",
        "bend-plate",
        &[
            "--id",
            flat_id.as_str(),
            "--operation-id",
            "019a7bf0-0000-7000-8000-00000000aa93",
            "--kind",
            "fold",
            "--point-a",
            "0,1001,0",
            "--point-b",
            "1000,1001,0",
            "--radius",
            "8",
            "--angle-rad",
            "1.5707963267948966",
        ],
        2,
    );
    assert_eq!(text(&overlapping, "outcome"), "rejected");
    assert_eq!(s.head("plates-draft"), plates_head);

    // The bend is one undo unit: the fold rolls back to the exact flat blank and the
    // redo restores the bent volume.
    let undo = s.execute(&["undo-workspace", "--workspace-dir", &s.path("plates-draft")]);
    assert_eq!(undo.0, 0);
    assert_eq!(text(&undo.1, "outcome"), "workspace_undone");
    let unfolded = parts(&s.ok("plates-draft", "inspect-elements", &[]));
    close(volume_of(&unfolded, flat_id.as_str()), 4_000_000.0);
    let redo = s.execute(&["redo-workspace", "--workspace-dir", &s.path("plates-draft")]);
    assert_eq!(redo.0, 0);
    assert_eq!(text(&redo.1, "outcome"), "workspace_redone");

    let plates_published = s.ok(
        "plates-published",
        "publish-workspace",
        &["--workspace-dir", &s.path("plates-draft")],
    );
    assert_eq!(text(&plates_published, "outcome"), "workspace_published");
    let reopened_plates = parts(&s.ok("plates-published", "inspect-elements", &[]));
    assert_eq!(reopened_plates.len(), 1);
    let folded_back = reopened_plates
        .iter()
        .find(|(other, _, _)| other == flat_id.as_str())
        .expect("the folded plate is present");
    close(folded_back.1, bent_volume);
    assert_eq!(
        folded_back.2, bent_digest,
        "folded mesh must survive the round trip"
    );
    let reopened_lofts = s.ok("plates-published", "inspect-lofted-plates", &[]);
    assert_eq!(text(&reopened_lofts, "outcome"), "lofted_plates");
    let reopened_loft_entries = at(&reopened_lofts, &["plates"]).array().unwrap();
    assert_eq!(reopened_loft_entries.len(), 1);
    assert_eq!(text(&reopened_loft_entries[0], "id"), lofted_id);
    assert_eq!(text(&reopened_loft_entries[0], "volume_status"), "current");
    assert!(
        (at(&reopened_loft_entries[0], &["volume_mm3"])
            .float()
            .unwrap()
            - 1_000_000.0)
            .abs()
            < 1e-6,
        "the lofted quantity must survive the round trip"
    );
    assert!(
        at(&reopened_loft_entries[0], &["triangles"])
            .unsigned()
            .unwrap()
            > 0
    );
}

/// The model-tool story (T03): rotate, mirror and the arrays run in their own published
/// store, so the numbered frame keeps the counts the manual runbook records, and in its
/// own test for the per-test budget. Every transform is exactly one placement or shape
/// step and one undo unit; the arrays land independent copies in one revision.
#[test]
fn tools_rotate_mirror_and_array_exactly() {
    let s = Story::new();
    s.ok(
        "tools-published",
        "define-profile",
        &[
            "--designation",
            "Q01-BAR",
            "--width",
            "100",
            "--height",
            "100",
        ],
    );
    s.ok(
        "tools-published",
        "create-workspace",
        &["--workspace-dir", &s.path("tools-draft"), "--user-id", USER],
    );
    let tool_volume = 10_000_000.0;
    let strut = |id: &str, start: &str, end: &str, name: &str| -> String {
        let created = s.ok(
            "tools-draft",
            "create-member",
            &[
                "--new-id",
                id,
                "--kind",
                "beam",
                "--start",
                start,
                "--end",
                end,
                "--profile",
                "Q01-BAR",
                "--name",
                name,
            ],
        );
        text(at(&created, &["member"]), "element_id")
    };

    // The linear array lands three independent copies in one revision: stepped
    // placements, the source body and quantity each, and one undo removes them all.
    let arrayed = strut(
        "019a7bf0-0000-7000-8000-00000000aaa0",
        "0,0,0",
        "1000,0,0",
        "Q01 array strut",
    );
    let array = s.ok(
        "tools-draft",
        "array-linear",
        &[
            "--id",
            &arrayed,
            "--count",
            "3",
            "--direction",
            "1,0,0",
            "--spacing",
            "2000",
        ],
    );
    assert_eq!(text(&array, "outcome"), "element_arrayed");
    for (index, element) in at(&array, &["elements"])
        .array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let expected = 2000.0 * (index as f64 + 1.0);
        assert_eq!(
            numbers(at(element, &["placement", "translation_mm"]))[0],
            expected,
            "the copy at the stepped placement"
        );
        assert_eq!(
            at(element, &["quantities", "volume", "mm3"])
                .float()
                .unwrap(),
            tool_volume
        );
    }
    assert_eq!(
        parts(&s.ok("tools-draft", "inspect-elements", &[])).len(),
        4
    );
    let undo = s.execute(&["undo-workspace", "--workspace-dir", &s.path("tools-draft")]);
    assert_eq!(undo.0, 0);
    assert_eq!(text(&undo.1, "outcome"), "workspace_undone");
    assert_eq!(
        parts(&s.ok("tools-draft", "inspect-elements", &[])).len(),
        1,
        "one undo removes every copy"
    );
    let redo = s.execute(&["redo-workspace", "--workspace-dir", &s.path("tools-draft")]);
    assert_eq!(redo.0, 0);
    assert_eq!(text(&redo.1, "outcome"), "workspace_redone");

    // The mirror reflects the path across the plane, keeping the length and the volume;
    // a mirrored polybeam path is not expressible and refuses with nothing written.
    let mirrored_strut = strut(
        "019a7bf0-0000-7000-8000-00000000aaa1",
        "2000,0,0",
        "3000,0,0",
        "Q01 mirrored strut",
    );
    let mirror = s.ok(
        "tools-draft",
        "update-element",
        &[
            "--id",
            &mirrored_strut,
            "--mirror-point",
            "0,0,0",
            "--mirror-normal",
            "1,0,0",
        ],
    );
    assert_eq!(text(&mirror, "outcome"), "element_updated");
    let mirror_element = at(&mirror, &["element"]);
    let mirrored_points = at(mirror_element, &["axis_points_mm"]).array().unwrap();
    assert_eq!(mirrored_points.len(), 2);
    assert_eq!(
        numbers(&mirrored_points[0]),
        [-2000.0, 0.0, 0.0],
        "the path reflects across x = 0"
    );
    assert_eq!(
        at(mirror_element, &["axis_length_mm"]).float().unwrap(),
        1000.0
    );
    let poly = s.ok(
        "tools-draft",
        "create-polybeam",
        &[
            "--points",
            "0,0,0;1000,0,0;1000,1000,0",
            "--profile",
            "Q01-BAR",
            "--name",
            "Q01 tool gutter",
        ],
    );
    let poly_id = text(at(&poly, &["member"]), "element_id");
    let tools_head = s.head("tools-draft");
    let refused_mirror = s.run(
        "tools-draft",
        "update-element",
        &[
            "--id",
            &poly_id,
            "--mirror-point",
            "0,0,0",
            "--mirror-normal",
            "1,0,0",
        ],
        2,
    );
    assert_eq!(text(&refused_mirror, "code"), "UNSUPPORTED_GEOMETRY");
    assert_eq!(s.head("tools-draft"), tools_head);

    // The rotation is one placement step: the quarter turn about Z moves the mirrored
    // placement onto -Y and composes the orientation; undo and redo each restore one step.
    let rotation = s.ok(
        "tools-draft",
        "update-element",
        &[
            "--id",
            &mirrored_strut,
            "--rotate-about",
            "0,0,0",
            "--rotate-axis",
            "0,0,1",
            "--rotate-angle",
            "1.5707963267948966",
        ],
    );
    assert_eq!(text(&rotation, "outcome"), "element_updated");
    let rotated = at(&rotation, &["element"]);
    // The turn composes the orientation and keeps the authored path: the member turns in
    // place.
    let turned = numbers(at(rotated, &["placement", "orientation_xyzw"]));
    assert!((turned[2] - 0.5_f64.sqrt()).abs() < 1e-6, "{turned:?}");
    let rotated_path = at(rotated, &["axis_points_mm"]).array().unwrap();
    assert_eq!(
        numbers(&rotated_path[0]),
        [-2000.0, 0.0, 0.0],
        "the rotation keeps the authored path"
    );
    let rotate_undo = s.execute(&["undo-workspace", "--workspace-dir", &s.path("tools-draft")]);
    assert_eq!(rotate_undo.0, 0);
    let undone_inspection = s.ok("tools-draft", "inspect-elements", &[]);
    assert_eq!(parts(&undone_inspection).len(), 6);
    let restored_entry = at(&undone_inspection, &["members"])
        .array()
        .unwrap()
        .iter()
        .find(|e| text(e, "element_id") == mirrored_strut)
        .expect("the mirrored strut entry is present");
    let upright = numbers(at(restored_entry, &["placement", "orientation_xyzw"]));
    assert!(
        (upright[2]).abs() < 1e-9 && (upright[3] - 1.0).abs() < 1e-9,
        "{upright:?}"
    );
    let rotate_redo = s.execute(&["redo-workspace", "--workspace-dir", &s.path("tools-draft")]);
    assert_eq!(rotate_redo.0, 0);
    assert_eq!(text(&rotate_redo.1, "outcome"), "workspace_redone");
    let final_inspection = s.ok("tools-draft", "inspect-elements", &[]);
    let final_parts = parts(&final_inspection);
    assert_eq!(final_parts.len(), 6);
    let rotated_again = at(&final_inspection, &["members"])
        .array()
        .unwrap()
        .iter()
        .find(|e| text(e, "element_id") == mirrored_strut)
        .expect("the mirrored strut entry is present");
    let turned_again = numbers(at(rotated_again, &["placement", "orientation_xyzw"]));
    assert!(
        (turned_again[2] - 0.5_f64.sqrt()).abs() < 1e-6,
        "{turned_again:?}"
    );

    let tools_published = s.ok(
        "tools-published",
        "publish-workspace",
        &["--workspace-dir", &s.path("tools-draft")],
    );
    assert_eq!(text(&tools_published, "outcome"), "workspace_published");
    let reopened_tools = parts(&s.ok("tools-published", "inspect-elements", &[]));
    assert_eq!(reopened_tools.len(), final_parts.len());
    for (id, volume, digest) in &final_parts {
        let found = reopened_tools
            .iter()
            .find(|(other, _, _)| other == id)
            .unwrap_or_else(|| panic!("{id} is present"));
        close(found.1, *volume);
        assert_eq!(found.2, *digest, "{id} mesh must survive the round trip");
    }
    let published_inspection = s.ok("tools-published", "inspect-elements", &[]);
    let published_entry = at(&published_inspection, &["members"])
        .array()
        .unwrap()
        .iter()
        .find(|e| text(e, "element_id") == mirrored_strut)
        .expect("the mirrored strut entry is present");
    let published_turn = numbers(at(published_entry, &["placement", "orientation_xyzw"]));
    assert!(
        (published_turn[2] - 0.5_f64.sqrt()).abs() < 1e-6,
        "the transformed orientation survives publication: {published_turn:?}"
    );
}

/// The bolt-group story (A14.3/A14.4): its own published store, so the numbered frame
/// keeps the counts the manual runbook records, and its own test for the per-test budget.
/// A group commits one atomic edit, generates one group-owned hole record per crossing
/// fastener on every connected plate, and undoes as one unit.
#[test]
fn a_bolt_group_generates_its_holes_and_survives_publication() {
    let s = Story::new();
    s.ok(
        "bolts-published",
        "define-material",
        &["--grade", "Q01-steel", "--density", "7850"],
    );
    s.ok(
        "bolts-published",
        "create-workspace",
        &["--workspace-dir", &s.path("bolts-draft"), "--user-id", USER],
    );
    let spliced = |id: &str, z: &str, name: &str| -> String {
        let created = s.ok(
            "bolts-draft",
            "create-plate",
            &[
                "--name",
                name,
                "--new-id",
                id,
                "--width",
                "1000",
                "--length",
                "500",
                "--thickness",
                "10",
                "--translation",
                z,
            ],
        );
        text(at(&created, &["plate"]), "element_id")
    };
    let low = spliced(
        "019a7bf0-0000-7000-8000-00000000bbb1",
        "0,0,0",
        "Q01 splice low",
    );
    let high = spliced(
        "019a7bf0-0000-7000-8000-00000000bbb2",
        "0,0,10",
        "Q01 splice high",
    );
    let parts_arg = format!("{low},{high}");
    let group = s.ok(
        "bolts-draft",
        "create-bolt-group",
        &[
            "--name",
            "Q01 splice group",
            "--new-id",
            "019a7bf0-0000-7000-8000-00000000bbb3",
            "--parts",
            parts_arg.as_str(),
            "--fastener-library",
            SYNTHETIC_FASTENERS,
            "--designation",
            "SYN-BOLT-12x50",
            "--point1",
            "0,250,0",
            "--point2",
            "600,250,0",
            "--plane-normal",
            "0,0,1",
            "--offset-start",
            "100",
            "--spacings",
            "150",
        ],
    );
    assert_eq!(text(&group, "outcome"), "bolt_group_committed");
    let group_id = text(&group, "element_id");
    // The record: the auto length resolves over the grip and its hardware; the group
    // generates its holes.
    let bolts_book = s.ok("bolts-draft", "bolts", &[]);
    let record = &at(&bolts_book, &["bolt_groups"]).array().unwrap()[0];
    assert_eq!(text(record, "id"), group_id);
    assert_eq!(text(record, "hole_mode"), "generate");
    assert_eq!(text(record, "length_mode"), "auto");
    assert_eq!(text(record, "length_mm"), "40");
    assert_eq!(text(record, "count"), "2");
    let bolted = parts(&s.ok("bolts-draft", "inspect-elements", &[]));
    assert_eq!(bolted.len(), 2);
    for id in [&low, &high] {
        let found = bolted
            .iter()
            .find(|(element, _, _)| element == id)
            .expect("the spliced plate is present");
        assert!(
            (5_000_000.0 - found.1 - BOLT_DROP_MM3).abs() <= 1.0,
            "the plate volume {} misses the two generated holes",
            found.1
        );
    }
    let bolted_inspection = s.ok("bolts-draft", "inspect-elements", &[]);
    for entry in at(&bolted_inspection, &["plates"]).array().unwrap().iter() {
        let records = at(entry, &["hole_records"]).array().unwrap();
        assert_eq!(records.len(), 2, "{}", text(entry, "element_id"));
        for record in records {
            assert_eq!(text(record, "generated_by"), group_id);
        }
    }

    // One atomic edit and one undo unit: the undo returns the blank plates, the redo
    // regenerates the same drops.
    let undo = s.execute(&["undo-workspace", "--workspace-dir", &s.path("bolts-draft")]);
    assert_eq!(undo.0, 0);
    assert_eq!(text(&undo.1, "outcome"), "workspace_undone");
    for found in parts(&s.ok("bolts-draft", "inspect-elements", &[])) {
        close(found.1, 5_000_000.0);
    }
    let redo = s.execute(&["redo-workspace", "--workspace-dir", &s.path("bolts-draft")]);
    assert_eq!(redo.0, 0);
    assert_eq!(text(&redo.1, "outcome"), "workspace_redone");

    let bolts_published = s.ok(
        "bolts-published",
        "publish-workspace",
        &["--workspace-dir", &s.path("bolts-draft")],
    );
    assert_eq!(text(&bolts_published, "outcome"), "workspace_published");
    let reopened = parts(&s.ok("bolts-published", "inspect-elements", &[]));
    assert_eq!(reopened.len(), 2);
    for id in [&low, &high] {
        let found = reopened
            .iter()
            .find(|(element, _, _)| element == id)
            .expect("the spliced plate is present");
        assert!(
            (5_000_000.0 - found.1 - BOLT_DROP_MM3).abs() <= 1.0,
            "the plate volume {} misses the generated holes after publication",
            found.1
        );
    }
    let reopened_inspection = s.ok("bolts-published", "inspect-elements", &[]);
    for entry in at(&reopened_inspection, &["plates"])
        .array()
        .unwrap()
        .iter()
    {
        let records = at(entry, &["hole_records"]).array().unwrap();
        assert_eq!(records.len(), 2, "{}", text(entry, "element_id"));
        for record in records {
            assert_eq!(
                text(record, "generated_by"),
                group_id,
                "the generated holes keep their owner across publication"
            );
        }
    }
    let published_book = s.ok("bolts-published", "bolts", &[]);
    let published_record = &at(&published_book, &["bolt_groups"]).array().unwrap()[0];
    assert_eq!(text(published_record, "hole_mode"), "generate");
    assert_eq!(text(published_record, "count"), "2");
    assert_eq!(text(published_record, "length_mm"), "40");
}

/// The model-data tool story (T01/T02): construction objects and the drawn grid line
/// tools run in their own published store, so the numbered frame keeps the counts the
/// manual runbook records, and in its own test for the per-test budget. The grid line
/// edits keep the axis identity through reshape, relabel and remove, and the construction
/// helpers reopen exactly as they were stored.
#[test]
fn model_data_construction_and_grid_line_tools() {
    let s = Story::new();
    s.ok(
        "data-published",
        "define-material",
        &["--grade", "Q01-steel", "--density", "7850"],
    );
    s.ok(
        "data-published",
        "create-workspace",
        &["--workspace-dir", &s.path("data-draft"), "--user-id", USER],
    );

    // The drawn grid starts with one U axis; the V axis is drawn, reshaped, relabelled
    // and removed, each edit keeping the identity it was given.
    s.ok(
        "data-draft",
        "create-grid",
        &[
            "--name",
            "Q01 tool grid",
            "--new-id",
            "019a7bf0-0000-7000-8000-00000000ccc1",
            "--u-from",
            "1000,0",
            "--u-to",
            "1000,8000",
            "--u-label",
            "1",
            "--u-axis-id",
            "019a7bf0-0000-7000-8000-00000000ccc2",
        ],
    );
    s.ok(
        "data-draft",
        "draw-grid-line",
        &[
            "--id",
            "019a7bf0-0000-7000-8000-00000000ccc1",
            "--family",
            "v",
            "--new-axis-id",
            "019a7bf0-0000-7000-8000-00000000ccc3",
            "--label",
            "A",
            "--from",
            "0,5000",
            "--to",
            "9000,5000",
        ],
    );
    s.ok(
        "data-draft",
        "reshape-grid-line",
        &[
            "--id",
            "019a7bf0-0000-7000-8000-00000000ccc1",
            "--axis-id",
            "019a7bf0-0000-7000-8000-00000000ccc3",
            "--from",
            "0,6000",
            "--to",
            "12000,6000",
        ],
    );
    s.ok(
        "data-draft",
        "relabel-grid-line",
        &[
            "--id",
            "019a7bf0-0000-7000-8000-00000000ccc1",
            "--axis-id",
            "019a7bf0-0000-7000-8000-00000000ccc3",
            "--label",
            "B",
        ],
    );
    let shaped = s.ok("data-draft", "list-grids", &[]);
    let shaped_grid = at(&shaped, &["grids"]).array().unwrap();
    assert_eq!(shaped_grid.len(), 1);
    assert_eq!(
        text(&shaped_grid[0], "id"),
        "019a7bf0-0000-7000-8000-00000000ccc1"
    );
    let shaped_v = at(&shaped_grid[0], &["v", "lines"]).array().unwrap();
    assert_eq!(shaped_v.len(), 1);
    assert_eq!(
        text(&shaped_v[0], "axis_id"),
        "019a7bf0-0000-7000-8000-00000000ccc3"
    );
    assert_eq!(text(&shaped_v[0], "label"), "B");
    assert_eq!(numbers(at(&shaped_v[0], &["start_mm"])), [0.0, 6000.0, 0.0]);
    assert_eq!(
        numbers(at(&shaped_v[0], &["end_mm"])),
        [12000.0, 6000.0, 0.0]
    );
    s.ok(
        "data-draft",
        "remove-grid-line",
        &[
            "--id",
            "019a7bf0-0000-7000-8000-00000000ccc1",
            "--axis-id",
            "019a7bf0-0000-7000-8000-00000000ccc3",
        ],
    );
    let removed = s.ok("data-draft", "list-grids", &[]);
    let removed_v = at(
        &at(&removed, &["grids"]).array().unwrap()[0],
        &["v", "lines"],
    )
    .array()
    .unwrap();
    assert_eq!(removed_v.len(), 0);
    // One undo brings the drawn axis back with its label and geometry.
    let undo = s.execute(&["undo-workspace", "--workspace-dir", &s.path("data-draft")]);
    assert_eq!(undo.0, 0);
    assert_eq!(text(&undo.1, "outcome"), "workspace_undone");
    let restored = s.ok("data-draft", "list-grids", &[]);
    let restored_v = at(
        &at(&restored, &["grids"]).array().unwrap()[0],
        &["v", "lines"],
    )
    .array()
    .unwrap();
    assert_eq!(restored_v.len(), 1);
    assert_eq!(text(&restored_v[0], "label"), "B");
    assert_eq!(
        numbers(at(&restored_v[0], &["start_mm"])),
        [0.0, 6000.0, 0.0]
    );

    // The construction helpers are stored elements of every kind; the listing reports
    // them exactly.
    for (kind, extra) in [
        ("point", vec![]),
        ("line", vec!["--b", "10000,2000,0"]),
        ("circle", vec!["--b", "0,0,1", "--radius", "500"]),
        ("plane", vec!["--b", "0,0,1"]),
    ] {
        let mut args = vec![
            "--name",
            "Q01 helper",
            "--kind",
            kind,
            "--a",
            "5000,0,0",
            "--new-id",
            "",
        ];
        args.extend(extra.iter().copied());
        let id_index = args.iter().position(|a| a.is_empty()).unwrap();
        let id = match kind {
            "point" => "019a7bf0-0000-7000-8000-00000000ccc4",
            "line" => "019a7bf0-0000-7000-8000-00000000ccc5",
            "circle" => "019a7bf0-0000-7000-8000-00000000ccc6",
            _ => "019a7bf0-0000-7000-8000-00000000ccc7",
        };
        args[id_index] = id;
        s.ok("data-draft", "create-construction", &args);
    }
    let helpers = s.ok("data-draft", "list-construction", &[]);
    let objects = at(&helpers, &["objects"]).array().unwrap();
    assert_eq!(objects.len(), 4);
    let kinds: Vec<String> = objects.iter().map(|o| text(o, "kind")).collect();
    assert_eq!(kinds, ["point", "line", "circle", "plane"]);
    assert_eq!(at(&objects[2], &["radius_mm"]).float().unwrap(), 500.0);

    let data_published = s.ok(
        "data-published",
        "publish-workspace",
        &["--workspace-dir", &s.path("data-draft")],
    );
    assert_eq!(text(&data_published, "outcome"), "workspace_published");
    let reopened_grids = s.ok("data-published", "list-grids", &[]);
    let reopened_grid = at(&reopened_grids, &["grids"]).array().unwrap();
    assert_eq!(reopened_grid.len(), 1);
    let reopened_v = at(&reopened_grid[0], &["v", "lines"]).array().unwrap();
    assert_eq!(reopened_v.len(), 1);
    assert_eq!(
        text(&reopened_v[0], "axis_id"),
        "019a7bf0-0000-7000-8000-00000000ccc3"
    );
    assert_eq!(text(&reopened_v[0], "label"), "B");
    assert_eq!(
        numbers(at(&reopened_v[0], &["start_mm"])),
        [0.0, 6000.0, 0.0]
    );
    let reopened_helpers = s.ok("data-published", "list-construction", &[]);
    assert_eq!(
        at(&reopened_helpers, &["objects"]).array().unwrap().len(),
        4,
        "the helpers survive the round trip"
    );
}

/// The contour plate story (A12.3): the vertex, insert and edge edits shape a plate's
/// contour in their own published store, so the numbered frame keeps the counts the
/// manual runbook records, and in its own test for the per-test budget. Every edit is one
/// shape revision with the closed-form polygon volume, and a treated corner rides along.
#[test]
fn contour_plate_edits_shape_exactly_and_survive_publication() {
    let s = Story::new();
    s.ok(
        "contour-published",
        "define-material",
        &["--grade", "Q01-steel", "--density", "7850"],
    );
    s.ok(
        "contour-published",
        "create-workspace",
        &[
            "--workspace-dir",
            &s.path("contour-draft"),
            "--user-id",
            USER,
        ],
    );
    let contoured = s.ok(
        "contour-draft",
        "create-plate",
        &[
            "--name",
            "Q01 contour web",
            "--new-id",
            "019a7bf0-0000-7000-8000-00000000ddd1",
            "--width",
            "1000",
            "--length",
            "1000",
            "--thickness",
            "10",
        ],
    );
    let web = text(at(&contoured, &["plate"]), "element_id");
    close(
        volume_of(
            &parts(&s.ok("contour-draft", "inspect-elements", &[])),
            &web,
        ),
        10_000_000.0,
    );

    // Case 6 of the amendment: a vertex delete gives the half square, the collinear
    // insert keeps the body, and the outward move adds the 1000 x 500 / 2 triangle.
    let deleted = s.ok(
        "contour-draft",
        "delete-plate-vertex",
        &[
            "--id",
            &web,
            "--vertex",
            "0",
            "--op-id",
            "019a7bf0-0000-7000-8000-00000000ddd3",
        ],
    );
    assert_eq!(text(&deleted, "outcome"), "plate_vertex_deleted");
    assert_eq!(at(&deleted, &["effect", "shape"]), &Json::Bool(true));
    s.ok(
        "contour-draft",
        "insert-plate-vertex",
        &["--id", &web, "--edge", "1", "--point", "500,1000"],
    );
    let web_parts = parts(&s.ok("contour-draft", "inspect-elements", &[]));
    close(volume_of(&web_parts, &web), 5_000_000.0);
    let moved = s.ok(
        "contour-draft",
        "move-plate-vertex",
        &["--id", &web, "--vertex", "2", "--to", "500,1500"],
    );
    assert_eq!(text(&moved, "outcome"), "plate_vertex_moved");
    let web_parts = parts(&s.ok("contour-draft", "inspect-elements", &[]));
    close(volume_of(&web_parts, &web), 7_500_000.0);
    let web_digest = web_parts
        .iter()
        .find(|(element, _, _)| element == web.as_str())
        .expect("the contour web is present")
        .2
        .clone();

    // The edge move changes exactly its two end vertices and the treated corner rides
    // along: the 200 mm growth adds 2,000,000 mm3 to the chamfered blank.
    let chamfered = s.ok(
        "contour-draft",
        "create-plate",
        &[
            "--name",
            "Q01 chamfered flange",
            "--new-id",
            "019a7bf0-0000-7000-8000-00000000ddd2",
            "--width",
            "1000",
            "--length",
            "1000",
            "--thickness",
            "10",
            "--corners",
            "2=chamfer:100",
        ],
    );
    let flange = text(at(&chamfered, &["plate"]), "element_id");
    let flange_parts = parts(&s.ok("contour-draft", "inspect-elements", &[]));
    close(volume_of(&flange_parts, &flange), 9_950_000.0);
    s.ok(
        "contour-draft",
        "move-plate-edge",
        &["--id", &flange, "--edge", "3", "--delta", "-200,0"],
    );
    let edge_inspection = s.ok("contour-draft", "inspect-elements", &[]);
    let edge_parts = parts(&edge_inspection);
    close(volume_of(&edge_parts, &flange), 11_950_000.0);
    let flange_entry = at(&edge_inspection, &["plates"])
        .array()
        .unwrap()
        .iter()
        .find(|p| text(p, "element_id") == flange)
        .expect("the chamfered plate entry is present");
    let corners = at(flange_entry, &["corners"]).array().unwrap();
    assert_eq!(corners.len(), 1);
    assert_eq!(at(&corners[0], &["vertex"]).float().unwrap(), 2.0);

    // The edge move is the last shape revision: one undo rolls it back to the chamfered
    // blank while the pentagon keeps its shape, and the redo restores the growth.
    let contour_undo = s.execute(&[
        "undo-workspace",
        "--workspace-dir",
        &s.path("contour-draft"),
    ]);
    assert_eq!(contour_undo.0, 0);
    let undone = parts(&s.ok("contour-draft", "inspect-elements", &[]));
    close(volume_of(&undone, &web), 7_500_000.0);
    close(volume_of(&undone, &flange), 9_950_000.0);
    let contour_redo = s.execute(&[
        "redo-workspace",
        "--workspace-dir",
        &s.path("contour-draft"),
    ]);
    assert_eq!(contour_redo.0, 0);
    assert_eq!(text(&contour_redo.1, "outcome"), "workspace_redone");

    let contour_published = s.ok(
        "contour-published",
        "publish-workspace",
        &["--workspace-dir", &s.path("contour-draft")],
    );
    assert_eq!(text(&contour_published, "outcome"), "workspace_published");
    let reopened = parts(&s.ok("contour-published", "inspect-elements", &[]));
    assert_eq!(reopened.len(), 2);
    close(volume_of(&reopened, &web), 7_500_000.0);
    close(volume_of(&reopened, &flange), 11_950_000.0);
    let reopened_web = reopened
        .iter()
        .find(|(element, _, _)| element == web.as_str())
        .expect("the contour web is present");
    assert_eq!(
        reopened_web.2, web_digest,
        "the web mesh must survive the round trip"
    );
    let reopened_flange = reopened
        .iter()
        .find(|(element, _, _)| element == flange.as_str())
        .expect("the chamfered flange is present");
    assert_eq!(
        reopened_flange.2,
        edge_parts
            .iter()
            .find(|(element, _, _)| element == flange.as_str())
            .expect("the chamfered flange is present")
            .2,
        "the flange mesh must survive the round trip"
    );
}

/// The part class story (P15): classes are authored metadata that rides on any element,
/// preserved by copies, independent of the engineering quantities, and refused outside
/// the integer bound. Its own published store keeps the numbered frame's counts the
/// manual runbook records, and its own test keeps the per-test budget honest.
#[test]
fn part_classes_carry_metadata_that_publication_preserves() {
    let s = Story::new();
    s.ok(
        "classes-published",
        "define-material",
        &["--grade", "Q01-steel", "--density", "7850"],
    );
    s.ok(
        "classes-published",
        "create-workspace",
        &[
            "--workspace-dir",
            &s.path("classes-draft"),
            "--user-id",
            USER,
        ],
    );
    let classed = s.ok(
        "classes-draft",
        "create-plate",
        &[
            "--name",
            "Q01 classed plate",
            "--new-id",
            "019a7bf0-0000-7000-8000-00000000eee1",
            "--width",
            "200",
            "--length",
            "300",
            "--thickness",
            "10",
            "--class",
            "-7",
        ],
    );
    let plate = at(&classed, &["plate"]);
    assert_eq!(at(plate, &["class"]).float().unwrap(), -7.0);
    let plate_id = text(plate, "element_id");
    close(
        at(plate, &["quantities", "volume", "mm3"]).float().unwrap(),
        600_000.0,
    );
    // The copy inherits the class; the edit retargets it without touching the mesh or
    // the quantities.
    let copied = s.ok(
        "classes-draft",
        "copy-element",
        &["--id", &plate_id, "--offset", "1000,0,0"],
    );
    assert_eq!(
        at(at(&copied, &["element"]), &["class"]).float().unwrap(),
        -7.0
    );
    let edited = s.ok(
        "classes-draft",
        "update-element",
        &["--id", &plate_id, "--class", "2147483647"],
    );
    let edited_element = at(&edited, &["element"]);
    assert_eq!(
        at(edited_element, &["class"]).float().unwrap(),
        2_147_483_647.0
    );
    assert_eq!(
        text(edited_element, "mesh_digest"),
        text(plate, "mesh_digest"),
        "the class is metadata: the mesh is untouched"
    );
    assert_eq!(
        at(edited_element, &["quantities", "volume", "mm3"])
            .float()
            .unwrap(),
        600_000.0
    );

    // A member defaults to class zero, and a nonzero class is authored.
    s.ok(
        "classes-draft",
        "define-profile",
        &[
            "--designation",
            "Q01-CLASS-BAR",
            "--width",
            "20",
            "--height",
            "30",
        ],
    );
    let plain = s.ok(
        "classes-draft",
        "create-member",
        &[
            "--kind",
            "beam",
            "--start",
            "0,0,0",
            "--end",
            "1000,0,0",
            "--profile",
            "Q01-CLASS-BAR",
        ],
    );
    assert_eq!(
        at(at(&plain, &["member"]), &["class"]).float().unwrap(),
        0.0
    );
    let classed_member = s.ok(
        "classes-draft",
        "create-member",
        &[
            "--kind",
            "beam",
            "--start",
            "2000,0,0",
            "--end",
            "3000,0,0",
            "--profile",
            "Q01-CLASS-BAR",
            "--class",
            "12",
        ],
    );
    assert_eq!(
        at(at(&classed_member, &["member"]), &["class"])
            .float()
            .unwrap(),
        12.0
    );

    // The integer bound holds: every out-of-range class refuses and writes nothing.
    let classed_head = s.head("classes-draft");
    let inspected = s.ok("classes-draft", "inspect-elements", &[]);
    for bad in ["1.5", "NaN", "2147483648", "-2147483649"] {
        let refused = s.run(
            "classes-draft",
            "update-element",
            &["--id", &plate_id, "--class", bad],
            2,
        );
        assert_eq!(text(&refused, "outcome"), "rejected");
    }
    assert_eq!(s.head("classes-draft"), classed_head);
    assert_eq!(
        text(
            &at(&inspected, &["plates"]).array().unwrap()[0],
            "element_id"
        ),
        plate_id
    );

    let classes_published = s.ok(
        "classes-published",
        "publish-workspace",
        &["--workspace-dir", &s.path("classes-draft")],
    );
    assert_eq!(text(&classes_published, "outcome"), "workspace_published");
    let reopened_inspection = s.ok("classes-published", "inspect-elements", &[]);
    let reopened_plates = at(&reopened_inspection, &["plates"]).array().unwrap();
    let reopened_plate = reopened_plates
        .iter()
        .find(|p| text(p, "element_id") == plate_id)
        .expect("the classed plate is present");
    assert_eq!(
        at(reopened_plate, &["class"]).float().unwrap(),
        2_147_483_647.0,
        "the edited class survives the round trip"
    );
    let reopened_copies = reopened_plates
        .iter()
        .filter(|p| text(p, "element_id") != plate_id)
        .collect::<Vec<_>>();
    assert_eq!(reopened_copies.len(), 1);
    assert_eq!(at(reopened_copies[0], &["class"]).float().unwrap(), -7.0);
    let reopened_members = at(&reopened_inspection, &["members"]).array().unwrap();
    let reopened_classed_member = reopened_members
        .iter()
        .find(|m| at(m, &["class"]).float().unwrap() == 12.0)
        .expect("the classed member is present");
    assert_eq!(
        at(reopened_classed_member, &["class"]).float().unwrap(),
        12.0
    );
    let reopened_plain = reopened_members
        .iter()
        .find(|m| at(m, &["class"]).float().unwrap() == 0.0)
        .expect("the plain member is present");
    assert_eq!(
        at(reopened_plain, &["quantities", "volume", "mm3"])
            .float()
            .unwrap(),
        600_000.0
    );
}

/// The cut lifecycle story (A03): a plane cut moves, replays and removes through the
/// owned record, in its own published store so the numbered frame keeps the counts the
/// manual runbook records, and in its own test for the per-test budget. The update keeps
/// the record identity, the identical retry replays the original revision, and the
/// removal frees the ordinal without reusing it.
#[test]
fn cut_updates_and_removal_keep_identity_and_survive_publication() {
    let s = Story::new();
    s.ok(
        "cuts-published",
        "define-material",
        &["--grade", "Q01-steel", "--density", "7850"],
    );
    s.ok(
        "cuts-published",
        "create-workspace",
        &["--workspace-dir", &s.path("cuts-draft"), "--user-id", USER],
    );
    let cut_plate = s.ok(
        "cuts-draft",
        "create-plate",
        &[
            "--name",
            "Q01 cut web",
            "--new-id",
            "019a7bf0-0000-7000-8000-00000000fff1",
            "--width",
            "1000",
            "--length",
            "500",
            "--thickness",
            "10",
        ],
    );
    let web = text(at(&cut_plate, &["plate"]), "element_id");
    close(
        volume_of(&parts(&s.ok("cuts-draft", "inspect-elements", &[])), &web),
        5_000_000.0,
    );

    // The owned cut trims the plate to its kept half-space, exactly.
    let applied = s.ok(
        "cuts-draft",
        "cut-by-plane",
        &[
            "--id",
            &web,
            "--operation-id",
            "019a7bf0-0000-7000-8000-00000000fff3",
            "--point",
            "250,0,0",
            "--normal",
            "1,0,0",
            "--keep",
            "negative",
        ],
    );
    assert_eq!(text(&applied, "outcome"), "cut_applied");
    assert_eq!(
        at(at(&applied, &["record"]), &["ordinal"])
            .unsigned()
            .unwrap(),
        0
    );
    let cut_parts = parts(&s.ok("cuts-draft", "inspect-elements", &[]));
    close(volume_of(&cut_parts, &web), 1_250_000.0);

    // The update moves the same record: the identity stays, the volume follows the point.
    let edited = s.ok(
        "cuts-draft",
        "update-cut",
        &[
            "--id",
            &web,
            "--operation-id",
            "019a7bf0-0000-7000-8000-00000000fff3",
            "--point",
            "500,0,0",
            "--normal",
            "1,0,0",
            "--keep",
            "negative",
            "--op-id",
            "019a7bf0-0000-7000-8000-00000000fff4",
        ],
    );
    let cut_parts = parts(&s.ok("cuts-draft", "inspect-elements", &[]));
    close(volume_of(&cut_parts, &web), 2_500_000.0);
    let cuts_head = s.head("cuts-draft");
    let replay = s.ok(
        "cuts-draft",
        "update-cut",
        &[
            "--id",
            &web,
            "--operation-id",
            "019a7bf0-0000-7000-8000-00000000fff3",
            "--point",
            "500,0,0",
            "--normal",
            "1,0,0",
            "--keep",
            "negative",
            "--op-id",
            "019a7bf0-0000-7000-8000-00000000fff4",
        ],
    );
    assert_eq!(text(&replay, "outcome"), "cut_replayed");
    assert_eq!(at(&replay, &["revision"]), at(&edited, &["revision"]));
    assert_eq!(s.head("cuts-draft"), cuts_head);

    // The removal restores the blank; one undo returns the moved cut and the redo
    // removes it again.
    s.ok(
        "cuts-draft",
        "remove-cut",
        &[
            "--id",
            &web,
            "--operation-id",
            "019a7bf0-0000-7000-8000-00000000fff3",
            "--op-id",
            "019a7bf0-0000-7000-8000-00000000fff5",
        ],
    );
    let removed_parts = parts(&s.ok("cuts-draft", "inspect-elements", &[]));
    close(volume_of(&removed_parts, &web), 5_000_000.0);
    let cuts_undo = s.execute(&["undo-workspace", "--workspace-dir", &s.path("cuts-draft")]);
    assert_eq!(cuts_undo.0, 0);
    let undone = parts(&s.ok("cuts-draft", "inspect-elements", &[]));
    close(volume_of(&undone, &web), 2_500_000.0);
    let cuts_redo = s.execute(&["redo-workspace", "--workspace-dir", &s.path("cuts-draft")]);
    assert_eq!(cuts_redo.0, 0);
    assert_eq!(text(&cuts_redo.1, "outcome"), "workspace_redone");

    // A fresh cut after the removal takes the next ordinal, and publication preserves
    // the record.
    let fresh = s.ok(
        "cuts-draft",
        "cut-by-plane",
        &[
            "--id",
            &web,
            "--operation-id",
            "019a7bf0-0000-7000-8000-00000000fff6",
            "--point",
            "750,0,0",
            "--normal",
            "1,0,0",
            "--keep",
            "negative",
        ],
    );
    assert_eq!(
        at(at(&fresh, &["record"]), &["ordinal"])
            .unsigned()
            .unwrap(),
        1,
        "the removed ordinal is never reused"
    );
    let final_parts = parts(&s.ok("cuts-draft", "inspect-elements", &[]));
    close(volume_of(&final_parts, &web), 3_750_000.0);
    let web_digest = final_parts
        .iter()
        .find(|(element, _, _)| element == web.as_str())
        .expect("the cut web is present")
        .2
        .clone();

    let cuts_published = s.ok(
        "cuts-published",
        "publish-workspace",
        &["--workspace-dir", &s.path("cuts-draft")],
    );
    assert_eq!(text(&cuts_published, "outcome"), "workspace_published");
    let reopened = parts(&s.ok("cuts-published", "inspect-elements", &[]));
    close(volume_of(&reopened, &web), 3_750_000.0);
    let reopened_web = reopened
        .iter()
        .find(|(element, _, _)| element == web.as_str())
        .expect("the cut web is present");
    assert_eq!(
        reopened_web.2, web_digest,
        "the mesh must survive the round trip"
    );
    let reopened_inspection = s.ok("cuts-published", "inspect-elements", &[]);
    let reopened_entry = at(&reopened_inspection, &["plates"])
        .array()
        .unwrap()
        .iter()
        .find(|p| text(p, "element_id") == web)
        .expect("the cut web entry is present");
    let records = at(reopened_entry, &["cuts"]).array().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(
        text(&records[0], "operation_id"),
        "019a7bf0-0000-7000-8000-00000000fff6",
        "the owned cut record survives the round trip"
    );
}

/// The fits and combine story (A13): a member end fits flush with a tool plate's near
/// face and two touching members combine into one, in their own published store so the
/// numbered frame keeps the counts the manual runbook records, and in its own test for
/// the per-test budget. A missing tool refuses not-found, a gapped combine refuses
/// before anything is written, and the combine is one undo unit.
#[test]
fn a_fit_trims_flush_and_a_combine_joins_touching_members() {
    let s = Story::new();
    s.ok(
        "fits-published",
        "define-material",
        &["--grade", "Q01-steel", "--density", "7850"],
    );
    s.ok(
        "fits-published",
        "define-profile",
        &[
            "--designation",
            "Q01-COMBINE-BAR",
            "--width",
            "200",
            "--height",
            "300",
        ],
    );
    s.ok(
        "fits-published",
        "create-workspace",
        &["--workspace-dir", &s.path("fits-draft"), "--user-id", USER],
    );

    // The fit trims the plate flush with the tool's near face (z = 5): the kept side is
    // the plate's midpoint side, so half the plate stays, exactly.
    let fitted = s.ok(
        "fits-draft",
        "create-plate",
        &[
            "--name",
            "Q01 fitted web",
            "--new-id",
            "019a7bf0-0000-7000-8000-00000000a131",
            "--width",
            "1000",
            "--length",
            "500",
            "--thickness",
            "10",
        ],
    );
    let web = text(at(&fitted, &["plate"]), "element_id");
    s.ok(
        "fits-draft",
        "create-plate",
        &[
            "--name",
            "Q01 fit tool",
            "--new-id",
            "019a7bf0-0000-7000-8000-00000000a132",
            "--width",
            "200",
            "--length",
            "200",
            "--thickness",
            "20",
            "--translation",
            "100,100,5",
        ],
    );
    let fit = s.ok(
        "fits-draft",
        "fit-end-to-part",
        &[
            "--id",
            &web,
            "--operation-id",
            "019a7bf0-0000-7000-8000-00000000a133",
            "--tool",
            "019a7bf0-0000-7000-8000-00000000a132",
        ],
    );
    assert_eq!(text(&fit, "outcome"), "a13_applied");
    close(
        at(at(&fit, &["element"]), &["quantities", "volume", "mm3"])
            .float()
            .unwrap(),
        2_500_000.0,
    );
    let fits_head = s.head("fits-draft");
    let missing = s.run(
        "fits-draft",
        "fit-end-to-part",
        &[
            "--id",
            &web,
            "--operation-id",
            "019a7bf0-0000-7000-8000-00000000a134",
            "--tool",
            "019a7bf0-0000-7000-8000-00000000ffff",
        ],
        2,
    );
    assert_eq!(text(&missing, "code"), "NOT_FOUND");
    assert_eq!(s.head("fits-draft"), fits_head);

    // Two touching members combine into one exact body; a gapped pair refuses before
    // anything is written.
    let member = |id: &str, translation: &str| -> String {
        let created = s.ok(
            "fits-draft",
            "create-member",
            &[
                "--kind",
                "beam",
                "--start",
                "0,0,0",
                "--end",
                "4000,0,0",
                "--profile",
                "Q01-COMBINE-BAR",
                "--translation",
                translation,
                "--new-id",
                id,
            ],
        );
        text(at(&created, &["member"]), "element_id")
    };
    let survivor = member("019a7bf0-0000-7000-8000-00000000a135", "0,0,0");
    let consumed = member("019a7bf0-0000-7000-8000-00000000a136", "4000,0,0");
    let apart = member("019a7bf0-0000-7000-8000-00000000a137", "9000,0,0");
    let members_head = s.head("fits-draft");
    let gapped = s.run(
        "fits-draft",
        "combine-parts",
        &[
            "--survivor",
            &survivor,
            "--consumed",
            &apart,
            "--operation-id",
            "019a7bf0-0000-7000-8000-00000000a138",
        ],
        2,
    );
    assert_eq!(text(&gapped, "outcome"), "rejected");
    assert_eq!(s.head("fits-draft"), members_head);
    let combined = s.ok(
        "fits-draft",
        "combine-parts",
        &[
            "--survivor",
            &survivor,
            "--consumed",
            &consumed,
            "--operation-id",
            "019a7bf0-0000-7000-8000-00000000a139",
        ],
    );
    assert_eq!(text(&combined, "outcome"), "a13_applied");
    close(
        at(
            at(&combined, &["element"]),
            &["quantities", "volume", "mm3"],
        )
        .float()
        .unwrap(),
        480_000_000.0,
    );
    assert_eq!(
        parts(&s.ok("fits-draft", "inspect-elements", &[])).len(),
        4,
        "the consumed member is gone; the tool plate and the apart member stay"
    );

    // The combine is one undo unit: the consumed member returns with its own volume.
    let fits_undo = s.execute(&["undo-workspace", "--workspace-dir", &s.path("fits-draft")]);
    assert_eq!(fits_undo.0, 0);
    let undone = parts(&s.ok("fits-draft", "inspect-elements", &[]));
    assert_eq!(undone.len(), 5);
    close(volume_of(&undone, &survivor), 240_000_000.0);
    close(volume_of(&undone, &consumed), 240_000_000.0);
    let fits_redo = s.execute(&["redo-workspace", "--workspace-dir", &s.path("fits-draft")]);
    assert_eq!(fits_redo.0, 0);
    assert_eq!(text(&fits_redo.1, "outcome"), "workspace_redone");
    let final_parts = parts(&s.ok("fits-draft", "inspect-elements", &[]));
    assert_eq!(final_parts.len(), 4);
    close(volume_of(&final_parts, &survivor), 480_000_000.0);
    close(volume_of(&final_parts, &web), 2_500_000.0);
    let web_digest = final_parts
        .iter()
        .find(|(element, _, _)| element == web.as_str())
        .expect("the fitted web is present")
        .2
        .clone();

    let fits_published = s.ok(
        "fits-published",
        "publish-workspace",
        &["--workspace-dir", &s.path("fits-draft")],
    );
    assert_eq!(text(&fits_published, "outcome"), "workspace_published");
    let reopened = parts(&s.ok("fits-published", "inspect-elements", &[]));
    assert_eq!(reopened.len(), 4);
    close(volume_of(&reopened, &survivor), 480_000_000.0);
    close(volume_of(&reopened, &web), 2_500_000.0);
    let reopened_web = reopened
        .iter()
        .find(|(element, _, _)| element == web.as_str())
        .expect("the fitted web is present");
    assert_eq!(
        reopened_web.2, web_digest,
        "the fitted mesh must survive the round trip"
    );
}

/// The hole record story (A04): authored plate holes update and remove through their
/// owned records, in their own published store so the numbered frame keeps the counts
/// the manual runbook records, and in its own test for the per-test budget. The update
/// replaces one record's geometry, the identical retry replays, and the removal frees
/// the record while the survivors keep their exact quantities.
#[test]
fn hole_records_update_remove_and_survive_publication() {
    let s = Story::new();
    s.ok(
        "holes-published",
        "define-material",
        &["--grade", "Q01-steel", "--density", "7850"],
    );
    s.ok(
        "holes-published",
        "create-workspace",
        &["--workspace-dir", &s.path("holes-draft"), "--user-id", USER],
    );

    // The authored creation hole rides on the plate; the exact quantity carries it.
    let holed = s.ok(
        "holes-draft",
        "create-plate",
        &[
            "--name",
            "Q01 holed gusset",
            "--new-id",
            "019a7bf0-0000-7000-8000-00000000a141",
            "--width",
            "1000",
            "--length",
            "1000",
            "--thickness",
            "20",
            "--hole-rect",
            "500,500,200,100",
        ],
    );
    let gusset = text(at(&holed, &["plate"]), "element_id");
    close(
        at(at(&holed, &["plate"]), &["quantities", "volume", "mm3"])
            .float()
            .unwrap(),
        19_600_000.0,
    );

    // A second record joins, then the update replaces its geometry in place.
    s.ok(
        "holes-draft",
        "add-hole",
        &[
            "--id",
            &gusset,
            "--operation-id",
            "019a7bf0-0000-7000-8000-00000000a142",
            "--hole-rect",
            "100,100,100,100",
        ],
    );
    let updated = s.ok(
        "holes-draft",
        "update-hole",
        &[
            "--id",
            &gusset,
            "--operation-id",
            "019a7bf0-0000-7000-8000-00000000a142",
            "--hole-rect",
            "100,100,300,300",
            "--op-id",
            "019a7bf0-0000-7000-8000-00000000a144",
        ],
    );
    assert_eq!(text(&updated, "outcome"), "hole_updated");
    close(
        at(at(&updated, &["element"]), &["quantities", "volume", "mm3"])
            .float()
            .unwrap(),
        17_800_000.0,
    );
    let holes_head = s.head("holes-draft");
    let replay = s.ok(
        "holes-draft",
        "update-hole",
        &[
            "--id",
            &gusset,
            "--operation-id",
            "019a7bf0-0000-7000-8000-00000000a142",
            "--hole-rect",
            "100,100,300,300",
            "--op-id",
            "019a7bf0-0000-7000-8000-00000000a144",
        ],
    );
    assert_eq!(text(&replay, "outcome"), "hole_update_replayed");
    assert_eq!(at(&replay, &["revision"]), at(&updated, &["revision"]));
    assert_eq!(s.head("holes-draft"), holes_head);

    // A third record joins and its removal restores its exact share; the two survivors
    // keep their records.
    s.ok(
        "holes-draft",
        "add-hole",
        &[
            "--id",
            &gusset,
            "--operation-id",
            "019a7bf0-0000-7000-8000-00000000a143",
            "--hole-rect",
            "700,100,100,100",
        ],
    );
    let removed = s.ok(
        "holes-draft",
        "remove-hole",
        &[
            "--id",
            &gusset,
            "--operation-id",
            "019a7bf0-0000-7000-8000-00000000a143",
            "--op-id",
            "019a7bf0-0000-7000-8000-00000000a145",
        ],
    );
    assert_eq!(text(&removed, "outcome"), "hole_removed");
    close(
        at(at(&removed, &["element"]), &["quantities", "volume", "mm3"])
            .float()
            .unwrap(),
        17_800_000.0,
    );
    let removed_inspection = s.ok("holes-draft", "inspect-elements", &[]);
    let records = at(
        at(&removed_inspection, &["plates"])
            .array()
            .unwrap()
            .iter()
            .find(|p| text(p, "element_id") == gusset)
            .expect("the gusset entry is present"),
        &["hole_records"],
    )
    .array()
    .unwrap();
    assert_eq!(
        records.len(),
        1,
        "the authored contour hole is geometry, not a record; one record survives"
    );

    // The removal is one undo unit: the record returns, and the redo removes it again.
    let holes_undo = s.execute(&["undo-workspace", "--workspace-dir", &s.path("holes-draft")]);
    assert_eq!(holes_undo.0, 0);
    let undone = parts(&s.ok("holes-draft", "inspect-elements", &[]));
    close(volume_of(&undone, &gusset), 17_600_000.0);
    let holes_redo = s.execute(&["redo-workspace", "--workspace-dir", &s.path("holes-draft")]);
    assert_eq!(holes_redo.0, 0);
    assert_eq!(text(&holes_redo.1, "outcome"), "workspace_redone");
    let final_parts = parts(&s.ok("holes-draft", "inspect-elements", &[]));
    close(volume_of(&final_parts, &gusset), 17_800_000.0);
    let gusset_digest = final_parts
        .iter()
        .find(|(element, _, _)| element == gusset.as_str())
        .expect("the gusset is present")
        .2
        .clone();

    let holes_published = s.ok(
        "holes-published",
        "publish-workspace",
        &["--workspace-dir", &s.path("holes-draft")],
    );
    assert_eq!(text(&holes_published, "outcome"), "workspace_published");
    let reopened = parts(&s.ok("holes-published", "inspect-elements", &[]));
    close(volume_of(&reopened, &gusset), 17_800_000.0);
    let reopened_gusset = reopened
        .iter()
        .find(|(element, _, _)| element == gusset.as_str())
        .expect("the gusset is present");
    assert_eq!(
        reopened_gusset.2, gusset_digest,
        "the holed mesh must survive the round trip"
    );
    let reopened_inspection = s.ok("holes-published", "inspect-elements", &[]);
    let reopened_records = at(
        at(&reopened_inspection, &["plates"])
            .array()
            .unwrap()
            .iter()
            .find(|p| text(p, "element_id") == gusset)
            .expect("the gusset entry is present"),
        &["hole_records"],
    )
    .array()
    .unwrap();
    assert_eq!(
        reopened_records.len(),
        1,
        "the record survives the round trip"
    );
}

/// The storey assignment story (D2): levels are elements with their own identity and no
/// body, and assignment is authored metadata — a level never touches a mesh. The story
/// runs in its own published store so the numbered frame keeps the counts the manual
/// runbook records, and in its own test for the per-test budget.
#[test]
fn storey_assignment_rides_levels_across_publication() {
    let s = Story::new();
    s.ok(
        "storeys-published",
        "define-material",
        &["--grade", "Q01-steel", "--density", "7850"],
    );
    s.ok(
        "storeys-published",
        "define-profile",
        &[
            "--designation",
            "Q01-STOREY-BAR",
            "--width",
            "100",
            "--height",
            "100",
        ],
    );
    s.ok(
        "storeys-published",
        "create-workspace",
        &[
            "--workspace-dir",
            &s.path("storeys-draft"),
            "--user-id",
            USER,
        ],
    );
    let ground_level = s.ok(
        "storeys-draft",
        "create-level",
        &[
            "--name",
            "Q01 ground",
            "--new-id",
            "019a7bf0-0000-7000-8000-00000000a151",
            "--elevation",
            "0",
        ],
    );
    let ground_id = text(&ground_level, "element_id");
    let storey_level = s.ok(
        "storeys-draft",
        "create-level",
        &[
            "--name",
            "Q01 storey",
            "--new-id",
            "019a7bf0-0000-7000-8000-00000000a152",
            "--elevation",
            "3000",
        ],
    );
    let storey_id = text(&storey_level, "element_id");
    let plated = s.ok(
        "storeys-draft",
        "create-plate",
        &[
            "--name",
            "Q01 storey plate",
            "--new-id",
            "019a7bf0-0000-7000-8000-00000000a153",
            "--width",
            "200",
            "--length",
            "300",
            "--thickness",
            "10",
        ],
    );
    let plate = text(at(&plated, &["plate"]), "element_id");
    let beamed = s.ok(
        "storeys-draft",
        "create-member",
        &[
            "--kind",
            "beam",
            "--start",
            "0,0,0",
            "--end",
            "1000,0,0",
            "--profile",
            "Q01-STOREY-BAR",
            "--name",
            "Q01 storey beam",
            "--new-id",
            "019a7bf0-0000-7000-8000-00000000a154",
        ],
    );
    let beam = text(at(&beamed, &["member"]), "element_id");
    let before = parts(&s.ok("storeys-draft", "inspect-elements", &[]));

    // The assignment is authored metadata: the meshes stay byte-identical while the
    // levels carry their elements.
    let assigned = s.ok(
        "storeys-draft",
        "update-element",
        &["--id", &plate, "--level", &storey_id],
    );
    assert_eq!(text(&assigned, "outcome"), "element_updated");
    s.ok(
        "storeys-draft",
        "update-element",
        &["--id", &beam, "--level", &ground_id],
    );
    let after = parts(&s.ok("storeys-draft", "inspect-elements", &[]));
    for (found, was) in after.iter().zip(before.iter()) {
        assert_eq!(found.2, was.2, "{} mesh is untouched", was.0);
        close(found.1, was.1);
    }
    let storeys = s.ok("storeys-draft", "levels", &[]);
    let listed = at(&storeys, &["levels"]).array().unwrap();
    assert_eq!(listed.len(), 2);
    let ground = listed
        .iter()
        .find(|l| text(l, "name") == "Q01 ground")
        .expect("the ground level is present");
    let storey = listed
        .iter()
        .find(|l| text(l, "name") == "Q01 storey")
        .expect("the storey level is present");
    assert_eq!(
        at(ground, &["elements"]).array().unwrap().len(),
        1,
        "the beam sits on the ground level"
    );
    assert_eq!(
        at(storey, &["elements"]).array().unwrap().len(),
        1,
        "the plate sits on the storey level"
    );

    // Reassignment moves the element between storeys; an unknown level refuses and
    // writes nothing.
    s.ok(
        "storeys-draft",
        "update-element",
        &["--id", &beam, "--level", &storey_id],
    );
    let moved = s.ok("storeys-draft", "levels", &[]);
    let moved_listed = at(&moved, &["levels"]).array().unwrap();
    let moved_ground = moved_listed
        .iter()
        .find(|l| text(l, "name") == "Q01 ground")
        .expect("the ground level is present");
    let moved_storey = moved_listed
        .iter()
        .find(|l| text(l, "name") == "Q01 storey")
        .expect("the storey level is present");
    assert_eq!(at(moved_ground, &["elements"]).array().unwrap().len(), 0);
    assert_eq!(at(moved_storey, &["elements"]).array().unwrap().len(), 2);
    let storeys_head = s.head("storeys-draft");
    let unknown = s.run(
        "storeys-draft",
        "update-element",
        &[
            "--id",
            &beam,
            "--level",
            "019a7bf0-0000-7000-8000-00000000ffff",
        ],
        2,
    );
    assert_eq!(text(&unknown, "outcome"), "rejected");
    assert_eq!(s.head("storeys-draft"), storeys_head);

    // The reassignment is one undo unit: the beam returns to the ground level.
    let storeys_undo = s.execute(&[
        "undo-workspace",
        "--workspace-dir",
        &s.path("storeys-draft"),
    ]);
    assert_eq!(storeys_undo.0, 0);
    let undone = s.ok("storeys-draft", "levels", &[]);
    let undone_listed = at(&undone, &["levels"]).array().unwrap();
    let undone_ground = undone_listed
        .iter()
        .find(|l| text(l, "name") == "Q01 ground")
        .expect("the ground level is present");
    assert_eq!(at(undone_ground, &["elements"]).array().unwrap().len(), 1);
    let storeys_redo = s.execute(&[
        "redo-workspace",
        "--workspace-dir",
        &s.path("storeys-draft"),
    ]);
    assert_eq!(storeys_redo.0, 0);
    assert_eq!(text(&storeys_redo.1, "outcome"), "workspace_redone");

    let storeys_published = s.ok(
        "storeys-published",
        "publish-workspace",
        &["--workspace-dir", &s.path("storeys-draft")],
    );
    assert_eq!(text(&storeys_published, "outcome"), "workspace_published");
    let reopened = s.ok("storeys-published", "levels", &[]);
    let reopened_listed = at(&reopened, &["levels"]).array().unwrap();
    assert_eq!(reopened_listed.len(), 2);
    let reopened_storey = reopened_listed
        .iter()
        .find(|l| text(l, "name") == "Q01 storey")
        .expect("the storey level is present");
    assert_eq!(
        at(reopened_storey, &["elements"]).array().unwrap().len(),
        2,
        "both elements ride the storey across publication"
    );
    let reopened_parts = parts(&s.ok("storeys-published", "inspect-elements", &[]));
    assert_eq!(reopened_parts.len(), 2);
    close(volume_of(&reopened_parts, &plate), 600_000.0);
    close(volume_of(&reopened_parts, &beam), 10_000_000.0);
}

/// The renumbering story (A08): the preview goes stale exactly when an edit changes the
/// numbering, the approval follows the fresh token, and the marks survive publication —
/// in its own published store so the numbered frame keeps the counts the manual runbook
/// records, and in its own test for the per-test budget.
#[test]
fn renumbering_goes_stale_exactly_when_the_numbering_changes() {
    let s = Story::new();
    s.ok(
        "numbering-published",
        "define-material",
        &["--grade", "Q01-steel", "--density", "7850"],
    );
    s.ok(
        "numbering-published",
        "define-profile",
        &[
            "--designation",
            "Q01-NUM-BAR",
            "--width",
            "100",
            "--height",
            "50",
        ],
    );
    s.ok(
        "numbering-published",
        "create-workspace",
        &[
            "--workspace-dir",
            &s.path("numbering-draft"),
            "--user-id",
            USER,
        ],
    );
    let numbered = |id: &str, name: &str| -> String {
        let created = s.ok(
            "numbering-draft",
            "create-member",
            &[
                "--kind",
                "beam",
                "--start",
                "0,0,0",
                "--end",
                "2000,0,0",
                "--profile",
                "Q01-NUM-BAR",
                "--name",
                name,
                "--new-id",
                id,
            ],
        );
        text(at(&created, &["member"]), "element_id")
    };
    let first = numbered("019a7bf0-0000-7000-8000-00000000a161", "Q01 numbered beam");
    numbered("019a7bf0-0000-7000-8000-00000000a162", "Q01 numbered beam");
    let plated = s.ok(
        "numbering-draft",
        "create-plate",
        &[
            "--name",
            "Q01 numbered plate",
            "--new-id",
            "019a7bf0-0000-7000-8000-00000000a163",
            "--width",
            "100",
            "--length",
            "100",
            "--thickness",
            "10",
        ],
    );
    let plate = text(at(&plated, &["plate"]), "element_id");

    // The first preview is fresh, and a preview never writes the store.
    let numbering_head = s.head("numbering-draft");
    let first_preview = s.ok("numbering-draft", "preview-numbering", &[]);
    let token = text(&first_preview, "preview_token");
    assert_eq!(s.head("numbering-draft"), numbering_head);
    let fresh = s.ok(
        "numbering-draft",
        "preview-numbering",
        &["--previous", &token],
    );
    assert!(matches!(fresh.get("stale").unwrap(), Json::Bool(false)));

    // A rename does not change the numbering; a thickness edit that splits the plate
    // from its group does.
    s.ok(
        "numbering-draft",
        "update-element",
        &["--id", &first, "--name", "Q01 renamed beam"],
    );
    let after_rename = s.ok(
        "numbering-draft",
        "preview-numbering",
        &["--previous", &token],
    );
    assert!(matches!(
        after_rename.get("stale").unwrap(),
        Json::Bool(false)
    ));
    s.ok(
        "numbering-draft",
        "update-element",
        &["--id", &plate, "--thickness", "20"],
    );
    let after_edit = s.ok(
        "numbering-draft",
        "preview-numbering",
        &["--previous", &token],
    );
    assert!(matches!(after_edit.get("stale").unwrap(), Json::Bool(true)));
    let token_two = text(
        &s.ok("numbering-draft", "preview-numbering", &[]),
        "preview_token",
    );
    assert_ne!(token_two, token);
    let approved = s.ok(
        "numbering-draft",
        "approve-numbering",
        &["--preview-token", &token_two],
    );
    assert_eq!(approved.get("marks").unwrap().unsigned().unwrap(), 3);

    // A copy is a numbering edit too: stale again, and the second approval adds the mark.
    s.ok("numbering-draft", "copy-element", &["--id", &plate]);
    let after_copy = s.ok(
        "numbering-draft",
        "preview-numbering",
        &["--previous", &token_two],
    );
    assert!(matches!(after_copy.get("stale").unwrap(), Json::Bool(true)));
    let token_three = text(
        &s.ok("numbering-draft", "preview-numbering", &[]),
        "preview_token",
    );
    let approved_again = s.ok(
        "numbering-draft",
        "approve-numbering",
        &["--preview-token", &token_three],
    );
    assert_eq!(approved_again.get("marks").unwrap().unsigned().unwrap(), 4);

    let numbering_published = s.ok(
        "numbering-published",
        "publish-workspace",
        &["--workspace-dir", &s.path("numbering-draft")],
    );
    assert_eq!(text(&numbering_published, "outcome"), "workspace_published");
    let still_fresh = s.ok(
        "numbering-published",
        "preview-numbering",
        &["--previous", &token_three],
    );
    assert!(
        matches!(still_fresh.get("stale").unwrap(), Json::Bool(false)),
        "the published head is exactly the approved numbering"
    );
    assert_eq!(
        at(&still_fresh, &["counts", "parts"]).unsigned().unwrap(),
        4
    );
}
