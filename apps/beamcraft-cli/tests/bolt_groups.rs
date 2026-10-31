// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic scenarios; the fastener values are the shipped synthetic
// library's, which follows no national standard, MPL-2.0.

//! A14.3 end to end through the real `beamcraft` binary: `create-bolt-group` places a row of
//! identical fasteners of a pinned library definition between two or more parts,
//! `update-bolt-group` edits one group as one atomic edit and one undo unit, and `bolts` lists
//! both forms with the A05 groups as annotations. The positions, generators, length modes and
//! washer cases of the bolt amendment section 9 drive the assertions, with the amendment's
//! negative cases refused without a revision.

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
const USER: &str = "019a7bf0-0000-7000-8000-000000719001";
const UPGRADE_OP: &str = "019a7bf0-0000-7000-8000-0000007100c1";

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
    fn annotations(&self) -> Vec<Json> {
        list(&self.ok(&strs("bolts", &[])), "bolts").to_vec()
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

#[test]
fn a_group_places_its_row_and_edits_as_one_undo_unit() {
    let mut f = Fixture::new();
    let parts = f.parts();

    let created = f.group(GROUP, &parts);
    assert_eq!(text(&created, "outcome"), "bolt_group_committed");
    assert_eq!(text(&created, "element_id"), GROUP);
    // No body: only the authored layer changes.
    assert_eq!(strings(&created, "changed_layers"), ["authored.usda"]);
    // The parts' lifecycles and the pinned definition are read; the group's seven groups and
    // the definition's entry (pinned by the same transaction) are written.
    let writes = entries(&created, "writes");
    assert_eq!(writes.len(), 8);
    assert_eq!(
        writes.iter().filter(|(target, _)| target == GROUP).count(),
        7
    );
    let reads = entries(&created, "reads");
    for part in &parts {
        assert!(reads.contains(&(part.clone(), "lifecycle".to_owned())));
    }
    assert_eq!(
        reads.len(),
        4,
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
    assert_eq!(text(group, "length_mm"), "40");
    assert_eq!(text(group, "washer_under_head"), "true");
    assert_eq!(text(group, "washer_under_nut"), "true");
    assert_eq!(text(group, "count"), "2");
    assert_eq!(strings(group, "point1_mm"), ["0", "250", "0"]);
    assert_eq!(strings(group, "point2_mm"), ["600", "250", "0"]);
    assert_eq!(strings(group, "plane_normal_mm"), ["0", "0", "1"]);

    // One atomic edit and one undo unit: moving `point2` keeps the authoritative spacings, and
    // the undo returns the whole record.
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
    f.undo();
    let group = &f.groups()[0];
    assert_eq!(strings(group, "point2_mm"), ["600", "250", "0"]);
    assert_eq!(text(group, "length_mm"), "40");

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
    // re-expands the row because the generator travels with the group.
    f.ok(&strs(
        "create-bolt-group",
        &f.group_args(
            GROUP2,
            &parts,
            WASHERED,
            &["--count", "3", "--offset-end", "150"],
        ),
    ));
    let group = &f.groups()[1];
    assert_eq!(text(group, "spacing_mode"), "offset_end");
    assert_eq!(text(group, "spacing_value_mm"), "150");
    // (600 - 100 - 150) / 2: the last fastener stands at 450, 150 before point2.
    assert!(strings(group, "spacings_mm") == ["175", "175"], "{group:?}");
    f.ok(&f_args_edit(GROUP2, &["--point2", "800,250,0"], &[]));
    let group = &f.groups()[1];
    assert!(strings(group, "spacings_mm") == ["275", "275"], "{group:?}");
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
fn the_definition_is_pinned_repinned_and_an_a05_group_is_upgraded() {
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
        "40",
        "grip 20 + nut 12 + protrusion 4 = 36"
    );

    // An A05 annotation group is upgraded by supplying the placement inputs and a fastener.
    // The A05 bounds rule wants both bolts' positions inside both parts, so the annotation
    // joins two coincident plates (the same box), not the stack.
    let twin = f.plate("019a7bf0-0000-7000-8000-0000007100f5", "0");
    let annotation = f.ok(&owned(
        "create-bolt",
        &[
            "--name",
            "A1",
            "--main",
            twin.as_str(),
            "--secondary",
            parts[0].as_str(),
            "--origin",
            "100,250,5",
            "--direction",
            "1,0,0",
            "--count",
            "2",
            "--pitch",
            "150",
            "--diameter",
            "12",
        ],
    ));
    let a05_id = text(&annotation, "element_id");
    assert_eq!(f.annotations().len(), 1);
    let mut upgrade = f.group_args(&a05_id, &parts, WASHERED, &["--spacings", "150,150"]);
    let slot = upgrade.iter().position(|a| a == "--new-id").unwrap();
    upgrade[slot] = "--target".to_owned();
    upgrade.extend(["--op-id".to_owned(), UPGRADE_OP.to_owned()]);
    let upgraded = f.ok(&strs("update-bolt-group", &upgrade));
    assert_eq!(text(&upgraded, "outcome"), "bolt_group_committed");
    assert!(f.annotations().is_empty(), "the upgrade replaces the form");
    // The same line replays: the guard against a silent definition change gives way to the
    // replay comparison for an already accepted operation, in either order of the checks.
    let again = f.ok(&strs("update-bolt-group", &upgrade));
    assert_eq!(text(&again, "outcome"), "bolt_group_replayed");
    let groups = f.groups();
    assert_eq!(groups.len(), 2);
    assert_eq!(text(&groups[1], "id"), a05_id);
    assert_eq!(text(&groups[1], "count"), "3");
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
