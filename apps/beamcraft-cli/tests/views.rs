// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic model and frames, MPL-2.0.

//! V12 model views through the real binary: every way of building a view plane, the display
//! properties, batches, update, apply and removal. A view is display state: the model head
//! proves itself unchanged across every view operation.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use crate::support;
use beamcraft_engine::views::{ModelView, ViewBook};
use beamcraft_engine::work_plane::WorkPlane;
use beamcraft_json::{Json, Limits};
use beamcraft_model::id::Uuid7;
use support::{StoreFixture, at, list, text};

struct Views {
    f: StoreFixture,
    file: String,
}

impl Views {
    fn new() -> Self {
        let f = StoreFixture::new("v12-views");
        let file = f.root.join("views.json").to_str().unwrap().to_owned();
        Self { f, file }
    }

    fn ok(&self, command: &str, args: &[&str]) -> Json {
        let (code, reply) = self.f.run(command, args);
        assert_eq!(code, 0, "{command} {args:?}: {reply:?}");
        reply
    }

    fn view(&self, command: &str, args: &[&str]) -> Json {
        let mut all = vec!["--file", self.file.as_str()];
        all.extend_from_slice(args);
        self.ok(command, &all)
    }

    fn refuse(&self, command: &str, args: &[&str]) {
        let mut all = vec!["--file", self.file.as_str()];
        all.extend_from_slice(args);
        let (code, reply) = self.f.run(command, &all);
        assert_eq!(code, 2, "{command} {args:?}: {reply:?}");
        assert_eq!(text(&reply, "outcome"), "rejected");
    }

    fn names(&self) -> Vec<String> {
        list(&self.view("list-views", &[]), "views")
            .iter()
            .map(|v| text(v, "name"))
            .collect()
    }
}

fn numbers(json: &Json) -> Vec<f64> {
    json.array()
        .unwrap()
        .iter()
        .map(|v| v.float().unwrap())
        .collect()
}

#[test]
fn every_plane_builds_a_view_and_the_model_stays_unchanged() {
    let v = Views::new();
    let plate = v.ok(
        "create-plate",
        &["--width", "1000", "--length", "500", "--thickness", "10"],
    );
    let plate_id = text(at(&plate, &["plate"]), "element_id");
    v.ok(
        "create-grid",
        &[
            "--name",
            "G",
            "--u-expression",
            "0 2*6000",
            "--v-expression",
            "0 7500",
        ],
    );
    for (name, elevation) in [("Ground", "0"), ("First", "4000")] {
        v.ok("create-level", &["--name", name, "--elevation", elevation]);
    }
    let assembly = text(&v.ok("create-assembly", &["--name", "Frame"]), "element_id");
    v.ok(
        "attach-to-assembly",
        &["--id", &plate_id, "--assembly-id", &assembly],
    );
    let head = v.ok("open-revision", &[]);

    let made = |args: &[&str]| -> Json {
        let reply = v.view("save-view", args);
        assert_eq!(text(&reply, "outcome"), "view_saved");
        reply
    };
    made(&["--name", "3D", "--yaw", "0.7", "--pitch", "0.85"]);
    let plan = made(&[
        "--name",
        "Plan +3000",
        "--parallel",
        "xy",
        "--offset",
        "3000",
    ]);
    let plane = at(&list(&plan, "views")[0], &["plane"]);
    assert_eq!(numbers(at(plane, &["origin_mm"])), [0.0, 0.0, 3000.0]);
    made(&["--name", "Section", "--two-points", "0,0,0;6000,0,0"]);
    made(&[
        "--name",
        "Across",
        "--two-points",
        "0,0,0;6000,0,500",
        "--work-plane",
        "0,0,0,1,0,0,0,1,0",
    ]);
    made(&[
        "--name",
        "Inclined",
        "--three-points",
        "0,0,0;1000,0,100;0,1000,200",
    ]);
    made(&["--name", "Frame", "--plane", "0,100,0,1,0,0,0,0,1"]);
    made(&["--name", "Axis 1", "--grid", "G", "--axis", "1"]);
    made(&["--name", "Level", "--level", "First"]);
    let part = made(&["--name", "Plate", "--element", &plate_id, "--side", "front"]);
    assert!(at(&list(&part, "views")[0], &["span_mm"]).float().unwrap() > 1000.0);
    made(&[
        "--name",
        "Assembly",
        "--assembly",
        &assembly,
        "--side",
        "top",
    ]);
    made(&[
        "--name",
        "Face",
        "--ray-origin",
        "500,250,1000",
        "--ray-direction",
        "0,0,-1",
    ]);
    // Batches: one view per axis, family by family, named by prefix and label; four part views.
    let axes = made(&[
        "--name",
        "Elevation",
        "--grid",
        "G",
        "--u-axes",
        "first",
        "--v-axes",
        "all",
        "--v-prefix",
        "Line",
    ]);
    let axis_names: Vec<String> = list(&axes, "views")
        .iter()
        .map(|j| text(j, "name"))
        .collect();
    assert_eq!(axis_names, ["Elevation 1", "Line A", "Line B"]);
    let plans = made(&["--name", "Plan", "--levels", "all"]);
    assert_eq!(list(&plans, "views").len(), 2);
    let four = made(&["--name", "Part", "--element", &plate_id, "--side", "four"]);
    assert_eq!(
        list(&four, "views")
            .iter()
            .map(|j| text(j, "name"))
            .collect::<Vec<_>>(),
        ["Part front", "Part top", "Part end", "Part 3d"]
    );
    // A repeated batch suffixes its names rather than refusing.
    let again = made(&["--name", "Plan", "--levels", "first"]);
    assert_eq!(text(&list(&again, "views")[0], "name"), "Plan Ground (2)");
    assert_eq!(v.names().len(), 21);

    assert_eq!(
        v.ok("open-revision", &[]),
        head,
        "views never write the model"
    );
}

