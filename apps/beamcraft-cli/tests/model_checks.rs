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
use beamcraft_json::{Json, Limits};

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
    /// Declares one recorded input from its payload options; returns the element id.
    fn record(&self, name: &str, extra: &[&str]) -> String {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let id = format!("019a7bf0-0000-7000-8000-{n:012x}");
        let mut args: Vec<&str> = vec!["--name", name, "--new-id", &id];
        args.extend(extra);
        let created = self.ok("record-create", &args);
        text(&created, "element_id")
    }
    /// Declares one or more weld or contact connections into a joint intent
    /// record: the connections ride one `--connection` value, comma-separated,
    /// the form the command accepts (the option parser refuses a repeated flag).
    fn intent(&self, scope: &[&str], connections: &[&str]) -> String {
        let scope_text = scope.join(",");
        let connection_text = connections.join(",");
        let mut extra: Vec<&str> = vec!["--kind", "joint_intent", "--scope", &scope_text];
        extra.extend(["--connection", &connection_text]);
        self.record("joint intent", &extra)
    }
    /// Records the permitted local gap of one declared weld connection.
    fn gap(&self, a: &str, b: &str, min: &str, max: &str, tolerance: &str) -> String {
        self.record(
            "gap requirement",
            &[
                "--kind",
                "gap_requirement",
                "--gap-a",
                a,
                "--gap-b",
                b,
                "--gap-min",
                min,
                "--gap-max",
                max,
                "--gap-tolerance",
                tolerance,
            ],
        )
    }
    /// Creates one A06 weld without a path between the two parts.
    fn weld(&self, main: &str, secondary: &str) -> String {
        let created = self.ok(
            "create-weld",
            &[
                "--name",
                "weld",
                "--main",
                main,
                "--secondary",
                secondary,
                "--size",
                "6",
                "--location",
                "workshop",
                "--side",
                "arrow",
            ],
        );
        text(&created, "element_id")
    }
    /// Creates one A06 weld with an authored world path between the two
    /// parts: the placed weld the local-attachment validation can locate.
    fn path_weld(&self, main: &str, secondary: &str, path: &str) -> String {
        self.sized_path_weld(main, secondary, "6", path)
    }
    /// The same, with the recorded weld leg chosen by the test.
    fn sized_path_weld(&self, main: &str, secondary: &str, size: &str, path: &str) -> String {
        let created = self.ok(
            "create-weld",
            &[
                "--name",
                "weld",
                "--main",
                main,
                "--secondary",
                secondary,
                "--size",
                size,
                "--location",
                "workshop",
                "--side",
                "arrow",
                "--path",
                path,
            ],
        );
        text(&created, "element_id")
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
    // The measurement carries the mesh approximation bound it read and the
    // effective comparison tolerance δ = max(bound, t/2) it applied (contract
    // sections 7.2 and 7.3): flat-faced rectangles keep a zero bound, so δ
    // is the tolerance half.
    let bound = at(finding, &["mesh_bound_mm"]).float().unwrap();
    assert!(bound.abs() <= 1e-9, "{bound}");
    let delta = at(finding, &["effective_tolerance_mm"]).float().unwrap();
    assert!((delta - 0.25).abs() <= 1e-9, "{delta}");
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
fn without_joint_intent_the_connectivity_rules_are_not_applicable() {
    let f = Fixture::new();
    let steel = f.material();
    f.plate(&steel, "10", "0,0,0");
    f.plate(&steel, "10", "0,0,10");
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 0, "{reply:?}");
    assert_eq!(text(&reply, "status"), "green");
    // The connectivity rules are in the counts and stay silent...
    for rule in [
        "PART_UNCONNECTED",
        "WELD_WITHOUT_CONTACT",
        "CONTACT_WITHOUT_CONNECTION",
    ] {
        assert_eq!(fail_count(&reply, rule), 0, "{rule}");
        assert_eq!(unknown_count(&reply, rule), 0, "{rule}");
    }
    // ...and the coverage paragraph names the established absence: no
    // recorded joint intent, so the rules are not applicable (contract 5).
    let limitations = text(at(&reply, &["coverage"]), "limitations");
    assert!(
        limitations.contains("no joint intent is recorded"),
        "{limitations}"
    );
    assert!(limitations.contains("not applicable"), "{limitations}");
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
    // A finding without a measurement keeps its shape: no bound, no applied δ.
    assert!(matches!(finding.get("mesh_bound_mm"), Ok(Json::Null)));
    assert!(matches!(
        finding.get("effective_tolerance_mm"),
        Ok(Json::Null)
    ));
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

fn reason_of(reply: &Json, rule: &str) -> String {
    list(finding_of(reply, rule), "missing_inputs")
        .first()
        .unwrap()
        .string()
        .unwrap()
        .to_owned()
}

#[test]
fn a_subtree_check_is_quiet_about_declared_members_outside_it() {
    // A narrowed check of one joint member stays quiet about its other
    // members: they exist and are evaluated in the model, so the connection
    // is simply outside the checked scope, not unresolved.
    let f = Fixture::new();
    let steel = f.material();
    let base = f.plate(&steel, "10", "0,0,0");
    let column = f.plate(&steel, "10", "0,0,10");
    f.intent(&[&base, &column], &[&format!("{base}:contact:{column}")]);
    let (code, reply) = f.check(&["--element", &base]);
    assert_eq!(code, 0, "{reply:?}");
    assert_eq!(text(&reply, "status"), "green");
    assert_eq!(list(&reply, "findings").len(), 0);
    // The ghost is a different case: declared and absent from the model, so
    // the same subtree check reports it unresolved.
    let f = Fixture::new();
    let steel = f.material();
    let base = f.plate(&steel, "10", "0,0,0");
    let column = f.plate(&steel, "10", "0,0,10");
    let ghost = "019a7bf0-0000-7000-8000-00000000dead";
    f.intent(&[&base, &column, ghost], &[&format!("{base}:weld:{ghost}")]);
    let (code, reply) = f.check(&["--element", &base]);
    assert_eq!(code, 0, "{reply:?}");
    assert_eq!(text(&reply, "status"), "unknown");
    assert_eq!(reason_of(&reply, "PART_UNCONNECTED"), "target_unresolved");
}

#[test]
fn one_connection_option_carries_several_comma_separated_connections() {
    // Several declared connections ride ONE `--connection` value, comma
    // separated (`UUID:contact:UUID,UUID:contact:UUID`) — the option parser
    // refuses a repeated flag, so the comma form is the only multi-connection
    // form. The record keeps every declared connection, and the check judges
    // them all: the satisfied pair stays quiet, the separated pair turns the
    // check red.
    let f = Fixture::new();
    let steel = f.material();
    let base = f.plate(&steel, "10", "0,0,0");
    let column = f.plate(&steel, "10", "0,0,10");
    let far = f.plate(&steel, "10", "0,0,200");
    f.intent(
        &[&base, &column, &far],
        &[&format!("{base}:contact:{column},{base}:contact:{far}")],
    );
    let listed = f.run("record-list", &[]).1;
    let rows = list(&listed, "records");
    assert_eq!(rows.len(), 1);
    assert_eq!(list(rows.first().unwrap(), "connections").len(), 2);
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 2, "{reply:?}");
    assert_eq!(text(&reply, "status"), "red");
    assert!(fail_count(&reply, "PART_UNCONNECTED") >= 1, "{reply:?}");
}

#[test]
fn a_declared_contact_over_touching_parts_is_green() {
    // The connected variant of the sample column base: the owner declares the
    // parts in contact, and they touch within the product tolerance.
    let f = Fixture::new();
    let steel = f.material();
    let base = f.plate(&steel, "10", "0,0,0");
    let column = f.plate(&steel, "10", "0,0,10");
    f.intent(&[&base, &column], &[&format!("{base}:contact:{column}")]);
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 0, "{reply:?}");
    assert_eq!(text(&reply, "status"), "green");
    assert_eq!(fail_count(&reply, "PART_UNCONNECTED"), 0);
    // The coverage paragraph states the declared intent, not an absence.
    let limitations = text(at(&reply, &["coverage"]), "limitations");
    assert!(
        limitations.contains("declared joint intent"),
        "{limitations}"
    );
    // The record itself is listed and reports its payload.
    let listed = f.run("record-list", &[]).1;
    assert_eq!(text(&listed, "outcome"), "records");
    let rows = list(&listed, "records");
    assert_eq!(rows.len(), 1);
    assert_eq!(text(rows.first().unwrap(), "kind"), "joint_intent");
    assert_eq!(
        text(
            at(
                list(rows.first().unwrap(), "connections").first().unwrap(),
                &[]
            ),
            "kind"
        ),
        "contact"
    );
}

