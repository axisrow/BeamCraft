// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic frame and closed-form references, MPL-2.0.

//! Expanded Q01 (fork issue axisrow/BeamCraft#318, upstream issue #334): one real-binary story that extends the
//! delivered alpha scenario into the model tools, views/filters, grids/levels, invalid-input and
//! cancellation regressions, and exact stored outputs. Every command starts a fresh process; no
//! mock storage, renderer or geometry is used. The original scenario stays in `alpha_scenario.rs`;
//! `docs/design/q01-alpha-matrix.md` records what this covers and what stays open with reasons.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support::{at, text};

use beamcraft_json::{Json, Limits};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const USER: &str = "019a7bf0-0000-7000-8000-00000000aa01";
const PARTS: [&str; 5] = [
    "019a7bf0-0000-7000-8000-00000000aa10",
    "019a7bf0-0000-7000-8000-00000000aa11",
    "019a7bf0-0000-7000-8000-00000000aa12",
    "019a7bf0-0000-7000-8000-00000000aa13",
    "019a7bf0-0000-7000-8000-00000000aa14",
];
// Identities added by the expanded steps: a split half and the operation ids.
const SPLIT_HALF: &str = "019a7bf0-0000-7000-8000-00000000aa15";
const SPLIT_OP: &str = "019a7bf0-0000-7000-8000-00000000aa20";
const GRID: &str = "019a7bf0-0000-7000-8000-00000000aa21";
const LEVEL: &str = "019a7bf0-0000-7000-8000-00000000aa22";

/// Closed-form volumes of the five parts before any tool touches them (as in the original
/// scenario): two columns, the cut beam, and two plates with one rectangular hole each.
const STARTING_VOLUMES: [f64; 5] = [5_000_000.0, 5_000_000.0, 4_750_000.0, 396_000.0, 396_000.0];

struct Story(PathBuf);

impl Story {
    fn new() -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("q01-expanded-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("published")).unwrap();
        fs::create_dir(root.join("empty-store")).unwrap();
        Self(root)
    }

    fn path(&self, name: &str) -> String {
        self.0.join(name).to_str().unwrap().to_owned()
    }

    /// Runs one fresh process and returns its exit code with the decoded stdout JSON.
    fn execute(&self, args: &[&str]) -> (i32, Json) {
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(args)
            .output()
            .unwrap();
        (
            output.status.code().unwrap(),
            beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap(),
        )
    }

    fn run(&self, store: &str, command: &str, args: &[&str], expected: i32) -> Json {
        let store = self.path(store);
        let mut all = vec![command, "--store", store.as_str()];
        all.extend_from_slice(args);
        let (code, json) = self.execute(&all);
        assert_eq!(code, expected, "{command} {args:?}: {json:?}");
        json
    }

    fn ok(&self, store: &str, command: &str, args: &[&str]) -> Json {
        self.run(store, command, args, 0)
    }

    fn head(&self, store: &str) -> String {
        text(&self.ok(store, "open-revision", &[]), "revision_id")
    }
}

