// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original generated synthetic fixtures, MPL-2.0; covered by ATTRIBUTION.md's
// test-data row. Expectations follow F04 sections 2-4/8 and F06 sections 2/4.

//! U02 real-backend authoring regressions with original in-memory fixtures.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use beamcraft_layer_engine::{authored::*, *};
use beamcraft_model::{
    digest::{ContentDigest, Sha256},
    error::ErrorCode,
};
use openusd::sdf::AbstractData;
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};

const PLATE: &str = "0199e7a0-0000-7000-8000-000000000003";
const MEMBER: &str = "0199e7a0-0000-7000-8000-000000000004";
const ACTOR: &str = "0199e7a0-0000-7000-8000-000000000005";
const OP: &str = "0199e7a0-0000-7000-8000-000000000006";
fn digest(bytes: &[u8]) -> ContentDigest {
    Sha256::digest(bytes).unwrap()
}
fn control() -> Control {
    Control {
        cancellation: Cancellation::default(),
        deadline: Instant::now() + Duration::from_secs(10),
    }
}
fn asset(name: &str, text: impl AsRef<[u8]>) -> Asset {
    Asset {
        name: name.into(),
        digest: digest(text.as_ref()),
        bytes: text.as_ref().to_vec(),
    }
}
fn layer(body: &str) -> String {
    format!("#usda 1.0\n(metersPerUnit=0.001\nupAxis=\"Z\")\n{body}\n")
}
fn request(assets: Vec<Asset>, owned: &[&str]) -> OpenRequest {
    let mut request = OpenRequest {
        identity: Identity {
            project: "0199e7a0-0000-7000-8000-000000000001".into(),
            workspace: "0199e7a0-0000-7000-8000-000000000002".into(),
            revision: digest(b"pinned U02 base"),
            generation: 42,
        },
        root: "root.usda".into(),
        assets,
        owned_layers: owned.iter().map(|s| (*s).into()).collect(),
        manifest: digest(b"placeholder"),
        backend: BACKEND,
        profile: Profile::StaticLocal,
        budgets: Budgets::default(),
    };
    request.manifest = request.manifest_digest().unwrap();
    request
}
fn initial() -> OpenRequest {
    request(
        vec![
            asset(
                "root.usda",
                "#usda 1.0\n(metersPerUnit=0.001\nupAxis=\"Z\"\nsubLayers=[@authored.usda@,@derived.usda@,@weak.usda@]\ncustomLayerData={string future=\"keep\"})\n",
            ),
            asset(
                "authored.usda",
                layer(
                    "def Scope \"Unknown\" { custom double future:factor=1.125\ncustom double future:absent\ncustom double future:blocked=None\ncustom rel future:link=</Unknown> }",
                ),
            ),
            asset("derived.usda", layer("")),
            asset("weak.usda", layer("")),
        ],
        &["root.usda", "authored.usda", "derived.usda", "weak.usda"],
    )
}
fn targets() -> EditTargets {
    EditTargets {
        authored: "authored.usda".into(),
        derived: "derived.usda".into(),
    }
}
fn open(input: OpenRequest) -> (Author, Opened) {
    Author::open(input, targets(), schema_identity().unwrap(), &control()).unwrap()
}
fn pose() -> Placement {
    Placement {
        translation_mm: [12.25, -42.125, 0.0],
        orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
    }
}
fn resources() -> Vec<Resource> {
    vec![
        Resource::new(Definition::RectangleProfile {
            width_mm: 100.0,
            height_mm: 50.0,
            section: SectionProperties::default(),
        })
        .unwrap(),
        Resource::new(Definition::Material {
            density_kg_m3: Some(1000.0),
            grade: None,
            standard: None,
            display_name: None,
        })
        .unwrap(),
    ]
}
fn elements() -> Vec<Change> {
    let r = resources();
    vec![
        Change::Create(Element {
            id: PLATE.into(),
            name: "Plate Ω".into(),
            shape: Shape::Plate {
                contour_mm: vec![
                    [0.0, 0.0, 0.0],
                    [1000.0, 0.0, 0.0],
                    [1000.0, 500.0, 0.0],
                    [0.0, 500.0, 0.0],
                ],
                thickness_mm: 10.123456789012345,
                holes: Vec::new(),
                cut: None,
            },
            placement: pose(),
            material: Some(r[1].digest),
        }),
        Change::Create(Element {
            id: MEMBER.into(),
            name: "Member".into(),
            shape: Shape::Member {
                kind: MemberKind::Beam,
                axis_mm: [[0.0, 0.0, 0.0], [0.0, 0.0, 2000.0]],
                profile: r[0].digest,
                roll_rad: Some(0.125),
                cut: None,
            },
            placement: pose(),
            material: None,
        }),
    ]
}
fn mesh() -> Mesh {
    // Original synthetic topology transfer fixture; not a certified engineering solid.
    Mesh {
        id: PLATE.into(),
        shape_key: digest(b"engine-supplied-shape-key"),
        points_mm: vec![
            [0.0, 0.0, 0.0],
            [1000.0, 0.0, 0.0],
            [1000.0, 500.0, 0.0],
            [0.0, 500.0, 0.0],
        ],
        face_counts: vec![4],
        indices: vec![0, 1, 2, 3],
        quantities: None,
    }
}
fn batch(context: &Context, changes: Vec<Change>) -> Prepare {
    Prepare {
        context: context.clone(),
        op_id: OP.into(),
        actor: ACTOR.into(),
        schema: schema_identity().unwrap(),
        resources: Vec::new(),
        changes,
    }
}
fn prepare_initial(author: &mut Author, context: &Context) -> Prepared {
    let mut input = batch(context, elements());
    input.resources = resources();
    input.changes.push(Change::Mesh(mesh()));
    author.prepare(input, &control()).unwrap()
}
fn populated() -> (Author, Opened) {
    let (mut a, o) = open(initial());
    let prepared = prepare_initial(&mut a, &o.context);
    let installed = a
        .install(
            &o.context,
            &prepared.token,
            &prepared.op_id,
            Decision::Accepted,
            &control(),
        )
        .unwrap();
    (a, installed)
}
fn path(id: &str) -> String {
    use beamcraft_model::prim::ElementCategory;
    let category = match id {
        FRAME | OUTER => ElementCategory::Assembly,
        WELD | BOLT => ElementCategory::Connection,
        STOREY | ROOF => ElementCategory::Level,
        _ => ElementCategory::Part,
    };
    beamcraft_model::id::ElementId::parse(id)
        .unwrap()
        .prim_path(category)
}
fn snapshot(author: &Author, context: &Context) -> Vec<Prim> {
    let mut result = Vec::new();
    let mut cursor = None;
    loop {
        let page = author.read(context, cursor.as_ref(), &control()).unwrap();
        result.extend(page.items);
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    result
}
fn value<'a>(prims: &'a [Prim], id: &str, name: &str) -> Option<&'a Value> {
    prims
        .iter()
        .find(|p| p.element_id.as_deref() == Some(id))
        .unwrap()
        .properties
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| p.value.as_ref())
}

#[test]
fn author_install_reopen_standard_mesh_relationships_and_exact_unknown_opinions() {
    let (mut a, o) = open(initial());
    let before = a.export(&o.context, &control()).unwrap();
    let p = prepare_initial(&mut a, &o.context);
    assert_eq!(
        a.export(&o.context, &control())
            .unwrap()
            .iter()
            .map(|a| &a.bytes)
            .collect::<Vec<_>>(),
        before.iter().map(|a| &a.bytes).collect::<Vec<_>>()
    );
    assert_eq!(o.context.identity().generation, 42);
    assert_eq!(p.validation.profile, "u02-structural-v1");
    let new = a
        .install(
            &o.context,
            &p.token,
            &p.op_id,
            Decision::Accepted,
            &control(),
        )
        .unwrap();
    assert_eq!(new.context.identity().generation, 43);
    assert_eq!(new.meters_per_unit, 0.001);
    assert_eq!(new.up_axis, "Z");
    assert_eq!(
        a.read(&o.context, None, &control()).unwrap_err().code,
        ErrorCode::StaleWorkspace
    );
    let all = snapshot(&a, &new.context);
    assert_eq!(
        value(&all, PLATE, "steel:plate:thicknessMm"),
        Some(&Value::Number(10.123456789012345))
    );
    assert_eq!(
        value(&all, MEMBER, "steel:member:rollRad"),
        Some(&Value::Number(0.125))
    );
    assert!(
        all.iter()
            .find(|p| p.element_id.as_deref() == Some(PLATE))
            .unwrap()
            .relationships
            .iter()
            .any(|r| r.name == "bim:material" && r.targets.len() == 1)
    );
    let exported = a.export(&new.context, &control()).unwrap();
    let own = exported.iter().find(|a| a.name == "authored.usda").unwrap();
    let parsed = openusd::usda::parse(std::str::from_utf8(&own.bytes).unwrap()).unwrap();
    assert!(parsed.has_spec(&openusd::sdf::Path::new("/Unknown.future:link").unwrap()));
    assert!(parsed.has_spec(
        &openusd::sdf::Path::new(&format!("{}.bim:axisPointsMm", path(MEMBER))).unwrap()
    ));
    assert!(
        !parsed.has_spec(&openusd::sdf::Path::new(&format!("{}/Geometry", path(PLATE))).unwrap())
    );
    let bin = exported.iter().find(|a| a.name == "derived.usda").unwrap();
    let parsed = openusd::usda::parse(std::str::from_utf8(&bin.bytes).unwrap()).unwrap();
    let mesh_path = openusd::sdf::Path::new(&format!("{}/Geometry.points", path(PLATE))).unwrap();
    assert_eq!(
        parsed.get_field(&mesh_path, "typeName").unwrap().as_ref(),
        &openusd::sdf::Value::token("point3f[]")
    );
    let req = request(
        exported,
        &["root.usda", "authored.usda", "derived.usda", "weak.usda"],
    );
    assert!(validate_layers(req.clone(), schema_identity().unwrap(), &control()).is_ok());
    let (fresh, opened) = open(req);
    assert_eq!(snapshot(&fresh, &opened.context), all);
}

#[test]
fn twenty_seeded_permutations_produce_identical_owned_bytes_and_digests() {
    let mut expected = None;
    for seed in 0..20 {
        let (mut a, o) = open(initial());
        let mut changes = elements();
        changes.push(Change::Mesh(mesh()));
        let mut r = resources();
        if seed % 2 == 1 {
            r.reverse();
        }
        changes.rotate_left(seed % 3);
        if seed % 4 == 0 {
            changes.reverse();
        }
        let mut input = batch(&o.context, changes);
        input.resources = r;
        let p = a.prepare(input, &control()).unwrap();
        let bytes = p
            .layers
            .iter()
            .map(|a| (a.name.clone(), a.digest, a.bytes.clone()))
            .collect::<Vec<_>>();
        if let Some(expected) = &expected {
            assert_eq!(expected, &bytes);
        } else {
            expected = Some(bytes);
        }
        let o = a
            .install(
                &o.context,
                &p.token,
                &p.op_id,
                Decision::Accepted,
                &control(),
            )
            .unwrap();
        let req = request(
            a.export(&o.context, &control()).unwrap(),
            &["root.usda", "authored.usda", "derived.usda", "weak.usda"],
        );
        let (_, o) = open(req);
        assert!(o.prim_count >= 8);
    }
}

