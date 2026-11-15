// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Q03 slice 2: `check-model` against the real binary — the geometric clash
//! rule and the three completeness rules, the coverage buckets, the
//! near-threshold band, the exit codes and the revision binding.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support::{self, at, list, text};

use beamcraft_commands::plate::inspect;
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
            shared: support::StoreFixture::new("model-checks"),
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
    fn material(&self) -> String {
        text(
            at(
                &self.ok("define-material", &["--density", "7850"]),
                &["definition"],
            ),
            "digest",
        )
    }
    /// A 1000x500 plate with the given thickness and world placement.
    fn plate(&self, material: &str, thickness: &str, translation: &str) -> String {
        let created = self.ok(
            "create-plate",
            &[
                "--width",
                "1000",
                "--length",
                "500",
                "--thickness",
                thickness,
                "--material-digest",
                material,
                "--translation",
                translation,
            ],
        );
        text(at(&created, &["plate"]), "element_id")
    }
    fn check(&self, extra: &[&str]) -> (i32, Json) {
        self.run("check-model", extra)
    }
    /// Commits a head-based revision with changed layer text and no content
    /// verification, the way a layer edited outside BeamCraft would arrive.
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

fn fail_count(reply: &Json, rule: &str) -> u64 {
    at(at(reply, &["counts"]).get(rule).unwrap(), &["fail"])
        .unsigned()
        .unwrap()
}

fn unknown_count(reply: &Json, rule: &str) -> u64 {
    at(at(reply, &["counts"]).get(rule).unwrap(), &["unknown"])
        .unsigned()
        .unwrap()
}

fn finding_of<'a>(reply: &'a Json, rule: &str) -> &'a Json {
    list(reply, "findings")
        .iter()
        .find(|f| text(f, "rule") == rule)
        .expect("the rule finding is missing from the reply")
}

fn element_layer(id: &str) -> String {
    format!("derived/{}.usda", id.replace('-', ""))
}

#[test]
fn a_deep_overlap_is_red_with_a_part_clash_finding_and_exit_two() {
    let f = Fixture::new();
    let steel = f.material();
    f.plate(&steel, "10", "0,0,0");
    f.plate(&steel, "10", "100,0,5");
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 2, "{reply:?}");
    assert_eq!(text(&reply, "outcome"), "model_checked");
    assert_eq!(text(&reply, "status"), "red");
    assert_eq!(text(at(&reply, &["scope"]), "mode"), "model");
    assert_eq!(fail_count(&reply, "PART_CLASH"), 1);
    let finding = finding_of(&reply, "PART_CLASH");
    assert_eq!(text(finding, "status"), "fail");
    assert_eq!(list(finding, "elements").len(), 2);
    let measured = at(finding, &["measured_mm"]).float().unwrap();
    assert!((measured - 5.0).abs() <= 1e-6, "{measured}");
    let tolerance = at(finding, &["tolerance_mm"]).float().unwrap();
    assert!((tolerance - 0.5).abs() <= 1e-9);
    // Both parts were checked, nothing unknown, no fix offered in this slice.
    assert_eq!(at(&reply, &["coverage", "checked"]).unsigned().unwrap(), 2);
    assert_eq!(at(&reply, &["coverage", "unknown"]).unsigned().unwrap(), 0);
    assert!(matches!(finding.get("fix"), Ok(Json::Null)));
    assert!(!text(at(&reply, &["coverage"]), "limitations").is_empty());
    // The reply names the checked revision: the committed head as the strict
    // reader sees it.
    let store = RevisionStore::new(BlobStore::new(f.shared.root.join("store")).unwrap());
    let head = store.open().unwrap().info().revision().to_string();
    assert_eq!(text(&reply, "revision_id"), head);
    let inspection = inspect(&store).unwrap();
    assert_eq!(inspection.revision.revision_id, head);
}

#[test]
fn an_exact_stack_is_green_face_contact_and_exits_zero() {
    let f = Fixture::new();
    let steel = f.material();
    f.plate(&steel, "10", "0,0,0");
    f.plate(&steel, "10", "0,0,10");
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 0, "{reply:?}");
    assert_eq!(text(&reply, "status"), "green");
    assert_eq!(list(&reply, "findings").len(), 0);
    assert_eq!(fail_count(&reply, "PART_CLASH"), 0);
}

