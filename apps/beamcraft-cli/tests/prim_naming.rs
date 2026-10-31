// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! U16 end to end: every category, cross-category links, stable paths, view export.
//! All generated model data are contributor-original synthetic fixtures (ATTRIBUTION.md).

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use beamcraft_engine::{Limits as ViewLimits, ViewSnapshot};
use beamcraft_host::{revision::RevisionStore, storage::BlobStore};
use beamcraft_json::{Json, Limits};
use beamcraft_layer_engine::{
    Asset, BACKEND, Budgets, Cancellation, Control, Identity, OpenRequest, Prim, Profile, Reader,
};
use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};

const PLATE: &str = "019a7bf0-0000-7000-8000-000000139001";
const BEAM: &str = "019a7bf0-0000-7000-8000-000000139002";
const COLUMN: &str = "019a7bf0-0000-7000-8000-000000139003";
const ASSEMBLY: &str = "019a7bf0-0000-7000-8000-000000139004";
const LEVEL: &str = "019a7bf0-0000-7000-8000-000000139005";
const WELD: &str = "019a7bf0-0000-7000-8000-000000139006";
const BOLT: &str = "019a7bf0-0000-7000-8000-000000139007";
const REFERENCE: &str = "019a7bf0-0000-7000-8000-000000139008";

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("u16-prim-naming-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::create_dir(root.join("store")).unwrap();
        Self(root)
    }
    fn store(&self) -> RevisionStore {
        RevisionStore::new(BlobStore::new(self.0.join("store")).unwrap())
    }
    fn run(&self, command: &str, args: &[&str]) -> Json {
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .arg(command)
            .arg("--store")
            .arg(self.0.join("store"))
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{command}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap()
    }
    fn prims(&self) -> Vec<Prim> {
        let revision = self.store().open().unwrap();
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
        let index = reader.element_index(&opened.context, &control).unwrap();
        assert_eq!(index.len(), 8);
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
impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(self.0.is_absolute() && self.0.file_name().is_some());
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn string<'a>(value: &'a Json, key: &str) -> &'a str {
    value.get(key).unwrap().string().unwrap()
}
fn path(prefix: &str, id: &str) -> String {
    format!("{prefix}{}", id.replace('-', ""))
}
fn prim<'a>(prims: &'a [Prim], path: &str) -> &'a Prim {
    prims.iter().find(|p| p.path == path).unwrap()
}
fn targets(prims: &[Prim], at: &str, relationship: &str) -> Vec<String> {
    prim(prims, at)
        .relationships
        .iter()
        .find(|r| r.name == relationship)
        .unwrap()
        .targets
        .clone()
}

