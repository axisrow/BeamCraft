// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, beams and assemblies, MPL-2.0.

//! A01 end to end: `create-assembly`, `attach-to-assembly`, `detach-from-assembly` and `assemblies`
//! through the real CLI binary and committed revisions. An assembly has its own identity and no
//! body; the `part-of` record lives with the part; kinds, cardinality, self-links and cycles are
//! refused with nothing written; an assembly with parts cannot be deleted and a part with a
//! record cannot be copied.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

mod support;

use beamcraft_host::{
    ids::IdSource,
    revision::{ExpectedBase, LayerEntry, NoFault, RevisionDraft, RevisionStore, Uuid7},
    storage::BlobStore,
};
use beamcraft_json::Json;

const FRAME: &str = "019a7bf0-0000-7000-8000-000000200001";
const OUTER: &str = "019a7bf0-0000-7000-8000-000000200002";
const OTHER: &str = "019a7bf0-0000-7000-8000-000000200003";
const MAIN_OP: &str = "019a7bf0-0000-7000-8000-00000000c001";
const USER: &str = "01890a5d-ac96-774b-b9cc-aaaaaaaaaaaa";

struct Fixture {
    shared: support::StoreFixture,
}
impl Fixture {
    fn new() -> Self {
        Self {
            shared: support::StoreFixture::new("a01-tests"),
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
    fn assembly(&self, id: &str, name: &str) -> Json {
        self.ok("create-assembly", &["--name", name, "--new-id", id])
    }
    fn attach(&self, part: &str, assembly: &str) -> Json {
        self.ok(
            "attach-to-assembly",
            &["--id", part, "--assembly-id", assembly],
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
    fn assemblies(&self) -> Vec<Json> {
        list(&self.ok("assemblies", &[]), "assemblies").to_vec()
    }

    /// Runs a command against another store root (a workspace draft), always through the binary.
    fn run_in(&self, store: &str, command: &str, extra: &[&str]) -> (i32, Json) {
        support::StoreFixture::run_in(store, command, extra)
    }
    fn ok_in(&self, store: &str, command: &str, extra: &[&str]) -> Json {
        let (code, body) = self.run_in(store, command, extra);
        assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
        body
    }
    /// Runs a command that takes no `--store` (the workspace commands take a directory).
    fn ok_plain(&self, args: &[&str]) -> Json {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(args)
            .output()
            .unwrap();
        let body =
            beamcraft_json::decode(&output.stdout, &beamcraft_json::Limits::DEFAULT).unwrap();
        assert_eq!(output.status.code().unwrap(), 0, "{args:?}: {body:?}");
        body
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
fn assembly<'a>(items: &'a [Json], id: &str) -> &'a Json {
    items.iter().find(|a| text(a, "id") == id).unwrap()
}
fn groups(change: &Json, key: &str) -> Vec<(String, String)> {
    list(at(change, &["footprint"]), key)
        .iter()
        .map(|e| (text(e, "target"), text(e, "group")))
        .collect()
}
fn number(json: &Json, key: &str) -> u64 {
    at(json, &[key]).unsigned().unwrap()
}

#[test]
fn create_attach_nest_and_detach_write_only_the_part() {
    let f = Fixture::new();
    let plate = f.plate();
    let beam = f.beam();
    let mesh_before = text(
        &f.ok("inspect-elements", &[])
            .get("plates")
            .unwrap()
            .array()
            .unwrap()[0],
        "mesh_digest",
    );

    let created = f.assembly(FRAME, "Frame A");
    assert_eq!(text(&created, "outcome"), "assembly_created");
    // An assembly is an element of its own: new identity, label, no body in the derived layer.
    assert_eq!(strings(&created, "changed_layers"), ["authored.usda"]);
    assert_eq!(text(&created, "element_id"), FRAME);
    assert_ne!(FRAME, plate);
    f.assembly(OUTER, "Outer");

    let attached = f.attach(&plate, FRAME);
    assert_eq!(text(&attached, "outcome"), "part_attached");
    assert_eq!(strings(&attached, "changed_layers"), ["authored.usda"]);
    // The record lives with its source: the part's relationship group is the only write; the
    // assembly is read (lifecycle and relationships) to decide kind and cycles.
    assert_eq!(
        groups(&attached, "writes"),
        [(plate.clone(), "relationship".to_owned())]
    );
    assert!(groups(&attached, "reads").contains(&(FRAME.to_owned(), "lifecycle".to_owned())));
    f.attach(&beam, FRAME);
    let nested = f.attach(FRAME, OUTER);
    assert_eq!(text(&nested, "outcome"), "part_attached");

    let all = f.assemblies();
    assert_eq!(all.len(), 2);
    let frame = assembly(&all, FRAME);
    assert_eq!(text(frame, "name"), "Frame A");
    assert_eq!(text(frame, "part_of"), OUTER);
    let mut parts = strings(frame, "parts");
    parts.sort();
    let mut expected = vec![plate.clone(), beam.clone()];
    expected.sort();
    assert_eq!(parts, expected);
    assert_eq!(strings(assembly(&all, OUTER), "parts"), [FRAME]);
    assert_eq!(at(assembly(&all, OUTER), &["part_of"]), &Json::Null);

    // Membership never touches the parts' bodies, and the verified head still inspects.
    let inspected = f.ok("inspect-elements", &[]);
    assert_eq!(
        text(
            &inspected.get("plates").unwrap().array().unwrap()[0],
            "mesh_digest"
        ),
        mesh_before
    );

    let detached = f.ok("detach-from-assembly", &["--id", &plate]);
    assert_eq!(text(&detached, "outcome"), "part_detached");
    assert_eq!(
        groups(&detached, "writes"),
        [(plate.clone(), "relationship".to_owned())]
    );
    let all = f.assemblies();
    assert_eq!(strings(assembly(&all, FRAME), "parts"), [beam]);
}

#[test]
fn kinds_cardinality_self_links_and_cycles_are_refused_with_nothing_written() {
    let f = Fixture::new();
    let plate = f.plate();
    let beam = f.beam();
    f.assembly(FRAME, "Frame");
    f.assembly(OUTER, "Outer");
    f.assembly(OTHER, "Other");
    f.attach(&plate, FRAME);
    f.attach(FRAME, OUTER);

    let attach = |part: &str, assembly: &str| {
        f.rejected(
            "attach-to-assembly",
            &["--id", part, "--assembly-id", assembly],
        )
    };
    // DV-REL-003: the target must be an assembly.
    assert_eq!(attach(&beam, &plate), "INVALID_ARGUMENT");
    // DV-REL-006: no self-link.
    assert_eq!(attach(OTHER, OTHER), "INVALID_ARGUMENT");
    // DV-REL-004: one assembly per part; no silent re-parenting.
    assert_eq!(attach(&plate, OTHER), "DEPENDENCY_CONFLICT");
    assert_eq!(attach(&plate, FRAME), "INVALID_ARGUMENT");
    // DV-REL-005: cycles of length two and three.
    assert_eq!(attach(OUTER, FRAME), "DEPENDENCY_CONFLICT");
    f.attach(OTHER, FRAME);
    assert_eq!(attach(OUTER, OTHER), "DEPENDENCY_CONFLICT");
    // Unknown identities.
    let missing = "019a7bf0-0000-7000-8000-0000002000ff";
    assert_eq!(attach(missing, FRAME), "NOT_FOUND");
    assert_eq!(attach(&beam, missing), "NOT_FOUND");
    assert_eq!(
        f.rejected("detach-from-assembly", &["--id", &beam]),
        "INVALID_ARGUMENT"
    );
    // The identity of a new assembly must be free, whatever the kind that holds it.
    assert_eq!(
        f.rejected("create-assembly", &["--name", "Again", "--new-id", &plate]),
        "DUPLICATE_ID"
    );
    assert_eq!(
        f.rejected("create-assembly", &["--name", "", "--new-id", missing]),
        "INVALID_ARGUMENT"
    );
}

#[test]
fn replay_reuse_delete_and_copy_follow_the_part_of_records() {
    let f = Fixture::new();
    let plate = f.plate();
    let create_op = "019a7bf0-0000-7000-8000-00000000a001";
    let attach_op = "019a7bf0-0000-7000-8000-00000000a002";
    let first = f.ok(
        "create-assembly",
        &["--name", "Frame", "--new-id", FRAME, "--op-id", create_op],
    );
    let again = f.ok(
        "create-assembly",
        &["--name", "Frame", "--new-id", FRAME, "--op-id", create_op],
    );
    assert_eq!(text(&again, "outcome"), "assembly_created_replayed");
    assert_eq!(
        text(at(&again, &["revision"]), "revision_id"),
        text(at(&first, &["revision"]), "revision_id")
    );
    assert_eq!(
        f.rejected(
            "create-assembly",
            &["--name", "Renamed", "--new-id", FRAME, "--op-id", create_op],
        ),
        "OP_ID_REUSE"
    );
    let args = [
        "--id",
        plate.as_str(),
        "--assembly-id",
        FRAME,
        "--op-id",
        attach_op,
    ];
    f.ok("attach-to-assembly", &args);
    let replay = f.ok("attach-to-assembly", &args);
    assert_eq!(text(&replay, "outcome"), "part_attached_replayed");
    assert_eq!(at(&replay, &["footprint"]), &Json::Null);

    // DV-REL-009: an assembly with parts is not deleted, and there is no cascade.
    assert_eq!(
        f.rejected("delete-element", &["--id", FRAME]),
        "DEPENDENCY_CONFLICT"
    );
    // DV-REL-011: the copy is loose; the source remains in the assembly.
    let copied = f.ok("copy-element", &["--id", &plate]);
    let copied_id = text(at(&copied, &["element"]), "element_id");
    assert_ne!(copied_id, plate);
    assert_eq!(strings(&f.assemblies()[0], "parts"), vec![plate.clone()]);
    // Deleting the part removes its record with it; then the empty assembly can go.
    f.ok("delete-element", &["--id", &plate]);
    assert!(strings(&f.assemblies()[0], "parts").is_empty());
    let deleted = f.ok("delete-element", &["--id", FRAME]);
    assert_eq!(text(&deleted, "kind"), "assembly");
    assert!(f.assemblies().is_empty());
}

#[test]
fn a_cycle_stored_outside_beamcraft_is_refused_when_read() {
    let f = Fixture::new();
    f.assembly(FRAME, "Frame");
    f.assembly(OUTER, "Outer");
    f.attach(FRAME, OUTER);
    // OUTER -> FRAME closes a two-cycle that no command would author.
    let outer = format!("ASM_{}", OUTER.replace('-', ""));
    let frame_path = format!("/World/Steel/Assemblies/ASM_{}", FRAME.replace('-', ""));
    f.tamper(|name, text| {
        if name != "authored.usda" {
            return text;
        }
        let at = text.find(&format!("\"{outer}\"")).unwrap();
        let open = at + text[at..].find('{').unwrap() + 1;
        format!(
            "{}\n        custom rel bim:partOf = <{frame_path}>{}",
            &text[..open],
            &text[open..]
        )
    });
    let (code, body) = f.run("assemblies", &[]);
    assert_eq!(code, 2, "{body:?}");
    assert_eq!(text(&body, "code"), "DEPENDENCY_CONFLICT");
}

/// An operation id that committed one request is not a replay of another request whose result
/// the head happens to hold through a different operation (#145).
#[test]
fn a_different_request_under_an_accepted_op_id_is_reuse_not_a_replay() {
    let f = Fixture::new();
    let op = |n: u32| format!("019a7bf0-0000-7000-8000-00000000b{n:03}");
    let name_args = |name: &'static str, id: &'static str, op: String| {
        move || {
            vec![
                "--name".to_owned(),
                name.to_owned(),
                "--new-id".to_owned(),
                id.to_owned(),
                "--op-id".to_owned(),
                op.clone(),
            ]
        }
    };
    let run_create = |args: Vec<String>| {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        f.run("create-assembly", &refs)
    };
    let (code, _) = run_create(name_args("A", FRAME, op(1))());
    assert_eq!(code, 0);
    let (code, _) = run_create(name_args("B", OUTER, op(2))());
    assert_eq!(code, 0);
    let before = f.head();
    // B exists in the head, but operation 1 created A.
    let (code, body) = run_create(name_args("B", OUTER, op(1))());
    assert_eq!((code, text(&body, "code").as_str()), (2, "OP_ID_REUSE"));
    assert_eq!(f.head(), before);
    // The original request still replays.
    let (code, body) = run_create(name_args("A", FRAME, op(1))());
    assert_eq!(code, 0);
    assert_eq!(text(&body, "outcome"), "assembly_created_replayed");

    // The same holds for attach and detach.
    let (one, two) = (f.plate(), f.plate());
    let attach = |part: &str, o: String| {
        f.run(
            "attach-to-assembly",
            &["--id", part, "--assembly-id", FRAME, "--op-id", &o],
        )
    };
    assert_eq!(attach(&one, op(3)).0, 0);
    assert_eq!(attach(&two, op(4)).0, 0);
    let reused = f.rejected(
        "attach-to-assembly",
        &["--id", &two, "--assembly-id", FRAME, "--op-id", &op(3)],
    );
    assert_eq!(reused, "OP_ID_REUSE");
    let (code, body) = attach(&one, op(3));
    assert_eq!(code, 0);
    assert_eq!(text(&body, "outcome"), "part_attached_replayed");

    let detach =
        |part: &str, o: String| f.run("detach-from-assembly", &["--id", part, "--op-id", &o]);
    assert_eq!(detach(&one, op(5)).0, 0);
    assert_eq!(detach(&two, op(6)).0, 0);
    let reused = f.rejected("detach-from-assembly", &["--id", &two, "--op-id", &op(5)]);
    assert_eq!(reused, "OP_ID_REUSE");
    let (code, body) = detach(&one, op(5));
    assert_eq!(code, 0);
    assert_eq!(text(&body, "outcome"), "part_detached_replayed");
}

/// A16.1: the main part is stored on the assembly, is one of its direct parts, and guards its
/// own detach and delete until another main part is named.
#[test]
fn the_main_part_is_stored_on_the_assembly_and_guards_detach_and_delete() {
    let f = Fixture::new();
    let plate = f.plate();
    let beam = f.beam();
    f.assembly(FRAME, "Frame");
    f.attach(&plate, FRAME);
    f.attach(&beam, FRAME);

    // The record lives on the assembly: only the assembly's relationship group is written.
    let set = f.ok(
        "set-main-part",
        &["--id", &beam, "--assembly-id", FRAME, "--op-id", MAIN_OP],
    );
    assert_eq!(text(&set, "outcome"), "main_part_set");
    assert_eq!(
        groups(&set, "writes"),
        [(FRAME.to_owned(), "relationship".to_owned())]
    );
    assert_eq!(text(assembly(&f.assemblies(), FRAME), "main"), beam);
    let replay = f.ok(
        "set-main-part",
        &["--id", &beam, "--assembly-id", FRAME, "--op-id", MAIN_OP],
    );
    assert_eq!(text(&replay, "outcome"), "main_part_set_replayed");
    assert_eq!(at(&replay, &["footprint"]), &Json::Null);

    // A foreign part cannot be main: not a member at all, or a member of another assembly.
    let foreign = f.plate();
    assert_eq!(
        f.rejected("set-main-part", &["--id", &foreign, "--assembly-id", FRAME]),
        "INVALID_ARGUMENT"
    );
    f.assembly(OUTER, "Outer");
    f.attach(&foreign, OUTER);
    assert_eq!(
        f.rejected("set-main-part", &["--id", &foreign, "--assembly-id", FRAME]),
        "INVALID_ARGUMENT"
    );
    let missing = "019a7bf0-0000-7000-8000-0000002000ff";
    assert_eq!(
        f.rejected("set-main-part", &["--id", missing, "--assembly-id", FRAME]),
        "NOT_FOUND"
    );
    assert_eq!(
        f.rejected("set-main-part", &["--id", &beam, "--assembly-id", missing]),
        "NOT_FOUND"
    );
    // The same main part again is no change.
    assert_eq!(
        f.rejected("set-main-part", &["--id", &beam, "--assembly-id", FRAME]),
        "INVALID_ARGUMENT"
    );

    // Detaching the main part is refused until another main part is named; then it works.
    assert_eq!(
        f.rejected("detach-from-assembly", &["--id", &beam]),
        "DEPENDENCY_CONFLICT"
    );
    f.ok("set-main-part", &["--id", &plate, "--assembly-id", FRAME]);
    assert_eq!(text(assembly(&f.assemblies(), FRAME), "main"), plate);
    f.ok("detach-from-assembly", &["--id", &beam]);

    // A delete cannot remove the main part either.
    f.attach(&beam, FRAME);
    f.ok("set-main-part", &["--id", &beam, "--assembly-id", FRAME]);
    assert_eq!(
        f.rejected("delete-element", &["--id", &beam]),
        "DEPENDENCY_CONFLICT"
    );
    // An assembly that is itself a main part of its parent cannot be deleted while it has
    // parts, and the record dies with the assembly on an explode.
    f.attach(FRAME, OUTER);
    f.ok("set-main-part", &["--id", FRAME, "--assembly-id", OUTER]);
    assert_eq!(
        f.rejected("delete-element", &["--id", FRAME]),
        "DEPENDENCY_CONFLICT"
    );
    // Exploding the assembly that is the parent's main part is refused the same way, while
    // exploding the parent is allowed: its record dies with it and the sub-assembly stays.
    assert_eq!(
        f.rejected("explode-assembly", &["--id", FRAME]),
        "DEPENDENCY_CONFLICT"
    );
    f.ok("explode-assembly", &["--id", OUTER]);
    let all = f.assemblies();
    assert_eq!(all.len(), 1);
    let frame = assembly(&all, FRAME);
    assert_eq!(at(frame, &["part_of"]), &Json::Null);
    assert_eq!(text(frame, "main"), beam);
}

/// A16: explode dissolves one level; the parts do not change and the undo brings the assembly
/// back byte for byte. The whole scenario runs in a workspace draft, the only place an undo
/// exists.
#[test]
fn explode_dissolves_one_level_and_its_undo_restores_it() {
    let f = Fixture::new();
    let plate = f.plate();
    let beam = f.beam();
    f.assembly(FRAME, "Frame");
    f.attach(&plate, FRAME);
    f.attach(&beam, FRAME);
    f.assembly(OUTER, "Outer");
    f.attach(FRAME, OUTER);
    f.ok("set-main-part", &["--id", &plate, "--assembly-id", FRAME]);
    let mesh_before = text(
        &f.ok("inspect-elements", &[])
            .get("plates")
            .unwrap()
            .array()
            .unwrap()[0],
        "mesh_digest",
    );

    let published = f.shared.root.join("store").to_str().unwrap().to_owned();
    let ws = f.shared.root.join("ws").to_str().unwrap().to_owned();
    f.ok_in(
        &published,
        "create-workspace",
        &["--workspace-dir", &ws, "--user-id", USER],
    );

    // Exploding the outer level leaves the inner assembly an assembly.
    let exploded = f.ok_in(&ws, "explode-assembly", &["--id", OUTER]);
    assert_eq!(text(&exploded, "outcome"), "assembly_exploded");
    let ws_head = f.ok_in(&ws, "assemblies", &[]);
    let all = list(&ws_head, "assemblies");
    assert_eq!(all.len(), 1);
    let frame = assembly(all, FRAME);
    assert_eq!(at(frame, &["part_of"]), &Json::Null);
    let mut parts = strings(frame, "parts");
    parts.sort();
    let mut expected = vec![plate.clone(), beam.clone()];
    expected.sort();
    assert_eq!(parts, expected);
    assert_eq!(text(frame, "main"), plate);

    // Exploding the frame itself leaves the parts loose and unchanged.
    let exploded = f.ok_in(&ws, "explode-assembly", &["--id", FRAME]);
    assert_eq!(text(&exploded, "outcome"), "assembly_exploded");
    assert!(list(&f.ok_in(&ws, "assemblies", &[]), "assemblies").is_empty());
    let inspected = f.ok_in(&ws, "inspect-elements", &[]);
    assert_eq!(
        text(
            &inspected.get("plates").unwrap().array().unwrap()[0],
            "mesh_digest"
        ),
        mesh_before
    );
    // Nothing is left to explode; the refusal writes nothing.
    let missing = "019a7bf0-0000-7000-8000-0000002000ff";
    let (code, body) = f.run_in(&ws, "explode-assembly", &["--id", missing]);
    assert_eq!((code, text(&body, "code").as_str()), (2, "NOT_FOUND"));

    // Each undo restores one unit (one explode). The first brings the frame back, the second
    // also the outer assembly with its records and the main part.
    f.ok_plain(&["undo-workspace", "--workspace-dir", &ws]);
    let half_head = f.ok_in(&ws, "assemblies", &[]);
    let half = list(&half_head, "assemblies");
    assert_eq!(half.len(), 1);
    let frame = assembly(half, FRAME);
    assert_eq!(at(frame, &["part_of"]), &Json::Null);
    assert_eq!(text(frame, "main"), plate);
    f.ok_plain(&["undo-workspace", "--workspace-dir", &ws]);
    let restored_head = f.ok_in(&ws, "assemblies", &[]);
    let restored = list(&restored_head, "assemblies");
    assert_eq!(restored.len(), 2);
    let frame = assembly(restored, FRAME);
    assert_eq!(text(frame, "part_of"), OUTER);
    assert_eq!(text(frame, "main"), plate);
    let inspected = f.ok_in(&ws, "inspect-elements", &[]);
    assert_eq!(
        text(
            &inspected.get("plates").unwrap().array().unwrap()[0],
            "mesh_digest"
        ),
        mesh_before
    );
}

/// A16 acceptance: numbering and the bill of materials count the parts of a nested assembly
/// exactly once — the inner assembly's parts belong to it alone, the outer assembly's own
/// part to the outer one alone.
#[test]
fn numbering_and_bom_count_nested_parts_once() {
    let f = Fixture::new();
    let plate = f.plate();
    let beam = f.beam();
    f.assembly(FRAME, "Frame");
    f.attach(&plate, FRAME);
    f.attach(&beam, FRAME);
    f.assembly(OUTER, "Outer");
    // A part of a different size, so the two assemblies cannot share a mark.
    let extra = text(
        at(
            &f.ok(
                "create-plate",
                &["--width", "700", "--length", "400", "--thickness", "12"],
            ),
            &["plate"],
        ),
        "element_id",
    );
    f.attach(&extra, OUTER);
    f.attach(FRAME, OUTER);

    let preview = f.ok("preview-numbering", &[]);
    assert_eq!(text(&preview, "outcome"), "numbering_preview");
    // Three parts exist and each is counted once, whatever its nesting depth.
    assert_eq!(number(at(&preview, &["counts"]), "parts"), 3);
    let assemblies: Vec<_> = list(&preview, "groups")
        .iter()
        .filter(|g| text(g, "kind") == "assembly")
        .collect();
    assert_eq!(assemblies.len(), 2);

    let elements = format!("{}/bom-elements.csv", f.shared.root.to_str().unwrap());
    let summary = format!("{}/bom-summary.csv", f.shared.root.to_str().unwrap());
    let exported = f.ok("export-bom", &["--out", &elements, "--summary", &summary]);
    assert_eq!(at(&exported, &["elements"]).float(), Ok(3.0));
    let csv = std::fs::read_to_string(&elements).unwrap();
    // The mark is the element id until numbering is approved, so every id appears twice.
    for id in [&plate, &beam, &extra] {
        assert_eq!(csv.matches(id).count(), 2, "{id}");
    }
    assert!(!csv.contains(FRAME));
}
