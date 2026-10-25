// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic frame and closed-form references, MPL-2.0.

//! Q01: one real CLI story, from definitions through a personal draft to published output.
//! Every command starts a fresh process. No mock storage, renderer or geometry is used.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_json::{Json, Limits};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const USER: &str = "019a7bf0-0000-7000-8000-000000009001";
const PARTS: [&str; 5] = [
    "019a7bf0-0000-7000-8000-000000009010",
    "019a7bf0-0000-7000-8000-000000009011",
    "019a7bf0-0000-7000-8000-000000009012",
    "019a7bf0-0000-7000-8000-000000009013",
    "019a7bf0-0000-7000-8000-000000009014",
];

struct Story(PathBuf);

impl Story {
    fn new() -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("q01-alpha-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("published")).unwrap();
        Self(root)
    }

    fn path(&self, name: &str) -> String {
        self.0.join(name).to_str().unwrap().to_owned()
    }

    fn run(&self, store: &str, command: &str, args: &[&str], expected: i32) -> Json {
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args([command, "--store", &self.path(store)])
            .args(args)
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(expected),
            "{command} {args:?}: stdout={}, stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap()
    }

    fn ok(&self, store: &str, command: &str, args: &[&str]) -> Json {
        self.run(store, command, args, 0)
    }

    fn head(&self, store: &str) -> String {
        text(&self.ok(store, "revision-open", &[]), "revision_id")
    }
}

impl Drop for Story {
    fn drop(&mut self) {
        if std::env::var_os("BEAMCRAFT_Q01_KEEP").is_some() {
            eprintln!("Q01 artifacts: {}", self.0.display());
        } else {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

fn at<'a>(json: &'a Json, path: &[&str]) -> &'a Json {
    path.iter().fold(json, |value, key| value.get(key).unwrap())
}

fn text(json: &Json, key: &str) -> String {
    json.get(key).unwrap().string().unwrap().to_owned()
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1e-8 * expected.abs().max(1.0),
        "actual {actual}, expected {expected}"
    );
}

fn parts(inspection: &Json) -> BTreeMap<String, Json> {
    ["plates", "members"]
        .into_iter()
        .flat_map(|key| inspection.get(key).unwrap().array().unwrap())
        .map(|part| (text(part, "element_id"), part.clone()))
        .collect()
}

