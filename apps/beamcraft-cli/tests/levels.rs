// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, beams and levels, MPL-2.0.

//! D1 and D2 end to end through the real CLI binary and committed revisions.
//!
//! D1: the first revision records the project in the root layer; later revisions agree; an
//! explicit different `--project-id` is refused with nothing written; a fork reports its
//! parent's project; a store without a record is never given one; damaged records are typed
//! errors. D2: levels are elements with their own identity and no body; assignment is the
//! `bim:level` record of the plate or member (`update-element --level`); a level never touches a
//! body; a level in use is not deleted; a copy keeps the level.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support;

use support::{at, text};

use beamcraft_host::{
    ids::IdSource,
    revision::{ExpectedBase, LayerEntry, NoFault, RevisionDraft, RevisionStore, Uuid7},
    storage::BlobStore,
};
use beamcraft_json::Json;
use std::fs;
use std::process::Command;

const PROJECT: &str = "019a7bf0-0000-7000-8000-00000000d001";
const OTHER_PROJECT: &str = "019a7bf0-0000-7000-8000-00000000d002";
const STOREY: &str = "019a7bf0-0000-7000-8000-000000300001";
const ROOF: &str = "019a7bf0-0000-7000-8000-000000300002";
const MISSING: &str = "019a7bf0-0000-7000-8000-0000003000ff";
const ASSEMBLY: &str = "019a7bf0-0000-7000-8000-000000300020";
const WRAPPER: &str = "019a7bf0-0000-7000-8000-000000300030";

/// An invented foreign source for the wrapper of the reference regression.
const SITE: &str = "#usda 1.0\n(\n    defaultPrim = \"Site\"\n    metersPerUnit = 1.0\n    upAxis = \"Z\"\n)\ndef Xform \"Site\"\n{\n}\n";

struct Fixture {
    shared: support::StoreFixture,
}
impl Fixture {
    fn new() -> Self {
        Self {
            shared: support::StoreFixture::new("d1d2-tests"),
        }
    }

    /// Writes the invented foreign source and returns its path text.
    fn source(&self) -> String {
        let path = self.shared.root.join("site.usda");
        fs::write(&path, SITE).unwrap();
        path.to_str().unwrap().to_owned()
    }
    fn run_in(&self, store: &str, command: &str, extra: &[&str]) -> (i32, Json) {
        support::StoreFixture::run_in(store, command, extra)
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
    fn plate_with(&self, extra: &[&str]) -> String {
        let mut args = vec!["--width", "1000", "--length", "500", "--thickness", "10"];
        args.extend_from_slice(extra);
        text(
            at(&self.ok("create-plate", &args), &["plate"]),
            "element_id",
        )
    }
    fn plate(&self) -> String {
        self.plate_with(&[])
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
    fn level(&self, id: &str, name: &str, elevation: &str) -> Json {
        self.ok(
            "create-level",
            &["--name", name, "--elevation", elevation, "--new-id", id],
        )
    }
    fn assign(&self, element: &str, level: &str) -> Json {
        self.ok("update-element", &["--id", element, "--level", level])
    }
    fn inspect(&self) -> Json {
        self.ok("inspect-elements", &[])
    }
    fn levels(&self) -> Vec<Json> {
        list(&self.ok("levels", &[]), "levels").to_vec()
    }
    fn layer_digest(&self, name: &str) -> String {
        let body = self.ok("open-revision", &[]);
        list(&body, "layers")
            .iter()
            .find(|l| text(l, "name") == name)
            .map(|l| text(l, "digest"))
            .unwrap()
    }
    fn layer_text(&self, name: &str) -> String {
        let store = RevisionStore::new(BlobStore::new(self.shared.root.join("store")).unwrap());
        let head = store.open().unwrap();
        String::from_utf8(head.layer_bytes(name).unwrap()).unwrap()
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
fn project_record(inspected: &Json) -> Option<String> {
    inspected
        .get("project_id_record")
        .unwrap()
        .string()
        .ok()
        .map(str::to_owned)
}
fn manifest_project(json: &Json) -> String {
    text(at(json, &["revision", "manifest"]), "project_id")
}
fn element<'a>(inspected: &'a Json, key: &str, id: &str) -> &'a Json {
    list(inspected, key)
        .iter()
        .find(|e| text(e, "element_id") == id)
        .unwrap()
}
fn level_of(inspected: &Json, key: &str, id: &str) -> Option<String> {
    element(inspected, key, id)
        .get("level")
        .unwrap()
        .string()
        .ok()
        .map(str::to_owned)
}
fn level<'a>(levels: &'a [Json], id: &str) -> &'a Json {
    levels.iter().find(|l| text(l, "id") == id).unwrap()
}

