// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, beams and attributes, MPL-2.0.

//! I05 end to end: `schedule-export` through the real CLI binary against a verified store.
//! A schedule keeps unknown and stale values visible — an element without a value stays a
//! row with an empty cell counted next to its total, and a column naming an unknown
//! attribute stays in the schedule and is reported — and the same head plus the same
//! template render byte-identical files.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support;
use beamcraft_json::Json;
use std::fs;
use support::StoreFixture;

fn ok(f: &StoreFixture, c: &str, a: &[&str]) -> Json {
    let (code, j) = f.run(c, a);
    assert_eq!(code, 0, "{c} {a:?}: {j:?}");
    j
}
fn text(j: &Json, k: &str) -> String {
    j.get(k).unwrap().string().unwrap().to_owned()
}
fn num(j: &Json, k: &str) -> u64 {
    j.get(k).unwrap().unsigned().unwrap()
}
fn at<'a>(j: &'a Json, path: &[&str]) -> &'a Json {
    path.iter().fold(j, |j, k| j.get(k).unwrap())
}
fn rejected(f: &StoreFixture, c: &str, a: &[&str]) -> String {
    let before = text(&ok(f, "open-revision", &[]), "revision_id");
    let (code, j) = f.run(c, a);
    assert_eq!(code, 2, "{c} {a:?}: {j:?}");
    assert_eq!(text(&ok(f, "open-revision", &[]), "revision_id"), before);
    text(&j, "code")
}
/// Writes a template file under the fixture root and returns its path.
fn template(f: &StoreFixture, name: &str, body: &str) -> String {
    let path = f.root.join(name);
    fs::write(&path, body).unwrap();
    path.to_str().unwrap().to_owned()
}
fn out_of(f: &StoreFixture, name: &str) -> String {
    f.root.join(name).to_str().unwrap().to_owned()
}
/// The lines of an output file.
fn lines(f: &StoreFixture, name: &str) -> Vec<String> {
    fs::read_to_string(f.root.join(name))
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}
/// The one output row whose first cell is `key`, with its remaining cells.
fn row(f: &StoreFixture, name: &str, key: &str) -> Vec<String> {
    lines(f, name)
        .iter()
        .skip(1)
        .find(|l| l.split(',').next() == Some(key))
        .expect("scheduled row is present")
        .split(',')
        .map(str::to_owned)
        .collect()
}
fn def(f: &StoreFixture, name: &str, ty: &str, extra: &[&str]) {
    let mut a = vec!["--name", name, "--type", ty];
    a.extend_from_slice(extra);
    ok(f, "attribute-define", &a);
}
fn set(f: &StoreFixture, element: &str, name: &str, value: &str) {
    ok(
        f,
        "attribute-set",
        &["--element", element, "--name", name, "--value", value],
    );
}
/// The id of one created element: the result key is the command's subject.
fn element(f: &StoreFixture, kind: &str, a: &[&str]) -> String {
    let subject = kind.strip_prefix("create-").unwrap_or(kind);
    text(at(&ok(f, kind, a), &[subject]), "element_id")
}
/// The digest of one created definition.
fn digest(f: &StoreFixture, a: &[&str]) -> String {
    text(at(&ok(f, "define-profile", a), &["definition"]), "digest")
}

