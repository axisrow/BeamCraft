// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic members, MPL-2.0. Expected volumes and masses are closed forms
// (section area * axis length, volume * density); the saved layer text is read with plain string
// matching and no BeamCraft crate.

//! P03 end to end through the real `beamcraft` binary: `create-member` publishes a beam or a
//! column that pins its profile (and optionally a material) from earlier definitions, and fresh
//! processes reopen it with `plate-inspect`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_json::{Json, Limits};
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
        let body = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
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
        "def Xform \"PRT_{}\"",
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
        "custom rel bim:profile = </World/Definitions/Profiles/PRF_{profile}>"
    )));
    assert!(authored.contains(&format!(
        "custom rel bim:material = </World/Definitions/Materials/MAT_{steel}>"
    )));
}

#[test]
fn a_rotated_member_reopens_with_world_bounds_and_unchanged_shape() {
    let f = Fixture::new();
    let profile = f.profile("200", "300");
    let created = f.ok(
        "create-member",
        &[
            "--kind",
            "beam",
            "--start",
            "0,0,0",
            "--end",
            "4000,0,0",
            "--profile-digest",
            &profile,
            "--orientation-xyzw",
            "0,0,0.7071067811865476,0.7071067811865476",
            "--translation",
            "-50,70,1000",
        ],
    );
    let member = at(&created, &["member"]);
    assert_eq!(num(member, &["quantities", "volume", "mm3"]), 240_000_000.0);
    let digest = text(member, "mesh_digest");

    // A fresh CLI process reads the committed USD and rebuilds the view placement.
    let reopened = f.ok("plate-inspect", &[]);
    let member = &at(&reopened, &["members"]).array().unwrap()[0];
    assert_eq!(text(member, "mesh_digest"), digest);
    assert_eq!(num(member, &["quantities", "volume", "mm3"]), 240_000_000.0);
    let element = &at(&reopened, &["snapshot", "elements"]).array().unwrap()[0];
    let vector = |key| {
        at(element, &[key])
            .array()
            .unwrap()
            .iter()
            .map(|v| v.float().unwrap())
            .collect::<Vec<_>>()
    };
    for (actual, expected) in vector("world_min_mm").iter().zip([-150.0, 70.0, 850.0]) {
        assert!((actual - expected).abs() < 1e-8, "{actual} != {expected}");
    }
    for (actual, expected) in vector("world_max_mm").iter().zip([50.0, 4070.0, 1150.0]) {
        assert!((actual - expected).abs() < 1e-8, "{actual} != {expected}");
    }

    let member_id = text(member, "element_id");
    let feature_id = format!("fid1:{member_id}:member:v.x1.y0.z0");
    let feature = f.ok("feature-lookup", &["--feature-id", &feature_id]);
    assert_eq!(text(&feature, "feature_kind"), "vertex");
    let vector = |space| {
        at(&feature, &[space, "position"])
            .array()
            .unwrap()
            .iter()
            .map(|v| v.float().unwrap())
            .collect::<Vec<_>>()
    };
    let local = vector("local");
    let world = vector("world");
    // The member mesh carries the placement rotation (issue #130), so the snapshot places it
    // by the translation only and the world position is the rotated local plus the translation.
    let expected = [local[0] - 50.0, local[1] + 70.0, local[2] + 1000.0];
    for (actual, expected) in world.iter().zip(expected) {
        assert!((actual - expected).abs() < 1e-8, "{actual} != {expected}");
    }
    let picked = f.ok(
        "pick",
        &[
            "--origin",
            "100,2000,1000",
            "--direction",
            "-1,0,0",
            "--targets",
            "face",
        ],
    );
    let hit = &at(&picked, &["hits"]).array().unwrap()[0];
    assert!(text(hit, "feature_id").starts_with(&format!("fid1:{member_id}:member:")));
    let point = at(hit, &["point_world_mm"])
        .array()
        .unwrap()
        .iter()
        .map(|v| v.float().unwrap())
        .collect::<Vec<_>>();
    for (actual, expected) in point.iter().zip([50.0, 2000.0, 1000.0]) {
        assert!((actual - expected).abs() < 1e-8, "{actual} != {expected}");
    }

    // Independently inspect the saved USDA text for the authored transform quaternion.
    let authored = f.authored_text(&created);
    assert!(
        authored
            .contains("quatd xformOp:orient = (0.7071067811865476, 0.0, 0.0, 0.7071067811865476)"),
        "{authored}"
    );
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
fn hostile_orientation_input_is_refused_and_publishes_nothing() {
    let f = Fixture::new();
    let profile = f.profile("200", "300");
    let create = |orientation: &str| {
        f.rejected(
            "create-member",
            &[
                "--start",
                "0,0,0",
                "--end",
                "6000,0,0",
                "--profile-digest",
                profile.as_str(),
                "--orientation-xyzw",
                orientation,
            ],
        )
    };
    // Hostile create-member input: non-normalized, zero, extra components and NaN. Every
    // refusal is a typed exit, never a crash, and nothing is written.
    assert_eq!(create("0,0,0.5,0.5"), "INVALID_ARGUMENT");
    assert_eq!(create("0,0,0,0"), "INVALID_ARGUMENT");
    assert_eq!(
        create("0,0,0.7071067811865476,0.7071067811865476,1"),
        "INVALID_ARGUMENT"
    );
    assert_eq!(create("0,NaN,0,1"), "NON_FINITE_NUMBER");
    assert_eq!(num(&f.ok("plate-inspect", &[]), &["member_count"]), 0.0);

    // The same input on update-element, against a member that exists: the head stays where it
    // was and the stored orientation is untouched.
    let created = f.ok(
        "create-member",
        &[
            "--start",
            "0,0,0",
            "--end",
            "6000,0,0",
            "--profile-digest",
            profile.as_str(),
            "--new-id",
            "01a00000-0000-7000-8000-000000000031",
        ],
    );
    let member_id = text(at(&created, &["member"]), "element_id");
    let before = f.ok("plate-inspect", &[]);
    let head = text(at(&before, &["revision"]), "revision_id");
    let update = |orientation: &str| {
        f.rejected(
            "update-element",
            &[
                "--id",
                member_id.as_str(),
                "--orientation-xyzw",
                orientation,
            ],
        )
    };
    assert_eq!(update("0,0,0.5,0.5"), "INVALID_ARGUMENT");
    assert_eq!(update("0,0,0,0"), "INVALID_ARGUMENT");
    assert_eq!(
        update("0,0,0.7071067811865476,0.7071067811865476,1"),
        "INVALID_ARGUMENT"
    );
    assert_eq!(update("NaN,0,0,1"), "NON_FINITE_NUMBER");
    let after = f.ok("plate-inspect", &[]);
    assert_eq!(
        text(at(&after, &["revision"]), "revision_id"),
        head,
        "a refusal must not commit"
    );
    let member = &at(&after, &["members"]).array().unwrap()[0];
    let quaternion: Vec<f64> = at(member, &["placement", "orientation_xyzw"])
        .array()
        .unwrap()
        .iter()
        .map(|v| v.float().unwrap())
        .collect();
    for (actual, expected) in quaternion.iter().zip([0.0, 0.0, 0.0, 1.0]) {
        assert!((actual - expected).abs() < 1e-12, "{quaternion:?}");
    }
}

#[test]
fn an_orientation_update_sets_clears_and_replays() {
    let f = Fixture::new();
    let profile = f.profile("200", "300");
    let created = f.ok(
        "create-member",
        &[
            "--start",
            "0,0,0",
            "--end",
            "6000,0,0",
            "--profile-digest",
            profile.as_str(),
            "--new-id",
            "01a00000-0000-7000-8000-000000000041",
        ],
    );
    let member = at(&created, &["member"]);
    let member_id = text(member, "element_id");
    let plain = text(member, "mesh_digest");

    // Set: the stored orientation is the quarter turn and the mesh carries the rotation.
    let quarter = "0,0,0.7071067811865476,0.7071067811865476";
    let set = |op: &str| {
        f.ok(
            "update-element",
            &[
                "--id",
                member_id.as_str(),
                "--orientation-xyzw",
                quarter,
                "--op-id",
                op,
            ],
        )
    };
    let turned = set("01a00000-0000-7000-8000-000000000042");
    assert_eq!(text(&turned, "outcome"), "element_updated");
    assert_eq!(at(&turned, &["effect", "orientation"]), &Json::Bool(true));
    assert_eq!(at(&turned, &["effect", "placement"]), &Json::Bool(false));
    let inspected = f.ok("plate-inspect", &[]);
    let member = &at(&inspected, &["members"]).array().unwrap()[0];
    let quaternion: Vec<f64> = at(member, &["placement", "orientation_xyzw"])
        .array()
        .unwrap()
        .iter()
        .map(|v| v.float().unwrap())
        .collect();
    for (actual, expected) in quaternion
        .iter()
        .zip([0.0, 0.0, 0.5_f64.sqrt(), 0.5_f64.sqrt()])
    {
        assert!((actual - expected).abs() < 1e-8, "{quaternion:?}");
    }
    assert_ne!(text(member, "mesh_digest"), plain);
    let translation: Vec<f64> = at(member, &["placement", "translation_mm"])
        .array()
        .unwrap()
        .iter()
        .map(|v| v.float().unwrap())
        .collect();
    assert_eq!(
        translation, [0.0; 3],
        "an orientation change keeps the translation"
    );

    // An immediate retry of the same --op-id is a replay of the same state.
    let replayed = set("01a00000-0000-7000-8000-000000000042");
    assert_eq!(text(&replayed, "outcome"), "element_update_replayed");

    // Clear: none resets the rotation to the identity and the body to the unrotated mesh.
    let cleared = f.ok(
        "update-element",
        &[
            "--id",
            member_id.as_str(),
            "--orientation-xyzw",
            "none",
            "--op-id",
            "01a00000-0000-7000-8000-000000000043",
        ],
    );
    assert_eq!(text(&cleared, "outcome"), "element_updated");
    let member = at(&cleared, &["element"]);
    let quaternion: Vec<f64> = at(member, &["placement", "orientation_xyzw"])
        .array()
        .unwrap()
        .iter()
        .map(|v| v.float().unwrap())
        .collect();
    assert_eq!(quaternion, [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(
        text(member, "mesh_digest"),
        plain,
        "the identity rebuild is the original mesh"
    );
}

#[test]
fn a_rotated_member_replays_identically_on_the_same_op_id() {
    let f = Fixture::new();
    let profile = f.profile("200", "300");
    let args = [
        "--start",
        "0,0,0",
        "--end",
        "4000,0,0",
        "--profile-digest",
        profile.as_str(),
        "--orientation-xyzw",
        "0,0,0.7071067811865476,0.7071067811865476",
        "--new-id",
        "01a00000-0000-7000-8000-000000000051",
        "--op-id",
        "01a00000-0000-7000-8000-000000000052",
    ];
    let first = f.ok("create-member", &args);
    assert_eq!(text(&first, "outcome"), "member_created");
    let replay = f.ok("create-member", &args);
    assert_eq!(text(&replay, "outcome"), "member_replayed");
    assert_eq!(
        text(at(&replay, &["member"]), "mesh_digest"),
        text(at(&first, &["member"]), "mesh_digest")
    );
    assert_eq!(
        text(at(&replay, &["member"]), "element_id"),
        text(at(&first, &["member"]), "element_id")
    );
    // The rotated state is what a fresh process reports for the stored orientation.
    let inspected = f.ok("plate-inspect", &[]);
    let member = &at(&inspected, &["members"]).array().unwrap()[0];
    let quaternion: Vec<f64> = at(member, &["placement", "orientation_xyzw"])
        .array()
        .unwrap()
        .iter()
        .map(|v| v.float().unwrap())
        .collect();
    for (actual, expected) in quaternion
        .iter()
        .zip([0.0, 0.0, 0.5_f64.sqrt(), 0.5_f64.sqrt()])
    {
        assert!((actual - expected).abs() < 1e-8, "{quaternion:?}");
    }
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
