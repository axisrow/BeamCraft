// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, members and assemblies, MPL-2.0. The independent
// `beamcraft-usd-oracle` process is used as the external test reader: it is not a dependency
// and shares no code with the layer engine.

//! A09 end to end through the real CLI binary: `approve-numbering` stores the approved marks as
//! authored data through the normal revision path, `inspect-numbering` and `preview-numbering` read
//! them back, a stale token, overlapping series and an empty model are refused with nothing
//! written, fabrication edits make the approval stale while a move and a rename do not, and an
//! undo restores the earlier marks.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use crate::support;

use support::{Scripted, at, text};

use beamcraft_host::{
    ids::IdSource,
    revision::{ExpectedBase, LayerEntry, NoFault, RevisionDraft, RevisionStore, Uuid7},
    storage::BlobStore,
};
use beamcraft_json::{Json, Limits};
use std::collections::BTreeMap;
use std::fs;
use std::process::Command;

const USER: &str = "01890a5d-ac96-774b-b9cc-aaaaaaaaaaaa";

struct Fixture {
    shared: support::StoreFixture,
}

struct Scene {
    plates: Vec<String>,
    beams: Vec<String>,
}

impl Fixture {
    fn new() -> Self {
        Self {
            shared: support::StoreFixture::new("a09-tests"),
        }
    }
    fn store(&self) -> String {
        self.shared.root.join("store").to_str().unwrap().to_owned()
    }
    fn dir(&self, name: &str) -> String {
        self.shared.root.join(name).to_str().unwrap().to_owned()
    }
    fn run_in(&self, store: &str, command: &str, extra: &[&str]) -> (i32, Json) {
        let mut args = vec![command, "--store", store];
        args.extend_from_slice(extra);
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(&args)
            .output()
            .unwrap();
        let body = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
        (output.status.code().unwrap(), body)
    }
    fn run(&self, command: &str, extra: &[&str]) -> (i32, Json) {
        self.run_in(&self.store(), command, extra)
    }
    /// One `run-script` invocation over the fixture store: the same commands the suite would
    /// spawn one process each, amortized into a single process (#738 P7; each command's own
    /// work is unchanged). Replies come back in the lines' order.
    fn script(&self, lines: &[Scripted]) -> Vec<Json> {
        let store = self.store();
        support::script_lines(&self.shared.root, &store, lines)
    }
    fn ok(&self, command: &str, extra: &[&str]) -> Json {
        let (code, body) = self.run(command, extra);
        assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
        body
    }
    fn ok_in(&self, store: &str, command: &str, extra: &[&str]) -> Json {
        let (code, body) = self.run_in(store, command, extra);
        assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
        body
    }
    /// Runs a command that must be refused and returns its code; no revision may be written.
    /// The layers read bracketing the command and the command itself ride one process
    /// (#738 P7), so the nothing-written proof keeps both reads.
    fn refused(&self, command: &str, extra: &[&str]) -> String {
        let store = self.store();
        let mut line = vec![command, "--store", store.as_str()];
        line.extend_from_slice(extra);
        let reads = self.script(&[
            Scripted::ok(&["open-revision", "--store", &store]),
            Scripted::refused(&line),
            Scripted::ok(&["open-revision", "--store", &store]),
        ]);
        let body = &reads[1];
        assert_eq!(text(body, "outcome"), "rejected", "{command} {extra:?}");
        assert_eq!(
            at(&reads[0], &["layers"]),
            at(&reads[2], &["layers"]),
            "{command} {extra:?} wrote"
        );
        text(body, "code")
    }
    fn preview(&self, extra: &[&str]) -> Json {
        self.ok("preview-numbering", extra)
    }
    fn token(&self, extra: &[&str]) -> String {
        text(&self.preview(extra), "preview_token")
    }
    fn approve(&self, token: &str, extra: &[&str]) -> Json {
        let mut args = vec!["--preview-token", token];
        args.extend_from_slice(extra);
        self.ok("approve-numbering", &args)
    }
    fn status(&self) -> Json {
        self.ok("inspect-numbering", &[])
    }
    fn state(&self) -> String {
        text(at(&self.status(), &["approval"]), "state")
    }
    fn layers(&self) -> Vec<(String, String)> {
        match self.run("open-revision", &[]) {
            (0, body) => list(&body, "layers")
                .iter()
                .map(|l| (text(l, "name"), text(l, "digest")))
                .collect(),
            _ => Vec::new(),
        }
    }
    fn plate(&self, material: &str, thickness: &str, extra: &[&str]) -> String {
        let mut args = vec![
            "--width",
            "400",
            "--length",
            "300",
            "--thickness",
            thickness,
            "--material-digest",
            material,
        ];
        args.extend_from_slice(extra);
        text(
            at(&self.ok("create-plate", &args), &["plate"]),
            "element_id",
        )
    }
    /// The `create-plate` line over `store`, the exact tokens the per-command path spawned.
    fn plate_line(store: &str, thickness: &str, material: &str, extra: &[&str]) -> Scripted {
        let mut args = vec![
            "create-plate",
            "--store",
            store,
            "--width",
            "400",
            "--length",
            "300",
            "--thickness",
            thickness,
            "--material-digest",
            material,
        ];
        args.extend_from_slice(extra);
        Scripted::ok(&args)
    }
    /// The `create-member` line over `store`, the exact tokens the per-command path spawned.
    fn beam_line(
        store: &str,
        profile: &str,
        material: &str,
        end: &str,
        translation: &str,
    ) -> Scripted {
        Scripted::ok(&[
            "create-member",
            "--store",
            store,
            "--start",
            "0,0,0",
            "--end",
            end,
            "--profile-digest",
            profile,
            "--material-digest",
            material,
            "--translation",
            translation,
        ])
    }
    /// Plates 0, 1 (identical, one moved), 2 (other material), 3 (a hole), 4 (thicker); beams
    /// 0, 1 (identical, one moved), 2 (other length). The definitions and the parts each ride
    /// one process (#738 P7); digests and ids come back in the replies' order.
    fn scene(&self) -> Scene {
        let store = self.store();
        let material = |store: &str, name: &str| {
            Scripted::ok(&[
                "define-material",
                "--store",
                store,
                "--density",
                "7850",
                "--name",
                name,
                "--grade",
                name,
            ])
        };
        let defined = self.script(&[
            material(&store, "S355"),
            material(&store, "S275"),
            Scripted::ok(&[
                "define-profile",
                "--store",
                &store,
                "--width",
                "100",
                "--height",
                "50",
            ]),
        ]);
        let steel = text(at(&defined[0], &["definition"]), "digest");
        let other_steel = text(at(&defined[1], &["definition"]), "digest");
        let profile = text(at(&defined[2], &["definition"]), "digest");
        let made = self.script(&[
            Self::plate_line(&store, "20", &steel, &[]),
            Self::plate_line(&store, "20", &steel, &["--translation", "1000,0,0"]),
            Self::plate_line(&store, "20", &other_steel, &[]),
            Self::plate_line(&store, "20", &steel, &["--hole-round", "100,100,40"]),
            Self::plate_line(&store, "25", &steel, &[]),
            Self::beam_line(&store, &profile, &steel, "3000,0,0", "0,0,0"),
            Self::beam_line(&store, &profile, &steel, "3000,0,0", "0,500,0"),
            Self::beam_line(&store, &profile, &steel, "3500,0,0", "0,1000,0"),
        ]);
        let id = |reply: &Json, key: &str| text(at(reply, &[key]), "element_id");
        Scene {
            plates: (0..5).map(|n| id(&made[n], "plate")).collect(),
            beams: (5..8).map(|n| id(&made[n], "member")).collect(),
        }
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
    fn authored_text(&self) -> String {
        let store = RevisionStore::new(BlobStore::new(self.shared.root.join("store")).unwrap());
        let head = store.open().unwrap();
        String::from_utf8(head.layer_bytes("authored.usda").unwrap()).unwrap()
    }
}

fn list<'a>(json: &'a Json, key: &str) -> &'a [Json] {
    at(json, &[key]).array().unwrap()
}
fn number(json: &Json, key: &str) -> u64 {
    json.get(key).unwrap().unsigned().unwrap()
}
fn is_null(json: &Json, key: &str) -> bool {
    matches!(json.get(key), Ok(Json::Null))
}
fn strings(json: &Json, key: &str) -> Vec<String> {
    list(json, key)
        .iter()
        .map(|v| v.string().unwrap().to_owned())
        .collect()
}
/// The `(name, digest)` pairs of the reply's layer manifest, as `layers()` reads them.
fn layers_of(reply: &Json) -> Vec<(String, String)> {
    list(reply, "layers")
        .iter()
        .map(|l| (text(l, "name"), text(l, "digest")))
        .collect()
}
/// The `derived.usda` entry of a layers manifest: the one layer an approval must not touch.
fn derived_layer(reply: &Json) -> (String, String) {
    layers_of(reply)
        .into_iter()
        .find(|(name, _)| name == "derived.usda")
        .unwrap()
}
/// The stored marks of an `inspect-numbering` reply, as `marks()` reads them.
fn marks_of(reply: &Json) -> BTreeMap<String, String> {
    list(reply, "marks")
        .iter()
        .map(|m| (text(m, "id"), text(m, "mark")))
        .collect()
}

