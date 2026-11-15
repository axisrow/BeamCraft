// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic libraries, environments and sections that follow no national
// standard, MPL-2.0. Every library and environment is written by the tests from plain text.

//! Layering B end to end through the real `beamcraft` binary: the user-owned firm layer of
//! `libraries/` in the application data sits above the environment and below the project's own
//! copy, the shipped neutral blank environment is the built-in fallback, the shadowed
//! designations are reported with both layers, a firm file that does not read is reported and
//! skipped, and no environment update ever writes into `libraries/`. Every run names its own
//! `BEAMCRAFT_CONFIG_DIR`, so the user's real application data is never read or written.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support::{self, at, list, text};

use beamcraft_json::{Json, Limits};
use std::fs;
use std::process::Command;

const ACTOR: &str = "0199e7a0-0000-7000-8000-00000000006f";

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("firm-layer-tests"),
        };
        fs::create_dir_all(fixture.shared.root.join("config/libraries")).unwrap();
        fs::create_dir(fixture.shared.root.join("models")).unwrap();
        fixture
    }
    fn path(&self, name: &str) -> String {
        self.shared.root.join(name).to_str().unwrap().to_owned()
    }
    fn raw(&self, args: &[&str]) -> (i32, Json) {
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(args)
            .env("BEAMCRAFT_CONFIG_DIR", self.path("config"))
            .output()
            .unwrap();
        let body = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
        (output.status.code().unwrap(), body)
    }
    fn ok(&self, args: &[&str]) -> Json {
        let (code, body) = self.raw(args);
        assert_eq!(code, 0, "{args:?}: {body:?}");
        body
    }
    fn code(&self, args: &[&str]) -> String {
        let (code, body) = self.raw(args);
        assert_eq!(code, 2, "{args:?}: {body:?}");
        text(&body, "code")
    }
    /// Writes one firm library file into the application data's `libraries/`.
    fn firm(&self, name: &str, text: &str) -> String {
        let path = self.shared.root.join("config/libraries").join(name);
        fs::write(&path, text).unwrap();
        path.to_str().unwrap().to_owned()
    }
    /// Installs the synthetic `acme` environment into the application data: one profile
    /// library with `X-1` (200 mm) and `AC-ONLY` (210 mm), no fastener libraries.
    fn install_acme(&self) {
        let dir = self.shared.root.join("config/environments/acme");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("environment.usda"),
            "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n    customLayerData = {\n        string \"bim:environment:id\" = \"acme\"\n        string \"bim:environment:title\" = \"Acme\"\n        string[] \"bim:environment:libraries\" = [\"acme.usda\"]\n    }\n)\n",
        )
        .unwrap();
        fs::write(dir.join("acme.usda"), self.acme_library(200.0, 210.0)).unwrap();
    }
    /// The acme profile library with `X-1` at `x_height` and `AC-ONLY` at `only_height`:
    /// rewriting it with other heights stands in for an environment update. Every entry prim
    /// carries its own name: two prims of one path are one merged prim in USD.
    fn acme_library(&self, x_height: f64, only_height: f64) -> String {
        let entry = |prim: &str, designation: &str, height: f64| {
            format!(
                "        def Scope \"{prim}\"\n        {{\nstring steel:profile:designation = \"{designation}\"\ntoken steel:profile:family = \"i_shape\"\ndouble steel:profile:hMm = {height}\ndouble steel:profile:bMm = 150\ndouble steel:profile:twMm = 7\ndouble steel:profile:tfMm = 10\ndouble steel:profile:rMm = 15\n        }}\n"
            )
        };
        format!(
            "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n    customLayerData = {{\n        string \"bim:library:id\" = \"acme-lib\"\n        string \"bim:library:source\" = \"Contributor-original synthetic values for tests\"\n        string \"bim:library:licence\" = \"MPL-2.0\"\n    }}\n)\ndef Scope \"Library\"\n{{\n    def Scope \"Profiles\"\n    {{\n{}{}    }}\n}}\n",
            entry("EX", "X-1", x_height),
            entry("EA", "AC-ONLY", only_height)
        )
    }
    /// Creates a project named `name` with the given extra options and answers its store path.
    fn create(&self, name: &str, extra: &[&str]) -> String {
        let models = self.path("models");
        let mut args: Vec<&str> = vec![
            "create-project",
            "--parent",
            &models,
            "--name",
            name,
            "--actor-id",
            ACTOR,
        ];
        args.extend_from_slice(extra);
        text(at(&self.ok(&args), &["project"]), "path")
    }
    /// The height the name resolves to in the store, after it was defined by its first use.
    fn profile_height(&self, store: &str, name: &str) -> f64 {
        let defined = self.ok(&["definitions", "--store", store, "--name", name]);
        let found = list(&defined, "definitions");
        assert_eq!(found.len(), 1, "{name}: {found:?}");
        at(&found[0], &["parameters_mm", "h"]).float().unwrap()
    }
    fn member(&self, store: &str, profile: &str) -> Json {
        self.ok(&[
            "create-member",
            "--store",
            store,
            "--kind",
            "beam",
            "--start",
            "0,0,0",
            "--end",
            "6000,0,0",
            "--profile",
            profile,
            "--material",
            "Steel",
        ])
    }
}

