// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, MPL-2.0.

//! V11 end to end: the window's click selection honours the selection switches on a real store
//! built by the `beamcraft` CLI in a separate process. A switch that is off changes only what a
//! click selects: the answer names no hit and asks for no details, the snapshot on screen keeps
//! its token and the accepted revision is untouched.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use beamcraft_engine::query::pick::{SelectionClass, SelectionFilter};
use beamcraft_json::{Json, Limits};
use beamcraft_ui::command;
use beamcraft_ui::edit::{Ray, pick_in};
use beamcraft_ui::source;
use beamcraft_ui::stats::Stats;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
const PLATE: &str = "019a7bf0-0000-7000-8000-0000000b0001";

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
            "build beamcraft-cli before running selection UI acceptance"
        );
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("selection-ui-tests")
            .join(format!(
                "{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        std::fs::create_dir_all(&root).unwrap();
        let f = Self { root, cli };
        f.ok(&[
            "create-project",
            "--parent",
            f.path().to_str().unwrap(),
            "--name",
            "ui",
        ]);
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
    fn head(&self, store: &Path) -> String {
        self.ok(&["open-revision", "--store", store.to_str().unwrap()])
            .get("revision_id")
            .unwrap()
            .string()
            .unwrap()
            .to_owned()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// A plate at (0, 0, 0) and the window's ray from above its centre.
fn fixture_with_plate() -> (Fixture, PathBuf, beamcraft_engine::ViewSnapshot, Ray) {
    let f = Fixture::new();
    let store = f.root.join("ui");
    f.ok(&[
        "create-plate",
        "--store",
        store.to_str().unwrap(),
        "--width",
        "1000",
        "--length",
        "500",
        "--thickness",
        "10",
        "--new-id",
        PLATE,
    ]);
    let snapshot = source::load(&f.cli, &store, &Stats::default()).unwrap();
    let ray = Ray {
        origin: [500.0, 250.0, 1000.0],
        direction: [0.0, 0.0, -1.0],
    };
    (f, store, snapshot, ray)
}

#[test]
fn a_switch_that_is_off_takes_its_class_out_of_the_click_selection_and_writes_nothing() {
    let (f, store, snapshot, ray) = fixture_with_plate();
    let head = f.head(&store);
    let token = snapshot.token().clone();

    // Every switch on: the click selects the plate.
    let on = pick_in(&snapshot, &ray, SelectionFilter::ALL).unwrap();
    assert_eq!(on.as_deref(), Some(PLATE));

    // Parts off: the same ray over the same snapshot selects nothing, and both the snapshot
    // token and the accepted revision stay exactly where they were.
    let parts_off = SelectionFilter::ALL.with(SelectionClass::Parts, false);
    let off = pick_in(&snapshot, &ray, parts_off).unwrap();
    assert_eq!(off, None);
    assert_eq!(snapshot.token(), &token);

    // The full click path (pick, then a details lookup) asks for no lookup at all while the
    // switch is off, and selects as usual with it on.
    let cancel = Arc::new(AtomicBool::new(false));
    let none = command::select(
        &f.cli,
        &store,
        &snapshot,
        &ray,
        &cancel,
        &Stats::default(),
        parts_off,
    )
    .unwrap();
    assert!(none.is_none());
    let picked = command::select(
        &f.cli,
        &store,
        &snapshot,
        &ray,
        &cancel,
        &Stats::default(),
        SelectionFilter::ALL,
    )
    .unwrap();
    assert_eq!(picked.unwrap().id, PLATE);

    assert_eq!(f.head(&store), head, "switches never write");
}

#[test]
fn the_grids_switch_is_part_of_the_same_filter_and_the_defaults_are_all_on() {
    // A switch of a class that has no pickable kind yet excludes nothing: the plate is still
    // selected with bolts, welds, holes, construction objects and items all off.
    let (f, store, snapshot, ray) = fixture_with_plate();
    let mut filter = SelectionFilter::ALL;
    for class in [
        SelectionClass::Bolts,
        SelectionClass::Welds,
        SelectionClass::Holes,
        SelectionClass::ConstructionObjects,
        SelectionClass::Items,
    ] {
        filter = filter.with(class, false);
    }
    let on = pick_in(&snapshot, &ray, filter).unwrap();
    assert_eq!(on.as_deref(), Some(PLATE));
    let _ = (f, store);
}