#[test]
fn approval_stores_one_mark_per_group_as_authored_data_and_reads_back() {
    let f = Fixture::new();
    let s = f.scene();
    let store = f.store();
    // Two assemblies with the same content: A-1 for both. The creations and the attachments
    // each ride one process (#738 P7).
    let created = f.script(&[
        Scripted::ok(&["create-assembly", "--store", &store, "--name", "Frame 1"]),
        Scripted::ok(&["create-assembly", "--store", &store, "--name", "Frame 2"]),
    ]);
    let id = |reply: &Json| text(reply, "element_id");
    let (first, second) = (id(&created[0]), id(&created[1]));
    let attached = f.script(&[
        Scripted::ok(&[
            "attach-to-assembly",
            "--store",
            &store,
            "--id",
            &s.plates[0],
            "--assembly-id",
            &first,
        ]),
        Scripted::ok(&[
            "attach-to-assembly",
            "--store",
            &store,
            "--id",
            &s.beams[0],
            "--assembly-id",
            &first,
        ]),
        Scripted::ok(&[
            "attach-to-assembly",
            "--store",
            &store,
            "--id",
            &s.plates[1],
            "--assembly-id",
            &second,
        ]),
        Scripted::ok(&[
            "attach-to-assembly",
            "--store",
            &store,
            "--id",
            &s.beams[1],
            "--assembly-id",
            &second,
        ]),
    ]);
    for reply in &attached {
        assert_eq!(text(reply, "outcome"), "part_attached");
    }
    let frames = [first, second];
    // The reads before the approval ride one process: the approval state, the inspection the
    // mesh digests are compared against, the layers with their derived digest, and the token.
    let before = f.script(&[
        Scripted::ok(&["inspect-numbering", "--store", &store]),
        Scripted::ok(&["inspect-elements", "--store", &store]),
        Scripted::ok(&["open-revision", "--store", &store]),
        Scripted::ok(&["preview-numbering", "--store", &store]),
    ]);
    assert_eq!(text(at(&before[0], &["approval"]), "state"), "not_approved");
    let inspected = before[1].clone();
    let derived_before = derived_layer(&before[2]);
    let before_head = text(&before[2], "revision_id");
    let token = text(&before[3], "preview_token");
    // The approval and the reads that judge it ride one process: the derived digest must not
    // move, the marks read back, the preview sees the approval as current, and the inspected
    // bodies stay as they were.
    let after = f.script(&[
        Scripted::ok(&[
            "approve-numbering",
            "--store",
            &store,
            "--preview-token",
            &token,
        ]),
        Scripted::ok(&["open-revision", "--store", &store]),
        Scripted::ok(&["inspect-numbering", "--store", &store]),
        Scripted::ok(&["preview-numbering", "--store", &store, "--previous", &token]),
        Scripted::ok(&["inspect-elements", "--store", &store]),
    ]);
    let approved = &after[0];
    assert_eq!(text(approved, "outcome"), "numbering_approved");
    // 5 plates, 3 beams and 2 assemblies carry a mark; 4 plate, 2 beam and 1 assembly groups.
    assert_eq!(number(approved, "marks"), 10);
    assert_eq!(number(approved, "groups"), 7);
    assert_ne!(text(approved, "revision_id"), before_head);
    // Only the authored layer changes: a mark touches no derived body.
    assert_eq!(strings(approved, "changed_layers"), ["authored.usda"]);
    let derived_after = derived_layer(&after[1]);
    assert_eq!(derived_after, derived_before);
    let footprint = at(approved, &["footprint"]);
    assert_eq!(number(footprint, "writes"), 10);
    assert_eq!(text(at(approved, &["approval"]), "state"), "current");
    assert_eq!(
        text(at(approved, &["approval"]), "approved_token"),
        token,
        "the approval records the token it approved"
    );

    // Equivalent parts share a mark; the others differ.
    let marks = marks_of(&after[2]);
    let mark = |id: &String| marks.get(id).unwrap().as_str();
    assert_eq!(mark(&s.plates[0]), "PL1");
    assert_eq!(mark(&s.plates[1]), "PL1");
    assert_eq!(mark(&s.plates[2]), "PL2");
    assert_eq!(mark(&s.plates[3]), "PL3");
    assert_eq!(mark(&s.plates[4]), "PL4");
    assert_eq!(mark(&s.beams[0]), "B1");
    assert_eq!(mark(&s.beams[1]), "B1");
    assert_eq!(mark(&s.beams[2]), "B2");
    assert_eq!(mark(&frames[0]), "A-1");
    assert_eq!(mark(&frames[1]), "A-1");
    let distinct: std::collections::BTreeSet<_> = marks.values().collect();
    assert_eq!(distinct.len(), 7);

    // The status lists series and number separately, for a reader that composes its own text.
    let status = &after[2];
    let first_mark = list(status, "marks")
        .iter()
        .find(|m| text(m, "id") == s.plates[4])
        .unwrap();
    assert_eq!(text(first_mark, "series"), "PL");
    assert_eq!(number(first_mark, "number"), 4);
    assert_eq!(text(first_mark, "kind"), "plate");

    // The preview reads the approval back: current, and every group holds its proposed position.
    let p = &after[3];
    assert!(matches!(p.get("stale").unwrap(), Json::Bool(false)));
    assert_eq!(text(at(p, &["approval"]), "state"), "current");
    assert_eq!(number(at(p, &["approval"]), "marked"), 10);
    assert!(
        list(p, "groups")
            .iter()
            .all(|g| text(g, "stored") == "same")
    );
    assert_eq!(
        text(at(p, &["approval", "approved_settings"]), "plate_series"),
        "PL:1"
    );

    // The marks are three plain properties of the element prim in the authored layer.
    let authored = f.authored_text();
    assert_eq!(
        authored
            .matches("custom string steel:numbering:series")
            .count(),
        10
    );
    assert_eq!(
        authored
            .matches("custom int steel:numbering:number")
            .count(),
        10
    );
    assert_eq!(
        authored
            .matches("custom string steel:numbering:basis")
            .count(),
        10
    );
    // inspect-elements still lists the same plates with the same bodies.
    let after_inspected = &after[4];
    assert_eq!(
        list(after_inspected, "plates").len(),
        list(&inspected, "plates").len()
    );
    for (a, b) in list(after_inspected, "plates")
        .iter()
        .zip(list(&inspected, "plates"))
    {
        assert_eq!(text(a, "mesh_digest"), text(b, "mesh_digest"));
    }
}