#[test]
fn the_first_revision_records_the_project_and_later_revisions_agree() {
    let f = Fixture::new();
    // The first command records its --project-id in the root layer, once.
    let plate = f.plate_with(&["--project-id", PROJECT]);
    let first = f.inspect();
    assert_eq!(project_record(&first).as_deref(), Some(PROJECT));
    assert_eq!(manifest_project(&first), PROJECT);
    let root = f.layer_text("root.usda");
    assert!(
        root.contains("bimProjectId") && root.contains(PROJECT),
        "{root}"
    );
    assert!(root.contains("bimSchemaDigest"));
    let root_digest = f.layer_digest("root.usda");

    // Every later revision reports the same record, whatever command wrote it, and the root
    // layer is not rewritten: nothing but the first revision authors the record.
    f.level(STOREY, "Level 1", "0");
    f.assign(&plate, STOREY);
    f.ok("update-element", &["--id", &plate, "--name", "Renamed"]);
    f.ok("create-assembly", &["--name", "A", "--project-id", PROJECT]);
    let later = f.inspect();
    assert_eq!(project_record(&later).as_deref(), Some(PROJECT));
    assert_eq!(manifest_project(&later), PROJECT);
    assert_eq!(f.layer_digest("root.usda"), root_digest);

    // A different explicit project id is refused by every command that writes a revision, with
    // no revision written; the same id is accepted; no id takes the store's own.
    let before = f.head();
    for (command, extra) in [
        (
            "create-plate",
            vec!["--width", "10", "--length", "10", "--thickness", "1"],
        ),
        ("create-level", vec!["--name", "L", "--elevation", "1"]),
        ("create-assembly", vec!["--name", "B"]),
        ("update-element", vec!["--id", &plate, "--name", "X"]),
        ("delete-element", vec!["--id", &plate]),
        ("define-material", vec![]),
    ] {
        let mut args = extra.clone();
        args.extend(["--project-id", OTHER_PROJECT]);
        assert_eq!(f.rejected(command, &args), "INVALID_ARGUMENT", "{command}");
    }
    assert_eq!(f.head(), before);
    f.ok(
        "update-element",
        &["--id", &plate, "--name", "Same", "--project-id", PROJECT],
    );
    assert_eq!(project_record(&f.inspect()).as_deref(), Some(PROJECT));
    // A malformed id is an identity error, not a mismatch.
    assert_eq!(
        f.rejected("create-assembly", &["--name", "C", "--project-id", "nope"]),
        "INVALID_ID"
    );
}

