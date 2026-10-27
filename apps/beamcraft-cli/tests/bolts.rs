// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A05 and A14.3 end to end: `create-bolt` (the bolt amendment's form) and `bolt-groups` through
//! the real CLI binary, and the safety behaviour a group keeps from A05: replay by operation id,
//! the delete and copy conflicts, and a damaged committed record that is refused, never reported.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

mod support;

use beamcraft_host::{
    ids::IdSource,
    revision::{ExpectedBase, LayerEntry, NoFault, RevisionDraft, RevisionStore, Uuid7},
    storage::BlobStore,
};
use beamcraft_json::Json;

const BOLT: &str = "019a7bf0-0000-7000-8000-000000700001";
const SECOND: &str = "019a7bf0-0000-7000-8000-000000700002";
const FRAME: &str = "019a7bf0-0000-7000-8000-000000700003";
const FASTENERS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/libraries/synthetic-fasteners.usda"
);

struct Fixture {
    shared: support::StoreFixture,
}
impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("a05-tests"),
        };
        // The fastener definition is pinned once; every group references it by digest.
        fixture.ok(
            "define-fastener",
            &[
                "--fastener-library",
                FASTENERS,
                "--designation",
                "SYN-BOLT-12x50",
            ],
        );
        fixture
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
        let before = self.head();
        let (code, body) = self.run(command, extra);
        assert_eq!(code, 2, "{command} {extra:?}: {body:?}");
        // A refusal writes no revision.
        assert_eq!(self.head(), before, "{command} {extra:?}");
        text(&body, "code")
    }
    fn head(&self) -> String {
        text(&self.ok("open-revision", &[]), "revision_id")
    }
    /// The first-slice plate: world bounds `[0,1000] x [0,500] x [0,10]`.
    fn plate(&self) -> String {
        let body = self.ok(
            "create-plate",
            &["--width", "1000", "--length", "500", "--thickness", "10"],
        );
        text(at(&body, &["plate"]), "element_id")
    }
    /// A straight beam from `0,0,0` to `3000,0,0` with a 100x50 rectangle section, so its
    /// conservative bounds are the axis line expanded to about +-56 in y and z.
    fn beam(&self) -> String {
        let profile = text(
            at(
                &self.ok("define-profile", &["--width", "100", "--height", "50"]),
                &["definition"],
            ),
            "digest",
        );
        let body = self.ok(
            "create-member",
            &[
                "--start",
                "0,0,0",
                "--end",
                "3000,0,0",
                "--profile-digest",
                &profile,
            ],
        );
        text(at(&body, &["member"]), "element_id")
    }
    /// A group of three SYN bolts along the seam of the plate and the beam: the shank points
    /// down from a bearing face at z = 30 and every shank crosses both parts' bounds.
    fn bolt_args<'a>(id: &'a str, parts: &'a str) -> Vec<&'a str> {
        vec![
            "--name",
            "B1",
            "--new-id",
            id,
            "--parts",
            parts,
            "--fastener",
            "SYN-BOLT-12x50",
            "--point1",
            "100,0,30",
            "--point2",
            "1000,0,30",
            "--plane-normal",
            "0,0,-1",
            "--spacings",
            "80,80",
            "--length-mode",
            "fixed",
            "--fixed-length",
            "30",
            "--hole-mode",
            "none",
        ]
    }
    fn bolt(&self, id: &str, main: &str, secondary: &str) -> Json {
        self.ok(
            "create-bolt",
            &Self::bolt_args(id, &format!("{main},{secondary}")),
        )
    }
    /// Commits a head-based revision with edited layer text and no content verification, the way
    /// a layer edited outside BeamCraft would arrive through `commit-revision`.
    fn tamper(&self, edit: impl Fn(&str, String) -> String) {
        let store = RevisionStore::new(BlobStore::new(self.shared.root.join("store")).unwrap());
        let head = store.open().unwrap();
        let content = head.manifest().content();
        let mut ids = IdSource::system();
        let mut entries = Vec::new();
        for entry in content.entries() {
            let original = String::from_utf8(head.layer_bytes(entry.name()).unwrap()).unwrap();
            let edited = edit(entry.name(), original);
            store.blobs().publish(edited.as_bytes()).unwrap();
            entries.push(
                LayerEntry::from_bytes(entry.name(), entry.role(), edited.as_bytes()).unwrap(),
            );
        }
        let draft = RevisionDraft {
            project: content.project().clone(),
            workspace: content.workspace().clone(),
            revision: Uuid7::parse(&ids.next_id().unwrap().to_string()).unwrap(),
            op: Uuid7::parse(&ids.next_id().unwrap().to_string()).unwrap(),
            workspace_generation: content.workspace_generation() + 1,
            schema: content.schema(),
            root_layer: content.root_layer().to_owned(),
            entries,
        };
        store
            .commit_with(
                &draft,
                &ExpectedBase::Revision(head.info().revision().clone()),
                None,
                &mut NoFault,
            )
            .unwrap();
    }
    fn groups(&self) -> Vec<Json> {
        list(&self.ok("bolt-groups", &[]), "groups").to_vec()
    }
}

