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

use support::{Scripted, at, text};

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
    fn store(&self) -> String {
        self.shared.root.join("store").to_str().unwrap().to_owned()
    }
    /// One `run-script` invocation over the fixture store: the same commands the suite would
    /// spawn one process each, amortized into a single process (#738 P7; each command's own
    /// work is unchanged). Replies come back in the lines' order.
    fn script(&self, lines: &[Scripted]) -> Vec<Json> {
        let store = self.store();
        support::script_lines(&self.shared.root, &store, lines)
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

/// The `create-member` line over the fixture store, the exact tokens the per-command path
/// spawned.
fn member_line(store: &str, kind: &str, start: &str, end: &str, profile: &str) -> Scripted {
    Scripted::ok(&[
        "create-member",
        "--store",
        store,
        "--kind",
        kind,
        "--start",
        start,
        "--end",
        end,
        "--profile-digest",
        profile,
    ])
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
    // Content-addressed: the same entry defined again is the same definition; the member, the
    // reopen inspection and the definition listing ride one process with it (#738 P7).
    let store = f.store();
    let define = |store: &str| {
        let mut args = vec!["define-profile", "--store", store];
        args.extend_from_slice(I300);
        Scripted::ok(&args)
    };
    let again_and_rest = f.script(&[
        define(&store),
        member_line(&store, "beam", "0,0,0", "6000,0,0", &profile),
        Scripted::ok(&["inspect-elements", "--store", &store]),
        Scripted::ok(&["definitions", "--store", &store]),
    ]);
    let again = &again_and_rest[0];
    assert_eq!(text(again, "outcome"), "definition_exists");
    assert_eq!(text(again.get("definition").unwrap(), "digest"), profile);

    let reply = &again_and_rest[1];
    assert_eq!(text(reply, "outcome"), "member_created");
    let beam = at(reply, &["member"]).clone();
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

    let inspected = &again_and_rest[2];
    assert_eq!(num(inspected, &["member_count"]), 1.0);
    let reopened = &at(inspected, &["members"]).array().unwrap()[0];
    assert_eq!(text(reopened, "mesh_digest"), text(&beam, "mesh_digest"));
    let elements = at(inspected, &["snapshot", "elements"]).array().unwrap();
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

    let listed = &again_and_rest[3];
    let definitions = at(listed, &["definitions"]).array().unwrap();
    assert!(definitions.iter().any(|d| text(d, "kind") == "i_shape"));

    let authored = f.authored_text(reply);
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
    let store = f.store();
    let define = |store: &str, args: &[&str]| {
        let mut line = vec!["define-profile", "--store", store];
        line.extend_from_slice(args);
        Scripted::ok(&line)
    };
    let shapes = f.script(&[
        define(
            &store,
            &[
                "--family", "lipped_c", "--h", "150", "--b", "50", "--c", "15", "--t", "1.5",
                "--r", "2",
            ],
        ),
        define(&store, &["--family", "chs", "--d", "168", "--t", "5"]),
        define(
            &store,
            &[
                "--family",
                "custom",
                "--outer",
                "-50,-50;50,-50;50,50;-50,50",
                "--cavity",
                "-10,-10;-10,10;10,10;10,-10",
            ],
        ),
    ]);
    let digest = |reply: &Json| text(reply.get("definition").unwrap(), "digest");
    let (lipped, tube, custom) = (digest(&shapes[0]), digest(&shapes[1]), digest(&shapes[2]));
    let made = f.script(&[
        member_line(&store, "beam", "0,0,0", "3000,0,0", &lipped),
        member_line(&store, "column", "0,1000,0", "0,1000,3000", &tube),
        member_line(&store, "beam", "0,2000,0", "1000,2000,0", &custom),
        Scripted::ok(&["inspect-elements", "--store", &store]),
    ]);
    for reply in &made[..3] {
        assert_eq!(text(reply, "outcome"), "member_created");
    }
    // A custom outline is exact: (100^2 - 20^2) mm2 along 1000 mm.
    assert_eq!(
        num(at(&made[2], &["member", "quantities"]), &["volume", "mm3"]),
        9_600_000.0
    );
    assert_eq!(num(&made[3], &["member_count"]), 3.0);
}

#[test]
fn bad_section_input_is_refused_with_its_code() {
    let f = Fixture::new();
    let store = f.store();
    let i_refusal = |store: &str, extra: &[&str]| {
        let mut args = vec![
            "define-profile",
            "--store",
            store,
            "--family",
            "i_shape",
            "--h",
            "300",
            "--b",
            "150",
            "--tw",
            "7",
        ];
        args.extend_from_slice(extra);
        Scripted::refused(&args)
    };
    // Missing parameter, flanges that leave no web, a parameter of another family, a section
    // parameter without --family: each refused, none written.
    let refusals = f.script(&[
        i_refusal(&store, &["--r", "15"]),
        i_refusal(&store, &["--tf", "200", "--r", "0"]),
        i_refusal(&store, &["--tf", "10", "--r", "15", "--d", "5"]),
        i_refusal(&store, &["--tf", "10", "--r", "15", "--width", "5"]),
        i_refusal(&store, &["--tf", "nan", "--r", "15"]),
        Scripted::refused(&[
            "define-profile",
            "--store",
            &store,
            "--family",
            "x_shape",
            "--h",
            "1",
        ]),
        // Section parameters without --family are refused, not dropped from a rectangle: the
        // two literal no-family lines cover that guard.
        Scripted::refused(&[
            "define-profile",
            "--store",
            &store,
            "--width",
            "100",
            "--height",
            "100",
            "--t",
            "5",
        ]),
        Scripted::refused(&[
            "define-profile",
            "--store",
            &store,
            "--width",
            "100",
            "--height",
            "100",
            "--outer",
            "0,0;1,0;0,1",
        ]),
        // A clockwise custom outline is stored as authored, and the kernel refuses to build it.
        Scripted::ok(&[
            "define-profile",
            "--store",
            &store,
            "--family",
            "custom",
            "--outer",
            "0,0;0,10;10,10;10,0",
        ]),
    ]);
    for (index, reply) in refusals[..8].iter().enumerate() {
        // The NaN parameter is named as such; every other case is a typed refusal.
        let expected = if index == 4 {
            "NON_FINITE_NUMBER"
        } else {
            "INVALID_ARGUMENT"
        };
        assert_eq!(text(reply, "code"), expected);
    }
    let clockwise = text(refusals[8].get("definition").unwrap(), "digest");
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
    let store = f.store();
    let mut define = vec!["define-profile", "--store", store.as_str()];
    define.extend_from_slice(I300);
    let script = f.script(&[Scripted::ok(&define)]);
    let profile = text(script[0].get("definition").unwrap(), "digest");
    let mut lines = Vec::new();
    for n in 0..8 {
        let y = format!("{}", n * 1000);
        lines.push(member_line(
            &store,
            "beam",
            &format!("0,{y},0"),
            &format!("6000,{y},0"),
            &profile,
        ));
    }
    lines.push(Scripted::ok(&["inspect-elements", "--store", &store]));
    let made = f.script(&lines);
    for reply in &made[..8] {
        assert_eq!(text(reply, "outcome"), "member_created");
    }
    assert_eq!(num(&made[8], &["member_count"]), 8.0);
}

#[test]
fn a_chs_member_hole_has_a_gross_that_ignores_it_and_certifies_the_drop() {
    // The tube-hole constant: the holed net used to be the faceted mesh volume while the gross
    // stayed the analytic `area x length`, so every CHS 114x5 hole overstated the drop by the
    // arc approximation times the length (~10 980 mm3), far past the engine's `2 * A * t`
    // bound. The measured rule pins the d = 6/10/20 triplet end to end through the binary.
    let f = Fixture::new();
    let store = f.store();
    let chs = f.script(&[Scripted::ok(&[
        "define-profile",
        "--store",
        &store,
        "--family",
        "chs",
        "--d",
        "114",
        "--t",
        "5",
        "--designation",
        "EX-CHS114",
    ])]);
    let profile = text(chs[0].get("definition").unwrap(), "digest");
    let made = f.script(&[
        member_line(&store, "beam", "0,0,0", "1000,0,0", &profile),
        member_line(&store, "beam", "0,0,0", "1000,0,0", &profile),
        member_line(&store, "beam", "0,0,0", "1000,0,0", &profile),
    ]);
    let mut holes = Vec::new();
    let mut grosses = Vec::new();
    for (index, reply) in made.iter().enumerate() {
        assert_eq!(text(reply, "outcome"), "member_created");
        let member = at(reply, &["member"]);
        let id = text(member, "element_id");
        grosses.push(num(member, &["quantities", "volume", "mm3"]));
        let op = format!("019a7bf0-0000-7000-8000-00000000aa3{index}");
        holes.push(Scripted::ok(&[
            "add-hole",
            "--store",
            &store,
            "--id",
            &id,
            "--operation-id",
            &op,
            "--axis-point",
            "0,0,500",
            "--axis-direction",
            "1,0,0",
            "--diameter",
            &format!("{}", [6.0, 10.0, 20.0][index]),
        ]));
    }
    let holed = f.script(&holes);
    for (index, reply) in holed.iter().enumerate() {
        let diameter = [6.0, 10.0, 20.0][index];
        let gross = grosses[index];
        let q = at(reply, &["element", "quantities"]);
        // The gross ignores the hole (the plate rule); the net and the removed volume carry
        // the drop, and it certifies within the engine bound of the smooth tube.
        let after_gross = num(q, &["volume_gross_mm3"]);
        assert!(
            (after_gross - gross).abs() <= 1e-9 * gross,
            "d={diameter}: gross {after_gross} was {gross}"
        );
        let net = num(q, &["volume_net_mm3"]);
        let removed = num(q, &["removed_volume_mm3"]);
        let bound = 2.0 * std::f64::consts::PI * diameter * diameter / 4.0 * 5.0;
        let drop = gross - net;
        assert!(
            drop > 0.0 && drop <= bound,
            "d={diameter}: drop {drop} past bound {bound}"
        );
        assert!(
            (removed - drop).abs() <= 1e-6 * drop,
            "d={diameter}: removed {removed} vs drop {drop}"
        );
    }
}
