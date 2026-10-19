// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic libraries and sections that follow no national standard,
// MPL-2.0. The shipped library is `assets/libraries/synthetic-sections.usda`; the hostile and
// partial libraries are written by the tests from plain text. Expected masses are the closed form
// volume * density; the saved layer text is read with plain string matching.

//! A07.2 end to end through the real `beamcraft` binary: a USD profile library is listed and
//! resolved by designation into a self-contained definition, definitions carry a readable name
//! that is part of their digest, and members and plates select definitions by name.

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

const SHIPPED: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/libraries/synthetic-sections.usda"
);

struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("a07-2-tests");
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
        let body = json::decode(&output.stdout, &message::read_limits()).unwrap();
        (output.status.code().unwrap(), body)
    }
    fn run(&self, command: &str, extra: &[&str]) -> (i32, Json) {
        let store = self.store();
        let mut args = vec![command, "--store", store.as_str()];
        args.extend_from_slice(extra);
        self.raw(&args)
    }
    fn ok(&self, command: &str, extra: &[&str]) -> Json {
        let (code, body) = self.run(command, extra);
        assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
        body
    }
    fn rejected(&self, command: &str, extra: &[&str]) -> Json {
        let (code, body) = self.run(command, extra);
        assert_eq!(code, 2, "{command} {extra:?}: {body:?}");
        assert_eq!(text(&body, "outcome"), "rejected");
        body
    }
    fn library_rejected(&self, command: &str, extra: &[&str]) -> String {
        let mut args = vec![command];
        args.extend_from_slice(extra);
        let (code, body) = self.raw(&args);
        assert_eq!(code, 2, "{command} {extra:?}: {body:?}");
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
fn digest_of(reply: &Json) -> String {
    text(at(reply, &["definition"]), "digest")
}
fn sequence(json: &Json) -> u64 {
    text(at(json, &["revision"]), "sequence").parse().unwrap()
}

const HEADER: &str = "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n    customLayerData = {\n        string \"bim:library:id\" = \"t\"\n    }\n)\n";
const I_BODY: &str = "token steel:profile:family = \"i_shape\"\ndouble steel:profile:hMm = 300\ndouble steel:profile:bMm = 150\ndouble steel:profile:twMm = 7\ndouble steel:profile:tfMm = 10\ndouble steel:profile:rMm = 15\n";

fn library_text(entries: &[(&str, &str, &str)]) -> String {
    let mut text =
        format!("{HEADER}def Scope \"Library\"\n{{\n    def Scope \"Profiles\"\n    {{\n");
    for (prim, designation, body) in entries {
        text.push_str(&format!(
            "        def Scope \"{prim}\"\n        {{\nstring steel:profile:designation = \"{designation}\"\n{body}        }}\n"
        ));
    }
    text.push_str("    }\n}\n");
    text
}

#[test]
fn a_library_is_listed_and_an_entry_becomes_a_self_contained_definition() {
    let f = Fixture::new();
    let listed = f.raw(&["library-list", "--library", SHIPPED]);
    assert_eq!(listed.0, 0, "{:?}", listed.1);
    let listed = listed.1;
    assert_eq!(text(&listed, "outcome"), "library");
    assert_eq!(text(at(&listed, &["library"]), "id"), "beamcraft-synthetic");
    assert_eq!(text(at(&listed, &["library"]), "licence"), "MPL-2.0");
    assert_eq!(num(&listed, &["entry_count"]), 9.0);
    assert!(at(&listed, &["rejected"]).array().unwrap().is_empty());
    let channels = f.raw(&["library-list", "--library", SHIPPED, "--family", "channel"]);
    assert_eq!(num(&channels.1, &["entry_count"]), 1.0);
    let found = f.raw(&["library-list", "--library", SHIPPED, "--filter", "i-300"]);
    assert_eq!(num(&found.1, &["entry_count"]), 1.0);
    let shown = f.raw(&[
        "library-show",
        "--library",
        SHIPPED,
        "--designation",
        "SYN-I-300x150",
    ]);
    assert_eq!(shown.0, 0);
    let predicted = text(at(&shown.1, &["entry"]), "digest");

    // Work from a private copy of the library and delete it: the model must not need it.
    let copy = f.file("work.usda", &fs::read(SHIPPED).unwrap());
    let created = f.ok(
        "define-profile",
        &["--library", &copy, "--designation", "SYN-I-300x150"],
    );
    assert_eq!(text(&created, "outcome"), "definition_created");
    let digest = digest_of(&created);
    assert_eq!(digest, predicted, "library-show predicts the pinned digest");
    let definition = at(&created, &["definition"]);
    assert_eq!(text(definition, "kind"), "i_shape");
    assert_eq!(text(definition, "display_name"), "SYN-I-300x150");
    assert_eq!(text(definition, "designation"), "SYN-I-300x150");
    assert_eq!(num(definition, &["parameters_mm", "tw"]), 7.0);
    assert_eq!(num(definition, &["area_mm2"]), 5153.0);
    assert_eq!(num(definition, &["mass_per_meter_kg"]), 40.45);
    fs::remove_file(&copy).unwrap();
    let authored = f.authored_text(&created);
    assert!(authored.contains("displayName = \"SYN-I-300x150\""));
    assert!(authored.contains("custom double steel:profile:twMm = 7.0"));

    // The same entry again is the same definition: nothing is written.
    let again = f.raw(&[
        "define-profile",
        "--store",
        &f.store(),
        "--library",
        SHIPPED,
        "--designation",
        "SYN-I-300x150",
    ]);
    assert_eq!(text(&again.1, "outcome"), "definition_exists");
    assert_eq!(sequence(&again.1), 1);

    // A fresh process places a member by name with the library gone.
    let member = f.ok(
        "create-member",
        &[
            "--start",
            "0,0,0",
            "--end",
            "0,0,3000",
            "--profile",
            "SYN-I-300x150",
        ],
    );
    assert_eq!(text(at(&member, &["member"]), "profile_digest"), digest);
    assert_eq!(num(&member, &["member", "section_mm", "width"]), 150.0);
    let inspected = f.ok("plate-inspect", &[]);
    let listed_member = at(&inspected, &["members"]).array().unwrap()[0].clone();
    assert_eq!(text(&listed_member, "profile_name"), "SYN-I-300x150");
    let definitions = f.ok("definitions", &[]);
    assert_eq!(num(&definitions, &["definition_count"]), 1.0);
}

#[test]
fn every_family_of_the_shipped_library_builds_a_member() {
    let f = Fixture::new();
    let listed = f.raw(&["library-list", "--library", SHIPPED]).1;
    for entry in at(&listed, &["entries"]).array().unwrap() {
        let designation = text(entry, "designation");
        let created = f.ok(
            "define-profile",
            &["--library", SHIPPED, "--designation", &designation],
        );
        assert_eq!(digest_of(&created), text(entry, "digest"), "{designation}");
        let member = f.ok(
            "create-member",
            &[
                "--start",
                "0,0,0",
                "--end",
                "1000,0,0",
                "--profile",
                &designation,
            ],
        );
        assert_eq!(
            text(at(&member, &["member"]), "profile_digest"),
            text(entry, "digest")
        );
    }
}

#[test]
fn an_unknown_designation_is_not_found_and_never_substituted() {
    let f = Fixture::new();
    f.ok(
        "define-profile",
        &["--library", SHIPPED, "--designation", "SYN-I-200x100"],
    );
    for wrong in ["SYN-I-300", "syn-i-200x100", "SYN-I-200x100 ", "", "NOPE"] {
        let body = f.rejected(
            "define-profile",
            &["--library", SHIPPED, "--designation", wrong],
        );
        assert_eq!(text(&body, "code"), "NOT_FOUND", "{wrong:?}");
        let shown = f.library_rejected(
            "library-show",
            &["--library", SHIPPED, "--designation", wrong],
        );
        assert_eq!(shown, "NOT_FOUND", "{wrong:?}");
    }
    // Nothing was published by the refusals: the next revision is the second one.
    let next = f.ok("define-material", &["--density", "7850"]);
    assert_eq!(sequence(&next), 2);
    // A name that no definition has is not found either.
    let member = f.rejected(
        "create-member",
        &[
            "--start",
            "0,0,0",
            "--end",
            "0,0,1000",
            "--profile",
            "SYN-I-300x150",
        ],
    );
    assert_eq!(text(&member, "code"), "NOT_FOUND");
}

#[test]
fn hostile_and_damaged_libraries_are_refused_without_a_crash() {
    let f = Fixture::new();
    let duplicate = f.file(
        "dup.usda",
        library_text(&[("A", "T-1", I_BODY), ("B", "T-1", I_BODY)]).as_bytes(),
    );
    assert_eq!(
        f.library_rejected("library-list", &["--library", &duplicate]),
        "INVALID_ARGUMENT"
    );
    let body = f.rejected(
        "define-profile",
        &["--library", &duplicate, "--designation", "T-1"],
    );
    assert_eq!(text(&body, "code"), "INVALID_ARGUMENT");

    let huge = f.file("huge.usda", &vec![b'a'; 8 * 1024 * 1024 + 1]);
    assert_eq!(
        f.library_rejected("library-list", &["--library", &huge]),
        "LIMIT_EXCEEDED"
    );
    let garbage = f.file("garbage.usda", b"this is not a usd layer \x00\xff\xfe");
    assert_eq!(
        f.library_rejected("library-list", &["--library", &garbage]),
        "INVALID_ARGUMENT"
    );
    // A binary layer the static read profile cannot decode is an unsupported encoding.
    let binary = f.file(
        "fake.usda",
        b"PXR-USDC\x00\x01\x02\x03 truncated crate file",
    );
    assert_eq!(
        f.library_rejected("library-list", &["--library", &binary]),
        "UNSUPPORTED_PROPERTY"
    );
    // A sublayer would name another file: it is refused whatever the spelling, never opened.
    for (name, target) in [("sub1.usda", "other.usda"), ("sub2.usda", "./other.usda")] {
        let layered = f.file(
            name,
            HEADER
                .replace(")\n", &format!("    subLayers = [@{target}@]\n)\n"))
                .as_bytes(),
        );
        let code = f.library_rejected("library-list", &["--library", &layered]);
        assert!(
            code == "INVALID_ARGUMENT" || code == "PERMISSION_DENIED",
            "{target}: {code}"
        );
    }
    let metres = f.file("metres.usda", HEADER.replace("0.001", "1").as_bytes());
    assert_eq!(
        f.library_rejected("library-list", &["--library", &metres]),
        "INVALID_ARGUMENT"
    );
    let anonymous = f.file(
        "noid.usda",
        b"#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n)\n",
    );
    assert_eq!(
        f.library_rejected("library-list", &["--library", &anonymous]),
        "INVALID_ARGUMENT"
    );
    let directory = f.root.to_str().unwrap().to_owned();
    assert_eq!(
        f.library_rejected("library-list", &["--library", &directory]),
        "INVALID_ARGUMENT"
    );
    let missing = f.root.join("missing.usda").to_str().unwrap().to_owned();
    assert_eq!(
        f.library_rejected("library-list", &["--library", &missing]),
        "NOT_FOUND"
    );
    for url in [
        "https://example.invalid/lib.usda",
        "\\\\server\\share\\lib.usda",
    ] {
        assert_eq!(
            f.library_rejected("library-list", &["--library", url]),
            "PERMISSION_DENIED",
            "{url}"
        );
    }
    // None of the refusals touched the store.
    assert!(fs::read_dir(f.root.join("store")).unwrap().next().is_none());
}

#[test]
fn a_bad_entry_is_reported_and_the_other_entries_still_load() {
    let f = Fixture::new();
    // A catalogue area that disagrees with the outline is a warning, not a bad entry.
    let wrong_area = format!("{I_BODY}double steel:profile:areaMm2 = 5000\n");
    let incomplete = "token steel:profile:family = \"i_shape\"\ndouble steel:profile:hMm = 300\n";
    let nan = format!("{I_BODY}double steel:profile:massPerMeterKg = -1\n");
    let unknown_family = "token steel:profile:family = \"banana\"\n";
    let library = f.file(
        "mixed.usda",
        library_text(&[
            ("GOOD", "T-GOOD", I_BODY),
            ("AREA", "T-AREA", &wrong_area),
            ("PART", "T-PART", incomplete),
            ("MASS", "T-MASS", &nan),
            ("FAM", "T-FAM", unknown_family),
        ])
        .as_bytes(),
    );
    let listed = f.raw(&["library-list", "--library", &library]);
    assert_eq!(listed.0, 0, "{:?}", listed.1);
    assert_eq!(num(&listed.1, &["entry_count"]), 2.0);
    let rejected = at(&listed.1, &["rejected"]).array().unwrap();
    assert_eq!(rejected.len(), 3);
    for entry in rejected {
        assert_eq!(text(entry, "code"), "INVALID_ARGUMENT");
        assert!(text(entry, "prim_path").starts_with("/Library/Profiles/"));
        assert!(text(entry, "designation").starts_with("T-"));
    }
    // The warning is reported in the listing, and only for the entry that disagrees.
    let entries = at(&listed.1, &["entries"]).array().unwrap();
    let warning_of = |designation: &str| {
        entries
            .iter()
            .find(|e| text(e, "designation") == designation)
            .unwrap()
            .get("declared_area_warning")
            .unwrap()
            .clone()
    };
    assert_eq!(warning_of("T-GOOD"), Json::Null);
    let warning = warning_of("T-AREA");
    assert_eq!(num(&warning, &["declared_mm2"]), 5000.0);
    assert!(num(&warning, &["kernel_mm2"]) > 5100.0);
    assert!(num(&warning, &["relative_difference"]) > 0.01);
    assert_eq!(num(&warning, &["threshold_percent"]), 1.0);
    f.ok(
        "define-profile",
        &["--library", &library, "--designation", "T-GOOD"],
    );
    let created = f.ok(
        "define-profile",
        &["--library", &library, "--designation", "T-AREA"],
    );
    assert_eq!(text(&created, "outcome"), "definition_created");
    assert_eq!(num(&created, &["definition", "area_mm2"]), 5000.0);
    assert_eq!(
        num(
            &created,
            &["definition", "declared_area_warning", "declared_mm2"]
        ),
        5000.0
    );
    for bad in ["T-PART", "T-MASS", "T-FAM"] {
        let body = f.rejected(
            "define-profile",
            &["--library", &library, "--designation", bad],
        );
        assert_eq!(text(&body, "code"), "INVALID_ARGUMENT", "{bad}");
    }
    assert_eq!(
        sequence(&f.ok("define-material", &["--density", "1"])),
        3,
        "the refused entries published nothing"
    );
}

#[test]
fn a_declared_section_area_that_disagrees_with_the_outline_warns_and_malformed_ones_are_refused() {
    let f = Fixture::new();
    let tube = |area: &'static str| {
        vec![
            "--family",
            "chs",
            "--d",
            "114",
            "--t",
            "5",
            "--designation",
            "TUBE",
            "--area-mm2",
            area,
        ]
    };
    // A ring of 114 x 5 mm has about 1,712 mm2.
    let agrees = f.ok("define-profile", &tube("1712"));
    assert_eq!(
        at(&agrees, &["definition"])
            .get("declared_area_warning")
            .unwrap(),
        &Json::Null
    );
    // A catalogue value far from the outline is kept as declared and reported.
    let differs = f.ok("define-profile", &tube("2000"));
    assert_eq!(text(&differs, "outcome"), "definition_created");
    let definition = at(&differs, &["definition"]);
    assert_eq!(num(definition, &["area_mm2"]), 2000.0);
    let warning = at(definition, &["declared_area_warning"]);
    assert_eq!(num(warning, &["declared_mm2"]), 2000.0);
    // The reference is the exact ring pi * t * (d - t), not the polygon of the mesh.
    let ring = std::f64::consts::PI * 5.0 * 109.0;
    assert!((num(warning, &["kernel_mm2"]) - ring).abs() <= 1e-9 * ring);
    assert_eq!(num(warning, &["exact_mm2"]), num(warning, &["kernel_mm2"]));
    assert!((num(warning, &["relative_difference"]) - 0.168).abs() < 0.01);
    assert_eq!(num(warning, &["threshold"]), 0.01);
    assert_eq!(num(warning, &["threshold_percent"]), 1.0);
    // The same warning is listed from the stored definition in a fresh process.
    let listed = f.ok("definitions", &["--name", "TUBE"]);
    let warnings = at(&listed, &["definitions"])
        .array()
        .unwrap()
        .iter()
        .filter(|d| d.get("declared_area_warning").unwrap() != &Json::Null)
        .count();
    assert_eq!(warnings, 1);
    // The warned definition is a normal definition: a member can pin it by digest.
    let member = f.ok(
        "create-member",
        &[
            "--start",
            "0,0,0",
            "--end",
            "0,0,1000",
            "--profile-digest",
            &digest_of(&differs),
        ],
    );
    assert_eq!(text(&member, "outcome"), "member_created");
    // Malformed values are still refused.
    for bad in ["0", "-5", "NaN", "inf", "2e12"] {
        let (code, _) = f.run("define-profile", &tube(bad));
        assert_eq!(code, 2, "{bad}");
    }
    // The rectangle keeps its refusing 0.1 percent check (P02.2).
    let (code, body) = f.run(
        "define-profile",
        &["--width", "200", "--height", "300", "--area-mm2", "70000"],
    );
    assert_eq!(code, 2, "{body:?}");
    assert_eq!(text(&body, "code"), "INVALID_ARGUMENT");
}

