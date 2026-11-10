// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic scenarios; the fastener values are the shipped synthetic
// library's, which follows no national standard, MPL-2.0.

//! A14.3 and A14.4 end to end through the real `beamcraft` binary: `create-bolt-group` places a
//! row of identical fasteners of a pinned library definition between two or more parts and,
//! with the bolt's default `hole_mode = generate`, writes one group-owned hole record per
//! crossing fastener on every connected plate in the same revision; `update-bolt-group` edits
//! one group as one atomic edit and one undo unit and regenerates its holes; `bolts` lists the
//! A05 annotation groups (readable, upgradable) next to the placed ones. The positions,
//! generators, length modes, washer and hole cases of the bolt amendment sections 5 and 9 drive
//! the assertions, with the amendment's negative cases refused without a revision.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support::{StoreFixture, at, list, text};

use beamcraft_json::{Json, Limits};
use std::fs;
use std::path::Path;
use std::process::Command;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

const SYNTHETIC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/libraries/synthetic-fasteners.usda"
);
const GROUP: &str = "019a7bf0-0000-7000-8000-000000710001";
const GROUP2: &str = "019a7bf0-0000-7000-8000-000000710002";
const GROUP3: &str = "019a7bf0-0000-7000-8000-000000710003";
const BRACE: &str = "019a7bf0-0000-7000-8000-0000007100b1";
const GUSSET: &str = "019a7bf0-0000-7000-8000-0000007100b2";
const USER: &str = "019a7bf0-0000-7000-8000-000000719001";

/// SYN-BOLT-12x50 of the shipped synthetic library: washered, lengths [30, 40, 50, 60, 80],
/// nut 9, washer 2, protrusion 3, washer defaults true/true.
const WASHERED: &str = "SYN-BOLT-12x50";
/// SYN-BOLT-16x70-BARE: no washer, lengths [40, 50, 70, 90], nut 12, protrusion 4.
const BARE: &str = "SYN-BOLT-16x70-BARE";
/// SYN-SCREW-10x30: a screw with one washer.
const SCREW: &str = "SYN-SCREW-10x30";

struct Fixture {
    shared: StoreFixture,
    workspace: Option<String>,
}
impl Fixture {
    fn new() -> Self {
        Self {
            shared: StoreFixture::new("a14-3-tests"),
            workspace: None,
        }
    }
    /// Forks a personal workspace from the published head: every later edit runs against the
    /// workspace store, so `undo-workspace` can revert one command at a time.
    fn fork(&mut self) {
        let workspace = self
            .shared
            .root
            .join(format!(
                "workspace-{}",
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ))
            .to_str()
            .unwrap()
            .to_owned();
        fs::create_dir(&workspace).unwrap();
        let (code, body) = raw(
            self.shared.root.join("store").to_str().unwrap(),
            &[
                "create-workspace",
                "--workspace-dir",
                &workspace,
                "--user-id",
                USER,
            ],
        );
        assert_eq!(code, 0, "{body:?}");
        self.workspace = Some(workspace);
    }
    fn run(&self, args: &[String]) -> (i32, Json) {
        let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        let base = self.shared.root.join("store");
        let store = self
            .workspace
            .as_deref()
            .unwrap_or_else(|| base.to_str().unwrap());
        raw(store, &refs)
    }
    fn ok(&self, args: &[String]) -> Json {
        let (code, body) = self.run(args);
        assert_eq!(code, 0, "{} {args:?}: {body:?}", args[0]);
        body
    }
    fn rejected(&self, args: &[String]) -> String {
        let before = self.head();
        let (code, body) = self.run(args);
        assert_eq!(code, 2, "{} {args:?}: {body:?}", args[0]);
        // A refusal writes no revision.
        assert_eq!(self.head(), before, "{} {args:?}", args[0]);
        text(&body, "code")
    }
    fn head(&self) -> String {
        text(&self.ok(&strs("open-revision", &[])), "revision_id")
    }
    fn undo(&self) {
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args([
                "undo-workspace",
                "--workspace-dir",
                self.workspace.as_deref().unwrap(),
            ])
            .output()
            .unwrap();
        assert_eq!(output.status.code().unwrap(), 0);
    }
    /// The first-slice plate: world box `[0,1000] x [0,500] x [0,10]` at the given elevation.
    /// Runs against the store that is live: the base store before the fork, the workspace after.
    fn plate(&self, id: &str, z: &str) -> String {
        let mut args = vec![
            "--name".to_owned(),
            "P".to_owned(),
            "--new-id".to_owned(),
            id.to_owned(),
            "--width".to_owned(),
            "1000".to_owned(),
            "--length".to_owned(),
            "500".to_owned(),
            "--thickness".to_owned(),
            "10".to_owned(),
        ];
        if z != "0" {
            args.push("--translation".to_owned());
            args.push(format!("0,0,{z}"));
        }
        let body = self.ok(&strs("create-plate", &args));
        text(at(&body, &["plate"]), "element_id")
    }
    /// Two stacked plates and a row along `y = 250`: every shank crosses `[0,10]` and `[10,20]`,
    /// so the grip is `20`. The plates go into the base store, then the workspace forks.
    fn parts(&mut self) -> Vec<String> {
        let low = self.plate("019a7bf0-0000-7000-8000-0000007100f1", "0");
        let high = self.plate("019a7bf0-0000-7000-8000-0000007100f2", "10");
        self.fork();
        vec![low, high]
    }
    /// The changed layers of a batch that rewrites both parts' meshes (U15): their own derived
    /// layers and — when the bodies change — the referenced asset layer. A batch that also
    /// creates or deletes elements adds or removes stack entries, so the derived stack's entry
    /// list changes with them (`stack_change`).
    fn mesh_changed(parts: &[String], stack_change: bool, geometry_change: bool) -> Vec<String> {
        let mut changed: Vec<String> = parts
            .iter()
            .map(|p| format!("derived/{}.usda", p.replace('-', "")))
            .collect();
        changed.push("authored.usda".to_owned());
        if geometry_change {
            changed.push("derived/geometry.usda".to_owned());
        }
        if stack_change {
            changed.push("derived.usda".to_owned());
        }
        changed.sort();
        changed
    }
    /// The create arguments of one group: two points on `y = 250`, the row along `x`, the
    /// plane normal up, offset start 100.
    fn group_args(
        &self,
        id: &str,
        parts: &[String],
        designation: &str,
        extra: &[&str],
    ) -> Vec<String> {
        let joined = parts.join(",");
        let mut args = vec![
            "--name",
            "G1",
            "--new-id",
            id,
            "--parts",
            joined.as_str(),
            "--fastener-library",
            SYNTHETIC,
            "--designation",
            designation,
            "--point1",
            "0,250,0",
            "--point2",
            "600,250,0",
            "--plane-normal",
            "0,0,1",
            "--offset-start",
            "100",
        ];
        args.extend_from_slice(extra);
        args.iter().map(|s| (*s).to_owned()).collect()
    }
    /// One washered group of two fasteners at `x = 100, 250` (spacing 150).
    fn group(&self, id: &str, parts: &[String]) -> Json {
        self.ok(&strs(
            "create-bolt-group",
            &self.group_args(id, parts, WASHERED, &["--spacings", "150"]),
        ))
    }
    fn groups(&self) -> Vec<Json> {
        list(&self.ok(&strs("bolts", &[])), "bolt_groups").to_vec()
    }
}