#[test]
fn a_stale_token_overlapping_series_and_an_empty_model_are_refused_with_nothing_written() {
    let f = Fixture::new();
    // A store with no revision at all, then one with only a material.
    assert_eq!(
        f.run("approve-numbering", &["--preview-token", &"0".repeat(64)])
            .0,
        2
    );
    f.ok("define-material", &["--density", "7850"]);
    let empty_token = f.token(&[]);
    assert_eq!(
        f.refused("approve-numbering", &["--preview-token", &empty_token]),
        "INVALID_ARGUMENT"
    );
    assert_eq!(f.state(), "not_approved");

    let s = f.scene();
    let token = f.token(&[]);
    // Another part edit after the preview: the token is no longer the proposal.
    f.ok(
        "update-element",
        &["--id", &s.plates[1], "--thickness", "30"],
    );
    assert_eq!(
        f.refused("approve-numbering", &["--preview-token", &token]),
        "STALE_WORKSPACE"
    );
    assert_eq!(f.state(), "not_approved");
    // The token of other settings is not the proposal either.
    let current = f.token(&[]);
    assert_eq!(
        f.refused(
            "approve-numbering",
            &["--preview-token", &current, "--compare-name", "true"]
        ),
        "STALE_WORKSPACE"
    );
    // Overlapping series: the beams would take the plates' positions.
    let overlap = ["--beam-series", "PL:1"];
    let overlapping = f.token(&overlap);
    assert_eq!(
        f.refused(
            "approve-numbering",
            &["--preview-token", &overlapping, "--beam-series", "PL:1"]
        ),
        "INVALID_ARGUMENT"
    );
    // Series values the engine refuses.
    for bad in ["--plate-series", "--assembly-series"] {
        assert_eq!(
            f.refused(
                "approve-numbering",
                &["--preview-token", &current, bad, "TOOLONG_PREFIX_OVER_16:1"]
            ),
            "INVALID_ARGUMENT"
        );
    }
    assert_eq!(f.state(), "not_approved");
    // The proposal that is still the proposal is approved.
    assert_eq!(
        text(&f.approve(&current, &[]), "outcome"),
        "numbering_approved"
    );
}

