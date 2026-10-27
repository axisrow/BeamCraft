// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic libraries and sections that follow no national standard,
// MPL-2.0. Every library and environment is written by the tests from plain text or by
// `add-library` itself.

//! A07.3 end to end through the real `beamcraft` binary: an environment orders libraries and the
//! strongest one answers a designation, a user library comes first, a hostile environment is
//! refused, and `add-library` writes admitted entries into a library file that reads back.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

mod support;

use support::{at, text};

use beamcraft_json::{Json, Limits};
use std::fs;
use std::path::Path;
use std::process::Command;

struct Fixture {
    shared: support::StoreFixture,
}
impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("a07-3-tests"),
        };
        fs::create_dir(fixture.shared.root.join("libs")).unwrap();
        fixture
    }
    fn path(&self, name: &str) -> String {
        self.shared.root.join(name).to_str().unwrap().to_owned()
    }
    fn write(&self, name: &str, text: &str) -> String {
        fs::write(self.shared.root.join(name), text).unwrap();
        self.path(name)
    }
    fn raw(&self, args: &[&str]) -> (i32, Json) {
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(args)
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
}
fn num(json: &Json, path: &[&str]) -> f64 {
    at(json, path).float().unwrap()
}
fn designations(listed: &Json) -> Vec<String> {
    at(listed, &["entries"])
        .array()
        .unwrap()
        .iter()
        .map(|e| text(e, "designation"))
        .collect()
}

fn library(id: &str, entries: &[(&str, f64)]) -> String {
    let mut out = format!(
        "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n    customLayerData = {{\n        string \"bim:library:id\" = \"{id}\"\n    }}\n)\ndef Scope \"Library\"\n{{\n    def Scope \"Profiles\"\n    {{\n"
    );
    for (n, (designation, height)) in entries.iter().enumerate() {
        out.push_str(&format!(
            "        def Scope \"E{n}\"\n        {{\nstring steel:profile:designation = \"{designation}\"\ntoken steel:profile:family = \"i_shape\"\ndouble steel:profile:hMm = {height}\ndouble steel:profile:bMm = 150\ndouble steel:profile:twMm = 7\ndouble steel:profile:tfMm = 10\ndouble steel:profile:rMm = 15\n        }}\n"
        ));
    }
    out.push_str("    }\n}\n");
    out
}

fn environment(libraries: &str) -> String {
    format!(
        "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n    customLayerData = {{\n        string \"bim:environment:id\" = \"env\"\n        string \"bim:environment:title\" = \"Test environment\"\n        string[] \"bim:environment:libraries\" = [{libraries}]\n    }}\n)\n"
    )
}

