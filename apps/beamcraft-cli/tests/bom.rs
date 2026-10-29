// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, beams and materials, MPL-2.0.

//! E03 end to end: `export-bom` through the real CLI binary against a verified store. The
//! exported per-element and summary CSVs match `inspect-elements`, the mark is the element id
//! (the fallback until numbering exists), a mass without a density is an empty cell excluded
//! from the totals and counted in `mass_unknown`, and the files are created new, never
//! overwritten.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support;

use support::{at, text};

use beamcraft_json::Json;
use std::fs;

struct Fixture {
    shared: support::StoreFixture,
}
impl Fixture {
    fn new() -> Self {
        Self {
            shared: support::StoreFixture::new("e03-tests"),
        }
    }
    fn run(&self, command: &str, extra: &[&str]) -> (i32, Json) {
        self.shared.run(command, extra)
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
        text(&self.ok("open-revision", &[]), "revision_id")
    }
    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.shared.root.join(name)).unwrap()
    }
    /// Absolute path of an output file under the fixture root.
    fn out(&self, name: &str) -> String {
        self.shared.root.join(name).to_str().unwrap().to_owned()
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
        // The numbering-approval path: approve only the beam series (B), so the beam carries
        // the stored mark B1 and every other element stays unnumbered.
        let token = text(
            &self.ok("preview-numbering", &["--beam-series", "B:1"]),
            "preview_token",
        );
        self.ok(
            "approve-numbering",
            &["--preview-token", &token, "--beam-series", "B:1"],
        );
        (plate, bare_plate, beam, column)
    }
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
    // A part created after the approval carries no mark: the report falls back to the id.
    let late = text(
        at(
            &f.ok(
                "create-plate",
                &["--width", "600", "--length", "300", "--thickness", "10"],
            ),
            &["plate"],
        ),
        "element_id",
    );

    // The quantities the CSV must repeat come from the verified read of the same head.
    let inspected = f.ok("inspect-elements", &[]);
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
        "export-bom",
        &["--out", &elements_path, "--summary", &summary_path],
    );
    assert_eq!(text(&exported, "outcome"), "bom_exported");
    assert_eq!(at(&exported, &["elements"]).float(), Ok(5.0));
    assert_eq!(at(&exported, &["groups"]).float(), Ok(4.0));
    assert_eq!(text(&exported, "elements_file"), elements_path);
    assert_eq!(text(&exported, "summary_file"), summary_path);

    let elements = f.read("elements.csv");
    let lines: Vec<&str> = elements.lines().collect();
    assert_eq!(lines.len(), 6, "{elements}");
    assert_eq!(
        lines[0],
        "mark,element_id,kind,name,profile,material,length_mm,area_mm2,volume_mm3,volume_m3,mass_kg"
    );
    // Report order: plates, then beams, then columns; within a kind by profile, then mark.
    // The approved marks are PL1 (the bare plate), PL2, B1 and C1; the late plate is
    // unnumbered, so its mark falls back to the element id and sorts before the PL marks.
    let rows: Vec<Vec<&str>> = lines[1..].iter().map(|l| cells(l)).collect();
    assert_eq!(rows[0][2], "plate");
    assert_eq!(rows[0][0], late, "an unnumbered mark falls back to the id");
    assert_eq!(rows[0][0], rows[0][1]);
    assert_eq!(
        rows[0][5], "",
        "a plate without a material has no material cell"
    );
    assert_eq!(rows[0][6], "", "a plate has no length cell");
    assert_eq!(rows[0][7], "180000", "net face area = volume / thickness");
    assert_eq!(
        rows[0][10], "",
        "an unknown mass is an empty cell, never zero"
    );
    assert_eq!(
        rows[1][0], "PL1",
        "the approved numbering labels the element"
    );
    assert_eq!(rows[1][1], bare_plate);
    assert_eq!(rows[1][5], "");
    assert_eq!(rows[1][7], "320000");
    assert_eq!(rows[1][10], "");
    assert_eq!(rows[2][0], "PL2");
    assert_eq!(rows[2][1], plate);
    assert_eq!(rows[2][5], "S355");
    assert_eq!(rows[2][7], "500000");
    assert_eq!(
        rows[2][10],
        format!("{}", mass_of(&plate)),
        "the CSV mass is the verified mass"
    );
    assert_eq!(rows[3][2], "beam");
    assert_eq!(rows[3][0], "B1", "an approved mark labels the element");
    assert_eq!(rows[3][1], beam);
    assert_eq!(
        rows[3][4], "RECT100x50",
        "the designation labels the profile"
    );
    assert_eq!(rows[3][6], "3000");
    assert_eq!(rows[3][7], "5000", "net section area = volume / length");
    assert_eq!(rows[3][10], format!("{}", mass_of(&beam)));
    assert_eq!(rows[4][2], "column");
    assert_eq!(rows[4][1], column);
    assert_eq!(rows[4][0], "C1");
    assert_eq!(rows[4][4], "SYNI300");
    assert_eq!(rows[4][5], "");
    assert_eq!(rows[4][10], "");

    let summary = f.read("summary.csv");
    let lines: Vec<&str> = summary.lines().collect();
    assert_eq!(lines.len(), 5, "{summary}");
    assert_eq!(
        lines[0],
        "kind,profile,material,count,total_length_mm,total_area_m2,total_volume_m3,total_mass_kg,mass_unknown"
    );
    let groups: Vec<Vec<&str>> = lines[1..].iter().map(|l| cells(l)).collect();
    assert_eq!(groups[0][..3], ["plate", "PL10", ""]);
    assert_eq!(
        groups[0][3], "2",
        "the bare and the late plate share the group"
    );
    assert_eq!(groups[0][4], "", "plates have no total length");
    assert_eq!(groups[0][7], "");
    assert_eq!(
        groups[0][8], "2",
        "both plates without a material are counted as unknown mass"
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
            "export-bom",
            &[
                "--out",
                &f.out("elements.csv"),
                "--summary",
                &f.out("summary.csv")
            ]
        ),
        "NOT_FOUND"
    );
    assert!(!f.shared.root.join("elements.csv").exists());

    f.build_model();
    let elements_path = f.out("elements.csv");
    let exported = f.ok("export-bom", &["--out", &elements_path]);
    assert_eq!(text(&exported, "outcome"), "bom_exported");
    // Files are created new, never overwritten.
    assert_eq!(
        f.rejected("export-bom", &["--out", &elements_path]),
        "INVALID_ARGUMENT"
    );
    let other = f.out("other.csv");
    assert_eq!(
        f.rejected("export-bom", &["--out", &other, "--bogus", "x"]),
        "INVALID_ARGUMENT"
    );
    assert!(!f.shared.root.join("other.csv").exists());
    // At least one output file is required.
    assert_eq!(f.rejected("export-bom", &[]), "INVALID_ARGUMENT");
    // A refusal on either path leaves nothing, not even the file of the other output: both
    // files are reserved before any bytes are written.
    let fresh = f.out("fresh.csv");
    let taken = f.out("taken.csv");
    fs::write(&taken, "occupied").unwrap();
    assert_eq!(
        f.rejected("export-bom", &["--out", &fresh, "--summary", &taken]),
        "INVALID_ARGUMENT"
    );
    assert!(!f.shared.root.join("fresh.csv").exists());
    let same = f.out("same.csv");
    assert_eq!(
        f.rejected("export-bom", &["--out", &same, "--summary", &same]),
        "INVALID_ARGUMENT"
    );
    assert!(!f.shared.root.join("same.csv").exists());
}
