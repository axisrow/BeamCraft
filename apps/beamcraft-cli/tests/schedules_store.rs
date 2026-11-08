// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates and attributes, MPL-2.0.

//! I05 stored templates end to end: `schedule-define`, `schedules`, `schedule-undefine`
//! and `schedule-export --schedule` through the real CLI binary. A template saved in the
//! model is a shared owned definition: it exports byte-identically to the file form,
//! survives a reopen, replays its operation and keeps the unknown-and-stale semantics of
//! the file form when the attribute state moves under it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::support;
use support::{StoreFixture, text};

use beamcraft_host::{revision::RevisionStore, storage::BlobStore};
use beamcraft_json::Json;
use beamcraft_layer_engine::{
    Asset, BACKEND, Budgets, Cancellation, Control, Identity, OpenRequest, Prim, Profile, Reader,
};
use std::fs;
use std::time::{Duration, Instant};

const PLATE: &str = "019a7bf0-0000-7000-8000-0000000b0401";

struct Fixture(StoreFixture);
impl Fixture {
    fn new(suite: &str) -> Self {
        let shared = StoreFixture::new(suite);
        let fixture = Self(shared);
        fixture.ok(
            "create-plate",
            &[
                "--width",
                "1000",
                "--length",
                "500",
                "--thickness",
                "10",
                "--new-id",
                PLATE,
            ],
        );
        fixture
    }
    fn run(&self, command: &str, extra: &[&str]) -> (i32, Json) {
        self.0.run(command, extra)
    }
    fn ok(&self, command: &str, extra: &[&str]) -> Json {
        let (code, body) = self.run(command, extra);
        assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
        body
    }
    fn rejected(&self, command: &str, extra: &[&str]) -> String {
        let (code, body) = self.run(command, extra);
        assert_eq!(code, 2, "{command} {extra:?}");
        text(&body, "code")
    }
    fn out(&self, name: &str) -> String {
        self.0.root.join(name).to_str().unwrap().to_owned()
    }
    /// Writes a template file under the fixture root and returns its path.
    fn template(&self, name: &str, body: &str) -> String {
        let path = self.0.root.join(name);
        fs::write(&path, body).unwrap();
        path.to_str().unwrap().to_owned()
    }
    /// The names of the stored templates, in the listing's order.
    fn stored_names(&self) -> Vec<String> {
        let j = self.ok("schedules", &[]);
        j.get("schedules")
            .unwrap()
            .array()
            .unwrap()
            .iter()
            .map(|s| text(s, "name"))
            .collect()
    }
    fn revisions(&self) -> RevisionStore {
        RevisionStore::new(BlobStore::new(self.0.root.join("store")).unwrap())
    }
    /// The prims of the verified head, read through the generic USD reader.
    fn prims(&self) -> Vec<Prim> {
        let revision = self.revisions().open().unwrap();
        let content = revision.manifest().content();
        let assets: Vec<_> = content
            .entries()
            .iter()
            .map(|entry| Asset {
                name: entry.name().into(),
                digest: entry.digest(),
                bytes: revision.layer_bytes(entry.name()).unwrap(),
            })
            .collect();
        let mut request = OpenRequest {
            identity: Identity {
                project: content.project().to_string(),
                workspace: content.workspace().to_string(),
                revision: revision.manifest().digest().unwrap(),
                generation: 1,
            },
            root: content.root_layer().into(),
            owned_layers: assets
                .iter()
                .filter(|a| !a.name.starts_with("external/"))
                .map(|a| a.name.clone())
                .collect(),
            assets,
            manifest: revision.manifest().digest().unwrap(),
            backend: BACKEND,
            profile: Profile::StaticLocal,
            budgets: Budgets::default(),
        };
        request.manifest = request.manifest_digest().unwrap();
        let control = Control {
            cancellation: Cancellation::default(),
            deadline: Instant::now() + Duration::from_secs(10),
        };
        let mut reader = Reader::default();
        let opened = reader.open(request, &control).unwrap();
        let mut prims = Vec::new();
        let mut cursor = None;
        loop {
            let page = reader
                .read(&opened.context, cursor.as_ref(), &control)
                .unwrap();
            prims.extend(page.items);
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        prims
    }
}

fn property<'a>(prim: &'a Prim, name: &str) -> (&'a str, Option<&'a str>) {
    let prop = prim
        .properties
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("{name} missing from {}", prim.path));
    let value = match prop.value.as_ref() {
        Some(beamcraft_layer_engine::Value::Text(text)) => Some(text.as_str()),
        _ => None,
    };
    (prop.type_name.as_str(), value)
}

/// One graded attribute on the plate and the evidence template over a standard column and
/// that attribute.
fn graded(f: &Fixture) -> String {
    f.ok(
        "attribute-define",
        &["--name", "Paint_status", "--type", "text"],
    );
    f.ok(
        "attribute-set",
        &[
            "--element",
            PLATE,
            "--name",
            "Paint_status",
            "--value",
            "New",
        ],
    );
    f.template(
        "paint.json",
        r#"{"version":1,"columns":[{"column":"mark"},{"attribute":"Paint_status"}]}"#,
    )
}

