// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, members and assemblies, MPL-2.0.

//! A08 end to end through the real CLI binary: `preview-numbering` groups equivalent parts,
//! proposes series, shows overlaps, goes stale exactly when an edit changes the numbering, and
//! never writes to the store.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

mod support;

use support::{at, text};

use beamcraft_json::{Json, Limits};
use std::process::Command;

struct Fixture {
    shared: support::StoreFixture,
}

struct Scene {
    steel: String,
    other_steel: String,
    plates: Vec<String>,
    beams: Vec<String>,
}

impl Fixture {
    fn new() -> Self {
        Self {
            shared: support::StoreFixture::new("a08-tests"),
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
    fn preview(&self, extra: &[&str]) -> Json {
        self.ok("preview-numbering", extra)
    }
    fn plate(&self, material: &str, thickness: &str, extra: &[&str]) -> String {
        let mut args = vec![
            "--width",
            "400",
            "--length",
            "300",
            "--thickness",
            thickness,
            "--material-digest",
            material,
        ];
        args.extend_from_slice(extra);
        text(
            at(&self.ok("create-plate", &args), &["plate"]),
            "element_id",
        )
    }
    fn beam(&self, profile: &str, material: &str, end: &str, translation: &str) -> String {
        text(
            at(
                &self.ok(
                    "create-member",
                    &[
                        "--start",
                        "0,0,0",
                        "--end",
                        end,
                        "--profile-digest",
                        profile,
                        "--material-digest",
                        material,
                        "--translation",
                        translation,
                    ],
                ),
                &["member"],
            ),
            "element_id",
        )
    }
    fn head(&self) -> String {
        text(&self.ok("open-revision", &[]), "revision_id")
    }
    fn layers(&self) -> Vec<(String, String)> {
        list(&self.ok("open-revision", &[]), "layers")
            .iter()
            .map(|l| (text(l, "name"), text(l, "digest")))
            .collect()
    }
    /// Plates 1, 2 (identical), 3 (other material), 4 (a hole), 5 (thicker); beams 1, 2
    /// (identical, one moved), 3 (other length).
    fn scene(&self) -> Scene {
        let material = |name: &str, density: &str| {
            text(
                at(
                    &self.ok(
                        "define-material",
                        &["--density", density, "--name", name, "--grade", name],
                    ),
                    &["definition"],
                ),
                "digest",
            )
        };
        let steel = material("S355", "7850");
        let other_steel = material("S275", "7850");
        let profile = text(
            at(
                &self.ok("define-profile", &["--width", "100", "--height", "50"]),
                &["definition"],
            ),
            "digest",
        );
        let plates = vec![
            self.plate(&steel, "20", &[]),
            self.plate(&steel, "20", &["--translation", "1000,0,0"]),
            self.plate(&other_steel, "20", &[]),
            self.plate(&steel, "20", &["--hole-round", "100,100,40"]),
            self.plate(&steel, "25", &[]),
        ];
        let beams = vec![
            self.beam(&profile, &steel, "3000,0,0", "0,0,0"),
            self.beam(&profile, &steel, "3000,0,0", "0,500,0"),
            self.beam(&profile, &steel, "3500,0,0", "0,1000,0"),
        ];
        Scene {
            steel,
            other_steel,
            plates,
            beams,
        }
    }
}

fn list<'a>(json: &'a Json, key: &str) -> &'a [Json] {
    at(json, &[key]).array().unwrap()
}
fn number(json: &Json, key: &str) -> u64 {
    json.get(key).unwrap().unsigned().unwrap()
}
fn is_null(json: &Json, key: &str) -> bool {
    matches!(json.get(key), Ok(Json::Null))
}
fn groups(preview: &Json) -> Vec<(String, String, u64)> {
    list(preview, "groups")
        .iter()
        .map(|g| (text(g, "kind"), text(g, "position"), number(g, "count")))
        .collect()
}