#[test]
fn a_declared_contact_over_separated_parts_is_red_part_unconnected() {
    // The unconnected variant: the declared contact is absent, 2 mm of air.
    let f = Fixture::new();
    let steel = f.material();
    let base = f.plate(&steel, "10", "0,0,0");
    let column = f.plate(&steel, "10", "0,0,12");
    f.intent(&[&base, &column], &[&format!("{base}:contact:{column}")]);
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 2, "{reply:?}");
    assert_eq!(text(&reply, "status"), "red");
    let finding = finding_of(&reply, "PART_UNCONNECTED");
    assert_eq!(text(finding, "status"), "fail");
    let measured = at(finding, &["measured_mm"]).float().unwrap();
    assert!((measured - 2.0).abs() <= 1e-6, "{measured}");
    // Both declared parts are named.
    assert_eq!(list(finding, "elements").len(), 2);
}

#[test]
fn a_weld_present_without_contact_stays_unknown_by_the_recorded_rule() {
    // The weld-present-without-contact variant: a declared weld connection
    // whose weld exists but whose parts are apart. With a recorded gap
    // requirement the local gap is judged — the pathless A06 weld is never
    // locally attached, so the gap stays unknown (attachment_unvalidated).
    let f = Fixture::new();
    let steel = f.material();
    let base = f.plate(&steel, "10", "0,0,0");
    let column = f.plate(&steel, "10", "0,0,12");
    f.intent(&[&base, &column], &[&format!("{base}:weld:{column}")]);
    f.weld(&base, &column);
    f.gap(&base, &column, "0", "0.2", "0.05");
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 0, "{reply:?}");
    assert_eq!(text(&reply, "status"), "unknown");
    let finding = finding_of(&reply, "WELD_WITHOUT_CONTACT");
    assert_eq!(text(finding, "status"), "unknown");
    assert_eq!(
        reason_of(&reply, "WELD_WITHOUT_CONTACT"),
        "attachment_unvalidated"
    );
    // The weld exists, so the connection is satisfied: no PART_UNCONNECTED.
    let unconnected = list(&reply, "findings")
        .iter()
        .filter(|finding| text(finding, "rule") == "PART_UNCONNECTED")
        .count();
    assert_eq!(unconnected, 0);
    // Without a recorded gap requirement the same joint stays unknown for a
    // different reason: no supported rule decides the local gap.
    let f = Fixture::new();
    let steel = f.material();
    let base = f.plate(&steel, "10", "0,0,0");
    let column = f.plate(&steel, "10", "0,0,12");
    f.intent(&[&base, &column], &[&format!("{base}:weld:{column}")]);
    f.weld(&base, &column);
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 0, "{reply:?}");
    assert_eq!(
        reason_of(&reply, "WELD_WITHOUT_CONTACT"),
        "no_supported_rule"
    );
}

#[test]
fn a_lofted_plate_the_check_cannot_evaluate_is_unknown_not_green() {
    // The #726 case: a lofted plate driven straight through another plate is
    // a real part the check cannot evaluate — the model is unknown with the
    // part covered explicitly, never silently green (contract sections 6 and
    // 8.1: a solid part the check cannot evaluate is not an established
    // absence).
    let f = Fixture::new();
    let steel = f.material();
    f.plate(&steel, "10", "0,0,0");
    let created = f.ok(
        "create-lofted-plate",
        &[
            "--start",
            "100,100,-50;300,100,-50;300,300,-50;100,300,-50",
            "--end",
            "100,100,80;300,100,80;300,300,80;100,300,80",
            "--material-digest",
            &steel,
        ],
    );
    let loft = at(&created, &["plate", "id"]).string().unwrap();
    // The write reply's own summary is not green either.
    assert_eq!(text(at(&created, &["checks"]), "status"), "unknown");
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 0, "{reply:?}");
    assert_eq!(text(&reply, "status"), "unknown");
    // The lofted plate's coverage row names the unsupported geometry.
    let row = list(at(&reply, &["coverage"]), "elements")
        .iter()
        .find(|row| text(row, "id") == loft)
        .unwrap_or_else(|| panic!("no coverage row for {loft} in {reply:?}"));
    assert_eq!(text(row, "bucket"), "unknown");
    assert_eq!(text(row, "reason"), "geometry_unsupported");
    // The evaluated plate stays checked.
    assert_eq!(at(&reply, &["coverage", "checked"]).unsigned().unwrap(), 1);
}

#[test]
fn connection_elements_stay_not_applicable_and_records_are_not_parts() {
    // The not_applicable bucket stays an established absence with a reason
    // that fits: a weld or bolt group is a connection element the check does
    // not judge yet (not_a_clash_subject_yet), and a record is not a model
    // part at all (not_a_part) — no solid part lands in either (contract
    // sections 6 and 8.1).
    let f = Fixture::new();
    let steel = f.material();
    let base = f.plate(&steel, "10", "0,0,0");
    let column = f.plate(&steel, "10", "0,0,10");
    let intent = f.intent(&[&base, &column], &[&format!("{base}:weld:{column}")]);
    let weld = f.weld(&base, &column);
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 0, "{reply:?}");
    let rows = list(at(&reply, &["coverage"]), "elements");
    let row_of = |id: &str| {
        rows.iter()
            .find(|row| text(row, "id") == id)
            .unwrap_or_else(|| panic!("no coverage row for {id} in {reply:?}"))
    };
    let weld_row = row_of(&weld);
    assert_eq!(text(weld_row, "bucket"), "not_applicable");
    assert_eq!(text(weld_row, "reason"), "not_a_clash_subject_yet");
    let intent_row = row_of(&intent);
    assert_eq!(text(intent_row, "bucket"), "not_applicable");
    assert_eq!(text(intent_row, "reason"), "not_a_part");
    // Both real parts stay checked.
    assert_eq!(at(&reply, &["coverage", "checked"]).unsigned().unwrap(), 2);
}

#[test]
fn an_attached_weld_bridging_an_excessive_gap_turns_the_joint_red() {
    // The full path of the gap rule at the CLI (#724, closes the audit gap):
    // a pathed weld attached at the declared contact, a recorded gap
    // requirement, and a bridged opening beyond the permitted maximum —
    // the joint is red with the measured value and the named bounds. The
    // 3 mm opening is well beyond the contact tolerance: local attachment
    // does not cap the measurable gap there (contract section 5.1).
    let f = Fixture::new();
    let steel = f.material();
    let base = f.plate(&steel, "10", "0,0,0");
    let top = f.plate(&steel, "10", "0,0,13");
    f.intent(&[&base, &top], &[&format!("{base}:weld:{top}")]);
    f.path_weld(&base, &top, "100,100,11.5;300,100,11.5");
    f.gap(&base, &top, "0", "2", "0.3");
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 2, "{reply:?}");
    assert_eq!(text(&reply, "status"), "red");
    let finding = finding_of(&reply, "WELD_WITHOUT_CONTACT");
    assert_eq!(text(finding, "status"), "fail");
    let measured = at(finding, &["measured_mm"]).float().unwrap();
    assert!((measured - 3.0).abs() <= 1e-6, "{measured}");
    let bounds = list(finding, "bounds");
    assert_eq!(bounds.len(), 2);
    assert_eq!(text(&bounds[0], "name"), "permitted_min");
    assert_eq!(text(&bounds[1], "name"), "permitted_max");
    assert_eq!(at(&bounds[1], &["bound_mm"]).float().unwrap(), 2.0);
    assert!(text(finding, "message").contains("exceeds the permitted maximum"));
    // The measurement carries the mesh approximation bound it read and the
    // effective comparison tolerance δ = max(bound, t/2) it applied
    // (contract sections 7.2 and 7.3): flat-faced plates keep a zero bound.
    let bound = at(finding, &["mesh_bound_mm"]).float().unwrap();
    assert!(bound.abs() <= 1e-9, "{bound}");
    let delta = at(finding, &["effective_tolerance_mm"]).float().unwrap();
    assert!((delta - 0.25).abs() <= 1e-9, "{delta}");
}