#[test]
fn a_generated_project_is_recorded_and_a_fork_reports_its_parent_project() {
    let f = Fixture::new();
    f.plate();
    let inspected = f.inspect();
    let project = manifest_project(&inspected);
    assert_eq!(
        project_record(&inspected).as_deref(),
        Some(project.as_str())
    );

    let dir = f.shared.root.join("draft").to_str().unwrap().to_owned();
    f.ok(
        "create-workspace",
        &[
            "--workspace-dir",
            &dir,
            "--user-id",
            "019a7bf0-0000-7000-8000-00000000d0aa",
        ],
    );
    let (code, forked) = f.run_in(&dir, "inspect-elements", &[]);
    assert_eq!(code, 0, "{forked:?}");
    assert_eq!(project_record(&forked).as_deref(), Some(project.as_str()));
    assert_eq!(manifest_project(&forked), project);
    // An edit inside the fork keeps it, and the fork refuses another explicit project.
    let (code, edited) = f.run_in(
        &dir,
        "create-level",
        &["--name", "Draft level", "--elevation", "1000"],
    );
    assert_eq!(code, 0, "{edited:?}");
    let (code, refused) = f.run_in(
        &dir,
        "create-level",
        &[
            "--name",
            "Other",
            "--elevation",
            "1",
            "--project-id",
            OTHER_PROJECT,
        ],
    );
    assert_eq!(code, 2, "{refused:?}");
    assert_eq!(text(&refused, "code"), "INVALID_ARGUMENT");
    let (_, again) = f.run_in(&dir, "inspect-elements", &[]);
    assert_eq!(project_record(&again).as_deref(), Some(project.as_str()));
}

#[test]
fn a_store_without_a_record_is_never_given_one_and_damaged_records_are_typed_errors() {
    let f = Fixture::new();
    f.plate_with(&["--project-id", PROJECT]);
    // The record as an older store would not have it: removed outside BeamCraft.
    f.tamper(|name, text| {
        if name == "root.usda" {
            text.lines()
                .filter(|l| !l.contains("bimProjectId"))
                .collect::<Vec<_>>()
                .join("\n")
                + "\n"
        } else {
            text
        }
    });
    let legacy = f.inspect();
    assert_eq!(project_record(&legacy), None);
    assert_eq!(manifest_project(&legacy), PROJECT);
    // Later revisions keep it absent: the export refuses such a store, it is not repaired here.
    f.level(STOREY, "Level 1", "0");
    f.plate();
    assert_eq!(project_record(&f.inspect()), None);
    assert!(!f.layer_text("root.usda").contains("bimProjectId"));

    // An invalid stored value is INVALID_ID when read, and a valid one that is not the
    // revision's own project is an integrity error.
    for (value, code) in [
        ("not-a-uuid", "INVALID_ID"),
        (OTHER_PROJECT, "INTEGRITY_ERROR"),
    ] {
        let g = Fixture::new();
        g.plate_with(&["--project-id", PROJECT]);
        g.tamper(|name, text| {
            if name == "root.usda" {
                text.replace(PROJECT, value)
            } else {
                text
            }
        });
        let (status, body) = g.run("inspect-elements", &[]);
        assert_eq!(status, 2, "{value}: {body:?}");
        assert_eq!(text(&body, "code"), code, "{value}");
        // Nothing builds on a damaged record either.
        let (status, body) = g.run("create-level", &["--name", "L", "--elevation", "0"]);
        assert_eq!(status, 2, "{value}: {body:?}");
        assert_eq!(text(&body, "code"), code, "{value}");
    }
}

