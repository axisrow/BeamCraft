// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic projects, MPL-2.0. The first proof uses small synthetic
// stores that author KM and KMD through the same steel module; the scopes keep them apart
// (F10 0.2 DV-SCOPE-004, F12 PC-OWN-002, T01 §9).

//! T01.5 end to end through the real `beamcraft` binary: grid and level commands answer to
//! the shared scope, an ordinary section edit leaves every grids/levels layer byte-equal,
//! and a grid of a pinned coordination project is usable read-only with every write refused
//! as foreign content.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use crate::support::{self, at, text};
use beamcraft_json::{Json, Limits};
use std::fs;
use std::process::Command;

const ACTOR: &str = "019a7bf0-0000-7000-8000-00000000f001";
const COORD_PROJECT: &str = "019a7bf0-0000-7000-8000-00000000f002";
const COORD_WORKSPACE: &str = "019a7bf0-0000-7000-8000-00000000f003";
const KM_PROJECT: &str = "019a7bf0-0000-7000-8000-00000000f004";
const KM_WORKSPACE: &str = "019a7bf0-0000-7000-8000-00000000f005";
const CONNECTION: &str = "019a7bf0-0000-7000-8000-00000000f006";
const CONNECTION_NO_FRAME: &str = "019a7bf0-0000-7000-8000-00000000f007";
const MAIN_GRID: &str = "019a7bf0-0000-7000-8000-00000000f010";
const COPY_ID: &str = "019a7bf0-0000-7000-8000-00000000f011";
const PLATE_ID: &str = "019a7bf0-0000-7000-8000-00000000f012";

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        Self {
            shared: support::StoreFixture::new("t015-grid-rights"),
        }
    }

    /// Runs a start/project command without the implicit `--store`.
    fn raw(&self, args: &[&str]) -> (i32, Json) {
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(args)
            .output()
            .unwrap();
        let body = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
        (output.status.code().unwrap(), body)
    }

    fn raw_ok(&self, args: &[&str]) -> Json {
        let (code, body) = self.raw(args);
        assert_eq!(code, 0, "{args:?}: {body:?}");
        body
    }

    /// One empty store or workspace directory, as a `--store` argument.
    fn dir(&self, name: &str) -> String {
        let path = self.shared.root.join(name);
        fs::create_dir(&path).unwrap();
        path.to_str().unwrap().to_owned()
    }

    fn run(&self, store: &str, command: &str, extra: &[&str]) -> (i32, Json) {
        support::StoreFixture::run_in(store, command, extra)
    }

    fn ok_in(&self, store: &str, command: &str, extra: &[&str]) -> Json {
        let (code, body) = self.run(store, command, extra);
        assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
        body
    }

    fn refused_in(&self, store: &str, command: &str, extra: &[&str]) -> Json {
        let (code, body) = self.run(store, command, extra);
        assert_eq!(code, 2, "{command} {extra:?}: {body:?}");
        body
    }

    /// Asserts one refusal with its code, contract rule and an unchanged head.
    fn refused_unchanged(
        &self,
        store: &str,
        command: &str,
        extra: &[&str],
        expected_code: &str,
        expected_rule: &str,
    ) {
        let before = self.head(store);
        let body = self.refused_in(store, command, extra);
        assert_eq!(text(&body, "code"), expected_code, "{command}: {body:?}");
        let diagnostics = at(&body, &["diagnostics"]).array().unwrap();
        assert_eq!(
            text(&diagnostics[0], "rule_id"),
            expected_rule,
            "{command}: {body:?}"
        );
        assert_eq!(self.head(store), before, "{command} wrote a revision");
    }

    fn head(&self, store: &str) -> String {
        let (code, body) = self.run(store, "open-revision", &[]);
        if code == 0 {
            text(&body, "revision_id")
        } else {
            String::new()
        }
    }

    /// Creates one sectioned project and returns its store path and project identity.
    fn project(&self, name: &str, code: &str, sections: &str) -> (String, String) {
        let parent = self.shared.root.join("models");
        fs::create_dir_all(&parent).unwrap();
        let config = self.shared.root.join("config");
        let reply = self.raw_ok(&[
            "create-project",
            "--parent",
            parent.to_str().unwrap(),
            "--name",
            name,
            "--code",
            code,
            "--sections",
            sections,
            "--actor-id",
            ACTOR,
            "--config-dir",
            config.to_str().unwrap(),
        ]);
        let project = at(&reply, &["project"]);
        (text(project, "path"), text(project, "project_id"))
    }

    fn workspace(
        &self,
        store: &str,
        dir: &str,
        code: &str,
        section: &str,
        zone: Option<&str>,
        shared: bool,
    ) {
        let mut args = vec![
            "create-workspace",
            "--store",
            store,
            "--workspace-dir",
            dir,
            "--scope-code",
            code,
            "--scope-section",
            section,
        ];
        if let Some(zone) = zone {
            args.extend(["--scope-zone", zone]);
        }
        if shared {
            args.extend(["--scope-shared", "true"]);
        }
        self.raw_ok(&args);
    }

    fn publish(&self, store: &str, dir: &str) -> Json {
        self.ok_in(store, "publish-workspace", &["--workspace-dir", dir])
    }

    /// The bytes of one layer of the revision a publish reply carries.
    fn layer_of(&self, store: &str, revision: &Json, name: &str) -> Vec<u8> {
        let layers = at(revision, &["revision", "manifest", "layers"])
            .array()
            .unwrap();
        let digest = layers
            .iter()
            .find(|l| text(l, "name") == name)
            .unwrap_or_else(|| panic!("{name} missing from the revision layers"))
            .get("digest")
            .unwrap()
            .string()
            .unwrap()
            .to_owned();
        fs::read(
            std::path::Path::new(store)
                .join("blobs")
                .join(&digest[..2])
                .join(&digest),
        )
        .unwrap()
    }
}