#[test]
fn the_strongest_library_of_an_environment_answers_and_shadowing_is_reported() {
    let f = Fixture::new();
    f.write(
        "libs/strong.usda",
        &library("strong", &[("X-1", 300.0), ("S-ONLY", 250.0)]),
    );
    f.write(
        "libs/weak.usda",
        &library("weak", &[("X-1", 200.0), ("W-ONLY", 180.0)]),
    );
    let env = f.write(
        "env.usda",
        &environment("\"libs/strong.usda\", \"libs/weak.usda\""),
    );

    let listed = f.ok(&["list-library", "--environment", &env]);
    assert_eq!(text(&listed, "outcome"), "library");
    assert_eq!(designations(&listed), ["S-ONLY", "W-ONLY", "X-1"]);
    assert_eq!(num(&listed, &["entry_count"]), 3.0);
    assert_eq!(text(at(&listed, &["environment"]), "id"), "env");
    assert_eq!(
        at(&listed, &["environment", "libraries"])
            .array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(at(&listed, &["libraries"]).array().unwrap().len(), 2);
    let shadowed = at(&listed, &["shadowed"]).array().unwrap();
    assert_eq!(shadowed.len(), 1);
    assert_eq!(text(&shadowed[0], "designation"), "X-1");
    assert_eq!(text(&shadowed[0], "used_library"), "strong");
    assert_eq!(text(&shadowed[0], "shadowed_library"), "weak");
    let x1 = &at(&listed, &["entries"]).array().unwrap()[2];
    assert_eq!(text(x1, "library_id"), "strong");
    assert_eq!(num(x1, &["parameters_mm", "h"]), 300.0);

    // `show-library` names the library that answers.
    let shown = f.ok(&[
        "show-library",
        "--environment",
        &env,
        "--designation",
        "X-1",
    ]);
    assert_eq!(text(at(&shown, &["library"]), "id"), "strong");
    assert_eq!(num(&shown, &["entry", "parameters_mm", "h"]), 300.0);
    let weak_only = f.ok(&[
        "show-library",
        "--environment",
        &env,
        "--designation",
        "W-ONLY",
    ]);
    assert_eq!(text(at(&weak_only, &["library"]), "id"), "weak");
    assert_eq!(
        f.code(&[
            "show-library",
            "--environment",
            &env,
            "--designation",
            "NOPE"
        ]),
        "NOT_FOUND"
    );

    // A definition built through the environment takes the strongest entry's values and equals
    // the digest that `show-library` predicted.
    let store = f.path("store");
    let created = f.ok(&[
        "define-profile",
        "--store",
        &store,
        "--environment",
        &env,
        "--designation",
        "X-1",
    ]);
    assert_eq!(
        text(at(&created, &["definition"]), "digest"),
        text(at(&shown, &["entry"]), "digest")
    );
    assert_eq!(num(&created, &["definition", "parameters_mm", "h"]), 300.0);
}

#[test]
fn a_user_library_comes_first_and_a_refused_entry_is_not_replaced_by_a_weaker_one() {
    let f = Fixture::new();
    f.write(
        "libs/base.usda",
        &library("base", &[("X-1", 200.0), ("B-ONLY", 160.0)]),
    );
    let env = f.write("env.usda", &environment("\"libs/base.usda\""));
    let user = f.write("user.usda", &library("user", &[("X-1", 320.0)]));

    let shown = f.ok(&[
        "show-library",
        "--environment",
        &env,
        "--user-library",
        &user,
        "--designation",
        "X-1",
    ]);
    assert_eq!(text(at(&shown, &["library"]), "id"), "user");
    assert_eq!(num(&shown, &["entry", "parameters_mm", "h"]), 320.0);
    let listed = f.ok(&[
        "list-library",
        "--environment",
        &env,
        "--user-library",
        &user,
    ]);
    assert_eq!(designations(&listed), ["B-ONLY", "X-1"]);
    let store = f.path("store");
    let created = f.ok(&[
        "define-profile",
        "--store",
        &store,
        "--environment",
        &env,
        "--user-library",
        &user,
        "--designation",
        "X-1",
    ]);
    assert_eq!(num(&created, &["definition", "parameters_mm", "h"]), 320.0);
    // The user library alone, with a single --library, works the same way.
    let alone = f.ok(&[
        "show-library",
        "--library",
        &f.path("libs/base.usda"),
        "--user-library",
        &user,
        "--designation",
        "X-1",
    ]);
    assert_eq!(text(at(&alone, &["library"]), "id"), "user");

    // A stronger entry that is refused answers with its refusal; the weaker valid entry of the
    // same designation is never used instead.
    let bad = "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n    customLayerData = {\n        string \"bim:library:id\" = \"bad\"\n    }\n)\ndef Scope \"Library\"\n{\n    def Scope \"Profiles\"\n    {\n        def Scope \"E\"\n        {\nstring steel:profile:designation = \"X-1\"\ntoken steel:profile:family = \"i_shape\"\ndouble steel:profile:hMm = 300\n        }\n    }\n}\n";
    let broken = f.write("broken.usda", bad);
    assert_eq!(
        f.code(&[
            "show-library",
            "--environment",
            &env,
            "--user-library",
            &broken,
            "--designation",
            "X-1",
        ]),
        "INVALID_ARGUMENT"
    );
    let listed = f.ok(&[
        "list-library",
        "--environment",
        &env,
        "--user-library",
        &broken,
    ]);
    assert_eq!(designations(&listed), ["B-ONLY"]);
    assert_eq!(num(&listed, &["rejected_count"]), 1.0);
    assert_eq!(
        text(
            &at(&listed, &["rejected"]).array().unwrap()[0],
            "library_id"
        ),
        "bad"
    );
}

#[test]
fn hostile_and_inconsistent_environments_are_refused_without_a_crash() {
    let f = Fixture::new();
    f.write("libs/a.usda", &library("a", &[("A-1", 200.0)]));
    f.write("libs/b.usda", &library("a", &[("B-1", 210.0)]));
    let refused = |name: &str, libraries: &str| {
        let env = f.write(name, &environment(libraries));
        f.code(&["list-library", "--environment", &env])
    };
    for escaping in [
        "\"../libs/a.usda\"",
        "\"/etc/passwd\"",
        "\"libs\\\\a.usda\"",
        "\"https://example.invalid/a.usda\"",
        "\"C:/a.usda\"",
        "\"libs//a.usda\"",
        "\"./libs/a.usda\"",
        "\"\"",
    ] {
        assert_eq!(
            refused("env1.usda", escaping),
            "PERMISSION_DENIED",
            "{escaping}"
        );
    }
    assert_eq!(refused("env2.usda", ""), "INVALID_ARGUMENT");
    // A list of numbers, not strings.
    let numbers = f.write("env3.usda", &environment("5").replace("string[]", "int[]"));
    assert_eq!(
        f.code(&["list-library", "--environment", &numbers]),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        refused("env4.usda", "\"libs/a.usda\", \"libs/a.usda\""),
        "INVALID_ARGUMENT"
    );
    assert_eq!(refused("env5.usda", "\"libs/missing.usda\""), "NOT_FOUND");
    // Two libraries with the same bim:library:id.
    assert_eq!(
        refused("env6.usda", "\"libs/a.usda\", \"libs/b.usda\""),
        "INVALID_ARGUMENT"
    );
    let many: Vec<String> = (0..33).map(|n| format!("\"libs/l{n}.usda\"")).collect();
    assert_eq!(refused("env7.usda", &many.join(", ")), "LIMIT_EXCEEDED");
    // No id, garbage, sublayers, other units, a directory, a missing file, a URL.
    let no_id = f.write(
        "env8.usda",
        "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n)\n",
    );
    assert_eq!(
        f.code(&["list-library", "--environment", &no_id]),
        "INVALID_ARGUMENT"
    );
    let garbage = f.write("env9.usda", "not usd");
    assert_eq!(
        f.code(&["list-library", "--environment", &garbage]),
        "INVALID_ARGUMENT"
    );
    let sublayer = f.write(
        "env10.usda",
        &environment("\"libs/a.usda\"").replace(")\n", "    subLayers = [@libs/a.usda@]\n)\n"),
    );
    assert!(f.raw(&["list-library", "--environment", &sublayer]).0 != 0);
    let metres = f.write(
        "env11.usda",
        &environment("\"libs/a.usda\"").replace("0.001", "1"),
    );
    assert_eq!(
        f.code(&["list-library", "--environment", &metres]),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.code(&["list-library", "--environment", &f.path("libs")]),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.code(&["list-library", "--environment", &f.path("nope.usda")]),
        "NOT_FOUND"
    );
    assert_eq!(
        f.code(&[
            "list-library",
            "--environment",
            "https://example.invalid/e.usda"
        ]),
        "PERMISSION_DENIED"
    );
    // A library file named as the environment is not an environment.
    let lib = f.path("libs/a.usda");
    assert_eq!(
        f.code(&["list-library", "--environment", &lib]),
        "INVALID_ARGUMENT"
    );

    // Alternatives and missing inputs.
    let ok_env = f.write("ok.usda", &environment("\"libs/a.usda\""));
    assert_eq!(
        f.code(&["list-library", "--environment", &ok_env, "--library", &lib]),
        "INVALID_ARGUMENT"
    );
    assert_eq!(f.code(&["list-library"]), "INVALID_ARGUMENT");
    assert_eq!(
        f.code(&["list-library", "--user-library", &lib]),
        "INVALID_ARGUMENT"
    );
    // The user library may not repeat an environment library's id.
    let same = f.write("same.usda", &library("a", &[("U-1", 190.0)]));
    assert_eq!(
        f.code(&[
            "list-library",
            "--environment",
            &ok_env,
            "--user-library",
            &same
        ]),
        "INVALID_ARGUMENT"
    );
    // --environment does not combine with explicit values.
    assert_eq!(
        f.code(&[
            "define-profile",
            "--store",
            &f.path("store"),
            "--environment",
            &ok_env,
            "--designation",
            "A-1",
            "--width",
            "100",
        ]),
        "INVALID_ARGUMENT"
    );
}

#[test]
fn library_add_creates_a_library_and_every_kind_of_entry_reads_back() {
    let f = Fixture::new();
    let lib = f.path("user.usda");
    let created = f.ok(&[
        "add-library",
        "--library",
        &lib,
        "--library-id",
        "mine",
        "--title",
        "My sections",
        "--family",
        "i_shape",
        "--h",
        "250",
        "--b",
        "125",
        "--tw",
        "6",
        "--tf",
        "9",
        "--r",
        "12",
        "--designation",
        "MY-I250",
        "--standard",
        "own",
        "--mass-per-meter-kg",
        "30.5",
    ]);
    assert_eq!(text(&created, "outcome"), "library_entry_added");
    assert!(matches!(created.get("created"), Ok(Json::Bool(true))));
    assert_eq!(text(at(&created, &["library"]), "id"), "mine");
    assert_eq!(text(at(&created, &["library"]), "title"), "My sections");
    assert_eq!(num(&created, &["entry_count"]), 1.0);

    let more: &[&[&str]] = &[
        &[
            "--family",
            "lipped_c",
            "--h",
            "150",
            "--b",
            "60",
            "--c",
            "20",
            "--t",
            "2",
            "--r",
            "3",
            "--designation",
            "MY-C150",
        ],
        &[
            "--family",
            "rhs",
            "--h",
            "100",
            "--b",
            "60",
            "--t",
            "4",
            "--r",
            "6",
            "--designation",
            "MY-RHS",
        ],
        &[
            "--family",
            "chs",
            "--d",
            "114",
            "--t",
            "5",
            "--designation",
            "MY-CHS",
        ],
        &[
            "--width",
            "40",
            "--height",
            "200",
            "--designation",
            "MY-BAR",
        ],
        &[
            "--family",
            "custom",
            "--outer",
            "0,0;100,0;100,10;60,10;60,80;40,80;40,10;0,10",
            "--designation",
            "MY \"T\" back\\slash",
        ],
        &[
            "--family",
            "custom",
            "--outer",
            "0,0;100,0;100,60;0,60",
            "--cavity",
            "10,10;10,50;90,50;90,10",
            "--designation",
            "MY-BOX",
        ],
    ];
    for (n, entry) in more.iter().enumerate() {
        let mut args = vec!["add-library", "--library", lib.as_str()];
        args.extend_from_slice(entry);
        let added = f.ok(&args);
        assert!(matches!(added.get("created"), Ok(Json::Bool(false))));
        assert_eq!(num(&added, &["entry_count"]), (n + 2) as f64);
    }

    let listed = f.ok(&["list-library", "--library", &lib]);
    assert_eq!(num(&listed, &["entry_count"]), 7.0);
    assert_eq!(num(&listed, &["rejected_count"]), 0.0);
    let found = designations(&listed);
    assert!(
        found.contains(&"MY \"T\" back\\slash".to_owned()),
        "{found:?}"
    );
    let shown = f.ok(&[
        "show-library",
        "--library",
        &lib,
        "--designation",
        "MY-I250",
    ]);
    assert_eq!(text(at(&shown, &["entry"]), "standard"), "own");
    assert_eq!(num(&shown, &["entry", "mass_per_meter_kg"]), 30.5);
    assert_eq!(num(&shown, &["entry", "parameters_mm", "tf"]), 9.0);
    let boxed = f.ok(&["show-library", "--library", &lib, "--designation", "MY-BOX"]);
    assert_eq!(
        at(&boxed, &["entry", "cavities_mm"]).array().unwrap().len(),
        1
    );
    let bar = f.ok(&["show-library", "--library", &lib, "--designation", "MY-BAR"]);
    assert_eq!(text(at(&bar, &["entry"]), "kind"), "rectangle");

    // The predicted digest is the one of an explicit definition with the same name.
    let store = f.path("store");
    let explicit = f.ok(&[
        "define-profile",
        "--store",
        &store,
        "--family",
        "i_shape",
        "--h",
        "250",
        "--b",
        "125",
        "--tw",
        "6",
        "--tf",
        "9",
        "--r",
        "12",
        "--designation",
        "MY-I250",
        "--name",
        "MY-I250",
        "--standard",
        "own",
        "--mass-per-meter-kg",
        "30.5",
    ]);
    assert_eq!(
        text(at(&explicit, &["definition"]), "digest"),
        text(at(&shown, &["entry"]), "digest")
    );

    // A model places a member of a section that was added, after the library is gone.
    let from_library = f.ok(&[
        "define-profile",
        "--store",
        &store,
        "--library",
        &lib,
        "--designation",
        "MY-C150",
    ]);
    assert_eq!(text(at(&from_library, &["definition"]), "kind"), "lipped_c");
    fs::remove_file(&lib).unwrap();
    let member = f.ok(&[
        "create-member",
        "--store",
        &store,
        "--start",
        "0,0,0",
        "--end",
        "0,0,3000",
        "--profile",
        "MY-C150",
    ]);
    assert_eq!(text(at(&member, &["member"]), "profile_digest").len(), 64);
}

#[test]
fn library_add_refuses_what_it_cannot_write_faithfully_and_leaves_the_file_alone() {
    let f = Fixture::new();
    let lib = f.path("user.usda");
    let i300: &[&str] = &[
        "--family", "i_shape", "--h", "300", "--b", "150", "--tw", "7", "--tf", "10", "--r", "15",
    ];
    let add = |extra: &[&str]| -> Vec<String> {
        let mut args = vec!["add-library".to_owned(), "--library".into(), lib.clone()];
        args.extend(extra.iter().map(|s| (*s).to_owned()));
        args
    };
    let run = |args: Vec<String>| {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        f.raw(&refs)
    };
    let code = |args: Vec<String>| {
        let (status, body) = run(args);
        assert_eq!(status, 2, "{body:?}");
        text(&body, "code")
    };
    fn with<'a>(parts: &[&[&'a str]]) -> Vec<&'a str> {
        parts.concat()
    }

    // A missing file needs an identity; nothing is created otherwise.
    assert_eq!(
        code(add(&with(&[i300, &["--designation", "A-1"]]))),
        "NOT_FOUND"
    );
    assert!(!Path::new(&lib).exists());
    // An invalid section is refused before anything is written.
    assert_eq!(
        code(add(&with(&[
            &[
                "--library-id",
                "mine",
                "--family",
                "i_shape",
                "--h",
                "300",
                "--b",
                "150",
                "--tw",
                "7",
                "--tf",
                "200",
                "--r",
                "15"
            ],
            &["--designation", "BAD"],
        ]))),
        "INVALID_ARGUMENT"
    );
    assert!(!Path::new(&lib).exists(), "a refused add creates no file");
    // No designation, a title without an id, a malformed number.
    assert_eq!(
        code(add(&with(&[&["--library-id", "mine"], i300]))),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        code(add(&with(&[&["--title", "T", "--designation", "A"], i300]))),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        code(add(&with(&[&[
            "--library-id",
            "mine",
            "--designation",
            "A",
            "--width",
            "x",
            "--height",
            "1"
        ]]))),
        "INVALID_ARGUMENT"
    );

    let first = run(add(&with(&[
        &["--library-id", "mine", "--designation", "A-1"],
        i300,
    ])))
    .0;
    assert_eq!(first, 0);
    let before = fs::read(&lib).unwrap();
    // A duplicate designation, an identity for a file that exists, an invalid entry.
    assert_eq!(
        code(add(&with(&[&["--designation", "A-1"], i300]))),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        code(add(&with(&[
            &["--library-id", "other", "--designation", "A-2"],
            i300
        ]))),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        code(add(&with(&[&[
            "--designation",
            "A-3",
            "--family",
            "chs",
            "--d",
            "10",
            "--t",
            "6"
        ],]))),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        fs::read(&lib).unwrap(),
        before,
        "a refused add leaves the file byte for byte"
    );
    // Only the stated library is touched; no temporary file stays behind.
    let leftovers: Vec<_> = fs::read_dir(&f.shared.root)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");

    // A library that holds a refused entry is not rewritten (the entry would be lost).
    let partial = library("p", &[("OK", 300.0)]).replace(
        "    }\n}\n",
        "        def Scope \"Bad\"\n        {\nstring steel:profile:designation = \"BAD\"\ntoken steel:profile:family = \"i_shape\"\n        }\n    }\n}\n",
    );
    let partial_path = f.write("partial.usda", &partial);
    let mut args = vec![
        "add-library",
        "--library",
        partial_path.as_str(),
        "--designation",
        "N-1",
    ];
    args.extend_from_slice(i300);
    assert_eq!(f.code(&args), "INVALID_ARGUMENT");
    assert_eq!(fs::read_to_string(&partial_path).unwrap(), partial);
    // A foreign file is not a library, and a URL is never a path.
    let garbage = f.write("garbage.usda", "not usd");
    let mut args = vec![
        "add-library",
        "--library",
        garbage.as_str(),
        "--designation",
        "N-1",
    ];
    args.extend_from_slice(i300);
    assert_eq!(f.code(&args), "INVALID_ARGUMENT");
    assert_eq!(fs::read_to_string(&garbage).unwrap(), "not usd");
    let mut args = vec![
        "add-library",
        "--library",
        "https://example.invalid/l.usda",
        "--library-id",
        "x",
        "--designation",
        "N-1",
    ];
    args.extend_from_slice(i300);
    assert_eq!(f.code(&args), "PERMISSION_DENIED");
}