#[test]
fn separated_plates_are_green_and_full_containment_is_red() {
    let f = Fixture::new();
    let steel = f.material();
    // Separated: clear, no clash.
    f.plate(&steel, "10", "0,0,0");
    f.plate(&steel, "10", "0,0,500");
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 0, "{reply:?}");
    assert_eq!(text(&reply, "status"), "green");
    // Containment: a thick base holds a smaller plate entirely inside.
    f.ok(
        "create-plate",
        &[
            "--width",
            "1000",
            "--length",
            "500",
            "--thickness",
            "100",
            "--material-digest",
            &steel,
        ],
    );
    let inserted = f.ok(
        "create-plate",
        &[
            "--width",
            "400",
            "--length",
            "300",
            "--thickness",
            "10",
            "--material-digest",
            &steel,
            "--translation",
            "100,100,3",
        ],
    );
    let _ = text(at(&inserted, &["plate"]), "element_id");
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 2, "{reply:?}");
    assert_eq!(text(&reply, "status"), "red");
    let contained = list(&reply, "findings")
        .iter()
        .find(|finding| text(finding, "message").contains("contained"))
        .unwrap_or_else(|| panic!("no contained finding in {reply:?}"));
    assert_eq!(text(contained, "rule"), "PART_CLASH");
}

#[test]
fn a_band_edge_overlap_is_unknown_yellow_and_exits_zero() {
    let f = Fixture::new();
    let steel = f.material();
    f.plate(&steel, "10", "0,0,0");
    f.plate(&steel, "10", "100,0,9.5");
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 0, "{reply:?}");
    assert_eq!(text(&reply, "status"), "unknown");
    assert_eq!(unknown_count(&reply, "PART_CLASH"), 1);
    let finding = finding_of(&reply, "PART_CLASH");
    assert_eq!(text(finding, "status"), "unknown");
    assert_eq!(list(finding, "missing_inputs").len(), 1);
    // Yellow still means not checked, and the process says so with exit 0.
    assert!(matches!(finding.get("measured_mm"), Ok(Json::Number(_))));
}

#[test]
fn a_part_without_material_is_red_by_completeness() {
    let f = Fixture::new();
    let created = f.ok(
        "create-plate",
        &[
            "--width",
            "1000",
            "--length",
            "500",
            "--thickness",
            "10",
            "--translation",
            "0,0,0",
        ],
    );
    let _ = text(at(&created, &["plate"]), "element_id");
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 2, "{reply:?}");
    assert_eq!(text(&reply, "status"), "red");
    let finding = finding_of(&reply, "PART_WITHOUT_MATERIAL");
    assert_eq!(text(finding, "status"), "fail");
    assert_eq!(fail_count(&reply, "PART_WITHOUT_MATERIAL"), 1);
}

#[test]
fn a_degenerate_plate_is_reported_and_covered_unknown() {
    let f = Fixture::new();
    let steel = f.material();
    f.ok(
        "create-plate",
        &[
            "--contour",
            "0,0,0;0,500,0;1000,500,0;1000,0,0",
            "--thickness",
            "10",
            "--material-digest",
            &steel,
        ],
    );
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 2, "{reply:?}");
    let finding = finding_of(&reply, "PART_DEGENERATE");
    assert_eq!(text(finding, "status"), "fail");
    assert!(text(finding, "message").contains("contour_winding_clockwise"));
    // The element is explicit in coverage: unknown, geometry unsupported.
    let rows = list(at(&reply, &["coverage"]), "elements");
    let row = rows.first().unwrap_or_else(|| panic!("no coverage rows"));
    assert_eq!(text(row, "bucket"), "unknown");
    assert_eq!(text(row, "reason"), "geometry_unsupported");
}

#[test]
fn a_tampered_quantity_is_a_stale_integrity_finding() {
    let f = Fixture::new();
    let steel = f.material();
    let id = f.plate(&steel, "10", "0,0,0");
    f.tamper(|name, text| {
        if name == element_layer(&id) {
            text.replace("bim:massKg = 39.25", "bim:massKg = 99.0")
        } else {
            text
        }
    });
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 2, "{reply:?}");
    let finding = finding_of(&reply, "QUANTITY_STALE");
    assert_eq!(text(finding, "status"), "fail");
    assert!(text(finding, "message").contains("integrity failure"));
    // The same reply names the revision the check identified.
    let head = store_revision(&f);
    assert_eq!(text(&reply, "revision_id"), head);
}