#[test]
fn library_options_do_not_combine_with_explicit_values() {
    let f = Fixture::new();
    for extra in [
        vec!["--width", "10"],
        vec!["--family", "i_shape"],
        vec!["--h", "10"],
        vec!["--area-mm2", "10"],
        vec!["--standard", "X"],
    ] {
        let mut args = vec!["--library", SHIPPED, "--designation", "SYN-I-200x100"];
        args.extend(extra.iter());
        assert_eq!(
            text(&f.rejected("define-profile", &args), "code"),
            "INVALID_ARGUMENT",
            "{extra:?}"
        );
    }
    assert_eq!(
        text(
            &f.rejected("define-profile", &["--library", SHIPPED]),
            "code"
        ),
        "INVALID_ARGUMENT"
    );
}

#[test]
fn a_large_library_is_listed_in_bounded_pages_that_add_up_to_the_whole() {
    let f = Fixture::new();
    let bar = "token steel:profile:family = \"rectangle\"\ndouble steel:profile:hMm = 10\ndouble steel:profile:bMm = 10\n";
    // Half valid entries, half refused ones, 1024 in all. The 4096-entry admission bound is
    // proven by `the_entry_count_of_a_library_is_bounded`; paging stays well under it, because
    // every page is a fresh process that recomposes the whole layer.
    let designations: Vec<String> = (0..1024).map(|i| format!("T-{i:04}")).collect();
    let prims: Vec<String> = (0..1024).map(|i| format!("E{i}")).collect();
    let entries: Vec<(&str, &str, &str)> = prims
        .iter()
        .zip(&designations)
        .enumerate()
        .map(|(i, (p, d))| (p.as_str(), d.as_str(), if i % 2 == 0 { bar } else { "" }))
        .collect();
    let path = f.file("large.usda", library_text(&entries).as_bytes());
    let mut seen = std::collections::BTreeSet::new();
    let mut rejected = std::collections::BTreeSet::new();
    let mut offset = 0u64;
    let mut pages = 0;
    for limit in ["256", "64"] {
        seen.clear();
        rejected.clear();
        offset = 0;
        pages = 0;
        loop {
            let (code, body) = f.raw(&[
                "library-list",
                "--library",
                &path,
                "--offset",
                &offset.to_string(),
                "--limit",
                limit,
            ]);
            assert_eq!(code, 0, "offset {offset}: {body:?}");
            pages += 1;
            assert_eq!(num(&body, &["entry_count"]), 512.0);
            assert_eq!(num(&body, &["rejected_count"]), 512.0);
            let page = at(&body, &["entries"]).array().unwrap();
            let refused = at(&body, &["rejected"]).array().unwrap();
            assert_eq!(
                (page.len() + refused.len()) as f64,
                num(&body, &["page_size"])
            );
            assert!(page.len() + refused.len() <= limit.parse::<usize>().unwrap());
            for entry in page {
                assert!(seen.insert(text(entry, "designation")), "repeated entry");
            }
            for entry in refused {
                assert!(rejected.insert(text(entry, "designation")), "repeated");
            }
            match at(&body, &["next_offset"]) {
                Json::Null => break,
                next => offset = next.float().unwrap() as u64,
            }
        }
        assert_eq!(seen.len(), 512, "limit {limit}");
        assert_eq!(rejected.len(), 512, "limit {limit}");
    }
    assert!(pages >= 16 && offset > 0);
    // The default page is bounded too, and the limits are checked.
    let (code, body) = f.raw(&["library-list", "--library", &path]);
    assert_eq!(code, 0);
    assert_eq!(num(&body, &["page_size"]), 64.0);
    assert_eq!(num(&body, &["next_offset"]), 64.0);
    for limit in ["0", "257", "x"] {
        assert_eq!(
            f.library_rejected("library-list", &["--library", &path, "--limit", limit]),
            "INVALID_ARGUMENT"
        );
    }
}