fn firm_profiles(entries: &[(&str, f64)]) -> String {
    let bodies: String = entries
        .iter()
        .enumerate()
        .map(|(n, (designation, height))| {
            format!(
                "        def Scope \"E{n}\"\n        {{\nstring steel:profile:designation = \"{designation}\"\ntoken steel:profile:family = \"i_shape\"\ndouble steel:profile:hMm = {height}\ndouble steel:profile:bMm = 150\ndouble steel:profile:twMm = 7\ndouble steel:profile:tfMm = 10\ndouble steel:profile:rMm = 15\n        }}\n"
            )
        })
        .collect();
    format!(
        "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n    customLayerData = {{\n        string \"bim:library:id\" = \"firm-lib\"\n    }}\n)\ndef Scope \"Library\"\n{{\n    def Scope \"Profiles\"\n    {{\n{bodies}    }}\n}}\n"
    )
}

fn firm_bolts() -> String {
    "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n    customLayerData = {\n        string \"bim:library:id\" = \"firm-bolts\"\n        string \"bim:library:source\" = \"Contributor-original synthetic values for tests\"\n        string \"bim:library:licence\" = \"MPL-2.0\"\n    }\n)\ndef Scope \"Library\"\n{\n    def Scope \"Fasteners\"\n    {\n        def Scope \"SYN_M12\"\n        {\nstring steel:fastener:designation = \"SYN-M12\"\ntoken steel:fastener:kind = \"bolt\"\ntoken steel:fastener:headKind = \"hex\"\ndouble steel:fastener:nominalDiameterMm = 12\ndouble steel:fastener:headAcrossFlatsMm = 18\ndouble steel:fastener:headHeightMm = 7.5\ndouble steel:fastener:nutAcrossFlatsMm = 18\ndouble steel:fastener:nutHeightMm = 10.8\ndouble steel:fastener:holeClearanceMm = 1\ndouble[] steel:fastener:lengthsMm = [30, 40, 50, 70, 80]\ndouble steel:fastener:threadProtrusionMm = 4\nbool steel:fastener:washerUnderHead = false\nbool steel:fastener:washerUnderNut = false\ntoken steel:fastener:holeMode = \"generate\"\n        }\n    }\n}\n".to_string()
}

