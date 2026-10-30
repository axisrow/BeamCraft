// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic model data, MPL-2.0.

//! I04 end to end: defined attributes (text, number, choice, with units and sets) on any
//! element, stored in USD, and an unknown attribute is shown, never dropped.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::support;

use support::{at, list, text};

use beamcraft_host::{revision::RevisionStore, storage::BlobStore};
use beamcraft_json::Json;
use beamcraft_layer_engine::{
    Asset, BACKEND, Budgets, Cancellation, Control, Identity, OpenRequest, Prim, Profile, Reader,
};
use std::time::{Duration, Instant};

const PLATE: &str = "019a7bf0-0000-7000-8000-0000000a0401";
const OTHER: &str = "019a7bf0-0000-7000-8000-0000000a0402";

struct Fixture(support::StoreFixture);
impl Fixture {
    fn new() -> Self {
        let shared = support::StoreFixture::new("i04-attributes");
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
    fn get(&self) -> Json {
        self.ok("attribute-get", &["--element", PLATE])
    }
    fn entries(&self) -> Vec<(String, String)> {
        let get = self.get();
        list(&get, "attributes")
            .iter()
            .map(|e| {
                let name = text(e, "name");
                let value = |state: &str| {
                    let v = e.get("value").unwrap();
                    let rendered = match v.string() {
                        Ok(s) => s.to_owned(),
                        _ => match v.float() {
                            Ok(n) => format!("{n}"),
                            _ => "null".to_owned(),
                        },
                    };
                    format!("{state}:{rendered}")
                };
                let state = match e.get("defined").and_then(|d| d.boolean()) {
                    Ok(true) => value("defined"),
                    _ => value("unknown"),
                };
                (name, state)
            })
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
        .unwrap_or_else(|| panic!("{name} missing on {}", prim.path));
    let value = prop.value.as_ref().and_then(|v| match v {
        beamcraft_layer_engine::Value::Text(t) => Some(t.as_str()),
        _ => None,
    });
    (prop.type_name.as_str(), value)
}

#[test]
fn definitions_lists_types_units_options_and_sets() {
    let f = Fixture::new();
    f.ok(
        "attribute-define",
        &[
            "--name",
            "Finish",
            "--type",
            "choice",
            "--options",
            "plain,galvanized",
            "--set",
            "Coating",
        ],
    );
    f.ok("attribute-define", &["--name", "Note", "--type", "text"]);
    f.ok(
        "attribute-define",
        &[
            "--name", "Mass", "--type", "number", "--unit", "kg", "--set", "Coating",
        ],
    );
    let listed = f.ok("attributes", &[]);
    let defs = list(&listed, "attributes");
    assert_eq!(defs.len(), 3);
    let finish = defs.iter().find(|d| text(d, "name") == "Finish").unwrap();
    assert_eq!(text(finish, "type"), "choice");
    assert_eq!(text(finish, "set"), "Coating");
    let options: Vec<String> = list(finish, "options")
        .iter()
        .map(|o| o.string().unwrap().to_owned())
        .collect();
    assert_eq!(options, vec!["plain".to_owned(), "galvanized".to_owned()]);
    let mass = defs.iter().find(|d| text(d, "name") == "Mass").unwrap();
    assert_eq!(text(mass, "type"), "number");
    assert_eq!(text(mass, "unit"), "kg");
    assert_eq!(defs[0].get("name").unwrap().string().unwrap(), "Finish");
}

#[test]
fn a_value_is_set_shown_and_survives_an_element_edit() {
    let f = Fixture::new();
    f.ok("attribute-define", &["--name", "Note", "--type", "text"]);
    f.ok(
        "attribute-define",
        &["--name", "Mass", "--type", "number", "--unit", "kg"],
    );
    let set = f.ok(
        "attribute-set",
        &["--element", PLATE, "--name", "Note", "--value", "hot dip"],
    );
    assert_eq!(text(&set, "outcome"), "attribute_set");
    assert_eq!(text(at(&set, &["subject"]), "element_id"), PLATE);
    f.ok(
        "attribute-set",
        &["--element", PLATE, "--name", "Mass", "--value", "12.5"],
    );
    let entries = f.entries();
    assert!(entries.contains(&("Mass".to_owned(), "defined:12.5".to_owned())));
    assert!(entries.contains(&("Note".to_owned(), "defined:hot dip".to_owned())));
    // A rename rewrites the label alone: the stored values survive untouched.
    f.ok("update-element", &["--id", PLATE, "--name", "Gusset A"]);
    assert_eq!(f.entries(), entries);
    // A retry under the same operation replays; a contradicting reuse is refused.
    let op = "019a7bf0-0000-7000-8000-0000000a0e01";
    f.ok(
        "attribute-set",
        &[
            "--element",
            PLATE,
            "--name",
            "Note",
            "--value",
            "second",
            "--op-id",
            op,
        ],
    );
    let replay = f.ok(
        "attribute-set",
        &[
            "--element",
            PLATE,
            "--name",
            "Note",
            "--value",
            "second",
            "--op-id",
            op,
        ],
    );
    assert_eq!(text(&replay, "outcome"), "attribute_set_replayed");
    assert_eq!(
        f.rejected(
            "attribute-set",
            &[
                "--element",
                PLATE,
                "--name",
                "Note",
                "--value",
                "third",
                "--op-id",
                op
            ]
        ),
        "OP_ID_REUSE"
    );
    // The stored value and the definition are ordinary USD properties of the committed layers.
    let prims = f.prims();
    let part = prims
        .iter()
        .find(|p| p.path == format!("/World/Steel/Parts/PRT_{}", PLATE.replace('-', "")))
        .unwrap();
    let (type_name, value) = property(part, "bim:attr:Note");
    assert_eq!(type_name, "string");
    assert_eq!(value, Some("second"));
    let def = prims
        .iter()
        .find(|p| p.path == "/World/Definitions/Attributes/Note")
        .unwrap();
    let (type_name, value) = property(def, "bim:attr:type");
    assert_eq!(type_name, "token");
    assert_eq!(value, Some("text"));
}

#[test]
fn an_unknown_attribute_is_shown_and_never_dropped() {
    let f = Fixture::new();
    f.ok("attribute-define", &["--name", "Note", "--type", "text"]);
    f.ok(
        "attribute-set",
        &["--element", PLATE, "--name", "Note", "--value", "kept"],
    );
    // The definition goes away; the value stays in the model and is shown as unknown.
    f.ok("attribute-undefine", &["--name", "Note"]);
    let get = f.get();
    let unknown = &list(&get, "attributes")[0];
    assert_eq!(text(unknown, "name"), "Note");
    assert!(!unknown.get("defined").unwrap().boolean().unwrap());
    assert_eq!(text(unknown, "usd_type"), "string");
    assert_eq!(text(unknown, "value"), "kept");
    // Ordinary edits still never drop it.
    f.ok("update-element", &["--id", PLATE, "--name", "Gusset B"]);
    assert_eq!(
        f.entries(),
        vec![("Note".to_owned(), "unknown:kept".to_owned())]
    );
    // It clears by name even without a definition.
    f.ok("attribute-clear", &["--element", PLATE, "--name", "Note"]);
    assert!(f.entries().is_empty());
    // A stored value is never shown twice after the attribute is re-defined with another type:
    // a number survives undefine, a text definition must not present it as its own.
    f.ok("attribute-define", &["--name", "Mass", "--type", "number"]);
    f.ok(
        "attribute-set",
        &["--element", PLATE, "--name", "Mass", "--value", "12.5"],
    );
    f.ok("attribute-undefine", &["--name", "Mass"]);
    f.ok("attribute-define", &["--name", "Mass", "--type", "text"]);
    let entries = f.entries();
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0].0, "Mass");
    assert!(entries[0].1.starts_with("unknown:"), "{entries:?}");
}