fn store_revision(f: &Fixture) -> String {
    let store = RevisionStore::new(BlobStore::new(f.shared.root.join("store")).unwrap());
    store.open().unwrap().info().revision().to_string()
}

#[test]
fn element_scope_narrows_the_checks_and_unknown_ids_are_refused() {
    let f = Fixture::new();
    let steel = f.material();
    let base = f.plate(&steel, "10", "0,0,0");
    let clashed = f.plate(&steel, "10", "100,0,5");
    let far = f.plate(&steel, "10", "0,0,500");
    // An assembly over the clashing pair gives the pair a common subtree.
    let assembly = text(&f.ok("create-assembly", &["--name", "Bay"]), "element_id");
    f.ok(
        "attach-to-assembly",
        &["--id", &base, "--assembly-id", &assembly],
    );
    f.ok(
        "attach-to-assembly",
        &["--id", &clashed, "--assembly-id", &assembly],
    );
    // The assembly's subtree contains the clashing pair: the clash is found.
    let (code, reply) = f.check(&["--element", &assembly]);
    assert_eq!(code, 2, "{reply:?}");
    assert_eq!(text(at(&reply, &["scope"]), "mode"), "element_subtree");
    assert_eq!(text(at(&reply, &["scope"]), "element"), assembly);
    assert_eq!(fail_count(&reply, "PART_CLASH"), 1);
    // The subtree holds the assembly and its two parts; the far plate is
    // outside the scope and absent from the coverage.
    assert_eq!(at(&reply, &["scope", "elements"]).unsigned().unwrap(), 3);
    let rows = list(at(&reply, &["coverage"]), "elements");
    assert_eq!(rows.len(), 3);
    // A part's scope resolves to its assembly: the same subtree, same clash.
    let (code, reply) = f.check(&["--element", &base]);
    assert_eq!(code, 2, "{reply:?}");
    assert_eq!(fail_count(&reply, "PART_CLASH"), 1);
    // The far plate's own subtree has no clash and checks one element.
    let (code, reply) = f.check(&["--element", &far]);
    assert_eq!(code, 0, "{reply:?}");
    assert_eq!(fail_count(&reply, "PART_CLASH"), 0);
    assert_eq!(at(&reply, &["coverage", "checked"]).unsigned().unwrap(), 1);
    // An id the revision does not carry is a refusal, not an empty report.
    let (code, body) = f.check(&["--element", "019a7bf0-0000-7000-8000-00000000dead"]);
    assert_eq!(code, 2);
    assert_eq!(text(&body, "outcome"), "rejected");
    assert_eq!(text(&body, "code"), "NOT_FOUND");
}

#[test]
fn pagination_pages_findings_only_and_counts_cover_the_scope() {
    let f = Fixture::new();
    let steel = f.material();
    f.plate(&steel, "10", "0,0,0");
    f.plate(&steel, "10", "100,0,5");
    let (code, reply) = f.check(&["--limit", "1", "--offset", "0"]);
    assert_eq!(code, 2, "{reply:?}");
    // The findings page holds one finding; the status and counts stay whole.
    assert_eq!(list(&reply, "findings").len(), 1);
    assert_eq!(at(&reply, &["findings_total"]).unsigned().unwrap(), 1);
    assert_eq!(text(&reply, "status"), "red");
    assert_eq!(fail_count(&reply, "PART_CLASH"), 1);
    // Paging past the findings shows an empty page and the same red status.
    let (code, reply) = f.check(&["--limit", "1", "--offset", "1"]);
    assert_eq!(code, 2, "{reply:?}");
    assert_eq!(list(&reply, "findings").len(), 0);
    assert_eq!(text(&reply, "status"), "red");
    // A zero or oversized limit is refused like every listing.
    let (code, body) = f.check(&["--limit", "0"]);
    assert_eq!(code, 2);
    assert_eq!(text(&body, "outcome"), "rejected");
}
