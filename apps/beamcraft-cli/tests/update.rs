// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, MPL-2.0. Expected volumes and masses are closed forms
// (width * length * thickness, volume * density); layer text is read with plain string matching.

//! P04 end to end: stored quantity records with input signatures, typed `update-element`, and
//! freshness. A rename touches the authored layer alone, a material change rewrites the quantity
//! record beside the same mesh, a thickness change rebuilds the mesh, and a stored record that no
//! longer matches the inputs is reported `stale`, never `current`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_cli::plate::inspect;
use beamcraft_host::{
    ids::IdSource,
    revision::{ExpectedBase, LayerEntry, NoFault, RevisionDraft, RevisionStore, Uuid7},
    storage::BlobStore,
};
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
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("p04-tests");
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
    fn revisions(&self) -> RevisionStore {
        RevisionStore::new(BlobStore::new(self.root.join("store")).unwrap())
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
    fn material(&self, density: &str) -> String {
        text(
            self.ok("define-material", &["--density", density])
                .get("definition")
                .unwrap(),
            "digest",
        )
    }
    fn plate(&self, extra: &[&str]) -> Json {
        let mut args = vec!["--width", "1000", "--length", "500", "--thickness", "10"];
        args.extend_from_slice(extra);
        self.ok("create-plate", &args)
    }
    fn inspected(&self) -> Json {
        self.ok("inspect-elements", &[])
    }
    /// Commits a head-based revision with changed layer text and no content verification, the way
    /// a layer edited outside BeamCraft would arrive.
    fn tamper(&self, edit: impl Fn(&str, String) -> String) {
        let store = self.revisions();
        let head = store.open().unwrap();
        let content = head.manifest().content();
        let mut ids = IdSource::system();
        let mut entries = Vec::new();
        for entry in content.entries() {
            let original = String::from_utf8(head.layer_bytes(entry.name()).unwrap()).unwrap();
            let edited = edit(entry.name(), original);
            store.blobs().publish(edited.as_bytes()).unwrap();
            entries.push(
                LayerEntry::from_bytes(entry.name(), entry.role(), edited.as_bytes()).unwrap(),
            );
        }
        let draft = RevisionDraft {
            project: content.project().clone(),
            workspace: content.workspace().clone(),
            revision: Uuid7::parse(&ids.next_id().unwrap().to_string()).unwrap(),
            op: Uuid7::parse(&ids.next_id().unwrap().to_string()).unwrap(),
            workspace_generation: content.workspace_generation() + 1,
            schema: content.schema(),
            root_layer: content.root_layer().to_owned(),
            entries,
        };
        store
            .commit_with(
                &draft,
                &ExpectedBase::Revision(head.info().revision().clone()),
                None,
                &mut NoFault,
            )
            .unwrap();
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
fn layer_digest(revision: &Json, name: &str) -> String {
    at(revision, &["revision", "manifest", "layers"])
        .array()
        .unwrap()
        .iter()
        .find(|l| text(l, "name") == name)
        .map(|l| text(l, "digest"))
        .unwrap()
}
fn layers(json: &Json) -> Vec<String> {
    at(json, &["changed_layers"])
        .array()
        .unwrap()
        .iter()
        .map(|v| v.string().unwrap().to_owned())
        .collect()
}

#[test]
fn a_rename_touches_the_authored_layer_alone_and_keeps_body_and_quantities() {
    let f = Fixture::new();
    let steel = f.material("7850");
    let created = f.plate(&["--material-digest", &steel]);
    let id = text(at(&created, &["plate"]), "element_id");
    let before = f.inspected();
    let renamed = f.ok("update-element", &["--id", &id, "--name", "Renamed plate"]);
    assert_eq!(text(&renamed, "outcome"), "element_updated");
    assert_eq!(layers(&renamed), ["authored.usda"]);
    let after = f.inspected();
    // The derived layer is byte for byte the same: the body and its records were not rebuilt.
    assert_eq!(
        layer_digest(&before, "derived.usda"),
        layer_digest(&after, "derived.usda")
    );
    let plate = &at(&after, &["plates"]).array().unwrap()[0];
    assert_eq!(text(plate, "name"), "Renamed plate");
    assert_eq!(
        text(plate, "mesh_digest"),
        text(at(&created, &["plate"]), "mesh_digest")
    );
    assert_eq!(num(plate, &["quantities", "mass", "kg"]), 39.25);
    assert_eq!(
        text(at(plate, &["stored_quantities", "volume"]), "freshness"),
        "current"
    );
    assert_eq!(
        text(at(plate, &["stored_quantities", "mass"]), "freshness"),
        "current"
    );
    // The footprint of a rename is the label alone.
    let writes = at(&renamed, &["footprint", "writes"]).array().unwrap();
    assert!(writes.iter().all(|e| text(e, "group") == "label"));
}

#[test]
fn a_material_change_updates_the_mass_record_and_keeps_the_shape() {
    let f = Fixture::new();
    let steel = f.material("7850");
    let light = f.material("2700");
    let created = f.plate(&["--material-digest", &steel]);
    let id = text(at(&created, &["plate"]), "element_id");
    let before = f.inspected();
    let shape_signature = text(
        at(
            &at(&before, &["plates"]).array().unwrap()[0],
            &["stored_quantities"],
        ),
        "stored_shape_signature",
    );
    let mass_signature = text(
        at(
            &at(&before, &["plates"]).array().unwrap()[0],
            &["stored_quantities"],
        ),
        "stored_mass_signature",
    );
    let changed = f.ok(
        "update-element",
        &["--id", &id, "--material-digest", &light],
    );
    // 0.005 m3 * 2700 kg/m3 = 13.5 kg; the mesh bytes are the same.
    assert_eq!(
        num(&changed, &["element", "quantities", "mass", "kg"]),
        13.5
    );
    assert_eq!(
        text(at(&changed, &["element"]), "mesh_digest"),
        text(at(&created, &["plate"]), "mesh_digest")
    );
    assert_eq!(layers(&changed), ["authored.usda", "derived.usda"]);
    let after = f.inspected();
    let stored = at(
        &at(&after, &["plates"]).array().unwrap()[0],
        &["stored_quantities"],
    )
    .clone();
    assert_eq!(text(&stored, "stored_shape_signature"), shape_signature);
    assert_ne!(text(&stored, "stored_mass_signature"), mass_signature);
    assert_eq!(num(&stored, &["mass", "stored_value"]), 13.5);
    // The material link is read, the new definition is a read in the footprint.
    let reads = at(&changed, &["footprint", "reads"]).array().unwrap();
    assert!(reads.iter().any(|e| text(e, "target") == light));
    // Clearing the material removes the mass and keeps the volume: unknown, not zero.
    let cleared = f.ok(
        "update-element",
        &["--id", &id, "--material-digest", "none"],
    );
    let mass = at(&cleared, &["element", "quantities", "mass"]);
    assert_eq!(text(mass, "status"), "unknown");
    assert_eq!(*at(mass, &["kg"]), Json::Null);
    assert_eq!(
        num(&cleared, &["element", "quantities", "volume", "mm3"]),
        5_000_000.0
    );
    let last = f.inspected();
    let plate = &at(&last, &["plates"]).array().unwrap()[0];
    assert_eq!(
        text(at(plate, &["stored_quantities", "mass"]), "stored_status"),
        "unknown"
    );
    assert_eq!(
        *at(plate, &["stored_quantities", "mass", "stored_value"]),
        Json::Null
    );
    assert_eq!(*at(plate, &["material_digest"]), Json::Null);
}

#[test]
fn a_thickness_change_rebuilds_the_mesh_and_both_records() {
    let f = Fixture::new();
    let steel = f.material("7850");
    let created = f.plate(&[
        "--material-digest",
        &steel,
        "--hole-rect",
        "100,100,100,100",
    ]);
    let id = text(at(&created, &["plate"]), "element_id");
    let changed = f.ok("update-element", &["--id", &id, "--thickness", "20"]);
    // (1000 * 500 - 100 * 100) * 20 = 9,800,000 mm3; * 7850 kg/m3 = 76.93 kg.
    let volume = num(&changed, &["element", "quantities", "volume", "mm3"]);
    assert!((volume - 9_800_000.0).abs() < 1e-6, "{volume}");
    let mass = num(&changed, &["element", "quantities", "mass", "kg"]);
    assert!((mass - 76.93).abs() < 1e-9, "{mass}");
    assert_ne!(
        text(at(&changed, &["element"]), "mesh_digest"),
        text(at(&created, &["plate"]), "mesh_digest")
    );
    let inspected = f.inspected();
    let plate = &at(&inspected, &["plates"]).array().unwrap()[0];
    assert_eq!(
        num(plate, &["stored_quantities", "volume", "stored_value"]),
        volume
    );
    assert_eq!(
        text(at(plate, &["stored_quantities", "volume"]), "freshness"),
        "current"
    );
}

#[test]
fn a_member_can_be_renamed_and_given_a_material() {
    let f = Fixture::new();
    let steel = f.material("7850");
    let profile = text(
        f.ok("define-profile", &["--width", "200", "--height", "300"])
            .get("definition")
            .unwrap(),
        "digest",
    );
    let beam = f.ok(
        "create-member",
        &[
            "--start",
            "0,0,0",
            "--end",
            "6000,0,0",
            "--profile-digest",
            &profile,
        ],
    );
    let id = text(at(&beam, &["member"]), "element_id");
    assert_eq!(
        text(at(&beam, &["member", "quantities", "mass"]), "status"),
        "unknown"
    );
    let changed = f.ok(
        "update-element",
        &[
            "--id",
            &id,
            "--name",
            "Main beam",
            "--material-digest",
            &steel,
        ],
    );
    assert_eq!(
        num(&changed, &["element", "quantities", "mass", "kg"]),
        2826.0
    );
    assert_eq!(text(at(&changed, &["element"]), "name"), "Main beam");
    assert_eq!(
        text(at(&changed, &["element"]), "mesh_digest"),
        text(at(&beam, &["member"]), "mesh_digest")
    );
    // A beam has no thickness.
    assert_eq!(
        f.rejected("update-element", &["--id", &id, "--thickness", "5"]),
        "UNSUPPORTED_PROPERTY"
    );
    let inspected = f.inspected();
    let member = &at(&inspected, &["members"]).array().unwrap()[0];
    assert_eq!(
        text(at(member, &["stored_quantities", "mass"]), "freshness"),
        "current"
    );
}

#[test]
fn updates_have_f04_codes_replay_and_publish_nothing_on_rejection() {
    let f = Fixture::new();
    let steel = f.material("7850");
    let created = f.plate(&[]);
    let id = text(at(&created, &["plate"]), "element_id");
    let missing = "01a00000-0000-7000-8000-0000000000ff";
    assert_eq!(
        f.rejected("update-element", &["--id", missing, "--name", "x"]),
        "NOT_FOUND"
    );
    assert_eq!(
        f.rejected("update-element", &["--id", &id]),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.rejected("update-element", &["--id", &id, "--name", ""]),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.rejected("update-element", &["--id", &id, "--thickness", "NaN"]),
        "NON_FINITE_NUMBER"
    );
    assert_eq!(
        f.rejected("update-element", &["--id", &id, "--thickness", "2000000"]),
        "LIMIT_EXCEEDED"
    );
    assert_eq!(
        f.rejected("update-element", &["--id", &id, "--thickness", "0"]),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.rejected(
            "update-element",
            &["--id", &id, "--material-digest", &"0".repeat(64)]
        ),
        "NOT_FOUND"
    );
    // The patch sets what the plate already has: nothing to commit.
    assert_eq!(
        f.rejected("update-element", &["--id", &id, "--name", "Plate"]),
        "INVALID_ARGUMENT"
    );
    let ops = [
        "--op-id",
        "01a00000-0000-7000-8000-000000000031",
        "--id",
        id.as_str(),
    ];
    let update = |name: &str| {
        let mut args = vec!["--name", name];
        args.extend_from_slice(&ops);
        f.run("update-element", &args)
    };
    let (code, first) = update("Beta");
    assert_eq!(code, 0, "{first:?}");
    let (code, replay) = update("Beta");
    assert_eq!(code, 0);
    assert_eq!(text(&replay, "outcome"), "element_update_replayed");
    assert_eq!(text(at(&replay, &["element"]), "name"), "Beta");
    let (code, reuse) = update("Gamma");
    assert_eq!(code, 2);
    assert_eq!(text(&reuse, "code"), "OP_ID_REUSE");
    // Nothing was published by the rejections: definition, plate, one rename.
    let inspected = f.inspected();
    assert_eq!(text(at(&inspected, &["revision"]), "sequence"), "3");
    let _ = steel;
}

#[test]
fn a_stored_record_that_no_longer_matches_is_stale_never_current() {
    let f = Fixture::new();
    let steel = f.material("7850");
    let light = f.material("2700");
    f.plate(&["--material-digest", &steel]);
    let current = f.inspected();
    assert_eq!(
        text(
            at(
                &at(&current, &["plates"]).array().unwrap()[0],
                &["stored_quantities", "mass"]
            ),
            "freshness"
        ),
        "current"
    );
    // A stored mass edited outside BeamCraft: same signature, other value.
    f.tamper(|name, text| {
        if name == "derived.usda" {
            assert!(text.contains("bim:massKg = 39.25"), "{text}");
            text.replace("bim:massKg = 39.25", "bim:massKg = 99.0")
        } else {
            text
        }
    });
    let inspected = f.revisions();
    let report = inspect(&inspected).unwrap();
    let record = report.quantities.values().next().unwrap();
    assert_eq!(record.assessment.volume.as_str(), "current");
    assert_eq!(record.assessment.mass.as_str(), "stale");
    // The value the engine reports is its own, not the stored 99 kg.
    assert_eq!(report.plates[0].quantities.mass_kg, Some(39.25));
    assert_eq!(record.stored.as_ref().unwrap().mass_kg, Some(99.0));

    // The material reference edited outside BeamCraft: the stored mass signature is now stale.
    let steel_path = format!("MAT_{steel}");
    let light_path = format!("MAT_{light}");
    f.tamper(|name, text| {
        if name == "authored.usda" {
            let (before, after) = text.split_once("custom rel bim:material").unwrap();
            assert!(
                after.contains(&steel_path),
                "material target must be present before tampering"
            );
            format!(
                "{before}custom rel bim:material{}",
                after.replacen(&steel_path, &light_path, 1)
            )
        } else {
            text
        }
    });
    let report = inspect(&inspected).unwrap();
    let record = report.quantities.values().next().unwrap();
    assert_eq!(record.assessment.volume.as_str(), "current");
    assert_eq!(record.assessment.mass.as_str(), "stale");
    // 0.005 m3 * 2700 kg/m3: the engine's recomputed mass follows the authored material.
    assert_eq!(report.plates[0].quantities.mass_kg, Some(13.5));
}

#[test]
fn a_revision_with_stale_records_cannot_be_published_by_the_verifier() {
    // The strict publish-time verifier is exercised through an update on top of a tampered head:
    // the new revision would carry the stale record forward, so it is refused.
    let f = Fixture::new();
    let steel = f.material("7850");
    let created = f.plate(&["--material-digest", &steel]);
    let id = text(at(&created, &["plate"]), "element_id");
    f.tamper(|name, text| {
        if name == "derived.usda" {
            text.replace("bim:massKg = 39.25", "bim:massKg = 99.0")
        } else {
            text
        }
    });
    // A rename leaves the derived layer untouched, so the stale record would stay: refused.
    assert_eq!(
        f.rejected("update-element", &["--id", &id, "--name", "After tamper"]),
        "INTEGRITY_ERROR"
    );
}

#[test]
fn a_partial_quantity_record_is_damaged_not_missing() {
    // Dropping only the signature must not turn a tampered record into a "missing" one that the
    // verifier lets through.
    let f = Fixture::new();
    let steel = f.material("7850");
    let created = f.plate(&["--material-digest", &steel]);
    let id = text(at(&created, &["plate"]), "element_id");
    f.tamper(|name, text| {
        if name == "derived.usda" {
            let kept: Vec<&str> = text
                .lines()
                .filter(|l| !l.contains("bim:shapeSignature"))
                .collect();
            kept.join("\n")
                .replace("bim:massKg = 39.25", "bim:massKg = 99.0")
        } else {
            text
        }
    });
    assert_eq!(
        f.rejected("update-element", &["--id", &id, "--name", "After tamper"]),
        "INTEGRITY_ERROR"
    );
}