impl Drop for Story {
    fn drop(&mut self) {
        if std::env::var_os("BEAMCRAFT_Q01_KEEP").is_some() {
            eprintln!("Q01 expanded artifacts: {}", self.0.display());
        } else {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1e-8 * expected.abs().max(1.0),
        "actual {actual}, expected {expected}"
    );
}

/// (element_id, volume mm3, mesh_digest) of every part, sorted by id.
fn parts(inspection: &Json) -> Vec<(String, f64, String)> {
    let mut all = Vec::new();
    for key in ["plates", "members"] {
        for part in inspection.get(key).unwrap().array().unwrap() {
            all.push((
                text(part, "element_id"),
                at(part, &["quantities", "volume", "mm3"]).float().unwrap(),
                text(part, "mesh_digest"),
            ));
        }
    }
    all.sort_by(|a, b| a.0.cmp(&b.0));
    all
}

fn volume_of(parts: &[(String, f64, String)], id: &str) -> f64 {
    parts
        .iter()
        .find(|(element, _, _)| element == id)
        .expect("part is present in the inspection")
        .1
}

#[test]
fn expanded_alpha_runs_tools_views_regressions_and_exact_outputs() {
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
    s.ok(
        "published",
        "create-workspace",
        &["--workspace-dir", &s.path("draft"), "--user-id", USER],
    );

    // The same synthetic frame as the delivered scenario: two columns and a beam, the beam
    // trimmed by an owned plane cut, two base plates with holes.
    for (id, kind, start, end) in [
        (PARTS[0], "column", "0,0,0", "0,0,1000"),
        (PARTS[1], "column", "1000,0,0", "1000,0,1000"),
        (PARTS[2], "beam", "0,0,1000", "1000,0,1000"),
    ] {
        s.ok(
            "draft",
            "create-member",
            &[
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
            ],
        );
    }
    s.ok(
        "draft",
        "cut-by-plane",
        &[
            "--id",
            PARTS[2],
            "--operation-id",
            "019a7bf0-0000-7000-8000-00000000aa17",
            "--point",
            "950,0,1000",
            "--normal",
            "1,0,0",
            "--keep",
            "negative",
        ],
    );
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
    let before = parts(&s.ok("draft", "inspect-elements", &[]));
    assert_eq!(before.len(), 5);
    for (id, volume) in PARTS.into_iter().zip(STARTING_VOLUMES) {
        close(volume_of(&before, id), volume);
    }

    // Model tools (split, copy, chamfer) change exactly the parts they name. The split cuts
    // the second column at mid height, so the halves sum to the closed-form column volume.
    s.ok(
        "draft",
        "split-part",
        &[
            "--id",
            PARTS[1],
            "--new-id",
            SPLIT_HALF,
            "--new-name",
            "Split column",
            "--operation-id",
            SPLIT_OP,
            "--point",
            "1000,0,500",
            "--normal",
            "0,0,1",
            "--keep",
            "positive",
        ],
    );
    let after_split = parts(&s.ok("draft", "inspect-elements", &[]));
    assert_eq!(after_split.len(), 6);
    close(
        volume_of(&after_split, PARTS[1]) + volume_of(&after_split, SPLIT_HALF),
        STARTING_VOLUMES[1],
    );
    let copied = s.ok(
        "draft",
        "copy-element",
        &[
            "--id",
            PARTS[4],
            "--offset",
            "0,300,0",
            "--name",
            "Copied plate",
        ],
    );
    let copy_id = text(at(&copied, &["element"]), "element_id");
    let after_copy = parts(&s.ok("draft", "inspect-elements", &[]));
    assert_eq!(after_copy.len(), 7);
    close(
        volume_of(&after_copy, copy_id.as_str()),
        volume_of(&after_copy, PARTS[4]),
    );
    s.ok(
        "draft",
        "chamfer-part",
        &[
            "--id",
            PARTS[3],
            "--operation-id",
            "019a7bf0-0000-7000-8000-00000000aa23",
            "--vertex-index",
            "0",
            "--first-distance",
            "15",
            "--second-distance",
            "15",
        ],
    );
    let after_chamfer = parts(&s.ok("draft", "inspect-elements", &[]));
    assert_eq!(after_chamfer.len(), 7);
    assert!(
        volume_of(&after_chamfer, PARTS[3]) < volume_of(&after_split, PARTS[3]),
        "chamfer must remove material"
    );
    for id in [PARTS[0], PARTS[1], PARTS[2], PARTS[4], SPLIT_HALF] {
        close(volume_of(&after_chamfer, id), volume_of(&after_split, id));
    }
    close(
        volume_of(&after_chamfer, copy_id.as_str()),
        volume_of(&after_copy, copy_id.as_str()),
    );

    // Undo returns the workspace across one operation; redo restores it. Workspace commands
    // address the workspace folder directly, without a store argument.
    let undo_json = s.execute(&["undo-workspace", "--workspace-dir", &s.path("draft")]);
    assert_eq!(undo_json.0, 0);
    assert_eq!(text(&undo_json.1, "outcome"), "workspace_undone");
    let after_undo = parts(&s.ok("draft", "inspect-elements", &[]));
    assert_eq!(after_undo.len(), 7);
    close(
        volume_of(&after_undo, PARTS[3]),
        volume_of(&after_split, PARTS[3]),
    );
    let redo_json = s.execute(&["redo-workspace", "--workspace-dir", &s.path("draft")]);
    assert_eq!(redo_json.0, 0);
    assert_eq!(text(&redo_json.1, "outcome"), "workspace_redone");
    let after_redo = parts(&s.ok("draft", "inspect-elements", &[]));
    assert_eq!(after_redo.len(), 7);
    close(
        volume_of(&after_redo, PARTS[3]),
        volume_of(&after_chamfer, PARTS[3]),
    );

    // Grids and levels are storey model data the real store carries (T01/D2).
    s.ok(
        "draft",
        "create-grid",
        &[
            "--name",
            "Q01 grid",
            "--new-id",
            GRID,
            "--u-expression",
            "0 1000",
            "--v-expression",
            "0",
        ],
    );
    assert_eq!(
        s.ok("draft", "list-grids", &[])
            .get("grids")
            .unwrap()
            .array()
            .unwrap()
            .len(),
        1
    );
    s.ok(
        "draft",
        "create-level",
        &[
            "--name",
            "Q01 level",
            "--new-id",
            LEVEL,
            "--elevation",
            "1000",
        ],
    );
    assert_eq!(
        s.ok("draft", "levels", &[])
            .get("levels")
            .unwrap()
            .array()
            .unwrap()
            .len(),
        1
    );

    // Assembly, bolts and welds complete the frame (the original scenario keeps the details).
    let assembly = text(
        &s.ok(
            "draft",
            "create-assembly",
            &["--name", "Q01 expanded frame"],
        ),
        "element_id",
    );
    for id in [
        PARTS[0],
        PARTS[1],
        PARTS[2],
        PARTS[3],
        PARTS[4],
        SPLIT_HALF,
        copy_id.as_str(),
    ] {
        s.ok(
            "draft",
            "attach-to-assembly",
            &["--id", id, "--assembly-id", &assembly],
        );
    }
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

    // Numbering and publication; the view bundle and BOM are then checked against the
    // published head.
    let token = text(&s.ok("draft", "preview-numbering", &[]), "preview_token");
    let approved = s.ok("draft", "approve-numbering", &["--preview-token", &token]);
    assert_eq!(approved.get("marks").unwrap().unsigned().unwrap(), 8);
    let published = s.ok(
        "published",
        "publish-workspace",
        &["--workspace-dir", &s.path("draft")],
    );
    assert_eq!(text(&published, "outcome"), "workspace_published");
    let accepted_head = s.head("published");

    // Reopen: every part keeps its exact volume and mesh across the publication round trip.
    let reopened = parts(&s.ok("published", "inspect-elements", &[]));
    assert_eq!(reopened.len(), after_redo.len());
    for (id, volume, digest) in &after_redo {
        let found = reopened.iter().find(|(other, _, _)| other == id).unwrap();
        close(found.1, *volume);
        assert_eq!(found.2, *digest, "{id} mesh must survive the round trip");
    }

    // Views: the synthetic scene stands in for P01 (recorded in the matrix), so the kind
    // filter is exercised there; the store itself exports its pinned view bundle (V03).
    let snapshot = s.execute(&["snapshot-view", "--plates", "10", "--hide", "support"]);
    assert_eq!(snapshot.0, 0);
    assert_eq!(text(&snapshot.1, "outcome"), "view_snapshot");
    assert_eq!(
        at(&snapshot.1, &["settings", "hidden_kinds"])
            .array()
            .unwrap()
            .len(),
        1
    );
    let view_args = ["--out", &s.path("view-1.bin")];
    s.ok("published", "export-view", &view_args);
    s.ok(
        "published",
        "export-view",
        &["--out", &s.path("view-2.bin")],
    );
    let first_bundle = fs::read(s.path("view-1.bin")).unwrap();
    assert!(!first_bundle.is_empty());
    assert_eq!(
        fs::read(s.path("view-2.bin")).unwrap(),
        first_bundle,
        "the view bundle must be deterministic for one head"
    );
    // A repeat export into the same file must refuse instead of overwriting the stored view.
    let refused_export = s.run("published", "export-view", &view_args, 2);
    assert_eq!(text(&refused_export, "code"), "INVALID_ARGUMENT");
    assert_eq!(fs::read(s.path("view-1.bin")).unwrap(), first_bundle);

    // Invalid-file regressions: each refusal leaves both heads intact.
    let missing_blob = s.run(
        "published",
        "put-blob",
        &["--file", &s.path("no-such-file")],
        2,
    );
    assert_eq!(text(&missing_blob, "outcome"), "rejected");
    let empty = s.run("empty-store", "open-revision", &[], 2);
    assert_eq!(text(&empty, "outcome"), "rejected");
    assert_eq!(s.head("published"), accepted_head);

    // Cancellation regression: an interrupted command answers "cancelled" and commits nothing;
    // recovery then reports a clean store (the full crash path lives in tests/recovery.rs).
    let cancelled = s.run(
        "published",
        "put-blob",
        &["--file", &s.path("view-1.bin"), "--cancel-after-ms", "0"],
        130,
    );
    assert_eq!(text(&cancelled, "outcome"), "cancelled");
    let recovered = s.ok("published", "recover-revision", &["--min-age-secs", "0"]);
    assert_eq!(text(&recovered, "outcome"), "recovery_complete");
    assert_eq!(s.head("published"), accepted_head);

    // Exact stored outputs: the BOM and its schedule summary reproduce byte for byte.
    for (name, summary) in [
        ("bom.csv", "bom-summary-1.csv"),
        ("bom-again.csv", "bom-summary-2.csv"),
    ] {
        s.ok(
            "published",
            "export-bom",
            &["--out", &s.path(name), "--summary", &s.path(summary)],
        );
    }
    assert_eq!(
        fs::read(s.path("bom.csv")).unwrap(),
        fs::read(s.path("bom-again.csv")).unwrap()
    );
    assert_eq!(
        fs::read(s.path("bom-summary-1.csv")).unwrap(),
        fs::read(s.path("bom-summary-2.csv")).unwrap()
    );

    // A published drawing over the numbered frame (the stale/version cycle lives in the
    // original scenario; this checks the fresh path end to end).
    let id = |suffix: u16| format!("019a7bf0-0000-7000-8000-{suffix:012}");
    let drawing = id(0xaa30);
    let view = id(0xaa31);
    let dimension = id(0xaa32);
    let create_op = id(0xaa33);
    let create_revision = id(0xaa34);
    let publish_op = id(0xaa35);
    let publish_revision = id(0xaa36);
    let version = id(0xaa37);
    let anchor_a = format!("fid1:{}:member:v.x0.y0.z0", PARTS[0]);
    let anchor_b = format!("fid1:{}:member:v.x0.y0.z1", PARTS[0]);
    let created = s.ok(
        "published",
        "create-drawing",
        &[
            "--document-dir",
            &s.path("drawing"),
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
    let published_drawing = s.ok(
        "published",
        "publish-drawing",
        &[
            "--document-dir",
            &s.path("drawing"),
            "--expected-head",
            &create_revision,
            "--op-id",
            &publish_op,
            "--revision-id",
            &publish_revision,
            "--actor-id",
            USER,
            "--version-id",
            &version,
        ],
    );
    assert_eq!(text(&published_drawing, "state"), "published");
    assert_eq!(
        text(
            &s.ok(
                "published",
                "inspect-drawing",
                &[
                    "--document-dir",
                    &s.path("drawing"),
                    "--model-store",
                    &s.path("published")
                ]
            ),
            "freshness"
        ),
        "current"
    );
}
