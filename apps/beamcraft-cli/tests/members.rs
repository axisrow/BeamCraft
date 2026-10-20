// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic members, MPL-2.0. Expected volumes and masses are closed forms
// (section area * axis length, volume * density); the saved layer text is read with plain string
// matching and no BeamCraft crate.

//! P03 end to end through the real `beamcraft` binary: `create-member` publishes a beam or a
//! column that pins its profile (and optionally a material) from earlier definitions, and fresh
//! processes reopen it with `plate-inspect`.

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
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("p03-tests");
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
        let store = self.store();
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
        let (code, body) = self.run(command, extra);
        assert_eq!(code, 2, "{command} {extra:?}");
        text(&body, "code")
    }
    fn profile(&self, width: &str, height: &str) -> String {
        let reply = self.ok("define-profile", &["--width", width, "--height", height]);
        text(reply.get("definition").unwrap(), "digest")
    }
    fn material(&self, density: &str) -> String {
        let reply = self.ok("define-material", &["--density", density]);
        text(reply.get("definition").unwrap(), "digest")
    }
    fn authored_text(&self, revision: &Json) -> String {
        let layers = at(revision, &["revision", "manifest", "layers"])
            .array()
            .unwrap();
        let layer = layers
            .iter()
            .find(|l| text(l, "name") == "authored.usda")
            .unwrap();
        let digest = text(layer, "digest");
        let path = self
            .root
            .join("store")
            .join("blobs")
            .join(&digest[..2])
            .join(&digest);
        String::from_utf8(fs::read(path).unwrap()).unwrap()
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
fn num(json: &Json, path: &[&str]) -> f64 {
    at(json, path).float().unwrap()
}

#[test]
fn a_beam_pins_its_profile_and_material_and_reopens_in_a_fresh_process() {
    let f = Fixture::new();
    let profile = f.profile("200", "300");
    let steel = f.material("7850");
    let created = f.ok(
        "create-member",
        &[
            "--kind",
            "beam",
            "--start",
            "0,0,0",
            "--end",
            "6000,0,0",
            "--profile-digest",
            &profile,
            "--material-digest",
            &steel,
            "--name",
            "Beam B1",
        ],
    );
    assert_eq!(text(&created, "outcome"), "member_created");
    let member = at(&created, &["member"]);
    assert_eq!(text(member, "kind"), "beam");
    assert_eq!(text(member, "profile_digest"), profile);
    assert_eq!(num(member, &["section_mm", "width"]), 200.0);
    assert_eq!(num(member, &["section_mm", "height"]), 300.0);
    // 200 x 300 mm section along 6000 mm = 0.36 m3; 0.36 m3 * 7850 kg/m3 = 2826 kg.
    assert_eq!(num(member, &["quantities", "volume", "mm3"]), 360_000_000.0);
    assert_eq!(num(member, &["quantities", "volume", "m3"]), 0.36);
    assert_eq!(num(member, &["quantities", "mass", "kg"]), 2826.0);
    // The profile is read, never written, by the command.
    let reads = at(&created, &["footprint", "reads"]).array().unwrap();
    assert!(
        reads
            .iter()
            .any(|e| text(e, "target") == profile && text(e, "group") == "relationship")
    );
    let writes = at(&created, &["footprint", "writes"]).array().unwrap();
    assert!(writes.iter().all(|e| text(e, "target_kind") == "element"));

    let inspected = f.ok("plate-inspect", &[]);
    assert_eq!(num(&inspected, &["plate_count"]), 0.0);
    assert_eq!(num(&inspected, &["member_count"]), 1.0);
    let reopened = &at(&inspected, &["members"]).array().unwrap()[0];
    assert_eq!(text(reopened, "element_id"), text(member, "element_id"));
    assert_eq!(text(reopened, "mesh_digest"), text(member, "mesh_digest"));
    assert_eq!(num(reopened, &["quantities", "mass", "kg"]), 2826.0);
    let elements = at(&inspected, &["snapshot", "elements"]).array().unwrap();
    assert_eq!(elements.len(), 1);
    // World bounds of the pinned snapshot: x from 0 to 6000, the width spans Y, the height Z.
    let lo: Vec<f64> = at(&elements[0], &["world_min_mm"])
        .array()
        .unwrap()
        .iter()
        .map(|v| v.float().unwrap())
        .collect();
    let hi: Vec<f64> = at(&elements[0], &["world_max_mm"])
        .array()
        .unwrap()
        .iter()
        .map(|v| v.float().unwrap())
        .collect();
    assert_eq!(
        (lo, hi),
        (vec![0.0, -100.0, -150.0], vec![6000.0, 100.0, 150.0])
    );

    // An ordinary reader of the saved text sees a standard Xform with explicit inputs and links.
    let authored = f.authored_text(&created);
    let prim = format!(
        "def Xform \"E_{}\"",
        text(member, "element_id").replace('-', "")
    );
    assert!(authored.contains(&prim), "{authored}");
    for line in [
        "custom token bim:elementKind = \"beam\"",
        "custom double3[] bim:axisPointsMm = [(0.0, 0.0, 0.0), (6000.0, 0.0, 0.0)]",
    ] {
        assert!(authored.contains(line), "{line}\n{authored}");
    }
    assert!(authored.contains(&format!(
        "custom rel bim:profile = </World/Definitions/D_{profile}>"
    )));
    assert!(authored.contains(&format!(
        "custom rel bim:material = </World/Definitions/D_{steel}>"
    )));
}

#[test]
fn a_column_a_roll_and_a_plate_live_in_one_revision() {
    let f = Fixture::new();
    let profile = f.profile("200", "300");
    let column = f.ok(
        "create-member",
        &[
            "--kind",
            "column",
            "--start",
            "0,0,0",
            "--end",
            "0,0,3000",
            "--profile-digest",
            &profile,
            "--roll",
            "1.5707963267948966",
        ],
    );
    assert_eq!(text(at(&column, &["member"]), "kind"), "column");
    assert_eq!(
        num(&column, &["member", "quantities", "volume", "mm3"]),
        180_000_000.0
    );
    // No material: the mass is unknown, never a default.
    assert_eq!(
        text(at(&column, &["member", "quantities", "mass"]), "status"),
        "unknown"
    );
    f.ok(
        "create-plate",
        &["--width", "1000", "--length", "500", "--thickness", "10"],
    );
    let inspected = f.ok("plate-inspect", &[]);
    assert_eq!(num(&inspected, &["plate_count"]), 1.0);
    assert_eq!(num(&inspected, &["member_count"]), 1.0);
    let elements = at(&inspected, &["snapshot", "elements"]).array().unwrap();
    assert_eq!(elements.len(), 2);
    // Rolled a quarter turn: the 200 mm width spans Y and the 300 mm height spans X.
    let member = elements
        .iter()
        .find(|e| text(e, "id") == text(at(&column, &["member"]), "element_id"))
        .unwrap();
    let corner = |key: &str| -> Vec<f64> {
        at(member, &[key])
            .array()
            .unwrap()
            .iter()
            .map(|v| v.float().unwrap())
            .collect()
    };
    let (lo, hi) = (corner("world_min_mm"), corner("world_max_mm"));
    for (got, want) in lo.iter().zip([-150.0, -100.0, 0.0]) {
        assert!((got - want).abs() < 1e-6, "{lo:?}");
    }
    for (got, want) in hi.iter().zip([150.0, 100.0, 3000.0]) {
        assert!((got - want).abs() < 1e-6, "{hi:?}");
    }
}

#[test]
fn rejections_use_the_f04_codes_and_publish_nothing() {
    let f = Fixture::new();
    let profile = f.profile("200", "300");
    let steel = f.material("7850");
    let member = |extra: &[&str]| {
        let mut args = vec!["--start", "0,0,0", "--end", "6000,0,0"];
        args.extend_from_slice(extra);
        f.rejected("create-member", &args)
    };
    assert_eq!(member(&["--profile-digest", &"0".repeat(64)]), "NOT_FOUND");
    assert_eq!(
        member(&["--profile-digest", &steel]),
        "INVALID_ARGUMENT",
        "a material is not a profile"
    );
    assert_eq!(
        member(&["--profile-digest", &profile, "--material-digest", &profile]),
        "INVALID_ARGUMENT",
        "a profile is not a material"
    );
    assert_eq!(
        member(&[
            "--profile-digest",
            &profile,
            "--material-digest",
            &"1".repeat(64)
        ]),
        "NOT_FOUND"
    );
    assert_eq!(
        member(&["--profile-digest", &profile, "--roll", "NaN"]),
        "NON_FINITE_NUMBER"
    );
    assert_eq!(
        member(&["--profile-digest", &profile, "--roll", "7"]),
        "LIMIT_EXCEEDED"
    );
    assert_eq!(
        member(&["--profile-digest", &profile, "--kind", "rafter"]),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.rejected("create-member", &["--start", "0,0,0", "--end", "1,0,0"]),
        "INVALID_ARGUMENT"
    );
    // Zero length and a hostile coordinate.
    assert_eq!(
        f.rejected(
            "create-member",
            &[
                "--start",
                "5,5,5",
                "--end",
                "5,5,5",
                "--profile-digest",
                &profile
            ]
        ),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.rejected(
            "create-member",
            &[
                "--start",
                "0,0,0",
                "--end",
                "2e9,0,0",
                "--profile-digest",
                &profile
            ]
        ),
        "LIMIT_EXCEEDED"
    );
    let inspected = f.ok("plate-inspect", &[]);
    assert_eq!(num(&inspected, &["member_count"]), 0.0);
    // Two definitions only: nothing was created by any rejection.
    assert_eq!(
        text(at(&inspected, &["revision"]), "sequence"),
        "2",
        "{inspected:?}"
    );
}

#[test]
fn replay_reuse_and_a_duplicate_id_across_kinds() {
    let f = Fixture::new();
    let profile = f.profile("200", "300");
    let ids = [
        "--new-id",
        "01a00000-0000-7000-8000-000000000011",
        "--op-id",
        "01a00000-0000-7000-8000-000000000012",
    ];
    let create = |end: &str| {
        let mut args = vec![
            "--start",
            "0,0,0",
            "--end",
            end,
            "--profile-digest",
            profile.as_str(),
        ];
        args.extend_from_slice(&ids);
        f.run("create-member", &args)
    };
    let (code, first) = create("6000,0,0");
    assert_eq!(code, 0);
    assert_eq!(text(&first, "outcome"), "member_created");
    let (code, replay) = create("6000,0,0");
    assert_eq!(code, 0);
    assert_eq!(text(&replay, "outcome"), "member_replayed");
    assert_eq!(
        text(at(&replay, &["member"]), "mesh_digest"),
        text(at(&first, &["member"]), "mesh_digest")
    );
    let (code, reuse) = create("5000,0,0");
    assert_eq!(code, 2);
    assert_eq!(text(&reuse, "code"), "OP_ID_REUSE");
    // The same id cannot be a plate: element ids are unique across kinds.
    assert_eq!(
        f.rejected(
            "create-plate",
            &[
                "--width",
                "1000",
                "--length",
                "500",
                "--thickness",
                "10",
                "--new-id",
                "01a00000-0000-7000-8000-000000000011",
            ]
        ),
        "DUPLICATE_ID"
    );
}

#[test]
fn a_member_whose_mesh_would_collapse_in_float32_is_refused() {
    let f = Fixture::new();
    let profile = f.profile("1", "1");
    let args = [
        "--start",
        "100000000,0,0",
        "--end",
        "100000000,0,1000",
        "--profile-digest",
        profile.as_str(),
    ];
    assert_eq!(f.rejected("create-member", &args), "UNSUPPORTED_GEOMETRY");
    // A normal offset of the same size of member is fine.
    let ok = [
        "--start",
        "1000,0,0",
        "--end",
        "1000,0,1000",
        "--profile-digest",
        profile.as_str(),
    ];
    f.ok("create-member", &ok);
}

#[test]
fn a_member_above_one_cubic_metre_is_within_the_stage_budget() {
    // 200 mm x 300 mm over 20 m is 1.2 m3 = 1.2e9 mm3 of stored volume: the bounded read used
    // to refuse any double above 1e9, so this very member could not be created at all (the
    // same reproduction the issue reported).
    let f = Fixture::new();
    let profile = f.profile("200", "300");
    let args = [
        "--kind",
        "beam",
        "--start",
        "0,0,0",
        "--end",
        "20000,0,0",
        "--profile-digest",
        profile.as_str(),
    ];
    let reply = f.ok("create-member", &args);
    assert_eq!(text(&reply, "outcome"), "member_created");
    assert_eq!(
        num(at(&reply, &["member", "quantities", "volume"]), &["mm3"]),
        1_200_000_000.0
    );
    // A fresh process reopens the revision and keeps the quantity current.
    let inspected = f.ok("plate-inspect", &[]);
    let members = inspected.get("members").unwrap().array().unwrap();
    assert_eq!(members.len(), 1);
}