#[test]
fn a_detailed_frame_is_numbered_published_exported_and_reopened() {
    let s = Story::new();
    s.ok(
        "published",
        "define-material",
        &["--grade", "Q01-steel", "--density", "7850"],
    );
    s.ok(
        "published",
        "define-profile",
        &[
            "--designation",
            "Q01-RECT",
            "--width",
            "100",
            "--height",
            "50",
        ],
    );
    let base = s.head("published");
    s.ok(
        "published",
        "workspace-create",
        &["--workspace-dir", &s.path("draft"), "--user-id", USER],
    );

    // Two columns and a cut top beam; dimensions are invented, not a catalogue section.
    for (id, kind, start, end, cut) in [
        (PARTS[0], "column", "0,0,0", "0,0,1000", false),
        (PARTS[1], "column", "1000,0,0", "1000,0,1000", false),
        (PARTS[2], "beam", "0,0,1000", "1000,0,1000", true),
    ] {
        let mut args = vec![
            "--new-id",
            id,
            "--kind",
            kind,
            "--start",
            start,
            "--end",
            end,
            "--profile",
            "Q01-RECT",
            "--material",
            "Q01-steel",
            "--name",
            kind,
        ];
        if cut {
            args.extend_from_slice(&["--cut-point", "950,0,1000", "--cut-normal", "1,0,0"]);
        }
        s.ok("draft", "create-member", &args);
    }
    for (id, translation) in [(PARTS[3], "-100,-100,0"), (PARTS[4], "900,-100,0")] {
        s.ok(
            "draft",
            "create-plate",
            &[
                "--new-id",
                id,
                "--width",
                "200",
                "--length",
                "200",
                "--thickness",
                "10",
                "--hole-rect",
                "150,150,20,20",
                "--translation",
                translation,
                "--material",
                "Q01-steel",
                "--name",
                "Base plate",
            ],
        );
    }
    let assembly = text(
        &s.ok("draft", "create-assembly", &["--name", "Q01 frame"]),
        "element_id",
    );
    for id in PARTS {
        s.ok(
            "draft",
            "assembly-attach",
            &["--id", id, "--assembly-id", &assembly],
        );
    }
    s.ok(
        "draft",
        "create-bolt",
        &[
            "--name",
            "Q01 bolts",
            "--main",
            PARTS[3],
            "--secondary",
            PARTS[0],
            "--origin",
            "-25,0,5",
            "--direction",
            "1,0,0",
            "--count",
            "2",
            "--pitch",
            "50",
            "--diameter",
            "16",
        ],
    );
    s.ok(
        "draft",
        "create-weld",
        &[
            "--name",
            "Q01 weld",
            "--main",
            PARTS[4],
            "--secondary",
            PARTS[1],
            "--size",
            "6",
            "--location",
            "workshop",
            "--side",
            "both",
        ],
    );
    assert_eq!(
        s.ok("draft", "bolts", &[])
            .get("bolts")
            .unwrap()
            .array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        s.ok("draft", "welds", &[])
            .get("welds")
            .unwrap()
            .array()
            .unwrap()
            .len(),
        1
    );

    // Refused input must leave both the personal draft and the accepted head intact.
    let draft_head = s.head("draft");
    let refused = s.run(
        "draft",
        "create-plate",
        &["--width", "200", "--length", "200", "--thickness", "-1"],
        2,
    );
    assert_eq!(text(&refused, "code"), "INVALID_ARGUMENT");
    assert_eq!(s.head("draft"), draft_head);
    assert_eq!(s.head("published"), base);

    let before = parts(&s.ok("draft", "plate-inspect", &[]));
    assert_eq!(before.len(), 5);
    let volumes = [5_000_000.0, 5_000_000.0, 4_750_000.0, 396_000.0, 396_000.0];
    for (id, volume) in PARTS.into_iter().zip(volumes) {
        let part = &before[id];
        close(
            at(part, &["quantities", "volume", "mm3"]).float().unwrap(),
            volume,
        );
        close(
            at(part, &["quantities", "mass", "kg"]).float().unwrap(),
            volume * 7.85e-6,
        );
    }
    let token = text(&s.ok("draft", "numbering-preview", &[]), "preview_token");
    let approved = s.ok("draft", "numbering-approve", &["--preview-token", &token]);
    assert_eq!(approved.get("marks").unwrap().unsigned().unwrap(), 6);
    let numbering = s.ok("draft", "numbering-status", &[]);
    assert_eq!(text(at(&numbering, &["approval"]), "state"), "current");

    let published = s.ok(
        "published",
        "workspace-publish",
        &["--workspace-dir", &s.path("draft")],
    );
    assert_eq!(text(&published, "outcome"), "workspace_published");
    assert_ne!(s.head("published"), base);
    let accepted_head = s.head("published");
    let status = s.ok(
        "published",
        "workspace-status",
        &["--workspace-dir", &s.path("draft")],
    );
    assert_eq!(text(at(&status, &["workspace"]), "unsent_revisions"), "0");
    let reopened = parts(&s.ok("published", "plate-inspect", &[]));
    for id in PARTS {
        for key in ["name", "mesh_digest", "quantities"] {
            assert_eq!(reopened[id].get(key).unwrap(), before[id].get(key).unwrap());
        }
    }
    assert_eq!(s.ok("published", "numbering-status", &[]), numbering);

    // Five parts, once each: assembly membership and connection records add no BOM rows.
    for name in ["bom.csv", "bom-again.csv"] {
        s.ok("published", "bom-export", &["--out", &s.path(name)]);
    }
    let csv = fs::read_to_string(s.path("bom.csv")).unwrap();
    assert_eq!(csv, fs::read_to_string(s.path("bom-again.csv")).unwrap());
    let rows: Vec<Vec<&str>> = csv
        .lines()
        .skip(1)
        .map(|line| line.split(',').collect())
        .collect();
    assert_eq!(rows.len(), 5, "{csv}");
    let mut ids: Vec<_> = rows.iter().map(|row| row[1]).collect();
    ids.sort();
    assert_eq!(ids, PARTS);
    assert!(
        rows.iter().all(|row| row[0] != row[1]),
        "approved marks must reach the BOM"
    );
    close(
        rows.iter().map(|row| row[10].parse::<f64>().unwrap()).sum(),
        122.0047,
    );

    // Historical fork issue #9 also asks for IFC; keep this experimental compatibility check.
    s.ok("published", "export-ifc", &["--out", &s.path("frame.ifc")]);
    let ifc = beamcraft_ifc::verify::parse(&fs::read(s.path("frame.ifc")).unwrap()).unwrap();
    for (kind, count) in [("IFCPLATE", 2), ("IFCCOLUMN", 2), ("IFCBEAM", 1)] {
        assert_eq!(ifc.all(kind).len(), count, "{kind}");
    }
    assert_eq!(
        s.head("published"),
        accepted_head,
        "read/export commands cannot commit"
    );
    let replay = s.ok(
        "published",
        "workspace-publish",
        &["--workspace-dir", &s.path("draft")],
    );
    assert_eq!(text(&replay, "outcome"), "workspace_publish_replayed");
    assert_eq!(s.head("published"), accepted_head);
    drawing_lifecycle(&s, &accepted_head);
}