#[test]
fn a_tight_fit_and_an_in_range_gap_pass_with_a_path_at_the_joint() {
    // A tight fit (measured gap zero) passes the range [0, 2] with a small
    // recorded tolerance: the lower band edge at or below zero cannot
    // straddle (contract section 7.3, #724).
    let f = Fixture::new();
    let steel = f.material();
    let base = f.plate(&steel, "10", "0,0,0");
    let top = f.plate(&steel, "10", "0,0,10");
    f.intent(&[&base, &top], &[&format!("{base}:weld:{top}")]);
    f.path_weld(&base, &top, "100,100,10;300,100,10");
    f.gap(&base, &top, "0", "2", "0.05");
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 0, "{reply:?}");
    assert_eq!(text(&reply, "status"), "green");
    assert_eq!(list(&reply, "findings").len(), 0);
    // A 1.5 mm opening inside the permitted range passes too, judged from
    // the path that spans the contact.
    let f = Fixture::new();
    let steel = f.material();
    let base = f.plate(&steel, "10", "0,0,0");
    let top = f.plate(&steel, "10", "0,0,11.5");
    f.intent(&[&base, &top], &[&format!("{base}:weld:{top}")]);
    f.path_weld(&base, &top, "100,100,10.75;300,100,10.75");
    f.gap(&base, &top, "0", "2", "0.3");
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 0, "{reply:?}");
    assert_eq!(text(&reply, "status"), "green");
    assert_eq!(list(&reply, "findings").len(), 0);
}

#[test]
fn a_weld_path_away_from_the_joint_stays_unknown_attachment_unvalidated() {
    // The path runs in the open air beside the plate edge: it neither
    // touches the surfaces nor spans the contact, so the weld is not
    // locally attached and its gap stays unjudged while the rule keeps
    // reporting the unvalidated attachment.
    let f = Fixture::new();
    let steel = f.material();
    let base = f.plate(&steel, "10", "0,0,0");
    let top = f.plate(&steel, "10", "0,0,13");
    f.intent(&[&base, &top], &[&format!("{base}:weld:{top}")]);
    f.path_weld(&base, &top, "1100,250,11.5;1200,250,11.5");
    f.gap(&base, &top, "0", "2", "0.3");
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 0, "{reply:?}");
    assert_eq!(text(&reply, "status"), "unknown");
    assert_eq!(
        reason_of(&reply, "WELD_WITHOUT_CONTACT"),
        "attachment_unvalidated"
    );
}

#[test]
fn contact_without_connection_is_red_where_a_weld_is_declared() {
    // The contact-without-connection variant: the parts touch and a weld is
    // declared, but none exists. Contact alone never proves a weld; the
    // declared one is missing.
    let f = Fixture::new();
    let steel = f.material();
    let base = f.plate(&steel, "10", "0,0,0");
    let column = f.plate(&steel, "10", "0,0,10");
    f.intent(&[&base, &column], &[&format!("{base}:weld:{column}")]);
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 2, "{reply:?}");
    assert_eq!(text(&reply, "status"), "red");
    let unconnected = finding_of(&reply, "PART_UNCONNECTED");
    assert_eq!(text(unconnected, "status"), "fail");
    let contacted = finding_of(&reply, "CONTACT_WITHOUT_CONNECTION");
    assert_eq!(text(contacted, "status"), "fail");
}

#[test]
fn an_unresolved_declared_target_stays_unknown_and_named() {
    // A declared connection to an element that never existed (or was
    // deleted after the declaration) is unresolved: unknown with the
    // target named, the record kept, never auto-repaired.
    let f = Fixture::new();
    let steel = f.material();
    let base = f.plate(&steel, "10", "0,0,0");
    let column = f.plate(&steel, "10", "0,0,10");
    let ghost = "019a7bf0-0000-7000-8000-00000000dead";
    f.intent(&[&base, &column, ghost], &[&format!("{base}:weld:{ghost}")]);
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 0, "{reply:?}");
    assert_eq!(text(&reply, "status"), "unknown");
    let finding = finding_of(&reply, "PART_UNCONNECTED");
    assert_eq!(text(finding, "status"), "unknown");
    assert_eq!(reason_of(&reply, "PART_UNCONNECTED"), "target_unresolved");
    assert!(
        text(finding, "message").contains("evaluated parts"),
        "{reply:?}"
    );
}

/// A standing plate: width along world X, height along world Z (a quarter
/// turn about X), thickness running along world −Y.
impl Fixture {
    fn standing(
        &self,
        steel: &str,
        width: &str,
        length: &str,
        thickness: &str,
        translation: &str,
        corners: &[&str],
    ) -> String {
        let corner_text = corners.join(";");
        let mut args: Vec<&str> = vec![
            "--width",
            width,
            "--length",
            length,
            "--thickness",
            thickness,
            "--material-digest",
            steel,
            "--orientation-xyzw",
            "0.7071067811865476,0,0,0.7071067811865476",
            "--translation",
            translation,
        ];
        if !corners.is_empty() {
            args.extend(["--corners", &corner_text]);
        }
        let created = self.ok("create-plate", &args);
        text(at(&created, &["plate"]), "element_id")
    }
    /// The sample column base: the base plate on the floor, the column
    /// standing on it with its face at x = 150, and the rib perpendicular to
    /// that face with its welded edge on it and its bottom on the plate. The
    /// standing parts sit one nanometre clear of their neighbours, so the
    /// quarter-turn rounding never dips a corner into them and the pair
    /// relations stay determinate.
    fn joint(&self, steel: &str, rib_corners: &[&str]) -> (String, String, String) {
        let base = self.plate(steel, "10", "0,0,0");
        let column = self.standing(steel, "100", "200", "40", "50,40,10.000000001", &[]);
        let rib = self.standing(
            steel,
            "80",
            "100",
            "10",
            "150.000000001,10,10.000000001",
            rib_corners,
        );
        (base, column, rib)
    }
    /// Records the corner-cut requirement of the sample's rib corner: the
    /// owner's 10 mm cut with a 1 mm tolerance unless overridden.
    fn corner(&self, rib: &str, base: &str, column: &str, extra: &[&str]) -> String {
        let neighbours = format!("{base},{column}");
        let mut extra_args: Vec<&str> = vec![
            "--kind",
            "corner_cut",
            "--target",
            rib,
            "--neighbours",
            &neighbours,
            "--required-mm",
            "10",
            "--tolerance-mm",
            "1",
        ];
        extra_args.extend(extra);
        self.record("corner cut", &extra_args)
    }
    /// The sample rib from an authored local contour (the corner cut rides the
    /// contour points, no corner treatment): the same standing placement as
    /// [`Fixture::joint`]'s rib.
    fn standing_contour(&self, steel: &str, contour: &str, translation: &str) -> String {
        let created = self.ok(
            "create-plate",
            &[
                "--contour",
                contour,
                "--thickness",
                "10",
                "--material-digest",
                steel,
                "--orientation-xyzw",
                "0.7071067811865476,0,0,0.7071067811865476",
                "--translation",
                translation,
            ],
        );
        text(at(&created, &["plate"]), "element_id")
    }
    /// Cuts the rib's lower corner triangle off by plane, in the rib's element
    /// frame: `leg` is the setback along both contour edges from the corner.
    fn cut_rib_corner_by_plane(&self, rib: &str, leg: &str, operation: &str) -> Json {
        let point = format!("{leg},0,0");
        self.ok(
            "cut-by-plane",
            &[
                "--id",
                rib,
                "--operation-id",
                operation,
                "--point",
                &point,
                "--normal",
                "0.7071067811865476,0.7071067811865476,0",
                "--keep",
                "positive",
            ],
        )
    }
    /// Cuts the rib's lower corner off as an A13 chamfer-part fabrication.
    fn chamfer_rib_part(&self, rib: &str, first: &str, second: &str, operation: &str) -> Json {
        self.ok(
            "chamfer-part",
            &[
                "--id",
                rib,
                "--operation-id",
                operation,
                "--vertex-index",
                "0",
                "--first-distance",
                first,
                "--second-distance",
                second,
            ],
        )
    }
}

#[test]
fn without_a_recorded_requirement_the_corner_rules_are_not_applicable() {
    let f = Fixture::new();
    let steel = f.material();
    let _ = f.joint(&steel, &[]);
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 0, "{reply:?}");
    assert_eq!(text(&reply, "status"), "green");
    assert_eq!(fail_count(&reply, "CORNER_NOT_CUT"), 0);
    assert_eq!(unknown_count(&reply, "CORNER_NOT_CUT"), 0);
    let limitations = text(at(&reply, &["coverage"]), "limitations");
    assert!(
        limitations.contains("no corner-cut requirement is recorded"),
        "{limitations}"
    );
    assert!(limitations.contains("not applicable"), "{limitations}");
}

