// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic layers, MPL-2.0; existing test-data attribution applies.

//! Real-backend candidate materialization. History/command-read checks belong to the planner.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use beamcraft_layer_engine::{authored::*, *};
use beamcraft_model::changeset::{Access, Dependency, DependencyGroup as G, DependencyTarget as T};
use beamcraft_model::digest::{ContentDigest, Sha256};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;
use openusd::sdf::{self, AbstractData};
use std::time::{Duration, Instant};
use steel::layer as steel_layer;
use steel::layer::*;

const ACTOR: &str = "0199e7a0-0000-7000-8000-000000000005";
const OP: &str = "0199e7a0-0000-7000-8000-000000000006";
fn id(n: usize) -> ElementId {
    ElementId::parse(&format!("0199e7a0-0000-7000-8000-{n:012x}")).unwrap()
}
fn path(n: usize) -> String {
    id(n).prim_path(beamcraft_model::prim::ElementCategory::Part)
}
fn digest(bytes: &[u8]) -> ContentDigest {
    Sha256::digest(bytes).unwrap()
}
fn control() -> Control {
    steel_layer::install();
    Control {
        cancellation: Cancellation::default(),
        deadline: Instant::now() + Duration::from_secs(90),
    }
}
fn asset(name: &str, text: &str) -> Asset {
    Asset {
        name: name.into(),
        digest: digest(text.as_bytes()),
        bytes: text.as_bytes().to_vec(),
    }
}
fn targets() -> EditTargets {
    EditTargets {
        authored: "authored.usda".into(),
        derived: "derived.usda".into(),
    }
}
fn empty() -> OpenRequest {
    let mut request = OpenRequest {
        identity: Identity {
            project: ACTOR.into(),
            workspace: OP.into(),
            revision: digest(b"base"),
            generation: 1,
        },
        root: "root.usda".into(),
        assets: vec![
            asset(
                "root.usda",
                "#usda 1.0\n(metersPerUnit=0.001\nupAxis=\"Z\"\nsubLayers=[@authored.usda@,@derived.usda@])\n",
            ),
            asset(
                "authored.usda",
                "#usda 1.0\ndef Scope \"Unknown\" { custom double future:factor=1.125 }\n",
            ),
            asset("derived.usda", "#usda 1.0\n"),
            asset("foreign.usda", "#usda 1.0\ndef Xform \"External\" {}\n"),
        ],
        owned_layers: ["root.usda", "authored.usda", "derived.usda"]
            .map(String::from)
            .into(),
        manifest: digest(b"placeholder"),
        backend: BACKEND,
        profile: Profile::StaticLocal,
        budgets: Budgets::default(),
    };
    request.manifest = request.manifest_digest().unwrap();
    request
}
fn create(n: usize) -> Change {
    Change::Create(Element {
        id: id(n).to_string(),
        name: format!("part {n}"),
        shape: steel_layer::plate(PlateShape {
            contour_mm: vec![
                [0.0, 0.0, 0.0],
                [100.0, 0.0, 0.0],
                [100.0, 100.0, 0.0],
                [0.0, 100.0, 0.0],
            ],
            thickness_mm: 10.0,
            thickness_side: None,
            corners: Vec::new(),
            bends: Vec::new(),
            holes: Vec::new(),
            hole_records: Vec::new(),
            cuts: Vec::new(),
        }),
        placement: Placement {
            translation_mm: [0.0; 3],
            orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
        },
        material: None,
    })
}
fn edit(mut request: OpenRequest, changes: Vec<Change>, resources: Vec<Resource>) -> OpenRequest {
    let schema = schema_identity().unwrap();
    let (mut author, opened) =
        Author::open(request.clone(), targets(), None, schema, &control()).unwrap();
    let prepared = author
        .prepare(
            Prepare {
                context: opened.context,
                op_id: OP.into(),
                actor: ACTOR.into(),
                schema,
                resources,
                changes,
            },
            &control(),
        )
        .unwrap();
    // The candidate's owned set replaces the request's: an element delete drops its file (U15).
    let names: std::collections::BTreeSet<String> =
        prepared.layers.iter().map(|l| l.name.clone()).collect();
    request
        .assets
        .retain(|a| !request.owned_layers.contains(&a.name) || names.contains(&a.name));
    request.owned_layers.retain(|n| names.contains(n));
    for layer in prepared.layers {
        let name = layer.name.clone();
        match request.assets.iter_mut().find(|a| a.name == name) {
            Some(slot) => *slot = layer,
            // A new element's own derived layer (U15) joins the stage.
            None => {
                request.owned_layers.insert(name.clone());
                request.assets.push(layer);
            }
        }
    }
    request.identity.generation += 1;
    request.manifest = request.manifest_digest().unwrap();
    request
}
fn initial() -> OpenRequest {
    edit(empty(), vec![create(10), create(11)], Vec::new())
}
fn write(n: usize, group: G) -> Dependency {
    Dependency {
        target: T::Element(id(n)),
        group,
        access: Access::Write,
    }
}
fn lifecycle(n: usize) -> Vec<Dependency> {
    [
        G::Lifecycle,
        G::Label,
        G::ShapeInputs,
        G::Placement,
        G::MaterialRef,
        G::Relationship,
        G::Derived,
    ]
    .map(|g| write(n, g))
    .into()
}
fn prepare(
    base: &OpenRequest,
    source: &OpenRequest,
    accepted: &OpenRequest,
    writes: &[Dependency],
) -> Result<OpenRequest, Diagnostic> {
    rebase::prepare(base, source, accepted, writes, &targets(), &control())
}
fn value(request: &OpenRequest, spec: &str, key: &str) -> sdf::Value {
    let bytes = &request
        .assets
        .iter()
        .find(|a| a.name == "authored.usda")
        .unwrap()
        .bytes;
    let data = openusd::usda::parse(std::str::from_utf8(bytes).unwrap()).unwrap();
    data.get_field(&sdf::Path::new(spec).unwrap(), key)
        .unwrap()
        .into_owned()
}
fn rename(n: usize, name: &str) -> Change {
    Change::Label {
        id: id(n).to_string(),
        name: name.into(),
    }
}