#[test]
fn identical_parts_get_one_number_and_every_difference_splits_a_group() {
    let f = Fixture::new();
    let s = f.scene();
    let p = f.preview(&[]);
    assert_eq!(text(&p, "outcome"), "numbering_preview");
    assert_eq!(text(&p, "revision_id"), f.head());
    assert_eq!(
        groups(&p),
        [
            ("plate".into(), "PL1".into(), 2),
            ("plate".into(), "PL2".into(), 1),
            ("plate".into(), "PL3".into(), 1),
            ("plate".into(), "PL4".into(), 1),
            ("beam".into(), "B1".into(), 2),
            ("beam".into(), "B2".into(), 1),
        ]
    );
    // The first group is the two identical plates (the other was only moved); the oldest is the
    // reference.
    let first = &list(&p, "groups")[0];
    assert_eq!(text(first, "reference"), s.plates[0]);
    let members: Vec<_> = list(first, "members")
        .iter()
        .map(|m| m.string().unwrap().to_owned())
        .collect();
    assert_eq!(members, [s.plates[0].clone(), s.plates[1].clone()]);
    // Material, a hole and the thickness each give their own group.
    let reference = |position: &str| {
        text(
            list(&p, "groups")
                .iter()
                .find(|g| text(g, "position") == position)
                .unwrap(),
            "reference",
        )
    };
    assert_eq!(reference("PL2"), s.plates[2]);
    assert_eq!(reference("PL3"), s.plates[3]);
    assert_eq!(reference("PL4"), s.plates[4]);
    assert_eq!(reference("B2"), s.beams[2]);
    assert_eq!(number(at(&p, &["counts"]), "parts"), 8);
    assert_eq!(number(at(&p, &["counts"]), "groups"), 6);
    assert_eq!(number(at(&p, &["counts"]), "overlapping_groups"), 0);
    assert!(is_null(&p, "stale"));
    assert!(matches!(p.get("writes_model"), Ok(Json::Bool(false))));
    let _ = (&s.steel, &s.other_steel);
}

#[test]
fn a_preview_changes_nothing_in_the_store() {
    let f = Fixture::new();
    f.scene();
    let (head, layers) = (f.head(), f.layers());
    f.preview(&[]);
    f.preview(&["--beam-series", "B:100", "--compare-name", "true"]);
    assert_eq!(f.head(), head);
    assert_eq!(f.layers(), layers);
}

#[test]
fn an_edit_that_changes_the_numbering_makes_the_preview_stale() {
    let f = Fixture::new();
    let s = f.scene();
    let token = text(&f.preview(&[]), "preview_token");
    let stale = |f: &Fixture| {
        f.preview(&["--previous", &token])
            .get("stale")
            .unwrap()
            .clone()
    };
    assert!(matches!(stale(&f), Json::Bool(false)));
    // A move and a rename do not change the numbering.
    f.ok(
        "update-element",
        &["--id", &s.plates[0], "--translation", "5,5,5"],
    );
    f.ok(
        "update-element",
        &["--id", &s.plates[1], "--name", "Renamed"],
    );
    assert!(matches!(stale(&f), Json::Bool(false)));
    // A thickness edit that splits the group does.
    f.ok(
        "update-element",
        &["--id", &s.plates[1], "--thickness", "30"],
    );
    assert!(matches!(stale(&f), Json::Bool(true)));
    let after_thickness = text(&f.preview(&[]), "preview_token");
    assert_ne!(after_thickness, token);
    // The new preview now lists the plates in three groups of their own.
    let p = f.preview(&["--previous", &after_thickness]);
    assert!(matches!(p.get("stale").unwrap(), Json::Bool(false)));
    assert_eq!(
        list(&p, "groups")
            .iter()
            .filter(|g| text(g, "kind") == "plate")
            .count(),
        5
    );
    // A material edit, a copy and a delete are edits of the numbering too.
    let steel = f.preview(&[]);
    let current = text(&steel, "preview_token");
    f.ok(
        "update-element",
        &["--id", &s.beams[0], "--material-digest", &s.other_steel],
    );
    assert!(matches!(
        f.preview(&["--previous", &current]).get("stale").unwrap(),
        Json::Bool(true)
    ));
    let current = text(&f.preview(&[]), "preview_token");
    f.ok("copy-element", &["--id", &s.beams[2]]);
    assert!(matches!(
        f.preview(&["--previous", &current]).get("stale").unwrap(),
        Json::Bool(true)
    ));
    let current = text(&f.preview(&[]), "preview_token");
    f.ok("delete-element", &["--id", &s.beams[2]]);
    assert!(matches!(
        f.preview(&["--previous", &current]).get("stale").unwrap(),
        Json::Bool(true)
    ));
}

