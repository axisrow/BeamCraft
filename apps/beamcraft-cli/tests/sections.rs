// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic sections that follow no national standard, MPL-2.0. Expected
// areas are closed forms (rectangles and quarter circles); the saved layer text is read with plain
// string matching and no BeamCraft crate.

//! A07.1 end to end through the real `beamcraft` binary: `define-profile --family` defines
//! section profiles of the parametric families and custom outlines, `create-member` builds members
//! of them, and fresh processes reopen the revision with `inspect-elements` and `definitions`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support;

use support::{at, text};

use beamcraft_json::Json;
use std::fs;

struct Fixture {
    shared: support::StoreFixture,
}
impl Fixture {
    fn new() -> Self {
        Self {
            shared: support::StoreFixture::new("a07-tests"),
        }
    }
    fn run(&self, command: &str, extra: &[&str]) -> (i32, Json) {
        self.shared.run(command, extra)
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

fn defined(f: &Fixture, args: &[&str]) -> String {
    let reply = f.ok("define-profile", args);
    text(reply.get("definition").unwrap(), "digest")
}

fn member(f: &Fixture, kind: &str, start: &str, end: &str, profile: &str) -> Json {
    let created = created(f, kind, start, end, profile);
    at(&created, &["member"]).clone()
}

fn created(f: &Fixture, kind: &str, start: &str, end: &str, profile: &str) -> Json {
    let created = f.ok(
        "create-member",
        &[
            "--kind",
            kind,
            "--start",
            start,
            "--end",
            end,
            "--profile-digest",
            profile,
        ],
    );
    assert_eq!(text(&created, "outcome"), "member_created");
    created
}

const I300: &[&str] = &[
    "--family",
    "i_shape",
    "--h",
    "300",
    "--b",
    "150",
    "--tw",
    "7",
    "--tf",
    "10",
    "--r",
    "15",
    "--designation",
    "EX-I300",
];

#[test]
fn an_i_beam_from_a_family_definition_is_built_and_reopened() {
    let f = Fixture::new();
    let reply = f.ok("define-profile", I300);
    assert_eq!(text(&reply, "outcome"), "definition_created");
    let definition = reply.get("definition").unwrap();
    assert_eq!(text(definition, "kind"), "i_shape");
    assert_eq!(num(definition, &["parameters_mm", "tw"]), 7.0);
    assert_eq!(num(definition, &["width_mm"]), 150.0);
    assert_eq!(text(definition, "designation"), "EX-I300");
    let profile = text(definition, "digest");
    // Content-addressed: the same entry defined again is the same definition.
    let again = f.ok("define-profile", I300);
    assert_eq!(text(&again, "outcome"), "definition_exists");
    assert_eq!(text(again.get("definition").unwrap(), "digest"), profile);

    let reply = created(&f, "beam", "0,0,0", "6000,0,0", &profile);
    let beam = at(&reply, &["member"]).clone();
    assert_eq!(num(&beam, &["section_mm", "width"]), 150.0);
    assert_eq!(num(&beam, &["section_mm", "height"]), 300.0);
    // Sharp I plus four root fillets (1 - pi/4) r^2: the exact area, not the arc polygon.
    let area = 2.0 * 150.0 * 10.0 + 280.0 * 7.0 + 4.0 * (1.0 - std::f64::consts::FRAC_PI_4) * 225.0;
    let volume = num(&beam, &["quantities", "volume", "mm3"]);
    assert!((volume / 6000.0 - area).abs() <= 1e-9 * area, "{volume}");
    let quantities = at(&beam, &["quantities"]);
    assert_eq!(text(quantities, "area_source"), "computed");
    assert!((num(quantities, &["section_area_mm2"]) - area).abs() <= 1e-9 * area);
    assert_eq!(num(quantities, &["volume_gross_mm3"]), volume);
    assert_eq!(num(quantities, &["volume_net_mm3"]), volume);
    assert_eq!(num(quantities, &["removed_volume_mm3"]), 0.0);
    assert_eq!(
        text(at(&beam, &["quantities", "volume"]), "algorithm"),
        "pmass-member-area-x-length/1;section-exact-area;tolerance-policy-3"
    );

    let inspected = f.ok("inspect-elements", &[]);
    assert_eq!(num(&inspected, &["member_count"]), 1.0);
    let reopened = &at(&inspected, &["members"]).array().unwrap()[0];
    assert_eq!(text(reopened, "mesh_digest"), text(&beam, "mesh_digest"));
    let elements = at(&inspected, &["snapshot", "elements"]).array().unwrap();
    let bound = |key: &str| -> Vec<f64> {
        at(&elements[0], &[key])
            .array()
            .unwrap()
            .iter()
            .map(|v| v.float().unwrap())
            .collect()
    };
    // The flange width spans Y and the height Z, as for the rectangle.
    assert_eq!(
        (bound("world_min_mm"), bound("world_max_mm")),
        (vec![0.0, -75.0, -150.0], vec![6000.0, 75.0, 150.0])
    );

    let listed = f.ok("definitions", &[]);
    let definitions = at(&listed, &["definitions"]).array().unwrap();
    assert!(definitions.iter().any(|d| text(d, "kind") == "i_shape"));

    let authored = f.authored_text(&reply);
    for line in [
        "custom token bim:definitionKind = \"i_shape\"",
        "custom double steel:profile:rMm = 15.0",
        "custom string steel:profile:designation = \"EX-I300\"",
    ] {
        assert!(authored.contains(line), "{line}");
    }
}

#[test]
fn cold_formed_and_custom_members_share_one_revision() {
    let f = Fixture::new();
    let lipped = defined(
        &f,
        &[
            "--family", "lipped_c", "--h", "150", "--b", "50", "--c", "15", "--t", "1.5", "--r",
            "2",
        ],
    );
    let tube = defined(&f, &["--family", "chs", "--d", "168", "--t", "5"]);
    let custom = defined(
        &f,
        &[
            "--family",
            "custom",
            "--outer",
            "-50,-50;50,-50;50,50;-50,50",
            "--cavity",
            "-10,-10;-10,10;10,10;10,-10",
        ],
    );
    member(&f, "beam", "0,0,0", "3000,0,0", &lipped);
    member(&f, "column", "0,1000,0", "0,1000,3000", &tube);
    let block = member(&f, "beam", "0,2000,0", "1000,2000,0", &custom);
    // A custom outline is exact: (100^2 - 20^2) mm2 along 1000 mm.
    assert_eq!(num(&block, &["quantities", "volume", "mm3"]), 9_600_000.0);
    let inspected = f.ok("inspect-elements", &[]);
    assert_eq!(num(&inspected, &["member_count"]), 3.0);
}

#[test]
fn bad_section_input_is_refused_with_its_code() {
    let f = Fixture::new();
    let i = |extra: &[&str]| {
        let mut args = vec![
            "--family", "i_shape", "--h", "300", "--b", "150", "--tw", "7",
        ];
        args.extend_from_slice(extra);
        f.rejected("define-profile", &args)
    };
    // Missing parameter, flanges that leave no web, a parameter of another family.
    assert_eq!(i(&["--r", "15"]), "INVALID_ARGUMENT");
    assert_eq!(i(&["--tf", "200", "--r", "0"]), "INVALID_ARGUMENT");
    assert_eq!(
        i(&["--tf", "10", "--r", "15", "--d", "5"]),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        i(&["--tf", "10", "--r", "15", "--width", "5"]),
        "INVALID_ARGUMENT"
    );
    assert_eq!(i(&["--tf", "nan", "--r", "15"]), "NON_FINITE_NUMBER");
    assert_eq!(
        f.rejected("define-profile", &["--family", "x_shape", "--h", "1"]),
        "INVALID_ARGUMENT"
    );
    // Section parameters without --family are refused, not dropped from a rectangle.
    for extra in [&["--t", "5"][..], &["--outer", "0,0;1,0;0,1"]] {
        let mut args = vec!["--width", "100", "--height", "100"];
        args.extend_from_slice(extra);
        assert_eq!(f.rejected("define-profile", &args), "INVALID_ARGUMENT");
    }
    // A clockwise custom outline is stored as authored, and the kernel refuses to build it.
    let clockwise = defined(
        &f,
        &["--family", "custom", "--outer", "0,0;0,10;10,10;10,0"],
    );
    assert_eq!(
        f.rejected(
            "create-member",
            &[
                "--start",
                "0,0,0",
                "--end",
                "1000,0,0",
                "--profile-digest",
                &clockwise
            ]
        ),
        "INVALID_ARGUMENT"
    );
}

#[test]
fn more_detailed_members_fit_one_revision_than_the_first_slice_allowed() {
    // The U01 first-slice layer ceilings stopped at 7 such I-sections; the model profile
    // keeps going. Eight prove it; every new member re-verifies the whole revision, so a
    // longer run adds quadratic kernel work without a new property.
    let f = Fixture::new();
    let profile = defined(&f, I300);
    for n in 0..8 {
        let y = format!("{}", n * 1000);
        member(
            &f,
            "beam",
            &format!("0,{y},0"),
            &format!("6000,{y},0"),
            &profile,
        );
    }
    let inspected = f.ok("inspect-elements", &[]);
    assert_eq!(num(&inspected, &["member_count"]), 8.0);
}