#[test]
fn approving_again_is_unchanged_and_the_same_operation_replays() {
    let f = Fixture::new();
    let s = f.scene();
    let store = f.store();
    let token = f.token(&[]);
    let op = "0199e7a0-0000-7000-8000-00000000a091";
    let first = f.approve(&token, &["--op-id", op]);
    assert_eq!(text(&first, "outcome"), "numbering_approved");
    // The head and marks reads ride one process (#738 P7), as do the replay and the head that
    // proves it wrote nothing new.
    let read = f.script(&[
        Scripted::ok(&["open-revision", "--store", &store]),
        Scripted::ok(&["inspect-numbering", "--store", &store]),
    ]);
    let head = text(&read[0], "revision_id");
    let marks = marks_of(&read[1]);
    // The same operation id with the same command is a replay of the same revision.
    let again_round = f.script(&[
        Scripted::ok(&[
            "approve-numbering",
            "--store",
            &store,
            "--preview-token",
            &token,
            "--op-id",
            op,
        ]),
        Scripted::ok(&["open-revision", "--store", &store]),
    ]);
    let again = &again_round[0];
    assert_eq!(text(again, "outcome"), "numbering_approved_replayed");
    assert_eq!(text(&again_round[1], "revision_id"), head);
    assert_eq!(
        text(at(again, &["revision"]), "revision_id"),
        text(at(&first, &["revision"]), "revision_id")
    );
    // The same operation id with other settings is reuse.
    let other = f.token(&["--plate-series", "P:10"]);
    assert_eq!(
        f.refused(
            "approve-numbering",
            &[
                "--preview-token",
                &other,
                "--plate-series",
                "P:10",
                "--op-id",
                op
            ]
        ),
        "OP_ID_REUSE"
    );
    // A new operation id over the same approved numbering writes nothing and renumbers
    // nothing: the layers read brackets the approval in the same process.
    let unchanged_round = f.script(&[
        Scripted::ok(&["open-revision", "--store", &store]),
        Scripted::ok(&[
            "approve-numbering",
            "--store",
            &store,
            "--preview-token",
            &token,
        ]),
        Scripted::ok(&["open-revision", "--store", &store]),
        Scripted::ok(&["inspect-numbering", "--store", &store]),
    ]);
    let unchanged = &unchanged_round[1];
    assert_eq!(text(unchanged, "outcome"), "numbering_approved_unchanged");
    assert!(is_null(unchanged, "footprint"));
    assert!(strings(unchanged, "changed_layers").is_empty());
    assert_eq!(
        text(&unchanged_round[0], "revision_id"),
        text(&unchanged_round[2], "revision_id")
    );
    assert_eq!(
        at(&unchanged_round[0], &["layers"]),
        at(&unchanged_round[2], &["layers"])
    );
    assert_eq!(marks_of(&unchanged_round[3]), marks);
    // A move changes nothing that approving reads: still unchanged.
    let moved_round = f.script(&[
        Scripted::ok(&[
            "update-element",
            "--store",
            &store,
            "--id",
            &s.plates[0],
            "--translation",
            "5,5,5",
        ]),
        Scripted::ok(&["preview-numbering", "--store", &store]),
    ]);
    let after_move = text(&moved_round[1], "preview_token");
    assert_eq!(after_move, token);
    let still_unchanged = f.script(&[
        Scripted::ok(&[
            "approve-numbering",
            "--store",
            &store,
            "--preview-token",
            &after_move,
        ]),
        Scripted::ok(&["inspect-numbering", "--store", &store]),
    ]);
    assert_eq!(
        text(&still_unchanged[0], "outcome"),
        "numbering_approved_unchanged"
    );
    assert_eq!(marks_of(&still_unchanged[1]), marks);
    // The replay of the first operation still names its own revision after later edits.
    let replay = f.approve(&token, &["--op-id", op]);
    assert_eq!(text(&replay, "outcome"), "numbering_approved_replayed");
}