#[test]
fn levels_are_created_listed_assigned_and_never_touch_a_body() {
    let f = Fixture::new();
    let plate = f.plate();
    let beam = f.beam();
    let before = f.inspect();
    let mesh_before = text(element(&before, "plates", &plate), "mesh_digest");
    let derived_before = f.layer_digest("derived.usda");
    // A project with elements and no level reports none and no level on any element.
    assert!(f.levels().is_empty());
    assert_eq!(level_of(&before, "plates", &plate), None);
    assert_eq!(level_of(&before, "members", &beam), None);

    let created = f.level(ROOF, "Roof", "-250.5");
    assert_eq!(text(&created, "outcome"), "level_created");
    assert_eq!(strings(&created, "changed_layers"), ["authored.usda"]);
    assert_eq!(text(&created, "element_id"), ROOF);
    // A level is its own element: it writes only itself (the create footprint is conservative and
    // declares every group of the new element).
    let written = groups(&created, "writes");
    assert!(written.iter().all(|(target, _)| target == ROOF));
    f.level(STOREY, "Level 1", "3000");

    // Levels list in ascending identity order with their elevation and elements.
    let listed = f.levels();
    assert_eq!(
        listed.iter().map(|l| text(l, "id")).collect::<Vec<_>>(),
        [STOREY, ROOF]
    );
    assert_eq!(text(level(&listed, STOREY), "name"), "Level 1");
    assert!(strings(level(&listed, STOREY), "elements").is_empty());
    let elevation = level(&listed, ROOF).get("elevation_mm").unwrap();
    assert_eq!(elevation, &Json::number(-250.5).unwrap());

    // Assigning writes the element's relationship group only and rebuilds no body.
    let assigned = f.assign(&plate, STOREY);
    assert_eq!(text(&assigned, "outcome"), "element_updated");
    assert_eq!(at(&assigned, &["effect", "level"]), &Json::Bool(true));
    assert_eq!(at(&assigned, &["effect", "shape"]), &Json::Bool(false));
    assert_eq!(at(&assigned, &["effect", "placement"]), &Json::Bool(false));
    assert_eq!(strings(&assigned, "changed_layers"), ["authored.usda"]);
    assert_eq!(
        groups(&assigned, "writes"),
        [(plate.clone(), "relationship".to_owned())]
    );
    assert!(groups(&assigned, "reads").contains(&(STOREY.to_owned(), "lifecycle".to_owned())));
    f.assign(&beam, STOREY);
    assert_eq!(f.layer_digest("derived.usda"), derived_before.clone());

    let after = f.inspect();
    assert_eq!(level_of(&after, "plates", &plate).as_deref(), Some(STOREY));
    assert_eq!(level_of(&after, "members", &beam).as_deref(), Some(STOREY));
    // Shape key, mass signature and mesh bytes are those they were before any level existed.
    assert_eq!(
        text(element(&after, "plates", &plate), "mesh_digest"),
        mesh_before
    );
    assert_eq!(
        at(element(&after, "plates", &plate), &["stored_quantities"]),
        at(element(&before, "plates", &plate), &["stored_quantities"])
    );
    assert_eq!(
        at(&after, &["snapshot"]).get("meshes"),
        at(&before, &["snapshot"]).get("meshes")
    );
    let listed = f.levels();
    let mut users = strings(level(&listed, STOREY), "elements");
    users.sort();
    let mut expected = vec![plate.clone(), beam.clone()];
    expected.sort();
    assert_eq!(users, expected);
    // The level list is part of the inspection too.
    assert_eq!(list(&after, "levels").len(), 2);

    // Another level replaces the record (one level per element); none removes it.
    f.assign(&plate, ROOF);
    assert_eq!(
        level_of(&f.inspect(), "plates", &plate).as_deref(),
        Some(ROOF)
    );
    let cleared = f.ok("update-element", &["--id", &plate, "--level", "none"]);
    assert_eq!(at(&cleared, &["effect", "level"]), &Json::Bool(true));
    assert_eq!(level_of(&f.inspect(), "plates", &plate), None);
    assert_eq!(f.layer_digest("derived.usda"), derived_before);
    // A move or a rename next to the level is one operation with both effects.
    let both = f.ok(
        "update-element",
        &[
            "--id",
            &plate,
            "--level",
            STOREY,
            "--translation",
            "100,0,0",
        ],
    );
    assert_eq!(at(&both, &["effect", "level"]), &Json::Bool(true));
    assert_eq!(at(&both, &["effect", "placement"]), &Json::Bool(true));
    assert_eq!(
        level_of(&f.inspect(), "plates", &plate).as_deref(),
        Some(STOREY)
    );
}

