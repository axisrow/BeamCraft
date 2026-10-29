// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic fixtures, MPL-2.0. Expected values are closed forms.

//! P11 end to end: a shared cut and a shared hole applied to the canonical definition ride the
//! whole instance group in one command. Every member keeps its own placement and label, while
//! the local mesh, the owned operation records and the quantities stay identical by definition.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support;

use support::{at, text};

use beamcraft_json::Json;

struct Fixture {
    shared: support::StoreFixture,
}
impl Fixture {
    fn new() -> Self {
        Self {
            shared: support::StoreFixture::new("p11-tests"),
        }
    }
    fn ok(&self, command: &str, extra: &[&str]) -> Json {
        let (code, body) = self.shared.run(command, extra);
        assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
        body
    }
    fn plates(&self) -> Vec<Json> {
        at(&self.ok("inspect-elements", &[]), &["plates"])
            .array()
            .unwrap()
            .to_vec()
    }
}

fn num(json: &Json, path: &[&str]) -> f64 {
    at(json, path).float().unwrap()
}
fn find<'a>(elements: &'a [Json], id: &str) -> &'a Json {
    elements
        .iter()
        .find(|e| text(e, "element_id") == id)
        .unwrap()
}
fn translation(element: &Json) -> Vec<f64> {
    at(element, &["placement", "translation_mm"])
        .array()
        .unwrap()
        .iter()
        .map(|v| v.float().unwrap())
        .collect()
}

#[test]
fn a_shared_cut_and_hole_ride_the_whole_instance_group() {
    let f = Fixture::new();
    let steel = text(
        f.ok("define-material", &["--density", "7850"])
            .get("definition")
            .unwrap(),
        "digest",
    );
    let created = f.ok(
        "create-plate",
        &[
            "--width",
            "1000",
            "--length",
            "500",
            "--thickness",
            "10",
            "--material-digest",
            &steel,
            "--name",
            "Definition",
            "--translation",
            "10,20,30",
            "--new-id",
            "019a7bf0-0000-7000-8000-000000000001",
        ],
    );
    let definition = text(at(&created, &["plate"]), "element_id");
    for (n, offset) in [
        ("019a7bf0-0000-7000-8000-000000000002", "1100,0,0"),
        ("019a7bf0-0000-7000-8000-000000000003", "2200,0,0"),
    ] {
        let copied = f.ok(
            "copy-instance",
            &[
                "--id",
                &definition,
                "--new-id",
                n,
                "--name",
                "Instance",
                "--offset",
                offset,
            ],
        );
        assert_eq!(text(&copied, "outcome"), "element_copied");
    }
    let group = [
        "019a7bf0-0000-7000-8000-000000000001",
        "019a7bf0-0000-7000-8000-000000000002",
        "019a7bf0-0000-7000-8000-000000000003",
    ];

    // One shared hole on the definition: every member gets the same canonical record.
    f.ok(
        "add-hole",
        &[
            "--id",
            &definition,
            "--operation-id",
            "01a00000-0000-7000-8000-0000000000c1",
            "--hole-rect",
            "100,100,100,100",
        ],
    );
    // One shared plane cut on the definition: the group is updated atomically.
    let cut = f.ok(
        "cut-by-plane",
        &[
            "--id",
            &definition,
            "--operation-id",
            "01a00000-0000-7000-8000-0000000000c2",
            "--point",
            "250,0,0",
            "--normal",
            "1,0,0",
            "--keep",
            "negative",
        ],
    );
    assert_eq!(text(&cut, "outcome"), "cut_applied");

    let plates = f.plates();
    assert_eq!(plates.len(), 3);
    let members: Vec<_> = group.iter().map(|id| find(&plates, id).clone()).collect();
    // The shared shape: identical local mesh, records and quantities everywhere.
    let mesh = text(&members[0], "mesh_digest");
    let cuts = format!("{:?}", at(&members[0], &["cuts"]));
    let holes = format!("{:?}", at(&members[0], &["hole_records"]));
    let volume = num(&members[0], &["quantities", "volume", "mm3"]);
    let mass = num(&members[0], &["quantities", "mass", "kg"]);
    assert_eq!(at(&members[0], &["cuts"]).array().unwrap().len(), 1);
    assert_eq!(at(&members[0], &["hole_records"]).array().unwrap().len(), 1);
    for member in &members[1..] {
        assert_eq!(text(member, "mesh_digest"), mesh);
        assert_eq!(format!("{:?}", at(member, &["cuts"])), cuts);
        assert_eq!(format!("{:?}", at(member, &["hole_records"])), holes);
        assert_eq!(num(member, &["quantities", "volume", "mm3"]), volume);
        assert_eq!(num(member, &["quantities", "mass", "kg"]), mass);
    }
    // The volume is the exact kept half-space minus the hole: closed forms.
    assert_eq!(volume, 1_150_000.0);
    // Each member keeps its own identity facts.
    assert_eq!(text(&members[0], "name"), "Definition");
    assert_eq!(text(&members[1], "name"), "Instance");
    assert_eq!(translation(&members[0]), [10.0, 20.0, 30.0]);
    assert_eq!(translation(&members[1]), [1110.0, 20.0, 30.0]);
    assert_eq!(translation(&members[2]), [2210.0, 20.0, 30.0]);
}