#[test]
fn fabrication_changes_make_the_approval_stale_and_a_move_or_a_rename_do_not() {
    let f = Fixture::new();
    let s = f.scene();
    let store = f.store();
    let token = f.token(&[]);
    // The approval and the state that judges it ride one process (#738 P7).
    let approved = f.script(&[
        Scripted::ok(&[
            "approve-numbering",
            "--store",
            &store,
            "--preview-token",
            &token,
        ]),
        Scripted::ok(&["inspect-numbering", "--store", &store]),
    ]);
    assert_eq!(text(at(&approved[1], &["approval"]), "state"), "current");
    // A move, a rename and a read-only view of the model do not; the whole round is one
    // process, ending with the state and the marks it is judged against.
    let benign = f.script(&[
        Scripted::ok(&[
            "update-element",
            "--store",
            &store,
            "--id",
            &s.plates[0],
            "--translation",
            "5,5,5",
        ]),
        Scripted::ok(&[
            "update-element",
            "--store",
            &store,
            "--id",
            &s.plates[1],
            "--name",
            "Renamed",
        ]),
        Scripted::ok(&["inspect-elements", "--store", &store]),
        Scripted::ok(&["preview-numbering", "--store", &store, "--tolerance", "5"]),
        Scripted::ok(&["inspect-numbering", "--store", &store]),
    ]);
    assert_eq!(text(at(&benign[4], &["approval"]), "state"), "current");
    let marks = marks_of(&benign[4]);

    // A thickness edit does, and the report names the part.
    let thickness = f.script(&[
        Scripted::ok(&[
            "update-element",
            "--store",
            &store,
            "--id",
            &s.plates[1],
            "--thickness",
            "30",
        ]),
        Scripted::ok(&["inspect-numbering", "--store", &store]),
    ]);
    let status = &thickness[1];
    let approval = at(status, &["approval"]);
    assert_eq!(text(approval, "state"), "stale");
    assert!(!is_null(approval, "reason"));
    assert_eq!(strings(approval, "differing_ids")[0], s.plates[1]);
    // The preview reports the same, and the stored marks are still there to read.
    let preview_round = f.script(&[
        Scripted::ok(&["preview-numbering", "--store", &store]),
        Scripted::ok(&["inspect-numbering", "--store", &store]),
    ]);
    assert_eq!(text(at(&preview_round[0], &["approval"]), "state"), "stale");
    assert_eq!(marks_of(&preview_round[1]), marks);
    // Re-approving renumbers from the rules: the part that left its group has a mark of its own
    // and the part that stayed keeps PL1.
    let renewed = f.token(&[]);
    assert_ne!(renewed, token);
    let renumbered = f.script(&[
        Scripted::ok(&[
            "approve-numbering",
            "--store",
            &store,
            "--preview-token",
            &renewed,
        ]),
        Scripted::ok(&["inspect-numbering", "--store", &store]),
    ]);
    assert_eq!(text(at(&renumbered[1], &["approval"]), "state"), "current");
    let marks = marks_of(&renumbered[1]);
    assert_eq!(marks[&s.plates[0]], "PL1");
    assert_ne!(marks[&s.plates[1]], marks[&s.plates[0]]);

    // A material edit, a copy and a delete each make it stale too. The definition and the
    // fresh token ride one process; every later edit lands with the state that judges it.
    let defined = f.script(&[
        Scripted::ok(&[
            "define-material",
            "--store",
            &store,
            "--density",
            "7850",
            "--name",
            "S460",
            "--grade",
            "S460",
        ]),
        Scripted::ok(&["preview-numbering", "--store", &store]),
    ]);
    let other_steel = text(at(&defined[0], &["definition"]), "digest");
    let renewed = text(&defined[1], "preview_token");
    let material_edit = f.script(&[
        Scripted::ok(&[
            "approve-numbering",
            "--store",
            &store,
            "--preview-token",
            &renewed,
        ]),
        Scripted::ok(&[
            "update-element",
            "--store",
            &store,
            "--id",
            &s.beams[0],
            "--material-digest",
            &other_steel,
        ]),
        Scripted::ok(&["inspect-numbering", "--store", &store]),
    ]);
    assert_eq!(text(at(&material_edit[2], &["approval"]), "state"), "stale");
    let approved_again = f.token(&[]);
    let material_approved = f.script(&[
        Scripted::ok(&[
            "approve-numbering",
            "--store",
            &store,
            "--preview-token",
            &approved_again,
        ]),
        Scripted::ok(&["inspect-numbering", "--store", &store]),
    ]);
    assert_eq!(
        text(at(&material_approved[1], &["approval"]), "state"),
        "current"
    );
    let copied = f.script(&[
        Scripted::ok(&["copy-element", "--store", &store, "--id", &s.beams[2]]),
        Scripted::ok(&["inspect-numbering", "--store", &store]),
    ]);
    let copy_id = text(at(&copied[0], &["element"]), "element_id");
    assert_eq!(text(at(&copied[1], &["approval"]), "state"), "stale");
    assert_eq!(number(at(&copied[1], &["approval"]), "unmarked"), 1);
    let copy_approved = f.token(&[]);
    let copy_round = f.script(&[
        Scripted::ok(&[
            "approve-numbering",
            "--store",
            &store,
            "--preview-token",
            &copy_approved,
        ]),
        Scripted::ok(&["inspect-numbering", "--store", &store]),
    ]);
    // The copy joined the group of its source and renumbered nothing.
    let marks = marks_of(&copy_round[1]);
    assert_eq!(marks[&copy_id], marks[&s.beams[2]]);
    let deleted = f.script(&[
        Scripted::ok(&["delete-element", "--store", &store, "--id", &s.beams[2]]),
        Scripted::ok(&["inspect-numbering", "--store", &store]),
    ]);
    assert_eq!(text(at(&deleted[1], &["approval"]), "state"), "stale");
}

