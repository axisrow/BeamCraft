// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates and beams, MPL-2.0. Expected values are closed forms.

//! P08 end to end: a move (`update-element --translation`) rewrites the placement alone and never
//! copies or rebuilds the body; `delete-element` removes the whole element from both layers, keeps
//! everything else, and is refused while another element still links to it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

mod support;

use support::{at, text};

use beamcraft_host::{
    ids::IdSource,
    revision::{ExpectedBase, LayerEntry, NoFault, RevisionDraft, RevisionStore, Uuid7},
    storage::BlobStore,
};
use beamcraft_json::Json;

struct Fixture {
    shared: support::StoreFixture,
}
impl Fixture {
    fn new() -> Self {
        Self {
            shared: support::StoreFixture::new("p08-tests"),
        }
    }
    fn revisions(&self) -> RevisionStore {
        RevisionStore::new(BlobStore::new(self.shared.root.join("store")).unwrap())
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
        assert_eq!(code, 2, "{command} {extra:?}: {body:?}");
        text(&body, "code")
    }
    fn definition(&self, command: &str, extra: &[&str]) -> String {
        text(self.ok(command, extra).get("definition").unwrap(), "digest")
    }
    fn plate(&self, extra: &[&str]) -> String {
        let mut args = vec!["--width", "1000", "--length", "500", "--thickness", "10"];
        args.extend_from_slice(extra);
        text(
            at(&self.ok("create-plate", &args), &["plate"]),
            "element_id",
        )
    }
    fn beam(&self, profile: &str, extra: &[&str]) -> String {
        let mut args = vec![
            "--start",
            "0,0,0",
            "--end",
            "3000,0,0",
            "--profile-digest",
            profile,
        ];
        args.extend_from_slice(extra);
        text(
            at(&self.ok("create-member", &args), &["member"]),
            "element_id",
        )
    }
    fn inspected(&self) -> Json {
        self.ok("inspect-elements", &[])
    }
    /// Commits a head-based revision with changed layer text and no content verification, the way
    /// a layer edited outside BeamCraft would arrive.
    fn tamper(&self, edit: impl Fn(&str, String) -> String) {
        let store = self.revisions();
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

fn num(json: &Json, path: &[&str]) -> f64 {
    at(json, path).float().unwrap()
}
fn list<'a>(json: &'a Json, key: &str) -> &'a [Json] {
    at(json, &[key]).array().unwrap()
}
fn find<'a>(items: &'a [Json], id: &str) -> &'a Json {
    items.iter().find(|e| text(e, "element_id") == id).unwrap()
}
fn strings(json: &Json, key: &str) -> Vec<String> {
    list(json, key)
        .iter()
        .map(|v| v.string().unwrap().to_owned())
        .collect()
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
fn a_move_rewrites_the_placement_alone_for_a_plate_and_a_beam() {
    let f = Fixture::new();
    let steel = f.definition("define-material", &["--density", "7850"]);
    let profile = f.definition("define-profile", &["--width", "100", "--height", "50"]);
    let plate = f.plate(&[
        "--material-digest",
        &steel,
        "--hole-rect",
        "100,100,100,100",
    ]);
    let beam = f.beam(&profile, &["--material-digest", &steel]);
    let before = f.inspected();
    let plate_before = find(list(&before, "plates"), &plate).clone();
    let beam_before = find(list(&before, "members"), &beam).clone();

    let moved = f.ok(
        "update-element",
        &["--id", &plate, "--translation", "100,200,300"],
    );
    assert_eq!(text(&moved, "outcome"), "element_updated");
    assert_eq!(at(&moved, &["effect", "placement"]), &Json::Bool(true));
    assert_eq!(at(&moved, &["effect", "shape"]), &Json::Bool(false));
    // Only the authored layer changes: the mesh and the stored quantity records are not rebuilt.
    assert_eq!(strings(&moved, "changed_layers"), ["authored.usda"]);
    assert_eq!(translation(at(&moved, &["element"])), [100.0, 200.0, 300.0]);
    let mut written: Vec<String> = list(at(&moved, &["footprint"]), "writes")
        .iter()
        .map(|e| text(e, "group"))
        .collect();
    written.sort();
    assert_eq!(written, ["derived", "placement"]);

    let after = f.inspected();
    let plate_after = find(list(&after, "plates"), &plate);
    assert_eq!(translation(plate_after), [100.0, 200.0, 300.0]);
    for key in ["mesh_digest", "name"] {
        assert_eq!(text(plate_after, key), text(&plate_before, key));
    }
    assert_eq!(
        num(plate_after, &["quantities", "volume", "mm3"]),
        num(&plate_before, &["quantities", "volume", "mm3"])
    );
    assert_eq!(num(plate_after, &["quantities", "mass", "kg"]), 38.465);
    for quantity in ["volume", "mass"] {
        assert_eq!(
            text(
                at(plate_after, &["stored_quantities", quantity]),
                "freshness"
            ),
            "current"
        );
    }
    // The beam is untouched, and can be moved the same way.
    assert_eq!(find(list(&after, "members"), &beam), &beam_before);
    let moved = f.ok(
        "update-element",
        &["--id", &beam, "--translation", "-5,0.5,7"],
    );
    assert_eq!(strings(&moved, "changed_layers"), ["authored.usda"]);
    let beam_after = find(list(&f.inspected(), "members"), &beam).clone();
    assert_eq!(translation(&beam_after), [-5.0, 0.5, 7.0]);
    assert_eq!(
        text(&beam_after, "mesh_digest"),
        text(&beam_before, "mesh_digest")
    );
}

#[test]
fn a_move_has_its_error_codes_and_replays_by_operation() {
    let f = Fixture::new();
    let plate = f.plate(&["--translation", "1,2,3"]);
    let op = ["--op-id", "01890a5d-ac96-774b-b9cc-0000000000a1"];
    let mut args = vec!["--id", plate.as_str(), "--translation", "10,20,30"];
    args.extend_from_slice(&op);
    let first = f.ok("update-element", &args);
    assert_eq!(text(&first, "outcome"), "element_updated");
    let replay = f.ok("update-element", &args);
    assert_eq!(text(&replay, "outcome"), "element_update_replayed");
    assert_eq!(
        f.rejected(
            "update-element",
            &["--id", &plate, "--translation", "11,20,30", op[0], op[1]]
        ),
        "OP_ID_REUSE"
    );
    // Setting the placement it already has changes nothing.
    assert_eq!(
        f.rejected(
            "update-element",
            &["--id", &plate, "--translation", "10,20,30"]
        ),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.rejected(
            "update-element",
            &["--id", &plate, "--translation", "NaN,0,0"]
        ),
        "NON_FINITE_NUMBER"
    );
    assert_eq!(
        f.rejected(
            "update-element",
            &["--id", &plate, "--translation", "2000000000,0,0"]
        ),
        "LIMIT_EXCEEDED"
    );
    assert_eq!(
        f.rejected("update-element", &["--id", &plate, "--translation", "1,2"]),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.rejected(
            "update-element",
            &[
                "--id",
                "01890a5d-ac96-774b-b9cc-0000000000ff",
                "--translation",
                "1,2,3"
            ]
        ),
        "NOT_FOUND"
    );
    let after = f.inspected();
    assert_eq!(
        translation(find(list(&after, "plates"), &plate)),
        [10.0, 20.0, 30.0]
    );
}

#[test]
fn a_delete_removes_the_whole_element_and_keeps_everything_else() {
    let f = Fixture::new();
    let steel = f.definition("define-material", &["--density", "7850"]);
    let profile = f.definition("define-profile", &["--width", "100", "--height", "50"]);
    let plate = f.plate(&[
        "--material-digest",
        &steel,
        "--hole-rect",
        "100,100,100,100",
    ]);
    let beam = f.beam(&profile, &["--material-digest", &steel]);
    let before = f.inspected();
    let beam_before = find(list(&before, "members"), &beam).clone();

    let op = ["--op-id", "01890a5d-ac96-774b-b9cc-0000000000b1"];
    let mut args = vec!["--id", plate.as_str()];
    args.extend_from_slice(&op);
    let deleted = f.ok("delete-element", &args);
    assert_eq!(text(&deleted, "outcome"), "element_deleted");
    assert_eq!(text(&deleted, "kind"), "plate");
    let mut changed = strings(&deleted, "changed_layers");
    changed.sort();
    assert_eq!(changed, ["authored.usda", "derived.usda"]);
    // The footprint writes every group of the deleted element and nothing else.
    let writes = list(at(&deleted, &["footprint"]), "writes");
    assert_eq!(writes.len(), 7);
    assert!(writes.iter().all(|e| text(e, "target") == plate));
    let reads: Vec<String> = list(at(&deleted, &["footprint"]), "reads")
        .iter()
        .map(|e| text(e, "group"))
        .collect();
    assert!(reads.contains(&"lifecycle".to_owned()) && reads.contains(&"relationship".to_owned()));

    // The plate is gone from the model, the beam is exactly as it was, and the definitions stay.
    let after = f.inspected();
    assert!(list(&after, "plates").is_empty());
    assert_eq!(find(list(&after, "members"), &beam), &beam_before);
    let definitions = f.ok("definitions", &[]);
    assert_eq!(list(&definitions, "definitions").len(), 2);
    assert!(
        list(&definitions, "definitions")
            .iter()
            .all(|d| d.get("digest").is_ok())
    );

    // A repeat of the same operation is a replay; the same operation for another element is a
    // reuse; an element that is gone is not found.
    let replay = f.ok("delete-element", &args);
    assert_eq!(text(&replay, "outcome"), "element_delete_replayed");
    assert_eq!(
        f.rejected("delete-element", &["--id", &beam, op[0], op[1]]),
        "OP_ID_REUSE"
    );
    assert_eq!(f.rejected("delete-element", &["--id", &plate]), "NOT_FOUND");
    assert_eq!(
        f.rejected("delete-element", &["--id", "not-a-uuid"]),
        "INVALID_ID"
    );

    // The last element can go too, and the empty model is still a valid, readable revision.
    f.ok("delete-element", &["--id", &beam]);
    let empty = f.inspected();
    assert!(list(&empty, "plates").is_empty() && list(&empty, "members").is_empty());
    let again = f.plate(&["--material-digest", &steel]);
    assert_eq!(list(&f.inspected(), "plates").len(), 1);
    assert_ne!(again, plate);
}

#[test]
fn a_delete_is_refused_while_another_element_links_to_it() {
    let f = Fixture::new();
    let profile = f.definition("define-profile", &["--width", "100", "--height", "50"]);
    let plate = f.plate(&[]);
    let beam = f.beam(&profile, &[]);
    // The plate gets a relationship to the beam, as a later command or another tool would add.
    let hex = |id: &str| id.replace('-', "");
    f.tamper(|name, mut text| {
        if name == "authored.usda" {
            let marker = format!("\"PRT_{}\"", hex(&plate));
            let start = text.find(&marker).unwrap();
            let brace = start + text[start..].find('{').unwrap() + 1;
            let link = format!(
                "\ncustom rel bim:test:supports = </World/Steel/Parts/PRT_{}>\n",
                hex(&beam)
            );
            text.insert_str(brace, &link);
        }
        text
    });
    let head = f.inspected();
    let pin = |json: &Json| text(at(json, &["snapshot"]), "pin_revision");
    let pinned = pin(&head);

    // The beam is the link target: no cascade, no dangling link, nothing is written.
    assert_eq!(
        f.rejected("delete-element", &["--id", &beam]),
        "DEPENDENCY_CONFLICT"
    );
    assert_eq!(pin(&f.inspected()), pinned);
    assert_eq!(list(&f.inspected(), "members").len(), 1);

    // The plate is not a target of any link, so it can go; that also removes the link.
    f.ok("delete-element", &["--id", &plate]);
    f.ok("delete-element", &["--id", &beam]);
    assert!(list(&f.inspected(), "members").is_empty());
}