#[test]
fn provenance_stamps_the_camera_with_author_time_comment_and_revision() {
    let v = Views::new();
    let plate = v.ok(
        "create-plate",
        &["--width", "500", "--length", "250", "--thickness", "8"],
    );
    let plate_id = text(at(&plate, &["plate"]), "element_id");
    let head = v.ok("open-revision", &[]);
    let revision = text(&head, "revision_id");

    // A provenance-bearing save records the author, the comment, the save time and the model
    // revision the view was captured against.
    let saved = v.view(
        "save-view",
        &[
            "--name",
            "Front",
            "--parallel",
            "xz",
            "--author",
            "Ana",
            "--comment",
            "member frames",
        ],
    );
    let stamp = at(&list(&saved, "views")[0], &["provenance"]);
    assert_eq!(text(stamp, "author"), "Ana");
    assert_eq!(text(stamp, "comment"), "member frames");
    assert_eq!(text(stamp, "model_revision"), revision);
    let created = at(stamp, &["created_at_ms"]).unsigned().unwrap();
    assert!(created > 1_700_000_000_000, "the stamp carries a real time");

    // Restoring returns the camera with its stamp; the book lists it too.
    let applied = v.view("apply-view", &["--view", "front"]);
    assert_eq!(
        numbers(at(&applied, &["camera_plane", "origin_mm"])),
        [0.0; 3]
    );
    assert_eq!(text(at(&applied, &["provenance"]), "author"), "Ana");
    assert_eq!(
        text(at(&applied, &["provenance"]), "model_revision"),
        revision
    );
    let listed = v.view("list-views", &[]);
    assert_eq!(
        text(at(&list(&listed, "views")[0], &["provenance"]), "author"),
        "Ana"
    );

    // A batch stamps every created view with the same revision.
    let four = v.view(
        "save-view",
        &[
            "--name",
            "Part",
            "--element",
            &plate_id,
            "--side",
            "four",
            "--author",
            "Bo",
        ],
    );
    assert_eq!(list(&four, "views").len(), 4);
    for entry in list(&four, "views") {
        assert_eq!(text(at(entry, &["provenance"]), "author"), "Bo");
        assert_eq!(text(at(entry, &["provenance"]), "model_revision"), revision);
    }
    // A plain save carries no stamp at all.
    let plain = v.view("save-view", &["--name", "Plain", "--parallel", "yz"]);
    assert!(matches!(
        at(&list(&plain, "views")[0], &["provenance"]),
        Json::Null
    ));

    // A comment without an author, an empty author, and a stamp rewrite are refused.
    v.refuse(
        "save-view",
        &["--name", "Bad", "--parallel", "xy", "--comment", "c"],
    );
    v.refuse(
        "save-view",
        &["--name", "Bad", "--parallel", "xy", "--author", "   "],
    );
    v.refuse("update-view", &["--view", "Front", "--author", "Bo"]);

    // Views never write the model: the head, and with it the keys and numbering, is unchanged.
    assert_eq!(v.ok("open-revision", &[]), head);
}