fn text(json: &Json, key: &str) -> String {
    json.get(key).unwrap().string().unwrap().to_owned()
}
fn at<'a>(json: &'a Json, path: &[&str]) -> &'a Json {
    path.iter().fold(json, |j, k| j.get(k).unwrap())
}
fn list<'a>(json: &'a Json, key: &str) -> &'a [Json] {
    at(json, &[key]).array().unwrap()
}
fn strings(json: &Json, key: &str) -> Vec<String> {
    list(json, key)
        .iter()
        .map(|v| v.string().unwrap().to_owned())
        .collect()
}
fn groups_of(change: &Json, key: &str) -> Vec<(String, String)> {
    list(at(change, &["footprint"]), key)
        .iter()
        .map(|e| (text(e, "target"), text(e, "group")))
        .collect()
}

#[test]
fn a_bolt_group_authors_its_properties_and_connects_record_and_reads_its_parts() {
    let f = Fixture::new();
    let plate = f.plate();
    let beam = f.beam();
    // The fastener definition is pinned first; the group references it by digest.
    let defined = f.ok(
        "define-fastener",
        &[
            "--fastener-library",
            FASTENERS,
            "--designation",
            "SYN-BOLT-12x50",
        ],
    );
    let fastener = text(at(&defined, &["definition"]), "digest");
    let inspected = f.ok("inspect-elements", &[]);
    let mesh_before = text(&list(&inspected, "plates")[0], "mesh_digest");

    let created = f.bolt(BOLT, &plate, &beam);
    assert_eq!(text(&created, "outcome"), "bolt_group_created");
    assert_eq!(text(&created, "element_id"), BOLT);
    // No body: only the authored layer changes.
    assert_eq!(strings(&created, "changed_layers"), ["authored.usda"]);
    // The record lives with the group: every write is the group's own or the immutable
    // definition it references (already present, so only read); both parts are read.
    let writes = groups_of(&created, "writes");
    assert_eq!(writes.len(), 7);
    assert!(writes.iter().all(|(target, _)| target == BOLT));
    for part in [&plate, &beam] {
        assert!(groups_of(&created, "reads").contains(&(part.clone(), "lifecycle".to_owned())));
    }
    let groups = f.groups();
    assert_eq!(groups.len(), 1);
    let b = &groups[0];
    assert_eq!(text(b, "id"), BOLT);
    assert_eq!(text(b, "name"), "B1");
    assert_eq!(strings(b, "parts"), [plate.clone(), beam.clone()]);
    assert_eq!(text(b, "fastener_digest"), fastener);
    assert_eq!(num(b, &["fastener_count"]), 3.0);
    assert_eq!(num(b, &["length_mm"]), 30.0);
    assert_eq!(text(b, "length_mode"), "fixed");
    assert_eq!(text(b, "hole_mode"), "none");

    // The parts' bodies are untouched and the group adds nothing to the view.
    let inspected = f.ok("inspect-elements", &[]);
    assert_eq!(
        text(&list(&inspected, "plates")[0], "mesh_digest"),
        mesh_before
    );
    assert_eq!(list(at(&inspected, &["snapshot"]), "elements").len(), 2);

    // A retry of the same operation replays; the same id with other content is reuse.
    let op = "019a7bf0-0000-7000-8000-0000007000a1";
    let pair = format!("{beam},{plate}");
    let mut args = Fixture::bolt_args(SECOND, &pair);
    args.extend_from_slice(&["--op-id", op]);
    let first = f.ok("create-bolt", &args);
    assert_eq!(text(&first, "outcome"), "bolt_group_created");
    let head = f.head();
    let again = f.ok("create-bolt", &args);
    assert_eq!(text(&again, "outcome"), "bolt_group_created_replayed");
    assert_eq!(f.head(), head);
    let mut other = Fixture::bolt_args(SECOND, &pair);
    other[1] = "B2";
    other.extend_from_slice(&["--op-id", op]);
    assert_eq!(f.rejected("create-bolt", &other), "OP_ID_REUSE");

    // DV-REL-009: a bolted part is not deleted under its group; DV-REL-011: a group is not
    // copied.
    assert_eq!(
        f.rejected("delete-element", &["--id", &plate]),
        "DEPENDENCY_CONFLICT"
    );
    assert_eq!(
        f.rejected("copy-element", &["--id", BOLT]),
        "DEPENDENCY_CONFLICT"
    );
    for id in [BOLT, SECOND] {
        let deleted = f.ok("delete-element", &["--id", id]);
        assert_eq!(text(&deleted, "kind"), "bolt");
    }
    assert!(f.groups().is_empty());
    f.ok("delete-element", &["--id", &plate]);
}

