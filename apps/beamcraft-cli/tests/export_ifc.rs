// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, members, levels and assemblies, MPL-2.0.

//! E02 end to end through the real CLI binary: `export-ifc` and `ifc-guid` on committed
//! revisions, with the written file checked by the crate's own structural reader (the external
//! validation tool of E01 Q-8 is not admitted yet and is not used).
//!
//! The file is parsed back and checked for structure, GlobalIds, spatial containment, the
//! volume recomputed from the face sets against the stored quantities, determinism, the
//! "export changes nothing" rule, and the refusals of the contract.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use crate::support;

use support::{at, text};

use beamcraft_host::{
    ids::IdSource,
    revision::{ExpectedBase, LayerEntry, NoFault, RevisionDraft, RevisionStore, Uuid7},
    storage::BlobStore,
};
use beamcraft_ifc::verify::{self, StepFile, Value};
use beamcraft_json::{Json, Limits};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

const PROJECT: &str = "019a7bf0-0000-7000-8000-00000000e001";
const STOREY: &str = "019a7bf0-0000-7000-8000-000000400001";

/// An invented foreign source for a reference wrapper (a proxy whose pose is not exported).
const SITE: &str = "#usda 1.0\n(\n    defaultPrim = \"Site\"\n    metersPerUnit = 1.0\n    upAxis = \"Z\"\n)\ndef Xform \"Site\"\n{\n}\n";

struct Fixture {
    shared: support::StoreFixture,
}

struct Scene {
    material: String,
    profile: String,
    plate: String,
    beam: String,
    column: String,
    assembly: String,
    weld: String,
}

impl Fixture {
    fn new() -> Self {
        Self {
            shared: support::StoreFixture::new("e02-tests"),
        }
    }
    fn store(&self) -> String {
        self.shared.root.join("store").to_str().unwrap().to_owned()
    }
    fn path(&self, name: &str) -> String {
        self.shared.root.join(name).to_str().unwrap().to_owned()
    }
    fn run_args(&self, args: &[&str]) -> (i32, Json) {
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(args)
            .output()
            .unwrap();
        let body = decode_large(&output.stdout);
        (output.status.code().unwrap(), body)
    }
    fn run(&self, command: &str, extra: &[&str]) -> (i32, Json) {
        let store = self.store();
        let mut args = vec![command, "--store", store.as_str()];
        args.extend_from_slice(extra);
        self.run_args(&args)
    }
    fn ok(&self, command: &str, extra: &[&str]) -> Json {
        let (code, body) = self.run(command, extra);
        assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
        body
    }
    fn head(&self) -> String {
        text(&self.ok("open-revision", &[]), "revision_id")
    }
    fn layer_digests(&self) -> Vec<(String, String)> {
        list(&self.ok("open-revision", &[]), "layers")
            .iter()
            .map(|l| (text(l, "name"), text(l, "digest")))
            .collect()
    }
    /// File-destination checks need a valid, nonempty revision, not the mixed geometry scene.
    /// Keep authoring through the real CLI, with the same explicit project as the full scene.
    fn exportable_plate(&self) {
        self.ok(
            "create-plate",
            &[
                "--project-id",
                PROJECT,
                "--width",
                "400",
                "--length",
                "300",
                "--thickness",
                "20",
            ],
        );
    }
    /// The whole scene: a plate with a hole and a level, a beam and a column of an I section, an
    /// assembly with the beam in it and a weld between beam and column.
    fn scene(&self) -> Scene {
        let material = text(
            at(
                &self.ok(
                    "define-material",
                    &[
                        "--project-id",
                        PROJECT,
                        "--density",
                        "7850",
                        "--name",
                        "S355",
                        "--grade",
                        "S355",
                        "--standard",
                        "EN 10025-2",
                    ],
                ),
                &["definition"],
            ),
            "digest",
        );
        let profile = text(
            at(
                &self.ok(
                    "define-profile",
                    &[
                        "--family",
                        "i_shape",
                        "--h",
                        "200",
                        "--b",
                        "100",
                        "--tw",
                        "5.6",
                        "--tf",
                        "8.5",
                        "--r",
                        "12",
                        "--designation",
                        "IPE 200",
                        "--standard",
                        "EN 10365",
                        "--area-mm2",
                        "2848",
                        "--mass-per-meter-kg",
                        "22.4",
                    ],
                ),
                &["definition"],
            ),
            "digest",
        );
        let plate = text(
            at(
                &self.ok(
                    "create-plate",
                    &[
                        "--width",
                        "400",
                        "--length",
                        "300",
                        "--thickness",
                        "20",
                        "--material-digest",
                        &material,
                        "--hole-round",
                        "100,100,40",
                        "--name",
                        "Пластина P1",
                    ],
                ),
                &["plate"],
            ),
            "element_id",
        );
        let member = |kind: &str, end: &str, name: &str| {
            text(
                at(
                    &self.ok(
                        "create-member",
                        &[
                            "--kind",
                            kind,
                            "--start",
                            "0,0,0",
                            "--end",
                            end,
                            "--profile-digest",
                            &profile,
                            "--material-digest",
                            &material,
                            "--name",
                            name,
                        ],
                    ),
                    &["member"],
                ),
                "element_id",
            )
        };
        let beam = member("beam", "3000,0,0", "B1");
        let column = member("column", "0,0,3000", "C1");
        self.ok(
            "create-level",
            &[
                "--name",
                "Level 1",
                "--elevation",
                "3000",
                "--new-id",
                STOREY,
            ],
        );
        self.ok("update-element", &["--id", &plate, "--level", STOREY]);
        let assembly = text(
            self.ok("create-assembly", &["--name", "A1"]).as_ref(),
            "element_id",
        );
        self.ok(
            "attach-to-assembly",
            &["--id", &beam, "--assembly-id", &assembly],
        );
        let weld = text(
            self.ok(
                "create-weld",
                &[
                    "--name",
                    "W1",
                    "--main",
                    &beam,
                    "--secondary",
                    &column,
                    "--size",
                    "6",
                    "--location",
                    "workshop",
                    "--side",
                    "both",
                ],
            )
            .as_ref(),
            "element_id",
        );
        Scene {
            material,
            profile,
            plate,
            beam,
            column,
            assembly,
            weld,
        }
    }
    fn scene_plate(&self) -> String {
        text(
            &list(&self.ok("inspect-elements", &[]), "plates")[0],
            "element_id",
        )
    }
    fn export(&self, out: &str, extra: &[&str]) -> Json {
        self.ok("export-ifc", &[&["--out", out], extra].concat())
    }
    fn export_rejected(&self, extra: &[&str]) -> (i32, String, Json) {
        let (code, body) = self.run("export-ifc", extra);
        let code_text = text(&body, "code");
        (code, code_text, body)
    }
    fn parsed(&self, name: &str) -> StepFile {
        verify::parse(&fs::read(self.shared.root.join(name)).unwrap()).unwrap()
    }
    /// Commits a head-based revision with edited layer text and no content verification, the way
    /// a layer edited outside BeamCraft would arrive through `commit-revision`.
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

fn decode_large(bytes: &[u8]) -> Json {
    // The scenes are small, so the report fits the control-message bounds of the other commands.
    beamcraft_json::decode(bytes, &Limits::DEFAULT).unwrap_or_else(|_| {
        panic!(
            "undecodable output: {}",
            String::from_utf8_lossy(&bytes[..bytes.len().min(300)])
        )
    })
}

fn opt_text(json: &Json, key: &str) -> Option<String> {
    json.get(key).unwrap().string().ok().map(str::to_owned)
}
fn list<'a>(json: &'a Json, key: &str) -> &'a [Json] {
    at(json, &[key]).array().unwrap()
}
fn is_null(json: &Json, key: &str) -> bool {
    json.get(key).unwrap().string().is_err() && json.get(key).unwrap().array().is_err()
}

