// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! T01.1 acceptance through the actual binary, revision store and readable USD layers.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod support;
use beamcraft_host::{
    ids::IdSource,
    revision::{ExpectedBase, LayerEntry, NoFault, RevisionDraft, RevisionStore, Uuid7},
    storage::BlobStore,
};
use beamcraft_json::Json;
const GRID: &str = "01990000-0000-7000-8000-000000000001";
const OP: &str = "01990000-0000-7000-8000-000000000002";
const AXES: &str = "01990000-0000-7000-8000-000000000010,01990000-0000-7000-8000-000000000011";
fn text(j: &Json, key: &str) -> String {
    j.get(key).unwrap().string().unwrap().to_owned()
}
fn list<'a>(j: &'a Json, key: &str) -> &'a [Json] {
    j.get(key).unwrap().array().unwrap()
}
fn ok(f: &support::StoreFixture, command: &str, args: &[&str]) -> Json {
    let (code, j) = f.run(command, args);
    assert_eq!(code, 0, "{command}: {j:?}");
    j
}
fn store(f: &support::StoreFixture) -> RevisionStore {
    RevisionStore::new(BlobStore::new(f.root.join("store")).unwrap())
}
fn head(f: &support::StoreFixture) -> String {
    text(&ok(f, "open-revision", &[]), "revision_id")
}
fn rejected(f: &support::StoreFixture, args: &[&str], expected: &str) {
    let before = head(f);
    let (code, j) = f.run("create-grid", args);
    assert_eq!(code, 2, "{j:?}");
    assert_eq!(text(&j, "code"), expected, "{j:?}");
    assert_eq!(before, head(f));
}
fn create(f: &support::StoreFixture) -> Json {
    ok(
        f,
        "create-grid",
        &[
            "--name",
            "Main",
            "--new-id",
            GRID,
            "--u-expression",
            "0 6000",
            "--u-axis-ids",
            AXES,
            "--op-id",
            OP,
        ],
    )
}

