// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic members, MPL-2.0. Expected volumes and masses are closed forms
// (section area * true arc length, volume * density); the saved layer text is read with
// plain string matching and no BeamCraft crate.

//! P13 circular members through the real CLI and fresh process reopen.

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
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("p13-tests");
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

const ARC: &str = "1000,0,0;0,1000,0;-1000,0,0";
#[test]
fn curved_member_reopens_copies_and_replays_after_update_and_delete() {
    let f = Fixture::new();
    let profile = f.profile("20", "30");
    let material = f.material("7850");
    let id = "019a7bf0-0000-7000-8000-000000cd0101";
    let op = "019a7bf0-0000-7000-8000-000000cd0102";
    let args = [
        "--points",
        ARC,
        "--profile-digest",
        &profile,
        "--material-digest",
        &material,
        "--new-id",
        id,
        "--op-id",
        op,
    ];
    let created = f.ok("create-curved-beam", &args);
    let m = at(&created, &["member"]);
    assert_eq!(text(m, "path_type"), "arc");
    let length = std::f64::consts::PI * 1000.;
    assert!((num(m, &["axis_length_mm"]) - length).abs() < 1e-8);
    assert!(num(m, &["arc_deviation_mm"]) <= 0.05);
    assert!((num(m, &["quantities", "volume", "mm3"]) - 600. * length).abs() < 200.);
    let authored = f.authored_text(&created);
    assert!(authored.contains("steel:member:pathType = \"arc\""));
    let inspected = f.ok("inspect-elements", &[]);
    assert_eq!(
        text(
            &at(&inspected, &["members"]).array().unwrap()[0],
            "mesh_digest"
        ),
        text(m, "mesh_digest")
    );
    let copied = f.ok("copy-element", &["--id", id]);
    assert_eq!(text(at(&copied, &["element"]), "path_type"), "arc");
    let instance = f.ok("copy-instance", &["--id", id]);
    assert_eq!(text(at(&instance, &["element"]), "path_type"), "arc");
    f.ok("update-element", &["--id", id, "--name", "later name"]);
    let replay = f.ok("create-curved-beam", &args);
    assert_eq!(at(&replay, &["revision"]), at(&created, &["revision"]));
    assert_eq!(at(&replay, &["member"]), m);
    // Delete independent and linked dependents before deleting the definition.
    let instance_id = text(at(&instance, &["element"]), "element_id");
    f.ok("delete-element", &["--id", &instance_id]);
    f.ok("delete-element", &["--id", id]);
    let head = f.ok("open-revision", &[]);
    let replay = f.ok("create-curved-beam", &args);
    assert_eq!(at(&replay, &["member"]), m);
    assert_eq!(f.ok("open-revision", &[]), head);
    let mut different = args.to_vec();
    different[1] = "1000,0,0;0,-1000,0;-1000,0,0";
    assert_eq!(f.rejected("create-curved-beam", &different), "OP_ID_REUSE");
}
#[test]
fn radius_mode_and_invalid_arcs_are_atomic() {
    let f = Fixture::new();
    let p = f.profile("20", "30");
    let created = f.ok(
        "create-curved-beam",
        &[
            "--start",
            "1000,0,0",
            "--end",
            "-1000,0,0",
            "--radius",
            "1000",
            "--normal",
            "0,0,-1",
            "--profile-digest",
            &p,
        ],
    );
    assert!(
        (num(at(&created, &["member"]), &["axis_length_mm"]) - 1000. * std::f64::consts::PI).abs()
            < 1e-8
    );
    let head = f.ok("open-revision", &[]);
    assert_eq!(f.rejected("preview-numbering", &[]), "UNSUPPORTED_GEOMETRY");
    for points in [
        "0,0,0;1,0,0;2,0,0",
        "0,0,0;0,0,0;1,1,0",
        "0,0,0;1,1,0",
        "0,0,0;1,1,0;2,0,0;3,1,0",
        "NaN,0,0;1,1,0;2,0,0",
        "1000000001,0,0;1,1,0;2,0,0",
    ] {
        let (code, _) = f.run(
            "create-curved-beam",
            &["--points", points, "--profile-digest", &p],
        );
        assert_eq!(code, 2);
        assert_eq!(f.ok("open-revision", &[]), head);
    }
    for extra in [
        vec!["--radius", "999", "--normal", "0,0,1"],
        vec!["--radius", "1000", "--normal", "1,0,0"],
        vec!["--radius", "1000"],
        vec!["--radius", "1000", "--normal", "0,0,1", "--points", ARC],
    ] {
        let mut args = vec![
            "--start",
            "1000,0,0",
            "--end",
            "-1000,0,0",
            "--profile-digest",
            &p,
        ];
        args.extend(extra);
        assert_eq!(f.run("create-curved-beam", &args).0, 2);
        assert_eq!(f.ok("open-revision", &[]), head);
    }
    // A curved path cannot silently become a polyline via the generic point edit.
    let id = text(at(&created, &["member"]), "element_id");
    assert_eq!(
        f.run(
            "update-element",
            &["--id", &id, "--axis-point", "0,0,0;1000,0,0"]
        )
        .0,
        2
    );
    assert_eq!(f.ok("open-revision", &[]), head);
}

#[test]
fn curved_creation_and_point_replacement_each_undo_as_one_command() {
    let f = Fixture::new();
    let profile = f.profile("20", "30");
    let workspace = f.root.join("draft");
    let dir = workspace.to_str().unwrap();
    f.ok("create-workspace", &["--workspace-dir", dir]);
    let run = |command: &str, args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .arg(command)
            .args(args)
            .output()
            .unwrap();
        let result = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
        assert!(output.status.success(), "{command}: {result:?}");
        result
    };
    let created = run(
        "create-curved-beam",
        &[
            "--store",
            dir,
            "--points",
            ARC,
            "--profile-digest",
            &profile,
        ],
    );
    let id = text(at(&created, &["member"]), "element_id");
    run("undo-workspace", &["--workspace-dir", dir]);
    assert_eq!(
        num(
            &run("inspect-elements", &["--store", dir]),
            &["member_count"]
        ),
        0.
    );
    run("redo-workspace", &["--workspace-dir", dir]);
    let updated = run(
        "update-element",
        &[
            "--store",
            dir,
            "--id",
            &id,
            "--axis-point",
            "2000,0,0;0,2000,0;-2000,0,0",
        ],
    );
    assert_eq!(text(at(&updated, &["element"]), "path_type"), "arc");
    run("undo-workspace", &["--workspace-dir", dir]);
    let restored = run("inspect-elements", &["--store", dir]);
    let member = &at(&restored, &["members"]).array().unwrap()[0];
    assert_eq!(
        text(member, "mesh_digest"),
        text(at(&created, &["member"]), "mesh_digest")
    );
    run("redo-workspace", &["--workspace-dir", dir]);
    let restored = run("inspect-elements", &["--store", dir]);
    assert_eq!(
        text(
            &at(&restored, &["members"]).array().unwrap()[0],
            "mesh_digest"
        ),
        text(at(&updated, &["element"]), "mesh_digest")
    );
}
