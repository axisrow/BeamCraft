// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic grids; no commercial model or catalogue data.

//! Grid insertion uses the same CLI command as a script, and the pinned refresh carries guides.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use beamcraft_json::{Json, Limits};
use beamcraft_ui::camera::Camera;
use beamcraft_ui::grid::{GridForm, GridScene};
use beamcraft_ui::source::{self, Known, Refreshed};
use beamcraft_ui::stats::Stats;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
const ACTOR: &str = "0199e7a0-0000-7000-8000-0000000000a1";

struct Fixture {
    root: PathBuf,
    cli: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let cli = Path::new(env!("CARGO_BIN_EXE_beamcraft-ui"))
            .parent()
            .unwrap()
            .join(format!("beamcraft{}", std::env::consts::EXE_SUFFIX));
        assert!(
            cli.is_file(),
            "build beamcraft-cli before running grid UI acceptance"
        );
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("grid-ui-tests")
            .join(format!(
                "{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        std::fs::create_dir_all(&root).unwrap();
        let f = Self { root, cli };
        for name in ["ui", "script"] {
            f.ok(&[
                "create-project",
                "--parent",
                f.path().to_str().unwrap(),
                "--name",
                name,
                "--actor-id",
                ACTOR,
                "--config-dir",
                f.root.join("config").to_str().unwrap(),
            ]);
        }
        f
    }
    fn path(&self) -> &Path {
        &self.root
    }
    fn run(&self, args: &[&str]) -> (i32, Json) {
        let out = Command::new(&self.cli).args(args).output().unwrap();
        (
            out.status.code().unwrap(),
            beamcraft_json::decode(&out.stdout, &Limits::DEFAULT).unwrap(),
        )
    }
    fn ok(&self, args: &[&str]) -> Json {
        let (exit, json) = self.run(args);
        assert_eq!(exit, 0, "{args:?}: {json:?}");
        json
    }
    fn head(&self, store: &Path) -> Json {
        self.ok(&["open-revision", "--store", store.to_str().unwrap()])
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn at<'a>(json: &'a Json, keys: &[&str]) -> &'a Json {
    keys.iter().fold(json, |j, k| j.get(k).unwrap())
}

#[test]
fn displayed_grid_can_be_picked_edited_and_restored_by_workspace_history() {
    let f = Fixture::new();
    let published = f.root.join("ui");
    let form = GridForm::default();
    let created = beamcraft_ui::grid::commit(
        &f.cli,
        &form.prepare(&published, Some(ACTOR)).unwrap(),
        &Stats::default(),
    )
    .unwrap();
    let workspace = f.root.join("draft");
    f.ok(&[
        "create-workspace",
        "--store",
        published.to_str().unwrap(),
        "--workspace-dir",
        workspace.to_str().unwrap(),
    ]);
    let initial = f.ok(&["list-grids", "--store", workspace.to_str().unwrap()]);
    let read = source::reload(&f.cli, &workspace, &Known::default(), &Stats::default());
    let guides = read.grids.unwrap();
    let grid = guides.get(&created.id).unwrap();
    assert!(grid.editable);
    assert_eq!(grid.form.u.expression, "0 2*4000");
    assert_eq!(grid.form.u.span, ["2000", "12000"]);
    let Refreshed::New(scene) = read.scene.unwrap() else {
        panic!("expected snapshot")
    };
    let summary = scene.1.summary();
    let rect = egui::Rect::from_min_size(egui::pos2(30.0, 50.0), egui::vec2(1200.0, 800.0));
    let (target, extent) = guides.framing(&summary);
    let view = Camera::new(target, extent, 1.5).unwrap().view(1.5).unwrap();
    let line = &guides.lines[0];
    let middle = std::array::from_fn(|i| {
        ((line.points[0][i] + line.points[1][i]) * 0.5 - summary.origin_mm[i]) as f32
    });
    let [x, y, _] = view.project(middle);
    let pixel = egui::pos2(
        rect.left() + (x + 1.0) * rect.width() / 2.0,
        rect.top() + (1.0 - y) * rect.height() / 2.0,
    );
    assert_eq!(
        guides
            .hit_test(rect, &view, summary.origin_mm, pixel)
            .unwrap()
            .id,
        created.id
    );
    assert!(
        guides
            .hit_test(
                rect,
                &view,
                summary.origin_mm,
                rect.min - egui::vec2(20.0, 20.0)
            )
            .is_none()
    );
    let before = f.head(&workspace);
    let mut no_op = grid.form.clone();
    no_op.name = format!(" {} ", no_op.name);
    no_op.origin = ["0.0".into(), "0.00".into()];
    no_op.rotation = "0.0".into();
    assert!(
        no_op
            .prepare_update(&grid.form, &created.id, &workspace, Some(ACTOR))
            .is_err()
    );
    let mut edited = grid.form.clone();
    edited.name = "Edited grid".into();
    edited.u.expression = "0 2*6500".into();
    edited.u.labels = "1 2".into();
    edited.v.labels = "A B C D E".into();
    edited.u.span = ["1000".into(), "14000".into()];
    edited.origin = ["1000".into(), "-250".into()];
    edited.rotation = "30".into();
    let args = edited
        .prepare_update(&grid.form, &created.id, &workspace, Some(ACTOR))
        .unwrap();
    assert_eq!(
        before,
        f.head(&workspace),
        "editing and cancelling a form must not publish"
    );
    let updated = beamcraft_ui::grid::commit(&f.cli, &args, &Stats::default()).unwrap();
    assert_eq!(updated.id, created.id);
    let after = f.ok(&["list-grids", "--store", workspace.to_str().unwrap()]);
    let old = &initial.get("grids").unwrap().array().unwrap()[0];
    let new = &after.get("grids").unwrap().array().unwrap()[0];
    assert_eq!(new.get("name").unwrap().string().unwrap(), "Edited grid");
    assert_eq!(
        at(new, &["u", "span_mm"])
            .array()
            .unwrap()
            .iter()
            .map(|n| n.float().unwrap())
            .collect::<Vec<_>>(),
        [-1000.0, 14000.0]
    );
    for family in ["u", "v"] {
        for (a, b) in old
            .get(family)
            .unwrap()
            .get("lines")
            .unwrap()
            .array()
            .unwrap()
            .iter()
            .zip(
                new.get(family)
                    .unwrap()
                    .get("lines")
                    .unwrap()
                    .array()
                    .unwrap(),
            )
        {
            assert_eq!(a.get("axis_id"), b.get("axis_id"));
            assert_eq!(a.get("label"), b.get("label"));
        }
    }
    let reopened = source::reload(&f.cli, &workspace, &Known::default(), &Stats::default())
        .grids
        .unwrap();
    let saved = &reopened.get(&created.id).unwrap().form;
    assert_eq!(saved.u.expression, "0 2*6500");
    assert_eq!(saved.u.labels, "1 2 3");
    assert_eq!(saved.u.span, ["1000", "14000"]);
    assert_eq!(saved.origin, ["1000", "-250"]);
    assert!((saved.rotation.parse::<f64>().unwrap() - 30.0).abs() < 1e-10);
    f.ok(&[
        "undo-workspace",
        "--workspace-dir",
        workspace.to_str().unwrap(),
    ]);
    assert_eq!(
        initial.get("grids"),
        f.ok(&["list-grids", "--store", workspace.to_str().unwrap()])
            .get("grids")
    );
    f.ok(&[
        "redo-workspace",
        "--workspace-dir",
        workspace.to_str().unwrap(),
    ]);
    assert_eq!(
        after.get("grids"),
        f.ok(&["list-grids", "--store", workspace.to_str().unwrap()])
            .get("grids")
    );
    // The UI's direct Delete command removes the selected grid in one undo unit.
    let mut editor = beamcraft_ui::edit::Editor::default();
    editor.select_id(created.id.clone());
    let deletion = editor
        .prepare(
            beamcraft_ui::edit::EditKind::Delete,
            beamcraft_ui::edit::Target {
                store: &workspace,
                workspace: Some(&workspace),
            },
        )
        .unwrap();
    beamcraft_ui::command::commit_edit(&f.cli, &deletion, &Stats::default()).unwrap();
    assert!(
        source::reload(&f.cli, &workspace, &Known::default(), &Stats::default())
            .grids
            .unwrap()
            .grids
            .is_empty()
    );
    f.ok(&[
        "undo-workspace",
        "--workspace-dir",
        workspace.to_str().unwrap(),
    ]);
    assert_eq!(
        after.get("grids"),
        f.ok(&["list-grids", "--store", workspace.to_str().unwrap()])
            .get("grids")
    );
}

#[test]
fn inserted_grid_matches_the_cli_and_is_displayed_from_one_pinned_read() {
    for (u_labels, v_labels) in [("1 2 3 4 5 6 7 8 9", "A B C D E F"), ("1 2", "A B")] {
        let f = Fixture::new();
        let store = f.root.join("ui");
        let other = f.root.join("script");
        let mut form = GridForm {
            name: "Frame grid".into(),
            origin: ["1250".into(), "-750".into()],
            rotation: "20".into(),
            ..GridForm::default()
        };
        form.u.labels = u_labels.into();
        form.v.labels = v_labels.into();
        // Filling and validating the form is read-only; cancelling before insertion writes nothing.
        let before = f.head(&store);
        let args = form.prepare(&store, Some(ACTOR)).unwrap();
        assert_eq!(f.head(&store), before);
        let created = beamcraft_ui::grid::commit(&f.cli, &args, &Stats::default()).unwrap();
        f.ok(&[
            "create-grid",
            "--store",
            other.to_str().unwrap(),
            "--name",
            "Frame grid",
            "--origin",
            "1250,-750",
            "--rotation",
            "20",
            "--u-expression",
            "0 2*4000",
            "--v-expression",
            "0 2*5000",
            "--u-labels",
            u_labels,
            "--v-labels",
            v_labels,
            "--actor-id",
            ACTOR,
        ]);
        let listed = f.ok(&["list-grids", "--store", store.to_str().unwrap()]);
        let script = f.ok(&["list-grids", "--store", other.to_str().unwrap()]);
        for family in ["u", "v"] {
            let left = at(&listed, &["grids"]).array().unwrap()[0]
                .get(family)
                .unwrap()
                .get("lines")
                .unwrap()
                .array()
                .unwrap();
            let right = at(&script, &["grids"]).array().unwrap()[0]
                .get(family)
                .unwrap()
                .get("lines")
                .unwrap()
                .array()
                .unwrap();
            assert_eq!(left.len(), 3);
            let expected = if family == "u" {
                ["1", "2", "3"]
            } else {
                ["A", "B", "C"]
            };
            for (line, label) in left.iter().zip(expected) {
                assert_eq!(line.get("label").unwrap().string().unwrap(), label);
            }
            for (a, b) in left.iter().zip(right) {
                for key in ["label", "start_mm", "end_mm"] {
                    assert_eq!(a.get(key), b.get(key));
                }
            }
        }
        let stats = Stats::default();
        let refreshed = source::reload(&f.cli, &store, &Known::default(), &stats);
        assert_eq!(stats.cli_runs.get(), 1);
        let guides = refreshed.grids.unwrap();
        assert_eq!(guides.lines.len(), 6);
        let Refreshed::New(scene) = refreshed.scene.unwrap() else {
            panic!("expected a first scene");
        };
        assert_eq!(
            scene.0.pin().revision().to_string(),
            created.revision.manifest_digest
        );
        let summary = scene.1.summary();
        let (target, extent) = guides.framing(&summary);
        let view = Camera::new(target, extent, 1.5).unwrap().view(1.5).unwrap();
        for line in &guides.lines {
            for point in line.points {
                let local = std::array::from_fn(|i| (point[i] - summary.origin_mm[i]) as f32);
                let [x, y, z] = view.project(local);
                assert!(x.abs() <= 1.0 && y.abs() <= 1.0 && (0.0..=1.0).contains(&z));
            }
        }
        assert_eq!(
            f.head(&store).get("revision_id").unwrap().string().unwrap(),
            created.revision.revision_id
        );
    }
}

#[test]
fn growing_both_coordinate_families_keeps_every_line_across_the_whole_grid() {
    let f = Fixture::new();
    let store = f.root.join("ui");
    let created = beamcraft_ui::grid::commit(
        &f.cli,
        &GridForm::default().prepare(&store, Some(ACTOR)).unwrap(),
        &Stats::default(),
    )
    .unwrap();
    let initial = source::reload(&f.cli, &store, &Known::default(), &Stats::default())
        .grids
        .unwrap();
    let original = &initial.get(&created.id).unwrap().form;
    let mut larger = original.clone();
    larger.u.expression = "0 5*6000".into();
    larger.v.expression = "0 9*5000".into();
    let args = larger
        .prepare_update(original, &created.id, &store, Some(ACTOR))
        .unwrap();
    assert!(args.windows(2).any(|pair| pair == ["--fit-spans", "true"]));
    beamcraft_ui::grid::commit(&f.cli, &args, &Stats::default()).unwrap();
    let enlarged = source::reload(&f.cli, &store, &Known::default(), &Stats::default())
        .grids
        .unwrap();
    let grid = enlarged.get(&created.id).unwrap();
    assert_eq!(grid.form.u.span, ["2000", "47000"]);
    assert_eq!(grid.form.v.span, ["2000", "32000"]);
    assert_eq!(grid.form.u.labels, "1 2 3 4 5 6");
    assert_eq!(grid.form.v.labels, "A B C D E F G H I J");
    let (vertical, horizontal) = enlarged.lines.split_at(6);
    assert_eq!(horizontal.len(), 10);
    for u in vertical {
        for v in horizontal {
            assert!(u.points[0][0] >= v.points[0][0] && u.points[0][0] <= v.points[1][0]);
            assert!(v.points[0][1] >= u.points[0][1] && v.points[0][1] <= u.points[1][1]);
        }
    }
    // The panel's repair action fixes short stored spans without changing axes or labels.
    f.ok(&[
        "update-grid",
        "--store",
        store.to_str().unwrap(),
        "--id",
        &created.id,
        "--u-span",
        "0,1000",
        "--v-span",
        "0,1000",
    ]);
    let fit = beamcraft_ui::grid::fit_args(&store, &created.id, Some(ACTOR)).unwrap();
    beamcraft_ui::grid::commit(&f.cli, &fit, &Stats::default()).unwrap();
    let repaired = source::reload(&f.cli, &store, &Known::default(), &Stats::default())
        .grids
        .unwrap();
    assert_eq!(enlarged, repaired);
}

#[test]
#[ignore = "the window still runs a removed CLI command, see #763"]
fn edited_rotated_rectangle_uses_master_queries_and_preserves_placed_parts() {
    let f = Fixture::new();
    let store = f.root.join("ui");
    let path = store.to_str().unwrap();
    let form = GridForm {
        origin: ["1250".into(), "-750".into()],
        rotation: "30".into(),
        ..GridForm::default()
    };
    let created = beamcraft_ui::grid::commit(
        &f.cli,
        &form.prepare(&store, Some(ACTOR)).unwrap(),
        &Stats::default(),
    )
    .unwrap();
    let address = format!("{}:2,B", created.id);
    f.ok(&[
        "create-plate",
        "--store",
        path,
        "--width",
        "100",
        "--length",
        "200",
        "--thickness",
        "10",
        "--origin-at-grid",
        &address,
        "--z",
        "3500",
    ]);
    let before = f.ok(&["inspect-elements", "--store", path]);
    let initial = source::reload(&f.cli, &store, &Known::default(), &Stats::default())
        .grids
        .unwrap();
    let original = &initial.get(&created.id).unwrap().form;
    let mut edited = original.clone();
    edited.u.expression = "-3000 5*6000".into();
    edited.v.expression = "-5000 9*5000".into();
    edited.u.labels = "1".into();
    edited.v.labels = "A".into();
    beamcraft_ui::grid::commit(
        &f.cli,
        &edited
            .prepare_update(original, &created.id, &store, Some(ACTOR))
            .unwrap(),
        &Stats::default(),
    )
    .unwrap();
    let after = f.ok(&["inspect-elements", "--store", path]);
    assert_eq!(before.get("plates"), after.get("plates"));
    let head = f.head(&store);
    // Every crossing, including newly added outer axes, must resolve and snap in world XY.
    let angle = 30.0_f64.to_radians();
    for (u, x) in [("1", -3000.0), ("6", 27000.0)] {
        for (v, y) in [("A", -5000.0), ("J", 40000.0)] {
            let labels = format!("{u},{v}");
            let resolved = f.ok(&[
                "resolve-grid-point",
                "--store",
                path,
                "--grid",
                &created.id,
                "--labels",
                &labels,
            ]);
            let xy = resolved.get("point_world_mm").unwrap().array().unwrap();
            let expected = [
                1250.0 + x * angle.cos() - y * angle.sin(),
                -750.0 + x * angle.sin() + y * angle.cos(),
            ];
            for (value, expected) in xy.iter().zip(expected) {
                assert!((value.float().unwrap() - expected).abs() < 1e-7);
            }
            let point = format!("{},{},3500", xy[0].float().unwrap(), xy[1].float().unwrap());
            let snapped = f.ok(&[
                "snap",
                "--store",
                path,
                "--point",
                &point,
                "--radius",
                "0",
                "--kinds",
                "grid_intersection",
                "--grids",
                &created.id,
            ]);
            let hits = snapped.get("candidates").unwrap().array().unwrap();
            assert_eq!(hits.len(), 1, "{snapped:?}");
            assert_eq!(
                at(&hits[0], &["grid", "grid_id"]).string().unwrap(),
                created.id
            );
            assert_eq!(
                at(&hits[0], &["point_world_mm"]).array().unwrap()[2]
                    .float()
                    .unwrap(),
                3500.0
            );
        }
    }
    assert_eq!(head, f.head(&store), "grid queries must not publish edits");
}

#[test]
fn invalid_input_and_engine_refusals_publish_no_grid() {
    let f = Fixture::new();
    let store = f.root.join("ui");
    let before = f.head(&store);
    let mut form = GridForm::default();
    for expression in ["NaN 4000", "0 0", "0 201*1", "0 -1"] {
        form.u.expression = expression.into();
        assert!(form.prepare(&store, None).is_err());
    }
    form = GridForm::default();
    form.u.labels = "1 2".into();
    form.v.labels = "A B".into();
    assert!(form.prepare(&store, None).is_ok());
    form.u.labels = ", ,".into();
    let missing = form.prepare(&store, None).unwrap_err();
    assert_eq!(missing.field, "U labels");
    assert!(missing.message.contains("starting label"));
    form.u.labels = "1, 2, 3".into();
    form.v.labels = "A B C".into();
    assert!(form.prepare(&store, None).is_ok());
    form.v.labels = "A A C".into();
    let duplicate = form.prepare(&store, None).unwrap_err();
    assert_eq!(duplicate.field, "V labels");
    assert!(duplicate.message.contains("unique"));
    assert_eq!(f.head(&store), before);
    form = GridForm::default();
    form.u.labels = "A B C".into();
    form.v.labels = "A B C".into();
    let args = form.prepare(&store, Some(ACTOR)).unwrap();
    let refused = beamcraft_ui::grid::commit(&f.cli, &args, &Stats::default()).unwrap_err();
    assert_eq!(refused.code, "INVALID_ARGUMENT");
    assert!(refused.message.contains("unique"));
    assert!(!refused.message.contains("GRID_LABEL_DUPLICATE"));
    assert_eq!(f.head(&store), before);
    assert!(GridScene::from_reply(br#"{"outcome":"grids","grids":[{"u":{"lines":[{"label":"A","start_mm":[0,1],"end_mm":[0,1,0]}]},"v":{"lines":[]}}]}"#).is_err());
}

#[test]
fn drawn_line_forms_round_trip_through_the_real_cli_and_engine_snaps() {
    use beamcraft_engine::query::pick::{SnapKind, SnapQuery, snap_with_grids};
    use beamcraft_ui::grid::{DrawLineForm, commit};
    let f = Fixture::new();
    let store = f.root.join("ui");
    let stats = Stats::default();
    let form = GridForm {
        rotation: "12.3456789".into(),
        origin: ["1000".into(), "2000".into()],
        ..GridForm::default()
    };
    let mut line = DrawLineForm {
        label: "D1".into(),
        points: [["0".into(), "0".into()], ["3000".into(), "4000".into()]],
        ..DrawLineForm::default()
    };
    let before = f.head(&store);
    let args = line.prepare(None, &form, &store, Some(ACTOR)).unwrap();
    // Preparing and abandoning a draft publishes nothing.
    assert_eq!(before, f.head(&store));
    let created = commit(&f.cli, &args, &stats).unwrap();
    let id = created.id;
    let properties = source::reload(&f.cli, &store, &Known::default(), &stats)
        .grids
        .unwrap();
    let original = &properties.get(&id).unwrap().form;
    let mut renamed_form = original.clone();
    renamed_form.name = "Renamed drawn grid".into();
    commit(
        &f.cli,
        &renamed_form
            .prepare_properties(original, &id, &store, Some(ACTOR))
            .unwrap(),
        &stats,
    )
    .unwrap();
    let after = f.ok(&["list-grids", "--store", store.to_str().unwrap()]);
    let rotation = after.get("grids").unwrap().array().unwrap()[0]
        .get("rotation_rad")
        .unwrap()
        .float()
        .unwrap();
    assert!((rotation.to_degrees() - 12.3456789).abs() < 1e-10);
    line.family = "v".into();
    line.label = "D2".into();
    line.points = [["3000".into(), "0".into()], ["0".into(), "4000".into()]];
    commit(
        &f.cli,
        &line.prepare(Some(&id), &form, &store, Some(ACTOR)).unwrap(),
        &stats,
    )
    .unwrap();
    let inspect = || f.ok(&["list-grids", "--store", store.to_str().unwrap()]);
    let initial = inspect();
    // The additive local endpoints must agree with what the user sees.
    fn corrupt_local(json: &mut Json) {
        match json {
            Json::Object(fields) => {
                if fields.contains_key("start_local_mm") {
                    fields.insert(
                        "start_local_mm".into(),
                        Json::Array(vec![Json::uint(999), Json::uint(999)]),
                    );
                }
                for value in fields.values_mut() {
                    corrupt_local(value);
                }
            }
            Json::Array(values) => {
                for value in values {
                    corrupt_local(value);
                }
            }
            _ => {}
        }
    }
    let mut corrupt = initial.clone();
    corrupt_local(&mut corrupt);
    assert!(GridScene::from_reply(&corrupt.encode(&Limits::DEFAULT).unwrap()).is_err());
    let v = &at(&initial, &["grids"]).array().unwrap()[0]
        .get("v")
        .unwrap()
        .get("lines")
        .unwrap()
        .array()
        .unwrap()[0];
    line.axis = v.get("axis_id").unwrap().string().unwrap().into();
    let read = source::reload(&f.cli, &store, &Known::default(), &stats);
    let guides = read.grids.unwrap();
    assert_eq!(guides.query_grids.len(), 1);
    assert!(!guides.get(&id).unwrap().editable);
    let Refreshed::New(scene) = read.scene.unwrap() else {
        panic!("expected snapshot")
    };
    let head = f.head(&store);
    let query = SnapQuery {
        point_mm: [1503.0, 2002.0, 3000.0],
        radius_mm: 10.0,
        kinds: vec![SnapKind::GridIntersection],
        max_hits: 10,
    };
    let hit = snap_with_grids(&scene.0, &query, &guides.query_grids, None).unwrap();
    let point = hit.candidates[0].point_world_mm;
    assert!((point[0] - 1500.0).abs() < 1e-6 && (point[1] - 2000.0).abs() < 1e-6);
    assert_eq!(point[2], 3000.0);
    let hidden = std::collections::BTreeSet::new();
    assert!(
        snap_with_grids(&scene.0, &query, &guides.query_grids, Some(&hidden))
            .unwrap()
            .candidates
            .is_empty()
    );
    assert_eq!(head, f.head(&store));
    line.action = "reshape-grid-line".into();
    line.points = [["3000".into(), "0".into()], ["3000".into(), "4000".into()]];
    commit(
        &f.cli,
        &line.prepare(Some(&id), &form, &store, Some(ACTOR)).unwrap(),
        &stats,
    )
    .unwrap();
    line.action = "relabel-grid-line".into();
    line.label = "Б/1".into();
    commit(
        &f.cli,
        &line.prepare(Some(&id), &form, &store, Some(ACTOR)).unwrap(),
        &stats,
    )
    .unwrap();
    let renamed = inspect();
    let v = &at(&renamed, &["grids"]).array().unwrap()[0]
        .get("v")
        .unwrap()
        .get("lines")
        .unwrap()
        .array()
        .unwrap()[0];
    assert_eq!(v.get("axis_id").unwrap().string().unwrap(), line.axis);
    assert_eq!(v.get("label").unwrap().string().unwrap(), "Б/1");
    line.label = "D1".into();
    let before = f.head(&store);
    let refused = commit(
        &f.cli,
        &line.prepare(Some(&id), &form, &store, Some(ACTOR)).unwrap(),
        &stats,
    )
    .unwrap_err();
    assert_eq!(refused.code, "INVALID_ARGUMENT");
    assert_eq!(before, f.head(&store));
    line.action = "remove-grid-line".into();
    commit(
        &f.cli,
        &line.prepare(Some(&id), &form, &store, Some(ACTOR)).unwrap(),
        &stats,
    )
    .unwrap();
    let removed = inspect();
    assert!(
        at(&removed, &["grids"]).array().unwrap()[0]
            .get("v")
            .unwrap()
            .get("lines")
            .unwrap()
            .array()
            .unwrap()
            .is_empty()
    );
    // New data remains available after a fresh pinned reload.
    assert_eq!(
        source::reload(&f.cli, &store, &Known::default(), &stats)
            .grids
            .unwrap()
            .lines
            .len(),
        1
    );
}