#[test]
fn absence_block_removal_reveal_weaker_defaults_without_flattening() {
    let (a, o) = populated();
    let mut assets = a.export(&o.context, &control()).unwrap();
    let weak = assets.iter_mut().find(|a| a.name == "weak.usda").unwrap();
    *weak = asset(
        "weak.usda",
        layer(&format!(
            "over \"World\" {{ over \"Steel\" {{ over \"Parts\" {{ over \"PRT_{}\" {{ custom double steel:member:rollRad=0.75 }} }} }} }}",
            MEMBER.replace('-', "")
        )),
    );
    let weak_before = weak.bytes.clone();
    let (mut a, mut o) = open(request(
        assets,
        &["root.usda", "authored.usda", "derived.usda", "weak.usda"],
    ));
    for (opinion, expected, spec_exists) in [
        (Opinion::Absent, Some(Value::Number(0.75)), true),
        (Opinion::Block, Some(Value::Block), true),
        (Opinion::Remove, Some(Value::Number(0.75)), false),
        (Opinion::Value(0.125), Some(Value::Number(0.125)), true),
    ] {
        let expected_local = opinion.clone();
        let p = a
            .prepare(
                batch(
                    &o.context,
                    vec![Change::Roll {
                        id: MEMBER.into(),
                        opinion,
                    }],
                ),
                &control(),
            )
            .unwrap();
        let local = p
            .opinions
            .iter()
            .find(|v| v.element_id == MEMBER && v.attribute == "steel:member:rollRad")
            .unwrap();
        let expected_local = match expected_local {
            Opinion::Value(n) => Opinion::Value(Value::Number(n)),
            Opinion::Absent => Opinion::Absent,
            Opinion::Block => Opinion::Block,
            Opinion::Remove => Opinion::Remove,
        };
        assert_eq!(local.opinion, expected_local);
        o = a
            .install(
                &o.context,
                &p.token,
                &p.op_id,
                Decision::Accepted,
                &control(),
            )
            .unwrap();
        assert_eq!(
            value(&snapshot(&a, &o.context), MEMBER, "steel:member:rollRad"),
            expected.as_ref()
        );
        let assets = a.export(&o.context, &control()).unwrap();
        assert_eq!(
            assets.iter().find(|a| a.name == "weak.usda").unwrap().bytes,
            weak_before
        );
        let d = openusd::usda::parse(
            std::str::from_utf8(
                &assets
                    .iter()
                    .find(|a| a.name == "authored.usda")
                    .unwrap()
                    .bytes,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            d.has_spec(
                &openusd::sdf::Path::new(&format!("{}.steel:member:rollRad", path(MEMBER)))
                    .unwrap()
            ),
            spec_exists
        );
    }
}

#[test]
fn foreign_reference_overrides_and_foreign_ancestor_writes_fail_atomically() {
    let mut input = initial();
    let foreign = layer("def Xform \"Source\" { custom double future:value=23 }");
    input.assets.push(asset("foreign.usda", foreign));
    let body = layer(&format!(
        "over \"World\" {{ over \"Steel\" {{ over \"Parts\" {{ def Xform \"PRT_{}\" (references=@foreign.usda@</Source>) {{ custom string bim:id=\"{PLATE}\" }} }} }} }}",
        PLATE.replace('-', "")
    ));
    *input
        .assets
        .iter_mut()
        .find(|a| a.name == "authored.usda")
        .unwrap() = asset("authored.usda", body);
    input.manifest = input.manifest_digest().unwrap();
    let (mut a, o) = open(input);
    let before = a.export(&o.context, &control()).unwrap();
    for change in [
        Change::Label {
            id: PLATE.into(),
            name: "Attempt".into(),
        },
        Change::Thickness {
            id: PLATE.into(),
            mm: 10.0,
        },
        Change::Material {
            id: PLATE.into(),
            opinion: MaterialOpinion::Empty,
        },
        Change::Mesh(mesh()),
    ] {
        assert_eq!(
            a.prepare(batch(&o.context, vec![change]), &control())
                .unwrap_err()
                .code,
            ErrorCode::ForeignReadOnly
        );
    }
    assert_eq!(
        a.export(&o.context, &control())
            .unwrap()
            .iter()
            .map(|a| &a.bytes)
            .collect::<Vec<_>>(),
        before.iter().map(|a| &a.bytes).collect::<Vec<_>>()
    );
    // A foreign ancestor also forbids creating a new child in our stronger layer.
    let mut req = initial();
    req.assets.push(asset(
        "foreign.usda",
        layer("over \"World\" { over \"Steel\" {} }"),
    ));
    let root = req
        .assets
        .iter_mut()
        .find(|a| a.name == "root.usda")
        .unwrap();
    *root = asset(
        "root.usda",
        String::from_utf8(root.bytes.clone())
            .unwrap()
            .replace("@weak.usda@]", "@weak.usda@,@foreign.usda@]"),
    );
    req.manifest = req.manifest_digest().unwrap();
    let (mut a, o) = open(req);
    assert_eq!(
        a.prepare(batch(&o.context, elements()), &control())
            .unwrap_err()
            .code,
        ErrorCode::ForeignReadOnly
    );
}

#[test]
fn mismatched_schema_pin_corruption_and_nonfinite_never_change_live_state() {
    let (mut a, o) = populated();
    let before = snapshot(&a, &o.context);
    let mut input = batch(
        &o.context,
        vec![Change::Label {
            id: PLATE.into(),
            name: "after".into(),
        }],
    );
    input.schema.digest = digest(b"wrong schema");
    assert_eq!(
        a.prepare(input, &control()).unwrap_err().code,
        ErrorCode::UnsupportedVersion
    );
    for mm in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        0.0,
        -1.0,
        0.0009,
        1_000_000.1,
    ] {
        let code = if !mm.is_finite() {
            ErrorCode::NonFiniteNumber
        } else if mm > 1e6 {
            ErrorCode::LimitExceeded
        } else {
            ErrorCode::InvalidArgument
        };
        assert_eq!(
            a.prepare(
                batch(
                    &o.context,
                    vec![Change::Thickness {
                        id: PLATE.into(),
                        mm
                    }]
                ),
                &control()
            )
            .unwrap_err()
            .code,
            code
        );
        assert_eq!(snapshot(&a, &o.context), before);
    }
    let mut req = initial();
    req.assets[1].bytes.push(0);
    assert_eq!(
        validate_layers(req, schema_identity().unwrap(), &control())
            .unwrap_err()
            .code,
        ErrorCode::IntegrityError
    );
    let mut req = initial();
    req.backend.version = "0.9.0";
    req.manifest = req.manifest_digest().unwrap();
    assert_eq!(
        validate_layers(req, schema_identity().unwrap(), &control())
            .unwrap_err()
            .code,
        ErrorCode::UnsupportedVersion
    );
    let mut req = initial();
    req.assets[1] = asset("authored.usda", layer("def {"));
    req.manifest = req.manifest_digest().unwrap();
    assert!(validate_layers(req, schema_identity().unwrap(), &control()).is_err());
}

