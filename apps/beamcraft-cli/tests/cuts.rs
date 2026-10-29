// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic fixtures, MPL-2.0. Expected volumes are closed forms. The saved
// layer text is read with plain string matching and no BeamCraft crate.

//! A03 plane cuts as owned operation records (amendment A-2) end to end through the real
//! `beamcraft` binary: `cut-by-plane` appends one record to a stored plate or beam, the replay
//! rebuilds the mesh and the quantities in the same transaction, fresh processes reopen and
//! re-verify, the record survives a copy with fresh identities, and the workspace undo takes the
//! cut back.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use crate::support;

use support::{at, text};

use beamcraft_json::{Json, Limits};
use std::fs;
use std::process::Command;

struct Fixture {
    shared: support::StoreFixture,
}
impl Fixture {
    fn new() -> Self {
        Self {
            shared: support::StoreFixture::new("a03-tests"),
        }
    }
    fn store(&self) -> String {
        self.shared.root.join("store").to_str().unwrap().to_owned()
    }
    fn run(&self, command: &str, extra: &[&str]) -> (i32, Json) {
        self.shared.run(command, extra)
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
    fn cut(&self, target: &str, operation: &str, extra: &[&str]) -> (i32, Json) {
        let mut args = vec!["--id", target, "--operation-id", operation];
        args.extend_from_slice(extra);
        // Defaults only for the options the caller did not spell out, so an override is never a
        // repeated option.
        for (option, default) in [
            ("--point", "250,0,0"),
            ("--normal", "1,0,0"),
            ("--keep", "negative"),
        ] {
            if !extra.contains(&option) {
                args.push(option);
                args.push(default);
            }
        }
        self.run("cut-by-plane", &args)
    }
    fn ok_cut(&self, target: &str, operation: &str, extra: &[&str]) -> Json {
        let (code, body) = self.cut(target, operation, extra);
        assert_eq!(code, 0, "{extra:?}: {body:?}");
        body
    }
    fn rejected_cut(&self, target: &str, operation: &str, extra: &[&str]) -> (i32, String, Json) {
        let (code, body) = self.cut(target, operation, extra);
        if code != 2 {
            panic!("expected a rejection for {extra:?}, got {body:?}");
        }
        let kind = text(&body, "code");
        (code, kind, body)
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
            .shared
            .root
            .join("store")
            .join("blobs")
            .join(&digest[..2])
            .join(&digest);
        String::from_utf8(fs::read(path).unwrap()).unwrap()
    }
}

fn num(json: &Json, path: &[&str]) -> f64 {
    at(json, path).float().unwrap()
}
/// The single cut record of an element, as `inspect-elements` reports it.
fn first_cut(plate: &Json) -> &Json {
    at(plate, &["cuts"]).array().unwrap().first().unwrap()
}

const CUT_RECORD: &str = "01a00000-0000-7000-8000-0000000000c2";

#[test]
fn a_cut_plate_has_the_exact_volume_and_mass_and_reopens_verified() {
    let f = Fixture::new();
    let steel = {
        let (_, reply) = f.run("define-material", &["--density", "7850"]);
        text(reply.get("definition").unwrap(), "digest")
    };
    let created = f.ok_plate(&["--material-digest", &steel]);
    let element = text(at(&created, &["plate"]), "element_id");
    // The uncut plate first, then the owned record that trims it.
    assert_eq!(
        num(&created, &["plate", "quantities", "volume", "mm3"]),
        5_000_000.0
    );
    let cut = f.ok_cut(&element, CUT_RECORD, &[]);
    assert_eq!(text(&cut, "outcome"), "cut_applied");
    assert_eq!(num(&cut, &["record", "ordinal"]), 0.0, "{cut:?}");
    let plate = at(&cut, &["element"]);
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
    let record = first_cut(plate);
    assert_eq!(text(record, "operation_id"), CUT_RECORD);
    assert_eq!(text(record, "keep"), "negative");
    let point: Vec<f64> = at(record, &["point_mm"])
        .array()
        .unwrap()
        .iter()
        .map(|v| v.float().unwrap())
        .collect();
    assert_eq!(point, vec![250.0, 0.0, 0.0]);

    // A fresh process re-evaluates the authored record and agrees.
    let inspected = f.run("inspect-elements", &[]).1;
    let reopened = &at(&inspected, &["plates"]).array().unwrap()[0];
    assert_eq!(text(reopened, "mesh_digest"), text(plate, "mesh_digest"));
    assert_eq!(num(reopened, &["quantities", "volume", "mm3"]), volume);
    assert_eq!(text(first_cut(reopened), "operation_id"), CUT_RECORD);

    // An ordinary reader of the saved text sees the record as plain values.
    let authored = f.authored_text(&cut);
    assert!(
        authored.contains(&format!(
            "custom string[] steel:cut:operationId = [\"{CUT_RECORD}\"]"
        )),
        "{authored}"
    );
    assert!(
        authored.contains("custom int[] steel:cut:ordinal = [0]"),
        "{authored}"
    );
    assert!(
        authored.contains("custom double3[] steel:cut:pointMm = [(250.0, 0.0, 0.0)]"),
        "{authored}"
    );
    assert!(
        authored.contains("custom token[] steel:cut:keep = [\"negative\"]"),
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
    let (code, beam) = f.run(
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
    assert_eq!(code, 0, "{beam:?}");
    let element = text(at(&beam, &["member"]), "element_id");
    let cut = f.ok_cut(
        &element,
        "01a00000-0000-7000-8000-0000000000b1",
        &["--point", "3000,0,0"],
    );
    let member = at(&cut, &["element"]);
    // The kept half is 3000 mm long: 3000 * 200 * 300 = 180,000,000 mm3 = 0.18 m3.
    let volume = num(member, &["quantities", "volume", "mm3"]);
    assert!((volume - 180_000_000.0).abs() <= 1e-6, "{volume}");
    assert!(num(member, &["triangles"]) > 12.0);

    // A plate cut after a hole keeps both effects; the hole lies inside the kept part.
    let created = f.ok_plate(&["--hole-rect", "100,100,100,100"]);
    let plate_element = text(at(&created, &["plate"]), "element_id");
    let cut = f.ok_cut(&plate_element, CUT_RECORD, &[]);
    let plate = at(&cut, &["element"]);
    // (250 * 500 - 100 * 100) * 10 = 1,150,000 mm3.
    let volume = num(plate, &["quantities", "volume", "mm3"]);
    assert!((volume - 1_150_000.0).abs() <= 1e-6, "{volume}");

    // A second record takes the next ordinal and replays after the first: keeping the far side
    // of x = 50 trims the front strip (the plane must not cross the hole, which would leave two
    // section loops). Kept: (200 * 500 - 100 * 100) * 10 = 900,000 mm3.
    let second = f.ok_cut(
        &plate_element,
        "01a00000-0000-7000-8000-0000000000c3",
        &["--point", "50,0,0", "--keep", "positive"],
    );
    assert_eq!(num(&second, &["record", "ordinal"]), 1.0, "{second:?}");
    let volume = num(at(&second, &["element"]), &["quantities", "volume", "mm3"]);
    assert!((volume - 900_000.0).abs() <= 1e-6, "{volume}");
}

#[test]
fn a_solid_element_authors_no_cut_attributes() {
    let f = Fixture::new();
    let solid = f.ok_plate(&[]);
    assert!(!f.authored_text(&solid).contains("steel:cut"));
    assert_eq!(
        at(&solid, &["plate", "cuts"]).array().unwrap().len(),
        0,
        "{solid:?}"
    );
}

#[test]
fn cut_rejections_use_the_f04_codes_and_publish_nothing() {
    let f = Fixture::new();
    let created = f.ok_plate(&[]);
    let element = text(at(&created, &["plate"]), "element_id");
    // The plane misses the plate entirely.
    let (code, kind, _) = f.rejected_cut(&element, CUT_RECORD, &["--point", "2000,0,0"]);
    assert_eq!((code, kind.as_str()), (2, "INVALID_ARGUMENT"));
    // The plane only touches the bottom face: a tangent cut, not a trim.
    let (code, kind, _) = f.rejected_cut(
        &element,
        CUT_RECORD,
        &["--point", "0,0,0", "--normal", "0,0,1"],
    );
    assert_eq!((code, kind.as_str()), (2, "UNSUPPORTED_GEOMETRY"));
    // A zero normal never reaches the kernel.
    let (code, kind, _) = f.rejected_cut(
        &element,
        CUT_RECORD,
        &["--point", "0,0,0", "--normal", "0,0,0"],
    );
    assert_eq!((code, kind.as_str()), (2, "INVALID_ARGUMENT"));
    // Malformed, non-finite and out-of-cap values.
    let (code, kind, _) = f.rejected_cut(
        &element,
        CUT_RECORD,
        &["--point", "250,0", "--normal", "1,0,0"],
    );
    assert_eq!((code, kind.as_str()), (2, "INVALID_ARGUMENT"));
    let (code, kind, _) = f.rejected_cut(
        &element,
        CUT_RECORD,
        &["--point", "NaN,0,0", "--normal", "1,0,0"],
    );
    assert_eq!((code, kind.as_str()), (2, "NON_FINITE_NUMBER"));
    let (code, kind, _) = f.rejected_cut(
        &element,
        CUT_RECORD,
        &["--point", "2e9,0,0", "--normal", "1,0,0"],
    );
    assert_eq!((code, kind.as_str()), (2, "LIMIT_EXCEEDED"));
    // An unknown target and an operation id already owned by a record.
    let (code, kind, _) = f.rejected_cut("01a00000-0000-7000-8000-000000000099", CUT_RECORD, &[]);
    assert_eq!((code, kind.as_str()), (2, "NOT_FOUND"));
    f.ok_cut(&element, CUT_RECORD, &[]);
    let (code, kind, _) = f.rejected_cut(&element, CUT_RECORD, &["--point", "300,0,0"]);
    assert_eq!((code, kind.as_str()), (2, "DUPLICATE_ID"));
    // --keep only takes the two contract tokens.
    let (code, body) = f.run(
        "cut-by-plane",
        &[
            "--store",
            &f.store(),
            "--id",
            &element,
            "--operation-id",
            "01a00000-0000-7000-8000-0000000000c4",
            "--point",
            "250,0,0",
            "--normal",
            "1,0,0",
            "--keep",
            "sideways",
        ],
    );
    assert_eq!(code, 2, "{body:?}");
    // Nothing of the rejected work was published: the head still holds exactly one record.
    let inspected = f.run("inspect-elements", &[]).1;
    let reopened = &at(&inspected, &["plates"]).array().unwrap()[0];
    assert_eq!(at(reopened, &["cuts"]).array().unwrap().len(), 1);
}

#[test]
fn a_retry_with_the_same_cut_replays_and_another_plane_is_reuse() {
    let f = Fixture::new();
    let created = f.ok_plate(&[
        "--new-id",
        "01a00000-0000-7000-8000-000000000031",
        "--op-id",
        "01a00000-0000-7000-8000-000000000032",
    ]);
    let element = text(at(&created, &["plate"]), "element_id");
    let (code, first) = f.cut(&element, CUT_RECORD, &["--op-id", CUT_RECORD]);
    assert_eq!(code, 0, "{first:?}");
    assert_eq!(text(&first, "outcome"), "cut_applied");
    let (code, replay) = f.cut(&element, CUT_RECORD, &["--op-id", CUT_RECORD]);
    assert_eq!(code, 0);
    assert_eq!(text(&replay, "outcome"), "cut_replayed");
    assert_eq!(
        text(at(&replay, &["element"]), "mesh_digest"),
        text(at(&first, &["element"]), "mesh_digest")
    );
    // A retried command with different content under the accepted operation id is a reuse
    // refusal (the op-id decision runs before the record admission, F04 section 8).
    let (code, reuse) = f.cut(
        &element,
        CUT_RECORD,
        &["--op-id", CUT_RECORD, "--point", "300,0,0"],
    );
    assert_eq!(code, 2);
    assert_eq!(text(&reuse, "code"), "OP_ID_REUSE");
    // A new command reusing a record identity another accepted command owns is DUPLICATE_ID.
    let (code, duplicate) = f.cut(&element, CUT_RECORD, &["--point", "300,0,0"]);
    assert_eq!(code, 2);
    assert_eq!(text(&duplicate, "code"), "DUPLICATE_ID");
}

#[test]
fn a_copy_carries_the_records_with_fresh_ids_and_the_copy_is_cut_too() {
    let f = Fixture::new();
    let created = f.ok_plate(&[]);
    let source_id = text(at(&created, &["plate"]), "element_id");
    let cut = f.ok_cut(&source_id, CUT_RECORD, &[]);
    let (code, copy) = f.run(
        "copy-element",
        &["--id", &source_id, "--offset", "0,1000,0"],
    );
    assert_eq!(code, 0, "{copy:?}");
    let copy_id = text(at(&copy, &["element"]), "element_id");
    assert_ne!(copy_id, source_id);
    // The copy has the same body (the same mesh digest) and carries the record.
    assert_eq!(
        text(at(&copy, &["element"]), "mesh_digest"),
        text(at(&cut, &["element"]), "mesh_digest")
    );
    let copied = first_cut(at(&copy, &["element"]));
    assert_ne!(text(copied, "operation_id"), CUT_RECORD);
    assert_eq!(
        at(copied, &["point_mm"]).array().unwrap()[0]
            .float()
            .unwrap(),
        250.0
    );
    // A fresh process re-verifies both elements against their authored records.
    let inspected = f.run("inspect-elements", &[]).1;
    assert_eq!(num(&inspected, &["plate_count"]), 2.0);
    let plates = at(&inspected, &["plates"]).array().unwrap();
    for plate in plates {
        let volume = num(plate, &["quantities", "volume", "mm3"]);
        assert!((volume - 1_250_000.0).abs() <= 1e-6, "{volume}");
        assert_eq!(
            at(first_cut(plate), &["point_mm"]).array().unwrap()[0]
                .float()
                .unwrap(),
            250.0
        );
    }
}

#[test]
fn undo_takes_the_cut_back_and_features_follow_the_plane() {
    let f = Fixture::new();
    let created = f.ok_plate(&[]);
    let element = text(at(&created, &["plate"]), "element_id");
    // A personal workspace holds the draft the undo rolls back.
    let draft = f.shared.root.join("draft");
    fs::create_dir(&draft).unwrap();
    let draft = draft.to_str().unwrap().to_owned();
    let (code, _) = f.run("create-workspace", &["--workspace-dir", &draft]);
    assert_eq!(code, 0);
    let run_in = |command: &str, args: &[&str]| {
        let mut all = vec![command, "--store", &draft];
        all.extend_from_slice(args);
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(&all)
            .output()
            .unwrap();
        (output.status.code().unwrap(), output.stdout.clone())
    };
    let (code, _) = run_in(
        "cut-by-plane",
        &[
            "--id",
            &element,
            "--operation-id",
            CUT_RECORD,
            "--point",
            "250,0,0",
            "--normal",
            "1,0,0",
            "--keep",
            "negative",
        ],
    );
    assert_eq!(code, 0);
    // The cap face of the record is a stable address; a clipped corner is an explicit loss.
    let cap = format!("fid1:{element}:cut:f.p0");
    let (code, _) = run_in("lookup-feature", &["--feature-id", &cap]);
    assert_eq!(code, 0, "the cap face resolves");
    let corner = format!("fid1:{element}:plate:v.x1.y0.z1");
    let (code, out) = run_in("lookup-feature", &["--feature-id", &corner]);
    assert_eq!(code, 2, "a clipped corner is an explicit loss");
    assert!(String::from_utf8_lossy(&out).contains("GEO_FEATURE_NOT_FOUND"),);
    // One command is one undo unit: the draft goes back to the uncut volume.
    let history = |command: &str| {
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args([command, "--workspace-dir", &draft])
            .output()
            .unwrap();
        assert!(output.status.success());
    };
    let volume_of = |json: &[u8]| -> f64 {
        let body = beamcraft_json::decode(json, &Limits::DEFAULT).unwrap();
        at(&body, &["plates"]).array().unwrap()[0]
            .get("quantities")
            .unwrap()
            .get("volume")
            .unwrap()
            .get("mm3")
            .unwrap()
            .float()
            .unwrap()
    };
    let inspected = run_in("inspect-elements", &[]).1;
    assert!((volume_of(&inspected) - 1_250_000.0).abs() <= 1e-6);
    history("undo-workspace");
    let inspected = run_in("inspect-elements", &[]).1;
    assert!((volume_of(&inspected) - 5_000_000.0).abs() <= 1e-6);
    assert!(
        at(
            &beamcraft_json::decode(&inspected, &Limits::DEFAULT).unwrap(),
            &["plates"]
        )
        .array()
        .unwrap()[0]
            .get("cuts")
            .unwrap()
            .array()
            .unwrap()
            .is_empty()
    );
    history("redo-workspace");
    let inspected = run_in("inspect-elements", &[]).1;
    assert!((volume_of(&inspected) - 1_250_000.0).abs() <= 1e-6);
}
