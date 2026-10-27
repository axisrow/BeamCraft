// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic fasteners that follow no national standard, MPL-2.0. The DIN
// set is the shipped `assets/libraries/din-fasteners.usda` (public-standard dimensions re-entered
// as our own data); the hostile and partial libraries are written by the tests from plain text.

//! A14.1 end to end through the real `beamcraft` binary: a fastener library in the A07 format is
//! listed and resolved by designation, `define-fastener` pins the definition into the project by
//! digest, an unknown designation is refused, and the pinned definition needs the library file no
//! longer.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_json::{Json, Limits};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

const SYNTHETIC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/libraries/synthetic-fasteners.usda"
);
const DIN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/libraries/din-fasteners.usda"
);

struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("a14-1-tests");
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
    fn file(&self, name: &str, bytes: &[u8]) -> String {
        let path = self.root.join(name);
        fs::write(&path, bytes).unwrap();
        path.to_str().unwrap().to_owned()
    }
    fn raw(&self, args: &[&str]) -> (i32, Json) {
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(args)
            .output()
            .unwrap();
        let body = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
        (output.status.code().unwrap(), body)
    }
    fn ok(&self, command: &str, extra: &[&str]) -> Json {
        let mut args = vec![command];
        args.extend_from_slice(extra);
        let (code, body) = self.raw(&args);
        assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
        body
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

fn text(json: &Json, key: &str) -> String {
    json.get(key).unwrap().string().unwrap().to_owned()
}
fn at<'a>(json: &'a Json, path: &[&str]) -> &'a Json {
    path.iter().fold(json, |j, k| j.get(k).unwrap())
}
fn num(json: &Json, path: &[&str]) -> f64 {
    at(json, path).float().unwrap()
}
fn digest_of(reply: &Json) -> String {
    text(at(reply, &["definition"]), "digest")
}
fn sequence(json: &Json) -> u64 {
    text(at(json, &["revision"]), "sequence").parse().unwrap()
}

#[test]
fn the_shipped_libraries_list_and_show_their_fasteners() {
    let f = Fixture::new();
    let listed = f.raw(&["list-library", "--fastener-library", SYNTHETIC]).1;
    assert_eq!(num(&listed, &["entry_count"]), 3.0);
    assert_eq!(num(&listed, &["rejected_count"]), 0.0);
    let designations: Vec<String> = at(&listed, &["entries"])
        .array()
        .unwrap()
        .iter()
        .map(|e| text(e, "designation"))
        .collect();
    assert_eq!(
        designations,
        ["SYN-BOLT-12x50", "SYN-BOLT-16x70-BARE", "SYN-SCREW-10x30"]
    );
    // The DIN set answers by designation with its recorded source and licence.
    let shown = f.raw(&[
        "show-library",
        "--fastener-library",
        DIN,
        "--designation",
        "M16",
    ]);
    assert_eq!(shown.0, 0);
    let entry = at(&shown.1, &["entry"]);
    assert_eq!(text(entry, "kind"), "fastener");
    assert_eq!(num(entry, &["nominal_diameter_mm"]), 16.0);
    assert_eq!(num(entry, &["head_across_flats_mm"]), 24.0);
    assert_eq!(num(entry, &["washer_inner_diameter_mm"]), 17.0);
    assert_eq!(text(entry, "standard"), "DIN EN ISO 4014");
    assert_eq!(text(entry, "grade"), "8.8");
    assert_eq!(
        text(at(&shown.1, &["library"]), "source"),
        "Public standards DIN EN ISO 4014/4032/7089 and EN 1090-2, re-entered as our own data; thread protrusion is a contributor default"
    );
    assert_eq!(text(at(&shown.1, &["library"]), "licence"), "MPL-2.0");
    // A fastener library does not combine with the profile lookup options.
    let (code, body) = f.raw(&["list-library", "--fastener-library", DIN, "--library", DIN]);
    assert_eq!(code, 2, "{body:?}");
    assert_eq!(text(&body, "code"), "INVALID_ARGUMENT");
}