#[test]
fn an_undo_of_an_approval_restores_the_earlier_marks() {
    let f = Fixture::new();
    let published = f.store();
    let steel = text(
        at(
            &f.ok("define-material", &["--density", "7850"]),
            &["definition"],
        ),
        "digest",
    );
    for _ in 0..2 {
        f.plate(&steel, "10", &[]);
    }
    let dir = f.dir("ws");
    f.ok_in(
        &published,
        "create-workspace",
        &["--workspace-dir", &dir, "--user-id", USER],
    );
    let history = |op: &str| {
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args([op, "--workspace-dir", &dir])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{op}");
        beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap()
    };
    let layers = || {
        list(
            at(
                &history("inspect-workspace-history"),
                &["draft_head", "manifest"],
            ),
            "layers",
        )
        .iter()
        .map(|l| (text(l, "name"), text(l, "digest")))
        .collect::<Vec<_>>()
    };
    let marks = || -> BTreeMap<String, String> {
        list(&f.ok_in(&dir, "inspect-numbering", &[]), "marks")
            .iter()
            .map(|m| (text(m, "id"), text(m, "mark")))
            .collect()
    };
    let before = layers();
    let token = text(&f.ok_in(&dir, "preview-numbering", &[]), "preview_token");
    f.ok_in(&dir, "approve-numbering", &["--preview-token", &token]);
    let approved = layers();
    assert_ne!(approved, before);
    let first = marks();
    assert_eq!(first.len(), 2);
    assert!(first.values().all(|m| m == "PL1"));

    history("undo-workspace");
    assert_eq!(layers(), before, "undo restores the layers byte for byte");
    assert!(marks().is_empty());
    let undone = f.ok_in(&dir, "inspect-numbering", &[]);
    assert_eq!(text(at(&undone, &["approval"]), "state"), "not_approved");
    history("redo-workspace");
    assert_eq!(layers(), approved);
    assert_eq!(marks(), first);
    // The published store never saw the approval.
    let published_status = f.ok_in(&published, "inspect-numbering", &[]);
    assert_eq!(number(&published_status, "count"), 0);
}

