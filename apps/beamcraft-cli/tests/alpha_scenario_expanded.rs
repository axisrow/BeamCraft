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
    assert_eq!(numbers(&mirrored_points[0]), [-2000.0, 0.0, 0.0]);
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
