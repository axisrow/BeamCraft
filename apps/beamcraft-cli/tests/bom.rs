// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, beams and materials, MPL-2.0.

//! E03 end to end: `bom-export` through the real CLI binary against a verified store. The
//! exported per-element and summary CSVs match `plate-inspect`, the mark is the element id
//! (the fallback until numbering exists), a mass without a density is an empty cell excluded
//! from the totals and counted in `mass_unknown`, and the files are created new, never
//! overwritten.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_protocol::{
    json::{self, Json},
    message,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("e03-tests");
        fs::create_dir_all(&base).unwrap();
        let root = base.join(format!(
            "e2e-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("store")).unwrap();
        Self { root }
    }
    fn run(&self, command: &str, extra: &[&str]) -> (i32, Json) {
        let store = self.root.join("store").to_str().unwrap().to_owned();
        let mut args = vec![command, "--store", store.as_str()];
        args.extend_from_slice(extra);
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(&args)
            .output()
            .unwrap();
        let body = json::decode(&output.stdout, &message::read_limits()).unwrap();
        (output.status.code().unwrap(), body)
    }
    fn ok(&self, command: &str, extra: &[&str]) -> Json {
        let (code, body) = self.run(command, extra);
        assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
        body
    }
    fn rejected(&self, command: &str, extra: &[&str]) -> String {
        let before = self.head();
        let (code, body) = self.run(command, extra);
        assert_eq!(code, 2, "{command} {extra:?}: {body:?}");
        assert_eq!(self.head(), before, "{command} {extra:?}");
        text(&body, "code")
    }
    /// A refusal of a store that has no revision yet: there is no head to compare.
    fn rejected_empty(&self, command: &str, extra: &[&str]) -> String {
        let (code, body) = self.run(command, extra);
        assert_eq!(code, 2, "{command} {extra:?}: {body:?}");
        text(&body, "code")
    }
    fn head(&self) -> String {
        text(&self.ok("revision-open", &[]), "revision_id")
    }
    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.root.join(name)).unwrap()
    }
    /// Absolute path of an output file under the fixture root.
    fn out(&self, name: &str) -> String {
        self.root.join(name).to_str().unwrap().to_owned()
    }
    /// The model of the design-note evidence: a graded material, a plate on it, a plate without
    /// any material, a rectangle-section beam on the graded material and an I-section column
    /// without a material.
    fn build_model(&self) -> (String, String, String, String) {
        self.ok("define-material", &["--grade", "S355", "--density", "7850"]);
        // Created first, so its UUIDv7 mark sorts before the other plate's.
        let bare_plate = text(
            at(
                &self.ok(
                    "create-plate",
                    &["--width", "800", "--length", "400", "--thickness", "10"],
                ),
                &["plate"],
            ),
            "element_id",
        );
        let plate = text(
            at(
                &self.ok(
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
                ),
                &["plate"],
            ),
            "element_id",
        );
        let profile = text(
            at(
                &self.ok(
                    "define-profile",
                    &[
                        "--width",
                        "100",
                        "--height",
                        "50",
                        "--designation",
                        "RECT100x50",
                    ],
                ),
                &["definition"],
            ),
            "digest",
        );
        let beam = text(
            at(
                &self.ok(
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
                ),
                &["member"],
            ),
            "element_id",
        );
        let iprofile = text(
            at(
                &self.ok(
                    "define-profile",
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
                ),
                &["definition"],
            ),
            "digest",
        );
        let column = text(
            at(
                &self.ok(
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
                ),
                &["member"],
            ),
            "element_id",
        );
        // A09: the beam is numbered, the column is not, so the report shows both mark sources.
        self.ok("number-assign", &["--id", &beam, "--mark", "B1"]);
        (plate, bare_plate, beam, column)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(self.root.is_absolute() && self.root.file_name().is_some());
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn text(json: &Json, key: &str) -> String {
    json.get(key).unwrap().string().unwrap().to_owned()
}
fn at<'a>(json: &'a Json, path: &[&str]) -> &'a Json {
    path.iter().fold(json, |j, k| j.get(k).unwrap())
}
fn list<'a>(json: &'a Json, key: &str) -> &'a [Json] {
    at(json, &[key]).array().unwrap()
}

/// Splits one CSV line into its cells; the fixture uses no quoting.
fn cells(line: &str) -> Vec<&str> {
    line.split(',').collect()
}

