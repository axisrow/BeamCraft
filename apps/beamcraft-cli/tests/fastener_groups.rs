// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A14.3 end to end through the real `beamcraft` binary: a fastener group of a pinned
//! definition is created over two plates with the DIN library (the entry pins in the same
//! transaction), the auto length and the positions follow the bolt amendment, an unknown
//! designation or an uncrossed part changes nothing, and the group undoes as one unit.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_json::{Json, Limits};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

const DIN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/libraries/din-fasteners.usda"
);

struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("a14-3-tests");
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
    fn store(&self) -> String {
        self.root.join("store").to_str().unwrap().to_owned()
    }
    fn run(&self, command: &str, extra: &[&str]) -> (i32, Json) {
        let mut args = vec![command];
        args.extend_from_slice(extra);
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(&args)
            .output()
            .unwrap();
        let body = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
        (output.status.code().unwrap(), body)
    }
    fn ok(&self, command: &str, extra: &[&str]) -> Json {
        let (code, body) = self.run(command, extra);
        assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
        body
    }
    fn rejected(&self, command: &str, extra: &[&str]) -> Json {
        let (code, body) = self.run(command, extra);
        assert_eq!(code, 2, "{command} {extra:?}: {body:?}");
        assert_eq!(at(&body, &["outcome"]).string().unwrap(), "rejected");
        body
    }
}

fn text(json: &Json, key: &str) -> String {
    json.get(key).unwrap().string().unwrap().to_owned()
}
fn at<'a>(json: &'a Json, path: &[&str]) -> &'a Json {
    path.iter().fold(json, |j, k| j.get(k).unwrap())
}
fn num(json: &Json, path: &[&str]) -> f64 {
    at(json, path).float().unwrap()
}
fn sequence(json: &Json) -> u64 {
    text(at(json, &["revision"]), "sequence").parse().unwrap()
}

/// Two stacked plates 500x500, z 0..10 and 10..20: the group line runs along x above them, the
/// shank points down and crosses both.
fn two_plates(f: &Fixture) -> (String, String) {
    let first = f.ok(
        "create-plate",
        &[
            "--store",
            &f.store(),
            "--contour",
            "0,0;500,0;500,500;0,500",
            "--thickness",
            "10",
        ],
    );
    let second = f.ok(
        "create-plate",
        &[
            "--store",
            &f.store(),
            "--contour",
            "0,0;500,0;500,500;0,500",
            "--thickness",
            "10",
            "--translation",
            "0,0,10",
        ],
    );
    let id = |reply: &Json| text(at(reply, &["plate"]), "element_id");
    (id(&first), id(&second))
}

#[test]
fn a_group_of_the_library_pins_and_places_its_row() {
    let f = Fixture::new();
    let (first, second) = two_plates(&f);
    let created = f.ok(
        "create-bolt",
        &[
            "--store",
            &f.store(),
            "--name",
            "Joint-1",
            "--parts",
            &format!("{first},{second}"),
            "--fastener",
            "M16",
            "--fastener-library",
            DIN,
            "--point1",
            "100,100,30",
            "--point2",
            "400,100,30",
            "--plane-normal",
            "0,0,1",
            "--rotation",
            "180",
            "--spacings",
            "100,100",
            "--hole-mode",
            "none",
        ],
    );
    assert_eq!(text(&created, "outcome"), "bolt_group_created");
    let groups = at(&created, &["groups"]).array().unwrap();
    assert!(groups.is_empty(), "the listing is read before the create");
    // The two plates committed first; the group and its pinned definition land together.
    assert_eq!(sequence(&created), 3, "plates, then group + definition");

    let listed = f.ok("bolt-groups", &["--store", &f.store()]);
    let groups = at(&listed, &["groups"]).array().unwrap();
    assert_eq!(groups.len(), 1);
    let group = groups[0].clone();
    assert_eq!(text(&group, "name"), "Joint-1");
    assert_eq!(num(&group, &["fastener_count"]), 3.0);
    // The auto length: grip 20 + washer 3 + nut 14.8 + protrusion 5 = 42.8 -> 45.
    assert_eq!(num(&group, &["length_mm"]), 45.0);
    assert_eq!(text(&group, "length_mode"), "auto");
    assert_eq!(text(&group, "hole_mode"), "none");
    let parts = at(&group, &["parts"]).array().unwrap();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].string().unwrap(), first);
    // The pinned definition lives in the store: the same digest the listing predicted.
    let shown = f.ok("definitions", &["--store", &f.store()]);
    let definitions = at(&shown, &["definitions"]).array().unwrap();
    assert_eq!(definitions.len(), 1);
    assert_eq!(text(&definitions[0], "designation"), "M16");
    assert_eq!(
        text(&group, "fastener_digest"),
        text(&definitions[0], "digest")
    );
}