#[test]
fn a_retry_by_name_replays_even_after_a_second_definition_has_the_name() {
    let f = Fixture::new();
    let steel = digest_of(&f.ok("define-material", &["--density", "7850", "--grade", "S355"]));
    let profile = digest_of(&f.ok(
        "define-profile",
        &["--width", "200", "--height", "300", "--designation", "P1"],
    ));
    let (plate_op, plate_id) = (
        "01900000-0000-7000-8000-00000000a001",
        "01900000-0000-7000-8000-00000000a002",
    );
    let (member_op, member_id) = (
        "01900000-0000-7000-8000-00000000a003",
        "01900000-0000-7000-8000-00000000a004",
    );
    let plate_args = |material: &'static str| {
        vec![
            "--width",
            "1000",
            "--length",
            "500",
            "--thickness",
            "10",
            "--op-id",
            plate_op,
            "--new-id",
            plate_id,
            "--material",
            material,
        ]
    };
    let member_args = |profile: &'static str, material: &'static str| {
        vec![
            "--start",
            "0,0,0",
            "--end",
            "0,0,3000",
            "--op-id",
            member_op,
            "--new-id",
            member_id,
            "--profile",
            profile,
            "--material",
            material,
        ]
    };
    let plate = f.ok("create-plate", &plate_args("S355"));
    assert_eq!(text(&plate, "outcome"), "plate_created");
    let member = f.ok("create-member", &member_args("P1", "S355"));
    assert_eq!(text(&member, "outcome"), "member_created");

    // Another material with the grade and another profile with the designation are published.
    f.ok("define-material", &["--density", "7800", "--grade", "S355"]);
    f.ok(
        "define-profile",
        &["--width", "100", "--height", "100", "--designation", "P1"],
    );
    let sequence_before = sequence(&f.ok("definitions", &[]));

    // A new operation by those names is ambiguous now ...
    let fresh = f.rejected(
        "create-plate",
        &[
            "--width",
            "1000",
            "--length",
            "500",
            "--thickness",
            "10",
            "--material",
            "S355",
        ],
    );
    assert_eq!(text(&fresh, "code"), "INVALID_ARGUMENT");
    // ... but the accepted operations are replayed with their original binding.
    let again = f.ok("create-plate", &plate_args("S355"));
    assert_eq!(text(&again, "outcome"), "plate_replayed");
    assert_eq!(text(at(&again, &["plate"]), "material_digest"), steel);
    let again = f.ok("create-member", &member_args("P1", "S355"));
    assert_eq!(text(&again, "outcome"), "member_replayed");
    assert_eq!(text(at(&again, &["member"]), "profile_digest"), profile);
    assert_eq!(text(at(&again, &["member"]), "material_digest"), steel);
    assert_eq!(sequence(&f.ok("definitions", &[])), sequence_before);

    // A changed request under the accepted operation is still reuse, by fingerprint.
    let other_material = f.rejected("create-plate", &plate_args("S355 7800 kg/m3"));
    assert_eq!(text(&other_material, "code"), "OP_ID_REUSE");
    let other_profile = f.rejected("create-member", &member_args("P1", "S355 7800 kg/m3"));
    assert_eq!(text(&other_profile, "code"), "OP_ID_REUSE");
    let mut resized = plate_args("S355");
    resized[1] = "1100";
    assert_eq!(
        text(&f.rejected("create-plate", &resized), "code"),
        "OP_ID_REUSE"
    );
}