#[test]
fn damaged_and_oversized_input_is_refused() {
    let f = Fixture::new();
    let store = f.store();
    f.scene();
    let token = f.token(&[]);
    f.approve(&token, &[]);
    assert_eq!(f.state(), "current");
    // Bad arguments. The oversized token exceeds the script file's 4 KiB string cap, so its
    // refusal keeps the original three real spawns (the layers reads bracket it).
    let oversized_token = "a".repeat(4097);
    let oversized = vec!["approve-numbering", "--preview-token", &oversized_token];
    let before = f.layers();
    let (code, body) = f.run("approve-numbering", &oversized);
    assert_eq!(code, 2, "{body:?}");
    assert_eq!(text(&body, "outcome"), "rejected");
    assert_eq!(f.layers(), before);
    for args in [
        vec!["--preview-token", "abc"],
        vec!["--preview-token", &"G".repeat(64)],
        vec!["--preview-token", &token, "--preview-token", &token],
        vec!["--preview-token", &token, "--previous", &token],
        vec!["--preview-token", &token, "--offset", "1"],
        vec!["--plate-series", "PL:1"],
        vec!["--preview-token", &token, "--plate-series", "PL"],
        vec!["--preview-token", &token, "--tolerance", "NaN"],
        vec!["--preview-token", &token, "--tolerance", "5000"],
        vec!["--preview-token", &token, "--compare-name", "maybe"],
    ] {
        assert_eq!(
            f.refused("approve-numbering", &args),
            "INVALID_ARGUMENT",
            "{args:?}"
        );
    }
    assert_eq!(
        f.refused(
            "approve-numbering",
            &["--preview-token", &token, "--op-id", "not-an-id"]
        ),
        "INVALID_ID"
    );
    for args in [
        vec!["--limit", "0"],
        vec!["--limit", "257"],
        vec!["--offset", "x"],
        vec!["--preview-token", &token],
    ] {
        assert_eq!(
            f.refused("inspect-numbering", &args),
            "INVALID_ARGUMENT",
            "{args:?}"
        );
    }
    // Damaged records in the model are refused, never read as "unnumbered".
    let before = f.layers();
    for (from, to) in [
        // A partial record.
        (
            "custom string steel:numbering:basis",
            "custom string steel:numbering:other",
        ),
        // A wrong type.
        (
            "custom int steel:numbering:number = 1",
            "custom double steel:numbering:number = 1",
        ),
        // A number outside the range.
        (
            "custom int steel:numbering:number = 1",
            "custom int steel:numbering:number = -4",
        ),
        // An oversized record.
        (
            "custom string steel:numbering:series = \"PL\"",
            &format!(
                "custom string steel:numbering:series = \"{}\"",
                "X".repeat(40)
            ),
        ),
    ] {
        f.tamper(|name, text| {
            if name == "authored.usda" {
                text.replacen(from, to, 1)
            } else {
                text
            }
        });
        // All four readers refuse the damaged model (exit 2 each); the refusals ride one
        // process (#738 P7).
        f.script(&[
            Scripted::refused(&["inspect-numbering", "--store", &store]),
            Scripted::refused(&["preview-numbering", "--store", &store]),
            Scripted::refused(&["inspect-elements", "--store", &store]),
            Scripted::refused(&[
                "approve-numbering",
                "--store",
                &store,
                "--preview-token",
                &token,
            ]),
        ]);
        // Put the model back for the next case: the tamper commits a new head, so rewind the text.
        f.tamper(|name, text| {
            if name == "authored.usda" {
                text.replacen(to, from, 1)
            } else {
                text
            }
        });
        assert_eq!(f.state(), "current", "{to}");
    }
    // Every tamper was undone byte for byte.
    assert_eq!(f.layers(), before);
}

