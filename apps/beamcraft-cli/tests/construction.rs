// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! T02 acceptance through the real binary: construction objects are stored helper elements,
//! snap targets, and are excluded from schedules and numbering.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
use crate::support::{StoreFixture, at, text};
use beamcraft_json::Json;

const POINT: &str = "01990000-0000-7000-8000-000000000101";
const LINE: &str = "01990000-0000-7000-8000-000000000102";
const CIRCLE: &str = "01990000-0000-7000-8000-000000000103";
const PLANE: &str = "01990000-0000-7000-8000-000000000104";

fn ok(f: &StoreFixture, command: &str, args: &[&str]) -> Json {
    let (code, result) = f.run(command, args);
    assert_eq!(code, 0, "{command}: {result:?}");
    result
}
fn list<'a>(value: &'a Json, key: &str) -> &'a [Json] {
    at(value, &[key]).array().unwrap()
}
/// Builds a store with one plate and one helper of each kind at fixed coordinates.
fn build(f: &StoreFixture) {
    ok(
        f,
        "create-plate",
        &["--width", "1000", "--length", "500", "--thickness", "10"],
    );
    ok(
        f,
        "create-construction",
        &[
            "--name", "Origin", "--kind", "point", "--a", "5000,0,0", "--new-id", POINT,
        ],
    );
    ok(
        f,
        "create-construction",
        &[
            "--name",
            "Ridge",
            "--kind",
            "line",
            "--a",
            "10000,0,0",
            "--b",
            "10000,2000,0",
            "--new-id",
            LINE,
        ],
    );
    ok(
        f,
        "create-construction",
        &[
            "--name",
            "Pad",
            "--kind",
            "circle",
            "--a",
            "20000,0,0",
            "--b",
            "0,0,1",
            "--radius",
            "500",
            "--new-id",
            CIRCLE,
        ],
    );
    ok(
        f,
        "create-construction",
        &[
            "--name",
            "Datum",
            "--kind",
            "plane",
            "--a",
            "30000,0,5000",
            "--b",
            "0,0,1",
            "--new-id",
            PLANE,
        ],
    );
}

#[test]
fn construction_objects_are_stored_helpers_that_reopen_exactly() {
    let f = StoreFixture::new("t02-store");
    build(&f);
    let listed = ok(&f, "list-construction", &[]);
    let objects = list(&listed, "objects");
    assert_eq!(objects.len(), 4, "{listed:?}");
    assert_eq!(text(&objects[0], "id"), POINT);
    assert_eq!(text(&objects[0], "kind"), "point");
    assert_eq!(text(&objects[1], "kind"), "line");
    assert_eq!(
        at(&objects[1], &["b_mm"]).array().unwrap()[1].float(),
        Ok(2000.0)
    );
    assert_eq!(text(&objects[2], "kind"), "circle");
    assert_eq!(at(&objects[2], &["radius_mm"]).float(), Ok(500.0));
    assert_eq!(text(&objects[3], "kind"), "plane");
    // The same identities are refused a second time.
    let (code, body) = f.run(
        "create-construction",
        &[
            "--name", "Again", "--kind", "point", "--a", "1,2,3", "--new-id", POINT,
        ],
    );
    assert_eq!(code, 2, "{body:?}");
    assert_eq!(text(&body, "code"), "DUPLICATE_ID");
}

#[test]
fn schedules_and_numbering_never_see_a_helper_element() {
    let f = StoreFixture::new("t02-excluded");
    build(&f);
    // The schedule has the plate row only; no helper id or name appears anywhere.
    let elements_path = f.root.join("elements.csv").to_str().unwrap().to_owned();
    let summary_path = f.root.join("summary.csv").to_str().unwrap().to_owned();
    let exported = ok(
        &f,
        "export-bom",
        &["--out", &elements_path, "--summary", &summary_path],
    );
    assert_eq!(at(&exported, &["elements"]).float(), Ok(1.0));
    let csv = std::fs::read_to_string(&elements_path).unwrap();
    for helper in [
        POINT, LINE, CIRCLE, PLANE, "Origin", "Ridge", "Pad", "Datum",
    ] {
        assert!(!csv.contains(helper), "{helper} leaked into the schedule");
    }
    // Numbering proposes nothing for a helper element.
    let preview = ok(&f, "preview-numbering", &[]);
    let rendered = format!("{preview:?}");
    for helper in [POINT, LINE, CIRCLE, PLANE] {
        assert!(!rendered.contains(helper), "{helper} leaked into numbering");
    }
}

#[test]
fn a_helper_element_deletes_like_any_other_element() {
    let f = StoreFixture::new("t02-delete");
    build(&f);
    let deleted = ok(&f, "delete-element", &["--id", POINT]);
    assert_eq!(text(&deleted, "kind"), "construction");
    let listed = ok(&f, "list-construction", &[]);
    assert_eq!(list(&listed, "objects").len(), 3);
}
