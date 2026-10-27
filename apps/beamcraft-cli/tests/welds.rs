// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, beams and welds, MPL-2.0.

//! A06 end to end: `create-weld` and `welds` through the real CLI binary and committed revisions.
//! A weld is a `connection` element with its own identity, explicitly authored properties
//! (type, size, workshop or site, side) and one `connects` record towards two distinct parts; it
//! has no body. Wrong parts and properties are refused with nothing written; a welded part cannot
//! be deleted while the weld connects it, and a weld cannot be copied.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

mod support;

use support::{at, text};

use beamcraft_host::{
    ids::IdSource,
    revision::{ExpectedBase, LayerEntry, NoFault, RevisionDraft, RevisionStore, Uuid7},
    storage::BlobStore,
};
use beamcraft_json::Json;

const WELD: &str = "019a7bf0-0000-7000-8000-000000600001";
const SECOND: &str = "019a7bf0-0000-7000-8000-000000600002";
const FRAME: &str = "019a7bf0-0000-7000-8000-000000600003";

struct Fixture {
    shared: support::StoreFixture,
}
impl Fixture {
    fn new() -> Self {
        Self {
            shared: support::StoreFixture::new("a06-tests"),
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
    fn plate(&self) -> String {
        let body = self.ok(
            "create-plate",
            &["--width", "1000", "--length", "500", "--thickness", "10"],
        );
        text(at(&body, &["plate"]), "element_id")
    }
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
    fn weld_args<'a>(id: &'a str, main: &'a str, secondary: &'a str) -> Vec<&'a str> {
        vec![
            "--name",
            "W1",
            "--new-id",
            id,
            "--main",
            main,
            "--secondary",
            secondary,
            "--size",
            "6",
            "--location",
            "workshop",
            "--side",
            "both",
        ]
    }
    fn weld(&self, id: &str, main: &str, secondary: &str) -> Json {
        self.ok("create-weld", &Self::weld_args(id, main, secondary))
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
    fn welds(&self) -> Vec<Json> {
        list(&self.ok("welds", &[]), "welds").to_vec()
    }
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
fn groups(change: &Json, key: &str) -> Vec<(String, String)> {
    list(at(change, &["footprint"]), key)
        .iter()
        .map(|e| (text(e, "target"), text(e, "group")))
        .collect()
}

#[test]
fn a_weld_authors_its_properties_and_connects_record_and_reads_its_parts() {
    let f = Fixture::new();
    let plate = f.plate();
    let beam = f.beam();
    let inspected = f.ok("inspect-elements", &[]);
    let mesh_before = text(&list(&inspected, "plates")[0], "mesh_digest");

    let created = f.weld(WELD, &plate, &beam);
    assert_eq!(text(&created, "outcome"), "weld_created");
    assert_eq!(text(&created, "element_id"), WELD);
    // No body: only the authored layer changes.
    assert_eq!(strings(&created, "changed_layers"), ["authored.usda"]);
    // The record lives with the weld: every write is the weld's own; both parts are read.
    let writes = groups(&created, "writes");
    assert_eq!(writes.len(), 7);
    assert!(writes.iter().all(|(target, _)| target == WELD));
    for part in [&plate, &beam] {
        assert!(groups(&created, "reads").contains(&(part.clone(), "lifecycle".to_owned())));
    }
    let welds = f.welds();
    assert_eq!(welds.len(), 1);
    let w = &welds[0];
    assert_eq!(text(w, "id"), WELD);
    assert_eq!(text(w, "name"), "W1");
    assert_eq!(text(w, "main"), plate);
    assert_eq!(text(w, "secondary"), beam);
    assert_eq!(text(w, "type"), "fillet");
    assert_eq!(text(w, "size_mm"), "6");
    assert_eq!(text(w, "location"), "workshop");
    assert_eq!(text(w, "side"), "both");

    // The parts' bodies are untouched and the weld adds nothing to the view.
    let inspected = f.ok("inspect-elements", &[]);
    assert_eq!(
        text(&list(&inspected, "plates")[0], "mesh_digest"),
        mesh_before
    );
    assert_eq!(list(at(&inspected, &["snapshot"]), "elements").len(), 2);

    // A retry of the same operation replays; the same id with other content is reuse.
    let op = "019a7bf0-0000-7000-8000-0000006000a1";
    let mut args = Fixture::weld_args(SECOND, &beam, &plate);
    args.extend_from_slice(&["--op-id", op]);
    let first = f.ok("create-weld", &args);
    assert_eq!(text(&first, "outcome"), "weld_created");
    let head = f.head();
    let again = f.ok("create-weld", &args);
    assert_eq!(text(&again, "outcome"), "weld_created_replayed");
    assert_eq!(f.head(), head);
    let mut other = Fixture::weld_args(SECOND, &beam, &plate);
    other[1] = "W2";
    other.extend_from_slice(&["--op-id", op]);
    assert_eq!(f.rejected("create-weld", &other), "OP_ID_REUSE");

    // DV-REL-009: a welded part is not deleted under its weld; DV-REL-011: a weld is not copied.
    assert_eq!(
        f.rejected("delete-element", &["--id", &plate]),
        "DEPENDENCY_CONFLICT"
    );
    assert_eq!(
        f.rejected("copy-element", &["--id", WELD]),
        "DEPENDENCY_CONFLICT"
    );
    for id in [WELD, SECOND] {
        let deleted = f.ok("delete-element", &["--id", id]);
        assert_eq!(text(&deleted, "kind"), "weld");
    }
    assert!(f.welds().is_empty());
    f.ok("delete-element", &["--id", &plate]);
}

#[test]
fn wrong_parts_and_properties_are_refused_with_nothing_written() {
    let f = Fixture::new();
    // No head yet: the parts cannot exist.
    let missing = "019a7bf0-0000-7000-8000-0000006000ff";
    let (code, body) = f.run("create-weld", &Fixture::weld_args(WELD, missing, FRAME));
    assert_eq!((code, text(&body, "code").as_str()), (2, "NOT_FOUND"));

    let plate = f.plate();
    let beam = f.beam();
    f.ok("create-assembly", &["--name", "Frame", "--new-id", FRAME]);
    f.weld(WELD, &plate, &beam);
    let weld = |main: &str, secondary: &str| {
        f.rejected("create-weld", &Fixture::weld_args(SECOND, main, secondary))
    };
    // DV-REL-006/007: two distinct parts.
    assert_eq!(weld(&plate, &plate), "INVALID_ARGUMENT");
    assert_eq!(weld(&plate, missing), "NOT_FOUND");
    // DV-REL-003: a weld joins plates, beams or columns; an assembly is not supported here.
    assert_eq!(weld(&plate, FRAME), "UNSUPPORTED_GEOMETRY");
    assert_eq!(weld(&plate, WELD), "INVALID_ARGUMENT");
    // A weld is not one of its own parts, and its identity must be free.
    assert_eq!(
        f.rejected("create-weld", &Fixture::weld_args(&plate, &plate, &beam)),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.rejected("create-weld", &Fixture::weld_args(FRAME, &plate, &beam)),
        "DUPLICATE_ID"
    );
    // Every property is authored explicitly and checked.
    let with = |key: &str, value: Option<&str>| {
        let mut args = Fixture::weld_args(SECOND, &plate, &beam);
        let at = args.iter().position(|a| *a == key);
        match (at, value) {
            (Some(i), Some(v)) => args[i + 1] = v,
            (Some(i), None) => {
                args.drain(i..i + 2);
            }
            (None, Some(v)) => args.extend_from_slice(&[key, v]),
            (None, None) => {}
        }
        f.rejected("create-weld", &args)
    };
    for key in [
        "--size",
        "--location",
        "--side",
        "--name",
        "--main",
        "--secondary",
    ] {
        assert_eq!(with(key, None), "INVALID_ARGUMENT", "{key}");
    }
    assert_eq!(with("--side", Some("left")), "INVALID_ARGUMENT");
    assert_eq!(with("--location", Some("yard")), "INVALID_ARGUMENT");
    assert_eq!(with("--size", Some("0")), "INVALID_ARGUMENT");
    assert_eq!(with("--size", Some("1000.5")), "INVALID_ARGUMENT");
    assert_eq!(with("--size", Some("NaN")), "NON_FINITE_NUMBER");
    assert_eq!(with("--type", Some("butt")), "UNSUPPORTED_GEOMETRY");
    assert_eq!(with("--colour", Some("red")), "INVALID_ARGUMENT");
    assert_eq!(f.welds().len(), 1);
}

#[test]
fn a_damaged_connects_record_is_refused_not_reported() {
    let f = Fixture::new();
    let plate = f.plate();
    let beam = f.beam();
    f.weld(WELD, &plate, &beam);
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
    let (code, body) = f.run("welds", &[]);
    assert_eq!(
        (code, text(&body, "code").as_str()),
        (2, "DEPENDENCY_CONFLICT")
    );
    let (code, _) = f.run("create-weld", &Fixture::weld_args(SECOND, &plate, &beam));
    assert_eq!(code, 2);
}

#[test]
fn an_operation_id_replays_only_the_weld_its_own_revision_created() {
    let f = Fixture::new();
    let plate = f.plate();
    let beam = f.beam();
    let (op_a, op_b) = (
        "019a7bf0-0000-7000-8000-0000006000b1",
        "019a7bf0-0000-7000-8000-0000006000b2",
    );
    let with_op = |id: &'static str, op: &'static str| {
        let mut args = Fixture::weld_args(id, &plate, &beam);
        args.extend_from_slice(&["--op-id", op]);
        args.iter().map(|s| s.to_string()).collect::<Vec<_>>()
    };
    let first = with_op(WELD, op_a);
    f.ok("create-weld", &as_strs(&first));
    let second = with_op(SECOND, op_b);
    f.ok("create-weld", &as_strs(&second));
    // W2's arguments under op A: the head holds W2, but op A's revision created W1.
    let reused = with_op(SECOND, op_a);
    assert_eq!(f.rejected("create-weld", &as_strs(&reused)), "OP_ID_REUSE");
    // Op A's own request still replays.
    let replay = f.ok("create-weld", &as_strs(&first));
    assert_eq!(text(&replay, "outcome"), "weld_created_replayed");
}

#[test]
fn a_newer_schema_connection_stays_inert_beside_supported_welds() {
    let f = Fixture::new();
    let plate = f.plate();
    let beam = f.beam();
    f.weld(WELD, &plate, &beam);
    let newer = "        def Xform \"CON_019a7bf0000070008000000000600099\" (\n            kind = \"component\"\n        )\n        {\n            custom string bim:id = \"019a7bf0-0000-7000-8000-000000600099\"\n            custom token bim:elementKind = \"connection\"\n            custom string bim:schemaVersion = \"0.2.0\"\n            custom token bim:geometryMode = \"parametric\"\n        }\n";
    f.tamper(|name, text| {
        if name != "authored.usda" {
            return text;
        }
        let folder = text.find("\"Connections\"").unwrap();
        let at = folder + text[folder..].find('{').unwrap() + 1;
        format!("{}{newer}{}", &text[..at], &text[at..])
    });
    // Only the supported weld is reported, and new welds can still be created.
    let welds = f.welds();
    assert_eq!(welds.len(), 1);
    assert_eq!(text(&welds[0], "id"), WELD);
    f.weld(SECOND, &beam, &plate);
    assert_eq!(f.welds().len(), 2);
}

#[test]
fn a_path_weld_keeps_its_symbol_data_and_reports_the_analytic_volume() {
    let f = Fixture::new();
    let plate = f.plate();
    let beam = f.beam();
    // A weld along a world edge: a two-point polyline with symbol data.
    let op = "019a7bf0-0000-7000-8000-0000006000c1";
    let mut args = Fixture::weld_args(WELD, &plate, &beam);
    args.extend_from_slice(&[
        "--path",
        "0,0,0;0,500,0",
        "--all-around",
        "true",
        "--contour",
        "concave",
        "--op-id",
        op,
    ]);
    let created = f.ok("create-weld", &args);
    // The solid is derived data: both layers change.
    let mut changed = strings(&created, "changed_layers");
    changed.sort();
    assert_eq!(changed, ["authored.usda", "derived.usda"]);
    let welds = f.welds();
    let w = welds.iter().find(|w| text(w, "id") == WELD).unwrap();
    // Workshop/site and side kept exactly as authored, with the symbol data beside them.
    assert_eq!(text(w, "location"), "workshop");
    assert_eq!(text(w, "side"), "both");
    assert!(at(w, &["all_around"]).boolean().unwrap());
    assert_eq!(text(w, "contour"), "concave");
    assert_eq!(text(w, "length_mm"), "500");
    // The solid volume against the analytical value: s^2/2 * L = 6^2/2 * 500 = 9000 mm3.
    assert_eq!(text(w, "volume_mm3"), "9000");

    // The same operation replays; different symbol data under its id is reuse.
    let head = f.head();
    let again = f.ok("create-weld", &args);
    assert_eq!(text(&again, "outcome"), "weld_created_replayed");
    assert_eq!(f.head(), head);
    let mut other = Fixture::weld_args(SECOND, &plate, &beam);
    other.extend_from_slice(&[
        "--path",
        "0,0,0;0,500,0",
        "--all-around",
        "false",
        "--contour",
        "concave",
        "--op-id",
        op,
    ]);
    assert_eq!(f.rejected("create-weld", &other), "OP_ID_REUSE");

    // Symbol data without a path has no supported meaning, and a degenerate path is refused.
    let mut symbol_only = Fixture::weld_args(SECOND, &plate, &beam);
    symbol_only.extend_from_slice(&["--all-around", "true"]);
    assert_eq!(f.rejected("create-weld", &symbol_only), "INVALID_ARGUMENT");
    let mut degenerate = Fixture::weld_args(SECOND, &plate, &beam);
    degenerate.extend_from_slice(&["--path", "0,0,0;0,0,0"]);
    assert_eq!(f.rejected("create-weld", &degenerate), "INVALID_ARGUMENT");

    // An A06 weld beside the geometric one has no path and no volume.
    f.weld(SECOND, &beam, &plate);
    let welds = f.welds();
    assert_eq!(welds.len(), 2);
    let plain = welds.iter().find(|w| text(w, "id") == SECOND).unwrap();
    assert_eq!(at(plain, &["path_mm"]), &Json::Null);
}

fn as_strs(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}