#[test]
fn an_added_entry_shows_through_an_environment_that_lists_the_library() {
    let f = Fixture::new();
    let lib = f.path("libs/team.usda");
    let args = [
        "add-library",
        "--library",
        lib.as_str(),
        "--library-id",
        "team",
        "--designation",
        "TEAM-PLATE",
        "--width",
        "20",
        "--height",
        "400",
    ];
    f.ok(&args);
    let env = f.write("env.usda", &environment("\"libs/team.usda\""));
    let listed = f.ok(&["list-library", "--environment", &env]);
    assert_eq!(designations(&listed), ["TEAM-PLATE"]);
    // Adding again through the same file keeps the identity and grows the list.
    f.ok(&[
        "add-library",
        "--library",
        &lib,
        "--designation",
        "TEAM-PLATE-2",
        "--width",
        "25",
        "--height",
        "400",
    ]);
    let listed = f.ok(&["list-library", "--environment", &env]);
    assert_eq!(designations(&listed), ["TEAM-PLATE", "TEAM-PLATE-2"]);
}

#[test]
fn a_user_library_may_not_repeat_the_id_of_a_single_library_either() {
    let f = Fixture::new();
    let base = f.write("base.usda", &library("same", &[("X-1", 200.0)]));
    let user = f.write("user.usda", &library("same", &[("X-1", 320.0)]));
    for command in ["list-library", "show-library"] {
        let mut args = vec![command, "--library", &base, "--user-library", &user];
        if command == "show-library" {
            args.extend(["--designation", "X-1"]);
        }
        assert_eq!(f.code(&args), "INVALID_ARGUMENT", "{command}");
    }
    // The same pair with distinct ids is a lookup.
    let other = f.write("other.usda", &library("other", &[("X-1", 320.0)]));
    let shown = f.ok(&[
        "show-library",
        "--library",
        &base,
        "--user-library",
        &other,
        "--designation",
        "X-1",
    ]);
    assert_eq!(text(at(&shown, &["library"]), "id"), "other");
}