#[test]
fn a_large_ambiguity_lists_the_first_candidates_and_the_rest_is_paged() {
    let f = Fixture::new();
    let total = 20;
    let mut digests = std::collections::BTreeSet::new();
    for density in 1..=total {
        let reply = f.ok(
            "define-material",
            &["--density", &density.to_string(), "--name", "Steel"],
        );
        digests.insert(digest_of(&reply));
    }
    let body = f.rejected(
        "create-plate",
        &[
            "--width",
            "10",
            "--length",
            "10",
            "--thickness",
            "1",
            "--material",
            "Steel",
        ],
    );
    assert_eq!(text(&body, "code"), "INVALID_ARGUMENT");
    let diagnostics = at(&body, &["diagnostics"]).array().unwrap();
    assert_eq!(diagnostics.len(), 17, "16 candidates and the summary");
    let summary = diagnostics.last().unwrap();
    assert_eq!(num(summary, &["candidate_count"]), total as f64);
    assert_eq!(num(summary, &["listed"]), 16.0);
    // The rest, and every candidate, can be listed by name in pages.
    let mut listed = std::collections::BTreeSet::new();
    let mut offset = 0u64;
    loop {
        let page = f.ok(
            "definitions",
            &[
                "--name",
                "Steel",
                "--offset",
                &offset.to_string(),
                "--limit",
                "7",
            ],
        );
        assert_eq!(num(&page, &["definition_count"]), total as f64);
        for definition in at(&page, &["definitions"]).array().unwrap() {
            assert!(listed.insert(text(definition, "digest")));
        }
        match at(&page, &["next_offset"]) {
            Json::Null => break,
            next => offset = next.float().unwrap() as u64,
        }
    }
    assert_eq!(listed, digests);
    let none = f.ok("definitions", &["--name", "Nothing"]);
    assert_eq!(num(&none, &["definition_count"]), 0.0);
}