#[test]
fn level_arguments_assignments_and_identities_are_validated_with_nothing_written() {
    let f = Fixture::new();
    let plate = f.plate();
    let beam = f.beam();
    f.level(STOREY, "Level 1", "0");
    f.ok(
        "create-assembly",
        &["--name", "Frame", "--new-id", ASSEMBLY],
    );
    let create = |name: &str, elevation: &str, id: &str| {
        f.rejected(
            "create-level",
            &["--name", name, "--elevation", elevation, "--new-id", id],
        )
    };
    // Duplicate identities, whatever the kind that holds them.
    assert_eq!(create("Again", "1", STOREY), "DUPLICATE_ID");
    assert_eq!(create("Again", "1", &plate), "DUPLICATE_ID");
    // Elevations: finite and within the coordinate cap.
    assert_eq!(create("L", "NaN", ROOF), "NON_FINITE_NUMBER");
    assert_eq!(create("L", "inf", ROOF), "NON_FINITE_NUMBER");
    assert_eq!(create("L", "1000000000.5", ROOF), "LIMIT_EXCEEDED");
    assert_eq!(create("L", "-1e12", ROOF), "LIMIT_EXCEEDED");
    assert_eq!(create("L", "high", ROOF), "INVALID_ARGUMENT");
    assert_eq!(create("", "1", ROOF), "INVALID_ARGUMENT");
    assert_eq!(create("L", "1", "not-a-uuid"), "INVALID_ID");
    assert_eq!(
        f.rejected("create-level", &["--name", "L", "--new-id", ROOF]),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.rejected("create-level", &["--elevation", "1", "--new-id", ROOF]),
        "INVALID_ARGUMENT"
    );
    // The edges are accepted.
    f.level(ROOF, "Far", "1000000000");

    let update = |element: &str, level: &str| {
        f.rejected("update-element", &["--id", element, "--level", level])
    };
    // An unknown level is a dangling relationship; a target that is not a level is an argument
    // error; a malformed id is an identity error.
    assert_eq!(update(&plate, MISSING), "DANGLING_RELATIONSHIP");
    assert_eq!(update(&plate, &beam), "INVALID_ARGUMENT");
    assert_eq!(update(&plate, "not-a-uuid"), "INVALID_ID");
    assert_eq!(update(MISSING, STOREY), "NOT_FOUND");
    // An element that exists but cannot hold a level is an unsupported assignment, not a missing
    // element: a level and an assembly.
    assert_eq!(update(STOREY, ROOF), "UNSUPPORTED_PROPERTY");
    assert_eq!(update(ASSEMBLY, STOREY), "UNSUPPORTED_PROPERTY");
    // Removing a level that is not there, or assigning the one already held, changes nothing.
    assert_eq!(update(&plate, "none"), "INVALID_ARGUMENT");
    f.assign(&plate, STOREY);
    assert_eq!(update(&plate, STOREY), "INVALID_ARGUMENT");
    assert_eq!(
        f.rejected(
            "update-element",
            &["--id", &plate, "--level", "none", "--name", ""]
        ),
        "INVALID_ARGUMENT"
    );
}

