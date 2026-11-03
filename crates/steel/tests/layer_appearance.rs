// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original generated synthetic fixtures, MPL-2.0; covered by ATTRIBUTION.md's
// test-data row. Expectations follow the U08 design: UsdShade Material + UsdPreviewSurface,
// material:binding on the element, primvars:displayColor as the fallback, colour from
// presentation (steel:class through the P15 palette), never from the engineering material.

//! U08 portable-appearance regressions on the real authoring backend.

#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

use beamcraft_layer_engine::{authored::*, *};
use beamcraft_model::{
    digest::{ContentDigest, Sha256},
    error::ErrorCode,
};
use openusd::{
    sdf::{self, AbstractData},
    usda::TextWriter,
};
use std::time::{Duration, Instant};
use steel::layer as steel_layer;
use steel::layer::*;

const PLATE: &str = "0199e7a0-0000-7000-8000-000000000003";
const ASSEMBLY: &str = "0199e7a0-0000-7000-8000-000000000009";
const ACTOR: &str = "0199e7a0-0000-7000-8000-000000000005";
const OP: &str = "0199e7a0-0000-7000-8000-000000000006";

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
            revision: digest(b"pinned U08 base"),
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
const OWNED: [&str; 3] = ["root.usda", "authored.usda", "derived.usda"];
fn initial() -> OpenRequest {
    request(
        vec![
            asset(
                "root.usda",
                "#usda 1.0\n(metersPerUnit=0.001\nupAxis=\"Z\"\nsubLayers=[@authored.usda@,@derived.usda@])\n",
            ),
            asset("authored.usda", layer("")),
            asset("derived.usda", layer("")),
        ],
        &OWNED,
    )
}
fn targets() -> EditTargets {
    EditTargets {
        authored: "authored.usda".into(),
        derived: "derived.usda".into(),
    }
}
fn open(input: OpenRequest) -> (Author, Opened) {
    Author::open(
        input,
        targets(),
        None,
        schema_identity().unwrap(),
        &control(),
    )
    .unwrap()
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
fn pose() -> Placement {
    Placement {
        translation_mm: [12.25, -42.125, 0.0],
        orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
    }
}
fn material() -> Resource {
    Resource::new(Definition::Material {
        density_kg_m3: Some(7850.0),
        grade: Some("S355".into()),
        standard: None,
        display_name: None,
    })
    .unwrap()
}
fn plate(material: Option<ContentDigest>) -> Change {
    Change::Create(Element {
        id: PLATE.into(),
        name: "Plate".into(),
        shape: steel_layer::plate(PlateShape {
            contour_mm: vec![
                [0.0, 0.0, 0.0],
                [1000.0, 0.0, 0.0],
                [1000.0, 500.0, 0.0],
                [0.0, 500.0, 0.0],
            ],
            thickness_mm: 10.0,
            thickness_side: None,
            corners: Vec::new(),
            holes: Vec::new(),
            hole_records: Vec::new(),
            cuts: Vec::new(),
            bends: Vec::new(),
        }),
        placement: pose(),
        material,
    })
}
fn assembly() -> Change {
    Change::Create(Element {
        id: ASSEMBLY.into(),
        name: "Frame".into(),
        shape: Shape::Assembly,
        placement: Placement {
            translation_mm: [0.0; 3],
            orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
        },
        material: None,
    })
}
fn install(author: &mut Author, context: &Context, prepared: &Prepared) -> Opened {
    author
        .install(
            context,
            &prepared.token,
            &prepared.op_id,
            Decision::Accepted,
            &control(),
        )
        .unwrap()
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
fn plate_path() -> String {
    format!("/World/Steel/Parts/PRT_{}", PLATE.replace('-', ""))
}
fn assembly_path() -> String {
    format!("/World/Steel/Assemblies/ASM_{}", ASSEMBLY.replace('-', ""))
}
fn prim<'a>(all: &'a [Prim], at: &str) -> &'a Prim {
    all.iter().find(|p| p.path == at).unwrap()
}
fn value<'a>(p: &'a Prim, name: &str) -> Option<&'a Value> {
    p.properties
        .iter()
        .find(|a| a.name == name)
        .and_then(|a| a.value.as_ref())
}
/// The test's own sRGB-to-linear oracle, independent of the implementation.
fn linear(colour: [u8; 3]) -> [f32; 3] {
    let [r, g, b] = colour;
    let channel = |byte: u8| {
        let c = f64::from(byte) / 255.0;
        let linear = if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        };
        linear as f32
    };
    [channel(r), channel(g), channel(b)]
}
fn numbers(value: &Value) -> Vec<f32> {
    let Value::Array(items) = value else {
        panic!("not an array: {value:?}")
    };
    let mut out = Vec::new();
    for item in items {
        match item {
            Value::Array(triple) => triple.iter().for_each(|v| match v {
                Value::Number(n) => out.push(*n as f32),
                other => panic!("not a number: {other:?}"),
            }),
            Value::Number(n) => out.push(*n as f32),
            other => panic!("not numeric: {other:?}"),
        }
    }
    out
}
/// A populated store with one plate carrying an engineering material.
fn with_plate() -> (Author, Opened, Resource) {
    let m = material();
    let (mut a, o) = open(initial());
    let mut input = batch(&o.context, vec![plate(Some(m.digest))]);
    input.resources = vec![m.clone()];
    let p = a.prepare(input, &control()).unwrap();
    let installed = install(&mut a, &o.context, &p);
    (a, installed, m)
}
/// The authored layer bytes of a live store, re-parsed for tampering.
fn authored_data(author: &Author, context: &Context) -> sdf::Data {
    let exported = author.export(context, &control()).unwrap();
    let own = exported
        .iter()
        .find(|a| a.name == "authored.usda")
        .unwrap()
        .bytes
        .clone();
    openusd::usda::parse(std::str::from_utf8(&own).unwrap()).unwrap()
}
fn serialize(data: &sdf::Data) -> Vec<u8> {
    let mut buffer = Vec::new();
    TextWriter::write(data, &mut buffer).unwrap();
    buffer
}
/// Adds `name` to the property children of a prim in a parsed layer.
fn push_property(data: &mut sdf::Data, holder: &str, name: &str) {
    let at = openusd::sdf::Path::new(holder).unwrap();
    let mut children = match data.try_field(&at, "propertyChildren") {
        Ok(Some(v)) => match v.as_ref() {
            sdf::Value::TokenVec(v) => v.clone(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    };
    children.push(name.into());
    data.set_field(&at, "propertyChildren", sdf::Value::TokenVec(children));
}
/// Authors a uniform attribute spec on `holder` in a parsed layer, with or without a default.
fn author_attribute(
    data: &mut sdf::Data,
    holder: &str,
    name: &str,
    ty: &str,
    default: Option<sdf::Value>,
) {
    let full = openusd::sdf::Path::new(&format!("{holder}.{name}")).unwrap();
    data.create_spec(full.clone(), sdf::SpecType::Attribute);
    push_property(data, holder, name);
    data.set_field(&full, "typeName", sdf::Value::token(ty));
    data.set_field(&full, "custom", sdf::Value::Bool(false));
    data.set_field(
        &full,
        "variability",
        sdf::Value::Variability(sdf::Variability::Uniform),
    );
    if let Some(v) = default {
        data.set_field(&full, "default", v);
    }
}
/// Applies `edit` to the authored layer of a live store and returns the strict-profile
/// refusal of the tampered bytes.
fn tampered_refusal(
    author: &Author,
    context: &Context,
    edit: impl FnOnce(&mut sdf::Data),
) -> Diagnostic {
    let mut data = authored_data(author, context);
    edit(&mut data);
    let bytes = serialize(&data);
    let exported = author.export(context, &control()).unwrap();
    let assets = exported
        .into_iter()
        .map(|mut a| {
            if a.name == "authored.usda" {
                a.bytes = bytes.clone();
                a.digest = digest(&a.bytes);
            }
            a
        })
        .collect();
    validate_layers(
        request(assets, &OWNED),
        schema_identity().unwrap(),
        &control(),
    )
    .err()
    .unwrap_or_else(|| panic!("tampered layers must be refused"))
}

#[test]
fn a_created_part_carries_the_portable_appearance_of_its_class() {
    let (author, opened, m) = with_plate();
    let all = snapshot(&author, &opened.context);
    let part = prim(&all, &plate_path());
    // The fallback colour is the class zero palette entry in linear RGB, never the
    // engineering material.
    assert_eq!(
        numbers(value(part, "primvars:displayColor").unwrap()),
        linear([126, 143, 159])
    );
    let binding = part
        .relationships
        .iter()
        .find(|r| r.name == "material:binding")
        .unwrap();
    assert_eq!(binding.targets, vec!["/World/Looks/PartClass0".to_owned()]);
    // The reserved scope holds the eight fixed looks; the bound one carries the UsdShade form.
    let look = prim(&all, "/World/Looks/PartClass0");
    assert_eq!(look.type_name.as_deref(), Some("Material"));
    let shader = prim(&all, "/World/Looks/PartClass0/PreviewSurface");
    assert_eq!(shader.type_name.as_deref(), Some("Shader"));
    assert_eq!(
        value(shader, "info:id"),
        Some(&Value::Text("UsdPreviewSurface".into()))
    );
    assert_eq!(
        numbers(value(shader, "inputs:diffuseColor").unwrap()),
        linear([126, 143, 159])
    );
    for index in 0..8 {
        let path = format!("/World/Looks/PartClass{index}");
        assert!(all.iter().any(|p| p.path == path), "{path} missing");
    }
    // The stored text is the standard portable form any USD consumer reads.
    let exported = author.export(&opened.context, &control()).unwrap();
    let text = std::str::from_utf8(
        &exported
            .iter()
            .find(|a| a.name == "authored.usda")
            .unwrap()
            .bytes,
    )
    .unwrap();
    assert!(
        text.contains(r#"prepend apiSchemas = ["MaterialBindingAPI"]"#),
        "{text}"
    );
    assert!(
        text.contains(r#"token info:id = "UsdPreviewSurface""#),
        "{text}"
    );
    assert!(
        text.contains("token outputs:surface.connect = </World/Looks/PartClass0/PreviewSurface.outputs:surface>"),
        "{text}"
    );
    assert!(
        text.contains("custom uniform color3f[] primvars:displayColor"),
        "{text}"
    );
    let _ = m;
}

#[test]
fn a_class_edit_moves_the_binding_and_the_fallback_together() {
    let (mut author, opened, _) = with_plate();
    let mut opened = opened;
    for class in [2i32, -1, 10] {
        let p = author
            .prepare(
                batch(
                    &opened.context,
                    vec![Change::PartClass {
                        id: PLATE.into(),
                        class,
                    }],
                ),
                &control(),
            )
            .unwrap();
        opened = install(&mut author, &opened.context, &p);
        let all = snapshot(&author, &opened.context);
        let part = prim(&all, &plate_path());
        let index = (class.unsigned_abs() as usize) % 8;
        let palette = beamcraft_model::class_palette::DEFAULT_CLASS_PALETTE;
        assert_eq!(
            part.relationships
                .iter()
                .find(|r| r.name == "material:binding")
                .unwrap()
                .targets,
            vec![format!("/World/Looks/PartClass{index}")]
        );
        assert_eq!(
            numbers(value(part, "primvars:displayColor").unwrap()),
            linear(palette[index])
        );
    }
}

#[test]
fn the_appearance_never_moves_with_the_engineering_material() {
    // A material edit changes the definition link and nothing about the colour.
    let (mut author, opened, m) = with_plate();
    let second = Resource::new(Definition::Material {
        density_kg_m3: Some(1000.0),
        grade: Some("Q235".into()),
        standard: None,
        display_name: None,
    })
    .unwrap();
    let mut input = batch(
        &opened.context,
        vec![Change::Material {
            id: PLATE.into(),
            opinion: MaterialOpinion::Target(second.digest),
        }],
    );
    input.resources = vec![second.clone()];
    let p = author.prepare(input, &control()).unwrap();
    let opened = install(&mut author, &opened.context, &p);
    let all = snapshot(&author, &opened.context);
    let part = prim(&all, &plate_path());
    assert_eq!(
        part.relationships
            .iter()
            .find(|r| r.name == "bim:material")
            .unwrap()
            .targets,
        vec![format!(
            "/World/Definitions/Materials/MAT_{}",
            second.digest
        )]
    );
    assert_eq!(
        part.relationships
            .iter()
            .find(|r| r.name == "material:binding")
            .unwrap()
            .targets,
        vec!["/World/Looks/PartClass0".to_owned()]
    );
    assert_eq!(
        numbers(value(part, "primvars:displayColor").unwrap()),
        linear([126, 143, 159])
    );
    let _ = m;
}

#[test]
fn a_tampered_look_is_refused_by_the_strict_profile() {
    let (author, opened, _) = with_plate();
    let error = tampered_refusal(&author, &opened.context, |data| {
        data.set_field(
            &openusd::sdf::Path::new("/World/Looks/PartClass0/PreviewSurface.inputs:diffuseColor")
                .unwrap(),
            "default",
            sdf::Value::Vec3f([0.0, 0.0, 0.0].into()),
        );
    });
    assert_eq!(error.code, ErrorCode::InvalidArgument, "{}", error.message);
}

#[test]
fn a_tampered_binding_is_refused_by_the_strict_profile() {
    let (author, opened, _) = with_plate();
    let error = tampered_refusal(&author, &opened.context, |data| {
        data.set_field(
            &openusd::sdf::Path::new(&format!("{}.material:binding", plate_path())).unwrap(),
            "targetPaths",
            sdf::Value::PathListOp(sdf::ListOp::explicit(vec![
                openusd::sdf::Path::new("/World/Looks/PartClass7").unwrap(),
            ])),
        );
    });
    assert_eq!(error.code, ErrorCode::InvalidArgument, "{}", error.message);
}

#[test]
fn an_extra_property_on_a_look_is_refused_by_the_strict_profile() {
    // The looks are closed content: an extra shader input, value and all, is outside the
    // reserved scope.
    let (author, opened, _) = with_plate();
    let error = tampered_refusal(&author, &opened.context, |data| {
        author_attribute(
            data,
            "/World/Looks/PartClass0/PreviewSurface",
            "inputs:metallic",
            "float",
            Some(sdf::Value::Float(0.5)),
        );
    });
    assert_eq!(
        error.code,
        ErrorCode::UnsupportedProperty,
        "{}",
        error.message
    );
}

#[test]
fn a_declared_diffuse_colour_outside_the_shader_is_refused() {
    let (author, opened, _) = with_plate();
    // A declared-but-valueless diffuse colour on the material: the property is outside the
    // material's closed set, with or without a value.
    let error = tampered_refusal(&author, &opened.context, |data| {
        author_attribute(
            data,
            "/World/Looks/PartClass0",
            "inputs:diffuseColor",
            "color3f",
            None,
        );
    });
    assert_eq!(
        error.code,
        ErrorCode::UnsupportedProperty,
        "{}",
        error.message
    );
    // The same property on the shader without a value is a declared-but-empty colour:
    // refused as invalid, never read as absent.
    let error = tampered_refusal(&author, &opened.context, |data| {
        data.erase_field(
            &openusd::sdf::Path::new("/World/Looks/PartClass0/PreviewSurface.inputs:diffuseColor")
                .unwrap(),
            "default",
        );
    });
    assert_eq!(error.code, ErrorCode::InvalidArgument, "{}", error.message);
    // A diffuse colour of another USD type holding the same linear triple is refused too.
    let error = tampered_refusal(&author, &opened.context, |data| {
        data.set_field(
            &openusd::sdf::Path::new("/World/Looks/PartClass0/PreviewSurface.inputs:diffuseColor")
                .unwrap(),
            "typeName",
            sdf::Value::token("double3"),
        );
    });
    assert_eq!(error.code, ErrorCode::InvalidArgument, "{}", error.message);
}

#[test]
fn a_valued_output_on_a_look_is_refused_by_the_strict_profile() {
    let (author, opened, _) = with_plate();
    // The surface output is valueless by contract: a stored default refuses, on the
    // material and on the shader alike.
    for holder in [
        "/World/Looks/PartClass0",
        "/World/Looks/PartClass0/PreviewSurface",
    ] {
        let error = tampered_refusal(&author, &opened.context, |data| {
            data.set_field(
                &openusd::sdf::Path::new(&format!("{holder}.outputs:surface")).unwrap(),
                "default",
                sdf::Value::token("fake"),
            );
        });
        assert_eq!(
            error.code,
            ErrorCode::InvalidArgument,
            "{}: {}",
            holder,
            error.message
        );
    }
}

#[test]
fn an_appearance_on_a_non_part_is_refused_by_the_strict_profile() {
    let (mut author, opened, _) = with_plate();
    let p = author
        .prepare(batch(&opened.context, vec![assembly()]), &control())
        .unwrap();
    let opened = install(&mut author, &opened.context, &p);
    let mut data = authored_data(&author, &opened.context);
    let holder = assembly_path();
    let full = openusd::sdf::Path::new(&format!("{holder}.material:binding")).unwrap();
    data.create_spec(full.clone(), sdf::SpecType::Relationship);
    let mut children = match data.try_field(
        &openusd::sdf::Path::new(&holder).unwrap(),
        "propertyChildren",
    ) {
        Ok(Some(v)) => match v.as_ref() {
            sdf::Value::TokenVec(v) => v.clone(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    };
    children.push("material:binding".into());
    data.set_field(
        &openusd::sdf::Path::new(&holder).unwrap(),
        "propertyChildren",
        sdf::Value::TokenVec(children),
    );
    data.set_field(&full, "custom", sdf::Value::Bool(true));
    data.set_field(
        &full,
        "variability",
        sdf::Value::Variability(sdf::Variability::Uniform),
    );
    data.set_field(
        &full,
        "targetPaths",
        sdf::Value::PathListOp(sdf::ListOp::explicit(vec![
            openusd::sdf::Path::new("/World/Looks/PartClass0").unwrap(),
        ])),
    );
    let tampered = serialize(&data);
    let exported = author.export(&opened.context, &control()).unwrap();
    let assets = exported
        .into_iter()
        .map(|mut a| {
            if a.name == "authored.usda" {
                a.bytes = tampered.clone();
                a.digest = digest(&a.bytes);
            }
            a
        })
        .collect();
    let error = validate_layers(
        request(assets, &OWNED),
        schema_identity().unwrap(),
        &control(),
    )
    .unwrap_err();
    assert_eq!(
        error.code,
        ErrorCode::UnsupportedProperty,
        "{}",
        error.message
    );
}

#[test]
fn a_foreign_or_stray_prim_under_the_looks_scope_is_refused() {
    let (author, opened, _) = with_plate();
    let mut data = authored_data(&author, &opened.context);
    let stray = openusd::sdf::Path::new("/World/Looks/Custom").unwrap();
    data.create_spec(stray.clone(), sdf::SpecType::Prim);
    data.set_field(
        &stray,
        "specifier",
        sdf::Value::Specifier(sdf::Specifier::Def),
    );
    data.set_field(&stray, "typeName", sdf::Value::token("Material"));
    let parent = openusd::sdf::Path::new("/World/Looks").unwrap();
    let mut children = match data.try_field(&parent, "primChildren") {
        Ok(Some(v)) => match v.as_ref() {
            sdf::Value::TokenVec(v) => v.clone(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    };
    children.push("Custom".into());
    data.set_field(&parent, "primChildren", sdf::Value::TokenVec(children));
    let tampered = serialize(&data);
    let exported = author.export(&opened.context, &control()).unwrap();
    let assets = exported
        .into_iter()
        .map(|mut a| {
            if a.name == "authored.usda" {
                a.bytes = tampered.clone();
                a.digest = digest(&a.bytes);
            }
            a
        })
        .collect();
    let error = validate_layers(
        request(assets, &OWNED),
        schema_identity().unwrap(),
        &control(),
    )
    .unwrap_err();
    assert_eq!(
        error.code,
        ErrorCode::UnsupportedProperty,
        "{}",
        error.message
    );
}