fn num(json: &Json, path: &[&str]) -> f64 {
    at(json, path).float().unwrap()
}

#[test]
fn wrong_parts_and_properties_are_refused_with_nothing_written() {
    let f = Fixture::new();
    // No head yet: the parts cannot exist.
    let missing = "019a7bf0-0000-7000-8000-0000007000ff";
    let (code, body) = f.run(
        "create-bolt",
        &Fixture::bolt_args(BOLT, &format!("{missing},{FRAME}")),
    );
    assert_eq!((code, text(&body, "code").as_str()), (2, "NOT_FOUND"));

    let plate = f.plate();
    let beam = f.beam();
    f.ok("create-assembly", &["--name", "Frame", "--new-id", FRAME]);
    f.bolt(BOLT, &plate, &beam);
    let group = |main: &str, secondary: &str| {
        f.rejected(
            "create-bolt",
            &Fixture::bolt_args(SECOND, &format!("{main},{secondary}")),
        )
    };
    // DV-REL-006/007: two distinct parts.
    assert_eq!(group(&plate, &plate), "INVALID_ARGUMENT");
    assert_eq!(group(&plate, missing), "NOT_FOUND");
    // DV-REL-003: a group joins plates, beams or columns; an assembly is not supported here.
    assert_eq!(group(&plate, FRAME), "UNSUPPORTED_GEOMETRY");
    assert_eq!(group(&plate, BOLT), "INVALID_ARGUMENT");
    // A group is not one of its own parts, and its identity must be free.
    assert_eq!(
        f.rejected(
            "create-bolt",
            &Fixture::bolt_args(&plate, &format!("{plate},{beam}"))
        ),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.rejected(
            "create-bolt",
            &Fixture::bolt_args(FRAME, &format!("{plate},{beam}"))
        ),
        "DUPLICATE_ID"
    );
    // Every property is authored explicitly and checked.
    let pair = format!("{plate},{beam}");
    let with = |key: &str, value: Option<&str>| {
        let mut args = Fixture::bolt_args(SECOND, &pair);
        let at = args.iter().position(|a| *a == key);
        match (at, value) {
            (Some(i), Some(v)) => args[i + 1] = v,
            (Some(i), None) => {
                args.drain(i..i + 2);
            }
            (None, Some(v)) => args.extend_from_slice(&[key, v]),
            (None, None) => {}
        }
        f.rejected("create-bolt", &args)
    };
    for key in [
        "--parts",
        "--point1",
        "--point2",
        "--plane-normal",
        "--spacings",
        "--fastener",
        "--name",
    ] {
        assert_eq!(with(key, None), "INVALID_ARGUMENT", "{key}");
    }
    assert_eq!(with("--spacings", Some("0")), "INVALID_ARGUMENT");
    assert_eq!(with("--spacings", Some("NaN")), "NON_FINITE_NUMBER");
    assert_eq!(with("--spacings", Some("80,0")), "INVALID_ARGUMENT");
    assert_eq!(with("--point2", Some("100,0,30")), "INVALID_ARGUMENT");
    assert_eq!(with("--point1", Some("NaN,0,30")), "NON_FINITE_NUMBER");
    assert_eq!(with("--plane-normal", Some("0,0,0")), "INVALID_ARGUMENT");
    assert_eq!(
        with("--plane-normal", Some("1,0,0")),
        "INVALID_ARGUMENT",
        "the normal is parallel to the line"
    );
    assert_eq!(with("--length-mode", Some("bent")), "INVALID_ARGUMENT");
    assert_eq!(
        with("--washer-under-head", Some("maybe")),
        "INVALID_ARGUMENT"
    );
    assert_eq!(with("--hole-mode", Some("blind")), "INVALID_ARGUMENT");
    assert_eq!(with("--colour", Some("red")), "INVALID_ARGUMENT");
    assert_eq!(f.groups().len(), 1);

    // The same row via the generator: count and pitch expand to the same three fasteners.
    let mut args = Fixture::bolt_args(SECOND, &pair);
    let at = args.iter().position(|a| *a == "--spacings").unwrap();
    args.splice(at..at + 2, ["--count", "3", "--pitch", "80"]);
    f.ok("create-bolt", &args);
    let groups = f.groups();
    assert_eq!(groups.len(), 2);
    assert_eq!(num(&groups[1], &["fastener_count"]), 3.0);
}