fn mesh_for(n: usize) -> Change {
    Change::Mesh(Mesh {
        id: id(n).to_string(),
        shape_key: digest(format!("mesh {n}").as_bytes()),
        points_mm: vec![
            [0.0, 0.0, 0.0],
            [100.0, 0.0, 0.0],
            [100.0, 100.0, 0.0],
            [0.0, 100.0, 0.0],
        ],
        face_counts: vec![4],
        indices: vec![0, 1, 2, 3],
        quantities: None,
    })
}

fn element_file(n: usize) -> String {
    element_layer(&id(n).to_string()).unwrap()
}

#[test]
fn u15_a_local_element_file_and_stack_survive_a_rebase() {
    // A realistic base: one published revision that already stamped the layer pins.
    let base = edit(empty(), vec![create(12), mesh_for(12)], Vec::new());
    let source = edit(base.clone(), vec![create(10), mesh_for(10)], Vec::new());
    let accepted = edit(base.clone(), vec![create(11)], Vec::new());
    let file = element_file(10);
    let source_file = source
        .assets
        .iter()
        .find(|a| a.name == file)
        .unwrap()
        .clone();
    let merged = prepare(&base, &source, &accepted, &lifecycle(10)).unwrap();
    // The locally created element's derived file is carried whole.
    let carried = merged.assets.iter().find(|a| a.name == file).unwrap();
    assert_eq!(carried.digest, source_file.digest);
    assert!(merged.owned_layers.contains(&file));
    // The accepted stack is rebuilt from the candidate's asset set: both meshed elements.
    let stack_bytes = &merged
        .assets
        .iter()
        .find(|a| a.name == "derived.usda")
        .unwrap()
        .bytes;
    let mut stack = layer_sublayers(stack_bytes).unwrap();
    stack.sort();
    assert_eq!(stack, [element_file(10), element_file(12)]);
    // The remote authored work survives beside it.
    assert_eq!(
        value(&merged, &format!("{}.bim:name", path(11)), "default"),
        sdf::Value::String("part 11".into())
    );
}

