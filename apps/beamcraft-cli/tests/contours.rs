// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, MPL-2.0. Expected volumes are closed forms: the polygon
// area times the thickness. The saved layer text is read with plain string matching and no
// BeamCraft crate.

//! A02 (experimental first slice) contour plates end to end through the real `beamcraft`
//! binary: `create-plate --outer` defines a plate by an arbitrary closed counter-clockwise
//! outline, the kernel extrudes and certifies it, and fresh processes reopen and re-verify the
//! result.

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
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("a02-tests");
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
    fn plate(&self, extra: &[&str]) -> (i32, Json) {
        let mut args = vec!["--thickness", "10"];
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

/// The counter-clockwise right triangle (0,0) (300,0) (0,400), area 60,000 mm2.
const TRIANGLE: &str = "0,0;300,0;0,400";

#[test]
fn a_contour_plate_has_the_exact_volume_and_mass_and_reopens_verified() {
    let f = Fixture::new();
    let steel = {
        let (_, reply) = f.run("define-material", &["--density", "7850"]);
        text(reply.get("definition").unwrap(), "digest")
    };
    let created = f.ok_plate(&[
        "--outer",
        TRIANGLE,
        "--material-digest",
        &steel,
        "--translation",
        "1000,0,0",
    ]);
    let plate = at(&created, &["plate"]);
    // 60,000 mm2 * 10 mm = 600,000 mm3 = 0.0006 m3; * 7850 kg/m3 = 4.71 kg.
    let volume = num(plate, &["quantities", "volume", "mm3"]);
    assert!((volume - 600_000.0).abs() <= 1e-9, "{volume}");
    let mass = num(plate, &["quantities", "mass", "kg"]);
    assert!((mass - 4.71).abs() <= 1e-9, "{mass}");
    // A contour plate reports a bounding box under its own name, never dimensions that a
    // consumer could multiply into a size, and names its shape.
    assert_eq!(text(plate, "shape"), "contour");
    assert!(plate.get("dimensions_mm").is_err());
    assert_eq!(num(plate, &["bounding_box_mm", "width"]), 300.0);
    assert_eq!(num(plate, &["bounding_box_mm", "length"]), 400.0);
    assert_eq!(num(plate, &["bounding_box_mm", "thickness"]), 10.0);

    let inspected = f.run("plate-inspect", &[]).1;
    let reopened = &at(&inspected, &["plates"]).array().unwrap()[0];
    assert_eq!(text(reopened, "mesh_digest"), text(plate, "mesh_digest"));
    assert_eq!(
        num(reopened, &["quantities", "volume", "mm3"]),
        num(plate, &["quantities", "volume", "mm3"])
    );
    assert_eq!(num(reopened, &["bounding_box_mm", "width"]), 300.0);

    // An ordinary reader of the saved text sees the authored contour as a plain array.
    let authored = f.authored_text(&created);
    assert!(
        authored.contains(
            "custom double3[] steel:plate:contourPointsMm = [(0.0, 0.0, 0.0), (300.0, 0.0, 0.0), (0.0, 400.0, 0.0)]"
        ),
        "{authored}"
    );

    // The view snapshot bounds are those of the outline, moved by the placement.
    let entry = &at(&inspected, &["snapshot", "elements"]).array().unwrap()[0];
    let corner = |key: &str| -> Vec<f64> {
        at(entry, &[key])
            .array()
            .unwrap()
            .iter()
            .map(|v| v.float().unwrap())
            .collect()
    };
    assert_eq!(corner("world_min_mm"), vec![1000.0, 0.0, 0.0]);
    assert_eq!(corner("world_max_mm"), vec![1300.0, 400.0, 10.0]);

    // One trailing separator is ignored: the closure of a contour is implicit.
    let (code, second) = f.plate(&["--outer", "0,0;300,0;0,400;"]);
    assert_eq!(code, 0, "{second:?}");
    let inspected = f.run("plate-inspect", &[]).1;
    assert_eq!(num(&inspected, &["plate_count"]), 2.0);
}

#[test]
fn a_contour_plate_takes_holes_and_survives_update_and_copy() {
    let f = Fixture::new();
    // The counter-clockwise L (0,0) (600,0) (600,200) (200,200) (200,600) (0,600):
    // 600 * 600 - 400 * 400 = 200,000 mm2.
    let l_shape = "0,0;600,0;600,200;200,200;200,600;0,600";
    // The hole sits strictly inside the material, clear of the re-entrant corner at (200,200).
    let created = f.ok_plate(&["--outer", l_shape, "--hole-rect", "50,50,100,100"]);
    let plate = at(&created, &["plate"]);
    // (200,000 - 100 * 100) mm2 * 10 mm = 1,900,000 mm3.
    let volume = num(plate, &["quantities", "volume", "mm3"]);
    assert!((volume - 1_900_000.0).abs() <= 1e-6, "{volume}");
    assert_eq!(num(plate, &["hole_count"]), 1.0);
    let id = text(plate, "element_id");

    // A thickness edit re-evaluates the contour plate like a rectangle.
    let (code, _) = f.run(
        "update-element",
        &["--id", id.as_str(), "--thickness", "20"],
    );
    assert_eq!(code, 0);
    let inspected = f.run("plate-inspect", &[]).1;
    let updated = &at(&inspected, &["plates"]).array().unwrap()[0];
    let doubled = num(updated, &["quantities", "volume", "mm3"]);
    assert!((doubled - 3_800_000.0).abs() <= 1e-6, "{doubled}");
    let digest = text(updated, "mesh_digest");

    // A copy of a contour plate is a new identity with the same body.
    let (code, copied) = f.run(
        "copy-element",
        &["--id", id.as_str(), "--offset", "0,2000,0"],
    );
    assert_eq!(code, 0, "{copied:?}");
    let inspected = f.run("plate-inspect", &[]).1;
    assert_eq!(num(&inspected, &["plate_count"]), 2.0);
    let plates = at(&inspected, &["plates"]).array().unwrap();
    let copy_entry = plates.iter().find(|p| text(p, "element_id") != id).unwrap();
    assert_eq!(text(copy_entry, "mesh_digest"), digest);
}

#[test]
fn contour_rejections_use_the_f04_codes_and_publish_nothing() {
    let f = Fixture::new();
    // A clockwise outline is refused by the kernel's winding rule.
    assert_eq!(
        f.rejected(&["--outer", "0,0;0,400;300,0"]),
        "INVALID_ARGUMENT"
    );
    // A self-intersecting bowtie.
    assert_eq!(
        f.rejected(&["--outer", "0,0;1000,500;1000,0;0,500"]),
        "INVALID_ARGUMENT"
    );
    // Fewer than three points.
    assert_eq!(f.rejected(&["--outer", "0,0;300,0"]), "INVALID_ARGUMENT");
    // Malformed points.
    assert_eq!(f.rejected(&["--outer", "0,0;300"]), "INVALID_ARGUMENT");
    assert_eq!(f.rejected(&["--outer", "0,0;300,0,x"]), "INVALID_ARGUMENT");
    // Out of the coordinate cap.
    assert_eq!(f.rejected(&["--outer", "2e9,0;1,0;0,1"]), "LIMIT_EXCEEDED");
    // --outer and --width/--length are two ways of one outline, never both.
    assert_eq!(
        f.rejected(&["--outer", TRIANGLE, "--width", "1000", "--length", "500"]),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.rejected(&["--outer", TRIANGLE, "--length", "500"]),
        "INVALID_ARGUMENT"
    );
    // Missing outline entirely.
    assert_eq!(f.rejected(&[]), "INVALID_ARGUMENT");
    // A hole outside the triangle is refused by the kernel, nothing is published.
    assert_eq!(
        f.rejected(&["--outer", TRIANGLE, "--hole-rect", "200,200,100,100"]),
        "INVALID_ARGUMENT"
    );
    // Nothing was created: there is no revision at all.
    let (code, body) = f.run("plate-inspect", &[]);
    assert_eq!(code, 2, "{body:?}");
}

#[test]
fn a_retry_with_the_same_contour_replays_and_another_contour_is_reuse() {
    let f = Fixture::new();
    let ids = [
        "--new-id",
        "01a00000-0000-7000-8000-000000000031",
        "--op-id",
        "01a00000-0000-7000-8000-000000000032",
    ];
    let create = |outer: &str| {
        let mut args = vec!["--outer", outer];
        args.extend_from_slice(&ids);
        f.plate(&args)
    };
    let (code, first) = create(TRIANGLE);
    assert_eq!(code, 0);
    assert_eq!(text(&first, "outcome"), "plate_created");
    let (code, replay) = create(TRIANGLE);
    assert_eq!(code, 0);
    assert_eq!(text(&replay, "outcome"), "plate_replayed");
    assert_eq!(
        text(at(&replay, &["plate"]), "mesh_digest"),
        text(at(&first, &["plate"]), "mesh_digest")
    );
    let (code, reuse) = create("0,0;600,0;600,200;200,200;200,600;0,600");
    assert_eq!(code, 2);
    assert_eq!(text(&reuse, "code"), "OP_ID_REUSE");
}