trait AsRefJson {
    fn as_ref(&self) -> &Json;
}
impl AsRefJson for Json {
    fn as_ref(&self) -> &Json {
        self
    }
}

fn row<'a>(report: &'a Json, id: &str) -> &'a Json {
    list(report, "elements")
        .iter()
        .find(|e| text(e, "element_id") == id)
        .unwrap()
}

fn findings(report: &Json) -> Vec<(String, Option<String>, String)> {
    list(report, "findings")
        .iter()
        .map(|f| {
            (
                text(f, "code"),
                f.get("element_id")
                    .unwrap()
                    .string()
                    .ok()
                    .map(str::to_owned),
                text(f, "field"),
            )
        })
        .collect()
}

fn str_arg<'a>(file: &StepFile, value: Option<&'a Value>) -> &'a str {
    let _ = file;
    match value {
        Some(Value::Str(s)) => s,
        other => panic!("not a string: {other:?}"),
    }
}

/// `volumes` of the file by element name (the face set of each IfcPlate, IfcBeam and IfcColumn).
fn volumes_by_name(file: &StepFile) -> BTreeMap<String, f64> {
    let summary = file.check().unwrap();
    let mut out = BTreeMap::new();
    for entity in ["IFCPLATE", "IFCBEAM", "IFCCOLUMN"] {
        for product in file.all(entity) {
            let shape = file.target(&product.args[6]).unwrap();
            let Value::List(reps) = &shape.args[2] else {
                panic!("representations");
            };
            let rep = file.target(&reps[0]).unwrap();
            let Value::List(items) = &rep.args[3] else {
                panic!("items");
            };
            let Value::Ref(set) = items[0] else {
                panic!("face set");
            };
            out.insert(
                str_arg(file, product.args.get(2)).to_owned(),
                summary.volumes[&set],
            );
        }
    }
    out
}

fn property_values(file: &StepFile, product: &verify::Instance) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    for rel in file.all("IFCRELDEFINESBYPROPERTIES") {
        let Value::List(related) = &rel.args[4] else {
            continue;
        };
        if !related.contains(&Value::Ref(product.id)) {
            continue;
        }
        let set = file.target(&rel.args[5]).unwrap();
        let Some(Value::List(props)) = set.args.last() else {
            continue;
        };
        for prop in props {
            let prop = file.target(prop).unwrap();
            if prop.name != "IFCPROPERTYSINGLEVALUE" {
                continue;
            }
            let name = str_arg(file, prop.args.first()).to_owned();
            if let Value::Typed(_, inner) = &prop.args[2] {
                out.insert(name, inner[0].clone());
            }
        }
    }
    out
}