#[test]
fn u15_an_upstream_element_delete_conflicts_only_where_local_writes_reach_it() {
    let base = edit(
        empty(),
        vec![create(10), mesh_for(10), create(12)],
        Vec::new(),
    );
    let file = element_file(10);
    // Upstream deletes the element: its file and stack entry go with it.
    let accepted = edit(
        base.clone(),
        vec![Change::Delete {
            id: id(10).to_string(),
        }],
        Vec::new(),
    );
    assert!(accepted.assets.iter().all(|a| a.name != file));
    // A local rename of another element rebases cleanly over the deletion.
    let source = edit(base.clone(), vec![rename(12, "elsewhere")], Vec::new());
    let merged = prepare(&base, &source, &accepted, &[write(12, G::Label)]).unwrap();
    assert!(merged.assets.iter().all(|a| a.name != file));
    // A local write to the deleted element itself conflicts.
    let source = edit(base.clone(), vec![rename(10, "local")], Vec::new());
    let error = prepare(&base, &source, &accepted, &lifecycle(10))
        .err()
        .unwrap_or_else(|| panic!("a write into an upstream-deleted element must conflict"));
    assert_eq!(error.code, ErrorCode::DependencyConflict);
}
fn replace(request: &mut OpenRequest, name: &str, from: &str, to: &str) {
    let entry = request.assets.iter_mut().find(|a| a.name == name).unwrap();
    assert!(std::str::from_utf8(&entry.bytes).unwrap().contains(from));
    let text = std::str::from_utf8(&entry.bytes).unwrap().replace(from, to);
    *entry = asset(name, &text);
    request.manifest = request.manifest_digest().unwrap();
}

#[test]
fn separate_names_and_same_part_move_keep_both_clients_and_inert_accepted_opinions() {
    let base = initial();
    let source = edit(base.clone(), vec![rename(10, "local")], Vec::new());
    let mut accepted = edit(
        base.clone(),
        vec![
            rename(11, "remote"),
            Change::Placement {
                id: id(10).to_string(),
                placement: Placement {
                    translation_mm: [12.0, 34.0, 56.0],
                    orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
                },
            },
        ],
        Vec::new(),
    );
    replace(
        &mut accepted,
        "authored.usda",
        "future:factor = 1.125",
        "future:factor = 8.5",
    );
    let merged = prepare(&base, &source, &accepted, &[write(10, G::Label)]).unwrap();
    assert_eq!(
        value(&merged, &format!("{}.bim:name", path(10)), "default"),
        sdf::Value::String("local".into())
    );
    assert_eq!(
        value(&merged, &format!("{}.bim:name", path(11)), "default"),
        sdf::Value::String("remote".into())
    );
    assert_eq!(
        value(
            &merged,
            &format!("{}.xformOp:translate", path(10)),
            "default"
        ),
        value(
            &accepted,
            &format!("{}.xformOp:translate", path(10)),
            "default"
        )
    );
    assert_eq!(
        value(&merged, "/Unknown.future:factor", "default"),
        sdf::Value::Double(8.5)
    );
    for name in ["root.usda", "derived.usda", "foreign.usda"] {
        assert_eq!(
            merged.assets.iter().find(|a| a.name == name).unwrap().bytes,
            accepted
                .assets
                .iter()
                .find(|a| a.name == name)
                .unwrap()
                .bytes
        );
    }
    assert_eq!(
        value(&base, &format!("{}.bim:name", path(10)), "default"),
        sdf::Value::String("part 10".into())
    );
}