/// One command line against the store root.
fn raw(store: &str, args: &[&str]) -> (i32, Json) {
    let mut all = vec![args[0], "--store", store];
    all.extend_from_slice(&args[1..]);
    let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .args(&all)
        .output()
        .unwrap();
    let body = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
    (output.status.code().unwrap(), body)
}

fn strs(command: &str, args: &[String]) -> Vec<String> {
    let mut all = vec![command.to_owned()];
    all.extend(args.iter().cloned());
    all
}

/// An argument list of literals.
fn owned(command: &str, args: &[&str]) -> Vec<String> {
    let mut all = vec![command.to_owned()];
    all.extend(args.iter().map(|s| (*s).to_owned()));
    all
}

fn strings(json: &Json, key: &str) -> Vec<String> {
    list(json, key)
        .iter()
        .map(|v| v.string().unwrap().to_owned())
        .collect()
}

fn entries(change: &Json, key: &str) -> Vec<(String, String)> {
    list(at(change, &["footprint"]), key)
        .iter()
        .map(|e| (text(e, "target"), text(e, "group")))
        .collect()
}

/// The hole records of one plate as `inspect-elements` lists them.
fn holes_of(f: &Fixture, plate: &str) -> Vec<Json> {
    let inspected = f.ok(&strs("inspect-elements", &[]));
    let plates = at(&inspected, &["plates"]).array().unwrap().to_vec();
    let record = plates
        .iter()
        .find(|p| text(p, "element_id") == plate)
        .expect("inspected plate is present");
    at(record, &["hole_records"]).array().unwrap().to_vec()
}

/// The mesh digest of one plate, so a generated or removed hole shows in the derived layer.
fn mesh_of(f: &Fixture, plate: &str) -> String {
    let inspected = f.ok(&strs("inspect-elements", &[]));
    let plates = at(&inspected, &["plates"]).array().unwrap().to_vec();
    let record = plates
        .iter()
        .find(|p| text(p, "element_id") == plate)
        .expect("inspected plate is present");
    text(record, "mesh_digest")
}

