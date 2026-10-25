// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic fixtures, MPL-2.0. Expected volumes are closed forms. The saved
// layer text is read with plain string matching and no BeamCraft crate.

//! A03 (first slice) plane cuts end to end through the real `beamcraft` binary:
//! `create-plate`/`create-member --cut-point --cut-normal` author one plane cut per element,
//! the kernel trims the body after any holes, and fresh processes reopen and re-verify.

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
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("a03-tests");
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
    fn plate(&self, extra: &[&str]) -> (i32, Json) {
        let mut args = vec!["--width", "1000", "--length", "500", "--thickness", "10"];
        args.extend_from_slice(extra);
        self.run("create-plate", &args)
    }
    fn ok_plate(&self, extra: &[&str]) -> Json {
        let (code, body) = self.plate(extra);
        assert_eq!(code, 0, "{extra:?}: {body:?}");
        body
    }
    fn rejected(&self, extra: &[&str]) -> String {
        let (code, body) = self.plate(extra);
        assert_eq!(code, 2, "{extra:?}");
        text(&body, "code")
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

const CUT: [&str; 4] = ["--cut-point", "250,0,0", "--cut-normal", "1,0,0"];

#[test]
fn a_cut_plate_has_the_exact_volume_and_mass_and_reopens_verified() {
    let f = Fixture::new();
    let steel = {
        let (_, reply) = f.run("define-material", &["--density", "7850"]);
        text(reply.get("definition").unwrap(), "digest")
    };
    let mut args: Vec<&str> = CUT.to_vec();
    args.extend_from_slice(&["--material-digest", &steel]);
    let created = f.ok_plate(&args);
    let plate = at(&created, &["plate"]);
    // The kept half-space is x <= 250: 250 * 500 * 10 = 1,250,000 mm3; * 7850 kg/m3.
    let volume = num(plate, &["quantities", "volume", "mm3"]);
    assert!((volume - 1_250_000.0).abs() <= 1e-6, "{volume}");
    let mass = num(plate, &["quantities", "mass", "kg"]);
    assert!((mass - 1_250_000.0 * 7850.0 / 1e9).abs() <= 1e-9, "{mass}");
    assert_eq!(
        text(at(plate, &["quantities", "volume"]), "algorithm"),
        "kernel-a03-plane-cut-volume/1;tolerance-policy-3"
    );
    assert!(num(plate, &["triangles"]) > 12.0);
    let point: Vec<f64> = at(plate, &["cut", "point_mm"])
        .array()
        .unwrap()
        .iter()
        .map(|v| v.float().unwrap())
        .collect();
    assert_eq!(point, vec![250.0, 0.0, 0.0]);

    // A fresh process re-evaluates the authored cut and agrees.
    let inspected = f.run("inspect-elements", &[]).1;
    let reopened = &at(&inspected, &["plates"]).array().unwrap()[0];
    assert_eq!(text(reopened, "mesh_digest"), text(plate, "mesh_digest"));
    assert_eq!(num(reopened, &["quantities", "volume", "mm3"]), volume);
    let cut_point = |plate: &Json| {
        at(plate, &["cut", "point_mm"]).array().unwrap()[0]
            .float()
            .unwrap()
    };
    assert_eq!(cut_point(reopened), 250.0);

    // An ordinary reader of the saved text sees the cut as plain doubles.
    let authored = f.authored_text(&created);
    assert!(
        authored.contains("custom double3 steel:cut:pointMm = (250.0, 0.0, 0.0)"),
        "{authored}"
    );
    assert!(
        authored.contains("custom double3 steel:cut:normal = (1.0, 0.0, 0.0)"),
        "{authored}"
    );
}

#[test]
fn a_cut_beam_is_trimmed_and_a_cut_after_holes_keeps_both() {
    let f = Fixture::new();
    let profile = {
        let (_, reply) = f.run("define-profile", &["--width", "200", "--height", "300"]);
        text(reply.get("definition").unwrap(), "digest")
    };
    // The beam is cut at its mid-length, keeping the first 3000 mm.
    let (code, beam) = {
        let args: Vec<&str> = vec![
            "--start",
            "0,0,0",
            "--end",
            "6000,0,0",
            "--profile-digest",
            &profile,
            "--cut-point",
            "3000,0,0",
            "--cut-normal",
            "1,0,0",
        ];
        f.run("create-member", &args)
    };
    assert_eq!(code, 0, "{beam:?}");
    let member = at(&beam, &["member"]);
    // The kept half is 3000 mm long: 3000 * 200 * 300 = 180,000,000 mm3 = 0.18 m3.
    let volume = num(member, &["quantities", "volume", "mm3"]);
    assert!((volume - 180_000_000.0).abs() <= 1e-6, "{volume}");
    assert!(num(member, &["triangles"]) > 12.0);

    // A plate cut after a hole keeps both effects; the hole lies inside the kept part.
    let mut args: Vec<&str> = vec!["--hole-rect", "100,100,100,100"];
    args.extend_from_slice(&CUT);
    let both = f.ok_plate(&args);
    let plate = at(&both, &["plate"]);
    // (250 * 500 - 100 * 100) * 10 = 1,150,000 mm3.
    let volume = num(plate, &["quantities", "volume", "mm3"]);
    assert!((volume - 1_150_000.0).abs() <= 1e-6, "{volume}");
}

#[test]
fn a_solid_element_authors_no_cut_attributes() {
    let f = Fixture::new();
    let solid = f.ok_plate(&[]);
    assert!(!f.authored_text(&solid).contains("steel:cut"));
    assert_eq!(at(&solid, &["plate", "cut"]), &Json::Null);
}

#[test]
fn cut_rejections_use_the_f04_codes_and_publish_nothing() {
    let f = Fixture::new();
    // The plane misses the plate entirely.
    assert_eq!(
        f.rejected(&["--cut-point", "2000,0,0", "--cut-normal", "1,0,0"]),
        "INVALID_ARGUMENT"
    );
    // The plane only touches the bottom face: a tangent cut, not a trim.
    assert_eq!(
        f.rejected(&["--cut-point", "0,0,0", "--cut-normal", "0,0,1"]),
        "UNSUPPORTED_GEOMETRY"
    );
    // A zero normal never reaches the kernel.
    assert_eq!(
        f.rejected(&["--cut-point", "0,0,0", "--cut-normal", "0,0,0"]),
        "INVALID_ARGUMENT"
    );
    // One option without the other is refused at the CLI.
    assert_eq!(f.rejected(&["--cut-point", "250,0,0"]), "INVALID_ARGUMENT");
    assert_eq!(f.rejected(&["--cut-normal", "1,0,0"]), "INVALID_ARGUMENT");
    // Malformed, non-finite and out-of-cap values.
    assert_eq!(
        f.rejected(&["--cut-point", "250,0", "--cut-normal", "1,0,0"]),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.rejected(&["--cut-point", "NaN,0,0", "--cut-normal", "1,0,0"]),
        "NON_FINITE_NUMBER"
    );
    assert_eq!(
        f.rejected(&["--cut-point", "2e9,0,0", "--cut-normal", "1,0,0"]),
        "LIMIT_EXCEEDED"
    );
    // Nothing was created: there is no revision at all.
    let (code, body) = f.run("inspect-elements", &[]);
    assert_eq!(code, 2, "{body:?}");
}

#[test]
fn a_retry_with_the_same_cut_replays_and_another_cut_is_reuse() {
    let f = Fixture::new();
    let ids = [
        "--new-id",
        "01a00000-0000-7000-8000-000000000031",
        "--op-id",
        "01a00000-0000-7000-8000-000000000032",
    ];
    let create = |point: &str| {
        let mut args = vec!["--cut-point", point, "--cut-normal", "1,0,0"];
        args.extend_from_slice(&ids);
        f.plate(&args)
    };
    let (code, first) = create("250,0,0");
    assert_eq!(code, 0);
    assert_eq!(text(&first, "outcome"), "plate_created");
    let (code, replay) = create("250,0,0");
    assert_eq!(code, 0);
    assert_eq!(text(&replay, "outcome"), "plate_replayed");
    assert_eq!(
        text(at(&replay, &["plate"]), "mesh_digest"),
        text(at(&first, &["plate"]), "mesh_digest")
    );
    let (code, reuse) = create("300,0,0");
    assert_eq!(code, 2);
    assert_eq!(text(&reuse, "code"), "OP_ID_REUSE");
}

#[test]
fn a_copy_carries_the_cut_and_the_copy_is_cut_too() {
    let f = Fixture::new();
    let source = f.ok_plate(&CUT);
    let source_id = text(at(&source, &["plate"]), "element_id");
    let (code, copy) = f.run(
        "copy-element",
        &["--id", &source_id, "--offset", "0,1000,0"],
    );
    assert_eq!(code, 0, "{copy:?}");
    let copy_id = text(at(&copy, &["element"]), "element_id");
    assert_ne!(copy_id, source_id);
    // The copy has the same body (the same mesh digest) and reports the cut.
    assert_eq!(
        text(at(&copy, &["element"]), "mesh_digest"),
        text(at(&source, &["plate"]), "mesh_digest")
    );
    assert_eq!(
        at(&copy, &["element", "cut", "point_mm"]).array().unwrap()[0]
            .float()
            .unwrap(),
        250.0
    );
    // A fresh process re-verifies both elements against their authored cuts.
    let inspected = f.run("inspect-elements", &[]).1;
    assert_eq!(num(&inspected, &["plate_count"]), 2.0);
    let plates = at(&inspected, &["plates"]).array().unwrap();
    for plate in plates {
        let volume = num(plate, &["quantities", "volume", "mm3"]);
        assert!((volume - 1_250_000.0).abs() <= 1e-6, "{volume}");
        assert_eq!(
            at(plate, &["cut", "point_mm"]).array().unwrap()[0]
                .float()
                .unwrap(),
            250.0
        );
    }
}