/// The axis identity of the first `u` line of the first grid of a grid reply.
fn first_axis(reply: &Json) -> String {
    text(
        at(
            at(
                at(reply, &["grids"]).array().unwrap().first().unwrap(),
                &["u"],
            ),
            &["lines"],
        )
        .array()
        .unwrap()
        .first()
        .unwrap(),
        "axis_id",
    )
}

/// The coordination project with one GR-P01-like grid and a KM section store pinning it,
/// with a declared translation and a local level for placement heights. Returns the store
/// paths, the pinned revision and the level identity.
fn pinned_setup(f: &Fixture) -> (String, String, String, String, String, String) {
    let coord = f.dir("coord");
    let created = f.ok_in(
        &coord,
        "create-grid",
        &[
            "--project-id",
            COORD_PROJECT,
            "--workspace-id",
            COORD_WORKSPACE,
            "--name",
            "Main",
            "--new-id",
            MAIN_GRID,
            "--u-expression",
            "0 3*6000",
            "--v-expression",
            "0 7500 4500",
        ],
    );
    let axis = first_axis(&created);
    let coord_head = f.head(&coord);
    let km = f.dir("km");
    let profile = f.ok_in(
        &km,
        "define-profile",
        &[
            "--project-id",
            KM_PROJECT,
            "--workspace-id",
            KM_WORKSPACE,
            "--width",
            "100",
            "--height",
            "50",
        ],
    );
    let digest = text(at(&profile, &["definition"]), "digest");
    let level = f.ok_in(
        &km,
        "create-level",
        &[
            "--project-id",
            KM_PROJECT,
            "--workspace-id",
            KM_WORKSPACE,
            "--name",
            "L",
            "--elevation",
            "3000",
        ],
    );
    let level_id = text(&level, "element_id");
    f.ok_in(
        &km,
        "connect-project",
        &[
            "--project-id",
            KM_PROJECT,
            "--workspace-id",
            KM_WORKSPACE,
            "--source-store",
            &coord,
            "--source-project",
            COORD_PROJECT,
            "--source-revision",
            &coord_head,
            "--connection-id",
            CONNECTION,
            "--label",
            "shared grids",
            "--translation",
            "1000,2000,0",
            "--orientation",
            "0,0,0,1",
        ],
    );
    (coord, coord_head, km, axis, level_id, digest)
}