#[test]
fn the_firm_layer_fills_a_seeded_project_below_its_copy_and_reports_shadowing() {
    let f = Fixture::new();
    f.install_acme();
    f.firm(
        "mine.usda",
        &firm_profiles(&[("X-1", 320.0), ("FIRM-ONLY", 280.0)]),
    );

    let store = f.create("Seeded", &["--environment", "acme"]);
    // The project's own copy is stronger than the firm layer: X-1 resolves to the copy's 200,
    // not the firm file's 320.
    f.member(&store, "X-1");
    assert_eq!(f.profile_height(&store, "X-1"), 200.0);
    // A designation the copy does not declare is filled by the firm layer.
    f.member(&store, "FIRM-ONLY");
    assert_eq!(f.profile_height(&store, "FIRM-ONLY"), 280.0);
    // And a designation no layer above the neutral environment declares falls back to it.
    f.member(&store, "I-200x100");
    assert_eq!(f.profile_height(&store, "I-200x100"), 200.0);

    // The listing shows the whole chain, every entry's layer, and the shadowed designations
    // with both layers.
    let listed = f.ok(&["list-library", "--project", &store]);
    let layers: Vec<String> = list(&listed, "libraries")
        .iter()
        .map(|s| text(s, "layer"))
        .collect();
    assert_eq!(
        layers,
        ["project", "firm", "neutral"],
        "the project copy, then firm, then the neutral fallback"
    );
    let of_layer = |layer: &str| -> Vec<String> {
        list(&listed, "entries")
            .iter()
            .filter(|e| text(e, "layer") == layer)
            .map(|e| text(e, "designation"))
            .collect()
    };
    assert!(of_layer("project").contains(&"X-1".to_owned()));
    assert_eq!(of_layer("firm"), ["FIRM-ONLY"]);
    assert!(of_layer("neutral").contains(&"I-200x100".to_owned()));
    let shadowed = list(&listed, "shadowed");
    let shadow_of = |designation: &str| -> Vec<Json> {
        shadowed
            .iter()
            .filter(|s| text(s, "designation") == designation)
            .cloned()
            .collect()
    };
    let x1 = shadow_of("X-1");
    assert_eq!(
        x1.len(),
        1,
        "the firm entry is shadowed by the project copy"
    );
    assert_eq!(text(&x1[0], "used_library"), "acme-lib");
    assert_eq!(text(&x1[0], "used_layer"), "project");
    assert_eq!(text(&x1[0], "shadowed_library"), "firm-lib");
    assert_eq!(text(&x1[0], "shadowed_layer"), "firm");
    let firm_only = shadow_of("FIRM-ONLY");
    assert_eq!(
        firm_only.len(),
        0,
        "the neutral environment does not declare FIRM-ONLY, so nothing shadows the firm entry"
    );
    assert_eq!(
        text(at(&listed, &["environment"]), "id"),
        "acme",
        "the chain carries the copy's own declarations"
    );

    // `show-library` names the layer that answers and the shadows of its designation.
    let shown = f.ok(&["show-library", "--project", &store, "--designation", "X-1"]);
    assert_eq!(text(at(&shown, &["library"]), "id"), "acme-lib");
    assert_eq!(num(&shown, &["entry", "parameters_mm", "h"]), 200.0);
    let shadows = list(&shown, "shadowed");
    assert_eq!(shadows.len(), 1);
    assert_eq!(text(&shadows[0], "shadowed_layer"), "firm");

    // A project created with no environment at all resolves from firm and neutral alone; it
    // has no seeded materials, so its members name no --material.
    let bare = f.create("Bare", &[]);
    f.ok(&[
        "create-member",
        "--store",
        &bare,
        "--kind",
        "beam",
        "--start",
        "0,0,0",
        "--end",
        "6000,0,0",
        "--profile",
        "FIRM-ONLY",
    ]);
    assert_eq!(f.profile_height(&bare, "FIRM-ONLY"), 280.0);
    f.ok(&[
        "create-member",
        "--store",
        &bare,
        "--kind",
        "beam",
        "--start",
        "0,0,0",
        "--end",
        "6000,0,0",
        "--profile",
        "H-200x200",
    ]);
    assert_eq!(f.profile_height(&bare, "H-200x200"), 200.0);
    let defined = f.ok(&["definitions", "--store", &bare]);
    let available = list(&defined, "environment_profiles")
        .iter()
        .map(|p| text(p, "designation"))
        .collect::<Vec<_>>();
    assert!(
        available.contains(&"I-300x150".to_owned()),
        "the neutral environment fills the chain of an unseeded project: {available:?}"
    );
    assert!(!available.contains(&"AC-ONLY".to_owned()));
}

#[test]
fn the_firm_layer_survives_an_environment_update_and_is_never_written() {
    let f = Fixture::new();
    f.install_acme();
    let firm_path = f.firm(
        "mine.usda",
        &firm_profiles(&[("X-1", 320.0), ("FIRM-ONLY", 280.0)]),
    );
    let firm_before = fs::read(&firm_path).unwrap();
    let libraries_before = fs::read_dir(f.shared.root.join("config/libraries"))
        .unwrap()
        .count();

    let store = f.create("Pinned", &["--environment", "acme"]);
    f.member(&store, "AC-ONLY");
    assert_eq!(f.profile_height(&store, "AC-ONLY"), 210.0);

    // An environment update: the acme library in the application data changes its heights.
    fs::write(
        f.shared.root.join("config/environments/acme/acme.usda"),
        f.acme_library(999.0, 999.0),
    )
    .unwrap();

    // The project still resolves through its own copy, and the firm layer is untouched: no
    // command of this run ever wrote into libraries/.
    f.member(&store, "X-1");
    assert_eq!(
        f.profile_height(&store, "X-1"),
        200.0,
        "the copy answers, not the updated environment"
    );
    assert_eq!(
        f.profile_height(&store, "AC-ONLY"),
        210.0,
        "the copy keeps the values it was created with"
    );
    f.member(&store, "FIRM-ONLY");
    assert_eq!(
        f.profile_height(&store, "FIRM-ONLY"),
        280.0,
        "the firm layer answers unchanged"
    );
    assert_eq!(
        fs::read(&firm_path).unwrap(),
        firm_before,
        "the firm file is byte for byte"
    );
    assert_eq!(
        fs::read_dir(f.shared.root.join("config/libraries"))
            .unwrap()
            .count(),
        libraries_before,
        "no file was added to or removed from libraries/"
    );
    let listed = f.ok(&["list-library", "--project", &store]);
    let project_entries = list(&listed, "libraries")
        .iter()
        .find(|s| text(s, "layer") == "project")
        .unwrap();
    assert_eq!(
        num(project_entries, &["entry_count"]),
        2.0,
        "the project copy still carries the environment it was created with"
    );
}