#[cfg(unix)]
#[test]
fn replacing_a_library_keeps_its_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let lib = f.path("private.usda");
    let width = ["--width", "20", "--height", "400"];
    let mut args = vec!["add-library", "--library", &lib, "--library-id", "mine"];
    args.extend(["--designation", "A-1"]);
    args.extend(width);
    f.ok(&args);
    fs::set_permissions(&lib, fs::Permissions::from_mode(0o600)).unwrap();
    let mut args = vec!["add-library", "--library", &lib, "--designation", "A-2"];
    args.extend(width);
    f.ok(&args);
    let mode = fs::metadata(&lib).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "a private library stays private");
    assert_eq!(
        designations(&f.ok(&["list-library", "--library", &lib])),
        ["A-1", "A-2"]
    );
}

#[test]
fn library_add_refuses_binary_usd_and_leaves_the_file_alone() {
    let f = Fixture::new();
    let width = ["--width", "20", "--height", "400"];
    // A .usdc destination is refused whether or not it exists; nothing is created.
    let missing = f.path("new.usdc");
    let mut args = vec!["add-library", "--library", &missing, "--library-id", "x"];
    args.extend(["--designation", "A-1"]);
    args.extend(width);
    assert_eq!(f.code(&args), "INVALID_ARGUMENT");
    assert!(!Path::new(&missing).exists());
    // An existing text library behind a .usdc name is left byte for byte as it was.
    let text_bytes = library("t", &[("X-1", 200.0)]);
    let named = f.write("text.USDC", &text_bytes);
    let mut args = vec!["add-library", "--library", &named, "--designation", "A-1"];
    args.extend(width);
    assert_eq!(f.code(&args), "INVALID_ARGUMENT");
    assert_eq!(fs::read_to_string(&named).unwrap(), text_bytes);
    // Content that only claims to be binary USD is not a library and is not touched either.
    let fake = f.path("fake.usda");
    fs::write(&fake, b"PXR-USDC not really").unwrap();
    let mut args = vec!["add-library", "--library", &fake, "--designation", "A-1"];
    args.extend(width);
    let (code, _) = f.raw(&args);
    assert_eq!(code, 2);
    assert_eq!(fs::read(&fake).unwrap(), b"PXR-USDC not really");
}