#[test]
fn the_record_vehicle_carries_the_corner_cut_end_to_end() {
    let f = Fixture::new();
    let steel = f.material();
    let (base, column, rib) = f.joint(&steel, &["0=chamfer:10"]);
    let requirement = f.corner(&rib, &base, &column, &[]);
    // The record lists the recorded values back.
    let listed = f.run("record-list", &[]).1;
    let rows = list(&listed, "records");
    let row = rows
        .iter()
        .find(|row| text(row, "kind") == "corner_cut")
        .unwrap_or_else(|| panic!("no corner_cut record in {listed:?}"));
    assert_eq!(text(row, "id"), requirement);
    assert_eq!(text(row, "target"), rib);
    let neighbours = list(row, "neighbours");
    assert_eq!(neighbours.len(), 2);
    assert_eq!(neighbours.first().unwrap().string().unwrap(), base);
    assert_eq!(neighbours.get(1).unwrap().string().unwrap(), column);
    let required = at(row, &["required_mm"]).float().unwrap();
    assert!((required - 10.0).abs() <= 1e-9, "{required}");
    assert_eq!(text(row, "purpose"), "clearance");
    assert!(matches!(row.get("upper_bound_mm"), Ok(Json::Null)));
    // The correct 10 mm cut is green against the recorded requirement.
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 0, "{reply:?}");
    assert_eq!(text(&reply, "status"), "green");
    assert_eq!(fail_count(&reply, "CORNER_NOT_CUT"), 0);
    assert_eq!(unknown_count(&reply, "CORNER_NOT_CUT"), 0);
    assert_eq!(unknown_count(&reply, "CORNER_OVERCUT"), 0);
    let limitations = text(at(&reply, &["coverage"]), "limitations");
    assert!(
        limitations.contains("corner-cut rules ran over 1 recorded requirement"),
        "{limitations}"
    );
}

#[test]
fn the_model_without_the_cut_is_red_and_the_fix_hint_reaches_green() {
    // The acceptance red case: the uncut rib against the recorded 10 mm
    // requirement is red "no corner cut", measured 0 mm, and the finding
    // carries the one unambiguous repair; applying it and rechecking lands
    // green (contract section 9: apply one fix, then recheck).
    let f = Fixture::new();
    let steel = f.material();
    let (base, column, rib) = f.joint(&steel, &[]);
    f.corner(&rib, &base, &column, &[]);
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 2, "{reply:?}");
    assert_eq!(text(&reply, "status"), "red");
    let finding = finding_of(&reply, "CORNER_NOT_CUT");
    assert_eq!(text(finding, "status"), "fail");
    let measured = at(finding, &["measured_mm"]).float().unwrap();
    assert!(measured.abs() <= 1e-9, "{measured}");
    assert!(text(finding, "message").contains("too little clearance"));
    // The fix: set the required chamfer at the authored corner vertex.
    let fix = at(finding, &["fix"]);
    assert_eq!(text(fix, "command"), "set-plate-corner");
    let arguments = list(fix, "arguments");
    assert_eq!(
        arguments
            .iter()
            .map(|a| a.string().unwrap().to_owned())
            .collect::<Vec<_>>(),
        vec![
            "--id".into(),
            rib.clone(),
            "--vertex".into(),
            "0".into(),
            "--corner".into(),
            "chamfer:10".into(),
        ]
    );
    assert!(text(fix, "effect").contains("produces the required final shape"));
    // Apply one fix and recheck: green, no findings.
    let argument_refs = arguments
        .iter()
        .map(|a| a.string().unwrap().to_owned())
        .collect::<Vec<_>>();
    let refs: Vec<&str> = argument_refs.iter().map(String::as_str).collect();
    f.ok("set-plate-corner", &refs);
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 0, "{reply:?}");
    assert_eq!(text(&reply, "status"), "green");
    assert_eq!(list(&reply, "findings").len(), 0);
}

#[test]
fn a_twenty_millimetre_cut_is_overcut_with_measured_and_required() {
    // The acceptance red case: 20 mm instead of the recorded 10 mm is
    // CORNER_OVERCUT, measured 20 mm and required 10 mm, subject to the
    // recorded tolerance; no fix is offered.
    let f = Fixture::new();
    let steel = f.material();
    let (base, column, rib) = f.joint(&steel, &["0=chamfer:20"]);
    f.corner(&rib, &base, &column, &[]);
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 2, "{reply:?}");
    assert_eq!(text(&reply, "status"), "red");
    let finding = finding_of(&reply, "CORNER_OVERCUT");
    assert_eq!(text(finding, "status"), "fail");
    let measured = at(finding, &["measured_mm"]).float().unwrap();
    assert!((measured - 20.0).abs() <= 1e-9, "{measured}");
    let required = at(finding, &["required_mm"]).float().unwrap();
    assert!((required - 10.0).abs() <= 1e-9, "{required}");
    // The corner family reports its bound and applied δ too (contract
    // sections 7.2 and 7.3): the joint's flat faces keep a zero bound.
    let bound = at(finding, &["mesh_bound_mm"]).float().unwrap();
    assert!(bound.abs() <= 1e-9, "{bound}");
    let delta = at(finding, &["effective_tolerance_mm"]).float().unwrap();
    assert!((delta - 0.25).abs() <= 1e-9, "{delta}");
    let bounds = list(finding, "bounds");
    assert_eq!(bounds.len(), 3);
    assert_eq!(text(bounds.first().unwrap(), "name"), "required");
    let values: Vec<&Json> = bounds
        .iter()
        .filter(|b| {
            text(b, "name").starts_with("admitted_") || text(b, "name").starts_with("largest_")
        })
        .collect();
    assert_eq!(values.len(), 2);
    for value in values {
        assert_eq!(text(value, "source"), "measured");
    }
    assert!(matches!(finding.get("fix"), Ok(Json::Null)));
    // An undersized 5 mm cut stays CORNER_NOT_CUT with the leg reported.
    let f = Fixture::new();
    let steel = f.material();
    let (base, column, rib) = f.joint(&steel, &["0=chamfer:5"]);
    f.corner(&rib, &base, &column, &[]);
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 2, "{reply:?}");
    let finding = finding_of(&reply, "CORNER_NOT_CUT");
    assert_eq!(text(finding, "status"), "fail");
    let measured = at(finding, &["measured_mm"]).float().unwrap();
    assert!((measured - 5.0).abs() <= 1e-9, "{measured}");
}

#[test]
fn a_recorded_upper_bound_replaces_the_band_upper_edge_in_the_reply() {
    // A 12 mm cut exceeds the required band's upper edge (10 + 1) and is red
    // overcut; with the owner's standalone maximum of 12 ± 2 recorded, the
    // same cut passes: the bound replaces the band's upper edge.
    let f = Fixture::new();
    let steel = f.material();
    let (base, column, rib) = f.joint(&steel, &["0=chamfer:12"]);
    f.corner(&rib, &base, &column, &[]);
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 2, "{reply:?}");
    assert_eq!(text(finding_of(&reply, "CORNER_OVERCUT"), "status"), "fail");
    let f = Fixture::new();
    let steel = f.material();
    let (base, column, rib) = f.joint(&steel, &["0=chamfer:12"]);
    f.corner(
        &rib,
        &base,
        &column,
        &["--upper-bound-mm", "12", "--upper-bound-tolerance-mm", "2"],
    );
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 0, "{reply:?}");
    assert_eq!(text(&reply, "status"), "green");
    // The bound travels through the record vehicle.
    let listed = f.run("record-list", &[]).1;
    let row = list(&listed, "records")
        .iter()
        .find(|row| text(row, "kind") == "corner_cut")
        .unwrap_or_else(|| panic!("no corner_cut record in {listed:?}"));
    let bound = at(row, &["upper_bound_mm"]).float().unwrap();
    assert!((bound - 12.0).abs() <= 1e-9, "{bound}");
}