#[test]
fn regular_grids_are_explicit_usd_and_reopen_with_exact_expressions_and_endpoints() {
    let f = support::StoreFixture::new("t01-regular");
    let j = ok(
        &f,
        "create-grid",
        &[
            "--name",
            "Main",
            "--u-expression",
            "0 3*6000",
            "--v-expression",
            "0 7500 4500",
        ],
    );
    let reopened = ok(&f, "list-grids", &[]);
    assert_eq!(j.get("grids"), reopened.get("grids"));
    let grid = &list(&j, "grids")[0];
    let u = grid.get("u").unwrap();
    let v = grid.get("v").unwrap();
    assert_eq!(text(u, "expression"), "0 3*6000");
    assert_eq!(list(u, "lines").len(), 4);
    assert_eq!(list(v, "lines").len(), 3);
    assert_eq!(
        list(u, "lines")[2].get("start_mm").unwrap(),
        &Json::Array(vec![
            Json::number(12000.0).unwrap(),
            Json::number(-2000.0).unwrap(),
            Json::number(0.0).unwrap()
        ])
    );
    let revision = store(&f).open().unwrap();
    let authored = String::from_utf8(revision.layer_bytes("authored.usda").unwrap()).unwrap();
    assert!(authored.contains("GRD_"));
    assert!(authored.contains("bim:grid:u:pointsMm"));
    assert!(!authored.contains("def Mesh"));
    let derived = String::from_utf8(revision.layer_bytes("derived.usda").unwrap()).unwrap();
    assert!(!derived.contains("Geometry"));
    let inspected = ok(&f, "inspect-elements", &[]);
    assert!(format!("{inspected:?}").contains("grid"));
}
#[test]
fn create_replay_is_bound_to_the_accepted_request_even_after_deletion() {
    let f = support::StoreFixture::new("t01-replay");
    create(&f);
    let before = head(&f);
    let j = create(&f);
    assert_eq!(text(&j, "outcome"), "grid_created_replayed");
    assert_eq!(head(&f), before);
    rejected(
        &f,
        &[
            "--name",
            "Other",
            "--new-id",
            GRID,
            "--u-expression",
            "0 6000",
            "--u-axis-ids",
            AXES,
            "--op-id",
            OP,
        ],
        "OP_ID_REUSE",
    );
    ok(&f, "delete-element", &["--id", GRID]);
    assert!(list(&ok(&f, "list-grids", &[]), "grids").is_empty());
    assert_eq!(text(&create(&f), "outcome"), "grid_created_replayed");
}
#[test]
fn copy_allocates_independent_axes_and_delete_preserves_the_source() {
    let f = support::StoreFixture::new("t01-copy");
    create(&f);
    let copied = ok(
        &f,
        "copy-element",
        &["--id", GRID, "--name", "Copy", "--offset", "1000,2000,0"],
    );
    let copy = text(&copied, "element_id");
    let all = ok(&f, "list-grids", &[]);
    let grids = list(&all, "grids");
    assert_eq!(grids.len(), 2);
    let source = grids.iter().find(|g| text(g, "id") == GRID).unwrap();
    let copy_grid = grids.iter().find(|g| text(g, "id") == copy).unwrap();
    let axes = |g: &Json| {
        list(g.get("u").unwrap(), "lines")
            .iter()
            .map(|l| text(l, "axis_id"))
            .collect::<Vec<_>>()
    };
    assert!(axes(source).iter().all(|id| !axes(copy_grid).contains(id)));
    ok(&f, "delete-element", &["--id", &copy]);
    assert_eq!(list(&ok(&f, "list-grids", &[]), "grids").len(), 1);
    assert_eq!(&list(&ok(&f, "list-grids", &[]), "grids")[0], source);
}
#[test]
fn invalid_arguments_refuse_without_publishing() {
    let f = support::StoreFixture::new("t01-negative");
    create(&f);
    for expression in ["0 0*6000", "0 -6000", "0 6000 x", "0 0.5", ""] {
        rejected(
            &f,
            &["--name", "Bad", "--u-expression", expression],
            "INVALID_ARGUMENT",
        );
    }
    rejected(
        &f,
        &["--name", "Bad", "--u-expression", "0 NaN"],
        "NON_FINITE_NUMBER",
    );
    rejected(
        &f,
        &["--name", "Bad", "--u-expression", "0 200*1"],
        "LIMIT_EXCEEDED",
    );
    rejected(
        &f,
        &[
            "--name",
            "Bad",
            "--u-expression",
            "0",
            "--v-expression",
            "0",
            "--u-labels",
            "A",
            "--v-labels",
            "A",
        ],
        "INVALID_ARGUMENT",
    );
    rejected(
        &f,
        &[
            "--name",
            "Bad",
            "--u-expression",
            "0 6000",
            "--u-axis-ids",
            AXES,
        ],
        "DUPLICATE_ID",
    );
    rejected(
        &f,
        &[
            "--name",
            "Bad",
            "--u-expression",
            "0",
            "--origin",
            "1000000001,0",
        ],
        "LIMIT_EXCEEDED",
    );
}
#[test]
fn unicode_rotation_and_family_count_boundaries() {
    let f = support::StoreFixture::new("t01-frame");
    let j = ok(
        &f,
        "create-grid",
        &[
            "--name",
            "Frame",
            "--u-expression",
            "0 6000",
            "--u-labels",
            "А Г-1",
            "--origin",
            "1000,2000",
            "--rotation",
            "90",
        ],
    );
    let g = &list(&j, "grids")[0];
    assert_eq!(text(&list(g.get("u").unwrap(), "lines")[1], "label"), "Г-1");
    let p = list(g.get("u").unwrap(), "lines")[1]
        .get("start_mm")
        .unwrap()
        .array()
        .unwrap();
    let number = |v: &Json| {
        if let Json::Number(s) = v {
            s.parse::<f64>().unwrap()
        } else {
            panic!()
        }
    };
    assert!((number(&p[0]) - 3000.0).abs() < 1e-8);
    assert!((number(&p[1]) - 8000.0).abs() < 1e-8);
    let j = ok(
        &f,
        "create-grid",
        &["--name", "Maximum", "--u-expression", "0 199*1"],
    );
    assert_eq!(
        list(
            list(&j, "grids")
                .iter()
                .find(|g| text(g, "name") == "Maximum")
                .unwrap()
                .get("u")
                .unwrap(),
            "lines"
        )
        .len(),
        200
    );
}
#[test]
fn damaged_grid_arrays_are_rejected_by_the_stored_graph_validator() {
    let f = support::StoreFixture::new("t01-damaged");
    create(&f);
    let store = store(&f);
    let revision = store.open().unwrap();
    let c = revision.manifest().content();
    let mut entries = Vec::new();
    for e in c.entries() {
        let mut bytes = revision.layer_bytes(e.name()).unwrap();
        if e.name() == "authored.usda" {
            let text = String::from_utf8(bytes).unwrap();
            assert!(text.contains("[0.0, -2000.0, 0.0, 2000.0, 6000.0, -2000.0, 6000.0, 2000.0]"));
            bytes = text
                .replace(
                    "[0.0, -2000.0, 0.0, 2000.0, 6000.0, -2000.0, 6000.0, 2000.0]",
                    "[0.0, -2000.0, 0.0, 2000.0]",
                )
                .into_bytes();
        }
        store.blobs().publish(&bytes).unwrap();
        entries.push(LayerEntry::from_bytes(e.name(), e.role(), &bytes).unwrap());
    }
    let mut ids = IdSource::system();
    let draft = RevisionDraft {
        project: c.project().clone(),
        workspace: c.workspace().clone(),
        revision: Uuid7::parse(&ids.next_id().unwrap().to_string()).unwrap(),
        op: Uuid7::parse(&ids.next_id().unwrap().to_string()).unwrap(),
        workspace_generation: c.workspace_generation() + 1,
        schema: c.schema(),
        root_layer: c.root_layer().to_owned(),
        entries,
    };
    store
        .commit_with(
            &draft,
            &ExpectedBase::Revision(revision.info().revision().clone()),
            None,
            &mut NoFault,
        )
        .unwrap();
    let (code, j) = f.run("list-grids", &[]);
    assert_eq!(code, 2, "{j:?}");
    assert_eq!(text(&j, "code"), "INVALID_ARGUMENT");
}