#[test]
fn a_stored_template_exports_byte_identically_and_replays() {
    let f = Fixture::new("i05-schedules-store");
    let t = graded(&f);
    let op = "019a7bf0-0000-7000-8000-0000000b05e1";
    let defined = f.ok(
        "schedule-define",
        &["--name", "paint_bom", "--template", &t, "--op-id", op],
    );
    assert_eq!(text(&defined, "outcome"), "schedule_define");
    // The definition is in the model: the listing names it, and the prim carries the
    // document verbatim as a definition-scope property.
    assert_eq!(f.stored_names(), vec!["paint_bom".to_owned()]);
    let prims = f.prims();
    let def = prims
        .iter()
        .find(|p| p.path == "/World/Definitions/Schedules/paint_bom")
        .unwrap();
    let (type_name, value) = property(def, "steel:schedule:template");
    assert_eq!(type_name, "string");
    assert_eq!(value, Some(fs::read_to_string(&t).unwrap().trim()));
    // The same operation replays.
    let replay = f.ok(
        "schedule-define",
        &["--name", "paint_bom", "--template", &t, "--op-id", op],
    );
    assert_eq!(text(&replay, "outcome"), "schedule_define_replayed");
    // The stored form renders byte-identically to the file form over the same head.
    f.ok(
        "schedule-export",
        &[
            "--template",
            &t,
            "--out",
            &f.out("from-file.csv"),
            "--html",
            &f.out("from-file.html"),
        ],
    );
    let stored = f.ok(
        "schedule-export",
        &[
            "--schedule",
            "paint_bom",
            "--out",
            &f.out("from-store.csv"),
            "--html",
            &f.out("from-store.html"),
        ],
    );
    assert_eq!(text(&stored, "outcome"), "schedule_exported");
    for (from_file, from_store) in [
        ("from-file.csv", "from-store.csv"),
        ("from-file.html", "from-store.html"),
    ] {
        assert_eq!(
            fs::read(f.0.root.join(from_file)).unwrap(),
            fs::read(f.0.root.join(from_store)).unwrap()
        );
    }
    // Naming both sources refuses; naming neither refuses.
    assert_eq!(
        f.rejected(
            "schedule-export",
            &[
                "--template",
                &t,
                "--schedule",
                "paint_bom",
                "--out",
                &f.out("never.csv")
            ]
        ),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.rejected("schedule-export", &["--out", &f.out("never.csv")]),
        "INVALID_ARGUMENT"
    );
}

#[test]
fn definition_refusals_and_the_undefine_lifecycle() {
    let f = Fixture::new("i05-schedules-store");
    let t = graded(&f);
    f.ok(
        "schedule-define",
        &["--name", "paint_bom", "--template", &t],
    );
    // A duplicate name refuses, and the stored listing is untouched.
    assert_eq!(
        f.rejected(
            "schedule-define",
            &["--name", "paint_bom", "--template", &t]
        ),
        "INVALID_ARGUMENT"
    );
    assert_eq!(f.stored_names(), vec!["paint_bom".to_owned()]);
    // A missing undefine refuses; a bad name refuses; an export naming an unknown stored
    // template refuses.
    assert_eq!(
        f.rejected("schedule-undefine", &["--name", "ghost_bom"]),
        "NOT_FOUND"
    );
    assert_eq!(
        f.rejected(
            "schedule-define",
            &["--name", "no-dashes", "--template", &t]
        ),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.rejected(
            "schedule-export",
            &["--schedule", "ghost_bom", "--out", &f.out("never.csv")]
        ),
        "NOT_FOUND"
    );
    // The removal leaves the listing empty, and an export naming the removed template
    // refuses afterwards.
    let removed = f.ok("schedule-undefine", &["--name", "paint_bom"]);
    assert_eq!(text(&removed, "outcome"), "schedule_undefine");
    assert!(f.stored_names().is_empty());
    assert_eq!(
        f.rejected(
            "schedule-export",
            &["--schedule", "paint_bom", "--out", &f.out("never.csv")]
        ),
        "NOT_FOUND"
    );
    // A second undefine of the same name refuses.
    assert_eq!(
        f.rejected("schedule-undefine", &["--name", "paint_bom"]),
        "NOT_FOUND"
    );
}

/// The store form keeps the file form's semantics when the attribute state moves: a column
/// whose definition disappears stays visible and is named in unknown_columns, while the
/// stored document itself is never rewritten.
#[test]
fn a_later_attribute_change_keeps_the_template_honest() {
    let f = Fixture::new("i05-schedules-store");
    let t = graded(&f);
    f.ok(
        "schedule-define",
        &["--name", "paint_bom", "--template", &t],
    );
    f.ok("attribute-undefine", &["--name", "Paint_status"]);
    let exported = f.ok(
        "schedule-export",
        &["--schedule", "paint_bom", "--out", &f.out("moved.csv")],
    );
    let unknown = exported
        .get("unknown_columns")
        .unwrap()
        .array()
        .unwrap()
        .iter()
        .map(|s| s.string().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(unknown, vec!["Paint_status".to_owned()]);
    assert_eq!(f.stored_names(), vec!["paint_bom".to_owned()]);
    // A template naming no attribute at all defines over any state and exports.
    let plain = f.template(
        "plain.json",
        r#"{"version":1,"columns":[{"column":"kind"}],"filter":{"key":"kind","op":"eq","value":"plate"}}"#,
    );
    f.ok(
        "schedule-define",
        &["--name", "plates_only", "--template", &plain],
    );
    f.ok(
        "schedule-export",
        &["--schedule", "plates_only", "--out", &f.out("plates.csv")],
    );
    assert_eq!(
        f.stored_names(),
        vec!["paint_bom".to_owned(), "plates_only".to_owned()]
    );
    // A malformed document refuses at define time and stores nothing.
    let broken = f.template("broken.json", "{\"version\":1,");
    assert_eq!(
        f.rejected(
            "schedule-define",
            &["--name", "broken_bom", "--template", &broken]
        ),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.stored_names(),
        vec!["paint_bom".to_owned(), "plates_only".to_owned()]
    );
}