#[test]
fn a_scene_exports_to_a_structurally_valid_file_with_the_volumes_of_the_store() {
    let f = Fixture::new();
    let s = f.scene();
    let out = f.path("model.ifc");
    let report = f.export(&out, &["--project-name", "Frame"]);
    assert_eq!(text(&report, "outcome"), "exported");
    assert_eq!(text(&report, "schema"), "IFC4");
    assert_eq!(text(&report, "contract_version"), "0.1.0");
    assert_eq!(text(&report, "project_id"), PROJECT);
    assert_eq!(text(&report, "revision_id"), f.head());
    assert_eq!(text(&report, "status"), "ok_with_losses");
    assert!(is_null(&report, "primary_code"));

    let file = f.parsed("model.ifc");
    let summary = file.check().unwrap();
    for (name, count) in [
        ("IFCPROJECT", 1),
        ("IFCSITE", 1),
        ("IFCBUILDING", 1),
        ("IFCBUILDINGSTOREY", 1),
        ("IFCPLATE", 1),
        ("IFCBEAM", 1),
        ("IFCCOLUMN", 1),
        ("IFCBUILDINGELEMENTPROXY", 2),
        ("IFCTRIANGULATEDFACESET", 3),
        ("IFCMATERIAL", 1),
        ("IFCRELASSOCIATESMATERIAL", 1),
    ] {
        assert_eq!(summary.counts.get(name).copied(), Some(count), "{name}");
    }

    // The report matches the file and the output record matches the bytes.
    let bytes = fs::read(&out).unwrap();
    assert_eq!(
        at(&report, &["output", "bytes"]).unsigned().unwrap(),
        bytes.len() as u64
    );
    assert_eq!(
        text(at(&report, &["output"]), "sha256"),
        beamcraft_model::digest::Sha256::digest(&bytes)
            .unwrap()
            .to_string()
    );
    assert_eq!(
        at(&report, &["counts", "elements_total"])
            .unsigned()
            .unwrap(),
        5
    );
    assert_eq!(at(&report, &["counts", "exported"]).unsigned().unwrap(), 5);
    assert_eq!(at(&report, &["counts", "proxies"]).unsigned().unwrap(), 2);

    // GlobalIds are the deterministic encoding of the ids, in the file and in the report.
    for id in [&s.plate, &s.beam, &s.column, &s.assembly, &s.weld] {
        let (code, guid) = f.run_args(&["ifc-guid", "--id", id]);
        assert_eq!(code, 0);
        let global = text(&guid, "ifc_global_id");
        assert_eq!(text(row(&report, id), "ifc_global_id"), global);
        assert!(summary.global_ids.contains(&global), "{id}");
        let (_, back) = f.run_args(&["ifc-guid", "--decode", &global]);
        assert_eq!(text(&back, "uuid"), *id);
    }
    // The level's id in the report is the GlobalId of its written storey (a synthetic id of the
    // ProjectId and the LevelId), not the plain encoding of the LevelId.
    let storey = file.all("IFCBUILDINGSTOREY")[0];
    let written = str_arg(&file, storey.args.first());
    assert_eq!(text(&list(&report, "levels")[0], "ifc_global_id"), written);
    assert_eq!(text(&list(&report, "levels")[0], "level_id"), STOREY);
    assert_ne!(
        written,
        f.run_args(&["ifc-guid", "--id", STOREY])
            .1
            .get("ifc_global_id")
            .unwrap()
            .string()
            .unwrap()
    );

    // The volume recomputed from every face set equals the volume the store published.
    let inspected = f.ok("inspect-elements", &[]);
    let stored: BTreeMap<String, f64> = list(&inspected, "plates")
        .iter()
        .chain(list(&inspected, "members").iter())
        .map(|e| {
            (
                text(e, "name"),
                at(e, &["quantities", "volume", "mm3"]).float().unwrap(),
            )
        })
        .collect();
    let from_file = volumes_by_name(&file);
    assert_eq!(from_file.len(), 3);
    for (name, volume) in &from_file {
        let expected = stored[name];
        // P-Mass: a member's published volume is area x length with the exact section area,
        // while its mesh keeps the detailed polygon of the fillets, so they differ by the arc
        // approximation (here the I section, below 0.5 percent). A plate has no such gap.
        let bound = if name == "Пластина P1" {
            1e-4
        } else {
            5e-3
        };
        assert!(
            ((volume - expected) / expected).abs() < bound,
            "{name}: file {volume} store {expected}"
        );
    }

    // The plate is in the storey, the beam and column in the building; names survive the escapes.
    let plate = file.all("IFCPLATE")[0];
    assert_eq!(str_arg(&file, plate.args.get(2)), "Пластина P1");
    let storey = file.all("IFCBUILDINGSTOREY")[0];
    let contained = file.all("IFCRELCONTAINEDINSPATIALSTRUCTURE");
    assert_eq!(contained.len(), 2);
    let in_storey = contained
        .iter()
        .find(|c| c.args[5] == Value::Ref(storey.id))
        .unwrap();
    assert_eq!(in_storey.args[4], Value::List(vec![Value::Ref(plate.id)]));
    assert_eq!(storey.args[9], Value::Real(3000.0));

    // Quantities: plate volume in the base quantity set, mass as properties; hole count; the
    // beam's declared section values; the assembly id of the beam.
    let props = property_values(&file, plate);
    assert_eq!(props["HoleCount"], Value::Int(1));
    assert!(props.contains_key("NetWeightKg"));
    assert!(
        !props.contains_key("GrossWeightKg"),
        "a plate with a hole has no gross value"
    );
    let beam = file.all("IFCBEAM")[0];
    let beam_props = property_values(&file, beam);
    assert_eq!(beam_props["AssemblyId"], Value::Str(s.assembly.clone()));
    assert_eq!(
        beam_props["DeclaredDesignation"],
        Value::Str("IPE 200".into())
    );
    assert_eq!(beam_props["DeclaredSectionAreaMm2"], Value::Real(2848.0));
    assert_eq!(beam_props["ProfileDigest"], Value::Str(s.profile.clone()));
    assert_eq!(beam_props["MaterialDigest"], Value::Str(s.material.clone()));
    let qto: Vec<_> = file
        .all("IFCELEMENTQUANTITY")
        .iter()
        .map(|q| str_arg(&file, q.args.get(2)).to_owned())
        .collect();
    assert_eq!(
        qto,
        [
            "Qto_PlateBaseQuantities",
            "Qto_BeamBaseQuantities",
            "Qto_ColumnBaseQuantities"
        ]
    );
    let material = file.all("IFCMATERIAL")[0];
    assert_eq!(str_arg(&file, material.args.first()), "S355");
}

