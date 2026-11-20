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

use crate::support::{self, Scripted};

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
            crate::support::boot_nonce(),
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
    /// One `run-script` invocation over the fixture store: the same commands the suite would
    /// spawn one process each, amortized into a single process (#738 P7; each command's own
    /// work is unchanged). Replies come back in the lines' order.
    fn script(&self, lines: &[Scripted]) -> Vec<Json> {
        let store = self.store();
        support::script_lines(&self.root, &store, lines)
    }
    fn profile(&self, width: &str, height: &str) -> String {
        let reply = self.ok("define-profile", &["--width", width, "--height", height]);
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
    let store = f.store();
    // The definition digests come back in order from one process (#738 P7).
    let defined = f.script(&[
        Scripted::ok(&[
            "define-profile",
            "--store",
            &store,
            "--width",
            "20",
            "--height",
            "30",
        ]),
        Scripted::ok(&["define-material", "--store", &store, "--density", "7850"]),
    ]);
    let profile = text(at(&defined[0], &["definition"]), "digest");
    let material = text(at(&defined[1], &["definition"]), "digest");
    let id = "019a7bf0-0000-7000-8000-000000cd0101";
    let op = "019a7bf0-0000-7000-8000-000000cd0102";
    let mut args: Vec<&str> = vec![
        "--store",
        &store,
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
    let mut line = vec!["create-curved-beam"];
    line.extend_from_slice(&args);
    let created_round = f.script(&[
        Scripted::ok(&line),
        Scripted::ok(&["inspect-elements", "--store", &store]),
        Scripted::ok(&["copy-element", "--store", &store, "--id", id]),
        Scripted::ok(&["copy-instance", "--store", &store, "--id", id]),
        Scripted::ok(&[
            "update-element",
            "--store",
            &store,
            "--id",
            id,
            "--name",
            "later name",
        ]),
    ]);
    let created = created_round[0].clone();
    let m = at(&created, &["member"]);
    assert_eq!(text(m, "path_type"), "arc");
    let length = std::f64::consts::PI * 1000.;
    assert!((num(m, &["axis_length_mm"]) - length).abs() < 1e-8);
    assert!(num(m, &["arc_deviation_mm"]) <= 0.05);
    assert!((num(m, &["quantities", "volume", "mm3"]) - 600. * length).abs() < 200.);
    let authored = f.authored_text(&created);
    assert!(authored.contains("steel:member:pathType = \"arc\""));
    // The stored body the inspection reads back carries the created mesh digest, and the
    // copies stay arcs; the fresh-process checks live in the replay block below.
    let inspected = &created_round[1];
    assert_eq!(
        text(
            &at(inspected, &["members"]).array().unwrap()[0],
            "mesh_digest"
        ),
        text(m, "mesh_digest")
    );
    let copied = &created_round[2];
    assert_eq!(text(at(copied, &["element"]), "path_type"), "arc");
    let instance = &created_round[3];
    assert_eq!(text(at(instance, &["element"]), "path_type"), "arc");
    // The replay in a fresh process names the created revision; the deletes and the empty
    // head read ride one process.
    let replay = f.ok("create-curved-beam", &args[2..]);
    assert_eq!(at(&replay, &["revision"]), at(&created, &["revision"]));
    assert_eq!(at(&replay, &["member"]), m);
    // Delete independent and linked dependents before deleting the definition.
    let instance_id = text(at(instance, &["element"]), "element_id");
    let deleted = f.script(&[
        Scripted::ok(&["delete-element", "--store", &store, "--id", &instance_id]),
        Scripted::ok(&["delete-element", "--store", &store, "--id", id]),
        Scripted::ok(&["open-revision", "--store", &store]),
    ]);
    let head = deleted[2].clone();
    let replay = f.ok("create-curved-beam", &args[2..]);
    assert_eq!(at(&replay, &["member"]), m);
    assert_eq!(f.ok("open-revision", &[]), head);
    args[3] = "1000,0,0;0,-1000,0;-1000,0,0";
    assert_eq!(f.rejected("create-curved-beam", &args[2..]), "OP_ID_REUSE");
}
#[test]
fn radius_mode_and_invalid_arcs_are_atomic() {
    let f = Fixture::new();
    let store = f.store();
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
    // Each refusal and the head read that proves its atomicity ride one process (#738 P7);
    // the head replies must match byte for byte.
    let head = f.ok("open-revision", &[]);
    let refused_head = |f: &Fixture, command: &str, extra: &[&str]| {
        let mut line = vec![command, "--store"];
        line.push(store.as_str());
        line.extend_from_slice(extra);
        let reads = f.script(&[
            Scripted::refused(&line),
            Scripted::ok(&["open-revision", "--store", &store]),
        ]);
        assert_eq!(reads[1], head, "{command} {extra:?} wrote");
        text(&reads[0], "code")
    };
    assert_eq!(
        refused_head(&f, "preview-numbering", &[]),
        "UNSUPPORTED_GEOMETRY"
    );
    for points in [
        "0,0,0;1,0,0;2,0,0",
        "0,0,0;0,0,0;1,1,0",
        "0,0,0;1,1,0",
        "0,0,0;1,1,0;2,0,0;3,1,0",
        "NaN,0,0;1,1,0;2,0,0",
        "1000000001,0,0;1,1,0;2,0,0",
    ] {
        refused_head(
            &f,
            "create-curved-beam",
            &["--points", points, "--profile-digest", &p],
        );
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
        refused_head(&f, "create-curved-beam", &args);
    }
    // A curved path cannot silently become a polyline via the generic point edit.
    let id = text(at(&created, &["member"]), "element_id");
    refused_head(
        &f,
        "update-element",
        &["--id", &id, "--axis-point", "0,0,0;1000,0,0"],
    );
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
    // The undo/redo checks keep their fresh-process boundaries: each reopen inspects the
    // draft the way a separate session would.
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

#[test]
fn a_large_radius_arc_is_built_and_an_over_budget_arc_names_its_limit() {
    let f = Fixture::new();
    let store = f.store();
    // A rolled I section with root fillets: about forty outline points per ring.
    let i_section = text(
        at(
            &f.ok(
                "define-profile",
                &[
                    "--family", "i_shape", "--h", "300", "--b", "150", "--tw", "7", "--tf", "10",
                    "--r", "15",
                ],
            ),
            &["definition"],
        ),
        "digest",
    );
    // A 30 m radius needs more edges per full turn than the profile-curve cap at 0.05 mm; the
    // 5 m arc itself needs far fewer facets. It used to be refused as LIMIT_EXCEEDED.
    let created = f.ok(
        "create-curved-beam",
        &[
            "--start",
            "0,0,0",
            "--end",
            "5000,0,0",
            "--radius",
            "30000",
            "--normal",
            "0,0,1",
            "--profile-digest",
            &i_section,
        ],
    );
    let member = at(&created, &["member"]);
    let deviation = num(member, &["arc_deviation_mm"]);
    assert!(deviation > 0.0 && deviation <= 0.05, "{deviation}");
    // Arc length of the minor arc over a 5000 mm chord at R = 30000: 2 R asin(c / 2R).
    let length = 2.0 * 30000.0 * (5000.0_f64 / 60000.0).asin();
    assert!((num(member, &["axis_length_mm"]) - length).abs() < 1e-6);

    // A long arc of the same section needs more rings than one member's mesh budget: still
    // refused, now with the budget and the way out named. The head read brackets the refusal
    // in the same process (#738 P7).
    let head = f.ok("open-revision", &[]);
    let reads = f.script(&[
        Scripted::refused(&[
            "create-curved-beam",
            "--store",
            &store,
            "--start",
            "0,0,0",
            "--end",
            "10000,0,0",
            "--radius",
            "12000",
            "--normal",
            "0,0,1",
            "--profile-digest",
            &i_section,
        ]),
        Scripted::ok(&["open-revision", "--store", &store]),
    ]);
    let refused = &reads[0];
    assert_eq!(text(refused, "code"), "LIMIT_EXCEEDED");
    assert!(text(refused, "message").contains("4096-vertex budget"));
    assert_eq!(reads[1], head);

    // The way out the refusal names (P13): split the arc into members of the same circle.
    // The split point is the refused arc's own mid-arc point (x = 5000, the sagitta top of
    // the R 12 m circle over the 10 m chord), so both pieces are minor arcs of that circle.
    let theta_half = (10000.0_f64 / 24000.0).asin();
    let mid_y = 12000.0 * (1.0 - theta_half.cos());
    let mid = format!("5000,{mid_y},0");
    let split_created = f.script(&[
        Scripted::ok(&[
            "create-curved-beam",
            "--store",
            &store,
            "--start",
            "0,0,0",
            "--end",
            mid.as_str(),
            "--radius",
            "12000",
            "--normal",
            "0,0,1",
            "--profile-digest",
            &i_section,
        ]),
        Scripted::ok(&[
            "create-curved-beam",
            "--store",
            &store,
            "--start",
            mid.as_str(),
            "--end",
            "10000,0,0",
            "--radius",
            "12000",
            "--normal",
            "0,0,1",
            "--profile-digest",
            &i_section,
        ]),
    ]);
    let first = at(&split_created[0], &["member"]);
    let second = at(&split_created[1], &["member"]);
    for part in [first, second] {
        let deviation = num(part, &["arc_deviation_mm"]);
        assert!(deviation > 0.0 && deviation <= 0.05, "{deviation}");
    }
    // The piece lengths add up to the refused arc's true length (R * angle, both halves).
    let full = 2.0 * 12000.0 * theta_half;
    let split = num(first, &["axis_length_mm"]) + num(second, &["axis_length_mm"]);
    assert!((split - full).abs() < 1e-6, "{split} != {full}");
}