#[test]
fn define_fastener_pins_the_entry_and_needs_the_library_no_longer() {
    let f = Fixture::new();
    let predicted = {
        let shown = f.raw(&[
            "show-library",
            "--fastener-library",
            DIN,
            "--designation",
            "M16",
        ]);
        text(at(&shown.1, &["entry"]), "digest")
    };
    // Work from a private copy and delete it after pinning: the model must not need the file.
    let copy = f.file("din.usda", &fs::read(DIN).unwrap());
    let created = f.ok(
        "define-fastener",
        &[
            "--store",
            &f.store(),
            "--fastener-library",
            &copy,
            "--designation",
            "M16",
        ],
    );
    assert_eq!(text(&created, "outcome"), "definition_created");
    assert_eq!(
        digest_of(&created),
        predicted,
        "the entry predicts the digest"
    );
    let definition = at(&created, &["definition"]);
    assert_eq!(text(definition, "kind"), "fastener");
    assert_eq!(text(definition, "display_name"), "M16");
    assert_eq!(
        text(definition, "prim_path"),
        format!("/World/Definitions/Fasteners/FST_{}", digest_of(&created))
    );
    assert_eq!(text(definition, "fastener_kind"), "bolt");
    assert_eq!(text(definition, "hole_mode"), "generate");
    assert_eq!(num(definition, &["nut_height_mm"]), 14.8);
    assert_eq!(at(definition, &["lengths_mm"]).array().unwrap().len(), 11);
    fs::remove_file(&copy).unwrap();
    let authored = f.authored_text(&created);
    assert!(
        authored.contains("\"Fasteners\"")
            && authored.contains(&format!("FST_{}", digest_of(&created))),
        "the pinned definition lives in the model, not the library"
    );
    assert!(authored.contains("double steel:fastener:nominalDiameterMm = 16.0"));
    assert!(authored.contains("token steel:fastener:holeMode = \"generate\""));

    // The same entry again is the same definition: nothing is written.
    let again = f.raw(&[
        "define-fastener",
        "--store",
        &f.store(),
        "--fastener-library",
        DIN,
        "--designation",
        "M16",
    ]);
    assert_eq!(text(&again.1, "outcome"), "definition_exists");
    assert_eq!(sequence(&again.1), 1);

    // A fresh process lists the pinned fastener definition from the store alone.
    let definitions = f.ok("definitions", &["--store", &f.store()]);
    assert_eq!(num(&definitions, &["definition_count"]), 1.0);
    let listed = at(&definitions, &["definitions"]).array().unwrap()[0].clone();
    assert_eq!(text(&listed, "kind"), "fastener");
    assert_eq!(text(&listed, "designation"), "M16");
    assert_eq!(num(&listed, &["head_height_mm"]), 10.0);
    // Selection by name finds it too, so a later group command can resolve it.
    let by_name = f.ok("definitions", &["--store", &f.store(), "--name", "M16"]);
    assert_eq!(num(&by_name, &["definition_count"]), 1.0);
}

#[test]
fn an_unknown_or_damaged_fastener_entry_changes_nothing() {
    let f = Fixture::new();
    let created = f.ok(
        "define-fastener",
        &[
            "--store",
            &f.store(),
            "--fastener-library",
            DIN,
            "--designation",
            "M12",
        ],
    );
    assert_eq!(sequence(&created), 1);
    // An unknown designation is refused, never replaced by a near one.
    for designation in ["M12x40", "m12", "M24"] {
        let (code, body) = f.raw(&[
            "define-fastener",
            "--store",
            f.store().as_str(),
            "--fastener-library",
            DIN,
            "--designation",
            designation,
        ]);
        assert_eq!(code, 2, "{designation}: {body:?}");
        assert_eq!(text(&body, "code"), "NOT_FOUND", "{designation}");
    }
    // A refused entry (a bolt without a nut) is reported, and its designation cannot pin.
    const HEADER: &str = "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n    customLayerData = {\n        string \"bim:library:id\" = \"hostile\"\n    }\n)\n";
    let damaged = format!(
        "{HEADER}def Scope \"Library\"\n{{\n    def Scope \"Fasteners\"\n    {{\n        def Scope \"B\"\n        {{\n            string steel:fastener:designation = \"NO-NUT\"\n            token steel:fastener:kind = \"bolt\"\n            token steel:fastener:headKind = \"hex\"\n            double steel:fastener:nominalDiameterMm = 12\n            double steel:fastener:headAcrossFlatsMm = 18\n            double steel:fastener:headHeightMm = 7.5\n            double steel:fastener:holeClearanceMm = 1\n            double[] steel:fastener:lengthsMm = [30]\n            double steel:fastener:threadProtrusionMm = 4\n            bool steel:fastener:washerUnderHead = false\n            bool steel:fastener:washerUnderNut = false\n            token steel:fastener:holeMode = \"generate\"\n        }}\n    }}\n}}\n"
    );
    let path = f.file("hostile.usda", damaged.as_bytes());
    let listed = f.raw(&["list-library", "--fastener-library", &path]);
    assert_eq!(listed.0, 0);
    assert_eq!(num(&listed.1, &["rejected_count"]), 1.0);
    let (code, body) = f.raw(&[
        "define-fastener",
        "--store",
        &f.store(),
        "--fastener-library",
        &path,
        "--designation",
        "NO-NUT",
    ]);
    assert_eq!(code, 2, "{body:?}");
    assert_eq!(text(&body, "code"), "INVALID_ARGUMENT");
    // The store still holds exactly the one healthy definition.
    let definitions = f.ok("definitions", &["--store", &f.store()]);
    assert_eq!(num(&definitions, &["definition_count"]), 1.0);
    assert_eq!(
        text(
            &at(&definitions, &["definitions"]).array().unwrap()[0],
            "designation"
        ),
        "M12"
    );
}

#[test]
fn every_shipped_entry_pins_its_predicted_digest() {
    let f = Fixture::new();
    for library in [SYNTHETIC, DIN] {
        let listed = f.raw(&["list-library", "--fastener-library", library]).1;
        for entry in at(&listed, &["entries"]).array().unwrap() {
            let designation = text(entry, "designation");
            let created = f.ok(
                "define-fastener",
                &[
                    "--store",
                    &f.store(),
                    "--fastener-library",
                    library,
                    "--designation",
                    &designation,
                ],
            );
            assert_eq!(digest_of(&created), text(entry, "digest"), "{designation}");
        }
    }
    let definitions = f.ok("definitions", &["--store", &f.store()]);
    assert_eq!(num(&definitions, &["definition_count"]), 6.0);
}