#[test]
fn a_firm_file_that_does_not_read_is_reported_and_skipped() {
    let f = Fixture::new();
    f.install_acme();
    f.firm("broken.usda", "not usd at all");
    let store = f.create("Reported", &["--environment", "acme"]);

    let listed = f.ok(&["list-library", "--project", &store]);
    let refused = list(&listed, "firm_refused");
    assert_eq!(refused.len(), 1);
    assert!(
        text(&refused[0], "path").ends_with("broken.usda"),
        "{}",
        text(&refused[0], "path")
    );
    assert_eq!(text(&refused[0], "code"), "INVALID_ARGUMENT");
    // The chain still resolves; the broken file is skipped, never silently.
    let shown = f.ok(&[
        "show-library",
        "--project",
        &store,
        "--designation",
        "AC-ONLY",
    ]);
    assert_eq!(text(at(&shown, &["library"]), "id"), "acme-lib");
    let refused = list(&shown, "firm_refused");
    assert_eq!(refused.len(), 1);
    assert!(text(&refused[0], "path").ends_with("broken.usda"));
}

#[test]
fn a_firm_fastener_library_rides_the_project_chain() {
    let f = Fixture::new();
    f.install_acme();
    f.firm("bolts.usda", &firm_bolts());
    // The acme environment declares no fastener libraries; the firm layer provides them.
    let store = f.create("Bolts", &["--environment", "acme"]);
    let p1 = text(
        at(
            &f.ok(&[
                "create-plate",
                "--store",
                &store,
                "--width",
                "200",
                "--length",
                "200",
                "--thickness",
                "20",
            ]),
            &["plate"],
        ),
        "element_id",
    );
    let p2 = text(
        at(
            &f.ok(&[
                "create-plate",
                "--store",
                &store,
                "--width",
                "200",
                "--length",
                "200",
                "--thickness",
                "20",
                "--translation",
                "0,0,20",
            ]),
            &["plate"],
        ),
        "element_id",
    );
    // Grip 40 + head 7.5 + nut 10.8 + protrusion 4 = 62.3, so the firm set answers 70.
    let made = f.ok(&[
        "create-bolt-group",
        "--store",
        &store,
        "--name",
        "SYN",
        "--parts",
        &format!("{p1},{p2}"),
        "--designation",
        "SYN-M12",
        "--point1",
        "50,50,40",
        "--point2",
        "50,150,40",
        "--plane-normal",
        "0,0,-1",
        "--offset-start",
        "25",
        "--count",
        "2",
        "--pitch",
        "50",
        "--hole-mode",
        "none",
    ]);
    let groups = list(&made, "bolt_groups");
    assert_eq!(groups.len(), 1);
    assert_eq!(text(&groups[0], "length_mm"), "70");
    // The pinned definition carries the firm entry's declared values.
    let definitions = f.ok(&["definitions", "--store", &store, "--name", "SYN-M12"]);
    let pinned = list(&definitions, "definitions");
    assert_eq!(pinned.len(), 1);
    assert_eq!(text(&pinned[0], "kind"), "fastener");
    assert_eq!(num(&pinned[0], &["nominal_diameter_mm"]), 12.0);
}

#[test]
fn project_is_an_alternative_to_the_named_sources() {
    let f = Fixture::new();
    f.install_acme();
    let store = f.create("Alt", &["--environment", "acme"]);
    let lib = f.firm("mine.usda", &firm_profiles(&[("X-1", 320.0)]));
    assert_eq!(
        f.code(&["list-library", "--project", &store, "--library", &lib]),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.code(&["list-library", "--project", &store, "--user-library", &lib]),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.code(&[
            "show-library",
            "--project",
            &store,
            "--user-library",
            &lib,
            "--designation",
            "X-1"
        ]),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.code(&["show-library", "--project", &store]),
        "INVALID_ARGUMENT",
        "show-library still needs --designation"
    );
    // The named sources keep their exact semantics: a single --library lists that file alone.
    let alone = f.ok(&["list-library", "--library", &lib]);
    assert_eq!(num(&alone, &["entry_count"]), 1.0);
    let libraries = list(&alone, "libraries");
    assert_eq!(libraries.len(), 1);
    assert_eq!(text(&libraries[0], "layer"), "library");
}

fn num(json: &Json, path: &[&str]) -> f64 {
    at(json, path).float().unwrap()
}