#[test]
fn properties_update_apply_and_remove() {
    let v = Views::new();
    let saved = v.view(
        "save-view",
        &[
            "--name",
            "Section A",
            "--three-points",
            "0,0,0;1000,0,0;0,0,1000",
            "--projection",
            "perspective",
            "--span",
            "8000",
            "--depth",
            "500,250",
            "--rotation",
            "90,0",
            "--render-mode",
            "wireframe",
            "--grids",
            "false",
            "--hidden-kinds",
            "beam,column",
            "--work-area",
            "0,0,0,12000,7500,6000",
            "--color",
            "0.2,0.4,0.6",
            "--opacity",
            "0.5",
            "--folder",
            "Sections/Main",
            "--favorite",
            "true",
        ],
    );
    let view = &list(&saved, "views")[0];
    let id = text(view, "id");
    assert_eq!(text(view, "projection"), "perspective");
    assert_eq!(numbers(at(view, &["depth_mm"])), [500.0, 250.0]);
    assert!((numbers(at(view, &["rotation_rad"]))[0] - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
    assert_eq!(text(view, "render_mode"), "wireframe");
    assert!(!at(view, &["grids_visible"]).boolean().unwrap());
    assert_eq!(
        at(view, &["hidden_kinds"])
            .array()
            .unwrap()
            .iter()
            .map(|k| k.string().unwrap())
            .collect::<Vec<_>>(),
        ["beam", "column"]
    );
    assert_eq!(text(view, "folder"), "Sections/Main");
    let listed = v.view("list-views", &[]);
    assert_eq!(
        list(&listed, "folders")
            .iter()
            .map(|f| f.string().unwrap())
            .collect::<Vec<_>>(),
        ["Sections", "Sections/Main"]
    );

    // Apply returns the camera frame (turned by the rotation) and the slab and box planes.
    let applied = v.view("apply-view", &["--view", "section a"]);
    assert_eq!(text(&applied, "outcome"), "view_applied");
    assert_eq!(list(&applied, "clip_planes").len(), 8);
    let camera_x = numbers(at(&applied, &["camera_plane", "x"]));
    assert!((camera_x[2] - 1.0).abs() < 1e-9, "{camera_x:?}");

    // Update keeps what it is not told to change; copy-from reuses display properties.
    v.view("save-view", &["--name", "Plain", "--parallel", "xz"]);
    let updated = v.view(
        "update-view",
        &[
            "--view",
            "Plain",
            "--name",
            "Plain renamed",
            "--copy-from",
            &id,
            "--opacity",
            "1",
        ],
    );
    let plain = &list(&updated, "views")[0];
    assert_eq!(text(plain, "name"), "Plain renamed");
    assert_eq!(text(plain, "render_mode"), "wireframe");
    assert_eq!(at(plain, &["opacity"]).float().unwrap(), 1.0);
    assert_eq!(
        text(plain, "folder"),
        "",
        "the folder is not a display property"
    );

    // Refusals change nothing: a taken name, a bad depth, degenerate points, a batch update.
    let before = std::fs::read(&v.file).unwrap();
    v.refuse("save-view", &["--name", "SECTION A", "--parallel", "xy"]);
    v.refuse(
        "save-view",
        &["--name", "Bad", "--parallel", "xy", "--depth", "-1,2"],
    );
    v.refuse(
        "save-view",
        &["--name", "Bad", "--three-points", "0,0,0;1,0,0;2,0,0"],
    );
    v.refuse(
        "save-view",
        &[
            "--name",
            "Bad",
            "--parallel",
            "xy",
            "--plane",
            "0,0,0,1,0,0,0,1,0",
        ],
    );
    v.refuse("save-view", &["--name", "Bad"]);
    v.refuse(
        "update-view",
        &["--view", "Plain renamed", "--name", "Section A"],
    );
    v.refuse("update-view", &["--view", "Missing", "--span", "100"]);
    assert_eq!(std::fs::read(&v.file).unwrap(), before);

    let removed = v.view("remove-view", &["--view", &id]);
    assert_eq!(at(&removed, &["view_count"]).unsigned().unwrap(), 1);
    v.refuse("apply-view", &["--view", &id]);
    // A broken file is refused, not overwritten.
    std::fs::write(&v.file, b"{\"version\":7}").unwrap();
    v.refuse("save-view", &["--name", "New", "--parallel", "xy"]);
    assert_eq!(std::fs::read(&v.file).unwrap(), b"{\"version\":7}");
}

#[test]
fn save_view_names_views_by_default() {
    let v = Views::new();
    for (name, elevation) in [("Ground", "0"), ("First", "4000")] {
        v.ok("create-level", &["--name", name, "--elevation", elevation]);
    }
    v.view("save-view", &["--parallel", "xy"]);
    v.view("save-view", &["--parallel", "xy"]);
    assert_eq!(v.names(), ["View 1", "View 2"]);
    v.view("remove-view", &["--view", "View 1"]);
    v.view("save-view", &["--parallel", "xz"]);
    let mut names = v.names();
    names.sort();
    assert_eq!(names, ["View 1", "View 2"]);

    // A batch without --name is named by its labels alone.
    let batch = v.view("save-view", &["--levels", "all"]);
    assert_eq!(
        list(&batch, "views")
            .iter()
            .map(|j| text(j, "name"))
            .collect::<Vec<_>>(),
        ["Ground", "First"]
    );

    // An explicit empty name is refused, not replaced by a default.
    v.refuse("save-view", &["--name", "  ", "--parallel", "xy"]);
}

#[test]
fn the_views_file_is_a_camera_prim_layer_and_a_json_sidecar_migrates() {
    let v = Views::new();
    v.ok(
        "create-plate",
        &["--width", "500", "--length", "250", "--thickness", "8"],
    );
    let head = v.ok("open-revision", &[]);
    v.view(
        "save-view",
        &["--name", "Front", "--parallel", "xz", "--author", "Ana"],
    );

    // The stored file is the W01 views layer: a USD document of placed camera prims, not JSON.
    let bytes = std::fs::read(&v.file).unwrap();
    assert!(
        bytes.starts_with(b"#usda"),
        "the views file is a USD views layer"
    );
    let body = std::str::from_utf8(&bytes).unwrap();
    assert!(body.contains("def Camera"), "a saved view is a camera prim");
    // Restoring reads the camera prim back with its provenance.
    let applied = v.view("apply-view", &["--view", "front"]);
    assert_eq!(text(at(&applied, &["provenance"]), "author"), "Ana");

    // A legacy JSON sidecar still reads, and the next save migrates it into the layer form.
    let mut book = ViewBook::new();
    let mut old = ModelView::new(
        Uuid7::parse("0199e7a0-0000-7000-8000-0000000000aa").unwrap(),
        "Old Plan",
        WorkPlane::default(),
        1234.0,
    );
    old.folder = "Old".into();
    book.insert(old).unwrap();
    let legacy = beamcraft_host::views::book_to_json(&book)
        .unwrap()
        .encode(&Limits::DEFAULT)
        .unwrap();
    std::fs::write(&v.file, &legacy).unwrap();
    let listed = v.view("list-views", &[]);
    assert_eq!(text(&list(&listed, "views")[0], "name"), "Old Plan");
    v.view(
        "save-view",
        &["--name", "New", "--parallel", "yz", "--author", "Bo"],
    );
    let bytes = std::fs::read(&v.file).unwrap();
    assert!(
        bytes.starts_with(b"#usda"),
        "the save migrated the sidecar into the layer form"
    );
    let listed = v.view("list-views", &[]);
    assert_eq!(list(&listed, "views").len(), 2);

    // Views never write the model, in either file form.
    assert_eq!(v.ok("open-revision", &[]), head);
}
