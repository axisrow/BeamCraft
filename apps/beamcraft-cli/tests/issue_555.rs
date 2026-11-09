// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates; the fastener values are the shipped DIN-derived
// library's (`assets/libraries/din-fasteners.usda`), MPL-2.0.

//! Issue 555 end to end through the real `beamcraft` binary, one case per repro of the issue:
//! the bolt grip reads the bearing ring around any coaxial hole (finding 1), a generated hole
//! follows a rotated plate (finding 2), the plate box honours `thickness_side` (finding 3), a
//! verbatim retry of a `create-*` without `--new-id` replays (finding 4) and a store pinned to
//! another layer mapping is reported honestly by `open-revision` (finding 5).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support::{StoreFixture, at, list, text};

use beamcraft_host::{
    ids::IdSource,
    revision::{ExpectedBase, LayerEntry, NoFault, RevisionDraft, RevisionStore, Uuid7},
    storage::BlobStore,
};
use beamcraft_json::Json;

const DIN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/libraries/din-fasteners.usda"
);

struct Fixture {
    shared: StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        Self {
            shared: StoreFixture::new("issue-555-tests"),
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

    fn plate(&self, extra: &[&str]) -> String {
        text(
            at(&self.ok("create-plate", extra), &["plate"]),
            "element_id",
        )
    }

    /// The stored length of every named group of a bolt-group reply.
    fn lengths(reply: &Json) -> Vec<(String, String)> {
        list(reply, "bolt_groups")
            .iter()
            .map(|g| (text(g, "name"), text(g, "length_mm")))
            .collect()
    }

    /// Commits a head-based revision with edited layer text, the way a store written by
    /// another build arrives (`commit-revision` checks bytes, never the layer mapping).
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

/// The DIN M16 group of the finding 1 repro over two stacked 20 mm plates at z 0..40.
fn grip_group<'a>(parts: &'a str, name: &'a str) -> Vec<&'a str> {
    vec![
        "--parts",
        parts,
        "--fastener-library",
        DIN,
        "--designation",
        "M16",
        "--point1",
        "50,50,40",
        "--point2",
        "50,150,40",
        "--plane-normal",
        "0,0,-1",
        "--offset-start",
        "25",
        "--count",
        "2",
        "--pitch",
        "50",
        "--hole-mode",
        "none",
        "--name",
        name,
    ]
}

#[test]
fn foreign_coaxial_holes_keep_the_grip_and_a_rename_keeps_the_length() {
    // Finding 1: M16 through 2 x 20 mm needs 40 + washers 2 x 3 + nut 14.8 + protrusion 5
    // = 65.8 -> 70, whether the coaxial holes were cut before or after the group, by
    // `add-hole` or by `create-plate --hole-round`, and a plain rename keeps it.
    let f = Fixture::new();
    let common = ["--width", "200", "--length", "200", "--thickness", "20"];
    let p1 = f.plate(&[&common[..], &["--name", "P1"]].concat());
    let p2 = f.plate(&[&common[..], &["--translation", "0,0,20", "--name", "P2"]].concat());
    let parts = format!("{p1},{p2}");
    let first = f.ok("create-bolt-group", &grip_group(&parts, "G_A"));
    assert_eq!(Fixture::lengths(&first), vec![("G_A".into(), "70".into())]);
    let group_a = text(&first, "element_id");
    for plate in [&p1, &p2] {
        for y in ["75", "125"] {
            f.ok(
                "add-hole",
                &["--id", plate, "--hole-round", &format!("50,{y},18")],
            );
        }
    }
    let second = f.ok("create-bolt-group", &grip_group(&parts, "G_B"));
    let renamed = f.ok(
        "update-bolt-group",
        &["--target", &group_a, "--name", "G_A_renamed"],
    );
    assert_eq!(
        Fixture::lengths(&second)
            .into_iter()
            .chain(Fixture::lengths(&renamed))
            .filter(|(_, length)| length != "70")
            .collect::<Vec<_>>(),
        Vec::<(String, String)>::new()
    );
    // Holes authored with the plate itself grip their bearing ring too.
    let holed: Vec<&str> = vec![
        "--width",
        "200",
        "--length",
        "200",
        "--thickness",
        "20",
        "--hole-round",
        "50,75,18;50,125,18",
    ];
    let q1 = f.plate(&[&holed[..], &["--translation", "500,0,0"]].concat());
    let q2 = f.plate(&[&holed[..], &["--translation", "500,0,20"]].concat());
    let parts_q = format!("{q1},{q2}");
    let mut args = grip_group(&parts_q, "G_C");
    args[7] = "550,50,40";
    args[9] = "550,150,40";
    let created = f.ok("create-bolt-group", &args);
    assert!(
        Fixture::lengths(&created).contains(&("G_C".into(), "70".into())),
        "{created:?}"
    );
}

#[test]
fn an_opening_wider_than_the_washer_is_refused_not_dropped_from_the_grip() {
    // Finding 1, adjacent case: both axes pass through a 120 mm opening of a listed plate, so
    // no bearing ring finds its material — the group would not clamp it. Before the fix the
    // part was admitted by its box and silently left out of the grip (length 45).
    let f = Fixture::new();
    let p1 = f.plate(&[
        "--width",
        "200",
        "--length",
        "200",
        "--thickness",
        "20",
        "--hole-round",
        "100,100,120",
    ]);
    let p2 = f.plate(&[
        "--width",
        "200",
        "--length",
        "200",
        "--thickness",
        "20",
        "--translation",
        "0,0,20",
    ]);
    // Two fasteners at (100, 75) and (100, 125), both inside the opening.
    let parts = format!("{p1},{p2}");
    let mut args = grip_group(&parts, "G_OPEN");
    args[7] = "100,50,40";
    args[9] = "100,150,40";
    let (code, body) = f.run("create-bolt-group", &args);
    assert_eq!(code, 2, "{body:?}");
    assert_eq!(text(&body, "code"), "INVALID_ARGUMENT");
    assert!(text(&body, "message").contains("never clamped"), "{body:?}");
}

#[test]
fn generate_cuts_holes_in_rotated_plates_on_the_bolt_axes() {
    // Finding 2: a gusset and a knife plate in the world XZ plane (`--plane`), M16 with the
    // default generated holes. Each plate gets one hole per bolt, centred on the axis
    // (plate-local (100, 100) and (200, 100) are world x 100/200 at z 100), and the grip is
    // 2 x 10 mm: 20 + 2 x 3 + 14.8 + 5 = 45.8 -> 50.
    let f = Fixture::new();
    let contour = ["--contour", "0,0;300,0;300,200;0,200", "--thickness", "10"];
    let gusset = f.plate(&[&contour[..], &["--plane", "0,0,0,1,0,0,0,0,1"]].concat());
    let knife = f.plate(&[&contour[..], &["--plane", "0,10,0,1,0,0,0,0,1"]].concat());
    let reply = f.ok(
        "create-bolt-group",
        &[
            "--parts",
            &format!("{gusset},{knife}"),
            "--fastener-library",
            DIN,
            "--designation",
            "M16",
            "--point1",
            "100,-10,100",
            "--point2",
            "200,-10,100",
            "--plane-normal",
            "0,1,0",
            "--offset-start",
            "0",
            "--count",
            "2",
            "--pitch",
            "100",
            "--name",
            "G_ROT",
        ],
    );
    assert_eq!(
        Fixture::lengths(&reply),
        vec![("G_ROT".into(), "50".into())]
    );
    let inspected = f.ok("inspect-elements", &[]);
    let plates = list(&inspected, "plates");
    assert_eq!(plates.len(), 2);
    for plate in plates {
        let holes = list(plate, "holes_mm");
        assert_eq!(holes.len(), 2, "{plate:?}");
        let mut centres: Vec<(f64, f64)> = holes
            .iter()
            .map(|ring| {
                let points = ring.array().unwrap();
                let n = points.len() as f64;
                let (mut x, mut y) = (0.0, 0.0);
                for p in points {
                    let p = p.array().unwrap();
                    x += p[0].float().unwrap();
                    y += p[1].float().unwrap();
                }
                (x / n, y / n)
            })
            .collect();
        centres.sort_by(|a, b| a.0.total_cmp(&b.0));
        for ((x, y), (ex, ey)) in centres.into_iter().zip([(100.0, 100.0), (200.0, 100.0)]) {
            assert!((x - ex).abs() < 1e-6 && (y - ey).abs() < 1e-6, "({x}, {y})");
        }
    }
}

#[test]
fn a_bottom_sided_plate_admits_a_bolt_from_its_real_top_face() {
    // Finding 3: a `bottom` plate's material is local z in [-20, 0]; the bolt bearing on its
    // real top face (z = 0) crosses it and the 20 mm plate below: 70.
    let f = Fixture::new();
    let p8 = f.plate(&[
        "--width",
        "60",
        "--length",
        "60",
        "--thickness",
        "20",
        "--thickness-side",
        "bottom",
        "--translation",
        "600,0,0",
    ]);
    let p9 = f.plate(&[
        "--width",
        "60",
        "--length",
        "60",
        "--thickness",
        "20",
        "--translation",
        "600,0,-40",
    ]);
    let reply = f.ok(
        "create-bolt-group",
        &[
            "--parts",
            &format!("{p8},{p9}"),
            "--fastener-library",
            DIN,
            "--designation",
            "M16",
            "--point1",
            "620,20,0",
            "--point2",
            "620,40,0",
            "--plane-normal",
            "0,0,-1",
            "--offset-start",
            "0",
            "--count",
            "2",
            "--pitch",
            "20",
            "--hole-mode",
            "none",
            "--name",
            "G_E",
        ],
    );
    assert_eq!(Fixture::lengths(&reply), vec![("G_E".into(), "70".into())]);
}

#[test]
fn a_verbatim_create_retry_without_new_id_replays() {
    // Finding 4: the caller of an exit-4/124 retry never saw the generated id, so the same
    // argv with the same `--op-id` must replay, for every `create-*` family.
    let f = Fixture::new();
    let retry = |command: &str, args: &[&str], op: &str| {
        let mut full = args.to_vec();
        full.extend_from_slice(&["--op-id", op]);
        let first = f.ok(command, &full);
        let again = f.ok(command, &full);
        let outcome = text(&again, "outcome");
        assert!(outcome.ends_with("_replayed"), "{command}: {again:?}");
        (first, again)
    };
    let (first, again) = retry(
        "create-plate",
        &["--width", "200", "--length", "200", "--thickness", "20"],
        "019a7bf0-0000-7000-8000-0000005550a1",
    );
    assert_eq!(
        text(at(&first, &["plate"]), "element_id"),
        text(at(&again, &["plate"]), "element_id")
    );
    let p1 = text(at(&first, &["plate"]), "element_id");
    let p2 = f.plate(&[
        "--width",
        "200",
        "--length",
        "200",
        "--thickness",
        "20",
        "--translation",
        "0,0,20",
    ]);
    retry(
        "create-bolt-group",
        &grip_group(&format!("{p1},{p2}"), "G_RETRY"),
        "019a7bf0-0000-7000-8000-0000005550a2",
    );
    retry(
        "create-level",
        &["--name", "Storey", "--elevation", "3000"],
        "019a7bf0-0000-7000-8000-0000005550a3",
    );
    retry(
        "create-assembly",
        &["--name", "A1"],
        "019a7bf0-0000-7000-8000-0000005550a4",
    );
}

#[test]
fn a_store_of_another_layer_mapping_is_not_reported_verified() {
    // Finding 5: open-revision reports the store's layer-schema pins next to this build's
    // digest; a store pinned to another mapping is refused with both digests, and content
    // commands say what happened instead of a bare mismatch.
    let f = Fixture::new();
    f.plate(&["--width", "200", "--length", "200", "--thickness", "20"]);
    let verified = f.ok("open-revision", &[]);
    assert_eq!(text(&verified, "outcome"), "revision_verified");
    let schema = at(&verified, &["schema"]);
    let build = text(schema, "build_digest");
    assert_eq!(at(schema, &["compatible"]), &Json::Bool(true));
    assert_eq!(
        list(schema, "store_digests"),
        &[Json::text(build.as_str())][..]
    );
    let other = "a".repeat(64);
    f.tamper(|_, text| text.replace(&build, &other));
    let (code, refused) = f.run("open-revision", &[]);
    assert_eq!(code, 2, "{refused:?}");
    assert_eq!(text(&refused, "code"), "UNSUPPORTED_VERSION");
    let diagnostic = &list(&refused, "diagnostics")[0];
    let schema = at(diagnostic, &["schema"]);
    assert_eq!(text(schema, "build_digest"), build);
    assert_eq!(
        list(schema, "store_digests"),
        &[Json::text(other.as_str())][..]
    );
    assert_eq!(at(schema, &["compatible"]), &Json::Bool(false));
    let (code, inspected) = f.run("inspect-elements", &[]);
    assert_eq!(code, 2, "{inspected:?}");
    assert_eq!(text(&inspected, "code"), "UNSUPPORTED_VERSION");
    assert!(
        text(&inspected, "message").contains("open-revision reports"),
        "{inspected:?}"
    );
}