/// GR-P11: an ordinary section edit leaves every grids/levels layer byte-equal, and every
/// grid or level command from a workspace without the shared authority is `PERMISSION_DENIED`.
#[test]
fn an_ordinary_section_edit_leaves_every_grids_layer_byte_equal() {
    let f = Fixture::new();
    let (store, _project_id) = f.project("Rights", "2401-DEMO", "KM,K1:KMD");

    // Both partitions get a grid and a level from their shared-authorized workspaces.
    let wcommon = f.dir("w-common");
    f.workspace(&store, &wcommon, "2401-DEMO", "KM", None, true);
    let spare = f.ok_in(
        &wcommon,
        "create-grid",
        &["--name", "Spare", "--u-expression", "0 6000"],
    );
    let spare_id = text(&spare, "element_id");
    let main = f.ok_in(
        &wcommon,
        "create-grid",
        &[
            "--name",
            "Main",
            "--u-expression",
            "0 3*6000",
            "--v-expression",
            "0 7500 4500",
        ],
    );
    let main_id = text(&main, "element_id");
    let main_axis = first_axis(&main);
    let l0 = f.ok_in(
        &wcommon,
        "create-level",
        &["--name", "L0", "--elevation", "3000"],
    );
    let l0_id = text(&l0, "element_id");
    let published_common = f.publish(&store, &wcommon);

    let wk1 = f.dir("w-k1");
    f.workspace(&store, &wk1, "2401-DEMO", "KMD", Some("K1"), true);
    f.ok_in(
        &wk1,
        "create-grid",
        &["--name", "K1Grid", "--u-expression", "0 6000"],
    );
    f.ok_in(
        &wk1,
        "create-level",
        &["--name", "L1", "--elevation", "6000"],
    );
    let published_k1 = f.publish(&store, &wk1);
    let common_bytes = f.layer_of(&store, &published_common, "2401-DEMO_Grids.usda");
    let k1_bytes = f.layer_of(&store, &published_k1, "2401-DEMO_K1_Grids.usda");

    // The KMD section renames and moves its own part; both shared layers answer byte for byte.
    let wkmd = f.dir("w-kmd");
    f.workspace(&store, &wkmd, "2401-DEMO", "KMD", Some("K1"), false);
    f.ok_in(
        &wkmd,
        "create-plate",
        &[
            "--new-id",
            PLATE_ID,
            "--name",
            "KMD plate",
            "--width",
            "1000",
            "--length",
            "500",
            "--thickness",
            "10",
        ],
    );
    f.ok_in(
        &wkmd,
        "update-element",
        &["--id", PLATE_ID, "--name", "KMD renamed"],
    );
    f.ok_in(
        &wkmd,
        "update-element",
        &["--id", PLATE_ID, "--translation", "500,250,0"],
    );
    let published_kmd = f.publish(&store, &wkmd);
    assert_eq!(
        f.layer_of(&store, &published_kmd, "2401-DEMO_Grids.usda"),
        common_bytes,
        "the ordinary section edit changed the common grids layer"
    );
    assert_eq!(
        f.layer_of(&store, &published_kmd, "2401-DEMO_K1_Grids.usda"),
        k1_bytes,
        "the ordinary section edit changed the zone grids layer"
    );

    // GR-P11 negative: every grid or level write answers to the shared scope.
    let axis_ids = "019a7bf0-0000-7000-8000-00000000fa01,019a7bf0-0000-7000-8000-00000000fa02,\
019a7bf0-0000-7000-8000-00000000fa03,019a7bf0-0000-7000-8000-00000000fa04,\
019a7bf0-0000-7000-8000-00000000fa05,019a7bf0-0000-7000-8000-00000000fa06,\
019a7bf0-0000-7000-8000-00000000fa07";
    let negatives: [(&str, Vec<&str>); 10] = [
        (
            "create-grid",
            vec!["--name", "Sneaky", "--u-expression", "0 6000"],
        ),
        ("update-grid", vec!["--id", &main_id, "--name", "X"]),
        (
            "draw-grid-line",
            vec![
                "--id", &main_id, "--family", "u", "--label", "Z9", "--from", "0,0", "--to",
                "1000,0",
            ],
        ),
        (
            "reshape-grid-line",
            vec![
                "--id",
                &main_id,
                "--axis-id",
                &main_axis,
                "--from",
                "0,0",
                "--to",
                "2000,0",
            ],
        ),
        (
            "remove-grid-line",
            vec!["--id", &main_id, "--axis-id", &main_axis],
        ),
        (
            "relabel-grid-line",
            vec!["--id", &main_id, "--axis-id", &main_axis, "--label", "Q7"],
        ),
        (
            "create-level",
            vec!["--name", "SneakyL", "--elevation", "1000"],
        ),
        ("delete-element", vec!["--id", &main_id]),
        ("delete-element", vec!["--id", &l0_id]),
        (
            "copy-element",
            vec![
                "--id",
                &main_id,
                "--new-id",
                COPY_ID,
                "--new-axis-ids",
                axis_ids,
            ],
        ),
    ];
    for (command, extra) in &negatives {
        f.refused_unchanged(&wkmd, command, extra, "PERMISSION_DENIED", "DV-SCOPE-004");
    }

    // The routing fix opens the shared path again: an authorized workspace deletes a grid
    // and an unassigned level (T01 §9).
    f.ok_in(&wcommon, "delete-element", &["--id", &spare_id]);
    f.ok_in(&wcommon, "delete-element", &["--id", &l0_id]);
}