#[test]
fn a_copy_joins_the_group_of_its_source_without_renumbering() {
    let f = Fixture::new();
    let s = f.scene();
    let before = groups(&f.preview(&[]));
    f.ok(
        "copy-element",
        &["--id", &s.beams[0], "--offset", "0,2000,0"],
    );
    let after = groups(&f.preview(&[]));
    let expected: Vec<_> = before
        .iter()
        .map(|(k, p, n)| (k.clone(), p.clone(), if p == "B1" { n + 1 } else { *n }))
        .collect();
    assert_eq!(after, expected);
}

#[test]
fn series_come_from_options_and_overlaps_are_shown() {
    let f = Fixture::new();
    f.scene();
    let p = f.preview(&[
        "--plate-series",
        "P:1",
        "--beam-series",
        "P:3",
        "--column-series",
        ":1000",
    ]);
    let table: Vec<_> = list(&p, "groups")
        .iter()
        .map(|g| {
            (
                text(g, "position"),
                text(g, "status"),
                g.get("overlaps_with")
                    .unwrap()
                    .string()
                    .ok()
                    .map(str::to_owned),
            )
        })
        .collect();
    assert_eq!(
        table,
        [
            ("P1".into(), "new".into(), None),
            ("P2".into(), "new".into(), None),
            ("P3".into(), "new".into(), None),
            ("P4".into(), "new".into(), None),
            ("P3".into(), "overlapping".into(), Some("P3".into())),
            ("P4".into(), "overlapping".into(), Some("P4".into())),
        ]
    );
    assert_eq!(number(at(&p, &["counts"]), "overlapping_groups"), 2);
    assert_eq!(number(at(&p, &["counts"]), "unnumbered_members"), 3);
    let series = list(&p, "series");
    assert_eq!(text(&series[1], "prefix"), "P");
    assert_eq!(number(&series[1], "start"), 3);
    assert!(is_null(&series[2], "first"));
    // An empty prefix is a plain number.
    let q = f.preview(&["--plate-series", ":1000"]);
    assert_eq!(text(&list(&q, "groups")[0], "position"), "1000");
}

#[test]
fn the_tolerance_and_the_name_setting_change_the_grouping() {
    let f = Fixture::new();
    let s = f.scene();
    // A beam 0.3 mm longer is another part until the tolerance covers it.
    let profile = text(
        at(
            &f.ok("define-profile", &["--width", "100", "--height", "50"]),
            &["definition"],
        ),
        "digest",
    );
    f.beam(&profile, &s.steel, "3000.3,0,0", "0,0,0");
    let exact = groups(&f.preview(&[]));
    assert_eq!(exact.iter().filter(|g| g.0 == "beam").count(), 3);
    let loose = groups(&f.preview(&["--tolerance", "0.5"]));
    assert_eq!(loose.iter().filter(|g| g.0 == "beam").count(), 2);
    // Names are compared only on request.
    let named = groups(&f.preview(&["--compare-name", "true"]));
    assert!(named.iter().filter(|g| g.0 == "plate").count() >= 4);
    f.ok(
        "update-element",
        &["--id", &s.plates[1], "--name", "Different"],
    );
    let by_name = groups(&f.preview(&["--compare-name", "true"]));
    assert_eq!(by_name.iter().filter(|g| g.0 == "plate").count(), 5);
    let by_default = groups(&f.preview(&[]));
    assert_eq!(by_default.iter().filter(|g| g.0 == "plate").count(), 4);
}