#[test]
fn a_ghost_corner_target_stays_unresolved_and_bad_input_refuses() {
    // The requirement naming an element the model does not carry is the
    // ghost case: judged, unknown (target_unresolved), the requirement kept;
    // nothing auto-retargets.
    let f = Fixture::new();
    let steel = f.material();
    let (base, column, rib) = f.joint(&steel, &["0=chamfer:10"]);
    let ghost = "019a7bf0-0000-7000-8000-00000000dead";
    let neighbours = format!("{base},{column}");
    f.record(
        "corner cut",
        &[
            "--kind",
            "corner_cut",
            "--target",
            ghost,
            "--neighbours",
            &neighbours,
            "--required-mm",
            "10",
            "--tolerance-mm",
            "1",
        ],
    );
    let _ = rib;
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 0, "{reply:?}");
    assert_eq!(text(&reply, "status"), "unknown");
    let finding = finding_of(&reply, "CORNER_NOT_CUT");
    assert_eq!(text(finding, "status"), "unknown");
    assert_eq!(reason_of(&reply, "CORNER_NOT_CUT"), "target_unresolved");
    assert!(text(finding, "message").contains("stays recorded"));
    // The command refuses a corner-cut requirement without its target.
    let (code, body) = f.run(
        "record-create",
        &[
            "--name",
            "corner cut",
            "--kind",
            "corner_cut",
            "--neighbours",
            &neighbours,
            "--required-mm",
            "10",
            "--tolerance-mm",
            "1",
            "--new-id",
            "019a7bf0-0000-7000-8000-00000000feed",
        ],
    );
    assert_eq!(code, 2);
    assert_eq!(text(&body, "outcome"), "rejected");
}

// #725: the corner-cut measurement judged every row of the issue's table on
// the real binary — both legs named, an overcut on either leg red, contour
// and plane-cut authorings judged like the parameterized chamfer, and a
// rounding judged by its tangent setbacks, never by the arc faceting.

#[test]
fn a_weld_leg_within_the_cut_is_green_and_past_it_turns_the_clearance_red() {
    // #723: the dependent weld-clearance check at the corner. The weld whose
    // validated path passes the derived corner fits the 10 mm chamfer with a
    // 6 mm leg — green. Raising the leg to 14 mm turns the clearance red
    // while the recorded requirement stays 10 mm: the finding rides the
    // corner token with the weld record named in its basis and bounds.
    let f = Fixture::new();
    let steel = f.material();
    let (base, column, rib) = f.joint(&steel, &["0=chamfer:10"]);
    f.corner(&rib, &base, &column, &[]);
    f.sized_path_weld(&base, &column, "6", "150,5,10;150,50,10");
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 0, "{reply:?}");
    assert_eq!(text(&reply, "status"), "green");
    assert_eq!(list(&reply, "findings").len(), 0);

    let f = Fixture::new();
    let steel = f.material();
    let (base, column, rib) = f.joint(&steel, &["0=chamfer:10"]);
    f.corner(&rib, &base, &column, &[]);
    f.sized_path_weld(&base, &column, "14", "150,5,10;150,50,10");
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 2, "{reply:?}");
    assert_eq!(text(&reply, "status"), "red");
    let finding = finding_of(&reply, "CORNER_NOT_CUT");
    assert_eq!(text(finding, "status"), "fail");
    assert!(
        text(finding, "basis").contains("contract 4.4"),
        "{:?}",
        text(finding, "basis")
    );
    assert!(text(finding, "message").contains("requirement stays 10 mm"));
    let bounds = list(finding, "bounds");
    let bound = |name: &str| -> (String, f64) {
        bounds
            .iter()
            .find(|b| text(b, "name") == name)
            .map(|b| (text(b, "source"), at(b, &["bound_mm"]).float().unwrap()))
            .unwrap_or_else(|| panic!("no {name} in {bounds:?}"))
    };
    let (weld_source, weld_leg) = bound("weld_leg");
    assert_eq!(weld_source, "weld record");
    assert!((weld_leg - 14.0).abs() <= 1e-9, "{weld_leg}");
    let (required_source, required) = bound("required");
    assert_eq!(required_source, "owner");
    assert!((required - 10.0).abs() <= 1e-9, "{required}");
}

#[test]
fn a_weld_elsewhere_between_the_neighbours_leaves_the_clearance_quiet() {
    // The validated weld runs along the joint away from the derived corner:
    // a weld elsewhere between the same parts produces no corner finding
    // (contract 4.4), and the 14 mm leg changes nothing.
    let f = Fixture::new();
    let steel = f.material();
    let (base, column, rib) = f.joint(&steel, &["0=chamfer:10"]);
    f.corner(&rib, &base, &column, &[]);
    f.sized_path_weld(&base, &column, "14", "160,20,10;200,20,10");
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 0, "{reply:?}");
    assert_eq!(text(&reply, "status"), "green");
    assert_eq!(list(&reply, "findings").len(), 0);
}

#[test]
fn a_pathless_weld_between_the_neighbours_stays_unknown_for_the_clearance() {
    // The pathless A06 weld is never locally attached: the model cannot
    // establish whether it passes the corner, so the clearance stays unknown
    // (attachment_unvalidated) — the clearance only.
    let f = Fixture::new();
    let steel = f.material();
    let (base, column, rib) = f.joint(&steel, &["0=chamfer:10"]);
    f.corner(&rib, &base, &column, &[]);
    f.weld(&base, &column);
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 0, "{reply:?}");
    assert_eq!(text(&reply, "status"), "unknown");
    let findings = list(&reply, "findings");
    assert_eq!(findings.len(), 1, "{findings:?}");
    let finding = finding_of(&reply, "CORNER_NOT_CUT");
    assert_eq!(text(finding, "status"), "unknown");
    assert_eq!(list(finding, "missing_inputs").len(), 1);
    assert!(
        text(finding, "message").contains("unvalidated local attachment"),
        "{:?}",
        text(finding, "message")
    );
}

#[test]
fn a_weld_leg_at_the_band_edge_is_unknown_indeterminate() {
    // The acceptance row: weld leg 10.5 against the admitted 10 mm sits at
    // the near-threshold band edge (contact tolerance 0.5), so the clearance
    // stays unknown (indeterminate) with the admitted leg reported.
    let f = Fixture::new();
    let steel = f.material();
    let (base, column, rib) = f.joint(&steel, &["0=chamfer:10"]);
    f.corner(&rib, &base, &column, &[]);
    f.sized_path_weld(&base, &column, "10.5", "150,5,10;150,50,10");
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 0, "{reply:?}");
    assert_eq!(text(&reply, "status"), "unknown");
    let findings = list(&reply, "findings");
    assert_eq!(findings.len(), 1, "{findings:?}");
    let finding = finding_of(&reply, "CORNER_NOT_CUT");
    assert_eq!(text(finding, "status"), "unknown");
    assert_eq!(reason_of(&reply, "CORNER_NOT_CUT"), "indeterminate");
    let measured = at(finding, &["measured_mm"]).float().unwrap();
    assert!((measured - 10.0).abs() <= 1e-6, "{measured}");
}

#[test]
fn a_rounding_and_a_chamfer_with_equal_setbacks_differ_in_the_clearance() {
    // The chamfer of 10 mm admits the 8 mm weld; the rounding of radius
    // 10 mm — the same setback — admits only R·(2 − √2) ≈ 5.86 mm, so the
    // same weld turns the clearance red there (beside the size finding the
    // rounding already carries at the required 10 mm).
    let f = Fixture::new();
    let steel = f.material();
    let (base, column, rib) = f.joint(&steel, &["0=chamfer:10"]);
    f.corner(&rib, &base, &column, &[]);
    f.sized_path_weld(&base, &column, "8", "150,5,10;150,50,10");
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 0, "{reply:?}");
    assert_eq!(text(&reply, "status"), "green");

    let f = Fixture::new();
    let steel = f.material();
    let (base, column, rib) = f.joint(&steel, &["0=round:10"]);
    f.corner(&rib, &base, &column, &[]);
    f.sized_path_weld(&base, &column, "8", "150,5,10;150,50,10");
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 2, "{reply:?}");
    assert_eq!(text(&reply, "status"), "red");
    assert_eq!(fail_count(&reply, "CORNER_NOT_CUT"), 2);
    let clearance = list(&reply, "findings")
        .iter()
        .find(|f| text(f, "basis").contains("contract 4.4"))
        .expect("the clearance finding is missing");
    assert_eq!(text(clearance, "status"), "fail");
}