#[test]
fn the_exported_tables_match_the_verified_revision() {
    let f = Fixture::new();
    let (plate, bare_plate, beam, column) = f.build_model();

    // The quantities the CSV must repeat come from the verified read of the same head.
    let inspected = f.ok("plate-inspect", &[]);
    let masses: Vec<(String, f64)> = list(&inspected, "plates")
        .iter()
        .chain(list(&inspected, "members").iter())
        .map(|e| {
            (
                text(e, "element_id"),
                at(e, &["quantities", "mass", "kg"])
                    .float()
                    .unwrap_or(f64::NAN),
            )
        })
        .collect();
    let mass_of = |id: &str| {
        masses
            .iter()
            .find(|(element, _)| element == id)
            .map(|(_, m)| *m)
            .unwrap_or(f64::NAN)
    };

    let elements_path = f.out("elements.csv");
    let summary_path = f.out("summary.csv");
    let exported = f.ok(
        "bom-export",
        &["--out", &elements_path, "--summary", &summary_path],
    );
    assert_eq!(text(&exported, "outcome"), "bom_exported");
    assert_eq!(at(&exported, &["elements"]).float(), Ok(4.0));
    assert_eq!(at(&exported, &["groups"]).float(), Ok(4.0));
    assert_eq!(text(&exported, "elements_file"), elements_path);
    assert_eq!(text(&exported, "summary_file"), summary_path);

    let elements = f.read("elements.csv");
    let lines: Vec<&str> = elements.lines().collect();
    assert_eq!(lines.len(), 5, "{elements}");
    assert_eq!(
        lines[0],
        "mark,element_id,kind,name,profile,material,length_mm,area_mm2,volume_mm3,volume_m3,mass_kg"
    );
    // Report order: plates, then beams, then columns; within a kind by profile, then mark.
    let rows: Vec<Vec<&str>> = lines[1..].iter().map(|l| cells(l)).collect();
    assert_eq!(rows[0][2], "plate");
    assert_eq!(rows[0][0], bare_plate, "mark falls back to the element id");
    assert_eq!(rows[0][0], rows[0][1]);
    assert_eq!(
        rows[0][5], "",
        "a plate without a material has no material cell"
    );
    assert_eq!(rows[0][6], "", "a plate has no length cell");
    assert_eq!(rows[0][7], "320000", "net face area = volume / thickness");
    assert_eq!(
        rows[0][10], "",
        "an unknown mass is an empty cell, never zero"
    );
    assert_eq!(rows[1][1], plate);
    assert_eq!(rows[1][5], "S355");
    assert_eq!(rows[1][7], "500000");
    assert_eq!(
        rows[1][10],
        format!("{}", mass_of(&plate)),
        "the CSV mass is the verified mass"
    );
    assert_eq!(rows[2][2], "beam");
    assert_eq!(rows[2][0], "B1", "an assigned mark labels the element");
    assert_eq!(rows[2][1], beam);
    assert_eq!(
        rows[2][4], "RECT100x50",
        "the designation labels the profile"
    );
    assert_eq!(rows[2][6], "3000");
    assert_eq!(rows[2][7], "5000", "net section area = volume / length");
    assert_eq!(rows[2][10], format!("{}", mass_of(&beam)));
    assert_eq!(rows[3][2], "column");
    assert_eq!(rows[3][1], column);
    assert_eq!(rows[3][0], column);
    assert_eq!(rows[3][4], "SYNI300");
    assert_eq!(rows[3][5], "");
    assert_eq!(rows[3][10], "");

    let summary = f.read("summary.csv");
    let lines: Vec<&str> = summary.lines().collect();
    assert_eq!(lines.len(), 5, "{summary}");
    assert_eq!(
        lines[0],
        "kind,profile,material,count,total_length_mm,total_area_m2,total_volume_m3,total_mass_kg,mass_unknown"
    );
    let groups: Vec<Vec<&str>> = lines[1..].iter().map(|l| cells(l)).collect();
    assert_eq!(groups[0][..3], ["plate", "PL10", ""]);
    assert_eq!(groups[0][3], "1");
    assert_eq!(groups[0][4], "", "plates have no total length");
    assert_eq!(groups[0][7], "");
    assert_eq!(
        groups[0][8], "1",
        "the bare plate is counted as unknown mass"
    );
    assert_eq!(groups[1][..3], ["plate", "PL10", "S355"]);
    assert_eq!(groups[1][7], format!("{}", mass_of(&plate)));
    assert_eq!(groups[1][8], "0");
    assert_eq!(groups[2][..3], ["beam", "RECT100x50", "S355"]);
    assert_eq!(groups[2][4], "3000");
    assert_eq!(groups[2][7], format!("{}", mass_of(&beam)));
    assert_eq!(groups[3][..3], ["column", "SYNI300", ""]);
    assert_eq!(groups[3][8], "1");
}

#[test]
fn refusals_write_nothing() {
    let f = Fixture::new();
    // A store without a revision has nothing to report.
    assert_eq!(
        f.rejected_empty(
            "bom-export",
            &[
                "--out",
                &f.out("elements.csv"),
                "--summary",
                &f.out("summary.csv")
            ]
        ),
        "NOT_FOUND"
    );
    assert!(!f.root.join("elements.csv").exists());

    f.build_model();
    let elements_path = f.out("elements.csv");
    let exported = f.ok("bom-export", &["--out", &elements_path]);
    assert_eq!(text(&exported, "outcome"), "bom_exported");
    // Files are created new, never overwritten.
    assert_eq!(
        f.rejected("bom-export", &["--out", &elements_path]),
        "INVALID_ARGUMENT"
    );
    let other = f.out("other.csv");
    assert_eq!(
        f.rejected("bom-export", &["--out", &other, "--bogus", "x"]),
        "INVALID_ARGUMENT"
    );
    assert!(!f.root.join("other.csv").exists());
    // At least one output file is required.
    assert_eq!(f.rejected("bom-export", &[]), "INVALID_ARGUMENT");
    // A refusal on either path leaves nothing, not even the file of the other output: both
    // files are reserved before any bytes are written.
    let fresh = f.out("fresh.csv");
    let taken = f.out("taken.csv");
    fs::write(&taken, "occupied").unwrap();
    assert_eq!(
        f.rejected("bom-export", &["--out", &fresh, "--summary", &taken]),
        "INVALID_ARGUMENT"
    );
    assert!(!f.root.join("fresh.csv").exists());
    let same = f.out("same.csv");
    assert_eq!(
        f.rejected("bom-export", &["--out", &same, "--summary", &same]),
        "INVALID_ARGUMENT"
    );
    assert!(!f.root.join("same.csv").exists());
}