#[test]
fn a_group_places_its_row_and_edits_as_one_undo_unit() {
    let mut f = Fixture::new();
    let parts = f.parts();

    let created = f.group(GROUP, &parts);
    assert_eq!(text(&created, "outcome"), "bolt_group_committed");
    assert_eq!(text(&created, "element_id"), GROUP);
    // No body of its own, but the group generates: the authored layer changes and the parts'
    // derived meshes are rewritten.
    assert_eq!(
        strings(&created, "changed_layers"),
        Fixture::mesh_changed(&parts, false, true)
    );
    // The parts' lifecycles and shapes and the pinned definition are read; the group's seven
    // groups, the definition's entry (pinned by the same transaction) and both plates' hole
    // records and meshes are written.
    let writes = entries(&created, "writes");
    assert_eq!(writes.len(), 12);
    assert_eq!(
        writes.iter().filter(|(target, _)| target == GROUP).count(),
        7
    );
    let reads = entries(&created, "reads");
    for part in &parts {
        assert!(reads.contains(&(part.clone(), "lifecycle".to_owned())));
        assert!(reads.contains(&(part.clone(), "shape_inputs".to_owned())));
    }
    assert_eq!(
        reads.len(),
        6,
        "the group, both parts and one definition are read"
    );

    // The record: the length `auto` resolved to the smallest standard length over grip 20 +
    // nut washer 2 + nut 9 + protrusion 3 = 34, and the definition's own washer defaults.
    let groups = f.groups();
    assert_eq!(groups.len(), 1);
    let group = &groups[0];
    assert_eq!(text(group, "id"), GROUP);
    assert_eq!(strings(group, "parts"), parts);
    assert!(strings(group, "spacings_mm") == ["150"], "{group:?}");
    assert_eq!(text(group, "spacing_mode"), "spacings");
    assert_eq!(text(group, "length_mode"), "auto");
    assert_eq!(text(group, "length_mm"), "50");
    assert_eq!(text(group, "washer_under_head"), "true");
    assert_eq!(text(group, "washer_under_nut"), "true");
    assert_eq!(text(group, "count"), "2");
    assert_eq!(text(group, "hole_mode"), "generate");
    assert_eq!(strings(group, "point1_mm"), ["0", "250", "0"]);
    assert_eq!(strings(group, "point2_mm"), ["600", "250", "0"]);
    assert_eq!(strings(group, "plane_normal_mm"), ["0", "0", "1"]);

    // One hole record per crossing fastener on every plate, owned by the group.
    for part in &parts {
        let holes = holes_of(&f, part);
        assert_eq!(holes.len(), 2, "{part}");
        for hole in &holes {
            assert_eq!(text(hole, "generated_by"), GROUP);
        }
    }

    // One atomic edit and one undo unit: moving `point2` keeps the authoritative spacings and
    // regenerates the holes, and the undo returns the whole record.
    let op = "019a7bf0-0000-7000-8000-0000007100b1";
    let moved = f.ok(&f_args_edit(
        GROUP,
        &["--point2", "700,250,0"],
        &[("--op-id", op)],
    ));
    assert_eq!(text(&moved, "outcome"), "bolt_group_committed");
    let group = &f.groups()[0];
    assert_eq!(strings(group, "point2_mm"), ["700", "250", "0"]);
    assert!(strings(group, "spacings_mm") == ["150"]);
    // The re-place runs over the meshes the group's own generated holes rewrote: the stored
    // length survives them (issue 519 review).
    assert_eq!(text(group, "length_mm"), "50");
    for part in &parts {
        assert_eq!(holes_of(&f, part).len(), 2, "{part}");
    }
    f.undo();
    let group = &f.groups()[0];
    assert_eq!(strings(group, "point2_mm"), ["600", "250", "0"]);
    assert_eq!(text(group, "length_mm"), "50");
    // The undo returns the created records with their identities.
    for part in &parts {
        let holes = holes_of(&f, part);
        assert_eq!(holes.len(), 2, "{part}");
        for hole in &holes {
            assert_eq!(text(hole, "generated_by"), GROUP);
        }
    }

    // A retry of the same operation replays; other content under the same id is reuse.
    let again = f.ok(&f_args_edit(
        GROUP,
        &["--point2", "700,250,0"],
        &[("--op-id", op)],
    ));
    assert_eq!(text(&again, "outcome"), "bolt_group_replayed");
    let other = f_args_edit(GROUP, &["--point2", "800,250,0"], &[("--op-id", op)]);
    assert_eq!(f.rejected(&other), "OP_ID_REUSE");
}

#[test]
fn a_rename_reaches_the_authored_layer_and_survives_a_reopen() {
    let mut f = Fixture::new();
    let parts = f.parts();

    f.group(GROUP, &parts);
    // The amendment's optional `--name` renames the group: the label is authored layer state,
    // so the rename declares its write, lands in the committed bytes and survives a reopen.
    let op = "019a7bf0-0000-7000-8000-0000007100c9";
    let renamed = f.ok(&f_args_edit(GROUP, &["--name", "G2"], &[("--op-id", op)]));
    assert_eq!(text(&renamed, "outcome"), "bolt_group_committed");
    let writes = entries(&renamed, "writes");
    assert!(
        writes.contains(&(GROUP.to_owned(), "label".to_owned())),
        "{writes:?}"
    );
    let group = &f.groups()[0];
    assert_eq!(text(group, "name"), "G2");
    // A plain rename re-places the group over the meshes its own generated holes rewrote: the
    // stored length survives them instead of collapsing (issue 519 review).
    assert_eq!(text(group, "length_mm"), "50");

    // A retry of the same operation replays; a different name under the same id is reuse:
    // the label is part of the operation's content.
    let again = f.ok(&f_args_edit(GROUP, &["--name", "G2"], &[("--op-id", op)]));
    assert_eq!(text(&again, "outcome"), "bolt_group_replayed");
    let other = f_args_edit(GROUP, &["--name", "G3"], &[("--op-id", op)]);
    assert_eq!(f.rejected(&other), "OP_ID_REUSE");

    // An edit that sends no `--name` keeps the stored label.
    let moved = f.ok(&f_args_edit(GROUP, &["--point2", "700,250,0"], &[]));
    assert_eq!(text(&moved, "outcome"), "bolt_group_committed");
    assert_eq!(text(&f.groups()[0], "name"), "G2");

    // A malformed label is refused without a revision.
    let bad = f_args_edit(GROUP, &["--name", ""], &[]);
    assert_eq!(f.rejected(&bad), "INVALID_ARGUMENT");
}