#[test]
fn assemblies_with_the_same_content_share_a_mark() {
    let f = Fixture::new();
    let s = f.scene();
    let mut marks = Vec::new();
    for (name, parts) in [
        ("A one", vec![&s.plates[0], &s.beams[0]]),
        ("A two", vec![&s.plates[1], &s.beams[1]]),
        ("A three", vec![&s.beams[2]]),
    ] {
        let id = text(&f.ok("create-assembly", &["--name", name]), "element_id");
        for part in parts {
            f.ok("attach-to-assembly", &["--id", part, "--assembly-id", &id]);
        }
        marks.push(id);
    }
    let p = f.preview(&[]);
    let assemblies: Vec<_> = list(&p, "groups")
        .iter()
        .filter(|g| text(g, "kind") == "assembly")
        .collect();
    assert_eq!(assemblies.len(), 2);
    assert_eq!(text(assemblies[0], "position"), "A-1");
    assert_eq!(number(assemblies[0], "count"), 2);
    assert_eq!(text(assemblies[0], "reference"), marks[0]);
    assert_eq!(text(assemblies[1], "position"), "A-2");
    let q = f.preview(&["--assembly-series", "AS:10"]);
    assert!(
        list(&q, "groups")
            .iter()
            .any(|g| text(g, "position") == "AS-10")
    );
}

#[test]
fn groups_are_paged() {
    let f = Fixture::new();
    f.scene();
    let first = f.preview(&["--limit", "4"]);
    assert_eq!(list(&first, "groups").len(), 4);
    assert_eq!(number(&first, "next_offset"), 4);
    let second = f.preview(&["--limit", "4", "--offset", "4"]);
    assert_eq!(list(&second, "groups").len(), 2);
    assert!(is_null(&second, "next_offset"));
    assert_eq!(text(&list(&second, "groups")[0], "position"), "B1");
    assert_eq!(number(at(&second, &["counts"]), "groups"), 6);
}

#[test]
fn bad_input_is_refused_without_a_preview() {
    let f = Fixture::new();
    f.scene();
    for extra in [
        vec!["--plate-series", "PL"],
        vec!["--plate-series", "PL:x"],
        vec!["--plate-series", "P-L:1"],
        vec!["--plate-series", "AAAAAAAAAAAAAAAAA:1"],
        vec!["--plate-series", "P:2000000000"],
        vec!["--tolerance", "-1"],
        vec!["--tolerance", "NaN"],
        vec!["--tolerance", "5000"],
        vec!["--compare-name", "maybe"],
        vec!["--previous", "abc"],
        vec!["--limit", "0"],
        vec!["--limit", "257"],
        vec!["--offset", "x"],
        vec!["--bogus", "1"],
        vec!["--limit", "1", "--limit", "2"],
    ] {
        let (code, body) = f.run("preview-numbering", &extra);
        assert_eq!(
            (code, text(&body, "code").as_str()),
            (2, "INVALID_ARGUMENT"),
            "{extra:?}"
        );
    }
    let (code, body) = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .args(["preview-numbering"])
        .output()
        .map(|o| {
            (
                o.status.code().unwrap(),
                beamcraft_json::decode(&o.stdout, &Limits::DEFAULT).unwrap(),
            )
        })
        .unwrap();
    assert_eq!(
        (code, text(&body, "code").as_str()),
        (2, "INVALID_ARGUMENT")
    );
}

#[test]
fn a_store_without_parts_has_an_empty_preview() {
    let f = Fixture::new();
    f.ok("define-material", &["--density", "7850"]);
    let p = f.preview(&[]);
    assert!(list(&p, "groups").is_empty());
    assert_eq!(number(at(&p, &["counts"]), "parts"), 0);
}