#[test]
fn a_long_grade_still_gets_a_valid_default_name_and_keeps_its_density() {
    let f = Fixture::new();
    let grade = "G".repeat(256);
    let created = f.ok("define-material", &["--density", "7850", "--grade", &grade]);
    let definition = at(&created, &["definition"]);
    assert_eq!(text(definition, "display_name"), grade);
    assert_eq!(num(definition, &["density_kg_m3"]), 7850.0);
    assert_eq!(text(definition, "grade"), grade);
}

#[test]
fn the_entry_count_of_a_library_is_bounded() {
    let f = Fixture::new();
    let bar = "token steel:profile:family = \"rectangle\"\ndouble steel:profile:hMm = 10\ndouble steel:profile:bMm = 10\n";
    let many = |count: usize| {
        let designations: Vec<String> = (0..count).map(|i| format!("T-{i}")).collect();
        let prims: Vec<String> = (0..count).map(|i| format!("E{i}")).collect();
        let entries: Vec<(&str, &str, &str)> = prims
            .iter()
            .zip(&designations)
            .map(|(p, d)| (p.as_str(), d.as_str(), bar))
            .collect();
        library_text(&entries)
    };
    let full = f.file("full.usda", many(4096).as_bytes());
    let (code, body) = f.raw(&["library-list", "--library", &full, "--filter", "T-4095"]);
    assert_eq!(code, 0, "{body:?}");
    assert_eq!(num(&body, &["entry_count"]), 1.0);
    let over = f.file("over.usda", many(4097).as_bytes());
    assert_eq!(
        f.library_rejected("library-list", &["--library", &over]),
        "LIMIT_EXCEEDED"
    );
}