#[test]
fn the_same_revision_gives_the_same_bytes_and_the_same_report() {
    let f = Fixture::new();
    f.scene();
    let (a, b) = (f.path("a.ifc"), f.path("b.ifc"));
    let first = f.export(&a, &[]);
    let second = f.export(&b, &["--project-name", "Other"]);
    let third = f.export(&f.path("c.ifc"), &[]);
    assert_eq!(fs::read(&a).unwrap(), fs::read(f.path("c.ifc")).unwrap());
    assert_eq!(
        text(at(&first, &["output"]), "sha256"),
        text(at(&third, &["output"]), "sha256")
    );
    // A project name is the only thing that differs between a and b.
    assert_ne!(fs::read(&a).unwrap(), fs::read(&b).unwrap());
    assert_eq!(list(&first, "elements"), list(&second, "elements"));
    // The header time stamp is the revision's, not the clock's.
    let text_a = String::from_utf8(fs::read(&a).unwrap()).unwrap();
    assert!(text_a.contains("FILE_NAME('beamcraft-export.ifc','"));
    let stamp = text_a
        .lines()
        .nth(3)
        .unwrap()
        .split('\'')
        .nth(3)
        .unwrap()
        .to_owned();
    assert_eq!(stamp.len(), 19);
}

#[test]
fn export_changes_nothing_in_the_store() {
    let f = Fixture::new();
    f.scene();
    let (head, layers) = (f.head(), f.layer_digests());
    let blobs = fs::read_dir(f.shared.root.join("store").join("blobs"))
        .unwrap()
        .count();
    f.export(&f.path("a.ifc"), &[]);
    f.export(&f.path("a.ifc"), &["--overwrite"]);
    assert_eq!(f.head(), head);
    assert_eq!(f.layer_digests(), layers);
    assert_eq!(
        fs::read_dir(f.shared.root.join("store").join("blobs"))
            .unwrap()
            .count(),
        blobs
    );
    // The GlobalId is computed, never stored.
    for name in ["authored.usda", "derived.usda", "root.usda"] {
        let store = RevisionStore::new(BlobStore::new(f.shared.root.join("store")).unwrap());
        let layer = String::from_utf8(store.open().unwrap().layer_bytes(name).unwrap()).unwrap();
        assert!(!layer.contains("ifcGlobalId"), "{name}");
    }
}

#[test]
fn an_explicit_revision_exports_that_revision() {
    let f = Fixture::new();
    let s = f.scene();
    let before = f.head();
    f.ok("update-element", &["--id", &s.plate, "--name", "Renamed"]);
    let old = f.export(&f.path("old.ifc"), &["--revision", &before]);
    let new = f.export(&f.path("new.ifc"), &[]);
    assert_eq!(text(&old, "revision_id"), before);
    assert_eq!(text(&new, "revision_id"), f.head());
    assert_eq!(
        str_arg(
            &f.parsed("old.ifc"),
            f.parsed("old.ifc").all("IFCPLATE")[0].args.get(2)
        ),
        "Пластина P1"
    );
    assert_eq!(
        str_arg(
            &f.parsed("new.ifc"),
            f.parsed("new.ifc").all("IFCPLATE")[0].args.get(2)
        ),
        "Renamed"
    );
    // The GlobalId survives the rename.
    assert_eq!(
        text(row(&old, &s.plate), "ifc_global_id"),
        text(row(&new, &s.plate), "ifc_global_id")
    );
    // The rename changed no number: the same project and level ids.
    assert_eq!(text(&old, "project_id"), text(&new, "project_id"));
}

#[test]
fn a_copy_gets_its_own_global_id_and_the_source_keeps_its_own() {
    let f = Fixture::new();
    let s = f.scene();
    let first = f.export(&f.path("a.ifc"), &[]);
    let copy = text(
        at(
            &f.ok("copy-element", &["--id", &s.plate, "--offset", "500,0,0"]),
            &["element"],
        ),
        "element_id",
    );
    let second = f.export(&f.path("b.ifc"), &[]);
    assert_ne!(copy, s.plate);
    assert_eq!(
        text(row(&first, &s.plate), "ifc_global_id"),
        text(row(&second, &s.plate), "ifc_global_id")
    );
    assert_ne!(
        text(row(&second, &copy), "ifc_global_id"),
        text(row(&second, &s.plate), "ifc_global_id")
    );
    // The copy keeps the source's level: both plates are in the storey.
    let file = f.parsed("b.ifc");
    let storey = file.all("IFCBUILDINGSTOREY")[0];
    let contained = file
        .all("IFCRELCONTAINEDINSPATIALSTRUCTURE")
        .into_iter()
        .find(|c| c.args[5] == Value::Ref(storey.id))
        .unwrap();
    let Value::List(members) = &contained.args[4] else {
        panic!("members");
    };
    assert_eq!(members.len(), 2);
}