/// The existing D03 slice is part of the real frame story, without claiming D04-D08 acceptance.
fn drawing_lifecycle(s: &Story, accepted_head: &str) {
    let document = s.path("drawing");
    let id = |suffix: u16| format!("019a7bf0-0000-7000-8000-{suffix:012}");
    let drawing = id(9200);
    let view = id(9201);
    let dimension = id(9202);
    let create_op = id(9203);
    let create_revision = id(9204);
    let publish_op = id(9205);
    let publish_revision = id(9206);
    let version = id(9207);
    let anchor_a = format!("fid1:{}:member:v.x0.y0.z0", PARTS[0]);
    let anchor_b = format!("fid1:{}:member:v.x0.y0.z1", PARTS[0]);
    let created = s.ok(
        "published",
        "drawing-create",
        &[
            "--document-dir",
            &document,
            "--drawing-id",
            &drawing,
            "--view-id",
            &view,
            "--dimension-id",
            &dimension,
            "--id",
            PARTS[0],
            "--anchor-a",
            &anchor_a,
            "--anchor-b",
            &anchor_b,
            "--op-id",
            &create_op,
            "--revision-id",
            &create_revision,
            "--actor-id",
            USER,
        ],
    );
    assert_eq!(text(&created, "state"), "draft");
    assert_eq!(text(&created, "source_revision_id"), accepted_head);
    let publish_args = [
        "--document-dir",
        document.as_str(),
        "--expected-head",
        create_revision.as_str(),
        "--op-id",
        publish_op.as_str(),
        "--revision-id",
        publish_revision.as_str(),
        "--actor-id",
        USER,
        "--version-id",
        version.as_str(),
    ];
    let published = s.ok("published", "drawing-publish", &publish_args);
    assert_eq!(text(&published, "state"), "published");
    assert_eq!(text(&published, "version_id"), version);
    assert_eq!(text(&published, "source_revision_id"), accepted_head);
    let stored = published.encode(&Limits::DEFAULT).unwrap();
    fs::write(s.path("drawing-version.json"), &stored).unwrap();
    let inspect_args = [
        "--document-dir",
        document.as_str(),
        "--model-store",
        &s.path("published"),
    ];
    assert_eq!(
        text(
            &s.ok("published", "drawing-inspect", &inspect_args),
            "freshness"
        ),
        "current"
    );

    // A model edit makes the drawing stale; it cannot silently replace the archived source.
    s.ok(
        "draft",
        "update-element",
        &["--id", PARTS[0], "--name", "Q01 revised column"],
    );
    assert_eq!(s.head("published"), accepted_head);
    s.ok(
        "published",
        "workspace-publish",
        &["--workspace-dir", &s.path("draft")],
    );
    let changed_head = s.head("published");
    assert_ne!(changed_head, accepted_head);
    let inspected = s.ok("published", "drawing-inspect", &inspect_args);
    assert_eq!(text(&inspected, "freshness"), "stale");
    assert_eq!(text(&inspected, "source_revision_id"), accepted_head);
    assert_eq!(text(&inspected, "version_id"), version);
    assert_eq!(
        s.ok("published", "drawing-publish", &publish_args)
            .encode(&Limits::DEFAULT)
            .unwrap(),
        stored
    );
    let versions = s.ok(
        "published",
        "drawing-versions",
        &["--document-dir", &document],
    );
    let versions = versions.get("versions").unwrap().array().unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].encode(&Limits::DEFAULT).unwrap(), stored);
    assert_eq!(s.head("published"), changed_head);

    // Reopen the stored document through a fresh application reader and verify its actual archive.
    let documents =
        beamcraft_cli::drawing::document::store::DocumentStore::open(Path::new(&document)).unwrap();
    let reopened = documents.head().unwrap();
    assert_eq!(reopened.version.unwrap().to_string(), version);
    assert_eq!(reopened.source.pin().revision.to_string(), accepted_head);
    let archived = reopened.source.inspect().unwrap();
    let column = archived
        .members
        .iter()
        .find(|member| member.authored.id.to_string() == PARTS[0])
        .unwrap();
    assert_eq!(column.authored.name, "column");
    assert_eq!(fs::read(s.path("drawing-version.json")).unwrap(), stored);
}
