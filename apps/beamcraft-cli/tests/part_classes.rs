// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! P15: classes are authored metadata, preserved by copies and independent of engineering.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use crate::support;
use beamcraft_json::Json;
use support::{StoreFixture, text};

fn ok(f: &StoreFixture, command: &str, options: &[&str]) -> Json {
    let (code, reply) = f.run(command, options);
    assert_eq!(code, 0, "{reply:?}");
    reply
}
fn class(j: &Json) -> f64 {
    j.get("class").unwrap().float().unwrap()
}

#[test]
fn classes_survive_reopen_copy_and_edit_without_changing_the_mesh_or_quantities() {
    let f = StoreFixture::new("p15-classes");
    let plate = ok(
        &f,
        "create-plate",
        &[
            "--width",
            "200",
            "--length",
            "300",
            "--thickness",
            "10",
            "--class",
            "-7",
        ],
    );
    let before = plate.get("plate").unwrap();
    assert_eq!(class(before), -7.0);
    let id = text(before, "element_id");
    let copied = ok(&f, "copy-element", &["--id", &id, "--offset", "1000,0,0"]);
    assert_eq!(class(copied.get("element").unwrap()), -7.0);
    let edited = ok(
        &f,
        "update-element",
        &["--id", &id, "--class", "2147483647"],
    );
    let after = edited.get("element").unwrap();
    assert_eq!(class(after), 2147483647.0);
    assert_eq!(
        before.get("mesh_digest").unwrap(),
        after.get("mesh_digest").unwrap()
    );
    assert_eq!(
        before.get("quantities").unwrap(),
        after.get("quantities").unwrap()
    );
    let inspected = ok(&f, "inspect-elements", &[]);
    let reopened = inspected
        .get("plates")
        .unwrap()
        .array()
        .unwrap()
        .iter()
        .find(|p| text(p, "element_id") == id)
        .unwrap();
    assert_eq!(class(reopened), 2147483647.0);
    for bad in ["1.5", "NaN", "2147483648", "-2147483649"] {
        let (code, _) = f.run("update-element", &["--id", &id, "--class", bad]);
        assert_eq!(code, 2);
    }
    assert_eq!(ok(&f, "inspect-elements", &[]), inspected);
}

#[test]
fn member_class_defaults_to_zero_and_nonzero_class_is_authored() {
    let f = StoreFixture::new("p15-member-classes");
    let profile = ok(&f, "define-profile", &["--width", "20", "--height", "30"]);
    let digest = text(profile.get("definition").unwrap(), "digest");
    for (extra, expected) in [(vec![], 0.0), (vec!["--class", "12"], 12.0)] {
        let mut args = vec![
            "--start",
            "0,0,0",
            "--end",
            "1000,0,0",
            "--profile-digest",
            &digest,
        ];
        args.extend(extra);
        assert_eq!(
            class(ok(&f, "create-member", &args).get("member").unwrap()),
            expected
        );
    }
}