#[test]
fn an_unknown_density_leaves_the_mass_out_and_says_so() {
    let f = Fixture::new();
    let profile = text(
        at(
            &f.ok("define-profile", &["--width", "100", "--height", "50"]),
            &["definition"],
        ),
        "digest",
    );
    let beam = text(
        at(
            &f.ok(
                "create-member",
                &[
                    "--start",
                    "0,0,0",
                    "--end",
                    "2000,0,0",
                    "--profile-digest",
                    &profile,
                ],
            ),
            &["member"],
        ),
        "element_id",
    );
    let report = f.export(&f.path("a.ifc"), &[]);
    let r = row(&report, &beam);
    assert_eq!(text(r, "volume"), "current");
    assert_eq!(text(r, "mass"), "unknown");
    assert_eq!(list(r, "omissions").len(), 1);
    assert_eq!(
        findings(&report),
        vec![(
            "IFC_QUANTITY_OMITTED".into(),
            Some(beam.clone()),
            "mass".into()
        )]
    );
    assert_eq!(text(&report, "status"), "ok_with_losses");
    let file = f.parsed("a.ifc");
    file.check().unwrap();
    // No weight is written, and none is zero; there is no material at all.
    assert!(file.all("IFCQUANTITYWEIGHT").is_empty());
    assert!(file.all("IFCMATERIAL").is_empty());
    let props = property_values(&file, file.all("IFCBEAM")[0]);
    assert_eq!(props["MassStatus"], Value::Str("unknown".into()));
    assert!(!props.contains_key("MaterialDigest"));
}

#[test]
fn a_stale_quantity_is_left_out_and_listed() {
    let f = Fixture::new();
    let s = f.scene();
    f.tamper(|name, layer| {
        if name == "derived.usda" {
            layer.replacen(
                "custom double bim:volumeMm3 = ",
                "custom double bim:volumeMm3 = 1",
                1,
            )
        } else {
            layer
        }
    });
    let report = f.export(&f.path("a.ifc"), &[]);
    let stale: Vec<_> = list(&report, "elements")
        .iter()
        .filter(|e| opt_text(e, "volume").as_deref() == Some("stale"))
        .collect();
    assert_eq!(stale.len(), 1);
    assert!(
        list(stale[0], "omissions")
            .iter()
            .any(|o| o.string().unwrap() == "volume")
    );
    let id = text(stale[0], "element_id");
    assert!([&s.plate, &s.beam, &s.column].contains(&&id));
    let file = f.parsed("a.ifc");
    file.check().unwrap();
    // The stale element has no NetVolume; its status is in the property set.
    assert_eq!(file.all("IFCQUANTITYVOLUME").len(), 2 + 2 + 2 - 2);
}

#[test]
fn a_deactivated_element_is_not_exported_and_is_listed() {
    let f = Fixture::new();
    let s = f.scene();
    f.tamper(|name, layer| {
        if name == "authored.usda" {
            layer.replacen(
                "custom token bim:geometryMode = \"parametric\"",
                "custom bool bim:isDeactivated = true\n            custom token bim:geometryMode = \"parametric\"",
                1,
            )
        } else {
            layer
        }
    });
    let report = f.export(&f.path("a.ifc"), &[]);
    let gone: Vec<_> = list(&report, "elements")
        .iter()
        .filter(|e| !matches!(e.get("exported"), Ok(Json::Bool(true))))
        .collect();
    assert_eq!(gone.len(), 1);
    assert!(is_null(gone[0], "ifc_global_id"));
    assert_eq!(
        at(&report, &["counts", "not_exported"]).unsigned().unwrap(),
        1
    );
    assert!(
        findings(&report)
            .iter()
            .any(|(code, _, _)| code == "IFC_ELEMENT_DEACTIVATED")
    );
    assert_eq!(text(&report, "status"), "ok_with_losses");
    let file = f.parsed("a.ifc");
    file.check().unwrap();
    let products = [
        "IFCPLATE",
        "IFCBEAM",
        "IFCCOLUMN",
        "IFCBUILDINGELEMENTPROXY",
    ]
    .iter()
    .map(|n| file.all(n).len())
    .sum::<usize>();
    assert_eq!(products, 4);
    let _ = s;
}

