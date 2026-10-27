// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic members, MPL-2.0. Expected volumes and masses are closed forms
// (section area * polyline path length, volume * density); the saved layer text is read with
// plain string matching and no BeamCraft crate.

//! P12 end to end through the real `beamcraft` binary: `create-polybeam` publishes a member
//! along a polyline path with one section, the interior corners are mitred, and
//! `update-element --axis-point` replaces the path. Fresh processes reopen every revision.

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
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("p12-tests");
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

const L_PATH: &str = "0,0,0;3000,0,0;3000,4000,0";
const STRAIGHT_PATH: &str = "0,0,0;10000,0,0";

#[test]
fn a_polybeam_sweeps_one_section_along_an_l_path_and_reopens() {
    let f = Fixture::new();
    let profile = f.profile("200", "300");
    let steel = f.material("7850");
    let created = f.ok(
        "create-polybeam",
        &[
            "--points",
            L_PATH,
            "--profile-digest",
            &profile,
            "--material-digest",
            &steel,
            "--name",
            "Gutter PB1",
        ],
    );
    assert_eq!(text(&created, "outcome"), "polybeam_created");
    let member = at(&created, &["member"]);
    assert_eq!(text(member, "kind"), "beam");
    assert_eq!(text(member, "profile_digest"), profile);
    assert_eq!(num(member, &["section_mm", "width"]), 200.0);
    assert_eq!(num(member, &["section_mm", "height"]), 300.0);
    // P12 acceptance: the mitre drops out of the quantity. 200 x 300 over the 3000 + 4000 mm
    // path = 0.42 m3; 0.42 m3 * 7850 kg/m3 = 3297 kg.
    assert_eq!(num(member, &["quantities", "volume", "mm3"]), 420_000_000.0);
    assert_eq!(num(member, &["quantities", "volume", "m3"]), 0.42);
    assert_eq!(num(member, &["quantities", "mass", "kg"]), 3297.0);
    let points = at(member, &["axis_points_mm"]).array().unwrap();
    assert_eq!(points.len(), 3);

    // A fresh process reopens the same revision with the same mesh and quantities.
    let inspected = f.ok("inspect-elements", &[]);
    assert_eq!(num(&inspected, &["member_count"]), 1.0);
    let reopened = &at(&inspected, &["members"]).array().unwrap()[0];
    assert_eq!(text(reopened, "element_id"), text(member, "element_id"));
    assert_eq!(text(reopened, "mesh_digest"), text(member, "mesh_digest"));
    assert_eq!(num(reopened, &["quantities", "mass", "kg"]), 3297.0);

    // The authored layer stores the whole polyline in the one axis attribute.
    let authored = f.authored_text(&created);
    assert!(
        authored.contains(
            "custom double3[] bim:axisPointsMm = [(0.0, 0.0, 0.0), (3000.0, 0.0, 0.0), (3000.0, 4000.0, 0.0)]"
        ),
        "{authored}"
    );
}

#[test]
fn a_path_patch_replaces_the_polyline_and_the_quantities_follow() {
    let f = Fixture::new();
    let profile = f.profile("200", "300");
    let created = f.ok(
        "create-polybeam",
        &["--points", L_PATH, "--profile-digest", &profile],
    );
    let element = text(at(&created, &["member"]), "element_id");
    // A straight member edited from the polybeam: 200 x 300 over 10000 mm = 0.6 m3. The
    // polybeam had no roll, so the straightened member keeps the same frame rule.
    let updated = f.ok(
        "update-element",
        &["--id", &element, "--axis-point", STRAIGHT_PATH],
    );
    let member = at(&updated, &["element"]);
    assert_eq!(num(member, &["quantities", "volume", "mm3"]), 600_000_000.0);
    let points = at(member, &["axis_points_mm"]).array().unwrap();
    assert_eq!(points.len(), 2);
    // A fresh process verifies the rewritten mesh and quantities.
    let inspected = f.ok("inspect-elements", &[]);
    let reopened = &at(&inspected, &["members"]).array().unwrap()[0];
    assert_eq!(
        num(reopened, &["quantities", "volume", "mm3"]),
        600_000_000.0
    );
}

#[test]
fn a_polybeam_refuses_a_two_point_or_degenerate_path() {
    let f = Fixture::new();
    let profile = f.profile("200", "300");
    // Two points author a straight member (P03), never a polybeam.
    assert_eq!(
        f.rejected(
            "create-polybeam",
            &["--points", "0,0,0;6000,0,0", "--profile-digest", &profile,]
        ),
        "INVALID_ARGUMENT"
    );
    // A zero-length chord is the kernel's refusal.
    assert_eq!(
        f.rejected(
            "create-polybeam",
            &[
                "--points",
                "0,0,0;3000,0,0;3000,0,0",
                "--profile-digest",
                &profile,
            ]
        ),
        "INVALID_ARGUMENT"
    );
}