#[test]
fn replay_reuse_delete_and_copy_follow_the_level_records() {
    let f = Fixture::new();
    let plate = f.plate();
    let create_op = "019a7bf0-0000-7000-8000-00000000b001";
    let assign_op = "019a7bf0-0000-7000-8000-00000000b002";
    let args = [
        "--name",
        "Level 1",
        "--elevation",
        "3000",
        "--new-id",
        STOREY,
    ];
    let first = f.ok(
        "create-level",
        &[&args[..], &["--op-id", create_op]].concat(),
    );
    let again = f.ok(
        "create-level",
        &[&args[..], &["--op-id", create_op]].concat(),
    );
    assert_eq!(text(&again, "outcome"), "level_created_replayed");
    assert_eq!(
        text(at(&again, &["revision"]), "revision_id"),
        text(at(&first, &["revision"]), "revision_id")
    );
    assert_eq!(at(&again, &["footprint"]), &Json::Null);
    assert_eq!(
        f.rejected(
            "create-level",
            &[
                "--name",
                "Level 1",
                "--elevation",
                "3001",
                "--new-id",
                STOREY,
                "--op-id",
                create_op
            ],
        ),
        "OP_ID_REUSE"
    );
    // The operation id belongs to the creation of STOREY: the arguments of another level that
    // the head holds are not a replay of it, whatever the head says.
    f.level(ROOF, "Roof", "9000");
    assert_eq!(
        f.rejected(
            "create-level",
            &[
                "--name",
                "Roof",
                "--elevation",
                "9000",
                "--new-id",
                ROOF,
                "--op-id",
                create_op
            ],
        ),
        "OP_ID_REUSE"
    );
    let assign = [
        "--id",
        plate.as_str(),
        "--level",
        STOREY,
        "--op-id",
        assign_op,
    ];
    f.ok("update-element", &assign);
    let replay = f.ok("update-element", &assign);
    assert_eq!(text(&replay, "outcome"), "element_update_replayed");
    assert_eq!(f.levels().len(), 2);

    // A copy stays on the level of its source and the source is untouched.
    let copy_id = "019a7bf0-0000-7000-8000-000000300010";
    let copied = f.ok("copy-element", &["--id", &plate, "--new-id", copy_id]);
    assert_eq!(text(&copied, "outcome"), "element_copied");
    let inspected = f.inspect();
    assert_eq!(
        level_of(&inspected, "plates", copy_id).as_deref(),
        Some(STOREY)
    );
    assert_eq!(
        level_of(&inspected, "plates", &plate).as_deref(),
        Some(STOREY)
    );
    assert_eq!(
        text(element(&inspected, "plates", copy_id), "mesh_digest"),
        text(element(&inspected, "plates", &plate), "mesh_digest")
    );
    // A level is not copied.
    assert_eq!(
        f.rejected("copy-element", &["--id", STOREY]),
        "UNSUPPORTED_PROPERTY"
    );

    // A level that elements use is not deleted, and there is no cascade.
    assert_eq!(
        f.rejected("delete-element", &["--id", STOREY]),
        "DEPENDENCY_CONFLICT"
    );
    // Deleting an element removes its own record; the level stays with the others.
    f.ok("delete-element", &["--id", &plate]);
    assert_eq!(strings(level(&f.levels(), STOREY), "elements"), [copy_id]);
    assert_eq!(
        f.rejected("delete-element", &["--id", STOREY]),
        "DEPENDENCY_CONFLICT"
    );
    f.ok("update-element", &["--id", copy_id, "--level", "none"]);
    let deleted = f.ok("delete-element", &["--id", STOREY]);
    assert_eq!(text(&deleted, "kind"), "level");
    assert_eq!(f.levels().len(), 1);
}