#[test]
fn grid_creation_is_one_workspace_undo_unit_and_survives_part_edits() {
    let f = support::StoreFixture::new("t01-undo");
    ok(
        &f,
        "create-level",
        &["--name", "Ground", "--elevation", "0"],
    );
    let draft = f.root.join("draft").to_str().unwrap().to_owned();
    ok(&f, "create-workspace", &["--workspace-dir", &draft]);
    let run = |command: &str, args: &[&str]| {
        let (code, j) = support::StoreFixture::run_in(&draft, command, args);
        assert_eq!(code, 0, "{command}: {j:?}");
        j
    };
    let created = run(
        "create-grid",
        &["--name", "Draft", "--u-expression", "0 6000"],
    );
    let undo = |command: &str| {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args([command, "--workspace-dir", &draft])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    };
    undo("undo-workspace");
    assert!(list(&run("list-grids", &[]), "grids").is_empty());
    undo("redo-workspace");
    assert_eq!(created.get("grids"), run("list-grids", &[]).get("grids"));
    run(
        "create-plate",
        &["--width", "1000", "--length", "500", "--thickness", "10"],
    );
    assert_eq!(created.get("grids"), run("list-grids", &[]).get("grids"));
}

#[test]
fn grids_are_excluded_from_bom_numbering_and_ifc_output() {
    let f = support::StoreFixture::new("t01-output");
    create(&f);
    let csv = f.root.join("bom.csv").to_str().unwrap().to_owned();
    let bom = ok(&f, "export-bom", &["--out", &csv]);
    assert_eq!(bom.get("elements").unwrap().float().unwrap(), 0.0);
    let ifc = f.root.join("model.ifc").to_str().unwrap().to_owned();
    ok(&f, "export-ifc", &["--out", &ifc]);
    assert!(
        !std::fs::read_to_string(ifc)
            .unwrap()
            .contains("IFCBUILDINGELEMENTPROXY")
    );
    let preview = ok(&f, "preview-numbering", &[]);
    assert!(!format!("{preview:?}").contains(GRID));
}

#[test]
fn exact_copy_retry_does_not_need_the_source_to_still_exist() {
    let f = support::StoreFixture::new("t01-copy-replay");
    create(&f);
    let args = [
        "--id",
        GRID,
        "--new-id",
        "01990000-0000-7000-8000-000000000100",
        "--new-axis-ids",
        "01990000-0000-7000-8000-000000000110,01990000-0000-7000-8000-000000000111",
        "--op-id",
        "01990000-0000-7000-8000-000000000200",
    ];
    ok(&f, "copy-element", &args);
    let before = head(&f);
    assert_eq!(
        text(&ok(&f, "copy-element", &args), "outcome"),
        "element_copy_replayed"
    );
    assert_eq!(head(&f), before);
    ok(&f, "delete-element", &["--id", GRID]);
    assert_eq!(
        text(&ok(&f, "copy-element", &args), "outcome"),
        "element_copy_replayed"
    );
}

#[test]
fn helper_options_and_fixed_grid_diagnostics_are_headless() {
    let f = support::StoreFixture::new("t01-label-schemes");
    let made = ok(
        &f,
        "create-grid",
        &[
            "--name",
            "Labels",
            "--u-expression",
            "0 2*6000",
            "--u-label-start",
            "2",
            "--u-label-step",
            "2",
            "--u-label-order",
            "descending",
        ],
    );
    let g = &list(&made, "grids")[0];
    let labels = list(g.get("u").unwrap(), "lines")
        .iter()
        .map(|l| text(l, "label"))
        .collect::<Vec<_>>();
    assert_eq!(labels, ["6", "4", "2"]);
    let (_, error) = f.run(
        "create-grid",
        &[
            "--name",
            "Bad",
            "--u-expression",
            "0 6000",
            "--u-labels",
            "A A",
        ],
    );
    assert_eq!(
        text(&list(&error, "diagnostics")[0], "reason_code"),
        "GRID_LABEL_DUPLICATE"
    );
    let (_, error) = f.run("create-grid", &["--name", "Bad", "--u-expression", "0 0.5"]);
    assert_eq!(
        text(&list(&error, "diagnostics")[0], "reason_code"),
        "GRID_SPACING_INVALID"
    );
}