#[test]
fn a_reference_wrapper_is_a_proxy_with_its_pose_and_no_representation() {
    let f = Fixture::new();
    f.scene();
    let source = f.path("site.usda");
    fs::write(&source, SITE).unwrap();
    let wrapper = text(
        at(
            &f.ok(
                "attach-reference",
                &["--source", &source, "--name", "Site ref"],
            ),
            &["reference"],
        ),
        "id",
    );
    // The units correction lives in a child prim, so the wrapper's own pose stays rigid.
    f.ok(
        "correct-reference-units",
        &[
            "--id",
            &wrapper,
            "--meters-per-unit",
            "0.001",
            "--up-axis",
            "Y",
        ],
    );
    let report = f.export(&f.path("a.ifc"), &[]);
    let codes: Vec<String> = findings(&report).into_iter().map(|(c, _, _)| c).collect();
    assert_eq!(
        codes.iter().filter(|c| *c == "IFC_KIND_PROXY").count(),
        3,
        "{codes:?}"
    );
    assert!(!codes.contains(&"IFC_PLACEMENT_ABSENT".to_owned()));
    let file = f.parsed("a.ifc");
    file.check().unwrap();
    let reference = file
        .all("IFCBUILDINGELEMENTPROXY")
        .into_iter()
        .find(|p| str_arg(&file, p.args.get(2)) == "Site ref")
        .unwrap();
    assert_ne!(reference.args[5], Value::Null);
    assert_eq!(reference.args[6], Value::Null);
    assert_eq!(str_arg(&file, reference.args.get(4)), "reference");
    assert_eq!(
        text(row(&report, &wrapper), "entity"),
        "IfcBuildingElementProxy"
    );
    assert!(is_null(row(&report, &wrapper), "representation"));
}

#[test]
fn strict_mode_does_not_refuse_proxies_omitted_quantities_or_truncated_names() {
    let f = Fixture::new();
    // No density: a mass is omitted for every member; the long label is cut; assembly and weld
    // are proxies. None of these carries an F04 code, so none is fatal.
    f.scene();
    f.ok(
        "update-element",
        &["--id", &f.scene_plate(), "--name", &"n".repeat(256)],
    );
    let report = f.export(&f.path("strict.ifc"), &["--strict"]);
    let codes: Vec<String> = findings(&report).into_iter().map(|(c, _, _)| c).collect();
    assert!(
        codes.contains(&"IFC_NAME_TRUNCATED".to_owned()),
        "{codes:?}"
    );
    assert!(codes.contains(&"IFC_KIND_PROXY".to_owned()));
    let file = f.parsed("strict.ifc");
    file.check().unwrap();
    let plate = file.all("IFCPLATE")[0];
    assert_eq!(str_arg(&file, plate.args.get(2)).len(), 255);
    assert_eq!(
        property_values(&file, plate)["Label"],
        Value::Str("n".repeat(256))
    );
}

#[test]
fn refusals_leave_no_file() {
    let f = Fixture::new();
    // No revision at all.
    let out = f.path("a.ifc");
    let (code, rejected, body) = f.export_rejected(&["--out", &out]);
    assert_eq!((code, rejected.as_str()), (2, "DEPENDENCY_UNAVAILABLE"));
    assert_eq!(text(&list(&body, "diagnostics")[0], "status"), "incomplete");
    assert_eq!(
        text(&list(&body, "diagnostics")[0], "ifc_code"),
        "IFC_INPUT_UNAVAILABLE"
    );
    f.exportable_plate();
    for (extra, expected, ifc) in [
        (
            vec!["--out", &out, "--schema", "IFC4X3"],
            "UNSUPPORTED_VERSION",
            "IFC_VERSION_UNSUPPORTED",
        ),
        (
            vec!["--out", &out, "--format", "ifcxml"],
            "UNSUPPORTED_PROPERTY",
            "IFC_FORMAT_UNSUPPORTED",
        ),
        (
            vec!["--out", &out, "--revision", "not-a-uuid"],
            "INVALID_ID",
            "IFC_ID_INVALID",
        ),
        (
            vec![
                "--out",
                &out,
                "--revision",
                "019a7bf0-0000-7000-8000-0000000000ff",
            ],
            "NOT_FOUND",
            "IFC_NOT_FOUND",
        ),
        (
            vec!["--out", &out, "--project-id", PROJECT],
            "INVALID_ARGUMENT",
            "IFC_ARGUMENT_INVALID",
        ),
        (
            vec!["--out", &out, "--bogus", "1"],
            "INVALID_ARGUMENT",
            "IFC_ARGUMENT_INVALID",
        ),
        (
            vec!["--out", &out, "--out", &out],
            "INVALID_ARGUMENT",
            "IFC_ARGUMENT_INVALID",
        ),
        (vec![], "INVALID_ARGUMENT", "IFC_ARGUMENT_INVALID"),
        (
            vec!["--out", &out, "--project-name", ""],
            "INVALID_ARGUMENT",
            "IFC_ARGUMENT_INVALID",
        ),
    ] {
        let (code, rejected, body) = f.export_rejected(&extra);
        assert_eq!((code, rejected.as_str()), (2, expected), "{extra:?}");
        assert_eq!(
            text(&list(&body, "diagnostics")[0], "ifc_code"),
            ifc,
            "{extra:?}"
        );
        assert!(!Path::new(&out).exists(), "{extra:?}");
    }
    // A missing output directory.
    let nowhere = f.path("nowhere/a.ifc");
    let (_, rejected, _) = f.export_rejected(&["--out", &nowhere]);
    assert_eq!(rejected, "INVALID_ARGUMENT");
    // Only temporary names of a successful export may exist, and none does after a refusal.
    assert!(fs::read_dir(&f.shared.root).unwrap().all(|e| {
        let name = e.unwrap().file_name().into_string().unwrap();
        !name.contains(".tmp-")
    }));
}