#[test]
fn an_asymmetric_parameterized_chamfer_is_overcut_with_named_values() {
    // `0=chamfer:10,40`: one leg is 40 mm — CORNER_OVERCUT measured 40 (the
    // largest setback), with the admitted leg 10 and the largest setback 40
    // named, in either order of the two distances.
    for corners in ["0=chamfer:10,40", "0=chamfer:40,10"] {
        let f = Fixture::new();
        let steel = f.material();
        let (base, column, rib) = f.joint(&steel, &[corners]);
        f.corner(&rib, &base, &column, &[]);
        let (code, reply) = f.check(&[]);
        assert_eq!(code, 2, "{corners}: {reply:?}");
        assert_eq!(text(&reply, "status"), "red");
        let finding = finding_of(&reply, "CORNER_OVERCUT");
        assert_eq!(text(finding, "status"), "fail");
        let bounds = list(finding, "bounds");
        assert_eq!(bounds.len(), 3, "{corners}");
        let value = |name: &str| -> f64 {
            bounds
                .iter()
                .find(|b| text(b, "name") == name)
                .map(|b| at(b, &["bound_mm"]).float().unwrap())
                .unwrap_or_else(|| panic!("no {name} in {bounds:?}"))
        };
        let admitted = value("admitted_leg");
        let setback = value("largest_setback");
        assert!(
            (admitted - 10.0).abs() <= 1e-9 && (setback - 40.0).abs() <= 1e-9,
            "{corners}: {admitted} {setback}"
        );
        assert_eq!(text(bounds.get(1).unwrap(), "source"), "measured");
        let measured = at(finding, &["measured_mm"]).float().unwrap();
        assert!((measured - 40.0).abs() <= 1e-9, "{corners}: {measured}");
        assert!(matches!(finding.get("fix"), Ok(Json::Null)));
    }
}

#[test]
fn contour_authored_cuts_are_judged_like_parameterized_ones() {
    // A chamfer authored as contour points leaves no corner vertex; the 10×10
    // cut is green like the parameterized one, and the 20×20 cut is
    // CORNER_OVERCUT with measured 20, no fix (contract 9: contour
    // reconstruction is no set-plate-corner repair).
    let f = Fixture::new();
    let steel = f.material();
    let base = f.plate(&steel, "10", "0,0,0");
    let column = f.standing(&steel, "100", "200", "40", "50,40,10.000000001", &[]);
    let rib = f.standing_contour(
        &steel,
        "10,0,0;80,0,0;80,100,0;0,100,0;0,10,0",
        "150.000000001,10,10.000000001",
    );
    f.corner(&rib, &base, &column, &[]);
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 0, "{reply:?}");
    assert_eq!(text(&reply, "status"), "green");
    assert_eq!(list(&reply, "findings").len(), 0);
    let f = Fixture::new();
    let steel = f.material();
    let base = f.plate(&steel, "10", "0,0,0");
    let column = f.standing(&steel, "100", "200", "40", "50,40,10.000000001", &[]);
    let rib = f.standing_contour(
        &steel,
        "20,0,0;80,0,0;80,100,0;0,100,0;0,20,0",
        "150.000000001,10,10.000000001",
    );
    f.corner(&rib, &base, &column, &[]);
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 2, "{reply:?}");
    assert_eq!(text(&reply, "status"), "red");
    let finding = finding_of(&reply, "CORNER_OVERCUT");
    let measured = at(finding, &["measured_mm"]).float().unwrap();
    assert!((measured - 20.0).abs() <= 1e-9, "{measured}");
    assert!(matches!(finding.get("fix"), Ok(Json::Null)));
}

#[test]
fn plane_cut_and_chamfer_part_corners_are_judged_like_parameterized_ones() {
    // A 10×10 corner cut authored as a plane cut — or as an A13 chamfer-part —
    // is green like the parameterized chamfer; a 20×20 plane cut is
    // CORNER_OVERCUT with measured 20.
    let f = Fixture::new();
    let steel = f.material();
    let (base, column, rib) = f.joint(&steel, &[]);
    f.cut_rib_corner_by_plane(&rib, "10", "019a7bf0-0000-7000-8000-00000000c011");
    f.corner(&rib, &base, &column, &[]);
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 0, "{reply:?}");
    assert_eq!(text(&reply, "status"), "green");
    assert_eq!(list(&reply, "findings").len(), 0);
    let f = Fixture::new();
    let steel = f.material();
    let (base, column, rib) = f.joint(&steel, &[]);
    f.cut_rib_corner_by_plane(&rib, "20", "019a7bf0-0000-7000-8000-00000000c012");
    f.corner(&rib, &base, &column, &[]);
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 2, "{reply:?}");
    let finding = finding_of(&reply, "CORNER_OVERCUT");
    let measured = at(finding, &["measured_mm"]).float().unwrap();
    assert!((measured - 20.0).abs() <= 1e-9, "{measured}");
    // The A13 fabrication route: the same final 10×10 shape, green.
    let f = Fixture::new();
    let steel = f.material();
    let (base, column, rib) = f.joint(&steel, &[]);
    f.chamfer_rib_part(&rib, "10", "10", "019a7bf0-0000-7000-8000-00000000c013");
    f.corner(&rib, &base, &column, &[]);
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 0, "{reply:?}");
    assert_eq!(text(&reply, "status"), "green");
    assert_eq!(list(&reply, "findings").len(), 0);
}

#[test]
fn a_rounding_is_judged_by_its_admitted_leg_and_stays_without_the_chamfer_fix() {
    // A rounding of radius R at a right angle admits the R·(2 − √2) chamfer
    // leg and sets back R: against the recorded 10 ± 1 mm band `round:10` is
    // CORNER_NOT_CUT measured 5.858 — a rounding is not the chamfer its
    // setback suggests — `round:5` measured 2.929, `round:20` CORNER_OVERCUT
    // measured 20, and a rounding never receives the chamfer the recorded
    // requirement's repair would set (contract 9: not an equivalent final
    // shape). The admitted leg is sampled on the faceted arc within its
    // 0.05 mm chord-deviation bound (#725).
    let admitted_of = |radius: f64| radius * (2.0 - std::f64::consts::SQRT_2);
    for (corners, rule, measured) in [
        ("0=round:10", Some("CORNER_NOT_CUT"), admitted_of(10.0)),
        ("0=round:5", Some("CORNER_NOT_CUT"), admitted_of(5.0)),
        ("0=round:20", Some("CORNER_OVERCUT"), 20.0),
    ] {
        let f = Fixture::new();
        let steel = f.material();
        let (base, column, rib) = f.joint(&steel, &[corners]);
        f.corner(&rib, &base, &column, &[]);
        let (code, reply) = f.check(&[]);
        assert_eq!(code, 2, "{corners}: {reply:?}");
        let finding = finding_of(&reply, rule.unwrap());
        assert_eq!(text(finding, "status"), "fail", "{corners}");
        let value = at(finding, &["measured_mm"]).float().unwrap();
        assert!(
            (value - measured).abs() <= 0.05,
            "{corners}: {value} vs {measured}"
        );
        // The rounding is not an equivalent repair of the undersized cut.
        assert!(
            matches!(finding.get("fix"), Ok(Json::Null)),
            "{corners}: {finding:?}"
        );
    }
}

// Contract section 9 (audit gap G4): parameterized chamfers, contour-authored
// chamfers and plane cuts that produce equivalent final geometry receive
// equivalent check results, and a rounding with equal setbacks is not
// equivalent — it never receives the chamfer fix.

/// Builds the sample joint with the corner cut of `size` mm authored three
/// ways; returns the three element ids the requirement names.
fn joint_with_cut(f: &Fixture, size: &str, authoring: &str) -> (String, String, String) {
    let steel = f.material();
    match authoring {
        "corner parameters" => {
            let corners = format!("0=chamfer:{size}");
            f.joint(&steel, &[&corners])
        }
        "contour points" => {
            let base = f.plate(&steel, "10", "0,0,0");
            let column = f.standing(&steel, "100", "200", "40", "50,40,10.000000001", &[]);
            let contour = format!("{size},0,0;80,0,0;80,100,0;0,100,0;0,{size},0");
            let rib = f.standing_contour(&steel, &contour, "150.000000001,10,10.000000001");
            (base, column, rib)
        }
        _ => {
            let (base, column, rib) = f.joint(&steel, &[]);
            f.cut_rib_corner_by_plane(&rib, size, "019a7bf0-0000-7000-8000-00000000e001");
            (base, column, rib)
        }
    }
}