#[test]
fn independent_creates_merge_parent_membership_and_immutable_resources() {
    let base = initial();
    let resource = Resource::new(Definition::Material {
        density_kg_m3: Some(7850.0),
        grade: None,
        standard: None,
        display_name: Some("original steel".into()),
    })
    .unwrap();
    let source = edit(base.clone(), vec![create(12)], vec![resource.clone()]);
    let accepted = edit(base.clone(), vec![create(13)], Vec::new());
    let mut writes = lifecycle(12);
    writes.push(Dependency {
        target: T::Definition(resource.digest),
        group: G::Relationship,
        access: Access::Write,
    });
    let merged = prepare(&base, &source, &accepted, &writes).unwrap();
    for n in 10..=13 {
        assert_eq!(
            value(&merged, &format!("{}.bim:id", path(n)), "default"),
            sdf::Value::String(id(n).to_string())
        );
    }
    let definition = beamcraft_model::prim::DefinitionCategory::Material.path(resource.digest);
    assert_eq!(
        value(&merged, &format!("{definition}.bim:densityKgM3"), "default"),
        sdf::Value::Double(7850.0)
    );
    assert_eq!(
        value(&merged, &format!("{}.bim:createdBy", path(12)), "default"),
        value(&source, &format!("{}.bim:createdBy", path(12)), "default")
    );
}

#[test]
fn independent_delete_keeps_the_other_parts_edit() {
    let base = initial();
    let source = edit(
        base.clone(),
        vec![Change::Delete {
            id: id(10).to_string(),
        }],
        Vec::new(),
    );
    let accepted = edit(base.clone(), vec![rename(11, "kept")], Vec::new());
    let merged = prepare(&base, &source, &accepted, &lifecycle(10)).unwrap();
    assert_eq!(
        value(&merged, &format!("{}.bim:name", path(11)), "default"),
        sdf::Value::String("kept".into())
    );
    let bytes = &merged
        .assets
        .iter()
        .find(|a| a.name == "authored.usda")
        .unwrap()
        .bytes;
    let data = openusd::usda::parse(std::str::from_utf8(bytes).unwrap()).unwrap();
    assert!(!data.has_spec(&sdf::Path::new(&path(10)).unwrap()));
}

#[test]
fn same_group_is_atomic_even_for_different_properties() {
    let base = initial();
    let source = edit(
        base.clone(),
        vec![steel_layer::thickness(id(10).to_string(), 20.0)],
        Vec::new(),
    );
    let accepted = edit(
        base.clone(),
        vec![steel_layer::contour(
            id(10).to_string(),
            vec![[0.0, 0.0, 0.0], [200.0, 0.0, 0.0], [0.0, 100.0, 0.0]],
        )],
        Vec::new(),
    );
    assert_eq!(
        prepare(
            &base,
            &source,
            &accepted,
            &[write(10, G::ShapeInputs), write(10, G::Derived)]
        )
        .err()
        .unwrap()
        .code,
        ErrorCode::DependencyConflict
    );
}

#[test]
fn narrowed_writes_unknown_changes_and_foreign_bytes_fail_closed() {
    let base = initial();
    let source = edit(base.clone(), vec![rename(10, "local")], Vec::new());
    for writes in [vec![write(11, G::Label)], vec![write(10, G::Lifecycle)]] {
        assert_eq!(
            prepare(&base, &source, &base, &writes).err().unwrap().code,
            ErrorCode::IntegrityError
        );
    }
    let mut unknown = source.clone();
    replace(
        &mut unknown,
        "authored.usda",
        "future:factor = 1.125",
        "future:factor = 2.0",
    );
    assert_eq!(
        prepare(&base, &unknown, &base, &[write(10, G::Label)])
            .err()
            .unwrap()
            .code,
        ErrorCode::UnsupportedProperty
    );
    let mut foreign = source;
    replace(&mut foreign, "foreign.usda", "External", "Rewritten");
    assert_eq!(
        prepare(&base, &foreign, &base, &[write(10, G::Label)])
            .err()
            .unwrap()
            .code,
        ErrorCode::UnsupportedProperty
    );
    let mut mixed = [base.clone(), unknown, base];
    for request in &mut mixed {
        replace(
            request,
            "root.usda",
            "@derived.usda@",
            "@derived.usda@,@foreign.usda@",
        );
        replace(
            request,
            "foreign.usda",
            "def Xform \"External\" {}",
            "def Xform \"World\" { custom double foreign:note = 1 }",
        );
    }
    assert_eq!(
        prepare(&mixed[0], &mixed[1], &mixed[2], &[write(10, G::Label)])
            .err()
            .unwrap()
            .code,
        ErrorCode::ForeignReadOnly
    );
}