#[test]
fn overlapping_large_libraries_page_their_shadow_records() {
    let f = Fixture::new();
    // Four libraries declare the same 1500 long designations: 3 * 1500 = 4500 shadow records of
    // about 280 bytes, more than the 1 MiB a response may carry. Entries and shadow records page
    // independently, and `show-library` reports only the shadows of its designation.
    let designations: Vec<String> = (0..1500)
        .map(|n| format!("{}{n:04}", "D".repeat(190)))
        .collect();
    let entries: Vec<(&str, f64)> = designations.iter().map(|d| (d.as_str(), 200.0)).collect();
    let mut listed = Vec::new();
    for n in 0..4 {
        let name = format!("lib{n}.usda");
        f.write(&name, &library(&format!("lib{n}"), &entries));
        listed.push(format!("\"{name}\""));
    }
    let env = f.write("big.usda", &environment(&listed.join(", ")));

    let page = f.ok(&["list-library", "--environment", &env, "--limit", "1"]);
    assert_eq!(num(&page, &["entry_count"]), 1500.0);
    assert_eq!(num(&page, &["shadowed_count"]), 4500.0);
    assert_eq!(at(&page, &["shadowed"]).array().unwrap().len(), 1);
    assert_eq!(num(&page, &["shadowed_next_offset"]), 1.0);
    let last = f.ok(&[
        "list-library",
        "--environment",
        &env,
        "--limit",
        "5",
        "--shadow-offset",
        "4497",
    ]);
    assert_eq!(at(&last, &["shadowed"]).array().unwrap().len(), 3);
    assert!(matches!(last.get("shadowed_next_offset"), Ok(Json::Null)));
    let shown = f.ok(&[
        "show-library",
        "--environment",
        &env,
        "--designation",
        &designations[7],
    ]);
    let shadows = at(&shown, &["shadowed"]).array().unwrap();
    assert_eq!(shadows.len(), 3);
    assert!(
        shadows
            .iter()
            .all(|s| text(s, "designation") == designations[7])
    );
}

#[test]
fn a_malformed_shadow_offset_is_refused() {
    let f = Fixture::new();
    let lib = f.write("a.usda", &library("a", &[("X-1", 200.0)]));
    assert_eq!(
        f.code(&["list-library", "--library", &lib, "--shadow-offset", "x"]),
        "INVALID_ARGUMENT"
    );
    let listed = f.ok(&["list-library", "--library", &lib, "--shadow-offset", "50"]);
    assert_eq!(num(&listed, &["shadowed_count"]), 0.0);
    assert!(at(&listed, &["shadowed"]).array().unwrap().is_empty());
}