#[test]
fn equivalent_final_cuts_receive_equivalent_check_results() {
    // The same final shape authored three ways checks the same: a 10×10 cut is
    // green with no findings through every authoring, a 20×20 cut is
    // CORNER_OVERCUT with measured 20 and no fix through every authoring —
    // the equivalence of the results, not only each row's verdict, is the
    // contract's claim (section 9).
    for size in ["10", "20"] {
        let mut verdicts: Vec<(String, f64)> = Vec::new();
        for authoring in ["corner parameters", "contour points", "plane cut"] {
            let f = Fixture::new();
            let (base, column, rib) = joint_with_cut(&f, size, authoring);
            f.corner(&rib, &base, &column, &[]);
            let (code, reply) = f.check(&[]);
            if size == "10" {
                assert_eq!(code, 0, "{authoring}: {reply:?}");
                assert_eq!(text(&reply, "status"), "green", "{authoring}");
                assert_eq!(list(&reply, "findings").len(), 0, "{authoring}");
                verdicts.push(("green".to_owned(), 0.0));
            } else {
                assert_eq!(code, 2, "{authoring}: {reply:?}");
                let finding = finding_of(&reply, "CORNER_OVERCUT");
                assert_eq!(text(finding, "status"), "fail", "{authoring}");
                let measured = at(finding, &["measured_mm"]).float().unwrap();
                assert!((measured - 20.0).abs() <= 1e-9, "{authoring}: {measured}");
                assert!(
                    matches!(finding.get("fix"), Ok(Json::Null)),
                    "{authoring}: {finding:?}"
                );
                verdicts.push((text(finding, "rule").to_owned(), measured));
            }
        }
        let first = &verdicts[0];
        for verdict in &verdicts[1..] {
            assert_eq!(verdict, first, "{size}");
        }
    }
}

#[test]
fn an_equal_setback_rounding_is_not_the_chamfer_it_mimics() {
    // The §9 counterexample at the level the report carries: a 5 mm setback
    // authored as a chamfer and as a rounding share the rule and the status,
    // but the rounding's admitted leg is 5·(2 − √2) ≈ 2.929 — less freed
    // space at a right-angle corner, a different measured value — and only
    // the chamfer receives the set-plate-corner repair. Equal setbacks do not
    // make the rounding the chamfer the requirement's repair would set
    // (contract 9).
    let f = Fixture::new();
    let steel = f.material();
    let (base, column, rib) = f.joint(&steel, &["0=chamfer:5"]);
    f.corner(&rib, &base, &column, &[]);
    let (_, reply) = f.check(&[]);
    let chamfer = finding_of(&reply, "CORNER_NOT_CUT");
    assert_eq!(text(chamfer, "status"), "fail");
    let chamfer_measured = at(chamfer, &["measured_mm"]).float().unwrap();
    assert!((chamfer_measured - 5.0).abs() <= 1e-9, "{chamfer_measured}");
    assert_eq!(text(at(chamfer, &["fix"]), "command"), "set-plate-corner");
    let f = Fixture::new();
    let steel = f.material();
    let (base, column, rib) = f.joint(&steel, &["0=round:5"]);
    f.corner(&rib, &base, &column, &[]);
    let (_, reply) = f.check(&[]);
    let rounding = finding_of(&reply, "CORNER_NOT_CUT");
    assert_eq!(text(rounding, "status"), "fail");
    let rounding_measured = at(rounding, &["measured_mm"]).float().unwrap();
    let admitted = 5.0 * (2.0 - std::f64::consts::SQRT_2);
    // Sampled on the faceted arc within its chord-deviation bound (#725).
    assert!(
        (rounding_measured - admitted).abs() <= 0.05,
        "{rounding_measured} vs {admitted}"
    );
    // The non-equivalence, in the report itself: the measured values differ.
    assert!(
        (chamfer_measured - rounding_measured).abs() > 1.0,
        "{chamfer_measured} vs {rounding_measured}"
    );
    // And the repair stays the chamfer's alone.
    assert!(
        matches!(rounding.get("fix"), Ok(Json::Null)),
        "{rounding:?}"
    );
}

// Q03 slice 5: the `checks` summary in write replies (contract section 11). A
// successful write on a model-store command reports the check of its resulting
// revision beside its result, with the write's own exit code; a drafted write
// reports the draft as not evaluated; a follow-up check that cannot run
// reports unavailable, never silently green; `BEAMCRAFT_CHECKS_SUMMARY=off`
// omits the summary for that process.

fn summary_of(reply: &Json) -> &Json {
    at(reply, &["checks"])
}

fn summary_fail(summary: &Json, rule: &str) -> u64 {
    at(summary, &["counts", rule, "fail"]).unsigned().unwrap()
}

fn summary_unknown(summary: &Json, rule: &str) -> u64 {
    at(summary, &["counts", rule, "unknown"])
        .unsigned()
        .unwrap()
}

/// Runs one command with one extra environment variable set, for the opt-out
/// cases the ordinary runner cannot express.
fn run_with_env(f: &Fixture, var: &str, value: &str, command: &str, extra: &[&str]) -> (i32, Json) {
    let store = f.shared.root.join("store").to_str().unwrap().to_owned();
    let mut args: Vec<String> = vec![command.to_owned(), "--store".to_owned(), store];
    args.extend(extra.iter().map(|a| (*a).to_owned()));
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .args(&args)
        .env(var, value)
        .output()
        .unwrap();
    let body = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
    (output.status.code().unwrap(), body)
}

/// A minimal session driver: one real `beamcraft session` process over pipes,
/// one JSON line per command line.
struct SessionLines {
    child: std::process::Child,
    out: std::io::BufReader<std::process::ChildStdout>,
}

impl SessionLines {
    fn start(store: &str) -> Self {
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .arg("session")
            .arg("--store")
            .arg(store)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let out = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut session = Self { child, out };
        let ready = session.answer();
        assert_eq!(text(&ready, "outcome"), "session_ready");
        session
    }

    /// Sends one line and reads the one answer line.
    fn ask(&mut self, line: &str) -> Json {
        use std::io::Write;
        writeln!(self.child.stdin.as_mut().unwrap(), "{line}").unwrap();
        self.answer()
    }

    fn answer(&mut self) -> Json {
        use std::io::BufRead;
        let mut line = String::new();
        self.out.read_line(&mut line).unwrap();
        assert!(!line.is_empty(), "the session closed its output");
        beamcraft_json::decode(line.trim_end().as_bytes(), &Limits::DEFAULT).unwrap()
    }
}