/// GR-P12: a section project places a member at an intersection of a pinned coordination
/// grid; the coordinates follow the connection's declared placement, and the coordination
/// project stays byte-identical.
#[test]
fn a_member_places_on_a_pinned_coordination_grid() {
    let f = Fixture::new();
    let (coord, coord_head, km, _axis, level_id, digest) = pinned_setup(&f);

    // GR-P08 numbers through the pin: B,3 is (12000, 7500) in the source frame and the
    // connection adds (1000, 2000, 0).
    let member = f.ok_in(
        &km,
        "create-member",
        &[
            "--project-id",
            KM_PROJECT,
            "--workspace-id",
            KM_WORKSPACE,
            "--start-at-grid",
            &format!("{CONNECTION}:{MAIN_GRID}:B,3"),
            "--end-at-grid",
            &format!("{CONNECTION}:{MAIN_GRID}:B,4"),
            "--level",
            &level_id,
            "--offset",
            "500",
            "--profile-digest",
            &digest,
        ],
    );
    let points = at(&member, &["member", "axis_points_mm"])
        .array()
        .unwrap()
        .iter()
        .map(|p| {
            p.array()
                .unwrap()
                .iter()
                .map(|v| v.float().unwrap())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        points,
        vec![vec![13000.0, 9500.0, 3500.0], vec![19000.0, 9500.0, 3500.0]]
    );

    // The pinned query answers in the parent frame.
    let point = f.ok_in(
        &km,
        "resolve-grid-point",
        &[
            "--grid",
            &format!("{CONNECTION}:{MAIN_GRID}"),
            "--labels",
            "B,3",
        ],
    );
    let world = at(&point, &["point_world_mm"])
        .array()
        .unwrap()
        .iter()
        .map(|v| v.float().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(world, vec![13000.0, 9500.0]);

    // The pinned listing shows the connection's verified grids.
    let listed = f.ok_in(&km, "list-grids", &["--connection", CONNECTION]);
    assert_eq!(text(&listed, "outcome"), "grids_pinned");
    let grids = at(&listed, &["grids"]).array().unwrap();
    assert_eq!(grids.len(), 1);
    assert_eq!(text(&grids[0], "id"), MAIN_GRID);

    // The coordination project is untouched by all the section's work.
    assert_eq!(f.head(&coord), coord_head);

    // A connection without a declared placement is an explicit unknown alignment.
    f.ok_in(
        &km,
        "connect-project",
        &[
            "--project-id",
            KM_PROJECT,
            "--workspace-id",
            KM_WORKSPACE,
            "--source-store",
            &coord,
            "--source-project",
            COORD_PROJECT,
            "--source-revision",
            &coord_head,
            "--connection-id",
            CONNECTION_NO_FRAME,
            "--label",
            "no frame",
        ],
    );
    f.refused_unchanged(
        &km,
        "resolve-grid-point",
        &[
            "--grid",
            &format!("{CONNECTION_NO_FRAME}:{MAIN_GRID}"),
            "--labels",
            "B,3",
        ],
        "DEPENDENCY_UNAVAILABLE",
        "PC-FRAME-001",
    );
}

/// GR-N16: every write aimed at a grid of a pinned coordination project is
/// `FOREIGN_READ_ONLY` (PC-OWN-002), from a classic section store and from a scoped
/// workspace with and without the shared authority.
#[test]
fn a_write_to_a_pinned_coordination_grid_is_foreign_read_only() {
    let f = Fixture::new();
    let (_coord, coord_head, km, axis, _level_id, _digest) = pinned_setup(&f);
    let axis_ids = "019a7bf0-0000-7000-8000-00000000fa01,019a7bf0-0000-7000-8000-00000000fa02,\
019a7bf0-0000-7000-8000-00000000fa03,019a7bf0-0000-7000-8000-00000000fa04,\
019a7bf0-0000-7000-8000-00000000fa05,019a7bf0-0000-7000-8000-00000000fa06,\
019a7bf0-0000-7000-8000-00000000fa07";

    // The dedicated grid commands refuse before any local or scope decision.
    f.refused_unchanged(
        &km,
        "update-grid",
        &["--id", MAIN_GRID, "--name", "X"],
        "FOREIGN_READ_ONLY",
        "PC-OWN-002",
    );
    f.refused_unchanged(
        &km,
        "draw-grid-line",
        &[
            "--id", MAIN_GRID, "--family", "u", "--label", "Z9", "--from", "0,0", "--to", "1000,0",
        ],
        "FOREIGN_READ_ONLY",
        "PC-OWN-002",
    );
    f.refused_unchanged(
        &km,
        "relabel-grid-line",
        &["--id", MAIN_GRID, "--axis-id", &axis, "--label", "Q7"],
        "FOREIGN_READ_ONLY",
        "PC-OWN-002",
    );
    f.refused_unchanged(
        &km,
        "remove-grid-line",
        &["--id", MAIN_GRID, "--axis-id", &axis],
        "FOREIGN_READ_ONLY",
        "PC-OWN-002",
    );
    f.refused_unchanged(
        &km,
        "copy-element",
        &[
            "--id",
            MAIN_GRID,
            "--new-id",
            COPY_ID,
            "--new-axis-ids",
            axis_ids,
        ],
        "FOREIGN_READ_ONLY",
        "PC-OWN-002",
    );
    f.refused_unchanged(
        &km,
        "delete-element",
        &["--id", MAIN_GRID],
        "FOREIGN_READ_ONLY",
        "PC-OWN-002",
    );

    // The scoped arrangement: the descriptor and its pins live on the parent store (the
    // descriptor changes are the explicit shared commands), and both workspaces fork from it.
    let (store, project_id) = f.project("Scoped", "2402-DEMO", "KM");
    f.ok_in(
        &store,
        "connect-project",
        &[
            "--project-id",
            &project_id,
            "--source-store",
            &_coord,
            "--source-project",
            COORD_PROJECT,
            "--source-revision",
            &coord_head,
            "--connection-id",
            CONNECTION,
            "--label",
            "shared grids",
            "--translation",
            "0,0,0",
            "--orientation",
            "0,0,0,1",
        ],
    );
    let wshared = f.dir("w-shared");
    f.workspace(&store, &wshared, "2402-DEMO", "KM", None, true);
    f.refused_unchanged(
        &wshared,
        "update-grid",
        &["--id", MAIN_GRID, "--name", "Y"],
        "FOREIGN_READ_ONLY",
        "PC-OWN-002",
    );
    let wplain = f.dir("w-plain");
    f.workspace(&store, &wplain, "2402-DEMO", "KM", None, false);
    f.refused_unchanged(
        &wplain,
        "update-grid",
        &["--id", MAIN_GRID, "--name", "Z"],
        "FOREIGN_READ_ONLY",
        "PC-OWN-002",
    );
}