/// The model of the evidence: a graded material, a graded plate with values, a bare plate
/// without any attribute or material, a graded beam with values, and a bare column without
/// a material. The definitions cover choice, exact integer and unitled number.
fn model(f: &StoreFixture) -> (String, String, String, String) {
    ok(
        f,
        "define-material",
        &["--grade", "S355", "--density", "7850"],
    );
    let plate = element(
        f,
        "create-plate",
        &[
            "--width",
            "1000",
            "--length",
            "500",
            "--thickness",
            "10",
            "--material",
            "S355",
        ],
    );
    let bare = element(
        f,
        "create-plate",
        &["--width", "800", "--length", "400", "--thickness", "10"],
    );
    let profile = digest(
        f,
        &[
            "--width",
            "100",
            "--height",
            "50",
            "--designation",
            "RECT100x50",
        ],
    );
    let beam = element(
        f,
        "create-member",
        &[
            "--start",
            "0,0,0",
            "--end",
            "3000,0,0",
            "--profile-digest",
            &profile,
            "--material",
            "S355",
        ],
    );
    let iprofile = digest(
        f,
        &[
            "--family",
            "i_shape",
            "--h",
            "300",
            "--b",
            "150",
            "--tw",
            "7",
            "--tf",
            "10",
            "--r",
            "15",
            "--designation",
            "SYNI300",
        ],
    );
    let column = element(
        f,
        "create-member",
        &[
            "--kind",
            "column",
            "--start",
            "0,0,0",
            "--end",
            "0,0,2000",
            "--profile-digest",
            &iprofile,
        ],
    );
    def(f, "Paint_status", "choice", &["--options", "New,Done"]);
    def(f, "Count", "integer", &[]);
    def(f, "Weight", "number", &["--unit", "kg"]);
    set(f, &plate, "Paint_status", "New");
    set(f, &plate, "Count", "2");
    set(f, &beam, "Count", "3");
    set(f, &beam, "Weight", "47.1");
    (plate, bare, beam, column)
}

fn export(f: &StoreFixture, t: &str, out: &str, extra: &[&str]) -> Json {
    let mut a = vec!["--template", t, "--out", out];
    a.extend_from_slice(extra);
    ok(f, "schedule-export", &a)
}
/// The evidence template: five columns over both column kinds, grouped, sorted, totalled.
fn full_template(f: &StoreFixture) -> String {
    template(
        f,
        "full.json",
        r#"{
            "version": 1,
            "columns": [
                {"column": "mark"},
                {"column": "profile"},
                {"attribute": "Paint_status"},
                {"attribute": "Count", "total": true},
                {"attribute": "Weight", "total": true}
            ],
            "group_by": ["profile"],
            "sort_by": ["mark"]
        }"#,
    )
}

#[test]
fn schedules_show_values_group_and_total_reproducibly() {
    let f = StoreFixture::new("i05-schedules");
    let (plate, bare, beam, column) = model(&f);
    let t = full_template(&f);
    let first = export(
        &f,
        &t,
        &out_of(&f, "elements.csv"),
        &[
            "--summary",
            &out_of(&f, "groups.csv"),
            "--html",
            &out_of(&f, "page.html"),
        ],
    );
    assert_eq!(text(&first, "outcome"), "schedule_exported");
    assert_eq!(num(&first, "elements"), 4);
    assert_eq!(num(&first, "groups"), 3);
    assert_eq!(
        first.get("unknown_columns").unwrap().array().unwrap().len(),
        0
    );
    // The element table carries the template's labels as its header and one row per element,
    // wherever the mark sort puts it; a value the element lacks is an empty cell.
    assert_eq!(
        lines(&f, "elements.csv")[0],
        "mark,profile,Paint_status,Count,Weight"
    );
    assert_eq!(
        row(&f, "elements.csv", &plate).join(","),
        format!("{plate},PL10,New,2,")
    );
    assert_eq!(
        row(&f, "elements.csv", &bare).join(","),
        format!("{bare},PL10,,,")
    );
    assert_eq!(
        row(&f, "elements.csv", &beam).join(","),
        format!("{beam},RECT100x50,,3,47.1")
    );
    assert_eq!(
        row(&f, "elements.csv", &column).join(","),
        format!("{column},SYNI300,,,"),
    );
    // The group table counts the empty cells of every totalled column next to its sum,
    // never zeroing them.
    assert_eq!(
        lines(&f, "groups.csv")[0],
        "profile,count,Count,Count_unknown,Weight,Weight_unknown"
    );
    assert_eq!(row(&f, "groups.csv", "PL10").join(","), "PL10,2,2,1,,2");
    assert_eq!(
        row(&f, "groups.csv", "RECT100x50").join(","),
        "RECT100x50,1,3,0,47.1,0"
    );
    assert_eq!(
        row(&f, "groups.csv", "SYNI300").join(","),
        "SYNI300,1,,1,,1"
    );
    // The page renders both tables of the same schedule.
    let body = fs::read_to_string(f.root.join("page.html")).unwrap();
    assert!(
        body.contains("<h2>Elements</h2>") && body.contains("<h2>Groups</h2>"),
        "{body}"
    );
    assert!(body.contains("<th>Paint_status</th>"), "{body}");
    // Reproducible: the same head and template render byte-identical files.
    export(
        &f,
        &t,
        &out_of(&f, "again.csv"),
        &[
            "--summary",
            &out_of(&f, "again-groups.csv"),
            "--html",
            &out_of(&f, "again.html"),
        ],
    );
    for (first, again) in [
        ("elements.csv", "again.csv"),
        ("groups.csv", "again-groups.csv"),
        ("page.html", "again.html"),
    ] {
        assert_eq!(
            fs::read(f.root.join(first)).unwrap(),
            fs::read(f.root.join(again)).unwrap()
        );
    }
}