#[test]
fn a_damaged_connects_record_is_refused_not_reported() {
    let f = Fixture::new();
    let plate = f.plate();
    let beam = f.beam();
    f.bolt(BOLT, &plate, &beam);
    let path = |id: &str| format!("</World/Steel/Parts/PRT_{}>", id.replace('-', ""));
    let full = format!("rel bim:connects = [{}, {}]", path(&plate), path(&beam));
    let single = format!("rel bim:connects = {}", path(&plate));
    f.tamper(|name, text| {
        if name == "authored.usda" {
            assert!(text.contains(&full), "{text}");
            text.replacen(&full, &single, 1)
        } else {
            text
        }
    });
    let (code, body) = f.run("bolt-groups", &[]);
    assert_eq!(
        (code, text(&body, "code").as_str()),
        (2, "DEPENDENCY_CONFLICT")
    );
    let (code, _) = f.run(
        "create-bolt",
        &Fixture::bolt_args(SECOND, &format!("{plate},{beam}")),
    );
    assert_eq!(code, 2);
}

#[test]
fn an_operation_id_replays_only_the_group_its_own_revision_created() {
    let f = Fixture::new();
    let plate = f.plate();
    let beam = f.beam();
    let (op_a, op_b) = (
        "019a7bf0-0000-7000-8000-0000007000b1",
        "019a7bf0-0000-7000-8000-0000007000b2",
    );
    let with_op = |id: &'static str, op: &'static str| {
        let pair = format!("{plate},{beam}");
        let mut args = Fixture::bolt_args(id, &pair);
        args.extend_from_slice(&["--op-id", op]);
        args.iter().map(|s| s.to_string()).collect::<Vec<_>>()
    };
    let first = with_op(BOLT, op_a);
    f.ok("create-bolt", &as_strs(&first));
    let second = with_op(SECOND, op_b);
    f.ok("create-bolt", &as_strs(&second));
    // B2's arguments under op A: the head holds B2, but op A's revision created B1.
    let reused = with_op(SECOND, op_a);
    assert_eq!(f.rejected("create-bolt", &as_strs(&reused)), "OP_ID_REUSE");
    // Op A's own request still replays.
    let replay = f.ok("create-bolt", &as_strs(&first));
    assert_eq!(text(&replay, "outcome"), "bolt_group_created_replayed");
}

#[test]
fn a_weld_and_a_bolt_group_coexist_and_neither_listing_decodes_the_other() {
    let f = Fixture::new();
    let plate = f.plate();
    let beam = f.beam();
    f.ok(
        "create-weld",
        &[
            "--name",
            "W1",
            "--new-id",
            FRAME,
            "--main",
            &plate,
            "--secondary",
            &beam,
            "--size",
            "6",
            "--location",
            "workshop",
            "--side",
            "both",
        ],
    );
    f.bolt(BOLT, &plate, &beam);
    let welds_body = f.ok("welds", &[]);
    let welds = list(&welds_body, "welds");
    assert_eq!(welds.len(), 1);
    assert_eq!(text(&welds[0], "id"), FRAME);
    assert_eq!(text(&welds[0], "type"), "fillet");
    let groups = f.groups();
    assert_eq!(groups.len(), 1);
    assert_eq!(text(&groups[0], "id"), BOLT);
    assert_eq!(num(&groups[0], &["fastener_count"]), 3.0);
    // Both connections hold the part: neither can be deleted under its connection.
    assert_eq!(
        f.rejected("delete-element", &["--id", &beam]),
        "DEPENDENCY_CONFLICT"
    );
    // The weld is refused as a group part, and the group as a weld part.
    assert_eq!(
        f.rejected(
            "create-weld",
            &[
                "--name",
                "W2",
                "--new-id",
                SECOND,
                "--main",
                &plate,
                "--secondary",
                BOLT,
                "--size",
                "6",
                "--location",
                "workshop",
                "--side",
                "both"
            ]
        ),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.rejected(
            "create-bolt",
            &Fixture::bolt_args(SECOND, &format!("{plate},{FRAME}"))
        ),
        "INVALID_ARGUMENT"
    );
    // Both delete in either order, and then the parts.
    f.ok("delete-element", &["--id", BOLT]);
    f.ok("delete-element", &["--id", FRAME]);
    f.ok("delete-element", &["--id", &beam]);
    f.ok("delete-element", &["--id", &plate]);
}

fn as_strs(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}