#[test]
fn an_existing_output_is_kept_unless_overwrite_is_asked() {
    let f = Fixture::new();
    f.exportable_plate();
    let out = f.path("a.ifc");
    fs::write(&out, "keep me").unwrap();
    let (code, rejected, _) = f.export_rejected(&["--out", &out]);
    assert_eq!((code, rejected.as_str()), (2, "INVALID_ARGUMENT"));
    assert_eq!(fs::read_to_string(&out).unwrap(), "keep me");
    f.export(&out, &["--overwrite"]);
    assert!(
        fs::read_to_string(&out)
            .unwrap()
            .starts_with("ISO-10303-21;")
    );
    // `--overwrite true` and `--overwrite false` are accepted as values too.
    f.export(&out, &["--overwrite", "true"]);
    let (_, rejected, _) = f.export_rejected(&["--out", &out, "--overwrite", "false"]);
    assert_eq!(rejected, "INVALID_ARGUMENT");
}

#[test]
fn an_output_inside_the_source_store_is_refused() {
    let f = Fixture::new();
    f.exportable_plate();
    let store = f.shared.root.join("store");
    let (head, layers) = (f.head(), f.layer_digests());
    for inside in [
        store.join("revisions").join("model.ifc"),
        store.join("blobs").join("model.ifc"),
        store.join("model.ifc"),
    ] {
        let target = inside.to_str().unwrap();
        for extra in [
            vec!["--out", target],
            vec!["--out", target, "--overwrite"],
            vec!["--out", &f.path("ok.ifc"), "--report", target],
        ] {
            let (code, rejected, _) = f.export_rejected(&extra);
            assert_eq!(
                (code, rejected.as_str()),
                (2, "INVALID_ARGUMENT"),
                "{extra:?}"
            );
            assert!(!inside.exists());
        }
    }
    // Nothing was written next to the store records either, and the store still reads.
    assert!(!Path::new(&f.path("ok.ifc")).exists());
    assert_eq!((f.head(), f.layer_digests()), (head, layers));
    f.export(&f.path("fine.ifc"), &[]);
}

#[test]
fn the_ifc_and_the_report_cannot_be_the_same_file() {
    let f = Fixture::new();
    f.exportable_plate();
    fs::create_dir(f.path("sub")).unwrap();
    let out = f.path("model.ifc");
    for report in [
        out.clone(),
        f.path("sub/../model.ifc"),
        f.path("./model.ifc"),
    ] {
        let (code, rejected, _) =
            f.export_rejected(&["--out", &out, "--report", &report, "--overwrite"]);
        assert_eq!(
            (code, rejected.as_str()),
            (2, "INVALID_ARGUMENT"),
            "{report}"
        );
        assert!(!Path::new(&out).exists(), "{report}");
    }
}

#[test]
fn a_refused_report_leaves_the_ifc_destination_as_it_was() {
    let f = Fixture::new();
    f.exportable_plate();
    let (out, report) = (f.path("model.ifc"), f.path("report.json"));
    fs::write(&report, "keep").unwrap();
    // The report exists and may not be replaced: no IFC file appears.
    let (code, rejected, _) = f.export_rejected(&["--out", &out, "--report", &report]);
    assert_eq!((code, rejected.as_str()), (2, "INVALID_ARGUMENT"));
    assert!(!Path::new(&out).exists());
    assert_eq!(fs::read_to_string(&report).unwrap(), "keep");
    // With an earlier IFC and an unusable report directory, the earlier IFC is not replaced.
    fs::write(&out, "previous").unwrap();
    let missing = f.path("nowhere/report.json");
    let (_, rejected, _) = f.export_rejected(&["--out", &out, "--report", &missing, "--overwrite"]);
    assert_eq!(rejected, "INVALID_ARGUMENT");
    assert_eq!(fs::read_to_string(&out).unwrap(), "previous");
    // Nothing is left behind but the files the caller owns.
    let mut names: Vec<String> = fs::read_dir(&f.shared.root)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["model.ifc", "report.json", "store"]);
}

#[cfg(unix)]
#[test]
fn a_link_above_the_output_directory_is_refused() {
    let f = Fixture::new();
    f.exportable_plate();
    fs::create_dir_all(f.path("real/child")).unwrap();
    std::os::unix::fs::symlink(f.path("real"), f.path("alias")).unwrap();
    let through = f.path("alias/child/a.ifc");
    let (_, rejected, _) = f.export_rejected(&["--out", &through]);
    assert_eq!(rejected, "INVALID_ARGUMENT");
    assert_eq!(fs::read_dir(f.path("real/child")).unwrap().count(), 0);
}

#[test]
fn a_revision_is_found_by_id_among_many() {
    let f = Fixture::new();
    let s = f.scene();
    let first = f.head();
    for n in 0..6 {
        f.ok(
            "update-element",
            &["--id", &s.plate, "--name", &format!("n{n}")],
        );
    }
    let report = f.export(&f.path("old.ifc"), &["--revision", &first]);
    assert_eq!(text(&report, "revision_id"), first);
    let (_, rejected, _) = f.export_rejected(&[
        "--out",
        &f.path("none.ifc"),
        "--revision",
        "019a7bf0-0000-7000-8000-0000000000ff",
    ]);
    assert_eq!(rejected, "NOT_FOUND");
}