/// A retry of an accepted create re-places over the meshes the operation itself rewrote with
/// its generated holes: it replays instead of mismatching the stored record into
/// `OP_ID_REUSE` (issue 519 review).
#[test]
fn a_create_retry_replays_over_its_own_holes() {
    let mut f = Fixture::new();
    let parts = f.parts();
    let mut args = f.group_args(GROUP, &parts, WASHERED, &["--spacings", "150"]);
    args.extend([
        "--op-id".to_owned(),
        "019a7bf0-0000-7000-8000-0000007100e1".to_owned(),
    ]);
    f.ok(&strs("create-bolt-group", &args));
    let again = f.ok(&strs("create-bolt-group", &args));
    assert_eq!(text(&again, "outcome"), "bolt_group_replayed");
}

/// An `update-bolt-group` line for `id` with the given values.
fn f_args_edit(id: &str, values: &[&str], op: &[(&str, &str)]) -> Vec<String> {
    let mut args = vec!["update-bolt-group", "--target", id];
    args.extend_from_slice(values);
    for (key, value) in op {
        args.push(key);
        args.push(value);
    }
    args.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn generators_expand_and_an_offset_end_follows_a_moved_point2() {
    let mut f = Fixture::new();
    let parts = f.parts();

    // count + pitch: one equal spacing, stored with its generator.
    f.ok(&strs(
        "create-bolt-group",
        &f.group_args(GROUP, &parts, WASHERED, &["--count", "3", "--pitch", "120"]),
    ));
    let group = &f.groups()[0];
    assert_eq!(text(group, "spacing_mode"), "pitch");
    assert_eq!(text(group, "spacing_value_mm"), "120");
    assert!(strings(group, "spacings_mm") == ["120", "120"]);

    // count + offset_end: the last fastener lies `offset_end` before `point2`; a moved `point2`
    // re-expands the row because the generator travels with the group. The row starts at 130,
    // clear of the first group's holes at 100, 220 and 340.
    let mut args = f.group_args(
        GROUP2,
        &parts,
        WASHERED,
        &["--count", "3", "--offset-end", "150"],
    );
    let slot = args.iter().position(|a| a == "--offset-start").unwrap();
    args[slot + 1] = "130".to_owned();
    f.ok(&strs("create-bolt-group", &args));
    let group = &f.groups()[1];
    assert_eq!(text(group, "spacing_mode"), "offset_end");
    assert_eq!(text(group, "spacing_value_mm"), "150");
    // (600 - 130 - 150) / 2: the last fastener stands at 450, 150 before point2.
    assert!(strings(group, "spacings_mm") == ["160", "160"], "{group:?}");
    f.ok(&f_args_edit(GROUP2, &["--point2", "800,250,0"], &[]));
    let group = &f.groups()[1];
    assert!(strings(group, "spacings_mm") == ["260", "260"], "{group:?}");
    // The row re-expanded around the moved point and regenerated its holes: still three
    // records per plate, all owned by the group.
    for part in &parts {
        let holes = holes_of(&f, part);
        assert_eq!(holes.len(), 6, "{part}");
        assert!(
            holes
                .iter()
                .all(|h| text(h, "generated_by") == GROUP || text(h, "generated_by") == GROUP2)
        );
    }
}

#[test]
fn length_modes_and_washer_cases_follow_the_amendment() {
    let mut f = Fixture::new();
    let parts = f.parts();

    // A washer a definition lacks refuses, as does one under a screw's missing nut.
    for designation in [BARE, SCREW] {
        let args = f.group_args(
            GROUP,
            &parts,
            designation,
            &["--spacings", "150", "--washer-under-nut", "true"],
        );
        assert_eq!(
            f.rejected(&strs("create-bolt-group", &args)),
            "INVALID_ARGUMENT"
        );
    }
    let args = f.group_args(
        GROUP,
        &parts,
        BARE,
        &["--spacings", "150", "--washer-under-head", "true"],
    );
    assert_eq!(
        f.rejected(&strs("create-bolt-group", &args)),
        "INVALID_ARGUMENT"
    );
    // A fixed length must be one of the definition's standard lengths and must cover the joint.
    for (length, code) in [("65", "INVALID_ARGUMENT"), ("30", "UNSUPPORTED_GEOMETRY")] {
        let args = f.group_args(
            GROUP,
            &parts,
            WASHERED,
            &[
                "--spacings",
                "150",
                "--length-mode",
                "fixed",
                "--length",
                length,
            ],
        );
        assert_eq!(
            f.rejected(&strs("create-bolt-group", &args)),
            code,
            "{length}"
        );
    }

    // An explicit switch overrides the definition's preference; a listed fixed length is kept.
    let args = f.group_args(
        GROUP,
        &parts,
        WASHERED,
        &[
            "--spacings",
            "150",
            "--washer-under-head",
            "false",
            "--length-mode",
            "fixed",
            "--length",
            "50",
        ],
    );
    f.ok(&strs("create-bolt-group", &args));
    let group = &f.groups()[0];
    assert_eq!(text(group, "washer_under_head"), "false");
    assert_eq!(text(group, "washer_under_nut"), "true");
    assert_eq!(text(group, "length_mode"), "fixed");
    assert_eq!(text(group, "length_mm"), "50");
}

#[test]
fn the_definition_is_pinned_repinned_and_the_holes_follow_a_repin() {
    let mut f = Fixture::new();
    let parts = f.parts();

    // The library file is copied beside the store and deleted after the create: the group keeps
    // its pinned definition, and a later edit without a repin needs no file.
    let library = f
        .shared
        .root
        .join(format!(
            "library-{}.usda",
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ))
        .to_str()
        .unwrap()
        .to_owned();
    fs::copy(Path::new(SYNTHETIC), &library).unwrap();
    let mut args = f.group_args(GROUP, &parts, WASHERED, &["--spacings", "150"]);
    let slot = args.iter().position(|a| a == "--fastener-library").unwrap();
    args[slot + 1] = library.clone();
    let created = f.ok(&strs("create-bolt-group", &args));
    let old_digest = text(&list(&created, "bolt_groups")[0], "fastener");
    fs::remove_file(&library).unwrap();
    f.ok(&f_args_edit(GROUP, &["--on-plane", "5"], &[]));
    let group = &f.groups()[0];
    assert_eq!(text(group, "fastener"), old_digest);
    assert_eq!(text(group, "on_plane_mm"), "5");

    // --repin follows the current library: the group takes the entry the file resolves now,
    // with the stored washer switches replaced, because the bare definition refuses true.
    let repin = f_args_edit(
        GROUP,
        &[
            "--repin",
            "true",
            "--fastener-library",
            SYNTHETIC,
            "--designation",
            BARE,
            "--washer-under-head",
            "false",
            "--washer-under-nut",
            "false",
        ],
        &[],
    );
    f.ok(&repin);
    let group = &f.groups()[0];
    assert_ne!(text(group, "fastener"), old_digest);
    assert_eq!(text(group, "washer_under_head"), "false");
    assert_eq!(text(group, "washer_under_nut"), "false");
    // The repinned record's length follows the new definition's lengths list over the same grip.
    assert_eq!(text(group, "length_mode"), "auto");
    assert_eq!(
        text(group, "length_mm"),
        "50",
        "grip 20 + nut 12 + head 14 + protrusion 4 = 50"
    );
    // The repin regenerates the holes: the records keep the group ownership and the count.
    for part in &parts {
        let holes = holes_of(&f, part);
        assert_eq!(holes.len(), 2, "{part}");
        for hole in &holes {
            assert_eq!(text(hole, "generated_by"), GROUP);
        }
    }
}

#[test]
fn wrong_rows_and_parts_are_refused_with_nothing_written() {
    let mut f = Fixture::new();
    let parts = f.parts();
    f.group(GROUP, &parts);

    // An unknown designation never creates a group.
    let args = f.group_args(GROUP2, &parts, "M-NO-SUCH", &["--spacings", "150"]);
    assert_eq!(f.rejected(&strs("create-bolt-group", &args)), "NOT_FOUND");

    // A missing library file is refused before anything is written.
    let missing = f
        .shared
        .root
        .join("no-library.usda")
        .to_str()
        .unwrap()
        .to_owned();
    let mut args = f.group_args(GROUP2, &parts, WASHERED, &["--spacings", "150"]);
    let slot = args.iter().position(|a| a == "--fastener-library").unwrap();
    args[slot + 1] = missing;
    assert_ne!(f.run(&strs("create-bolt-group", &args)).0, 0);

    // Degenerate rows: coincident points, a normal parallel to the row, a negative offset.
    for (key, value) in [
        ("--point2", "0,250,0"),
        ("--plane-normal", "1,0,0"),
        ("--offset-start", "-1"),
    ] {
        let mut args = f.group_args(GROUP2, &parts, WASHERED, &["--spacings", "150"]);
        let slot = args.iter().position(|a| a == key).unwrap();
        args[slot + 1] = value.to_owned();
        assert_eq!(
            f.rejected(&strs("create-bolt-group", &args)),
            "INVALID_ARGUMENT",
            "{key}"
        );
    }

    // A part the shank never crosses: a plate parked above and beside the row.
    let off = f.ok(&owned(
        "create-plate",
        &[
            "--name",
            "Off",
            "--new-id",
            "019a7bf0-0000-7000-8000-0000007100f4",
            "--width",
            "1000",
            "--length",
            "500",
            "--thickness",
            "10",
            "--translation",
            "0,300,30",
        ],
    ));
    let off = text(at(&off, &["plate"]), "element_id");
    let args = f.group_args(
        GROUP2,
        &[parts[0].clone(), off],
        WASHERED,
        &["--spacings", "150"],
    );
    assert_eq!(
        f.rejected(&strs("create-bolt-group", &args)),
        "INVALID_ARGUMENT"
    );

    // An assembly part, one part alone, a taken identity, and exclusive spacing forms.
    let assembly = f.ok(&owned("create-assembly", &["--name", "Frame"]));
    let assembly = text(&assembly, "element_id");
    let args = f.group_args(
        GROUP2,
        &[parts[0].clone(), assembly],
        WASHERED,
        &["--spacings", "150"],
    );
    assert_eq!(
        f.rejected(&strs("create-bolt-group", &args)),
        "UNSUPPORTED_GEOMETRY"
    );
    let args = f.group_args(
        GROUP2,
        &[parts[0].clone()],
        WASHERED,
        &["--spacings", "150"],
    );
    assert_eq!(
        f.rejected(&strs("create-bolt-group", &args)),
        "INVALID_ARGUMENT"
    );
    let args = f.group_args(GROUP, &parts, WASHERED, &["--spacings", "150"]);
    assert_eq!(
        f.rejected(&strs("create-bolt-group", &args)),
        "DUPLICATE_ID"
    );
    let args = f.group_args(
        GROUP2,
        &parts,
        WASHERED,
        &["--spacings", "150", "--count", "3", "--pitch", "120"],
    );
    assert_eq!(
        f.rejected(&strs("create-bolt-group", &args)),
        "INVALID_ARGUMENT"
    );
    let args = f.group_args(GROUP2, &parts, WASHERED, &["--pitch", "120"]);
    assert_eq!(
        f.rejected(&strs("create-bolt-group", &args)),
        "INVALID_ARGUMENT"
    );
    assert_eq!(f.groups().len(), 1);
}

#[test]
fn none_writes_no_part_and_a_member_part_refuses_generate() {
    let mut f = Fixture::new();
    let parts = f.parts();
    let plate = parts[0].clone();

    // The screw's default writes no part: the footprint is the quiet A14.3 one and the plate
    // keeps its mesh.
    let mesh_before = mesh_of(&f, &plate);
    let args = f.group_args(
        GROUP,
        &parts,
        SCREW,
        &["--spacings", "150", "--washer-under-head", "false"],
    );
    let quiet = f.ok(&strs("create-bolt-group", &args));
    assert_eq!(strings(&quiet, "changed_layers"), ["authored.usda"]);
    assert_eq!(entries(&quiet, "writes").len(), 8);
    assert_eq!(text(&f.groups()[0], "hole_mode"), "none");
    assert!(matches!(
        f.groups()[0].get("hole_clearance_mm"),
        Ok(Json::Null)
    ));
    assert_eq!(mesh_of(&f, &plate), mesh_before);
    assert!(holes_of(&f, &plate).is_empty());

    // An explicit generate overrides the screw's default and cuts the holes.
    let args = f.group_args(
        GROUP2,
        &parts,
        SCREW,
        &[
            "--spacings",
            "150",
            "--washer-under-head",
            "false",
            "--hole-mode",
            "generate",
        ],
    );
    let forced = f.ok(&strs("create-bolt-group", &args));
    assert_eq!(
        strings(&forced, "changed_layers"),
        Fixture::mesh_changed(&parts, false, true)
    );
    let holes = holes_of(&f, &plate);
    assert_eq!(holes.len(), 2);
    for hole in &holes {
        assert_eq!(text(hole, "generated_by"), GROUP2);
    }
    // A generating group stores its resolved clearance; the quiet one stores none.
    assert!(matches!(
        f.groups()[1].get("hole_clearance_mm"),
        Ok(Json::String(_))
    ));

    // A member part refuses generate wholesale (G14.1). With hole_mode none the length is
    // face-exact (issue 519 defect B): the beam's solid grips 30 mm along the shank (from the
    // bearing on its surface to its far face, the plates inside that span), so the screw's
    // requirement is 30 + protrusion 2 = 32 and the 40 mm standard covers it. Its conservative
    // box claimed over 60 mm of grip, which refused every listed length before the fix.
    let profile = text(
        at(
            &f.ok(&owned(
                "define-profile",
                &["--width", "100", "--height", "50"],
            )),
            &["definition"],
        ),
        "digest",
    );
    let beam = text(
        at(
            &f.ok(&owned(
                "create-member",
                &[
                    "--start",
                    "0,250,5",
                    "--end",
                    "3000,250,5",
                    "--profile-digest",
                    &profile,
                ],
            )),
            &["member"],
        ),
        "element_id",
    );
    let mut crossed = parts.clone();
    crossed.push(beam);
    let args = f.group_args(
        "019a7bf0-0000-7000-8000-000000710003",
        &crossed,
        WASHERED,
        &["--spacings", "150"],
    );
    assert_eq!(
        f.rejected(&strs("create-bolt-group", &args)),
        "UNSUPPORTED_GEOMETRY"
    );
    let mut args = f.group_args(
        "019a7bf0-0000-7000-8000-000000710003",
        &crossed,
        SCREW,
        &["--spacings", "150", "--washer-under-head", "false"],
    );
    args.extend([
        "--hole-mode".to_owned(),
        "none".to_owned(),
        "--op-id".to_owned(),
        "019a7bf0-0000-7000-8000-0000007100d1".to_owned(),
    ]);
    f.ok(&strs("create-bolt-group", &args));
    let group = f
        .groups()
        .into_iter()
        .find(|g| text(g, "id") == "019a7bf0-0000-7000-8000-000000710003")
        .expect("the screw group over the beam lists back");
    assert_eq!(text(&group, "length_mm"), "40");
}

#[test]
fn holes_are_removed_with_the_group_and_refuse_separate_edits() {
    let mut f = Fixture::new();
    let parts = f.parts();
    let plate = parts[0].clone();

    f.group(GROUP, &parts);
    assert_eq!(holes_of(&f, &plate).len(), 2);

    // A generated hole refuses its own edit and removal; the typed refusal names the group
    // through the record, and a hand-placed hole on the same plate still works.
    let hole = &holes_of(&f, &plate)[0];
    let record = text(hole, "operation_id");
    assert_eq!(
        f.rejected(&owned(
            "remove-hole",
            &["--id", &plate, "--operation-id", &record]
        )),
        "DEPENDENCY_CONFLICT"
    );
    assert_eq!(
        f.rejected(&owned(
            "update-hole",
            &[
                "--id",
                &plate,
                "--operation-id",
                &record,
                "--hole-round",
                "150,250,20"
            ]
        )),
        "DEPENDENCY_CONFLICT"
    );
    let manual = f.ok(&owned(
        "add-hole",
        &[
            "--id",
            &plate,
            "--operation-id",
            "019a7bf0-0000-7000-8000-0000007100e1",
            "--hole-round",
            "500,100,20",
        ],
    ));
    assert_eq!(
        at(&manual, &["element", "hole_records"])
            .array()
            .unwrap()
            .len(),
        3
    );

    // Turning generate off removes exactly the group's holes and keeps the manual one.
    f.ok(&f_args_edit(GROUP, &["--hole-mode", "none"], &[]));
    let holes = holes_of(&f, &plate);
    assert_eq!(holes.len(), 1);
    assert!(matches!(holes[0].get("generated_by"), Ok(Json::Null)));

    // Deleting the group removes its holes from the plates in the same revision.
    let mut args = f.group_args(GROUP2, &parts, WASHERED, &["--spacings", "150"]);
    args.extend([
        "--op-id".to_owned(),
        "019a7bf0-0000-7000-8000-0000007100d2".to_owned(),
    ]);
    f.ok(&strs("create-bolt-group", &args));
    assert_eq!(holes_of(&f, &plate).len(), 3);
    let deleted = f.ok(&owned("delete-element", &["--id", GROUP2]));
    // The delete writes the group's removal and both parts' hole records and meshes.
    assert_eq!(
        strings(&deleted, "changed_layers"),
        Fixture::mesh_changed(&parts, false, false)
    );
    let holes = holes_of(&f, &plate);
    assert_eq!(holes.len(), 1, "only the manual hole stays");
    for hole in &holes {
        assert!(matches!(hole.get("generated_by"), Ok(Json::Null)));
    }
}

/// The knee brace of #505: a beam whose placement turns it 40 degrees off the vertical and a
/// gusset plate whose placement stands it up out of its local XY plane take a bolt group with
/// `hole_mode = none`. The group commits and lists back with exit 0 and no
/// `UNSUPPORTED_GEOMETRY`: a rotated part is bounded by its placement, never refused.
#[test]
fn a_knee_brace_group_attaches_to_rotated_parts() {
    let f = Fixture::new();
    let profile = text(
        f.ok(&owned(
            "define-profile",
            &["--width", "60", "--height", "60"],
        ))
        .get("definition")
        .unwrap(),
        "digest",
    );
    // The brace's local axis runs along +Z; the placement turns it 40 degrees about +Y and
    // stands it at (100, 0, 1100), so its world axis runs from (100, 0, 1100) to about
    // (614, 0, 1713) and ends near the gusset.
    let brace = text(
        at(
            &f.ok(&owned(
                "create-member",
                &[
                    "--kind",
                    "beam",
                    "--name",
                    "B1",
                    "--new-id",
                    BRACE,
                    "--start",
                    "0,0,0",
                    "--end",
                    "0,0,800",
                    "--orientation-xyzw",
                    "0,0.3420201433256687,0,0.9396926207859084",
                    "--translation",
                    "100,0,1100",
                    "--profile-digest",
                    &profile,
                ],
            )),
            &["member"],
        ),
        "element_id",
    );
    // The plate is authored in its local XY; the 90-degree placement about +X stands it up in
    // the world XZ plane with its 12 mm thickness running toward -Y, so the plate occupies
    // x in [450, 850], y in [-12, 0], z in [1400, 1800].
    let gusset = text(
        at(
            &f.ok(&owned(
                "create-plate",
                &[
                    "--name",
                    "PL1",
                    "--new-id",
                    GUSSET,
                    "--width",
                    "400",
                    "--length",
                    "400",
                    "--thickness",
                    "12",
                    "--orientation-xyzw",
                    "0.7071067811865476,0,0,0.7071067811865476",
                    "--translation",
                    "450,0,1400",
                ],
            )),
            &["plate"],
        ),
        "element_id",
    );

    // A row of two fasteners along +X at y = -6.5 (inside the gusset's thickness), crossing
    // both rotated parts.
    let created = f.ok(&owned(
        "create-bolt-group",
        &[
            "--name",
            "BG1",
            "--new-id",
            GROUP3,
            "--parts",
            &format!("{brace},{gusset}"),
            "--fastener-library",
            SYNTHETIC,
            "--designation",
            WASHERED,
            "--point1",
            "500,-6.5,1600",
            "--point2",
            "650,-6.5,1600",
            "--plane-normal",
            "0,1,0",
            "--offset-start",
            "25",
            "--pitch",
            "100",
            "--count",
            "2",
            "--hole-mode",
            "none",
        ],
    ));
    assert_eq!(text(&created, "outcome"), "bolt_group_committed");
    assert_eq!(text(&created, "element_id"), GROUP3);

    // The same head reads back through the listing: exit 0, the group with its parts.
    let listed = f.groups();
    let group = listed
        .iter()
        .find(|g| text(g, "id") == GROUP3)
        .expect("the created group lists back");
    assert_eq!(strings(group, "parts"), [brace.clone(), gusset.clone()]);
    assert_eq!(text(group, "hole_mode"), "none");
    // The length is face-exact (issue 519 defect B): the first shank grips 36.5 mm of real
    // material (from its bearing inside the brace to the far face; the gusset lies inside that
    // span), so the requirement is 36.5 + washer 2 + nut 9 + protrusion 3 = 50.5 and the
    // smallest covering standard length is 60. The brace's conservative box reaches 48.9 mm
    // along the shank, whose requirement of 62.9 would demand the 80 mm entry.
    assert_eq!(text(group, "length_mm"), "60");
}

/// Issue 544: a bolt group belongs to its main part. Moving the main part carries the row
/// with it and regenerates the other parts' holes; moving another part keeps the fasteners
/// and regenerates that part's holes; parts moved apart, and a mirror, refuse with nothing
/// written; each follow is one undo unit, and moving back restores every mesh exactly.
#[test]
fn a_group_follows_its_main_part_and_never_comes_apart() {
    let mut f = Fixture::new();
    let parts = f.parts();
    let (low, high) = (parts[0].clone(), parts[1].clone());
    // The upper plate is the main part.
    let group = "019a7bf0-0000-7000-8000-0000007105a1";
    f.group(group, &[high.clone(), low.clone()]);
    let (low_mesh, high_mesh) = (mesh_of(&f, &low), mesh_of(&f, &high));
    let points = |f: &Fixture| {
        let g = f.groups()[0].clone();
        (strings(&g, "point1_mm"), strings(&g, "point2_mm"))
    };

    // The main part moves +50 along x: the row follows exactly, the fasteners stay in their
    // holes, and the lower plate's holes are regenerated under them.
    f.ok(&owned(
        "update-element",
        &["--id", &high, "--move-from", "0,0,0", "--move-to", "50,0,0"],
    ));
    assert_eq!(
        points(&f),
        (
            vec!["50".to_owned(), "250".to_owned(), "0".to_owned()],
            vec!["650".to_owned(), "250".to_owned(), "0".to_owned()]
        )
    );
    for part in [&low, &high] {
        let holes = holes_of(&f, part);
        assert_eq!(holes.len(), 2, "{part}");
        assert!(holes.iter().all(|h| text(h, "generated_by") == group));
    }
    assert_ne!(mesh_of(&f, &low), low_mesh, "the lower plate's holes moved");
    // The follow is one undo unit with the move.
    f.undo();
    assert_eq!(
        points(&f),
        (
            vec!["0".to_owned(), "250".to_owned(), "0".to_owned()],
            vec!["600".to_owned(), "250".to_owned(), "0".to_owned()]
        )
    );
    assert_eq!(mesh_of(&f, &low), low_mesh);

    // Another part moves: the fasteners stay, and that part's holes are regenerated in its
    // own frame; moving it back by an absolute placement restores its mesh exactly.
    f.ok(&owned(
        "update-element",
        &["--id", &low, "--move-from", "0,0,0", "--move-to", "0,40,0"],
    ));
    assert_eq!(
        points(&f),
        (
            vec!["0".to_owned(), "250".to_owned(), "0".to_owned()],
            vec!["600".to_owned(), "250".to_owned(), "0".to_owned()]
        )
    );
    assert_ne!(mesh_of(&f, &low), low_mesh);
    assert_eq!(
        mesh_of(&f, &high),
        high_mesh,
        "the main part's holes did not move"
    );
    f.ok(&owned(
        "update-element",
        &["--id", &low, "--translation", "0,0,0"],
    ));
    assert_eq!(mesh_of(&f, &low), low_mesh);
    assert_eq!(mesh_of(&f, &high), high_mesh);

    // Moved apart, the lower plate would no longer be crossed: the whole edit refuses and
    // the head keeps the node together.
    f.rejected(&owned(
        "update-element",
        &[
            "--id",
            &high,
            "--move-from",
            "0,0,0",
            "--move-to",
            "2000,0,0",
        ],
    ));
    // A mirror is not rigid: it refuses on a bolted part.
    assert_eq!(
        f.rejected(&owned(
            "update-element",
            &[
                "--id",
                &low,
                "--mirror-point",
                "500,0,0",
                "--mirror-normal",
                "1,0,0",
            ],
        )),
        "DEPENDENCY_CONFLICT"
    );
}
