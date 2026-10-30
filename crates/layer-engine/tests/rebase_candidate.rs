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
    Control {
        cancellation: Cancellation::default(),
        deadline: Instant::now() + Duration::from_secs(20),
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
        shape: Shape::Plate {
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
        },
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
        Author::open(request.clone(), targets(), schema, &control()).unwrap();
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
    for layer in prepared.layers {
        let name = layer.name.clone();
        *request.assets.iter_mut().find(|a| a.name == name).unwrap() = layer;
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
        vec![Change::Thickness {
            id: id(10).to_string(),
            mm: 20.0,
        }],
        Vec::new(),
    );
    let accepted = edit(
        base.clone(),
        vec![Change::Contour {
            id: id(10).to_string(),
            points_mm: vec![[0.0, 0.0, 0.0], [200.0, 0.0, 0.0], [0.0, 100.0, 0.0]],
        }],
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