#[test]
fn unknown_attribute_columns_stay_visible_and_filters_narrow() {
    let f = StoreFixture::new("i05-schedules");
    let (plate, ..) = model(&f);
    // A column naming an unknown definition stays in the schedule, renders empty and is
    // named in the result; a total on it refuses because an unknown column has nothing to
    // sum, and a filter naming it refuses.
    let ghost = template(
        &f,
        "ghost.json",
        r#"{"version":1,"columns":[{"column":"mark"},{"attribute":"Ghost"}]}"#,
    );
    let result = export(&f, &ghost, &out_of(&f, "ghost.csv"), &[]);
    let unknown = result.get("unknown_columns").unwrap().array().unwrap();
    assert_eq!(unknown.first().unwrap().string().unwrap(), "Ghost");
    assert_eq!(lines(&f, "ghost.csv").first().unwrap(), "mark,Ghost");
    assert_eq!(row(&f, "ghost.csv", &plate)[1], "");
    let total_ghost = template(
        &f,
        "ghost-total.json",
        r#"{"version":1,"columns":[{"attribute":"Ghost","total":true}]}"#,
    );
    rejected(
        &f,
        "schedule-export",
        &[
            "--template",
            &total_ghost,
            "--out",
            &out_of(&f, "never.csv"),
        ],
    );
    // A filter narrows the rows through the engine's own evaluator; the cells stay.
    let beams = template(
        &f,
        "beams.json",
        r#"{"version":1,"columns":[{"column":"kind"},{"column":"material"}],"filter":{"key":"kind","op":"eq","value":"beam"}}"#,
    );
    let result = export(&f, &beams, &out_of(&f, "beams.csv"), &[]);
    assert_eq!(num(&result, "elements"), 1);
    assert_eq!(lines(&f, "beams.csv").len(), 2);
    assert_eq!(lines(&f, "beams.csv")[1], "beam,S355");
    // A filter on an unknown column, and a literal that does not match its column type,
    // refuse before anything is written.
    for filter in [
        r#"{"key":"Ghost","op":"eq","value":"x"}"#,
        r#"{"key":"mark","op":"ge","value":1}"#,
        r#"{"key":"mark","op":"eq","value":true}"#,
    ] {
        let path = template(
            &f,
            "bad-filter.json",
            &format!(r#"{{"version":1,"columns":[{{"column":"mark"}}],"filter":{filter}}}"#),
        );
        rejected(
            &f,
            "schedule-export",
            &["--template", &path, "--out", &out_of(&f, "never.csv")],
        );
    }
    // An unknown attribute column over a store with no definitions at all: shown, empty.
    let empty = StoreFixture::new("i05-schedules");
    ok(
        &empty,
        "create-plate",
        &["--width", "100", "--length", "50", "--thickness", "10"],
    );
    let no_defs = template(
        &empty,
        "empty.json",
        r#"{"version":1,"columns":[{"column":"kind"},{"attribute":"Paint_status"}]}"#,
    );
    let result = export(&empty, &no_defs, &out_of(&empty, "empty.csv"), &[]);
    let unknown = result.get("unknown_columns").unwrap().array().unwrap();
    assert_eq!(unknown.first().unwrap().string().unwrap(), "Paint_status");
    assert_eq!(lines(&empty, "empty.csv")[1], "plate,");
}

/// A value that outlived its definition's type: the stored integers go stale under a
/// definition that is now a number, so the plate's value shows as a stale, empty, counted
/// cell while the fresh number value shows.
#[test]
fn stale_values_are_shown_and_counted() {
    let f = StoreFixture::new("i05-schedules");
    let (_, _, beam, _) = model(&f);
    ok(&f, "attribute-undefine", &["--name", "Count"]);
    def(&f, "Count", "number", &[]);
    set(&f, &beam, "Count", "3.5");
    let t = template(
        &f,
        "stale.json",
        r#"{
            "version": 1,
            "columns": [{"column": "kind"}, {"attribute": "Count", "total": true}],
            "group_by": ["kind"],
            "sort_by": ["kind"]
        }"#,
    );
    let result = export(
        &f,
        &t,
        &out_of(&f, "stale.csv"),
        &["--summary", &out_of(&f, "stale-groups.csv")],
    );
    // The stale value is not zero and not dropped: the cell is empty while the schedule
    // still names the definition, so the gap is visible in the table itself.
    assert_eq!(lines(&f, "stale.csv").first().unwrap(), "kind,Count");
    assert_eq!(row(&f, "stale.csv", "plate")[1], "");
    assert_eq!(row(&f, "stale.csv", "beam")[1], "3.5");
    // The group table counts the stale cell next to the sum.
    assert_eq!(
        lines(&f, "stale-groups.csv").first().unwrap(),
        "kind,count,Count,Count_unknown"
    );
    assert_eq!(row(&f, "stale-groups.csv", "plate").join(","), "plate,2,,2");
    assert_eq!(
        row(&f, "stale-groups.csv", "beam").join(","),
        "beam,1,3.5,0"
    );
    assert_eq!(
        result
            .get("unknown_columns")
            .unwrap()
            .array()
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn broken_templates_and_paths_refuse_without_writing() {
    let f = StoreFixture::new("i05-schedules");
    model(&f);
    let t = full_template(&f);
    // A store without a revision.
    let empty = StoreFixture::new("i05-schedules");
    let (code, _) = empty.run(
        "schedule-export",
        &["--template", &t, "--out", &out_of(&empty, "x.csv")],
    );
    assert_eq!(code, 2);
    // An unknown standard token, a bad version, and keys naming unknown columns.
    for body in [
        r#"{"version":1,"columns":[{"column":"ghostly"}]}"#,
        r#"{"version":2,"columns":[{"column":"mark"}]}"#,
        r#"{"version":1,"columns":[{"column":"mark"}],"group_by":["profile"]}"#,
        r#"{"version":1,"columns":[{"column":"mark"}],"sort_by":["profile","mark"]}"#,
        r#"{"version":1,"columns":[{"column":"mark","total":true}]}"#,
    ] {
        let path = template(&f, "bad.json", body);
        rejected(
            &f,
            "schedule-export",
            &["--template", &path, "--out", &out_of(&f, "never.csv")],
        );
        assert!(!f.root.join("never.csv").exists());
    }
    // An existing output file is never overwritten, and the run writes nothing else.
    fs::write(f.root.join("exists.csv"), "keep").unwrap();
    rejected(
        &f,
        "schedule-export",
        &[
            "--template",
            &t,
            "--out",
            &out_of(&f, "exists.csv"),
            "--html",
            &out_of(&f, "never.html"),
        ],
    );
    assert_eq!(
        fs::read_to_string(f.root.join("exists.csv")).unwrap(),
        "keep"
    );
    assert!(!f.root.join("never.html").exists());
    // The same path twice is refused by the reservation, leaving nothing behind.
    let same = out_of(&f, "same.csv");
    rejected(
        &f,
        "schedule-export",
        &["--template", &t, "--out", &same, "--summary", &same],
    );
    assert!(!f.root.join("same.csv").exists());
    // A malformed template refuses before the store is opened.
    let broken = template(&f, "broken.json", "{\"version\":1,");
    rejected(
        &f,
        "schedule-export",
        &["--template", &broken, "--out", &out_of(&f, "never.csv")],
    );
}