#[test]
fn definitions_have_readable_names_that_are_part_of_their_digest() {
    let f = Fixture::new();
    let steel = f.ok("define-material", &["--density", "7850", "--grade", "S355"]);
    assert_eq!(
        text(at(&steel, &["definition"]), "display_name"),
        "S355 7850 kg/m3"
    );
    let plain = f.ok("define-material", &["--density", "2700"]);
    assert_eq!(
        text(at(&plain, &["definition"]), "display_name"),
        "2700 kg/m3"
    );
    let unknown = f.ok("define-material", &[]);
    assert_eq!(
        text(at(&unknown, &["definition"]), "display_name"),
        "Material (density unknown)"
    );
    let rect = f.ok("define-profile", &["--width", "200", "--height", "300"]);
    assert_eq!(
        text(at(&rect, &["definition"]), "display_name"),
        "RECT 200x300"
    );
    let i = f.ok(
        "define-profile",
        &[
            "--family", "i_shape", "--h", "200", "--b", "100", "--tw", "5", "--tf", "8", "--r",
            "10",
        ],
    );
    assert_eq!(text(at(&i, &["definition"]), "display_name"), "I 200x100");
    let shs = f.ok(
        "define-profile",
        &[
            "--family", "rhs", "--h", "100", "--b", "60", "--t", "4", "--r", "8",
        ],
    );
    assert_eq!(
        text(at(&shs, &["definition"]), "display_name"),
        "RHS 100x60x4"
    );

    // The same content under another name is another definition.
    let renamed = f.ok(
        "define-profile",
        &["--width", "200", "--height", "300", "--name", "Flat bar"],
    );
    assert_eq!(text(&renamed, "outcome"), "definition_created");
    assert_ne!(digest_of(&renamed), digest_of(&rect));
    let listed = f.ok("definitions", &[]);
    let names: Vec<String> = at(&listed, &["definitions"])
        .array()
        .unwrap()
        .iter()
        .map(|d| text(d, "display_name"))
        .collect();
    assert!(names.contains(&"Flat bar".to_owned()));
    assert!(names.contains(&"RECT 200x300".to_owned()));
    assert_eq!(names.len(), 7);
}