#[test]
fn level_records_stored_outside_beamcraft_are_refused_when_read() {
    let path = |id: &str| {
        let category = if id == STOREY || id == ROOF {
            beamcraft_model::prim::ElementCategory::Level
        } else {
            beamcraft_model::prim::ElementCategory::Part
        };
        beamcraft_model::id::ElementId::parse(id)
            .unwrap()
            .prim_path(category)
    };
    // A target that is not a level, two targets, a target that does not exist, an elevation
    // beyond the cap and a level record on an owned foreign-source wrapper. Every reader that
    // interprets levels refuses them with the same code: the level list, the inspection and any
    // command that builds on the store.
    for (case, code) in [
        ("member", "INVALID_ARGUMENT"),
        ("two", "DEPENDENCY_CONFLICT"),
        ("missing", "DANGLING_RELATIONSHIP"),
        ("elevation", "LIMIT_EXCEEDED"),
        ("wrapper", "INVALID_ARGUMENT"),
    ] {
        let g = Fixture::new();
        let plate = g.plate();
        let beam = g.beam();
        g.level(STOREY, "Level 1", "0");
        g.assign(&plate, STOREY);
        if case == "wrapper" {
            let source = g.source();
            g.ok(
                "attach-reference",
                &["--source", &source, "--new-id", WRAPPER],
            );
        }
        let from = match case {
            "elevation" => "bim:elevationMm = 0.0".to_owned(),
            _ => format!("bim:level = <{}>", path(STOREY)),
        };
        let to = match case {
            "member" => format!("bim:level = <{}>", path(&beam)),
            "two" => format!("bim:level = [<{}>, <{}>]", path(STOREY), path(&beam)),
            "missing" => format!("bim:level = <{}>", path(MISSING)),
            "elevation" => "bim:elevationMm = 3000000000.5".to_owned(),
            _ => format!("bim:level = <{}>", path(STOREY)),
        };
        g.tamper(|name, text| {
            if name != "authored.usda" {
                return text;
            }
            if case == "wrapper" {
                // The record is added to the wrapper; the plate's own record stays valid.
                let at = text
                    .find(&format!("\"REF_{}\"", WRAPPER.replace('-', "")))
                    .unwrap();
                let open = at + text[at..].find('{').unwrap() + 1;
                return format!(
                    "{}\n        custom rel bim:level = <{}>{}",
                    &text[..open],
                    path(STOREY),
                    &text[open..]
                );
            }
            assert!(text.contains(&from), "{text}");
            text.replace(&from, &to)
        });
        for command in ["levels", "inspect-elements"] {
            let (status, body) = g.run(command, &[]);
            assert_eq!(status, 2, "{case} {command}: {body:?}");
            assert_eq!(text(&body, "code"), code, "{case} {command}");
        }
        let (status, body) = g.run("create-level", &["--name", "L", "--elevation", "1"]);
        assert_eq!(status, 2, "{case}: {body:?}");
        assert_eq!(text(&body, "code"), code, "{case}");
    }
}

#[test]
fn a_foreign_or_newer_schema_level_stays_inert_instead_of_blocking_readers() {
    let f = Fixture::new();
    let plate = f.plate();
    f.level(STOREY, "Level 1", "3000");
    f.level(ROOF, "Roof", "6000");
    let header = format!("\"LVL_{}\"", STOREY.replace('-', ""));
    // A level written by a newer schema: it does not carry the current elevation field.
    f.tamper(|name, text| {
        if name != "authored.usda" {
            return text;
        }
        let at = text.find(&header).unwrap();
        let (head, tail) = text.split_at(at);
        let tail = tail.replacen(
            "bim:schemaVersion = \"0.1.0\"",
            "bim:schemaVersion = \"0.2.0\"",
            1,
        );
        let tail: Vec<&str> = tail
            .lines()
            .filter(|l| !l.contains("bim:elevationMm") || l.contains("= 6000"))
            .collect();
        format!("{head}{}\n", tail.join("\n"))
    });
    // It is not listed, it does not block the inspection or an unrelated edit, and its
    // identity is still in use.
    let listed = f.levels();
    assert_eq!(
        listed.iter().map(|l| text(l, "id")).collect::<Vec<_>>(),
        [ROOF]
    );
    let inspected = f.inspect();
    assert_eq!(list(&inspected, "plates").len(), 1);
    f.ok("update-element", &["--id", &plate, "--name", "Renamed"]);
    assert_eq!(
        f.rejected(
            "create-level",
            &["--name", "Again", "--elevation", "1", "--new-id", STOREY],
        ),
        "DUPLICATE_ID"
    );
    assert_eq!(
        f.rejected("update-element", &["--id", STOREY, "--level", ROOF]),
        "UNSUPPORTED_PROPERTY"
    );
}

