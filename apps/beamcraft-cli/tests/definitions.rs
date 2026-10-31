// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic definitions and plates, MPL-2.0. Expected masses are the closed
// form volume * density with volume = width * length * thickness (F04 section 5), never
// production math. The USDA checks below read the saved layer text with plain string matching
// and use no BeamCraft crate to interpret it.

//! P02 end to end through the real `beamcraft` binary: `define-material` and `define-profile`
//! publish immutable definitions as USD assets of a committed revision, `create-plate
//! --material-digest` pins one, fresh processes reopen the result (`inspect-elements`,
//! `definitions`), and the saved layer text is checked without BeamCraft.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support;

use support::{at, text};

use beamcraft_json::{Json, Limits};
use beamcraft_layer_engine::authored::{Definition, Resource, SectionProperties};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("p02-tests");
        fs::create_dir_all(&base).unwrap();
        let root = base.join(format!(
            "e2e-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("store")).unwrap();
        Self(root)
    }
    fn store(&self) -> String {
        self.0.join("store").to_str().unwrap().to_owned()
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
        assert_eq!(code, 0, "{command} {extra:?}");
        body
    }
    fn rejected(&self, command: &str, extra: &[&str]) -> String {
        let (code, body) = self.run(command, extra);
        assert_eq!(code, 2, "{command} {extra:?}");
        assert_eq!(text(&body, "outcome"), "rejected");
        text(&body, "code")
    }
    fn plate(&self, extra: &[&str]) -> Json {
        let mut args = vec!["--width", "1000", "--length", "500", "--thickness", "10"];
        args.extend_from_slice(extra);
        self.ok("create-plate", &args)
    }
    fn layer_text(&self, name: &str, revision: &Json) -> String {
        let layers = at(revision, &["revision", "manifest", "layers"])
            .array()
            .unwrap();
        let layer = layers
            .iter()
            .find(|l| text(l, "name") == name)
            .expect("layer in manifest");
        let digest = text(layer, "digest");
        let path = self
            .0
            .join("store")
            .join("blobs")
            .join(&digest[..2])
            .join(&digest);
        String::from_utf8(fs::read(path).unwrap()).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(self.0.is_absolute() && self.0.file_name().is_some());
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn num(json: &Json, path: &[&str]) -> f64 {
    at(json, path).float().unwrap()
}
fn digest_of(created: &Json) -> String {
    text(at(created, &["definition"]), "digest")
}
fn sequence(json: &Json) -> u64 {
    text(at(json, &["revision"]), "sequence").parse().unwrap()
}

const STEEL: &str = "7850";

#[test]
fn a_material_with_density_gives_the_plate_a_current_mass_across_processes() {
    let f = Fixture::new();
    let steel = f.ok("define-material", &["--density", STEEL]);
    assert_eq!(text(&steel, "outcome"), "definition_created");
    let digest = digest_of(&steel);
    assert_eq!(digest.len(), 64);
    // The digest is the identity of the canonical isolated definition layer of the layer engine.
    // A definition published without --name carries its default display name in that layer, so
    // the name is part of the digest (A07.2).
    let named = |display_name: Option<&str>| {
        Resource::new(Definition::Material {
            density_kg_m3: Some(7850.0),
            grade: None,
            standard: None,
            display_name: display_name.map(str::to_owned),
        })
        .unwrap()
        .digest
        .to_string()
    };
    assert_eq!(digest, named(Some("7850 kg/m3")));
    assert_ne!(digest, named(None));
    assert_eq!(
        text(at(&steel, &["definition"]), "display_name"),
        "7850 kg/m3"
    );
    assert_eq!(text(at(&steel, &["definition"]), "units"), "mm");
    assert_eq!(num(&steel, &["definition", "density_kg_m3"]), 7850.0);
    assert_eq!(sequence(&steel), 1);

    let created = f.plate(&["--material-digest", &digest]);
    assert_eq!(text(&created, "outcome"), "plate_created");
    assert_eq!(sequence(&created), 2);
    let mass = at(&created, &["plate", "quantities", "mass"]);
    // 1000 x 500 x 10 mm = 0.005 m3; 0.005 m3 * 7850 kg/m3 = 39.25 kg.
    assert_eq!(text(mass, "status"), "current");
    assert_eq!(num(mass, &["kg"]), 39.25);
    assert_eq!(num(mass, &["density_kg_m3"]), 7850.0);
    assert_eq!(
        text(at(&created, &["plate"]), "material_digest"),
        digest.clone()
    );
    // The command declared the read of the pinned definition.
    let reads = at(&created, &["footprint", "reads"]).array().unwrap();
    assert!(
        reads
            .iter()
            .any(|e| text(e, "target") == digest && text(e, "group") == "relationship")
    );

    // A fresh process reads the same material, mass and shape back from the saved bytes.
    let inspected = f.ok("inspect-elements", &[]);
    let plate = &at(&inspected, &["plates"]).array().unwrap()[0];
    assert_eq!(text(plate, "material_digest"), digest);
    assert_eq!(num(plate, &["quantities", "mass", "kg"]), 39.25);
    assert_eq!(num(plate, &["quantities", "volume", "m3"]), 0.005);
    let listed = f.ok("definitions", &[]);
    assert_eq!(num(&listed, &["definition_count"]), 1.0);
    let definition = &at(&listed, &["definitions"]).array().unwrap()[0];
    assert_eq!(text(definition, "digest"), digest);
    assert_eq!(text(definition, "kind"), "material");
    assert_eq!(
        at(definition, &["used_by"]).array().unwrap()[0]
            .string()
            .unwrap(),
        text(plate, "element_id")
    );

    // An ordinary reader of the saved text sees the definition, its units and the pinned link.
    let authored = f.layer_text("authored.usda", &created);
    let prim = format!("def Scope \"MAT_{digest}\"");
    assert!(authored.contains(&prim), "{authored}");
    for line in [
        "custom token bim:units = \"mm\"",
        "custom string bim:schemaVersion = \"0.1.0\"",
        "custom token bim:definitionKind = \"material\"",
        "custom double bim:densityKgM3 = 7850.0",
    ] {
        assert!(authored.contains(line), "{line}\n{authored}");
    }
    assert!(authored.contains(&format!(
        "custom rel bim:material = </World/Definitions/Materials/MAT_{digest}>"
    )));
    // Physical material only: nothing in the layers is a shader, a colour or a surface binding.
    let derived = f.layer_text("derived.usda", &created);
    for layer in [&authored, &derived] {
        assert!(!layer.contains("Shader") && !layer.contains("material:binding"));
    }
}

#[test]
fn unknown_density_stays_unknown_and_never_selects_a_default() {
    let f = Fixture::new();
    let bare = f.ok("define-material", &[]);
    assert_eq!(*at(&bare, &["definition", "density_kg_m3"]), Json::Null);
    let digest = digest_of(&bare);
    let with_material = f.plate(&["--material-digest", &digest]);
    let mass = at(&with_material, &["plate", "quantities", "mass"]);
    assert_eq!(text(mass, "status"), "unknown");
    assert_eq!(*at(mass, &["kg"]), Json::Null);
    assert_eq!(*at(mass, &["density_kg_m3"]), Json::Null);
    // A plate with no material at all is the same: mass is unknown, volume is current.
    let none = f.plate(&[]);
    assert_eq!(
        text(at(&none, &["plate", "quantities", "mass"]), "status"),
        "unknown"
    );
    assert_eq!(*at(&none, &["plate", "material_digest"]), Json::Null);
    assert_eq!(
        text(at(&none, &["plate", "quantities", "volume"]), "status"),
        "current"
    );
    // The definition without density is distinct content from one with a density.
    let steel = f.ok("define-material", &["--density", STEEL]);
    assert_ne!(digest_of(&steel), digest);
    // Reopen: the layer carries no density attribute for the bare definition.
    let authored = f.layer_text("authored.usda", &steel);
    let bare_block = authored
        .split("def Scope \"MAT_")
        .find(|block| block.starts_with(&digest))
        .unwrap();
    let bare_block = bare_block.split('}').next().unwrap();
    assert!(!bare_block.contains("bim:densityKgM3"), "{bare_block}");
}

#[test]
fn the_material_changes_the_mass_and_not_the_shape() {
    let f = Fixture::new();
    let steel = digest_of(&f.ok("define-material", &["--density", STEEL]));
    let light = digest_of(&f.ok("define-material", &["--density", "2700"]));
    let a = f.plate(&["--material-digest", &steel]);
    let b = f.plate(&["--material-digest", &light]);
    let plain = f.plate(&[]);
    assert_eq!(num(&a, &["plate", "quantities", "mass", "kg"]), 39.25);
    assert_eq!(num(&b, &["plate", "quantities", "mass", "kg"]), 13.5);
    let mesh = |created: &Json| text(at(created, &["plate"]), "mesh_digest");
    assert_eq!(mesh(&a), mesh(&b));
    assert_eq!(mesh(&a), mesh(&plain));
    // The three plates and both materials survive a fresh reopen with their own links.
    let listed = f.ok("definitions", &[]);
    assert_eq!(num(&listed, &["definition_count"]), 2.0);
    for definition in at(&listed, &["definitions"]).array().unwrap() {
        assert_eq!(at(definition, &["used_by"]).array().unwrap().len(), 1);
    }
}

#[test]
fn used_by_lists_the_plates_that_pin_a_material_in_element_order_on_every_page() {
    let f = Fixture::new();
    let a = digest_of(&f.ok("define-material", &["--density", STEEL, "--name", "A"]));
    let b = digest_of(&f.ok("define-material", &["--density", "7800", "--name", "B"]));
    let unused = digest_of(&f.ok("define-material", &["--density", "2700", "--name", "U"]));
    let profile = digest_of(&f.ok("define-profile", &["--width", "200", "--height", "300"]));
    // Created out of id order: the lists follow the element ids, not the creation order.
    let id = |n: u8| format!("019a7bf0-0000-7000-8000-0000000000{n:02x}");
    for (n, material) in [
        (0x30, Some(&a)),
        (0x10, Some(&b)),
        (0x20, Some(&a)),
        (0x40, None),
    ] {
        let mut args = vec!["--new-id".to_owned(), id(n)];
        if let Some(material) = material {
            args.extend(["--material-digest".to_owned(), material.clone()]);
        }
        f.plate(&args.iter().map(String::as_str).collect::<Vec<_>>());
    }
    // A member that pins a material and a profile is not listed: `used_by` names plates.
    f.ok(
        "create-member",
        &[
            "--start",
            "0,0,0",
            "--end",
            "3000,0,0",
            "--profile-digest",
            &profile,
            "--material-digest",
            &a,
        ],
    );
    let users = |listed: &Json| -> Vec<(String, Vec<String>)> {
        at(listed, &["definitions"])
            .array()
            .unwrap()
            .iter()
            .map(|d| {
                let ids = at(d, &["used_by"]).array().unwrap();
                let ids = ids.iter().map(|i| i.string().unwrap().to_owned()).collect();
                (text(d, "digest"), ids)
            })
            .collect()
    };
    let all = users(&f.ok("definitions", &[]));
    let expected = |digest: &String| -> Vec<String> {
        match digest {
            d if *d == a => vec![id(0x20), id(0x30)],
            d if *d == b => vec![id(0x10)],
            _ => Vec::new(),
        }
    };
    assert_eq!(all.len(), 4);
    for (digest, ids) in &all {
        assert_eq!(ids, &expected(digest), "{digest}");
    }
    assert!(all.iter().any(|(d, _)| *d == unused));
    // A page lists the same users for each definition it shows.
    let page = users(&f.ok("definitions", &["--offset", "1", "--limit", "2"]));
    assert_eq!(page, all[1..3].to_vec());
}

#[test]
fn a_definition_is_immutable_and_publishing_it_again_writes_nothing() {
    let f = Fixture::new();
    let first = f.ok("define-profile", &["--width", "200", "--height", "300"]);
    assert_eq!(text(&first, "outcome"), "definition_created");
    assert_eq!(text(at(&first, &["definition"]), "kind"), "rectangle");
    assert_eq!(text(at(&first, &["definition"]), "units"), "mm");
    // The default display name is part of the content, hence of the digest.
    let expected = Resource::new(Definition::RectangleProfile {
        width_mm: 200.0,
        height_mm: 300.0,
        section: SectionProperties {
            display_name: Some("RECT 200x300".into()),
            ..Default::default()
        },
    })
    .unwrap()
    .digest
    .to_string();
    assert_eq!(digest_of(&first), expected);
    let again = f.ok("define-profile", &["--width", "200", "--height", "300"]);
    assert_eq!(text(&again, "outcome"), "definition_exists");
    assert_eq!(digest_of(&again), digest_of(&first));
    assert_eq!(
        text(at(&again, &["revision"]), "revision_id"),
        text(at(&first, &["revision"]), "revision_id")
    );
    assert_eq!(sequence(&again), 1);
    // Different content is a different identity: swapping width and height is not the same.
    let other = f.ok("define-profile", &["--width", "300", "--height", "200"]);
    assert_eq!(text(&other, "outcome"), "definition_created");
    assert_ne!(digest_of(&other), digest_of(&first));
    assert_eq!(sequence(&other), 2);
    let authored = f.layer_text("authored.usda", &other);
    assert!(authored.contains("custom double steel:profile:widthMm = 200.0"));
    assert!(authored.contains("custom double steel:profile:heightMm = 300.0"));
    let listed = f.ok("definitions", &[]);
    assert_eq!(num(&listed, &["definition_count"]), 2.0);
}

#[test]
fn negative_rectangle_dimensions_are_invalid_before_the_positive_dimension_cap() {
    let f = Fixture::new();
    for width in ["-3000000", "-1e308"] {
        assert_eq!(
            f.rejected("define-profile", &["--width", width, "--height", "300"]),
            "INVALID_ARGUMENT",
            "width {width}"
        );
    }
    assert_eq!(
        f.rejected("define-profile", &["--width", "1e308", "--height", "300"]),
        "LIMIT_EXCEEDED"
    );
    let created = f.ok("define-profile", &["--width", "200", "--height", "300"]);
    assert_eq!(
        sequence(&created),
        1,
        "rejected inputs must not publish revisions"
    );
}

#[test]
fn declared_section_properties_and_material_labels_are_stored_listed_and_pinned() {
    let f = Fixture::new();
    let profile = f.ok(
        "define-profile",
        &[
            "--width",
            "200",
            "--height",
            "300",
            "--designation",
            "RECT 200x300",
            "--standard",
            "Synthetic",
            "--area-mm2",
            "60000",
            "--mass-per-meter-kg",
            "471",
            "--surface-per-meter-m2",
            "1",
        ],
    );
    assert_eq!(text(&profile, "outcome"), "definition_created");
    let shown = at(&profile, &["definition"]);
    assert_eq!(text(shown, "designation"), "RECT 200x300");
    assert_eq!(num(shown, &["area_mm2"]), 60_000.0);
    assert_eq!(num(shown, &["mass_per_meter_kg"]), 471.0);
    let plain = f.ok("define-profile", &["--width", "200", "--height", "300"]);
    assert_eq!(text(&plain, "outcome"), "definition_created");
    assert_ne!(digest_of(&plain), digest_of(&profile));
    assert!(matches!(
        at(&plain, &["definition", "designation"]),
        Json::Null
    ));
    let steel = f.ok(
        "define-material",
        &[
            "--density",
            STEEL,
            "--grade",
            "S355",
            "--standard",
            "EN 10025-2",
        ],
    );
    assert_eq!(text(at(&steel, &["definition"]), "grade"), "S355");
    // The values are plain USD attributes in the saved layer, and a fresh process lists them.
    let authored = f.layer_text("authored.usda", &steel);
    assert!(authored.contains("custom string steel:profile:designation = \"RECT 200x300\""));
    assert!(authored.contains("custom double steel:profile:massPerMeterKg = 471.0"));
    assert!(authored.contains("custom string bim:materialGrade = \"S355\""));
    let listed = f.ok("definitions", &[]);
    assert_eq!(num(&listed, &["definition_count"]), 3.0);
    // A grade never selects a density: a material with only a grade stays unknown.
    let labelled = f.ok("define-material", &["--grade", "S355"]);
    assert!(matches!(
        at(&labelled, &["definition", "density_kg_m3"]),
        Json::Null
    ));
    // A declared area that disagrees with the rectangle formula is refused and publishes nothing.
    let before = sequence(&labelled);
    assert_eq!(
        f.rejected(
            "define-profile",
            &["--width", "200", "--height", "300", "--area-mm2", "1"]
        ),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.rejected(
            "define-profile",
            &["--width", "200", "--height", "300", "--designation", ""]
        ),
        "INVALID_ARGUMENT"
    );
    // Nothing was published by the refusals: the head is still the last accepted revision.
    assert_eq!(
        sequence(&f.ok("define-profile", &["--width", "200", "--height", "300"])),
        before
    );
}

#[test]
fn rejections_use_the_f04_codes_and_publish_nothing() {
    let f = Fixture::new();
    let steel = digest_of(&f.ok("define-material", &["--density", STEEL]));
    let profile = digest_of(&f.ok("define-profile", &["--width", "200", "--height", "300"]));
    for (extra, code) in [
        (vec!["--density", "-1"], "INVALID_ARGUMENT"),
        (vec!["--density", "-7850"], "INVALID_ARGUMENT"),
        (vec!["--density", "1000000.5"], "LIMIT_EXCEEDED"),
        (vec!["--density", "NaN"], "NON_FINITE_NUMBER"),
        (vec!["--density", "inf"], "NON_FINITE_NUMBER"),
        (vec!["--density", "heavy"], "INVALID_ARGUMENT"),
    ] {
        assert_eq!(f.rejected("define-material", &extra), code, "{extra:?}");
    }
    for (width, height, code) in [
        ("0.0009", "300", "INVALID_ARGUMENT"),
        ("200", "0", "INVALID_ARGUMENT"),
        ("-200", "300", "INVALID_ARGUMENT"),
        ("200", "1000001", "LIMIT_EXCEEDED"),
        ("NaN", "300", "NON_FINITE_NUMBER"),
    ] {
        assert_eq!(
            f.rejected("define-profile", &["--width", width, "--height", height]),
            code,
            "{width} x {height}"
        );
    }
    let plate = ["--width", "1000", "--length", "500", "--thickness", "10"];
    let with = |digest: &str| {
        let mut args = plate.to_vec();
        args.extend_from_slice(&["--material-digest", digest]);
        f.rejected("create-plate", &args)
    };
    assert_eq!(with(&"0".repeat(64)), "NOT_FOUND");
    assert_eq!(with(&profile), "INVALID_ARGUMENT");
    assert_eq!(with("not-a-digest"), "INVALID_ARGUMENT");
    assert_eq!(with(&steel.to_uppercase()), "INVALID_ARGUMENT");
    // Nothing was created by any rejection: still the two definitions and no plate.
    let inspected = f.ok("inspect-elements", &[]);
    assert_eq!(num(&inspected, &["plate_count"]), 0.0);
    assert_eq!(sequence(&inspected), 2);
    assert_eq!(num(&f.ok("definitions", &[]), &["definition_count"]), 2.0);
}

#[test]
fn a_retried_operation_with_a_material_is_replayed_and_a_changed_material_is_reuse() {
    let f = Fixture::new();
    let steel = digest_of(&f.ok("define-material", &["--density", STEEL]));
    let light = digest_of(&f.ok("define-material", &["--density", "2700"]));
    let ids = [
        "--new-id",
        "01a00000-0000-7000-8000-000000000001",
        "--op-id",
        "01a00000-0000-7000-8000-000000000002",
    ];
    let create = |material: &str| {
        let mut args = vec![
            "--width",
            "1000",
            "--length",
            "500",
            "--thickness",
            "10",
            "--material-digest",
            material,
        ];
        args.extend_from_slice(&ids);
        f.run("create-plate", &args)
    };
    let (code, first) = create(&steel);
    assert_eq!(code, 0);
    assert_eq!(text(&first, "outcome"), "plate_created");
    let (code, replay) = create(&steel);
    assert_eq!(code, 0);
    assert_eq!(text(&replay, "outcome"), "plate_replayed");
    assert_eq!(num(&replay, &["plate", "quantities", "mass", "kg"]), 39.25);
    let (code, reuse) = create(&light);
    assert_eq!(code, 2);
    assert_eq!(text(&reuse, "code"), "OP_ID_REUSE");
}