impl Drop for SessionLines {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn store_root_of(f: &Fixture) -> String {
    f.shared.root.join("store").to_str().unwrap().to_owned()
}

#[test]
fn a_write_on_a_red_model_returns_red_checks_with_write_exit_zero() {
    let f = Fixture::new();
    let steel = f.material();
    f.plate(&steel, "10", "0,0,0");
    f.plate(&steel, "10", "100,0,5");
    // The write succeeded: exit 0, its own created reply — and the summary of
    // the resulting revision rides beside it as red.
    let (code, reply) = f.run(
        "create-plate",
        &[
            "--width",
            "400",
            "--length",
            "300",
            "--thickness",
            "12",
            "--material-digest",
            &steel,
            "--translation",
            "8000,0,0",
        ],
    );
    assert_eq!(code, 0, "{reply:?}");
    let summary = summary_of(&reply);
    assert_eq!(text(summary, "status"), "red");
    assert!(at(summary, &["findings_total"]).unsigned().unwrap() >= 1);
    assert_eq!(summary_fail(summary, "PART_CLASH"), 1);
    assert_eq!(summary_unknown(summary, "PART_CLASH"), 0);
    // Revision-specific: the summary names the write's own resulting revision,
    // not an unpinned head or the pre-write one.
    assert_eq!(
        text(summary, "revision_id"),
        text(at(&reply, &["revision"]), "revision_id")
    );
}

#[test]
fn a_write_on_a_green_model_reports_green_checks() {
    let f = Fixture::new();
    let steel = f.material();
    f.plate(&steel, "10", "0,0,0");
    f.plate(&steel, "10", "0,0,10");
    let (code, reply) = f.run(
        "create-plate",
        &[
            "--width",
            "400",
            "--length",
            "300",
            "--thickness",
            "12",
            "--material-digest",
            &steel,
            "--translation",
            "8000,0,0",
        ],
    );
    assert_eq!(code, 0, "{reply:?}");
    assert_eq!(text(summary_of(&reply), "status"), "green");
    assert_eq!(
        at(summary_of(&reply), &["findings_total"])
            .unsigned()
            .unwrap(),
        0
    );
}

#[test]
fn the_opt_out_omits_the_summary_and_any_other_value_keeps_it() {
    let f = Fixture::new();
    // Exactly `off`: no `checks` key at all.
    let (code, reply) = run_with_env(
        &f,
        "BEAMCRAFT_CHECKS_SUMMARY",
        "off",
        "create-plate",
        &["--width", "400", "--length", "300", "--thickness", "12"],
    );
    assert_eq!(code, 0, "{reply:?}");
    assert!(reply.get("checks").is_err(), "{reply:?}");
    // Any other value, here not even a plausible one, keeps summaries on.
    let (code, reply) = run_with_env(
        &f,
        "BEAMCRAFT_CHECKS_SUMMARY",
        "certainly-not-off",
        "create-plate",
        &["--width", "400", "--length", "300", "--thickness", "12"],
    );
    assert_eq!(code, 0, "{reply:?}");
    assert!(reply.get("checks").is_ok(), "{reply:?}");
}

#[test]
fn a_session_write_carries_the_same_summary() {
    let f = Fixture::new();
    let steel = f.material();
    f.plate(&steel, "10", "0,0,0");
    f.plate(&steel, "10", "100,0,5");
    let store = store_root_of(&f);
    let mut session = SessionLines::start(&store);
    let reply = session.ask(&format!(
        "create-plate --store {store} --width 400 --length 300 --thickness 12 \
         --material-digest {steel} --translation 8000,0,0"
    ));
    assert_eq!(text(&reply, "outcome"), "plate_created");
    let summary = summary_of(&reply);
    assert_eq!(text(summary, "status"), "red");
    assert_eq!(summary_fail(summary, "PART_CLASH"), 1);
    assert_eq!(
        text(summary, "revision_id"),
        text(at(&reply, &["revision"]), "revision_id")
    );
}

#[test]
fn a_drafted_write_reports_draft_not_evaluated_and_the_commit_checks_the_result() {
    // The saved head is green; the drafted plate would clash. The draft reply
    // must not read the old head and call the draft green: it reports the
    // draft as not evaluated, with no revision named. The commit then checks
    // the resulting revision — which carries the drafted plate, so red.
    let f = Fixture::new();
    let steel = f.material();
    f.plate(&steel, "10", "0,0,0");
    f.plate(&steel, "10", "0,0,10");
    let store = store_root_of(&f);
    let mut session = SessionLines::start(&store);
    let opened = session.ask("batch-open");
    assert_eq!(text(&opened, "outcome"), "batch_opened");
    let drafted = session.ask(&format!(
        "create-plate --store {store} --width 400 --length 300 --thickness 12 \
         --material-digest {steel} --translation 0,0,3"
    ));
    assert_eq!(text(&drafted, "outcome"), "plate_created");
    let summary = summary_of(&drafted);
    assert_eq!(text(summary, "status"), "unknown");
    assert_eq!(text(summary, "reason"), "draft_not_evaluated");
    assert!(
        summary.get("revision_id").is_err(),
        "a draft names no checked revision: {summary:?}"
    );
    let committed = session.ask("batch-commit");
    assert_eq!(text(&committed, "outcome"), "batch_committed");
    let summary = summary_of(&committed);
    assert_eq!(text(summary, "status"), "red");
    // The drafted plate penetrates both stacked plates of the saved head.
    assert_eq!(summary_fail(summary, "PART_CLASH"), 2);
    assert_eq!(
        text(summary, "revision_id"),
        text(at(&committed, &["revision"]), "revision_id")
    );
}

#[test]
fn the_record_write_turns_the_summary_red_and_the_repair_write_green() {
    // The owner flow end to end: recording the 10 mm requirement against the
    // uncut rib makes the recording write's own summary red with
    // CORNER_NOT_CUT; the repair write's summary is green.
    let f = Fixture::new();
    let steel = f.material();
    let (base, column, rib) = f.joint(&steel, &[]);
    let neighbours = format!("{base},{column}");
    let (code, reply) = f.run(
        "record-create",
        &[
            "--name",
            "corner cut",
            "--kind",
            "corner_cut",
            "--target",
            &rib,
            "--neighbours",
            &neighbours,
            "--required-mm",
            "10",
            "--tolerance-mm",
            "1",
            "--new-id",
            "019a7bf0-0000-7000-8000-00000000f00d",
        ],
    );
    assert_eq!(code, 0, "{reply:?}");
    let summary = summary_of(&reply);
    assert_eq!(text(summary, "status"), "red");
    assert_eq!(summary_fail(summary, "CORNER_NOT_CUT"), 1);
    assert_eq!(
        text(summary, "revision_id"),
        text(at(&reply, &["revision"]), "revision_id")
    );
    // The unambiguous repair of the finding, applied as an ordinary write.
    let (code, repaired) = f.run(
        "set-plate-corner",
        &["--id", &rib, "--vertex", "0", "--corner", "chamfer:10"],
    );
    assert_eq!(code, 0, "{repaired:?}");
    assert_eq!(text(summary_of(&repaired), "status"), "green");
    assert_eq!(
        at(summary_of(&repaired), &["findings_total"])
            .unsigned()
            .unwrap(),
        0
    );
}

#[test]
fn the_summary_counts_equal_the_paged_report_of_the_same_revision() {
    // Pagination never hides the whole-scope verdict: the summary of the
    // write equals a findings-page-1 report of the same revision, and the
    // page shows one finding while the counts stay whole-scope.
    let f = Fixture::new();
    let steel = f.material();
    f.plate(&steel, "10", "0,0,0");
    f.plate(&steel, "10", "100,0,5");
    f.plate(&steel, "10", "200,0,5");
    let (code, reply) = f.run(
        "create-plate",
        &[
            "--width",
            "400",
            "--length",
            "300",
            "--thickness",
            "12",
            "--material-digest",
            &steel,
            "--translation",
            "8000,0,0",
        ],
    );
    assert_eq!(code, 0, "{reply:?}");
    let summary = summary_of(&reply);
    let (code, report) = f.check(&["--offset", "0", "--limit", "1"]);
    assert_eq!(code, 2, "{report:?}");
    assert_eq!(text(&report, "revision_id"), text(summary, "revision_id"));
    assert_eq!(
        at(&report, &["findings_total"]).unsigned().unwrap(),
        at(summary, &["findings_total"]).unsigned().unwrap()
    );
    assert_eq!(list(&report, "findings").len(), 1, "one finding per page");
    for rule in beamcraft_commands::check::RULES {
        assert_eq!(
            summary_fail(summary, rule),
            fail_count(&report, rule),
            "{rule} fail"
        );
        assert_eq!(
            summary_unknown(summary, rule),
            unknown_count(&report, rule),
            "{rule} unknown"
        );
    }
}

#[test]
fn reads_and_store_level_writes_carry_no_checks_summary() {
    let f = Fixture::new();
    let steel = f.material();
    f.plate(&steel, "10", "0,0,0");
    // A read reply never carries the summary.
    let (code, reply) = f.check(&[]);
    assert_eq!(code, 0, "{reply:?}");
    assert!(reply.get("checks").is_err());
    // A store-level write (the blob primitive) is not a model write: no summary.
    let blob = f.shared.root.join("blob.bin");
    std::fs::write(&blob, b"payload").unwrap();
    let (code, reply) = f.run("put-blob", &["--file", blob.to_str().unwrap()]);
    assert_eq!(code, 0, "{reply:?}");
    assert!(reply.get("checks").is_err());
    // A successful project creation provisions a fresh store and changes no
    // modelled revision: no summary, and the success is the point — the key is
    // absent because the route is out of the set, not because the write failed.
    // `create-project` takes `--parent`/`--name` and no `--store`, so this one
    // line runs without the fixture's injected store option.
    let parent = f.shared.root.join("models");
    std::fs::create_dir_all(&parent).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .args([
            "create-project",
            "--parent",
            parent.to_str().unwrap(),
            "--name",
            "Fresh Model",
            "--unique-name",
            "true",
            "--actor-id",
            "019a7bf0-0000-7000-8000-0000000000ac",
        ])
        .output()
        .unwrap();
    let reply = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
    assert_eq!(output.status.code().unwrap(), 0, "{reply:?}");
    assert!(reply.get("checks").is_err(), "{reply:?}");
}