#[test]
fn candidate_decision_cancellation_wrong_owner_stale_token_and_generation_gates() {
    let (mut a, o) = open(initial());
    let p = prepare_initial(&mut a, &o.context);
    assert_eq!(
        a.prepare(batch(&o.context, elements()), &control())
            .unwrap_err()
            .code,
        ErrorCode::DependencyConflict
    );
    for decision in [Decision::Accepted, Decision::Unknown] {
        assert_eq!(
            a.discard(&o.context, &p.token, &p.op_id, decision, &control())
                .unwrap_err()
                .code,
            ErrorCode::OutcomeUnknown
        );
    }
    assert_eq!(
        a.install(
            &o.context,
            &p.token,
            &p.op_id,
            Decision::Unknown,
            &control()
        )
        .unwrap_err()
        .code,
        ErrorCode::OutcomeUnknown
    );
    let cancelled = control();
    cancelled.cancellation.cancel();
    assert_eq!(
        a.install(
            &o.context,
            &p.token,
            &p.op_id,
            Decision::Accepted,
            &cancelled
        )
        .unwrap_err()
        .code,
        ErrorCode::OutcomeUnknown
    );
    assert_eq!(snapshot(&a, &o.context).len(), 1);
    let (mut other, other_o) = open(initial());
    assert_eq!(
        other
            .discard(
                &other_o.context,
                &p.token,
                &p.op_id,
                Decision::Undecided,
                &control()
            )
            .unwrap_err()
            .code,
        ErrorCode::StaleWorkspace
    );
    a.discard(
        &o.context,
        &p.token,
        &p.op_id,
        Decision::Rejected,
        &control(),
    )
    .unwrap();
    assert_eq!(
        a.install(
            &o.context,
            &p.token,
            &p.op_id,
            Decision::Accepted,
            &control()
        )
        .unwrap_err()
        .code,
        ErrorCode::StaleWorkspace
    );
    assert_eq!(
        a.prepare(batch(&o.context, elements()), &cancelled)
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
    let p = prepare_initial(&mut a, &o.context);
    let new = a
        .install(
            &o.context,
            &p.token,
            &p.op_id,
            Decision::Accepted,
            &control(),
        )
        .unwrap();
    assert_eq!(new.context.identity().generation, 43);
    let mut req = initial();
    req.identity.generation = u64::MAX;
    req.manifest = req.manifest_digest().unwrap();
    let (mut a, o) = open(req);
    let mut input = batch(&o.context, elements());
    input.resources = resources();
    assert_eq!(
        a.prepare(input, &control()).unwrap_err().code,
        ErrorCode::LimitExceeded
    );
}

#[test]
fn hundred_serialized_changes_invalidate_contexts_and_derive_semantic_groups() {
    let (mut a, mut o) = populated();
    for n in 0..100 {
        let id = format!("0199e7a0-0000-7000-8000-{:012x}", n + 100);
        let mut input = batch(
            &o.context,
            vec![Change::Label {
                id: PLATE.into(),
                name: format!("Plate {n}"),
            }],
        );
        input.op_id = id;
        let p = a.prepare(input, &control()).unwrap();
        assert!(p.writes.contains(&Footprint {
            identity: PLATE.into(),
            group: Group::Label
        }));
        assert!(!p.writes.contains(&Footprint {
            identity: PLATE.into(),
            group: Group::ShapeInputs
        }));
        o = a
            .install(
                &o.context,
                &p.token,
                &p.op_id,
                Decision::Accepted,
                &control(),
            )
            .unwrap();
    }
    assert_eq!(o.context.identity().generation, 143);
    let p = a
        .prepare(
            batch(
                &o.context,
                vec![Change::Thickness {
                    id: PLATE.into(),
                    mm: 12.0,
                }],
            ),
            &control(),
        )
        .unwrap();
    assert!(p.reads.contains(&Footprint {
        identity: PLATE.into(),
        group: Group::ShapeInputs
    }));
    assert!(p.writes.contains(&Footprint {
        identity: PLATE.into(),
        group: Group::Derived
    }));
    o = a
        .install(
            &o.context,
            &p.token,
            &p.op_id,
            Decision::Accepted,
            &control(),
        )
        .unwrap();
    assert!(
        !snapshot(&a, &o.context)
            .iter()
            .any(|p| p.path.ends_with("/Geometry"))
    );
}

#[test]
fn duplicate_keys_wrong_kind_bad_mesh_and_missing_or_forged_definitions_fail() {
    let (mut a, o) = populated();
    let c = Change::Label {
        id: PLATE.into(),
        name: "rename".into(),
    };
    assert_eq!(
        a.prepare(batch(&o.context, vec![c.clone(), c]), &control())
            .unwrap_err()
            .code,
        ErrorCode::MalformedCommand
    );
    assert_eq!(
        a.prepare(
            batch(
                &o.context,
                vec![Change::Roll {
                    id: PLATE.into(),
                    opinion: Opinion::Value(0.0)
                }]
            ),
            &control()
        )
        .unwrap_err()
        .code,
        ErrorCode::UnsupportedProperty
    );
    let mut bad = mesh();
    bad.indices[0] = -1;
    assert_eq!(
        a.prepare(batch(&o.context, vec![Change::Mesh(bad)]), &control())
            .unwrap_err()
            .code,
        ErrorCode::InvalidArgument
    );
    let mut bad = mesh();
    bad.points_mm[0][0] = f32::NAN;
    assert_eq!(
        a.prepare(batch(&o.context, vec![Change::Mesh(bad)]), &control())
            .unwrap_err()
            .code,
        ErrorCode::NonFiniteNumber
    );
    assert_eq!(
        a.prepare(
            batch(
                &o.context,
                vec![Change::Material {
                    id: PLATE.into(),
                    opinion: MaterialOpinion::Target(digest(b"missing"))
                }]
            ),
            &control()
        )
        .unwrap_err()
        .code,
        ErrorCode::DanglingRelationship
    );
    let (mut a, o) = open(initial());
    let mut input = batch(&o.context, elements());
    input.resources = resources();
    input.resources[0].digest = digest(b"forged");
    assert_eq!(
        a.prepare(input, &control()).unwrap_err().code,
        ErrorCode::IntegrityError
    );
    let mut req = initial();
    req.owned_layers = BTreeSet::from(["root.usda".into()]);
    req.manifest = req.manifest_digest().unwrap();
    assert_eq!(
        Author::open(req, targets(), schema_identity().unwrap(), &control())
            .err()
            .unwrap()
            .code,
        ErrorCode::ForeignReadOnly
    );
}

#[test]
fn material_empty_remove_profile_pin_and_loaded_unknown_schema_preservation() {
    let (mut a, mut o) = populated();
    for opinion in [MaterialOpinion::Empty, MaterialOpinion::Remove] {
        let p = a
            .prepare(
                batch(
                    &o.context,
                    vec![Change::Material {
                        id: PLATE.into(),
                        opinion: opinion.clone(),
                    }],
                ),
                &control(),
            )
            .unwrap();
        let local = p
            .opinions
            .iter()
            .find(|p| p.attribute == "bim:material")
            .unwrap();
        assert_eq!(
            local.opinion,
            if opinion == MaterialOpinion::Empty {
                Opinion::Value(Value::Array(vec![]))
            } else {
                Opinion::Remove
            }
        );
        o = a
            .install(
                &o.context,
                &p.token,
                &p.op_id,
                Decision::Accepted,
                &control(),
            )
            .unwrap();
    }
    let r = Resource::new(Definition::RectangleProfile {
        width_mm: 120.0,
        height_mm: 60.0,
        section: SectionProperties::default(),
    })
    .unwrap();
    let mut input = batch(
        &o.context,
        vec![Change::Profile {
            id: MEMBER.into(),
            digest: r.digest,
        }],
    );
    input.resources = vec![r];
    let p = a.prepare(input, &control()).unwrap();
    assert!(p.reads.contains(&Footprint {
        identity: resources()[0].digest.to_string(),
        group: Group::Relationship
    }));
    o = a
        .install(
            &o.context,
            &p.token,
            &p.op_id,
            Decision::Accepted,
            &control(),
        )
        .unwrap();
    let mut assets = a.export(&o.context, &control()).unwrap();
    let authored = assets
        .iter_mut()
        .find(|a| a.name == "authored.usda")
        .unwrap();
    // A newer schema is inert on load and cannot be silently migrated by editing it.
    let text = String::from_utf8(authored.bytes.clone()).unwrap().replace(
        "bim:schemaVersion = \"0.1.0\"",
        "bim:schemaVersion = \"0.9.0\"",
    );
    *authored = asset("authored.usda", text);
    let (mut a, o) = open(request(
        assets,
        &["root.usda", "authored.usda", "derived.usda", "weak.usda"],
    ));
    assert_eq!(
        a.prepare(
            batch(
                &o.context,
                vec![Change::Label {
                    id: PLATE.into(),
                    name: "attempt".into()
                }]
            ),
            &control()
        )
        .unwrap_err()
        .code,
        ErrorCode::UnsupportedVersion
    );
}

#[test]
fn validator_rejects_schema_markers_changed_definition_bytes_and_loaded_bad_mesh() {
    let (a, o) = populated();
    let base = a.export(&o.context, &control()).unwrap();
    let mut assets = base.clone();
    let own = assets
        .iter_mut()
        .find(|a| a.name == "authored.usda")
        .unwrap();
    *own = asset(
        "authored.usda",
        String::from_utf8(own.bytes.clone()).unwrap().replace(
            "steel:profile:widthMm = 100.0",
            "steel:profile:widthMm = 101.0",
        ),
    );
    assert_eq!(
        validate_layers(
            request(
                assets,
                &["root.usda", "authored.usda", "derived.usda", "weak.usda"]
            ),
            schema_identity().unwrap(),
            &control()
        )
        .unwrap_err()
        .code,
        ErrorCode::IntegrityError
    );
    let mut assets = base.clone();
    let derived = assets
        .iter_mut()
        .find(|a| a.name == "derived.usda")
        .unwrap();
    *derived = asset(
        "derived.usda",
        String::from_utf8(derived.bytes.clone())
            .unwrap()
            .replace("[0, 1, 2, 3]", "[0, 1, 2, 999]"),
    );
    assert_eq!(
        validate_layers(
            request(
                assets,
                &["root.usda", "authored.usda", "derived.usda", "weak.usda"]
            ),
            schema_identity().unwrap(),
            &control()
        )
        .unwrap_err()
        .code,
        ErrorCode::InvalidArgument
    );
    let mut assets = base;
    let root = assets.iter_mut().find(|a| a.name == "root.usda").unwrap();
    *root = asset(
        "root.usda",
        String::from_utf8(root.bytes.clone()).unwrap().replace(
            &schema_identity().unwrap().digest.to_string(),
            &digest(b"incorrect schema").to_string(),
        ),
    );
    assert_eq!(
        validate_layers(
            request(
                assets,
                &["root.usda", "authored.usda", "derived.usda", "weak.usda"]
            ),
            schema_identity().unwrap(),
            &control()
        )
        .unwrap_err()
        .code,
        ErrorCode::UnsupportedVersion
    );
}

#[test]
fn changed_input_bounds_are_inclusive_and_rejections_keep_the_entire_patch_atomic() {
    let (mut a, mut o) = populated();
    let before = snapshot(&a, &o.context);
    let bad = vec![
        Change::Label {
            id: PLATE.into(),
            name: "would be valid".into(),
        },
        Change::Thickness {
            id: PLATE.into(),
            mm: f64::INFINITY,
        },
    ];
    assert_eq!(
        a.prepare(batch(&o.context, bad), &control())
            .unwrap_err()
            .code,
        ErrorCode::NonFiniteNumber
    );
    assert_eq!(snapshot(&a, &o.context), before);
    let p = a
        .prepare(
            batch(
                &o.context,
                vec![Change::Thickness {
                    id: PLATE.into(),
                    mm: 0.001,
                }],
            ),
            &control(),
        )
        .unwrap();
    o = a
        .install(
            &o.context,
            &p.token,
            &p.op_id,
            Decision::Accepted,
            &control(),
        )
        .unwrap();
    let mut bad = mesh();
    // One point above the array ceiling: (MODEL.array_items / 3 + 1) * 3 scalars.
    bad.points_mm = vec![[0.0; 3]; Budgets::MODEL.array_items / 3 + 1];
    assert_eq!(
        a.prepare(batch(&o.context, vec![Change::Mesh(bad)]), &control())
            .unwrap_err()
            .code,
        ErrorCode::LimitExceeded
    );
    let p = a
        .prepare(
            batch(
                &o.context,
                vec![Change::Label {
                    id: PLATE.into(),
                    name: "x".repeat(256),
                }],
            ),
            &control(),
        )
        .unwrap();
    a.discard(
        &o.context,
        &p.token,
        &p.op_id,
        Decision::Undecided,
        &control(),
    )
    .unwrap();
    assert_eq!(
        a.prepare(
            batch(
                &o.context,
                vec![Change::Label {
                    id: PLATE.into(),
                    name: "x".repeat(257)
                }]
            ),
            &control()
        )
        .unwrap_err()
        .code,
        ErrorCode::LimitExceeded
    );
    assert_eq!(
        a.install(
            &o.context,
            &p.token,
            &p.op_id,
            Decision::Accepted,
            &control()
        )
        .unwrap_err()
        .code,
        ErrorCode::StaleWorkspace
    );
}

#[test]
fn a_definition_only_batch_publishes_immutable_definitions_and_an_empty_batch_is_rejected() {
    let (mut a, o) = open(initial());
    assert_eq!(
        a.prepare(batch(&o.context, Vec::new()), &control())
            .unwrap_err()
            .code,
        ErrorCode::InvalidArgument
    );
    let definitions = resources();
    let mut input = batch(&o.context, Vec::new());
    input.resources = definitions.clone();
    let p = a.prepare(input, &control()).unwrap();
    // Two new definitions are written, no element is touched.
    let written: BTreeSet<String> = p.writes.iter().map(|f| f.identity.clone()).collect();
    let expected: BTreeSet<String> = definitions.iter().map(|d| d.digest.to_string()).collect();
    assert_eq!(written, expected);
    assert!(p.opinions.is_empty());
    let o = a
        .install(
            &o.context,
            &p.token,
            &p.op_id,
            Decision::Accepted,
            &control(),
        )
        .unwrap();
    let prims = snapshot(&a, &o.context);
    for d in &definitions {
        let definition = prims
            .iter()
            .find(|p| {
                p.path
                    == (if matches!(d.definition, Definition::Material { .. }) {
                        beamcraft_model::prim::DefinitionCategory::Material
                    } else {
                        beamcraft_model::prim::DefinitionCategory::Profile
                    })
                    .path(d.digest)
            })
            .unwrap();
        let property = |name: &str| {
            definition
                .properties
                .iter()
                .find(|p| p.name == name)
                .and_then(|p| p.value.as_ref())
        };
        assert_eq!(
            property("bim:contentDigest"),
            Some(&Value::Text(d.digest.to_string()))
        );
        assert_eq!(property("bim:units"), Some(&Value::Text("mm".into())));
    }
    // The same definitions again change nothing and write nothing: they are immutable.
    let mut again = batch(&o.context, Vec::new());
    again.resources = definitions;
    let p = a.prepare(again, &control()).unwrap();
    assert!(p.writes.is_empty());
}

fn holed_plate(holes: Vec<Vec<[f64; 3]>>) -> Change {
    Change::Create(Element {
        id: PLATE.into(),
        name: "Holed".into(),
        shape: Shape::Plate {
            contour_mm: vec![
                [0.0, 0.0, 0.0],
                [1000.0, 0.0, 0.0],
                [1000.0, 500.0, 0.0],
                [0.0, 500.0, 0.0],
            ],
            thickness_mm: 10.0,
            holes,
            cut: None,
        },
        placement: pose(),
        material: None,
    })
}

#[test]
fn through_hole_polygons_are_authored_as_flat_points_and_counts_and_validated() {
    let square = |x: f64, y: f64| {
        vec![
            [x, y, 0.0],
            [x, y + 100.0, 0.0],
            [x + 100.0, y + 100.0, 0.0],
            [x + 100.0, y, 0.0],
        ]
    };
    let (mut a, o) = open(initial());
    let holes = vec![square(100.0, 100.0), square(400.0, 200.0)];
    let p = a
        .prepare(
            batch(&o.context, vec![holed_plate(holes.clone())]),
            &control(),
        )
        .unwrap();
    let o = a
        .install(
            &o.context,
            &p.token,
            &p.op_id,
            Decision::Accepted,
            &control(),
        )
        .unwrap();
    let prims = snapshot(&a, &o.context);
    let Some(Value::Array(points)) = value(&prims, PLATE, "steel:plate:holePointsMm") else {
        panic!("hole points are missing");
    };
    assert_eq!(points.len(), 8);
    assert_eq!(
        value(&prims, PLATE, "steel:plate:holeVertexCounts"),
        Some(&Value::Array(vec![Value::Int(4), Value::Int(4)]))
    );
    // The exported layers pass the structural validator, so a reader can rely on the shape.
    let layers = a.export(&o.context, &control()).unwrap();
    let mut request = initial();
    request.assets = layers;
    request.manifest = request.manifest_digest().unwrap();
    validate_layers(request, schema_identity().unwrap(), &control()).unwrap();

    // A plate without holes authors neither attribute.
    let (mut b, ob) = open(initial());
    let p = b
        .prepare(
            batch(&ob.context, vec![holed_plate(Vec::new())]),
            &control(),
        )
        .unwrap();
    let ob = b
        .install(
            &ob.context,
            &p.token,
            &p.op_id,
            Decision::Accepted,
            &control(),
        )
        .unwrap();
    assert_eq!(
        value(
            &snapshot(&b, &ob.context),
            PLATE,
            "steel:plate:holePointsMm"
        ),
        None
    );

    // Invalid hole input is refused before anything is written.
    let (mut c, oc) = open(initial());
    let mut bad = |holes: Vec<Vec<[f64; 3]>>| {
        c.prepare(batch(&oc.context, vec![holed_plate(holes)]), &control())
            .map(|_| ())
            .unwrap_err()
            .code
    };
    assert_eq!(
        bad(vec![vec![[1.0, 1.0, 0.0], [2.0, 2.0, 0.0]]]),
        ErrorCode::InvalidArgument
    );
    assert_eq!(
        bad(vec![
            square(10.0, 10.0)
                .into_iter()
                .map(|[x, y, _]| [x, y, 1.0])
                .collect()
        ]),
        ErrorCode::InvalidArgument
    );
    assert_eq!(
        bad(vec![vec![
            [f64::NAN, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0]
        ]]),
        ErrorCode::NonFiniteNumber
    );
    assert_eq!(
        bad(vec![vec![
            [2e9, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0]
        ]]),
        ErrorCode::LimitExceeded
    );
    assert_eq!(bad(vec![square(0.0, 0.0); 65]), ErrorCode::LimitExceeded);
}

#[test]
fn the_plane_cut_is_authored_as_point_and_normal_and_validated() {
    let cut = |point: [f64; 3], normal: [f64; 3]| {
        Some(PlaneCut {
            point_mm: point,
            normal,
        })
    };
    let (mut a, o) = open(initial());
    let p = a
        .prepare(
            batch(
                &o.context,
                vec![Change::Create(Element {
                    id: PLATE.into(),
                    name: "Cut".into(),
                    shape: Shape::Plate {
                        contour_mm: vec![
                            [0.0, 0.0, 0.0],
                            [1000.0, 0.0, 0.0],
                            [1000.0, 500.0, 0.0],
                            [0.0, 500.0, 0.0],
                        ],
                        thickness_mm: 10.0,
                        holes: Vec::new(),
                        cut: cut([250.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
                    },
                    placement: pose(),
                    material: None,
                })],
            ),
            &control(),
        )
        .unwrap();
    let o = a
        .install(
            &o.context,
            &p.token,
            &p.op_id,
            Decision::Accepted,
            &control(),
        )
        .unwrap();
    let prims = snapshot(&a, &o.context);
    assert_eq!(
        value(&prims, PLATE, "steel:cut:pointMm"),
        Some(&Value::Array(vec![
            Value::Number(250.0),
            Value::Number(0.0),
            Value::Number(0.0)
        ]))
    );
    assert_eq!(
        value(&prims, PLATE, "steel:cut:normal"),
        Some(&Value::Array(vec![
            Value::Number(1.0),
            Value::Number(0.0),
            Value::Number(0.0)
        ]))
    );
    // The exported layers pass the structural validator, so a reader can rely on the shape.
    let layers = a.export(&o.context, &control()).unwrap();
    let mut request = initial();
    request.assets = layers;
    request.manifest = request.manifest_digest().unwrap();
    validate_layers(request, schema_identity().unwrap(), &control()).unwrap();

    // A cut on a member is authored the same way.
    let (mut b, ob) = open(initial());
    let mut member_batch = batch(
        &ob.context,
        vec![Change::Create(Element {
            id: MEMBER.into(),
            name: "Cut beam".into(),
            shape: Shape::Member {
                kind: MemberKind::Beam,
                axis_mm: [[0.0, 0.0, 0.0], [6000.0, 0.0, 0.0]],
                profile: resources()[0].digest,
                roll_rad: None,
                cut: cut([3000.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
            },
            placement: pose(),
            material: None,
        })],
    );
    member_batch.resources = resources();
    let p = b.prepare(member_batch, &control()).unwrap();
    let ob = b
        .install(
            &ob.context,
            &p.token,
            &p.op_id,
            Decision::Accepted,
            &control(),
        )
        .unwrap();
    let prims = snapshot(&b, &ob.context);
    assert!(value(&prims, MEMBER, "steel:cut:pointMm").is_some());
    assert!(value(&prims, MEMBER, "steel:cut:normal").is_some());

    // Invalid or half-authored cuts are refused before anything is written.
    let refuse = |shape: Shape| {
        let (mut c, oc) = open(initial());
        c.prepare(
            batch(
                &oc.context,
                vec![Change::Create(Element {
                    id: PLATE.into(),
                    name: "Bad".into(),
                    shape,
                    placement: pose(),
                    material: None,
                })],
            ),
            &control(),
        )
        .map(|_| ())
        .unwrap_err()
        .code
    };
    let base = |cut: Option<PlaneCut>| Shape::Plate {
        contour_mm: vec![
            [0.0, 0.0, 0.0],
            [1000.0, 0.0, 0.0],
            [1000.0, 500.0, 0.0],
            [0.0, 500.0, 0.0],
        ],
        thickness_mm: 10.0,
        holes: Vec::new(),
        cut,
    };
    // A zero normal is not a plane.
    assert_eq!(
        refuse(base(cut([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]))),
        ErrorCode::InvalidArgument
    );
    assert_eq!(
        refuse(base(cut([f64::NAN, 0.0, 0.0], [1.0, 0.0, 0.0]))),
        ErrorCode::NonFiniteNumber
    );
    assert_eq!(
        refuse(base(cut([2e9, 0.0, 0.0], [1.0, 0.0, 0.0]))),
        ErrorCode::LimitExceeded
    );
    // Only the point attribute is a damaged record, not "no cut".
    assert_eq!(
        refuse(Shape::Member {
            kind: MemberKind::Beam,
            axis_mm: [[0.0, 0.0, 0.0], [6000.0, 0.0, 0.0]],
            profile: resources()[0].digest,
            roll_rad: None,
            cut: cut([3000.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
        }),
        ErrorCode::InvalidArgument
    );
}

#[test]
fn a_definition_display_name_is_part_of_the_resource_and_is_checked_before_reuse() {
    let named = Resource::new(Definition::Material {
        density_kg_m3: Some(1000.0),
        grade: None,
        standard: None,
        display_name: Some("Steel".into()),
    })
    .unwrap();
    let unnamed = Resource::new(Definition::Material {
        density_kg_m3: Some(1000.0),
        grade: None,
        standard: None,
        display_name: None,
    })
    .unwrap();
    assert_ne!(named.digest, unnamed.digest);
    let text = String::from_utf8(named.bytes().unwrap()).unwrap();
    assert!(text.contains("displayName = \"Steel\""), "{text}");
    let hex = named.digest.to_string();
    let stage = |metadata: &str| {
        let body = format!(
            "def Xform \"World\" {{ def Scope \"Definitions\" {{ def Scope \"Materials\" {{ def Scope \"MAT_{hex}\" {metadata}{{ \
             custom string bim:schemaVersion = \"0.1.0\"\n custom token bim:units = \"mm\"\n \
             custom token bim:definitionKind = \"material\"\n custom double bim:densityKgM3 = 1000.0\n \
             custom string bim:contentDigest = \"{hex}\" }} }} }} }}"
        );
        request(
            vec![
                asset(
                    "root.usda",
                    "#usda 1.0\n(metersPerUnit=0.001\nupAxis=\"Z\"\nsubLayers=[@authored.usda@,@derived.usda@])\n",
                ),
                asset("authored.usda", layer(&body)),
                asset("derived.usda", layer("")),
            ],
            &["root.usda", "authored.usda", "derived.usda"],
        )
    };
    let prepare = |request: OpenRequest| {
        let (mut a, o) = open(request);
        let mut input = batch(&o.context, Vec::new());
        input.resources = vec![named.clone()];
        a.prepare(input, &control()).map(|p| p.writes.len())
    };
    assert_eq!(prepare(stage("(displayName = \"Steel\")\n")).unwrap(), 0);
    // A changed or removed name no longer matches the pinned content.
    assert!(prepare(stage("(displayName = \"Other\")\n")).is_err());
    assert!(prepare(stage("")).is_err());
}

#[test]
fn an_existing_definition_must_match_the_resource_before_it_is_reused() {
    let resource = Resource::new(Definition::Material {
        density_kg_m3: Some(1000.0),
        grade: None,
        standard: None,
        display_name: None,
    })
    .unwrap();
    let hex = resource.digest.to_string();
    let stage = |density: &str, active: &str| {
        let body = format!(
            "def Xform \"World\" {{ def Scope \"Definitions\" {{ def Scope \"Materials\" {{ def Scope \"MAT_{hex}\" {active}{{ \
             custom string bim:schemaVersion = \"0.1.0\"\n custom token bim:units = \"mm\"\n \
             custom token bim:definitionKind = \"material\"\n {density}\n \
             custom string bim:contentDigest = \"{hex}\" }} }} }} }}"
        );
        request(
            vec![
                asset(
                    "root.usda",
                    "#usda 1.0\n(metersPerUnit=0.001\nupAxis=\"Z\"\nsubLayers=[@authored.usda@,@derived.usda@])\n",
                ),
                asset("authored.usda", layer(&body)),
                asset("derived.usda", layer("")),
            ],
            &["root.usda", "authored.usda", "derived.usda"],
        )
    };
    let prepare = |request: OpenRequest| {
        let (mut a, o) = open(request);
        let mut input = batch(&o.context, Vec::new());
        input.resources = vec![resource.clone()];
        a.prepare(input, &control()).map(|p| p.writes.len())
    };
    // The same content is an immutable no-op.
    assert_eq!(
        prepare(stage("custom double bim:densityKgM3 = 1000.0", "")).unwrap(),
        0
    );
    // A stored density that disagrees with the path digest, a removed density, and a deactivated
    // definition are refused instead of being accepted as a no-op.
    for (density, active) in [
        ("custom double bim:densityKgM3 = 1.0", ""),
        ("", ""),
        ("custom float bim:densityKgM3 = 1000.0", ""),
        (
            "custom double bim:densityKgM3 = 1000.0",
            "(active = false)\n",
        ),
    ] {
        assert!(
            prepare(stage(density, active)).is_err(),
            "{density:?} {active:?}"
        );
    }
}

#[test]
fn replacing_a_mesh_drops_only_the_quantity_record_and_reports_every_removal() {
    let (mut a, o) = populated();
    let with = |quantities| Mesh {
        quantities,
        ..mesh()
    };
    let quantities = StoredQuantities {
        volume_mm3: 5.0e6,
        volume_status: "current".into(),
        mass_kg: Some(39.25),
        mass_status: "current".into(),
        shape_signature: digest(b"shape"),
        mass_signature: digest(b"mass"),
        algorithm: "test/v1".into(),
    };
    let p = a
        .prepare(
            batch(&o.context, vec![Change::Mesh(with(Some(quantities)))]),
            &control(),
        )
        .unwrap();
    let installed = a
        .install(
            &o.context,
            &p.token,
            &p.op_id,
            Decision::Accepted,
            &control(),
        )
        .unwrap();

    // Unknown content that another tool put under the mesh prim.
    let mut assets = a.export(&installed.context, &control()).unwrap();
    let derived = assets
        .iter_mut()
        .find(|x| x.name == "derived.usda")
        .unwrap();
    let text = String::from_utf8(derived.bytes.clone()).unwrap();
    let at = text.find("\"Geometry\"").unwrap();
    let brace = at + text[at..].find('{').unwrap() + 1;
    let edited = format!(
        "{}\ncustom double future:note = 5\n{}",
        &text[..brace],
        &text[brace..]
    );
    *derived = asset("derived.usda", edited);
    let (mut b, o) = open(request(
        assets,
        &["root.usda", "authored.usda", "derived.usda", "weak.usda"],
    ));

    // A replacement without a record removes all seven fields and says so.
    let p = b
        .prepare(
            batch(&o.context, vec![Change::Mesh(with(None))]),
            &control(),
        )
        .unwrap();
    let removed: std::collections::BTreeSet<&str> = p
        .opinions
        .iter()
        .filter(|i| i.opinion == Opinion::Remove)
        .map(|i| i.attribute)
        .collect();
    for name in [
        "bim:volumeMm3",
        "bim:volumeStatus",
        "bim:massStatus",
        "bim:shapeSignature",
        "bim:massSignature",
        "bim:quantityAlgorithm",
        "bim:massKg",
    ] {
        assert!(removed.contains(name), "{name}");
    }
    let layer = p.layers.iter().find(|x| x.name == "derived.usda").unwrap();
    let text = String::from_utf8(layer.bytes.clone()).unwrap();
    assert!(text.contains("future:note"));
    assert!(!text.contains("bim:volumeMm3") && !text.contains("bim:massKg"));
}

#[test]
fn delete_removes_the_whole_element_in_both_layers_and_nothing_else() {
    let (mut a, o) = populated();
    let before = snapshot(&a, &o.context);
    assert!(
        before
            .iter()
            .any(|p| p.element_id.as_deref() == Some(PLATE))
    );
    let prepared = a
        .prepare(
            batch(&o.context, vec![Change::Delete { id: PLATE.into() }]),
            &control(),
        )
        .unwrap();
    // The footprint is the whole element: every group is written, the lifecycle and the
    // relationships are read.
    let groups = |set: &std::collections::BTreeSet<Footprint>| {
        set.iter()
            .map(|f| (f.identity.clone(), f.group))
            .collect::<std::collections::BTreeSet<_>>()
    };
    assert_eq!(prepared.writes.len(), 7);
    assert!(
        groups(&prepared.writes)
            .iter()
            .all(|(identity, _)| identity == PLATE)
    );
    assert!(prepared.reads.iter().any(|f| f.group == Group::Lifecycle));
    assert!(
        prepared
            .reads
            .iter()
            .any(|f| f.group == Group::Relationship)
    );
    // Every supported field of the element is reported as removed.
    assert!(
        prepared
            .opinions
            .iter()
            .all(|i| i.element_id == PLATE && i.opinion == Opinion::Remove)
    );
    let installed = a
        .install(
            &o.context,
            &prepared.token,
            &prepared.op_id,
            Decision::Accepted,
            &control(),
        )
        .unwrap();
    let after = snapshot(&a, &installed.context);
    assert!(!after.iter().any(|p| p.element_id.as_deref() == Some(PLATE)));
    assert!(!after.iter().any(|p| p.path.starts_with(&path(PLATE))));
    // The other element and the unknown content are exactly as they were.
    let member = |prims: &[Prim]| {
        prims
            .iter()
            .filter(|p| p.element_id.as_deref() == Some(MEMBER))
            .map(|p| Prim {
                sources: Vec::new(),
                ..p.clone()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(member(&after), member(&before));
    let exported = a.export(&installed.context, &control()).unwrap();
    let authored = exported.iter().find(|a| a.name == "authored.usda").unwrap();
    let parsed = openusd::usda::parse(std::str::from_utf8(&authored.bytes).unwrap()).unwrap();
    assert!(parsed.has_spec(&openusd::sdf::Path::new("/Unknown.future:link").unwrap()));
    assert!(!parsed.has_spec(&openusd::sdf::Path::new(&path(PLATE)).unwrap()));
    let derived = exported.iter().find(|a| a.name == "derived.usda").unwrap();
    let parsed = openusd::usda::parse(std::str::from_utf8(&derived.bytes).unwrap()).unwrap();
    assert!(
        !parsed.has_spec(&openusd::sdf::Path::new(&format!("{}/Geometry", path(PLATE))).unwrap())
    );
    let request = request(
        exported,
        &["root.usda", "authored.usda", "derived.usda", "weak.usda"],
    );
    assert!(validate_layers(request, schema_identity().unwrap(), &control()).is_ok());

    // The element is gone, so a second delete is not found, and a delete cannot share a batch
    // with another change of the same element.
    assert_eq!(
        a.prepare(
            batch(
                &installed.context,
                vec![Change::Delete { id: PLATE.into() }]
            ),
            &control()
        )
        .unwrap_err()
        .code,
        ErrorCode::NotFound
    );
    let (mut b, o) = populated();
    assert_eq!(
        b.prepare(
            batch(
                &o.context,
                vec![
                    Change::Delete { id: MEMBER.into() },
                    Change::Label {
                        id: MEMBER.into(),
                        name: "x".into()
                    }
                ]
            ),
            &control()
        )
        .unwrap_err()
        .code,
        ErrorCode::MalformedCommand
    );
}

#[test]
fn one_pass_open_validated_answers_like_validate_layers_then_open() {
    let (a, o) = populated();
    let schema = schema_identity().unwrap();
    let separate = |input: &OpenRequest| {
        validate_layers(input.clone(), schema, &control())?;
        Reader::default().open(input.clone(), &control())
    };
    // A valid stage: the same composed snapshot.
    let valid = export_request(&a, &o.context);
    let mut reader = Reader::default();
    let (joined, validation) = reader
        .open_validated(valid.clone(), schema, &control())
        .unwrap();
    let alone = separate(&valid).unwrap();
    assert_eq!(
        validation,
        validate_layers(valid.clone(), schema, &control()).unwrap()
    );
    assert_eq!(
        (joined.prim_count, &joined.metadata),
        (alone.prim_count, &alone.metadata)
    );
    let page = |reader: &Reader, context: &Context| {
        reader
            .read(context, None, &control())
            .unwrap()
            .items
            .into_iter()
            .map(|p| p.path)
            .collect::<Vec<_>>()
    };
    let paths = page(&reader, &joined.context);
    // Broken stages: the same code and message, and the live view stays the earlier one.
    let digest_text = schema.digest.to_string();
    let incorrect = digest(b"incorrect schema").to_string();
    for (layer, from, to) in [
        (
            "authored.usda",
            "steel:profile:widthMm = 100.0",
            "steel:profile:widthMm = 101.0",
        ),
        ("derived.usda", "[0, 1, 2, 3]", "[0, 1, 2, 999]"),
        ("root.usda", digest_text.as_str(), incorrect.as_str()),
        ("authored.usda", "#usda 1.0", "#usda 9.9"),
    ] {
        let input = tampered(&a, &o.context, layer, from, to);
        let refused = reader
            .open_validated(input.clone(), schema, &control())
            .unwrap_err();
        assert_eq!(refused, separate(&input).unwrap_err(), "{layer}: {from}");
        assert_eq!(page(&reader, &joined.context), paths);
    }
    // A schema pin that is not admitted is refused before anything is parsed, as by the checker.
    let mut unknown = schema;
    unknown.digest = digest(b"another schema");
    assert_eq!(
        reader
            .open_validated(valid.clone(), unknown, &control())
            .unwrap_err(),
        validate_layers(valid, unknown, &control()).unwrap_err()
    );
}

fn prim<'a>(prims: &'a [Prim], path: &str) -> &'a Prim {
    prims.iter().find(|p| p.path == path).unwrap()
}
fn export_request(a: &Author, context: &Context) -> OpenRequest {
    request(
        a.export(context, &control()).unwrap(),
        &["root.usda", "authored.usda", "derived.usda", "weak.usda"],
    )
}
fn tampered(a: &Author, context: &Context, layer: &str, from: &str, to: &str) -> OpenRequest {
    let mut input = export_request(a, context);
    let asset = input.assets.iter_mut().find(|a| a.name == layer).unwrap();
    let text = String::from_utf8(asset.bytes.clone()).unwrap();
    assert!(text.contains(from), "{from} not found in {text}");
    *asset = self::asset(layer, text.replacen(from, to, 1));
    input.manifest = input.manifest_digest().unwrap();
    input
}

#[test]
fn standard_usd_metadata_kind_display_name_extent_and_flat_normals() {
    let (mut a, o) = populated();
    let all = snapshot(&a, &o.context);
    assert_eq!(prim(&all, "/World").kind.as_deref(), Some("assembly"));
    assert_eq!(prim(&all, "/World/Steel").kind.as_deref(), Some("group"));
    assert_eq!(
        prim(&all, "/World/Steel/Parts").kind.as_deref(),
        Some("group")
    );
    let plate = prim(&all, &path(PLATE));
    assert_eq!(plate.kind.as_deref(), Some("component"));
    assert_eq!(plate.display_name.as_deref(), Some("Plate Ω"));
    let geometry = prim(&all, &format!("{}/Geometry", path(PLATE)));
    assert_eq!(geometry.kind.as_deref(), Some("subcomponent"));
    let v3 = |x: f64, y: f64, z: f64| {
        Value::Array(vec![Value::Number(x), Value::Number(y), Value::Number(z)])
    };
    assert_eq!(
        value_at(geometry, "extent"),
        Some(&Value::Array(vec![
            v3(0.0, 0.0, 0.0),
            v3(1000.0, 500.0, 0.0)
        ]))
    );
    assert_eq!(
        value_at(geometry, "normals"),
        Some(&Value::Array(vec![v3(0.0, 0.0, 1.0)]))
    );

    // A rename changes the display name only: path, identity and the mesh stay.
    let mut input = batch(
        &o.context,
        vec![Change::Label {
            id: PLATE.into(),
            name: "Renamed".into(),
        }],
    );
    input.op_id = "0199e7a0-0000-7000-8000-000000000007".into();
    let p = a.prepare(input, &control()).unwrap();
    let renamed = a
        .install(
            &o.context,
            &p.token,
            &p.op_id,
            Decision::Accepted,
            &control(),
        )
        .unwrap();
    let after = snapshot(&a, &renamed.context);
    let plate = prim(&after, &path(PLATE));
    assert_eq!(plate.display_name.as_deref(), Some("Renamed"));
    assert_eq!(plate.element_id.as_deref(), Some(PLATE));
    assert_eq!(
        prim(&after, &format!("{}/Geometry", path(PLATE))),
        prim(&all, &format!("{}/Geometry", path(PLATE)))
    );
    assert_eq!(
        p.layers.iter().filter(|l| l.name == "derived.usda").count(),
        1
    );
    assert!(
        validate_layers(
            export_request(&a, &renamed.context),
            schema_identity().unwrap(),
            &control()
        )
        .is_ok()
    );
}

#[test]
fn validator_refuses_a_wrong_display_name_kind_extent_or_normal() {
    let (a, o) = populated();
    for (layer, from, to) in [
        (
            "authored.usda",
            "displayName = \"Plate Ω\"",
            "displayName = \"Other\"",
        ),
        ("authored.usda", "kind = \"component\"", "kind = \"group\""),
        (
            "derived.usda",
            "kind = \"subcomponent\"",
            "kind = \"component\"",
        ),
        (
            "derived.usda",
            "(1000.0, 500.0, 0.0)]",
            "(1000.0, 500.0, 1.0)]",
        ),
        ("derived.usda", "[(0.0, 0.0, 1.0)]", "[(0.0, 0.0, -1.0)]"),
        (
            "derived.usda",
            "interpolation = \"uniform\"",
            "interpolation = \"vertex\"",
        ),
        (
            "derived.usda",
            "interpolation = \"uniform\"",
            "doc = \"no interpolation\"",
        ),
    ] {
        let input = tampered(&a, &o.context, layer, from, to);
        assert!(
            validate_layers(input, schema_identity().unwrap(), &control()).is_err(),
            "{from}"
        );
    }
}
fn value_at<'a>(prim: &'a Prim, name: &str) -> Option<&'a Value> {
    prim.properties
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| p.value.as_ref())
}

#[test]
fn a_store_pinned_to_the_previous_mapping_is_refused() {
    let previous = "70fc49a1d81314a6599284532bd2d45c18f77b4318266238cc6923b25c05f28e";
    let pinned = |body: &str| {
        format!(
            "#usda 1.0\n(metersPerUnit=0.001\nupAxis=\"Z\"\ncustomLayerData={{string bimSchemaDigest=\"{previous}\"}})\n{body}\n"
        )
    };
    let mut input = initial();
    for name in ["authored.usda", "derived.usda"] {
        let old = input.assets.iter_mut().find(|a| a.name == name).unwrap();
        let text = String::from_utf8(old.bytes.clone()).unwrap();
        let body = text.split_once(")\n").unwrap().1.to_owned();
        *old = asset(name, pinned(&body));
    }
    input.manifest = input.manifest_digest().unwrap();
    assert_eq!(
        Author::open(input, targets(), schema_identity().unwrap(), &control())
            .err()
            .map(|e| e.code),
        Some(ErrorCode::UnsupportedVersion)
    );
}

const FRAME: &str = "0199e7a0-0000-7000-8000-000000000101";
const OUTER: &str = "0199e7a0-0000-7000-8000-000000000102";
fn assembly(id: &str, name: &str) -> Change {
    Change::Create(Element {
        id: id.into(),
        name: name.into(),
        shape: Shape::Assembly,
        placement: Placement {
            translation_mm: [0.0; 3],
            orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
        },
        material: None,
    })
}
fn part_of(id: &str, assembly: Option<&str>) -> Change {
    Change::PartOf {
        id: id.into(),
        assembly: assembly.map(str::to_owned),
    }
}
fn targets_of(prims: &[Prim], id: &str) -> Option<Vec<String>> {
    prim(prims, &path(id))
        .relationships
        .iter()
        .find(|r| r.name == "bim:partOf")
        .map(|r| r.targets.clone())
}
fn installed(a: &mut Author, o: &Opened, changes: Vec<Change>) -> (Prepared, Opened) {
    let p = a.prepare(batch(&o.context, changes), &control()).unwrap();
    let next = a
        .install(
            &o.context,
            &p.token,
            &p.op_id,
            Decision::Accepted,
            &control(),
        )
        .unwrap();
    (p, next)
}

#[test]
fn assemblies_and_part_of_records_are_authored_on_the_part_and_validated() {
    let (mut a, o) = populated();
    let (p, o) = installed(
        &mut a,
        &o,
        vec![
            assembly(FRAME, "Frame"),
            assembly(OUTER, "Outer"),
            part_of(PLATE, Some(FRAME)),
            part_of(MEMBER, Some(FRAME)),
            part_of(FRAME, Some(OUTER)),
        ],
    );
    let prims = snapshot(&a, &o.context);
    let frame = prim(&prims, &path(FRAME));
    assert_eq!(frame.kind.as_deref(), Some("assembly"));
    assert_eq!(frame.display_name.as_deref(), Some("Frame"));
    assert_eq!(
        value(&prims, FRAME, "bim:elementKind"),
        Some(&Value::Text("assembly".into()))
    );
    // No body, no shape inputs, no material: an assembly is not a part with geometry.
    assert!(
        !prims
            .iter()
            .any(|p| p.path == format!("{}/Geometry", path(FRAME)))
    );
    assert!(value(&prims, FRAME, "steel:plate:thicknessMm").is_none());
    assert_eq!(targets_of(&prims, PLATE), Some(vec![path(FRAME)]));
    assert_eq!(targets_of(&prims, FRAME), Some(vec![path(OUTER)]));
    assert_eq!(targets_of(&prims, OUTER), None);
    // The record lives with its source: the part's relationship group is written, the
    // assembly is only read.
    let plate_writes: BTreeSet<_> = p
        .writes
        .iter()
        .filter(|f| f.identity == PLATE)
        .map(|f| f.group)
        .collect();
    assert_eq!(plate_writes, BTreeSet::from([Group::Relationship]));
    assert!(p.validation.rules.iter().any(|r| r.starts_with("A01-V01")));

    let refused = |a: &mut Author, o: &Opened, changes: Vec<Change>| {
        let digests = |a: &Author| {
            a.export(&o.context, &control())
                .unwrap()
                .iter()
                .map(|l| l.digest)
                .collect::<Vec<_>>()
        };
        let before = digests(a);
        let code = a
            .prepare(batch(&o.context, changes), &control())
            .err()
            .map(|e| e.code);
        assert_eq!(digests(a), before);
        code
    };
    // A cycle of length two (OUTER -> FRAME -> OUTER).
    assert_eq!(
        refused(&mut a, &o, vec![part_of(OUTER, Some(FRAME))]),
        Some(ErrorCode::DependencyConflict)
    );
    // A cycle of length one is the self-link rule.
    assert_eq!(
        refused(&mut a, &o, vec![part_of(OUTER, Some(OUTER))]),
        Some(ErrorCode::InvalidArgument)
    );
    // The target must be an assembly, and it must exist.
    assert_eq!(
        refused(&mut a, &o, vec![part_of(PLATE, Some(MEMBER))]),
        Some(ErrorCode::InvalidArgument)
    );
    assert_eq!(
        refused(
            &mut a,
            &o,
            vec![part_of(PLATE, Some("0199e7a0-0000-7000-8000-0000000001ff"))]
        ),
        Some(ErrorCode::DanglingRelationship)
    );
    // An assembly has no shape and no material, and an identity placement in this slice.
    let unsupported = refused(
        &mut a,
        &o,
        vec![Change::Thickness {
            id: FRAME.into(),
            mm: 5.0,
        }],
    );
    assert_eq!(unsupported, Some(ErrorCode::UnsupportedProperty));
    let mut moved = assembly("0199e7a0-0000-7000-8000-000000000103", "Moved");
    if let Change::Create(e) = &mut moved {
        e.placement.translation_mm = [1.0, 0.0, 0.0];
    }
    assert_eq!(
        refused(&mut a, &o, vec![moved]),
        Some(ErrorCode::UnsupportedProperty)
    );

    // Two targets on one part break the cardinality of loaded content.
    let doubled = tampered(
        &a,
        &o.context,
        "authored.usda",
        &format!("rel bim:partOf = <{}>", path(OUTER)),
        &format!("rel bim:partOf = [<{}>, <{}>]", path(OUTER), path(OUTER)),
    );
    assert_eq!(
        validate_layers(doubled, schema_identity().unwrap(), &control())
            .err()
            .map(|e| e.code),
        Some(ErrorCode::DependencyConflict)
    );

    // Detach authors an explicit empty record; the assembly stays.
    let (_, o) = installed(&mut a, &o, vec![part_of(PLATE, None)]);
    let prims = snapshot(&a, &o.context);
    assert_eq!(targets_of(&prims, PLATE), Some(vec![]));
    assert_eq!(targets_of(&prims, MEMBER), Some(vec![path(FRAME)]));
    assert!(prims.iter().any(|p| p.path == path(FRAME)));
}

#[test]
fn an_assembly_never_gets_a_body_reads_cover_the_chain_and_detach_masks_weaker_records() {
    let (mut a, o) = populated();
    // Create and mesh in one batch: the existing-element kind guard does not apply, the
    // candidate check must.
    let mut body = mesh();
    body.id = FRAME.into();
    let refused = a
        .prepare(
            batch(
                &o.context,
                vec![assembly(FRAME, "Frame"), Change::Mesh(body)],
            ),
            &control(),
        )
        .err()
        .map(|e| e.code);
    assert_eq!(refused, Some(ErrorCode::UnsupportedProperty));

    let (_, o) = installed(
        &mut a,
        &o,
        vec![
            assembly(FRAME, "Frame"),
            assembly(OUTER, "Outer"),
            part_of(FRAME, Some(OUTER)),
        ],
    );
    // Attaching to FRAME reads the whole chain FRAME -> OUTER.
    let p = a
        .prepare(
            batch(&o.context, vec![part_of(PLATE, Some(FRAME))]),
            &control(),
        )
        .unwrap();
    for id in [FRAME, OUTER] {
        for group in [Group::Lifecycle, Group::Relationship] {
            assert!(
                p.reads.contains(&Footprint {
                    identity: id.into(),
                    group
                }),
                "{id} {group:?}"
            );
        }
    }
    a.discard(
        &o.context,
        &p.token,
        &p.op_id,
        Decision::Undecided,
        &control(),
    )
    .unwrap();

    // A weaker owned layer supplies the membership; a detach must still end it.
    let mut weak = export_request(&a, &o.context);
    let text = format!(
        "over \"World\" {{ over \"Steel\" {{ over \"Parts\" {{ over \"PRT_{}\" {{ custom rel bim:partOf = <{}> }} }} }} }}",
        PLATE.replace('-', ""),
        path(FRAME)
    );
    *weak
        .assets
        .iter_mut()
        .find(|l| l.name == "weak.usda")
        .unwrap() = asset("weak.usda", layer(&text));
    weak.manifest = weak.manifest_digest().unwrap();
    let (mut a, o) = open(weak);
    assert_eq!(
        targets_of(&snapshot(&a, &o.context), PLATE),
        Some(vec![path(FRAME)])
    );
    let (_, o) = installed(&mut a, &o, vec![part_of(PLATE, None)]);
    assert_eq!(targets_of(&snapshot(&a, &o.context), PLATE), Some(vec![]));
}

const WELD: &str = "0199e7a0-0000-7000-8000-000000000201";
fn weld(main: &str, secondary: &str) -> Weld {
    Weld {
        connects: [main.into(), secondary.into()],
        weld_type: "fillet".into(),
        size_mm: 6.0,
        location: "workshop".into(),
        side: "both".into(),
    }
}
fn create_weld(id: &str, w: Weld) -> Change {
    Change::Create(Element {
        id: id.into(),
        name: "W1".into(),
        shape: Shape::Weld(w),
        placement: Placement {
            translation_mm: [0.0; 3],
            orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
        },
        material: None,
    })
}
fn refused_code(a: &mut Author, o: &Opened, changes: Vec<Change>) -> Option<ErrorCode> {
    let digests = |a: &Author| {
        a.export(&o.context, &control())
            .unwrap()
            .iter()
            .map(|l| l.digest)
            .collect::<Vec<_>>()
    };
    let before = digests(a);
    let code = a
        .prepare(batch(&o.context, changes), &control())
        .err()
        .map(|e| e.code);
    assert_eq!(digests(a), before);
    code
}

#[test]
fn a_weld_authors_its_properties_and_connects_record_and_writes_only_itself() {
    let (mut a, o) = populated();
    let (p, o) = installed(&mut a, &o, vec![create_weld(WELD, weld(PLATE, MEMBER))]);
    let prims = snapshot(&a, &o.context);
    let w = prim(&prims, &path(WELD));
    assert_eq!(w.kind.as_deref(), Some("component"));
    assert_eq!(
        value(&prims, WELD, "bim:elementKind"),
        Some(&Value::Text("connection".into()))
    );
    for (name, token) in [
        ("steel:weld:type", "fillet"),
        ("steel:weld:location", "workshop"),
        ("steel:weld:side", "both"),
    ] {
        assert_eq!(value(&prims, WELD, name), Some(&Value::Text(token.into())));
    }
    assert_eq!(
        value(&prims, WELD, "steel:weld:sizeMm"),
        Some(&Value::Number(6.0))
    );
    // The record lives with the weld, in order: main part first.
    let connects = w
        .relationships
        .iter()
        .find(|r| r.name == "bim:connects")
        .map(|r| r.targets.clone());
    assert_eq!(connects, Some(vec![path(PLATE), path(MEMBER)]));
    // No body: a weld is not solid geometry in this slice.
    assert!(
        !prims
            .iter()
            .any(|p| p.path == format!("{}/Geometry", path(WELD)))
    );
    // Only the weld is written; both parts are read (existence and kind).
    assert!(p.writes.iter().all(|f| f.identity == WELD));
    for part in [PLATE, MEMBER] {
        assert!(p.reads.contains(&Footprint {
            identity: part.into(),
            group: Group::Lifecycle,
        }));
    }
    assert!(p.validation.rules.iter().any(|r| r.starts_with("A06-V01")));
    // Every authored weld field is reported.
    for name in [
        "steel:weld:type",
        "steel:weld:sizeMm",
        "steel:weld:location",
        "steel:weld:side",
        "bim:connects",
    ] {
        assert!(
            p.opinions
                .iter()
                .any(|i| i.element_id == WELD && i.attribute == name),
            "{name}"
        );
    }

    // A weld can be renamed; its shape, placement and membership are not edits of this slice.
    let (_, o) = installed(
        &mut a,
        &o,
        vec![Change::Label {
            id: WELD.into(),
            name: "W2".into(),
        }],
    );
    for change in [
        Change::Thickness {
            id: WELD.into(),
            mm: 5.0,
        },
        Change::Placement {
            id: WELD.into(),
            placement: Placement {
                translation_mm: [1.0, 0.0, 0.0],
                orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
            },
        },
        Change::PartOf {
            id: WELD.into(),
            assembly: None,
        },
    ] {
        assert_eq!(
            refused_code(&mut a, &o, vec![change]),
            Some(ErrorCode::UnsupportedProperty)
        );
    }

    // A welded part cannot be deleted while the weld connects it (DV-REL-009); the weld itself
    // can, and then the part can.
    let index = a.element_index(&o.context, &control()).unwrap();
    let plate = index::ElementId::parse(PLATE).unwrap();
    assert_eq!(
        a.check_delete(&index, &plate, &control())
            .err()
            .map(|e| e.code),
        Some(ErrorCode::DependencyConflict)
    );
    // A weld has element links, so it is not copied incompletely (DV-REL-011).
    let fresh = index::ElementId::parse("0199e7a0-0000-7000-8000-0000000002ff").unwrap();
    assert_eq!(
        index
            .copy_plan(&index::ElementId::parse(WELD).unwrap(), &fresh)
            .err()
            .map(|e| e.code),
        Some(ErrorCode::DependencyConflict)
    );
    // The welded part itself copies: the incoming record stays on the original.
    assert!(index.copy_plan(&plate, &fresh).is_ok());
    let (_, o) = installed(&mut a, &o, vec![Change::Delete { id: WELD.into() }]);
    let index = a.element_index(&o.context, &control()).unwrap();
    a.check_delete(&index, &plate, &control()).unwrap();
}

#[test]
fn wrong_weld_inputs_targets_and_loaded_records_are_refused() {
    let (mut a, o) = populated();
    let (_, o) = installed(&mut a, &o, vec![assembly(FRAME, "Frame")]);
    let with = |f: &dyn Fn(&mut Weld)| {
        let mut w = weld(PLATE, MEMBER);
        f(&mut w);
        vec![create_weld(WELD, w)]
    };
    let cases: Vec<(Vec<Change>, ErrorCode)> = vec![
        (
            with(&|w| w.connects[1] = PLATE.into()),
            ErrorCode::InvalidArgument,
        ),
        (
            with(&|w| w.connects[0] = WELD.into()),
            ErrorCode::InvalidArgument,
        ),
        (
            with(&|w| w.connects[1] = "0199e7a0-0000-7000-8000-0000000002fe".into()),
            ErrorCode::DanglingRelationship,
        ),
        (
            with(&|w| w.connects[1] = FRAME.into()),
            ErrorCode::UnsupportedProperty,
        ),
        (
            with(&|w| w.connects[1] = "not-a-uuid".into()),
            ErrorCode::InvalidId,
        ),
        (
            with(&|w| w.weld_type = "butt".into()),
            ErrorCode::UnsupportedProperty,
        ),
        (
            with(&|w| w.side = "left".into()),
            ErrorCode::InvalidArgument,
        ),
        (
            with(&|w| w.location = "".into()),
            ErrorCode::InvalidArgument,
        ),
        (with(&|w| w.size_mm = 0.0), ErrorCode::InvalidArgument),
        (with(&|w| w.size_mm = 1000.5), ErrorCode::LimitExceeded),
        (with(&|w| w.size_mm = f64::NAN), ErrorCode::NonFiniteNumber),
    ];
    for (changes, code) in cases {
        assert_eq!(
            refused_code(&mut a, &o, changes.clone()),
            Some(code),
            "{changes:?}"
        );
    }
    // A weld has no material and no body; its placement is the identity.
    let mut placed = create_weld(WELD, weld(PLATE, MEMBER));
    if let Change::Create(e) = &mut placed {
        e.placement.translation_mm = [0.0, 0.0, 1.0];
    }
    assert_eq!(
        refused_code(&mut a, &o, vec![placed]),
        Some(ErrorCode::UnsupportedProperty)
    );
    let mut body = mesh();
    body.id = WELD.into();
    assert_eq!(
        refused_code(
            &mut a,
            &o,
            vec![create_weld(WELD, weld(PLATE, MEMBER)), Change::Mesh(body)]
        ),
        Some(ErrorCode::UnsupportedProperty)
    );

    // Loaded content: one target breaks the cardinality, and a connects record on a part is
    // outside the vocabulary.
    let (_, o) = installed(&mut a, &o, vec![create_weld(WELD, weld(PLATE, MEMBER))]);
    let from = format!("rel bim:connects = [<{}>, <{}>]", path(PLATE), path(MEMBER));
    for (to, code) in [
        (
            format!("rel bim:connects = <{}>", path(PLATE)),
            ErrorCode::DependencyConflict,
        ),
        (
            format!("rel bim:connects = [<{}>, <{}>]", path(PLATE), path(PLATE)),
            ErrorCode::InvalidArgument,
        ),
        (
            format!("rel bim:connects = [<{}>, <{}>]", path(PLATE), path(FRAME)),
            ErrorCode::UnsupportedProperty,
        ),
    ] {
        let input = tampered(&a, &o.context, "authored.usda", &from, &to);
        assert_eq!(
            validate_layers(input, schema_identity().unwrap(), &control())
                .err()
                .map(|e| e.code),
            Some(code),
            "{to}"
        );
    }
    let input = tampered(
        &a,
        &o.context,
        "authored.usda",
        "token steel:weld:side = \"both\"",
        "token steel:weld:side = \"left\"",
    );
    assert_eq!(
        validate_layers(input, schema_identity().unwrap(), &control())
            .err()
            .map(|e| e.code),
        Some(ErrorCode::InvalidArgument)
    );
}

const BOLT: &str = "0199e7a0-0000-7000-8000-000000000301";
fn bolt(main: &str, secondary: &str) -> Bolt {
    Bolt {
        connects: [main.into(), secondary.into()],
        diameter_mm: 16.0,
        count: 4,
        pitch_mm: 80.0,
        origin_mm: [100.0, 250.0, 5.0],
        direction_mm: [1.0, 0.0, 0.0],
    }
}
fn create_bolt(id: &str, b: Bolt) -> Change {
    Change::Create(Element {
        id: id.into(),
        name: "B1".into(),
        shape: Shape::Bolt(b),
        placement: Placement {
            translation_mm: [0.0; 3],
            orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
        },
        material: None,
    })
}

#[test]
fn a_bolt_group_authors_its_properties_and_connects_record_and_writes_only_itself() {
    let (mut a, o) = populated();
    let (p, o) = installed(&mut a, &o, vec![create_bolt(BOLT, bolt(PLATE, MEMBER))]);
    let prims = snapshot(&a, &o.context);
    let b = prim(&prims, &path(BOLT));
    assert_eq!(b.kind.as_deref(), Some("component"));
    assert_eq!(
        value(&prims, BOLT, "bim:elementKind"),
        Some(&Value::Text("connection".into()))
    );
    assert_eq!(
        value(&prims, BOLT, "steel:bolt:diameterMm"),
        Some(&Value::Number(16.0))
    );
    assert_eq!(
        value(&prims, BOLT, "steel:bolt:count"),
        Some(&Value::Int(4))
    );
    assert_eq!(
        value(&prims, BOLT, "steel:bolt:pitchMm"),
        Some(&Value::Number(80.0))
    );
    assert_eq!(
        value(&prims, BOLT, "steel:bolt:originMm"),
        Some(&Value::Array(vec![
            Value::Number(100.0),
            Value::Number(250.0),
            Value::Number(5.0)
        ]))
    );
    assert_eq!(
        value(&prims, BOLT, "steel:bolt:directionMm"),
        Some(&Value::Array(vec![
            Value::Number(1.0),
            Value::Number(0.0),
            Value::Number(0.0)
        ]))
    );
    // The record lives with the group, in order: main part first.
    let connects = b
        .relationships
        .iter()
        .find(|r| r.name == "bim:connects")
        .map(|r| r.targets.clone());
    assert_eq!(connects, Some(vec![path(PLATE), path(MEMBER)]));
    // No body: a bolt group is an annotation in this slice.
    assert!(
        !prims
            .iter()
            .any(|p| p.path == format!("{}/Geometry", path(BOLT)))
    );
    // Only the group is written; both parts are read (existence and kind).
    assert!(p.writes.iter().all(|f| f.identity == BOLT));
    for part in [PLATE, MEMBER] {
        assert!(p.reads.contains(&Footprint {
            identity: part.into(),
            group: Group::Lifecycle,
        }));
    }
    assert!(p.validation.rules.iter().any(|r| r.starts_with("A05-V01")));
    // Every authored bolt field is reported.
    for name in [
        "steel:bolt:diameterMm",
        "steel:bolt:count",
        "steel:bolt:pitchMm",
        "steel:bolt:originMm",
        "steel:bolt:directionMm",
        "bim:connects",
    ] {
        assert!(
            p.opinions
                .iter()
                .any(|i| i.element_id == BOLT && i.attribute == name),
            "{name}"
        );
    }

    // A bolted part cannot be deleted while the group connects it (DV-REL-009); the group
    // itself can, and then the part can. A group has links, so it is not copied (DV-REL-011).
    let index = a.element_index(&o.context, &control()).unwrap();
    let plate = index::ElementId::parse(PLATE).unwrap();
    assert_eq!(
        a.check_delete(&index, &plate, &control())
            .err()
            .map(|e| e.code),
        Some(ErrorCode::DependencyConflict)
    );
    let fresh = index::ElementId::parse("0199e7a0-0000-7000-8000-0000000003ff").unwrap();
    assert_eq!(
        index
            .copy_plan(&index::ElementId::parse(BOLT).unwrap(), &fresh)
            .err()
            .map(|e| e.code),
        Some(ErrorCode::DependencyConflict)
    );
    assert!(index.copy_plan(&plate, &fresh).is_ok());
    let (_, o) = installed(&mut a, &o, vec![Change::Delete { id: BOLT.into() }]);
    let index = a.element_index(&o.context, &control()).unwrap();
    a.check_delete(&index, &plate, &control()).unwrap();
}

#[test]
fn wrong_bolt_inputs_targets_and_loaded_records_are_refused() {
    let (mut a, o) = populated();
    let (_, o) = installed(&mut a, &o, vec![assembly(FRAME, "Frame")]);
    let with = |f: &dyn Fn(&mut Bolt)| {
        let mut b = bolt(PLATE, MEMBER);
        f(&mut b);
        vec![create_bolt(BOLT, b)]
    };
    let cases: Vec<(Vec<Change>, ErrorCode)> = vec![
        (
            with(&|b| b.connects[1] = PLATE.into()),
            ErrorCode::InvalidArgument,
        ),
        (
            with(&|b| b.connects[0] = BOLT.into()),
            ErrorCode::InvalidArgument,
        ),
        (
            with(&|b| b.connects[1] = FRAME.into()),
            ErrorCode::UnsupportedProperty,
        ),
        (
            with(&|b| b.connects[1] = "not-a-uuid".into()),
            ErrorCode::InvalidId,
        ),
        (with(&|b| b.diameter_mm = 0.0), ErrorCode::InvalidArgument),
        (with(&|b| b.diameter_mm = 1000.5), ErrorCode::LimitExceeded),
        (
            with(&|b| b.diameter_mm = f64::NAN),
            ErrorCode::NonFiniteNumber,
        ),
        (with(&|b| b.count = 0), ErrorCode::InvalidArgument),
        (with(&|b| b.count = 513), ErrorCode::InvalidArgument),
        (with(&|b| b.pitch_mm = 0.0), ErrorCode::InvalidArgument),
        (with(&|b| b.pitch_mm = 1e12), ErrorCode::LimitExceeded),
        (
            with(&|b| b.origin_mm = [1e10, 0.0, 0.0]),
            ErrorCode::LimitExceeded,
        ),
        (
            with(&|b| b.direction_mm = [2.0, 0.0, 0.0]),
            ErrorCode::InvalidArgument,
        ),
    ];
    for (changes, code) in cases {
        assert_eq!(
            refused_code(&mut a, &o, changes.clone()),
            Some(code),
            "{changes:?}"
        );
    }
    // A group has no material and no body; its placement is the identity.
    let mut placed = create_bolt(BOLT, bolt(PLATE, MEMBER));
    if let Change::Create(e) = &mut placed {
        e.placement.translation_mm = [0.0, 0.0, 1.0];
    }
    assert_eq!(
        refused_code(&mut a, &o, vec![placed]),
        Some(ErrorCode::UnsupportedProperty)
    );
    let mut body = mesh();
    body.id = BOLT.into();
    assert_eq!(
        refused_code(
            &mut a,
            &o,
            vec![create_bolt(BOLT, bolt(PLATE, MEMBER)), Change::Mesh(body)]
        ),
        Some(ErrorCode::UnsupportedProperty)
    );

    // Loaded content: one target breaks the cardinality, a count outside the range and a
    // connection with both property sets are refused, never guessed.
    let (_, o) = installed(&mut a, &o, vec![create_bolt(BOLT, bolt(PLATE, MEMBER))]);
    let from = format!("rel bim:connects = [<{}>, <{}>]", path(PLATE), path(MEMBER));
    for (to, code) in [
        (
            format!("rel bim:connects = <{}>", path(PLATE)),
            ErrorCode::DependencyConflict,
        ),
        (
            format!("rel bim:connects = [<{}>, <{}>]", path(PLATE), path(PLATE)),
            ErrorCode::InvalidArgument,
        ),
    ] {
        let input = tampered(&a, &o.context, "authored.usda", &from, &to);
        assert_eq!(
            validate_layers(input, schema_identity().unwrap(), &control())
                .err()
                .map(|e| e.code),
            Some(code),
            "{to}"
        );
    }
    let input = tampered(
        &a,
        &o.context,
        "authored.usda",
        "int steel:bolt:count = 4",
        "int steel:bolt:count = 513",
    );
    assert_eq!(
        validate_layers(input, schema_identity().unwrap(), &control())
            .err()
            .map(|e| e.code),
        Some(ErrorCode::InvalidArgument)
    );
    let input = tampered(
        &a,
        &o.context,
        "authored.usda",
        "int steel:bolt:count = 4",
        "int steel:bolt:count = 4\ncustom double steel:weld:sizeMm = 5",
    );
    assert_eq!(
        validate_layers(input, schema_identity().unwrap(), &control())
            .err()
            .map(|e| e.code),
        Some(ErrorCode::InvalidArgument)
    );
}

#[test]
fn section_profiles_round_trip_and_tampering_is_refused() {
    use beamcraft_model::section::{CustomSection, ParametricSection, SectionFamily};
    let (mut a, mut o) = populated();
    let i_shape = Resource::new(Definition::SectionProfile {
        shape: SectionShape::Parametric(
            ParametricSection::new(SectionFamily::IShape, vec![300.0, 150.0, 7.0, 10.0, 15.0])
                .unwrap(),
        ),
        section: SectionProperties {
            designation: Some("EX-I300".into()),
            ..SectionProperties::default()
        },
    })
    .unwrap();
    let custom = Resource::new(Definition::SectionProfile {
        shape: SectionShape::Custom(CustomSection {
            outer: vec![[-50.0, -50.0], [50.0, -50.0], [50.0, 50.0], [-50.0, 50.0]],
            cavities: vec![vec![
                [-10.0, -10.0],
                [-10.0, 10.0],
                [10.0, 10.0],
                [10.0, -10.0],
            ]],
        }),
        section: SectionProperties::default(),
    })
    .unwrap();
    // The member ends linked to the custom outline; only linked definitions are validated.
    for r in [i_shape, custom] {
        let mut input = batch(
            &o.context,
            vec![Change::Profile {
                id: MEMBER.into(),
                digest: r.digest,
            }],
        );
        input.resources = vec![r];
        let p = a.prepare(input, &control()).unwrap();
        o = a
            .install(
                &o.context,
                &p.token,
                &p.op_id,
                Decision::Accepted,
                &control(),
            )
            .unwrap();
    }
    let base = a.export(&o.context, &control()).unwrap();
    let owned = ["root.usda", "authored.usda", "derived.usda", "weak.usda"];
    let text = |assets: &[Asset]| {
        String::from_utf8(
            assets
                .iter()
                .find(|a| a.name == "authored.usda")
                .unwrap()
                .bytes
                .clone(),
        )
        .unwrap()
    };
    let authored = text(&base);
    for needle in [
        "bim:definitionKind = \"i_shape\"",
        "steel:profile:twMm = 7.0",
        "bim:definitionKind = \"custom\"",
        "steel:profile:cavityVertexCounts = [4]",
    ] {
        assert!(authored.contains(needle), "{needle}");
    }
    validate_layers(
        request(base.clone(), &owned),
        schema_identity().unwrap(),
        &control(),
    )
    .unwrap();
    let tampered = |from: &str, to: &str| {
        let mut assets = base.clone();
        let own = assets
            .iter_mut()
            .find(|a| a.name == "authored.usda")
            .unwrap();
        let changed = text(&base).replace(from, to);
        assert_ne!(changed, text(&base));
        *own = asset("authored.usda", changed);
        validate_layers(
            request(assets, &owned),
            schema_identity().unwrap(),
            &control(),
        )
        .unwrap_err()
        .code
    };
    assert_eq!(
        tampered("(-10.0, 10.0, 0.0)", "(-11.0, 10.0, 0.0)"),
        ErrorCode::IntegrityError
    );
    assert_eq!(
        tampered(
            "steel:profile:cavityVertexCounts = [4]",
            "steel:profile:cavityVertexCounts = [5]"
        ),
        ErrorCode::InvalidArgument
    );
}

const STOREY: &str = "0199e7a0-0000-7000-8000-000000000401";
const ROOF: &str = "0199e7a0-0000-7000-8000-000000000402";
fn level(id: &str, name: &str, elevation_mm: f64) -> Change {
    Change::Create(Element {
        id: id.into(),
        name: name.into(),
        shape: Shape::Level { elevation_mm },
        placement: Placement {
            translation_mm: [0.0; 3],
            orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
        },
        material: None,
    })
}
fn on_level(id: &str, level: Option<&str>) -> Change {
    Change::Level {
        id: id.into(),
        level: level.map(str::to_owned),
    }
}
fn level_targets(prims: &[Prim], id: &str) -> Option<Vec<String>> {
    prim(prims, &path(id))
        .relationships
        .iter()
        .find(|r| r.name == "bim:level")
        .map(|r| r.targets.clone())
}

#[test]
fn levels_and_level_records_are_authored_on_the_element_and_validated() {
    let (mut a, o) = populated();
    let (p, o) = installed(
        &mut a,
        &o,
        vec![
            level(STOREY, "Level 1", 3000.0),
            level(ROOF, "Roof", -250.5),
            on_level(PLATE, Some(STOREY)),
            on_level(MEMBER, Some(STOREY)),
        ],
    );
    let prims = snapshot(&a, &o.context);
    let storey = prim(&prims, &path(STOREY));
    assert_eq!(storey.kind.as_deref(), Some("component"));
    assert_eq!(storey.display_name.as_deref(), Some("Level 1"));
    assert_eq!(
        value(&prims, STOREY, "bim:elementKind"),
        Some(&Value::Text("level".into()))
    );
    assert_eq!(
        value(&prims, STOREY, "bim:elevationMm"),
        Some(&Value::Number(3000.0))
    );
    assert_eq!(
        value(&prims, ROOF, "bim:elevationMm"),
        Some(&Value::Number(-250.5))
    );
    // No body, no shape inputs, no material: a level is a storey record, not a part.
    assert!(
        !prims
            .iter()
            .any(|p| p.path == format!("{}/Geometry", path(STOREY)))
    );
    assert!(value(&prims, STOREY, "steel:plate:thicknessMm").is_none());
    assert_eq!(level_targets(&prims, PLATE), Some(vec![path(STOREY)]));
    assert_eq!(level_targets(&prims, MEMBER), Some(vec![path(STOREY)]));
    assert_eq!(level_targets(&prims, STOREY), None);
    // The record lives with its source: only the relationship group of the element is written,
    // and no derived group, so a level never changes a body.
    let plate_writes: BTreeSet<_> = p
        .writes
        .iter()
        .filter(|f| f.identity == PLATE)
        .map(|f| f.group)
        .collect();
    assert_eq!(plate_writes, BTreeSet::from([Group::Relationship]));
    assert!(p.reads.contains(&Footprint {
        identity: STOREY.into(),
        group: Group::Lifecycle
    }));
    assert!(p.validation.rules.iter().any(|r| r.starts_with("D2-V01")));

    let dangling = "0199e7a0-0000-7000-8000-0000000003ff";
    for (changes, code) in [
        // An unknown level, and a target that is not a level.
        (
            vec![on_level(PLATE, Some(dangling))],
            ErrorCode::DanglingRelationship,
        ),
        (
            vec![on_level(PLATE, Some(MEMBER))],
            ErrorCode::InvalidArgument,
        ),
        (
            vec![on_level(PLATE, Some("not-a-uuid"))],
            ErrorCode::InvalidId,
        ),
        // Only a plate, beam or column is assigned; a level is not.
        (
            vec![on_level(STOREY, Some(ROOF))],
            ErrorCode::UnsupportedProperty,
        ),
        // A level has no shape inputs, no body, a bounded finite elevation and the identity pose.
        (
            vec![Change::Thickness {
                id: STOREY.into(),
                mm: 5.0,
            }],
            ErrorCode::UnsupportedProperty,
        ),
        (
            vec![level(
                "0199e7a0-0000-7000-8000-000000000303",
                "Bad",
                f64::NAN,
            )],
            ErrorCode::NonFiniteNumber,
        ),
        (
            vec![level(
                "0199e7a0-0000-7000-8000-000000000303",
                "Far",
                1_000_000_000.5,
            )],
            ErrorCode::LimitExceeded,
        ),
        (
            vec![level("0199e7a0-0000-7000-8000-000000000303", "", 1.0)],
            ErrorCode::InvalidArgument,
        ),
        (vec![level(STOREY, "Again", 0.0)], ErrorCode::DuplicateId),
    ] {
        assert_eq!(
            refused_code(&mut a, &o, changes.clone()),
            Some(code),
            "{changes:?}"
        );
    }
    let mut placed = level("0199e7a0-0000-7000-8000-000000000303", "Moved", 1.0);
    if let Change::Create(e) = &mut placed {
        e.placement.translation_mm = [0.0, 0.0, 1.0];
    }
    assert_eq!(
        refused_code(&mut a, &o, vec![placed]),
        Some(ErrorCode::UnsupportedProperty)
    );
    let mut body = mesh();
    body.id = ROOF.into();
    assert_eq!(
        refused_code(&mut a, &o, vec![Change::Mesh(body)]),
        Some(ErrorCode::UnsupportedProperty)
    );

    // Loaded content: two targets break the cardinality, a target that is not a level or does
    // not exist is refused, and so is an elevation beyond the cap.
    let one = format!("rel bim:level = <{}>", path(STOREY));
    for (to, code) in [
        (
            format!("rel bim:level = [<{}>, <{}>]", path(STOREY), path(ROOF)),
            ErrorCode::DependencyConflict,
        ),
        (
            format!("rel bim:level = <{}>", path(MEMBER)),
            ErrorCode::InvalidArgument,
        ),
        (
            format!("rel bim:level = <{}>", path(dangling)),
            ErrorCode::DanglingRelationship,
        ),
    ] {
        let input = tampered(&a, &o.context, "authored.usda", &one, &to);
        assert_eq!(
            validate_layers(input, schema_identity().unwrap(), &control())
                .err()
                .map(|e| e.code),
            Some(code),
            "{to}"
        );
    }
    let input = tampered(
        &a,
        &o.context,
        "authored.usda",
        "double bim:elevationMm = 3000.0",
        "double bim:elevationMm = 3000000000.5",
    );
    assert_eq!(
        validate_layers(input, schema_identity().unwrap(), &control())
            .err()
            .map(|e| e.code),
        Some(ErrorCode::LimitExceeded)
    );

    // A level that is in use cannot be deleted; an unused one can. A copy of an element keeps
    // the level link.
    let index = a.element_index(&o.context, &control()).unwrap();
    let storey = index::ElementId::parse(STOREY).unwrap();
    assert_eq!(
        a.check_delete(&index, &storey, &control())
            .err()
            .map(|e| e.code),
        Some(ErrorCode::DependencyConflict)
    );
    a.check_delete(&index, &index::ElementId::parse(ROOF).unwrap(), &control())
        .unwrap();
    let fresh = index::ElementId::parse("0199e7a0-0000-7000-8000-0000000003fe").unwrap();
    let plan = index
        .copy_plan(&index::ElementId::parse(PLATE).unwrap(), &fresh)
        .unwrap();
    assert!(
        plan.links
            .iter()
            .any(|l| l.name == "bim:level" && l.target == index::LinkTarget::Element(storey))
    );

    // Unassigning authors an explicit empty record, then the level can go.
    let (_, o) = installed(
        &mut a,
        &o,
        vec![on_level(PLATE, None), on_level(MEMBER, None)],
    );
    let prims = snapshot(&a, &o.context);
    assert_eq!(level_targets(&prims, PLATE), Some(vec![]));
    let index = a.element_index(&o.context, &control()).unwrap();
    a.check_delete(&index, &storey, &control()).unwrap();
    let (_, o) = installed(&mut a, &o, vec![Change::Delete { id: STOREY.into() }]);
    assert!(
        !snapshot(&a, &o.context)
            .iter()
            .any(|p| p.path == path(STOREY))
    );
}

#[test]
fn the_project_record_is_validated_read_and_kept_through_edits() {
    const PROJECT: &str = "0199e7a0-0000-7000-8000-000000000001";
    let with_root = |data: &str| {
        let mut input = initial();
        let text = format!(
            "#usda 1.0\n(metersPerUnit=0.001\nupAxis=\"Z\"\nsubLayers=[@authored.usda@,@derived.usda@,@weak.usda@]\ncustomLayerData={{{data}}})\n"
        );
        *input
            .assets
            .iter_mut()
            .find(|a| a.name == "root.usda")
            .unwrap() = asset("root.usda", text);
        input.manifest = input.manifest_digest().unwrap();
        input
    };
    let (mut a, o) = open(with_root(&format!(
        "string bimProjectId=\"{PROJECT}\"\nstring future=\"keep\""
    )));
    assert_eq!(
        project_record(&o.metadata).unwrap().as_deref(),
        Some(PROJECT)
    );
    let mut populate = batch(&o.context, elements());
    populate.resources = resources();
    let installed = a
        .prepare(populate, &control())
        .and_then(|p| {
            a.install(
                &o.context,
                &p.token,
                &p.op_id,
                Decision::Accepted,
                &control(),
            )
        })
        .unwrap();
    // The record and the unknown entry survive the rewrite of the root layer, and the record
    // is the one the reader reports.
    assert_eq!(
        project_record(&installed.metadata).unwrap().as_deref(),
        Some(PROJECT)
    );
    let root = a
        .export(&installed.context, &control())
        .unwrap()
        .into_iter()
        .find(|l| l.name == "root.usda")
        .unwrap();
    let text = String::from_utf8(root.bytes).unwrap();
    assert!(text.contains(PROJECT) && text.contains("future"), "{text}");
    // A store without a record reports none; it is never invented.
    let (_, plain) = open(initial());
    assert_eq!(project_record(&plain.metadata).unwrap(), None);
    // A record that is not a full UUIDv7 text is refused when the layers are opened.
    for bad in [
        "string bimProjectId=\"not-a-uuid\"",
        "int bimProjectId=7",
        "string bimProjectId=\"0199E7A0-0000-7000-8000-000000000001\"",
        "string bimProjectId=\"0199e7a0-0000-4000-8000-000000000001\"",
    ] {
        let refused = Author::open(
            with_root(bad),
            targets(),
            schema_identity().unwrap(),
            &control(),
        )
        .err()
        .map(|e| e.code);
        assert_eq!(refused, Some(ErrorCode::InvalidId), "{bad}");
        assert_eq!(
            validate_layers(with_root(bad), schema_identity().unwrap(), &control())
                .err()
                .map(|e| e.code),
            Some(ErrorCode::InvalidId),
            "{bad}"
        );
    }
}

fn mark(series: &str, number: u32) -> Mark {
    Mark {
        series: series.into(),
        number,
        basis: "BCNUM1 plate=PL:1 token=abc".into(),
    }
}
fn marked(id: &str, series: &str, number: u32) -> Change {
    Change::Mark {
        id: id.into(),
        mark: mark(series, number),
    }
}

#[test]
fn a_mark_is_three_properties_of_the_element_replaced_whole_and_writes_only_the_label_group() {
    let (mut a, o) = populated();
    let (_, o) = installed(&mut a, &o, vec![assembly(FRAME, "Frame")]);
    let (p, o) = installed(
        &mut a,
        &o,
        vec![
            marked(PLATE, "PL", 7),
            marked(MEMBER, "B", 0),
            marked(FRAME, "A-", 2_147_483_647),
        ],
    );
    let prims = snapshot(&a, &o.context);
    assert_eq!(
        value(&prims, PLATE, MARK_SERIES),
        Some(&Value::Text("PL".into()))
    );
    assert_eq!(value(&prims, PLATE, MARK_NUMBER), Some(&Value::Int(7)));
    assert_eq!(
        value(&prims, PLATE, MARK_BASIS),
        Some(&Value::Text("BCNUM1 plate=PL:1 token=abc".into()))
    );
    assert_eq!(value(&prims, MEMBER, MARK_NUMBER), Some(&Value::Int(0)));
    assert_eq!(
        value(&prims, FRAME, MARK_NUMBER),
        Some(&Value::Int(i64::from(i32::MAX)))
    );
    // A mark touches the label group alone: no shape, placement, relationship or derived group,
    // so a mark never invalidates a body or a quantity.
    for id in [PLATE, MEMBER, FRAME] {
        let writes: BTreeSet<_> = p
            .writes
            .iter()
            .filter(|f| f.identity == id)
            .map(|f| f.group)
            .collect();
        assert_eq!(writes, BTreeSet::from([Group::Label]), "{id}");
    }
    assert!(
        p.opinions
            .iter()
            .any(|i| i.element_id == PLATE && i.attribute == MARK_NUMBER && i.type_name == "int")
    );
    // The derived mesh and the shape inputs are untouched.
    assert!(value(&prims, PLATE, "steel:plate:thicknessMm").is_some());
    assert!(
        prims
            .iter()
            .any(|p| p.path == format!("{}/Geometry", path(PLATE)))
    );
    // A second mark replaces the first as a whole; the stage stays valid.
    let (_, o) = installed(&mut a, &o, vec![marked(PLATE, "P", 8)]);
    let prims = snapshot(&a, &o.context);
    assert_eq!(
        value(&prims, PLATE, MARK_SERIES),
        Some(&Value::Text("P".into()))
    );
    assert_eq!(value(&prims, PLATE, MARK_NUMBER), Some(&Value::Int(8)));
    validate_layers(
        export_request(&a, &o.context),
        schema_identity().unwrap(),
        &control(),
    )
    .unwrap();
}

#[test]
fn wrong_marks_targets_and_loaded_mark_records_are_refused_with_nothing_written() {
    let (mut a, o) = populated();
    let (_, o) = installed(
        &mut a,
        &o,
        vec![level(STOREY, "Level 1", 3000.0), assembly(FRAME, "Frame")],
    );
    let bad = |m: Mark| Change::Mark {
        id: PLATE.into(),
        mark: m,
    };
    let too_long = "X".repeat(17);
    for (change, code) in [
        (marked(PLATE, &too_long, 1), ErrorCode::InvalidArgument),
        (marked(PLATE, "a b", 1), ErrorCode::InvalidArgument),
        (marked(PLATE, "Ω", 1), ErrorCode::InvalidArgument),
        (
            marked(PLATE, "PL", 2_147_483_648),
            ErrorCode::InvalidArgument,
        ),
        (
            bad(Mark {
                basis: String::new(),
                ..mark("PL", 1)
            }),
            ErrorCode::InvalidArgument,
        ),
        (
            bad(Mark {
                basis: "x".repeat(513),
                ..mark("PL", 1)
            }),
            ErrorCode::InvalidArgument,
        ),
        (
            bad(Mark {
                basis: "tab\there".into(),
                ..mark("PL", 1)
            }),
            ErrorCode::InvalidArgument,
        ),
        // A level is no part; an absent element is NOT_FOUND.
        (marked(STOREY, "PL", 1), ErrorCode::UnsupportedProperty),
        (
            marked("0199e7a0-0000-7000-8000-0000000009ff", "PL", 1),
            ErrorCode::NotFound,
        ),
        (marked("not-an-id", "PL", 1), ErrorCode::InvalidId),
    ] {
        let got = refused_code(&mut a, &o, vec![change.clone()]);
        assert_eq!(got, Some(code), "{change:?}");
    }
    // Two marks of one element in a batch are a duplicate key.
    assert!(
        refused_code(
            &mut a,
            &o,
            vec![marked(PLATE, "PL", 1), marked(PLATE, "PL", 2)]
        )
        .is_some()
    );
    // Loaded records: a partial record, a wrong type, an out-of-range number, a mark on a level.
    let (_, o) = installed(&mut a, &o, vec![marked(PLATE, "PL", 7)]);
    let number = "int steel:numbering:number = 7";
    let series = "string steel:numbering:series = \"PL\"";
    for (from, to) in [
        (number, "double steel:numbering:number = 7"),
        (number, "int steel:numbering:number = -1"),
        (series, "string steel:numbering:series = \"a b\""),
        (series, "token steel:numbering:series = \"PL\""),
    ] {
        let input = tampered(&a, &o.context, "authored.usda", from, to);
        let verdict = validate_layers(input, schema_identity().unwrap(), &control());
        assert_eq!(
            verdict.err().map(|e| e.code),
            Some(ErrorCode::InvalidArgument),
            "{to}"
        );
    }
}

#[test]
fn a_store_pinned_to_the_pre_a09_mapping_is_refused() {
    let previous = "fdaf890401a7bf4d8d17eb84ad1901b461ff21f1e7cec1e93f9e71203b83865b";
    let mut input = initial();
    for name in ["authored.usda", "derived.usda"] {
        let old = input.assets.iter_mut().find(|a| a.name == name).unwrap();
        let text = String::from_utf8(old.bytes.clone()).unwrap();
        let body = text.split_once(")\n").unwrap().1.to_owned();
        *old = asset(
            name,
            format!(
                "#usda 1.0\n(metersPerUnit=0.001\nupAxis=\"Z\"\ncustomLayerData={{string bimSchemaDigest=\"{previous}\"}})\n{body}\n"
            ),
        );
    }
    input.manifest = input.manifest_digest().unwrap();
    assert_eq!(
        Author::open(input, targets(), schema_identity().unwrap(), &control())
            .err()
            .map(|e| e.code),
        Some(ErrorCode::UnsupportedVersion)
    );
}

#[test]
fn connection_discriminator_roundtrips_and_rejects_missing_unknown_or_conflicting_values() {
    for (id, change, expected) in [
        (WELD, create_weld(WELD, weld(PLATE, MEMBER)), "weld"),
        (BOLT, create_bolt(BOLT, bolt(PLATE, MEMBER)), "bolt_group"),
    ] {
        let (mut author, opened) = populated();
        let (prepared, opened) = installed(&mut author, &opened, vec![change]);
        assert!(
            prepared
                .opinions
                .iter()
                .any(|v| v.attribute == "steel:connection:type")
        );
        let prims = snapshot(&author, &opened.context);
        assert_eq!(
            value(&prims, id, "steel:connection:type"),
            Some(&Value::Text(expected.into()))
        );
        let old = format!("custom token steel:connection:type = \"{expected}\"");
        for (replacement, rule) in [
            (String::new(), "DV-ATTR-001"),
            (
                "custom token steel:connection:type = \"unknown\"".into(),
                "DV-ATTR-002",
            ),
            (
                format!(
                    "custom token steel:connection:type = \"{}\"",
                    if expected == "weld" {
                        "bolt_group"
                    } else {
                        "weld"
                    }
                ),
                "DV-ATTR-002",
            ),
            (
                "custom string steel:connection:type = \"weld\"".into(),
                "DV-ATTR-002",
            ),
        ] {
            let request = tampered(
                &author,
                &opened.context,
                "authored.usda",
                &old,
                &replacement,
            );
            let error =
                validate_layers(request, schema_identity().unwrap(), &control()).unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidArgument);
            assert_eq!(error.rule_id, Some(rule));
        }
    }
}