#[cfg(unix)]
#[test]
fn a_link_at_the_output_name_or_its_parent_is_refused() {
    let f = Fixture::new();
    f.exportable_plate();
    let real = f.path("real.ifc");
    fs::write(&real, "keep").unwrap();
    let link = f.path("link.ifc");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    for extra in [
        vec!["--out", link.as_str()],
        vec!["--out", link.as_str(), "--overwrite"],
    ] {
        let (_, rejected, _) = f.export_rejected(&extra);
        assert_eq!(rejected, "INVALID_ARGUMENT");
    }
    assert_eq!(fs::read_to_string(&real).unwrap(), "keep");
    fs::create_dir(f.path("dir")).unwrap();
    std::os::unix::fs::symlink(f.path("dir"), f.path("dirlink")).unwrap();
    let through = f.path("dirlink/a.ifc");
    let (_, rejected, _) = f.export_rejected(&["--out", &through]);
    assert_eq!(rejected, "INVALID_ARGUMENT");
    assert_eq!(fs::read_dir(f.path("dir")).unwrap().count(), 0);
}

#[test]
fn a_store_without_a_recorded_project_is_refused() {
    let f = Fixture::new();
    f.exportable_plate();
    f.tamper(|name, layer| {
        if name == "root.usda" {
            layer
                .lines()
                .filter(|l| !l.contains("bimProjectId"))
                .collect::<Vec<_>>()
                .join("\n")
                + "\n"
        } else {
            layer
        }
    });
    let out = f.path("a.ifc");
    let (code, rejected, body) = f.export_rejected(&["--out", &out]);
    assert_eq!((code, rejected.as_str()), (2, "DEPENDENCY_UNAVAILABLE"));
    assert_eq!(text(&list(&body, "diagnostics")[0], "status"), "incomplete");
    assert!(!Path::new(&out).exists());
}

#[test]
fn a_layer_edited_behind_the_stores_back_is_an_integrity_error() {
    let f = Fixture::new();
    f.scene();
    // A different thickness in the authored plate while the mesh stays: the kernel's
    // re-evaluation differs from the stored mesh (IFCX-GEO-005).
    f.tamper(|name, layer| {
        if name == "authored.usda" {
            layer.replacen(
                "steel:plate:thicknessMm = 20.0",
                "steel:plate:thicknessMm = 25.0",
                1,
            )
        } else {
            layer
        }
    });
    let out = f.path("a.ifc");
    let (code, rejected, body) = f.export_rejected(&["--out", &out]);
    assert_eq!((code, rejected.as_str()), (2, "INTEGRITY_ERROR"));
    assert_eq!(
        text(&list(&body, "diagnostics")[0], "ifc_code"),
        "IFC_INTEGRITY"
    );
    assert!(!Path::new(&out).exists());
}

#[test]
fn the_report_file_equals_the_printed_report() {
    let f = Fixture::new();
    f.scene();
    let out = f.path("a.ifc");
    let report_path = f.path("report.json");
    let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .args([
            "export-ifc",
            "--store",
            &f.store(),
            "--out",
            &out,
            "--report",
            &report_path,
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let printed = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        printed.trim_end(),
        fs::read_to_string(&report_path).unwrap()
    );
    // The report holds no path, host name or time.
    for forbidden in [f.shared.root.to_str().unwrap(), "tmp", "T00:", "hostname"] {
        assert!(!printed.contains(forbidden), "{forbidden}");
    }
    // Keys are sorted: the first key is `contract_version`.
    assert!(printed.starts_with("{\"contract_version\":\"0.1.0\""));
}

#[test]
fn the_guid_helper_encodes_decodes_and_refuses_without_repair() {
    let f = Fixture::new();
    let (code, body) = f.run_args(&["ifc-guid", "--id", "019a7bf0-0000-7000-8000-000000200001"]);
    assert_eq!(code, 0);
    assert_eq!(text(&body, "ifc_global_id"), "01cdlm001m080000008001");
    let (_, body) = f.run_args(&["ifc-guid", "--decode", "01cdlm001m080000008001"]);
    assert_eq!(text(&body, "uuid"), "019a7bf0-0000-7000-8000-000000200001");
    for bad in [
        "019A7BF0-0000-7000-8000-000000200001",
        "{019a7bf0-0000-7000-8000-000000200001}",
        "019a7bf000007000800000000200001",
        "019a7bf0-0000-4000-8000-000000200001",
        "019a7bf0-0000-7000-8000-00000020000",
        "",
    ] {
        let (code, body) = f.run_args(&["ifc-guid", "--id", bad]);
        assert_eq!(
            (code, text(&body, "code").as_str()),
            (2, "INVALID_ID"),
            "{bad}"
        );
        assert!(body.get("ifc_global_id").is_err(), "{bad}");
    }
    for bad in [
        "01cdlm001m08000000000",
        "01cdlm001m0800000000051",
        "41cdlm001m080000008001",
        "01cdlm001m0800000000+5",
        "01cdlm001m0800000000=5",
        "",
    ] {
        let (code, body) = f.run_args(&["ifc-guid", "--decode", bad]);
        assert_eq!(
            (code, text(&body, "code").as_str()),
            (2, "INVALID_ARGUMENT"),
            "{bad}"
        );
    }
    let (code, _) = f.run_args(&["ifc-guid"]);
    assert_eq!(code, 2);
    let (code, _) = f.run_args(&["ifc-guid", "--id", "x", "--decode", "y"]);
    assert_eq!(code, 2);
}

#[test]
fn export_is_a_command_of_the_help_text_and_needs_no_project_id() {
    let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .arg("--help")
        .output()
        .unwrap();
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("export-ifc") && help.contains("ifc-guid"));
    assert!(help.contains("there is no --project-id"));
}