#[test]
fn all_categories_reopen_at_rule_paths_and_keep_identity_through_edits_and_view_queries() {
    let f = Fixture::new();
    let material = f.run("define-material", &["--density", "7850"]);
    let material = material.get("definition").unwrap();
    let material_digest = string(material, "digest");
    let material_path = format!("/World/Definitions/Materials/MAT_{material_digest}");
    assert_eq!(string(material, "prim_path"), material_path);
    let profile = f.run("define-profile", &["--width", "100", "--height", "50"]);
    let profile = profile.get("definition").unwrap();
    let profile_digest = string(profile, "digest");
    let profile_path = format!("/World/Definitions/Profiles/PRF_{profile_digest}");
    assert_eq!(string(profile, "prim_path"), profile_path);
    let plate_path = path("/World/Steel/Parts/PRT_", PLATE);
    let plate = f.run(
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
            "--material-digest",
            material_digest,
        ],
    );
    assert_eq!(string(plate.get("plate").unwrap(), "prim_path"), plate_path);
    for (id, kind) in [(BEAM, "beam"), (COLUMN, "column")] {
        let member = f.run(
            "create-member",
            &[
                "--start",
                "0,0,0",
                "--end",
                "1000,0,0",
                "--profile-digest",
                profile_digest,
                "--material-digest",
                material_digest,
                "--kind",
                kind,
                "--new-id",
                id,
            ],
        );
        assert_eq!(
            string(member.get("member").unwrap(), "prim_path"),
            path("/World/Steel/Parts/PRT_", id)
        );
    }
    f.run(
        "create-assembly",
        &["--name", "Frame", "--new-id", ASSEMBLY],
    );
    f.run(
        "attach-to-assembly",
        &["--id", PLATE, "--assembly-id", ASSEMBLY],
    );
    f.run(
        "create-level",
        &["--name", "Ground", "--elevation", "0", "--new-id", LEVEL],
    );
    f.run("update-element", &["--id", PLATE, "--level", LEVEL]);
    f.run(
        "create-weld",
        &[
            "--name",
            "Joint",
            "--main",
            PLATE,
            "--secondary",
            BEAM,
            "--size",
            "6",
            "--location",
            "workshop",
            "--side",
            "both",
            "--new-id",
            WELD,
        ],
    );
    f.run(
        "create-bolt",
        &[
            "--name",
            "Bolts",
            "--main",
            PLATE,
            "--secondary",
            BEAM,
            "--origin",
            "100,0,5",
            "--direction",
            "1,0,0",
            "--count",
            "2",
            "--pitch",
            "80",
            "--diameter",
            "16",
            "--new-id",
            BOLT,
        ],
    );
    let source = f.0.join("foreign.usda");
    fs::write(&source, "#usda 1.0\n(defaultPrim=\"Original\")\ndef Xform \"Original\" { def Scope \"Child\" {} }\n").unwrap();
    f.run(
        "attach-reference",
        &[
            "--source",
            source.to_str().unwrap(),
            "--name",
            "Context",
            "--new-id",
            REFERENCE,
        ],
    );

    let before = f.prims();
    for folder in [
        "/World/Steel/Parts",
        "/World/Steel/Assemblies",
        "/World/Steel/Connections",
        "/World/Levels",
        "/World/References",
    ] {
        assert_eq!(prim(&before, folder).kind.as_deref(), Some("group"));
    }
    for (id, prefix) in [
        (PLATE, "/World/Steel/Parts/PRT_"),
        (BEAM, "/World/Steel/Parts/PRT_"),
        (COLUMN, "/World/Steel/Parts/PRT_"),
        (ASSEMBLY, "/World/Steel/Assemblies/ASM_"),
        (LEVEL, "/World/Levels/LVL_"),
        (WELD, "/World/Steel/Connections/CON_"),
        (BOLT, "/World/Steel/Connections/CON_"),
        (REFERENCE, "/World/References/REF_"),
    ] {
        assert_eq!(
            prim(&before, &path(prefix, id)).element_id.as_deref(),
            Some(id)
        );
    }
    assert_eq!(
        targets(&before, &plate_path, "bim:partOf"),
        [path("/World/Steel/Assemblies/ASM_", ASSEMBLY)]
    );
    assert_eq!(
        targets(&before, &plate_path, "bim:level"),
        [path("/World/Levels/LVL_", LEVEL)]
    );
    assert_eq!(
        targets(&before, &plate_path, "bim:material"),
        std::slice::from_ref(&material_path)
    );
    let beam_path = path("/World/Steel/Parts/PRT_", BEAM);
    assert_eq!(targets(&before, &beam_path, "bim:profile"), [profile_path]);
    assert_eq!(
        targets(&before, &beam_path, "bim:material"),
        [material_path]
    );
    for id in [WELD, BOLT] {
        assert_eq!(
            targets(
                &before,
                &path("/World/Steel/Connections/CON_", id),
                "bim:connects"
            ),
            [plate_path.clone(), beam_path.clone()]
        );
    }
    let geometry = prim(&before, &format!("{plate_path}/Geometry")).clone();
    f.run(
        "update-element",
        &["--id", PLATE, "--name", "Renamed", "--level", "none"],
    );
    f.run("detach-from-assembly", &["--id", PLATE]);
    let after = f.prims();
    assert_eq!(
        prim(&after, &plate_path).display_name.as_deref(),
        Some("Renamed")
    );
    // Composition provenance changes on a rename; geometry values and the path do not.
    let after_geometry = prim(&after, &geometry.path);
    assert_eq!(after_geometry.properties, geometry.properties);
    assert!(targets(&after, &plate_path, "bim:partOf").is_empty());
    assert!(targets(&after, &plate_path, "bim:level").is_empty());
    let bundle = f.0.join("scene.bundle");
    f.run("export-view", &["--out", bundle.to_str().unwrap()]);
    let view =
        ViewSnapshot::from_bundle(&fs::read(bundle).unwrap(), &ViewLimits::default()).unwrap();
    for id in [PLATE, BEAM, COLUMN] {
        assert!(
            view.element(&beamcraft_model::id::ElementId::parse(id).unwrap())
                .is_some()
        );
    }
    // Deleting non-parts removes the leaf from its category folder as well.
    for id in [WELD, BOLT, ASSEMBLY, LEVEL] {
        f.run("delete-element", &["--id", id]);
    }
    let inspection = beamcraft_cli::plate::inspect(&f.store()).unwrap();
    assert_eq!(inspection.plates.len(), 1);
    assert_eq!(inspection.members.len(), 2);
}