#[test]
fn copying_connected_assembly_parts_keeps_only_the_level_replays_and_supports_undo_redo() {
    let f = Fixture::new();
    let plate = f.plate();
    let beam = f.beam();
    f.level(STOREY, "Level 1", "3000");
    f.ok(
        "create-assembly",
        &["--name", "Frame", "--new-id", ASSEMBLY],
    );
    for source in [&plate, &beam] {
        f.assign(source, STOREY);
        f.ok(
            "attach-to-assembly",
            &["--id", source, "--assembly-id", ASSEMBLY],
        );
    }
    f.ok(
        "create-weld",
        &[
            "--name",
            "Synthetic weld",
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
    let before_assembly = f.ok("assemblies", &[]);
    let before_welds = f.ok("welds", &[]);
    let before = f.inspect();
    let draft = f
        .shared
        .root
        .join("copy-draft")
        .to_str()
        .unwrap()
        .to_owned();
    f.ok("create-workspace", &["--workspace-dir", &draft]);
    let run = |command: &str, args: &[&str]| {
        let (code, json) = if matches!(command, "undo-workspace" | "redo-workspace") {
            let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
                .arg(command)
                .args(args)
                .output()
                .unwrap();
            (
                output.status.code().unwrap(),
                beamcraft_json::decode(&output.stdout, &beamcraft_json::Limits::DEFAULT).unwrap(),
            )
        } else {
            f.run_in(&draft, command, args)
        };
        assert_eq!(code, 0, "{command}: {json:?}");
        json
    };
    for (source, key, copy_id, op) in [
        (
            &plate,
            "plates",
            "019a7bf0-0000-7000-8000-000000300071",
            "019a7bf0-0000-7000-8000-000000300081",
        ),
        (
            &beam,
            "members",
            "019a7bf0-0000-7000-8000-000000300072",
            "019a7bf0-0000-7000-8000-000000300082",
        ),
    ] {
        let args = ["--id", source, "--new-id", copy_id, "--op-id", op];
        run("copy-element", &args);
        let inspected = run("inspect-elements", &[]);
        assert_eq!(level_of(&inspected, key, copy_id).as_deref(), Some(STOREY));
        assert_eq!(
            element(&inspected, key, source),
            element(&before, key, source)
        );
        assert_eq!(
            text(element(&inspected, key, copy_id), "mesh_digest"),
            text(element(&before, key, source), "mesh_digest")
        );
        assert_eq!(run("assemblies", &[]), before_assembly);
        assert_eq!(run("welds", &[]), before_welds);
        assert_eq!(
            text(&run("copy-element", &args), "outcome"),
            "element_copy_replayed"
        );
        run("undo-workspace", &["--workspace-dir", &draft]);
        let undone = run("inspect-elements", &[]);
        assert!(
            !list(&undone, key)
                .iter()
                .any(|e| text(e, "element_id") == copy_id)
        );
        assert_eq!(run("assemblies", &[]), before_assembly);
        run("redo-workspace", &["--workspace-dir", &draft]);
        let redone = run("inspect-elements", &[]);
        assert_eq!(list(&redone, key), list(&inspected, key));
    }
}

#[test]
fn invalid_deactivation_reaches_cli_as_a_domain_finding_without_writing() {
    let f = Fixture::new();
    let plate = f.plate();
    f.tamper(|name, layer| {
        if name == "authored.usda" {
            layer.replacen("custom token bim:geometryMode", "custom string bim:isDeactivated = \"hidden\"\n            custom token bim:geometryMode", 1)
        } else { layer }
    });
    let before = f.head();
    let (code, reply) = f.run("copy-element", &["--id", &plate]);
    assert_eq!(code, 2);
    assert_eq!(text(&reply, "code"), "INVALID_ARGUMENT");
    let finding = &list(&reply, "diagnostics")[0];
    assert_eq!(text(finding, "rule_id"), "DV-BASE-004");
    assert_eq!(f.head(), before);
}