#[test]
fn dependency_cap_context_and_cancellation_are_checked_before_output() {
    let base = initial();
    let source = edit(base.clone(), vec![rename(10, "local")], Vec::new());
    let mut writes = vec![write(10, G::Label); 4096];
    prepare(&base, &source, &base, &writes).unwrap();
    writes.push(write(10, G::Label));
    assert_eq!(
        prepare(&base, &source, &base, &writes).err().unwrap().code,
        ErrorCode::LimitExceeded
    );
    let mut wrong = source.clone();
    wrong.identity.project = OP.into();
    wrong.manifest = wrong.manifest_digest().unwrap();
    assert_eq!(
        prepare(&base, &wrong, &base, &[write(10, G::Label)])
            .err()
            .unwrap()
            .code,
        ErrorCode::InvalidArgument
    );
    let c = control();
    c.cancellation.cancel();
    assert_eq!(
        rebase::prepare(
            &base,
            &source,
            &base,
            &[write(10, G::Label)],
            &targets(),
            &c
        )
        .err()
        .unwrap()
        .code,
        ErrorCode::Cancelled
    );
}

#[test]
fn part_class_edits_rebase_as_label_opinions_and_keep_their_value() {
    let base = initial();
    let source = edit(
        base.clone(),
        vec![Change::PartClass {
            id: id(10).to_string(),
            class: 3,
        }],
        Vec::new(),
    );
    let accepted = edit(base.clone(), vec![rename(11, "remote")], Vec::new());
    let merged = prepare(&base, &source, &accepted, &[write(10, G::Label)]).unwrap();
    assert_eq!(
        value(&merged, &format!("{}.steel:class", path(10)), "default"),
        sdf::Value::Int(3)
    );
    assert_eq!(
        value(&merged, &format!("{}.bim:name", path(11)), "default"),
        sdf::Value::String("remote".into())
    );
}

#[test]
fn a_mesh_transfer_carries_its_shared_asset_over_an_accepted_rename() {
    let base = initial();
    let mesh = Mesh {
        id: id(11).to_string(),
        shape_key: digest(b"transfer-shape-key"),
        points_mm: vec![
            [0.0, 0.0, 0.0],
            [100.0, 0.0, 0.0],
            [100.0, 100.0, 5.0],
            [0.0, 100.0, 5.0],
        ],
        face_counts: vec![4],
        indices: vec![0, 1, 2, 3],
        quantities: None,
        reduced: None,
        proxy: None,
        axis: None,
    };
    let local = edit(base.clone(), vec![Change::Mesh(mesh.clone())], Vec::new());
    let accepted = edit(base.clone(), vec![rename(10, "renamed")], Vec::new());
    // The transfer's evidence names the immutable asset (U11) by content digest; without that
    // write the shared prim's delta is outside the verified writes.
    let asset_digest = mesh_asset_digest(&mesh).unwrap();
    let mut writes = lifecycle(11);
    writes.push(Dependency {
        target: T::Definition(asset_digest),
        group: G::Derived,
        access: Access::Write,
    });
    let merged = prepare(&base, &local, &accepted, &writes).unwrap();
    let derived = std::str::from_utf8(
        &merged
            .assets
            .iter()
            .find(|a| a.name == "derived.usda")
            .unwrap()
            .bytes,
    )
    .unwrap();
    assert!(derived.contains("instanceable = true"));
    assert!(derived.contains(&format!("GEO_{asset_digest}")));
    let authored = std::str::from_utf8(
        &merged
            .assets
            .iter()
            .find(|a| a.name == "authored.usda")
            .unwrap()
            .bytes,
    )
    .unwrap();
    assert!(authored.contains("renamed"));
    let mut reader = Reader::default();
    reader
        .open_validated(merged.clone(), schema_identity().unwrap(), &control())
        .unwrap();
    // An asset delta without its digest write stays outside the verified writes.
    assert_eq!(
        prepare(&base, &local, &accepted, &lifecycle(11))
            .err()
            .unwrap()
            .code,
        ErrorCode::IntegrityError
    );
}
