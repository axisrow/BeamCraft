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

use crate::support;

use support::{Scripted, at, text};

use beamcraft_json::{Json, Limits};
use std::process::Command;

struct Fixture {
    shared: support::StoreFixture,
}

struct Scene {
    steel: String,
    other_steel: String,
    profile: String,
    plates: Vec<String>,
    beams: Vec<String>,
}

impl Fixture {
    fn new() -> Self {
        Self {
            shared: support::StoreFixture::new("a08-tests"),
        }
    }
    fn store(&self) -> String {
        self.shared.root.join("store").to_str().unwrap().to_owned()
    }
    fn run(&self, command: &str, extra: &[&str]) -> (i32, Json) {
        self.shared.run(command, extra)
    }
    fn ok(&self, command: &str, extra: &[&str]) -> Json {
        let (code, body) = self.run(command, extra);
        assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
        body
    }
    /// One `run-script` invocation over the fixture store: the same commands the suite would
    /// spawn one process each, amortized into a single process (#738 P7; each command's own
    /// work is unchanged). Replies come back in the lines' order.
    fn script(&self, lines: &[Scripted]) -> Vec<Json> {
        let store = self.store();
        support::script_lines(&self.shared.root, &store, lines)
    }
    fn preview(&self, extra: &[&str]) -> Json {
        self.ok("preview-numbering", extra)
    }
    /// The `create-plate` line over the fixture store, the exact tokens the per-command path
    /// spawned.
    fn plate_line(store: &str, thickness: &str, material: &str, extra: &[&str]) -> Scripted {
        let mut args = vec![
            "create-plate",
            "--store",
            store,
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
        Scripted::ok(&args)
    }
    /// The `create-member` line over the fixture store, the exact tokens the per-command path
    /// spawned.
    fn beam_line(
        store: &str,
        profile: &str,
        material: &str,
        end: &str,
        translation: &str,
    ) -> Scripted {
        Scripted::ok(&[
            "create-member",
            "--store",
            store,
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
        ])
    }
    /// Plates 1, 2 (identical), 3 (other material), 4 (a hole), 5 (thicker); beams 1, 2
    /// (identical, one moved), 3 (other length). The definitions and the parts each ride one
    /// process (#738 P7); the digests and ids come back in the replies' order.
    fn scene(&self) -> Scene {
        let store = self.store();
        let material = |store: &str, name: &str| {
            Scripted::ok(&[
                "define-material",
                "--store",
                store,
                "--density",
                "7850",
                "--name",
                name,
                "--grade",
                name,
            ])
        };
        let defined = self.script(&[
            material(&store, "S355"),
            material(&store, "S275"),
            Scripted::ok(&[
                "define-profile",
                "--store",
                &store,
                "--width",
                "100",
                "--height",
                "50",
            ]),
        ]);
        let steel = text(at(&defined[0], &["definition"]), "digest");
        let other_steel = text(at(&defined[1], &["definition"]), "digest");
        let profile = text(at(&defined[2], &["definition"]), "digest");
        let made = self.script(&[
            Self::plate_line(&store, "20", &steel, &[]),
            Self::plate_line(&store, "20", &steel, &["--translation", "1000,0,0"]),
            Self::plate_line(&store, "20", &other_steel, &[]),
            Self::plate_line(&store, "20", &steel, &["--hole-round", "100,100,40"]),
            Self::plate_line(&store, "25", &steel, &[]),
            Self::beam_line(&store, &profile, &steel, "3000,0,0", "0,0,0"),
            Self::beam_line(&store, &profile, &steel, "3000,0,0", "0,500,0"),
            Self::beam_line(&store, &profile, &steel, "3500,0,0", "0,1000,0"),
        ]);
        let id = |reply: &Json, key: &str| text(at(reply, &[key]), "element_id");
        Scene {
            steel,
            other_steel,
            profile,
            plates: (0..5).map(|n| id(&made[n], "plate")).collect(),
            beams: (5..8).map(|n| id(&made[n], "member")).collect(),
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
    let store = f.store();
    // The preview and the head it is compared against ride one process (#738 P7).
    let opened = f.script(&[
        Scripted::ok(&["preview-numbering", "--store", &store]),
        Scripted::ok(&["open-revision", "--store", &store]),
    ]);
    let p = &opened[0];
    assert_eq!(text(p, "outcome"), "numbering_preview");
    assert_eq!(text(p, "revision_id"), text(&opened[1], "revision_id"));
    assert_eq!(
        groups(p),
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
    let first = &list(p, "groups")[0];
    assert_eq!(text(first, "reference"), s.plates[0]);
    let members: Vec<_> = list(first, "members")
        .iter()
        .map(|m| m.string().unwrap().to_owned())
        .collect();
    assert_eq!(members, [s.plates[0].clone(), s.plates[1].clone()]);
    // Material, a hole and the thickness each give their own group.
    let reference = |position: &str| {
        text(
            list(p, "groups")
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
    assert_eq!(number(at(p, &["counts"]), "parts"), 8);
    assert_eq!(number(at(p, &["counts"]), "groups"), 6);
    assert_eq!(number(at(p, &["counts"]), "overlapping_groups"), 0);
    assert!(is_null(p, "stale"));
    assert!(matches!(p.get("writes_model"), Ok(Json::Bool(false))));
    let _ = (&s.steel, &s.other_steel);
}

#[test]
fn a_preview_changes_nothing_in_the_store() {
    let f = Fixture::new();
    f.scene();
    let store = f.store();
    // The before/after head reads and both previews ride one process (#738 P7).
    let opened = f.script(&[
        Scripted::ok(&["open-revision", "--store", &store]),
        Scripted::ok(&["preview-numbering", "--store", &store]),
        Scripted::ok(&[
            "preview-numbering",
            "--store",
            &store,
            "--beam-series",
            "B:100",
            "--compare-name",
            "true",
        ]),
        Scripted::ok(&["open-revision", "--store", &store]),
    ]);
    assert_eq!(
        text(&opened[0], "revision_id"),
        text(&opened[3], "revision_id")
    );
    assert_eq!(at(&opened[0], &["layers"]), at(&opened[3], &["layers"]));
}

#[test]
fn an_edit_that_changes_the_numbering_makes_the_preview_stale() {
    let f = Fixture::new();
    let s = f.scene();
    let store = f.store();
    let token = text(&f.preview(&[]), "preview_token");
    let stale = |f: &Fixture| {
        f.preview(&["--previous", &token])
            .get("stale")
            .unwrap()
            .clone()
    };
    assert!(matches!(stale(&f), Json::Bool(false)));
    // A move and a rename do not change the numbering; the edits ride one process (#738 P7).
    let updated = f.script(&[
        Scripted::ok(&[
            "update-element",
            "--store",
            &store,
            "--id",
            &s.plates[0],
            "--translation",
            "5,5,5",
        ]),
        Scripted::ok(&[
            "update-element",
            "--store",
            &store,
            "--id",
            &s.plates[1],
            "--name",
            "Renamed",
        ]),
    ]);
    for reply in &updated {
        assert_eq!(text(reply, "outcome"), "element_updated");
    }
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
    let store = f.store();
    let around_the_copy = f.script(&[
        Scripted::ok(&["preview-numbering", "--store", &store]),
        Scripted::ok(&[
            "copy-element",
            "--store",
            &store,
            "--id",
            &s.beams[0],
            "--offset",
            "0,2000,0",
        ]),
        Scripted::ok(&["preview-numbering", "--store", &store]),
    ]);
    let before = groups(&around_the_copy[0]);
    let after = groups(&around_the_copy[2]);
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
    let store = f.store();
    let previews = f.script(&[
        Scripted::ok(&[
            "preview-numbering",
            "--store",
            &store,
            "--plate-series",
            "P:1",
            "--beam-series",
            "P:3",
            "--column-series",
            ":1000",
        ]),
        Scripted::ok(&[
            "preview-numbering",
            "--store",
            &store,
            "--plate-series",
            ":1000",
        ]),
    ]);
    let p = &previews[0];
    let table: Vec<_> = list(p, "groups")
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
    assert_eq!(number(at(p, &["counts"]), "overlapping_groups"), 2);
    assert_eq!(number(at(p, &["counts"]), "unnumbered_members"), 3);
    let series = list(p, "series");
    assert_eq!(text(&series[1], "prefix"), "P");
    assert_eq!(number(&series[1], "start"), 3);
    assert!(is_null(&series[2], "first"));
    // An empty prefix is a plain number.
    let q = &previews[1];
    assert_eq!(text(&list(q, "groups")[0], "position"), "1000");
}

#[test]
fn the_tolerance_and_the_name_setting_change_the_grouping() {
    let f = Fixture::new();
    let s = f.scene();
    let store = f.store();
    // A beam 0.3 mm longer is another part until the tolerance covers it. The profile is the
    // scene's 100x50 one: the same parameters are content-addressed to the same definition.
    let rounds = f.script(&[
        Fixture::beam_line(&store, &s.profile, &s.steel, "3000.3,0,0", "0,0,0"),
        Scripted::ok(&["preview-numbering", "--store", &store]),
        Scripted::ok(&["preview-numbering", "--store", &store, "--tolerance", "0.5"]),
        Scripted::ok(&[
            "preview-numbering",
            "--store",
            &store,
            "--compare-name",
            "true",
        ]),
    ]);
    let exact = groups(&rounds[1]);
    assert_eq!(exact.iter().filter(|g| g.0 == "beam").count(), 3);
    let loose = groups(&rounds[2]);
    assert_eq!(loose.iter().filter(|g| g.0 == "beam").count(), 2);
    // Names are compared only on request.
    let named = groups(&rounds[3]);
    assert!(named.iter().filter(|g| g.0 == "plate").count() >= 4);
    f.ok(
        "update-element",
        &["--id", &s.plates[1], "--name", "Different"],
    );
    let after_the_rename = f.script(&[
        Scripted::ok(&[
            "preview-numbering",
            "--store",
            &store,
            "--compare-name",
            "true",
        ]),
        Scripted::ok(&["preview-numbering", "--store", &store]),
    ]);
    let by_name = groups(&after_the_rename[0]);
    assert_eq!(by_name.iter().filter(|g| g.0 == "plate").count(), 5);
    let by_default = groups(&after_the_rename[1]);
    assert_eq!(by_default.iter().filter(|g| g.0 == "plate").count(), 4);
}

#[test]
fn assemblies_with_the_same_content_share_a_mark() {
    let f = Fixture::new();
    let s = f.scene();
    let store = f.store();
    // The three assemblies are created in one process, the attachments and the two previews
    // in another (#738 P7).
    let created = f.script(&[
        Scripted::ok(&["create-assembly", "--store", &store, "--name", "A one"]),
        Scripted::ok(&["create-assembly", "--store", &store, "--name", "A two"]),
        Scripted::ok(&["create-assembly", "--store", &store, "--name", "A three"]),
    ]);
    let id = |reply: &Json| text(reply, "element_id");
    let (one, two, three) = (id(&created[0]), id(&created[1]), id(&created[2]));
    let attached = f.script(&[
        Scripted::ok(&[
            "attach-to-assembly",
            "--store",
            &store,
            "--id",
            &s.plates[0],
            "--assembly-id",
            &one,
        ]),
        Scripted::ok(&[
            "attach-to-assembly",
            "--store",
            &store,
            "--id",
            &s.beams[0],
            "--assembly-id",
            &one,
        ]),
        Scripted::ok(&[
            "attach-to-assembly",
            "--store",
            &store,
            "--id",
            &s.plates[1],
            "--assembly-id",
            &two,
        ]),
        Scripted::ok(&[
            "attach-to-assembly",
            "--store",
            &store,
            "--id",
            &s.beams[1],
            "--assembly-id",
            &two,
        ]),
        Scripted::ok(&[
            "attach-to-assembly",
            "--store",
            &store,
            "--id",
            &s.beams[2],
            "--assembly-id",
            &three,
        ]),
        Scripted::ok(&["preview-numbering", "--store", &store]),
        Scripted::ok(&[
            "preview-numbering",
            "--store",
            &store,
            "--assembly-series",
            "AS:10",
        ]),
    ]);
    let marks = [one, two, three];
    let p = &attached[5];
    let assemblies: Vec<_> = list(p, "groups")
        .iter()
        .filter(|g| text(g, "kind") == "assembly")
        .collect();
    assert_eq!(assemblies.len(), 2);
    assert_eq!(text(assemblies[0], "position"), "A-1");
    assert_eq!(number(assemblies[0], "count"), 2);
    assert_eq!(text(assemblies[0], "reference"), marks[0]);
    assert_eq!(text(assemblies[1], "position"), "A-2");
    let q = &attached[6];
    assert!(
        list(q, "groups")
            .iter()
            .any(|g| text(g, "position") == "AS-10")
    );
}

#[test]
fn groups_are_paged() {
    let f = Fixture::new();
    f.scene();
    let store = f.store();
    let pages = f.script(&[
        Scripted::ok(&["preview-numbering", "--store", &store, "--limit", "4"]),
        Scripted::ok(&[
            "preview-numbering",
            "--store",
            &store,
            "--limit",
            "4",
            "--offset",
            "4",
        ]),
    ]);
    let first = &pages[0];
    assert_eq!(list(first, "groups").len(), 4);
    assert_eq!(number(first, "next_offset"), 4);
    let second = &pages[1];
    assert_eq!(list(second, "groups").len(), 2);
    assert!(is_null(second, "next_offset"));
    assert_eq!(text(&list(second, "groups")[0], "position"), "B1");
    assert_eq!(number(at(second, &["counts"]), "groups"), 6);
}

#[test]
fn bad_input_is_refused_without_a_preview() {
    let f = Fixture::new();
    let store = f.store();
    // Argument refusals need a nonempty model, not the eight-part grouping scene.
    f.ok(
        "create-plate",
        &["--width", "400", "--length", "300", "--thickness", "20"],
    );
    let bad: [Vec<&str>; 15] = [
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
    ];
    let mut lines = Vec::new();
    for extra in &bad {
        let mut args = vec!["preview-numbering", "--store", store.as_str()];
        args.extend_from_slice(extra);
        lines.push(Scripted::refused(&args));
    }
    let refusals = f.script(&lines);
    for (extra, reply) in bad.iter().zip(refusals.iter()) {
        assert_eq!(text(reply, "code"), "INVALID_ARGUMENT", "{extra:?}");
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
    let store = f.store();
    let replies = f.script(&[
        Scripted::ok(&["define-material", "--store", &store, "--density", "7850"]),
        Scripted::ok(&["preview-numbering", "--store", &store]),
    ]);
    let p = &replies[1];
    assert!(list(p, "groups").is_empty());
    assert_eq!(number(at(p, &["counts"]), "parts"), 0);
}

#[test]
fn numbering_binds_evaluated_shapes_and_stales_even_when_group_membership_does_not_change() {
    let f = Fixture::new();
    let store = f.store();
    let defined = f.script(&[Scripted::ok(&[
        "define-profile",
        "--store",
        &store,
        "--width",
        "100",
        "--height",
        "50",
    ])]);
    let profile = text(at(&defined[0], &["definition"]), "digest");
    let built = f.script(&[
        Scripted::ok(&[
            "create-polybeam",
            "--store",
            &store,
            "--points",
            "0,0,0;3000,0,0;3000,4000,0",
            "--profile-digest",
            &profile,
        ]),
        Scripted::ok(&[
            "create-member",
            "--store",
            &store,
            "--start",
            "0,0,0",
            "--end",
            "7000,0,0",
            "--profile-digest",
            &profile,
        ]),
        Scripted::ok(&["preview-numbering", "--store", &store]),
    ]);
    let id = text(at(&built[0], &["member"]), "element_id");
    assert_eq!(
        groups(&built[2]),
        [
            ("beam".into(), "B1".into(), 1),
            ("beam".into(), "B2".into(), 1)
        ]
    );
    let before = built[2].clone();
    let token = text(&before, "preview_token");
    // The approval, the move-with-rotation and the read that judges them ride one process
    // (#738 P7): moving and rotating the scene instance do not change its normalized geometry.
    let moved_round = f.script(&[
        Scripted::ok(&[
            "approve-numbering",
            "--store",
            &store,
            "--preview-token",
            &token,
        ]),
        Scripted::ok(&[
            "update-element",
            "--store",
            &store,
            "--id",
            &id,
            "--translation",
            "1200,800,300",
            "--orientation-xyzw",
            "0,0,0.7071067811865476,0.7071067811865476",
        ]),
        Scripted::ok(&["preview-numbering", "--store", &store, "--previous", &token]),
    ]);
    let moved = &moved_round[2];
    assert!(matches!(moved.get("stale").unwrap(), Json::Bool(false)));
    assert_eq!(text(at(moved, &["approval"]), "state"), "current");
    // The total length and the B1/B2 membership stay the same, but the finished body changes.
    let changed_round = f.script(&[
        Scripted::ok(&[
            "update-element",
            "--store",
            &store,
            "--id",
            &id,
            "--axis-point",
            "0,0,0;4000,0,0;4000,3000,0",
        ]),
        Scripted::ok(&["preview-numbering", "--store", &store, "--previous", &token]),
    ]);
    let changed = &changed_round[1];
    assert_eq!(groups(changed), groups(&before));
    assert!(matches!(changed.get("stale").unwrap(), Json::Bool(true)));
    assert_eq!(text(at(changed, &["approval"]), "state"), "stale");
    // The stale approval refuses re-approval and leaves the head alone; all three rides are
    // one process.
    let refused_round = f.script(&[
        Scripted::ok(&["open-revision", "--store", &store]),
        Scripted::refused(&[
            "approve-numbering",
            "--store",
            &store,
            "--preview-token",
            &token,
        ]),
        Scripted::ok(&["open-revision", "--store", &store]),
    ]);
    assert_eq!(text(&refused_round[1], "code"), "STALE_WORKSPACE");
    assert_eq!(
        text(&refused_round[0], "revision_id"),
        text(&refused_round[2], "revision_id")
    );
    let copied_round = f.script(&[
        Scripted::ok(&[
            "copy-element",
            "--store",
            &store,
            "--id",
            &id,
            "--offset",
            "0,1000,0",
        ]),
        Scripted::ok(&["preview-numbering", "--store", &store]),
    ]);
    assert_eq!(
        groups(&copied_round[1]),
        [
            ("beam".into(), "B1".into(), 2),
            ("beam".into(), "B2".into(), 1)
        ]
    );
    let token = text(&copied_round[1], "preview_token");
    // Straightening at the same length now joins the straight part's group.
    let straightened_round = f.script(&[
        Scripted::ok(&[
            "update-element",
            "--store",
            &store,
            "--id",
            &id,
            "--axis-point",
            "0,0,0;7000,0,0",
        ]),
        Scripted::ok(&["preview-numbering", "--store", &store, "--previous", &token]),
    ]);
    let straightened = &straightened_round[1];
    assert!(matches!(
        straightened.get("stale").unwrap(),
        Json::Bool(true)
    ));
    assert_eq!(
        groups(straightened),
        [
            ("beam".into(), "B1".into(), 2),
            ("beam".into(), "B2".into(), 1)
        ]
    );
}

#[test]
fn a_member_cut_changes_numbering_without_changing_the_axis_length() {
    let f = Fixture::new();
    let store = f.store();
    let defined = f.script(&[Scripted::ok(&[
        "define-profile",
        "--store",
        &store,
        "--width",
        "100",
        "--height",
        "50",
    ])]);
    let profile = text(at(&defined[0], &["definition"]), "digest");
    let built = f.script(&[
        Scripted::ok(&[
            "create-member",
            "--store",
            &store,
            "--start",
            "0,0,0",
            "--end",
            "7000,0,0",
            "--profile-digest",
            &profile,
        ]),
        Scripted::ok(&[
            "create-member",
            "--store",
            &store,
            "--start",
            "0,0,0",
            "--end",
            "7000,0,0",
            "--profile-digest",
            &profile,
        ]),
    ]);
    let ids: Vec<String> = built
        .iter()
        .map(|reply| text(at(reply, &["member"]), "element_id"))
        .collect();
    let token = text(&f.preview(&[]), "preview_token");
    assert_eq!(groups(&f.preview(&[])), [("beam".into(), "B1".into(), 2)]);
    let cut_round = f.script(&[
        Scripted::ok(&[
            "cut-by-plane",
            "--store",
            &store,
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
        ]),
        Scripted::ok(&["preview-numbering", "--store", &store, "--previous", &token]),
    ]);
    let after = &cut_round[1];
    assert!(matches!(after.get("stale").unwrap(), Json::Bool(true)));
    assert_eq!(
        groups(after),
        [
            ("beam".into(), "B1".into(), 1),
            ("beam".into(), "B2".into(), 1)
        ]
    );
}