#[test]
fn members_and_plates_select_definitions_by_name_and_refuse_an_ambiguous_name() {
    let f = Fixture::new();
    let steel = digest_of(&f.ok("define-material", &["--density", "7850", "--grade", "S355"]));
    let light = digest_of(&f.ok("define-material", &["--density", "7800", "--grade", "S355"]));
    // The grade is shared, so the grade alone is ambiguous; the display name is not.
    let (code, body) = f.run(
        "create-plate",
        &[
            "--width",
            "1000",
            "--length",
            "500",
            "--thickness",
            "10",
            "--material",
            "S355",
        ],
    );
    assert_eq!(code, 2, "{body:?}");
    assert_eq!(text(&body, "code"), "INVALID_ARGUMENT");
    let candidates = at(&body, &["diagnostics"]).array().unwrap();
    let mut listed: Vec<String> = candidates.iter().map(|c| text(c, "digest")).collect();
    listed.sort();
    let mut expected = vec![steel.clone(), light.clone()];
    expected.sort();
    assert_eq!(listed, expected, "every candidate is listed");
    // Nothing was guessed or published.
    assert_eq!(sequence(&f.ok("definitions", &[])), 2);

    let plate = f.ok(
        "create-plate",
        &[
            "--width",
            "1000",
            "--length",
            "500",
            "--thickness",
            "10",
            "--material",
            "S355 7850 kg/m3",
        ],
    );
    assert_eq!(text(at(&plate, &["plate"]), "material_digest"), steel);
    // 0.005 m3 * 7850 kg/m3.
    assert_eq!(num(&plate, &["plate", "quantities", "mass", "kg"]), 39.25);
    let inspected = f.ok("plate-inspect", &[]);
    let listed_plate = at(&inspected, &["plates"]).array().unwrap()[0].clone();
    assert_eq!(text(&listed_plate, "material_name"), "S355 7850 kg/m3");

    // A digest and a name together, a name of the other kind and an unknown name.
    for (args, code) in [
        (
            vec![
                "--width",
                "1000",
                "--length",
                "500",
                "--thickness",
                "10",
                "--material",
                "S355 7850 kg/m3",
                "--material-digest",
                steel.as_str(),
            ],
            "INVALID_ARGUMENT",
        ),
        (
            vec![
                "--width",
                "1000",
                "--length",
                "500",
                "--thickness",
                "10",
                "--material",
                "nope",
            ],
            "NOT_FOUND",
        ),
    ] {
        assert_eq!(text(&f.rejected("create-plate", &args), "code"), code);
    }

    // Two profiles with one designation: ambiguous by that name, still placeable by digest.
    let first = digest_of(&f.ok(
        "define-profile",
        &["--width", "200", "--height", "300", "--designation", "DUP"],
    ));
    let second = digest_of(&f.ok(
        "define-profile",
        &["--width", "100", "--height", "100", "--designation", "DUP"],
    ));
    let member_args = |profile: &'static [&'static str]| {
        let mut args = vec!["--start", "0,0,0", "--end", "0,0,2000"];
        args.extend_from_slice(profile);
        args
    };
    let ambiguous = f.rejected("create-member", &member_args(&["--profile", "DUP"]));
    assert_eq!(text(&ambiguous, "code"), "INVALID_ARGUMENT");
    assert_eq!(at(&ambiguous, &["diagnostics"]).array().unwrap().len(), 2);
    // A material is not a profile by name.
    let wrong_kind = f.rejected("create-member", &member_args(&["--profile", "S355"]));
    assert_eq!(text(&wrong_kind, "code"), "NOT_FOUND");
    let by_digest = f.ok(
        "create-member",
        &[
            "--start",
            "0,0,0",
            "--end",
            "0,0,2000",
            "--profile-digest",
            &first,
            "--material",
            "S355 7800 kg/m3",
        ],
    );
    assert_eq!(text(at(&by_digest, &["member"]), "profile_digest"), first);
    assert_eq!(text(at(&by_digest, &["member"]), "material_digest"), light);
    assert_ne!(first, second);
    let both = f.rejected(
        "create-member",
        &[
            "--start",
            "0,0,0",
            "--end",
            "0,0,2000",
            "--profile",
            "DUP",
            "--profile-digest",
            &first,
        ],
    );
    assert_eq!(text(&both, "code"), "INVALID_ARGUMENT");
    let neither = f.rejected("create-member", &["--start", "0,0,0", "--end", "0,0,2000"]);
    assert_eq!(text(&neither, "code"), "INVALID_ARGUMENT");
}