#[test]
fn a_value_is_carried_by_any_element_kind_and_only_its_own_group() {
    let f = Fixture::new();
    f.ok(
        "create-level",
        &["--name", "L1", "--elevation", "0", "--new-id", OTHER],
    );
    f.ok("attribute-define", &["--name", "Hot", "--type", "boolean"]);
    let set = f.ok(
        "attribute-set",
        &["--element", OTHER, "--name", "Hot", "--value", "true"],
    );
    assert_eq!(text(&set, "outcome"), "attribute_set");
    let get = f.ok("attribute-get", &["--element", OTHER]);
    let entries = list(&get, "attributes");
    assert_eq!(entries.len(), 1);
    assert!(entries[0].get("value").unwrap().boolean().unwrap());
}

#[test]
fn adversarial_inputs_are_refused() {
    let f = Fixture::new();
    f.ok(
        "attribute-define",
        &[
            "--name",
            "Finish",
            "--type",
            "choice",
            "--options",
            "plain,galvanized",
        ],
    );
    f.ok("attribute-define", &["--name", "Mass", "--type", "number"]);
    f.ok(
        "attribute-set",
        &["--element", PLATE, "--name", "Mass", "--value", "3"],
    );
    assert_eq!(
        f.rejected("attribute-define", &["--name", "Finish", "--type", "text"]),
        "DUPLICATE_ID"
    );
    assert_eq!(
        f.rejected("attribute-define", &["--name", "9x", "--type", "text"]),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.rejected(
            "attribute-define",
            &["--name", "Bad", "--type", "number", "--options", "a,b"]
        ),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.rejected("attribute-define", &["--name", "Bad", "--type", "choice"]),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.rejected("attribute-undefine", &["--name", "Missing"]),
        "NOT_FOUND"
    );
    assert_eq!(
        f.rejected(
            "attribute-set",
            &["--element", PLATE, "--name", "Nope", "--value", "x"]
        ),
        "NOT_FOUND"
    );
    assert_eq!(
        f.rejected(
            "attribute-set",
            &["--element", OTHER, "--name", "Mass", "--value", "1"]
        ),
        "NOT_FOUND"
    );
    assert_eq!(
        f.rejected(
            "attribute-set",
            &["--element", PLATE, "--name", "Mass", "--value", "abc"]
        ),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.rejected(
            "attribute-set",
            &["--element", PLATE, "--name", "Finish", "--value", "painted"]
        ),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.rejected(
            "attribute-set",
            &["--element", PLATE, "--name", "Mass", "--value", "NaN"]
        ),
        "NON_FINITE_NUMBER"
    );
    assert_eq!(
        f.rejected("attribute-clear", &["--element", PLATE, "--name", "Finish"]),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        f.rejected(
            "attributes",
            &["--op-id", "019a7bf0-0000-7000-8000-000000a0e09"]
        ),
        "INVALID_ARGUMENT"
    );
    // A no-op set of the stored value changes nothing.
    assert_eq!(
        f.rejected(
            "attribute-set",
            &["--element", PLATE, "--name", "Mass", "--value", "3"]
        ),
        "INVALID_ARGUMENT"
    );
}