/// The independent reader re-reads the stored marks from the committed layer text.
#[test]
fn the_reader_reads_the_stored_marks() {
    let f = Fixture::new();
    let s = f.scene();
    f.approve(&f.token(&[]), &[]);
    let layers = f.shared.root.join("layers");
    fs::create_dir(&layers).unwrap();
    let opened = f.ok("open-revision", &[]);
    for entry in list(&opened, "layers") {
        let digest = text(entry, "digest");
        fs::copy(
            f.shared
                .root
                .join("store")
                .join("blobs")
                .join(&digest[..2])
                .join(&digest),
            {
                let target = layers.join(text(entry, "name"));
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).unwrap();
                }
                target
            },
        )
        .unwrap();
    }
    let output = Command::new(env!("CARGO_BIN_EXE_beamcraft-usd-oracle"))
        .arg("marks")
        .arg(layers.join("root.usda"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = beamcraft_json::decode(
        String::from_utf8_lossy(&output.stdout).trim().as_bytes(),
        &Limits::DEFAULT,
    )
    .unwrap();
    // The reader and the CLI agree on every mark: series, number and the approval record.
    let status = f.status();
    let basis = text(at(&status, &["approval"]), "approved_token");
    for m in list(&status, "marks") {
        let seen = list(&report, &text(m, "id"));
        assert_eq!(seen[0].string().unwrap(), text(m, "series"));
        assert_eq!(seen[1].unsigned().unwrap(), number(m, "number"));
        assert!(seen[2].string().unwrap().contains(&basis));
    }
    assert_eq!(list(&status, "marks").len(), 8);
    assert!(report.get(&s.plates[0]).is_ok());
}