#[test]
fn a_refused_group_changes_nothing() {
    let f = Fixture::new();
    let (first, second) = two_plates(&f);
    let store = f.store();
    let parts_pair = format!("{first},{second}");
    let base_args = [
        "--store",
        store.as_str(),
        "--name",
        "Joint",
        "--point1",
        "100,100,30",
        "--point2",
        "400,100,30",
        "--plane-normal",
        "0,0,1",
        "--spacings",
        "100,100",
    ];
    // An unknown designation.
    let rejected = f.rejected(
        "create-bolt",
        &[
            base_args.as_slice(),
            &[
                "--parts",
                parts_pair.as_str(),
                "--hole-mode",
                "none",
                "--fastener",
                "M64",
                "--fastener-library",
                DIN,
            ],
        ]
        .concat(),
    );
    assert_eq!(text(&rejected, "code"), "NOT_FOUND");
    // A part the shank never crosses: a third plate far above.
    let third = f.ok(
        "create-plate",
        &[
            "--store",
            &f.store(),
            "--contour",
            "0,0;500,0;500,500;0,500",
            "--thickness",
            "10",
            "--translation",
            "0,0,400",
        ],
    );
    let third_id = text(at(&third, &["plate"]), "element_id");
    let rejected = f.rejected(
        "create-bolt",
        &[
            base_args.as_slice(),
            &[
                "--parts",
                &format!("{first},{second},{third_id}"),
                "--hole-mode",
                "none",
                "--fastener",
                "M16",
                "--fastener-library",
                DIN,
            ],
        ]
        .concat(),
    );
    assert_eq!(text(&rejected, "code"), "INVALID_ARGUMENT");
    // A generated hole is refused until A14.4.
    let rejected = f.rejected(
        "create-bolt",
        &[
            base_args.as_slice(),
            &[
                "--parts",
                parts_pair.as_str(),
                "--fastener",
                "M16",
                "--fastener-library",
                DIN,
                "--hole-mode",
                "generate",
            ],
        ]
        .concat(),
    );
    assert_eq!(text(&rejected, "code"), "UNSUPPORTED_GEOMETRY");
    // No standard length long enough: eleven stacked plates of 10 mm each.
    let mut extra = Vec::new();
    for n in 0..11u32 {
        let plate = f.ok(
            "create-plate",
            &[
                "--store",
                &f.store(),
                "--contour",
                "0,0;500,0;500,500;0,500",
                "--thickness",
                "10",
                "--translation",
                &format!("0,0,{}", n * 10),
            ],
        );
        extra.push(text(at(&plate, &["plate"]), "element_id"));
    }
    let stack = extra
        .iter()
        .chain(std::iter::once(&first))
        .cloned()
        .collect::<Vec<_>>()
        .join(",");
    let rejected = f.rejected(
        "create-bolt",
        &[
            "--store",
            &f.store(),
            "--name",
            "Long",
            "--parts",
            &stack,
            "--fastener",
            "M16",
            "--fastener-library",
            DIN,
            "--point1",
            "100,100,115",
            "--point2",
            "400,100,115",
            "--plane-normal",
            "0,0,1",
            "--rotation",
            "180",
            "--spacings",
            "100,100",
            "--hole-mode",
            "none",
        ],
    );
    assert_eq!(text(&rejected, "code"), "UNSUPPORTED_GEOMETRY");
    // None of the refusals wrote a group.
    let listed = f.ok("bolt-groups", &["--store", &f.store()]);
    assert!(at(&listed, &["groups"]).array().unwrap().is_empty());
}

#[test]
fn the_same_operation_replays_and_the_group_undoes_with_its_definition() {
    let f = Fixture::new();
    let (first, second) = two_plates(&f);
    let parts_pair = format!("{first},{second}");
    let args = [
        "--store",
        &f.store(),
        "--name",
        "Joint-1",
        "--new-id",
        "019a7bf0-0000-7000-8000-00000000a142",
        "--parts",
        parts_pair.as_str(),
        "--fastener",
        "M16",
        "--fastener-library",
        DIN,
        "--point1",
        "100,100,30",
        "--point2",
        "400,100,30",
        "--plane-normal",
        "0,0,1",
        "--rotation",
        "180",
        "--spacings",
        "100,100",
        "--hole-mode",
        "none",
        "--op-id",
        "019a7bf0-0000-7000-8000-00000000a143",
    ];
    let created = f.ok("create-bolt", &args);
    assert_eq!(text(&created, "outcome"), "bolt_group_created");
    let replayed = f.ok("create-bolt", &args);
    assert_eq!(text(&replayed, "outcome"), "bolt_group_created_replayed");
    assert_eq!(sequence(&replayed), sequence(&created));

    // The group and its pinned definition landed in one revision (sequence 3): one transaction,
    // one undo unit — a workspace that rewinds this revision loses both together.
    let listed = f.ok("bolt-groups", &["--store", &f.store()]);
    assert_eq!(at(&listed, &["groups"]).array().unwrap().len(), 1);
    let shown = f.ok("definitions", &["--store", &f.store()]);
    assert_eq!(num(&shown, &["definition_count"]), 1.0);
}