#[test]
fn numbering_binds_evaluated_shapes_and_stales_even_when_group_membership_does_not_change() {
    let f = Fixture::new();
    let profile = text(
        at(
            &f.ok("define-profile", &["--width", "100", "--height", "50"]),
            &["definition"],
        ),
        "digest",
    );
    let bent = f.ok(
        "create-polybeam",
        &[
            "--points",
            "0,0,0;3000,0,0;3000,4000,0",
            "--profile-digest",
            &profile,
        ],
    );
    let id = text(at(&bent, &["member"]), "element_id");
    f.ok(
        "create-member",
        &[
            "--start",
            "0,0,0",
            "--end",
            "7000,0,0",
            "--profile-digest",
            &profile,
        ],
    );
    assert_eq!(
        groups(&f.preview(&[])),
        [
            ("beam".into(), "B1".into(), 1),
            ("beam".into(), "B2".into(), 1)
        ]
    );
    let before = f.preview(&[]);
    let token = text(&before, "preview_token");
    f.ok("approve-numbering", &["--preview-token", &token]);
    // Moving and rotating the scene instance do not change its normalized geometry.
    f.ok(
        "update-element",
        &[
            "--id",
            &id,
            "--translation",
            "1200,800,300",
            "--orientation-xyzw",
            "0,0,0.7071067811865476,0.7071067811865476",
        ],
    );
    let moved = f.preview(&["--previous", &token]);
    assert!(matches!(moved.get("stale").unwrap(), Json::Bool(false)));
    assert_eq!(text(at(&moved, &["approval"]), "state"), "current");
    // The total length and the B1/B2 membership stay the same, but the finished body changes.
    f.ok(
        "update-element",
        &["--id", &id, "--axis-point", "0,0,0;4000,0,0;4000,3000,0"],
    );
    let changed = f.preview(&["--previous", &token]);
    assert_eq!(groups(&changed), groups(&before));
    assert!(matches!(changed.get("stale").unwrap(), Json::Bool(true)));
    assert_eq!(text(at(&changed, &["approval"]), "state"), "stale");
    let head = f.head();
    let (code, refusal) = f.run("approve-numbering", &["--preview-token", &token]);
    assert_eq!(code, 2, "{refusal:?}");
    assert_eq!(text(&refusal, "code"), "STALE_WORKSPACE");
    assert_eq!(f.head(), head);
    f.ok("copy-element", &["--id", &id, "--offset", "0,1000,0"]);
    assert_eq!(
        groups(&f.preview(&[])),
        [
            ("beam".into(), "B1".into(), 2),
            ("beam".into(), "B2".into(), 1)
        ]
    );
    let token = text(&f.preview(&[]), "preview_token");
    // Straightening at the same length now joins the straight part's group.
    f.ok(
        "update-element",
        &["--id", &id, "--axis-point", "0,0,0;7000,0,0"],
    );
    let straightened = f.preview(&["--previous", &token]);
    assert!(matches!(
        straightened.get("stale").unwrap(),
        Json::Bool(true)
    ));
    assert_eq!(
        groups(&straightened),
        [
            ("beam".into(), "B1".into(), 2),
            ("beam".into(), "B2".into(), 1)
        ]
    );
}

#[test]
fn a_member_cut_changes_numbering_without_changing_the_axis_length() {
    let f = Fixture::new();
    let profile = text(
        at(
            &f.ok("define-profile", &["--width", "100", "--height", "50"]),
            &["definition"],
        ),
        "digest",
    );
    let mut ids = Vec::new();
    for _ in 0..2 {
        let member = f.ok(
            "create-member",
            &[
                "--start",
                "0,0,0",
                "--end",
                "7000,0,0",
                "--profile-digest",
                &profile,
            ],
        );
        ids.push(text(at(&member, &["member"]), "element_id"));
    }
    let token = text(&f.preview(&[]), "preview_token");
    assert_eq!(groups(&f.preview(&[])), [("beam".into(), "B1".into(), 2)]);
    f.ok(
        "cut-by-plane",
        &[
            "--id",
            &ids[1],
            "--operation-id",
            "01900000-0000-7000-8000-000000000501",
            "--point",
            "3500,0,0",
            "--normal",
            "1,0,0",
            "--keep",
            "negative",
        ],
    );
    let after = f.preview(&["--previous", &token]);
    assert!(matches!(after.get("stale").unwrap(), Json::Bool(true)));
    assert_eq!(
        groups(&after),
        [
            ("beam".into(), "B1".into(), 1),
            ("beam".into(), "B2".into(), 1)
        ]
    );
}
