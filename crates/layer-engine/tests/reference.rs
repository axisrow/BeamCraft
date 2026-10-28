// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original generated synthetic fixtures, MPL-2.0; covered by ATTRIBUTION.md's
// test-data row. Expectations follow F04 sections 1 and 8, F10 `reference` and the
// R01 design note; the fixture "foreign source" is invented text, not third-party content.

//! R01 real-backend regressions: reference wrappers and the foreign read-only rule.

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

const WRAPPER: &str = "0199e7a0-0000-7000-8000-0000000000a1";
const OTHER: &str = "0199e7a0-0000-7000-8000-0000000000a2";
const ACTOR: &str = "0199e7a0-0000-7000-8000-000000000005";
const OP: &str = "0199e7a0-0000-7000-8000-000000000006";

/// A small invented "architect model": metres, Y up, a block with a mesh body and a note.
const FOREIGN: &str = r#"#usda 1.0
(
    defaultPrim = "Site"
    metersPerUnit = 1.0
    upAxis = "Y"
)
def Xform "Site"
{
    def Xform "Block"
    {
        custom string note = "declared by the source"
        def Mesh "Body"
        {
            point3f[] points = [(0, 0, 0), (2, 0, 0), (2, 1, 0), (0, 1, 0), (0, 0, 3), (2, 0, 3), (2, 1, 3), (0, 1, 3)]
            int[] faceVertexCounts = [4, 4, 4, 4, 4, 4]
            int[] faceVertexIndices = [0, 3, 2, 1, 4, 5, 6, 7, 0, 1, 5, 4, 1, 2, 6, 5, 2, 3, 7, 6, 3, 0, 4, 7]
        }
    }
}
"#;

fn digest(bytes: &[u8]) -> ContentDigest {
    Sha256::digest(bytes).unwrap()
}
fn control() -> Control {
    Control {
        cancellation: Cancellation::default(),
        deadline: Instant::now() + Duration::from_secs(10),
    }
}
fn asset(name: &str, bytes: impl AsRef<[u8]>) -> Asset {
    Asset {
        name: name.into(),
        digest: digest(bytes.as_ref()),
        bytes: bytes.as_ref().to_vec(),
    }
}
fn layer(body: &str) -> String {
    format!("#usda 1.0\n(metersPerUnit=0.001\nupAxis=\"Z\")\n{body}\n")
}
fn pinned_text(pin: &str) -> String {
    format!(
        "#usda 1.0\n(metersPerUnit=0.001\nupAxis=\"Z\"\ncustomLayerData={{string bimSchemaDigest=\"{pin}\"}})\n"
    )
}
fn pin_of(text: &str) -> SourcePin {
    SourcePin {
        digest: digest(text.as_bytes()),
        bytes: text.len() as u64,
        format: SourceFormat::Usda,
        default_prim: "Site".into(),
        version: 1,
        label: "site.usda".into(),
        declared_meters_per_unit: Some(1.0),
        declared_up_axis: Some(UpAxis::Y),
    }
}
fn pin() -> SourcePin {
    pin_of(FOREIGN)
}
fn request(assets: Vec<Asset>, owned: &[&str]) -> OpenRequest {
    let mut request = OpenRequest {
        identity: Identity {
            project: "0199e7a0-0000-7000-8000-000000000001".into(),
            workspace: "0199e7a0-0000-7000-8000-000000000002".into(),
            revision: digest(b"pinned R01 base"),
            generation: 7,
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
            asset(&pin().layer_name(), FOREIGN),
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
    Author::open(input, targets(), schema_identity().unwrap(), &control()).unwrap()
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
        translation_mm: [100.0, 200.0, 300.0],
        orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
    }
}
fn attach(id: &str) -> Change {
    Change::AttachReference(Reference {
        id: id.into(),
        name: "Architect site".into(),
        placement: pose(),
        source: pin(),
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
/// A store with one wrapper attached, as the layer set a host would hold.
fn attached() -> (Author, Opened) {
    let (mut a, o) = open(initial());
    let p = a
        .prepare(batch(&o.context, vec![attach(WRAPPER)]), &control())
        .unwrap();
    let installed = install(&mut a, &o.context, &p);
    (a, installed)
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
fn path(id: &str) -> String {
    format!("/World/References/REF_{}", id.replace('-', ""))
}
fn source_path(id: &str) -> String {
    format!("{}/Correction/Source", path(id))
}
fn prop<'a>(prims: &'a [Prim], at: &str, name: &str) -> Option<&'a Value> {
    prims
        .iter()
        .find(|p| p.path == at)
        .unwrap()
        .properties
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| p.value.as_ref())
}
fn exported(author: &Author, context: &Context) -> Vec<Asset> {
    author.export(context, &control()).unwrap()
}
fn named(assets: &[Asset], name: &str) -> Asset {
    assets.iter().find(|a| a.name == name).unwrap().clone()
}
fn validate(assets: Vec<Asset>) -> Result<Validation, Diagnostic> {
    validate_layers(
        request(assets, &OWNED),
        schema_identity().unwrap(),
        &control(),
    )
}
fn refused(error: Diagnostic, code: ErrorCode) {
    assert_eq!(error.code, code, "{error}");
}
/// Rewrites one owned layer through the spec API, the way a hostile editor would.
fn tamper(assets: &[Asset], layer_name: &str, edit: impl FnOnce(&mut sdf::Data)) -> Vec<Asset> {
    let mut out = assets.to_vec();
    let target = out.iter_mut().find(|a| a.name == layer_name).unwrap();
    let mut data = openusd::usda::parse(std::str::from_utf8(&target.bytes).unwrap()).unwrap();
    edit(&mut data);
    let mut bytes = Vec::new();
    TextWriter::write(&data, &mut bytes).unwrap();
    target.digest = digest(&bytes);
    target.bytes = bytes;
    out
}
fn push_child(data: &mut sdf::Data, parent: &str, key: &str, name: &str) {
    let at = sdf::Path::new(parent).unwrap();
    let mut names = match data.try_field(&at, key).unwrap() {
        Some(v) => match v.as_ref() {
            sdf::Value::TokenVec(v) => v.clone(),
            other => panic!("unexpected {other:?}"),
        },
        None => Vec::new(),
    };
    if !names.iter().any(|n| n.as_str() == name) {
        names.push(name.into());
    }
    data.set_field(&at, key, sdf::Value::TokenVec(names));
}
/// Creates a prim spec and every missing ancestor (as an `over`), registered so that it is written.
fn prim_spec(data: &mut sdf::Data, at: &str, specifier: sdf::Specifier) -> sdf::Path {
    let p = sdf::Path::new(at).unwrap();
    if data.has_spec(&p) {
        return p;
    }
    let (parent, name) = at.rsplit_once('/').unwrap();
    if parent.is_empty() {
        let root = sdf::Path::abs_root();
        if !data.has_spec(&root) {
            data.create_spec(root, sdf::SpecType::PseudoRoot);
        }
        push_child(data, "/", "primChildren", name);
    } else {
        prim_spec(data, parent, sdf::Specifier::Over);
        push_child(data, parent, "primChildren", name);
    }
    data.create_spec(p.clone(), sdf::SpecType::Prim);
    data.set_field(&p, "specifier", sdf::Value::Specifier(specifier));
    p
}
fn over(data: &mut sdf::Data, at: &str) -> sdf::Path {
    prim_spec(data, at, sdf::Specifier::Over)
}
fn def(data: &mut sdf::Data, at: &str) -> sdf::Path {
    prim_spec(data, at, sdf::Specifier::Def)
}
fn attribute_spec(data: &mut sdf::Data, at: &str) -> sdf::Path {
    let (prim, name) = at.rsplit_once('.').unwrap();
    over(data, prim);
    push_child(data, prim, "propertyChildren", name);
    let p = sdf::Path::new(at).unwrap();
    if !data.has_spec(&p) {
        data.create_spec(p.clone(), sdf::SpecType::Attribute);
        data.set_field(&p, "typeName", sdf::Value::token("string"));
    }
    p
}

#[test]
fn attach_composes_the_pinned_source_below_an_owned_wrapper_and_keeps_the_foreign_bytes() {
    let (mut a, o) = open(initial());
    let p = a
        .prepare(batch(&o.context, vec![attach(WRAPPER)]), &control())
        .unwrap();
    // Owned layers are returned for saving; the foreign one is retained byte for byte.
    let mut owned: Vec<_> = p.layers.iter().map(|l| l.name.as_str()).collect();
    owned.sort_unstable();
    assert_eq!(owned, ["authored.usda", "derived.usda", "root.usda"]);
    assert_eq!(p.retained.len(), 1);
    assert_eq!(p.retained[0].name, pin().layer_name());
    assert_eq!(p.retained[0].bytes, FOREIGN.as_bytes());
    assert_eq!(p.retained[0].digest, pin().digest);
    // Footprint: reads the absence of the id, writes lifecycle, label, shape inputs, placement.
    let groups = |set: &std::collections::BTreeSet<Footprint>| {
        set.iter()
            .map(|f| (f.identity.clone(), f.group))
            .collect::<Vec<_>>()
    };
    assert_eq!(groups(&p.reads), vec![(WRAPPER.into(), Group::Lifecycle)]);
    assert_eq!(
        groups(&p.writes),
        vec![
            (WRAPPER.into(), Group::Lifecycle),
            (WRAPPER.into(), Group::Label),
            (WRAPPER.into(), Group::ShapeInputs),
            (WRAPPER.into(), Group::Placement),
        ]
    );
    assert!(
        p.opinions
            .iter()
            .any(|i| i.attribute == "bim:reference:sourceDigest"
                && i.opinion == Opinion::Value(Value::Text(pin().digest.to_string())))
    );
    let new = install(&mut a, &o.context, &p);
    assert_eq!(new.context.identity().generation, 8);
    let prims = snapshot(&a, &new.context);
    let wrapper = prims.iter().find(|p| p.path == path(WRAPPER)).unwrap();
    assert_eq!(wrapper.kind.as_deref(), Some("group"));
    assert_eq!(wrapper.display_name.as_deref(), Some("Architect site"));
    assert!(wrapper.sources.iter().all(|s| s.owned));
    let holder = prims
        .iter()
        .find(|p| p.path == source_path(WRAPPER))
        .unwrap();
    // The arc holder composes exactly two sites: our arc and the pinned foreign root prim.
    assert_eq!(holder.sources.len(), 2);
    let foreign: Vec<_> = holder.sources.iter().filter(|s| !s.owned).collect();
    assert_eq!(foreign.len(), 1);
    assert_eq!(foreign[0].layer, pin().layer_name());
    assert_eq!(foreign[0].digest, pin().digest);
    assert_eq!(foreign[0].path, "/Site");
    // The foreign type shows through the typeless arc holder, unchanged.
    assert_eq!(holder.type_name.as_deref(), Some("Xform"));
    assert_eq!(
        prop(&prims, &format!("{}/Block", source_path(WRAPPER)), "note"),
        Some(&Value::Text("declared by the source".into()))
    );
    let views = read_references(&prims).unwrap();
    assert_eq!(views.len(), 1);
    let view = &views[0];
    assert_eq!(view.id, WRAPPER);
    assert_eq!(view.name, "Architect site");
    assert_eq!(view.placement, pose());
    assert_eq!(view.source, pin());
    assert_eq!(view.units, None);
    assert_eq!(
        view.foreign
            .iter()
            .map(|f| f.source_path.as_str())
            .collect::<Vec<_>>(),
        ["/Site", "/Site/Block", "/Site/Block/Body"]
    );
    // Saved layers validate offline and a fresh author reproduces the same snapshot.
    let assets = exported(&a, &new.context);
    assert_eq!(
        named(&assets, &pin().layer_name()).bytes,
        FOREIGN.as_bytes()
    );
    assert!(validate(assets.clone()).is_ok());
    let (fresh, opened) = open(request(assets, &OWNED));
    assert_eq!(snapshot(&fresh, &opened.context), prims);
    // The index knows the wrapper as an owned element of kind reference.
    let index = a.element_index(&new.context, &control()).unwrap();
    let id = beamcraft_layer_engine::index::ElementId::parse(WRAPPER).unwrap();
    a.authorize_write(&index, &id, &control()).unwrap();
    assert_eq!(
        index.element(&id).unwrap().kind.as_deref(),
        Some("reference")
    );
}

#[test]
fn saved_owned_layers_are_deterministic_and_the_foreign_layer_is_never_among_them() {
    let mut expected = None;
    for _ in 0..5 {
        let (mut a, o) = open(initial());
        let p = a
            .prepare(batch(&o.context, vec![attach(WRAPPER)]), &control())
            .unwrap();
        let bytes: Vec<_> = p
            .layers
            .iter()
            .map(|l| (l.name.clone(), l.digest, l.bytes.clone()))
            .collect();
        assert!(bytes.iter().all(|(n, ..)| !n.starts_with("external/")));
        match &expected {
            None => expected = Some(bytes),
            Some(first) => assert_eq!(first, &bytes),
        }
    }
}

#[test]
fn moving_and_renaming_a_wrapper_never_touches_the_foreign_content() {
    let (mut a, o) = attached();
    let before = exported(&a, &o.context);
    let moved = Placement {
        translation_mm: [-5.5, 6.25, 7.0],
        orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
    };
    let p = a
        .prepare(
            batch(
                &o.context,
                vec![
                    Change::Placement {
                        id: WRAPPER.into(),
                        placement: moved.clone(),
                    },
                    Change::Label {
                        id: WRAPPER.into(),
                        name: "Moved site".into(),
                    },
                ],
            ),
            &control(),
        )
        .unwrap();
    assert_eq!(p.retained.len(), 1);
    assert_eq!(p.retained[0].bytes, FOREIGN.as_bytes());
    let authored = p.layers.iter().find(|l| l.name == "authored.usda").unwrap();
    assert_ne!(authored.digest, named(&before, "authored.usda").digest);
    let new = install(&mut a, &o.context, &p);
    let prims = snapshot(&a, &new.context);
    let view = &read_references(&prims).unwrap()[0];
    assert_eq!(view.placement, moved);
    assert_eq!(view.name, "Moved site");
    assert_eq!(view.source, pin());
    // The composed foreign prims are the same prims with the same provenance.
    let holder = prims
        .iter()
        .find(|p| p.path == source_path(WRAPPER))
        .unwrap();
    assert!(
        holder
            .sources
            .iter()
            .any(|s| !s.owned && s.digest == pin().digest)
    );
    assert_eq!(
        named(&exported(&a, &new.context), &pin().layer_name()).bytes,
        FOREIGN.as_bytes()
    );
    // A move writes the placement and derived groups only, a rename the label only.
    let writes: Vec<_> = p.writes.iter().map(|f| f.group).collect();
    assert_eq!(writes, vec![Group::Label, Group::Placement, Group::Derived]);
}

#[test]
fn the_units_correction_is_an_explicit_operation_on_the_wrapper_only() {
    let (mut a, o) = attached();
    let set = |a: &mut Author, o: &Opened, i: Option<UnitsInterpretation>| {
        let p = a
            .prepare(
                batch(
                    &o.context,
                    vec![Change::ReferenceUnits {
                        id: WRAPPER.into(),
                        interpretation: i,
                    }],
                ),
                &control(),
            )
            .unwrap();
        assert_eq!(p.retained.len(), 1);
        assert_eq!(p.retained[0].bytes, FOREIGN.as_bytes());
        let groups: Vec<_> = p.writes.iter().map(|f| f.group).collect();
        assert_eq!(groups, vec![Group::ShapeInputs, Group::Derived]);
        let new = install(a, &o.context, &p);
        (p, new)
    };
    let correction = format!("{}/Correction", path(WRAPPER));
    // Metres, Y up: rotate +90 about X, scale 1000.
    let metres_y = UnitsInterpretation {
        meters_per_unit: 1.0,
        up_axis: UpAxis::Y,
    };
    let (p, o) = set(&mut a, &o, Some(metres_y));
    assert!(
        p.opinions
            .iter()
            .any(|i| i.attribute == "xformOp:scale:bimUnits")
    );
    let prims = snapshot(&a, &o.context);
    assert_eq!(
        prop(&prims, &correction, "xformOp:rotateX:bimUpAxis"),
        Some(&Value::Number(90.0))
    );
    assert_eq!(
        prop(&prims, &correction, "xformOp:scale:bimUnits"),
        Some(&Value::Array(vec![Value::Number(1000.0); 3]))
    );
    assert_eq!(
        prop(&prims, &correction, "xformOpOrder"),
        Some(&Value::Array(vec![
            Value::Text("xformOp:rotateX:bimUpAxis".into()),
            Value::Text("xformOp:scale:bimUnits".into())
        ]))
    );
    assert_eq!(
        prop(&prims, &correction, "bim:reference:correctionSourceDigest"),
        Some(&Value::Text(pin().digest.to_string()))
    );
    assert_eq!(read_references(&prims).unwrap()[0].units, Some(metres_y));
    // The wrapper's own placement and the source's own declarations are unchanged.
    assert_eq!(read_references(&prims).unwrap()[0].placement, pose());
    assert_eq!(read_references(&prims).unwrap()[0].source, pin());
    // Centimetres, Z up: scale only.
    let cm = UnitsInterpretation {
        meters_per_unit: 0.01,
        up_axis: UpAxis::Z,
    };
    let (_, o) = set(&mut a, &o, Some(cm));
    let prims = snapshot(&a, &o.context);
    assert_eq!(prop(&prims, &correction, "xformOp:rotateX:bimUpAxis"), None);
    assert_eq!(
        prop(&prims, &correction, "xformOp:scale:bimUnits"),
        Some(&Value::Array(vec![Value::Number(10.0); 3]))
    );
    // Millimetres, Z up: recorded, but no operation is authored.
    let mm = UnitsInterpretation {
        meters_per_unit: 0.001,
        up_axis: UpAxis::Z,
    };
    let (_, o) = set(&mut a, &o, Some(mm));
    let prims = snapshot(&a, &o.context);
    assert_eq!(prop(&prims, &correction, "xformOp:scale:bimUnits"), None);
    assert_eq!(
        prop(&prims, &correction, "xformOpOrder"),
        Some(&Value::Array(Vec::new()))
    );
    assert_eq!(read_references(&prims).unwrap()[0].units, Some(mm));
    // Clearing removes every record.
    let (_, o) = set(&mut a, &o, None);
    let prims = snapshot(&a, &o.context);
    assert_eq!(
        prop(&prims, &correction, "bim:reference:correctionMetersPerUnit"),
        None
    );
    assert_eq!(read_references(&prims).unwrap()[0].units, None);
    assert!(validate(exported(&a, &o.context)).is_ok());
}

#[test]
fn an_invalid_interpretation_is_refused_before_anything_is_authored() {
    let (mut a, o) = attached();
    let before = exported(&a, &o.context);
    for (meters_per_unit, code) in [
        (f64::NAN, ErrorCode::NonFiniteNumber),
        (f64::INFINITY, ErrorCode::NonFiniteNumber),
        (0.0, ErrorCode::InvalidArgument),
        (-1.0, ErrorCode::InvalidArgument),
        (1e7, ErrorCode::LimitExceeded),
    ] {
        let error = a
            .prepare(
                batch(
                    &o.context,
                    vec![Change::ReferenceUnits {
                        id: WRAPPER.into(),
                        interpretation: Some(UnitsInterpretation {
                            meters_per_unit,
                            up_axis: UpAxis::Y,
                        }),
                    }],
                ),
                &control(),
            )
            .unwrap_err();
        refused(error, code);
    }
    let after = exported(&a, &o.context);
    assert_eq!(
        before.iter().map(|a| a.digest).collect::<Vec<_>>(),
        after.iter().map(|a| a.digest).collect::<Vec<_>>()
    );
}

#[test]
fn every_write_aimed_at_foreign_content_is_refused() {
    let (a, o) = attached();
    let prims = snapshot(&a, &o.context);
    let mut reader = Reader::default();
    let mut assets = exported(&a, &o.context);
    assets.sort_by(|x, y| x.name.cmp(&y.name));
    let opened = reader.open(request(assets, &OWNED), &control()).unwrap();
    // Every foreign prim, the arc holder included, is FOREIGN_READ_ONLY.
    for foreign in &read_references(&prims).unwrap()[0].foreign {
        refused(
            reader
                .refuse_write(&opened.context, &foreign.path)
                .unwrap_err(),
            ErrorCode::ForeignReadOnly,
        );
    }
    // The foreign layer is a read-only layer; the owned ones are merely unavailable in a read view.
    refused(
        reader
            .refuse_layer_write(&opened.context, &pin().layer_name())
            .unwrap_err(),
        ErrorCode::ForeignReadOnly,
    );
    refused(
        reader
            .refuse_layer_write(&opened.context, "authored.usda")
            .unwrap_err(),
        ErrorCode::UnsupportedCommand,
    );
    refused(
        reader
            .refuse_layer_write(&opened.context, "nothing.usda")
            .unwrap_err(),
        ErrorCode::NotFound,
    );
    // Naming the foreign layer as an edit target is refused before any layer is read.
    let mut as_target = targets();
    as_target.authored = pin().layer_name();
    let error = Author::open(
        request(exported(&a, &o.context), &OWNED),
        as_target,
        schema_identity().unwrap(),
        &control(),
    )
    .err()
    .unwrap();
    refused(error, ErrorCode::ForeignReadOnly);
    // Element-level commands that need parametric content do not apply to a wrapper.
    let (mut a, o) = (a, o);
    for change in [
        Change::Delete { id: WRAPPER.into() },
        Change::Thickness {
            id: WRAPPER.into(),
            mm: 5.0,
        },
        Change::Material {
            id: WRAPPER.into(),
            opinion: MaterialOpinion::Empty,
        },
    ] {
        let error = a
            .prepare(batch(&o.context, vec![change]), &control())
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedProperty, "{error}");
    }
}

#[test]
fn hand_edited_layers_with_foreign_opinions_are_refused_by_validation() {
    let (a, o) = attached();
    let assets = exported(&a, &o.context);
    assert!(validate(assets.clone()).is_ok());
    let holder = source_path(WRAPPER);
    type Edit = fn(&mut sdf::Data, &str);
    let cases: Vec<(&str, &str, Edit)> = vec![
        ("an over of a foreign child", "authored.usda", |d, h| {
            let p = over(d, &format!("{h}/Block"));
            d.set_field(&p, "active", sdf::Value::Bool(false));
        }),
        (
            "an over deep in the foreign tree",
            "authored.usda",
            |d, h| {
                over(d, &format!("{h}/Block/Body"));
            },
        ),
        ("a property on a foreign prim", "authored.usda", |d, h| {
            let p = attribute_spec(d, &format!("{h}/Block.note"));
            d.set_field(&p, "default", sdf::Value::String("edited".into()));
        }),
        (
            "a new child below the foreign root",
            "authored.usda",
            |d, h| {
                let p = def(d, &format!("{h}/Extra"));
                d.set_field(&p, "specifier", sdf::Value::Specifier(sdf::Specifier::Def));
            },
        ),
        ("a property on the arc holder", "authored.usda", |d, h| {
            let p = attribute_spec(d, &format!("{h}.visibility"));
            d.set_field(&p, "default", sdf::Value::token("invisible"));
        }),
        (
            "a type opinion on the arc holder",
            "authored.usda",
            |d, h| {
                let p = sdf::Path::new(h).unwrap();
                d.set_field(&p, "typeName", sdf::Value::token("Scope"));
            },
        ),
        (
            "a kind opinion on the arc holder",
            "authored.usda",
            |d, h| {
                let p = sdf::Path::new(h).unwrap();
                d.set_field(&p, "kind", sdf::Value::token("component"));
            },
        ),
        (
            "an override in the derived layer",
            "derived.usda",
            |d, h| {
                let p = over(d, h);
                d.set_field(&p, "active", sdf::Value::Bool(false));
            },
        ),
        (
            "a deep override in the derived layer",
            "derived.usda",
            |d, h| {
                over(d, &format!("{h}/Block"));
            },
        ),
        ("an override in the root layer", "root.usda", |d, h| {
            over(d, &format!("{h}/Block"));
        }),
    ];
    for (what, layer_name, edit) in cases {
        let bad = tamper(&assets, layer_name, |d| edit(d, &holder));
        let error = validate(bad).unwrap_err();
        assert_eq!(error.code, ErrorCode::ForeignReadOnly, "{what}: {error}");
    }
}

#[test]
fn hand_edited_wrapper_data_that_does_not_match_its_pin_is_refused() {
    let (a, o) = attached();
    let assets = exported(&a, &o.context);
    let element = path(WRAPPER);
    let attribute = |d: &mut sdf::Data, at: &str, name: &str, ty: &str, v: sdf::Value| {
        let p = attribute_spec(d, &format!("{at}.{name}"));
        d.set_field(&p, "typeName", sdf::Value::token(ty));
        d.set_field(&p, "default", v);
    };
    // An operation the recorded interpretation does not produce.
    let bad = tamper(&assets, "authored.usda", |d| {
        attribute(
            d,
            &format!("{element}/Correction"),
            "xformOp:scale:bimUnits",
            "double3",
            sdf::Value::vec3d(25.4, 25.4, 25.4),
        );
        let p = sdf::Path::new(&format!("{element}/Correction.xformOpOrder")).unwrap();
        d.set_field(
            &p,
            "default",
            sdf::Value::token_vec(["xformOp:scale:bimUnits"]),
        );
    });
    refused(validate(bad).unwrap_err(), ErrorCode::InvalidArgument);
    // A pin that names bytes the arc does not compose.
    let bad = tamper(&assets, "authored.usda", |d| {
        attribute(
            d,
            &element,
            "bim:reference:sourceDigest",
            "string",
            sdf::Value::String(digest(b"other bytes").to_string()),
        );
    });
    refused(validate(bad).unwrap_err(), ErrorCode::IntegrityError);
    // A units interpretation made for another version of the source.
    let (mut a, o) = (a, o);
    let p = a
        .prepare(
            batch(
                &o.context,
                vec![Change::ReferenceUnits {
                    id: WRAPPER.into(),
                    interpretation: Some(UnitsInterpretation {
                        meters_per_unit: 1.0,
                        up_axis: UpAxis::Y,
                    }),
                }],
            ),
            &control(),
        )
        .unwrap();
    let o = install(&mut a, &o.context, &p);
    let bad = tamper(&exported(&a, &o.context), "authored.usda", |d| {
        attribute(
            d,
            &format!("{element}/Correction"),
            "bim:reference:correctionSourceDigest",
            "string",
            sdf::Value::String(digest(b"older version").to_string()),
        );
    });
    refused(validate(bad).unwrap_err(), ErrorCode::IntegrityError);
    // Another prim under the wrapper.
    let bad = tamper(&assets, "authored.usda", |d| {
        let p = def(d, &format!("{element}/Geometry"));
        d.set_field(&p, "specifier", sdf::Value::Specifier(sdf::Specifier::Def));
    });
    refused(validate(bad).unwrap_err(), ErrorCode::InvalidArgument);
    // An arc holder with no arc composes no source.
    let bad = tamper(&assets, "authored.usda", |d| {
        let p = sdf::Path::new(&source_path(WRAPPER)).unwrap();
        d.erase_field(&p, "references");
    });
    refused(validate(bad).unwrap_err(), ErrorCode::DependencyUnavailable);
}

#[test]
fn attach_refuses_sources_that_are_not_foreign_or_not_exactly_pinned() {
    let attach_error = |request: OpenRequest, change: Change| {
        let (mut a, o) = open(request);
        a.prepare(batch(&o.context, vec![change]), &control())
            .unwrap_err()
    };
    // The source layer is selected as owned: ownership is the host's choice, not the pin's.
    let mut owned = OWNED.to_vec();
    let name = pin().layer_name();
    owned.push(&name);
    let error = attach_error(request(initial().assets, &owned), attach(WRAPPER));
    refused(error, ErrorCode::ForeignReadOnly);
    // The source layer is absent from the manifest.
    let mut without = initial();
    without.assets.retain(|a| a.name != name);
    without.manifest = without.manifest_digest().unwrap();
    refused(
        attach_error(without, attach(WRAPPER)),
        ErrorCode::DependencyUnavailable,
    );
    // The pinned size differs from the stored bytes.
    let mut short = pin();
    short.bytes -= 1;
    refused(
        attach_error(
            initial(),
            Change::AttachReference(Reference {
                id: WRAPPER.into(),
                name: "x".into(),
                placement: pose(),
                source: short,
            }),
        ),
        ErrorCode::IntegrityError,
    );
    // The default prim the pin names is not defined by the source: nothing composes.
    let mut wrong = pin();
    wrong.default_prim = "Missing".into();
    let mut with_wrong = initial();
    with_wrong.assets.retain(|a| a.name != name);
    with_wrong.assets.push(asset(&wrong.layer_name(), FOREIGN));
    with_wrong.manifest = with_wrong.manifest_digest().unwrap();
    let error = attach_error(
        with_wrong,
        Change::AttachReference(Reference {
            id: WRAPPER.into(),
            name: "x".into(),
            placement: pose(),
            source: wrong,
        }),
    );
    assert!(
        matches!(
            error.code,
            ErrorCode::DependencyUnavailable | ErrorCode::BackendUnavailable
        ),
        "{error}"
    );
    // Value rules.
    for (what, change, code) in [
        (
            "label",
            Change::AttachReference(Reference {
                id: WRAPPER.into(),
                name: "tab\there".into(),
                placement: pose(),
                source: pin(),
            }),
            ErrorCode::InvalidArgument,
        ),
        (
            "placement",
            Change::AttachReference(Reference {
                id: WRAPPER.into(),
                name: "x".into(),
                placement: Placement {
                    translation_mm: [f64::NAN, 0.0, 0.0],
                    orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
                },
                source: pin(),
            }),
            ErrorCode::NonFiniteNumber,
        ),
        (
            "id",
            Change::AttachReference(Reference {
                id: "not-a-uuid".into(),
                name: "x".into(),
                placement: pose(),
                source: pin(),
            }),
            ErrorCode::InvalidId,
        ),
    ] {
        let error = attach_error(initial(), change);
        assert_eq!(error.code, code, "{what}: {error}");
    }
    // A second wrapper with the same id.
    let (mut a, o) = attached();
    let error = a
        .prepare(batch(&o.context, vec![attach(WRAPPER)]), &control())
        .unwrap_err();
    refused(error, ErrorCode::DuplicateId);
}

#[test]
fn two_wrappers_may_share_one_stored_source_and_stay_independent() {
    let (mut a, o) = attached();
    let p = a
        .prepare(batch(&o.context, vec![attach(OTHER)]), &control())
        .unwrap();
    assert_eq!(p.retained.len(), 1);
    let o = install(&mut a, &o.context, &p);
    let prims = snapshot(&a, &o.context);
    let views = read_references(&prims).unwrap();
    assert_eq!(views.len(), 2);
    assert!(
        views
            .iter()
            .all(|v| v.source == pin() && v.foreign.len() == 3)
    );
    // Only the layer is shared; a correction on one wrapper leaves the other untouched.
    let p = a
        .prepare(
            batch(
                &o.context,
                vec![Change::ReferenceUnits {
                    id: OTHER.into(),
                    interpretation: Some(UnitsInterpretation {
                        meters_per_unit: 1.0,
                        up_axis: UpAxis::Y,
                    }),
                }],
            ),
            &control(),
        )
        .unwrap();
    let o = install(&mut a, &o.context, &p);
    let views = read_references(&snapshot(&a, &o.context)).unwrap();
    assert_eq!(views.iter().filter(|v| v.units.is_some()).count(), 1);
    assert!(validate(exported(&a, &o.context)).is_ok());
}

#[test]
fn foreign_layer_inspection_reports_declarations_and_refuses_unsafe_content() {
    let ctl = control();
    let info = inspect_foreign_layer(FOREIGN.as_bytes(), Budgets::FIRST_SLICE, &ctl).unwrap();
    assert_eq!(info.default_prim.as_deref(), Some("Site"));
    assert_eq!(info.meters_per_unit, Some(1.0));
    assert_eq!(info.up_axis.as_deref(), Some("Y"));
    assert_eq!(info.prim_count, 3);
    assert_eq!(info.format, SourceFormat::Usda);
    // A source that declares nothing is reported as declaring nothing.
    let bare = "#usda 1.0\n(\n    defaultPrim = \"A\"\n)\ndef Scope \"A\" {}\n";
    let info = inspect_foreign_layer(bare.as_bytes(), Budgets::FIRST_SLICE, &ctl).unwrap();
    assert_eq!((info.meters_per_unit, info.up_axis.clone()), (None, None));
    let none = "#usda 1.0\ndef Scope \"A\" {}\n";
    let info = inspect_foreign_layer(none.as_bytes(), Budgets::FIRST_SLICE, &ctl).unwrap();
    assert_eq!(info.default_prim, None);
    let check = |text: &str, code: ErrorCode| {
        let error = inspect_foreign_layer(text.as_bytes(), Budgets::FIRST_SLICE, &ctl).unwrap_err();
        assert_eq!(error.code, code, "{text:?}: {error}");
    };
    // URLs, escaping identifiers and arcs outside the static profile are inert in a displayed
    // source: nothing is resolved, and the rest of the layer is admitted.
    for text in [
        "#usda 1.0\ndef \"A\" (references = @https://example.invalid/model.usd@</X>)\n{\n}\n",
        "#usda 1.0\n(subLayers = [@omniverse://server/model.usd@])\n",
        "#usda 1.0\ndef \"A\" (payload = @https://example.invalid/m.usd@</X>)\n{\n}\n",
        "#usda 1.0\ndef \"A\" (payload = @local.usda@</X>)\n{\n}\n",
        "#usda 1.0\ndef \"A\" (variantSets = [\"v\"])\n{\n}\n",
        "#usda 1.0\n(subLayers = [@other.usda@])\n",
        "#usda 1.0\ndef \"A\" (references = @other.usda@</X>)\n{\n}\n",
    ] {
        assert!(
            inspect_foreign_layer(text.as_bytes(), Budgets::FIRST_SLICE, &ctl).is_ok(),
            "{text:?}"
        );
    }
    // Numbers and the default prim.
    check(
        "#usda 1.0\n(metersPerUnit = inf)\ndef Scope \"A\" {}\n",
        ErrorCode::NonFiniteNumber,
    );
    check(
        "#usda 1.0\n(defaultPrim = \"B\")\ndef Scope \"A\" {}\n",
        ErrorCode::NotFound,
    );
    check(
        "#usda 1.0\n(defaultPrim = \"A\")\nover Scope \"A\" {}\n",
        ErrorCode::NotFound,
    );
    // Bytes that are not a layer.
    check("", ErrorCode::IntegrityError);
    check("not a layer", ErrorCode::IntegrityError);
    check("#usda 1.0\n(", ErrorCode::IntegrityError);
    check("PK\u{3}\u{4}zip", ErrorCode::UnsupportedProperty);
    let big = format!("#usda 1.0\n# {}\n", "x".repeat(70_000));
    check(&big, ErrorCode::LimitExceeded);
    // A cancelled request does no work.
    let cancelled = control();
    cancelled.cancellation.cancel();
    assert_eq!(
        inspect_foreign_layer(FOREIGN.as_bytes(), Budgets::FIRST_SLICE, &cancelled)
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
}

#[test]
fn a_binary_foreign_layer_is_inspected_attached_and_kept_byte_for_byte() {
    // The restricted binary profile reads scalars and selected vectors: this fixture fits it.
    let text = r#"#usda 1.0
(
    defaultPrim = "Site"
    metersPerUnit = 0.01
    upAxis = "Z"
)
def Xform "Site"
{
    custom string note = "binary source"
    def Scope "Room" {}
}
"#;
    let data = openusd::usda::parse(text).unwrap();
    let mut buffer = std::io::Cursor::new(Vec::new());
    openusd::usdc::CrateWriter::write(&data, &mut buffer).unwrap();
    let bytes = buffer.into_inner();
    let info = inspect_foreign_layer(&bytes, Budgets::FIRST_SLICE, &control()).unwrap();
    assert_eq!(info.format, SourceFormat::Usdc);
    assert_eq!(info.default_prim.as_deref(), Some("Site"));
    assert_eq!(info.meters_per_unit, Some(0.01));
    let mut binary_pin = pin_of(text);
    binary_pin.digest = digest(&bytes);
    binary_pin.bytes = bytes.len() as u64;
    binary_pin.format = SourceFormat::Usdc;
    binary_pin.declared_meters_per_unit = Some(0.01);
    binary_pin.declared_up_axis = Some(UpAxis::Z);
    let mut input = initial();
    input.assets.retain(|a| a.name != pin().layer_name());
    input.assets.push(asset(&binary_pin.layer_name(), &bytes));
    input.manifest = input.manifest_digest().unwrap();
    let (mut a, o) = open(input);
    let p = a
        .prepare(
            batch(
                &o.context,
                vec![Change::AttachReference(Reference {
                    id: WRAPPER.into(),
                    name: "binary".into(),
                    placement: pose(),
                    source: binary_pin.clone(),
                })],
            ),
            &control(),
        )
        .unwrap();
    assert_eq!(p.retained.len(), 1);
    assert_eq!(p.retained[0].bytes, bytes);
    assert!(binary_pin.layer_name().ends_with(".usdc"));
    let o = install(&mut a, &o.context, &p);
    let prims = snapshot(&a, &o.context);
    let view = &read_references(&prims).unwrap()[0];
    assert_eq!(view.source, binary_pin);
    assert_eq!(view.foreign.len(), 2);
    assert!(validate(exported(&a, &o.context)).is_ok());
}

#[test]
fn a_binary_layer_outside_the_restricted_profile_fails_closed_with_a_diagnostic() {
    // Vector arrays are inside the restricted profile only when uncompressed and small; the
    // mesh fixture is expected to be refused or admitted, never to crash or to be half-read.
    let data = openusd::usda::parse(FOREIGN).unwrap();
    let mut buffer = std::io::Cursor::new(Vec::new());
    openusd::usdc::CrateWriter::write(&data, &mut buffer).unwrap();
    let bytes = buffer.into_inner();
    match inspect_foreign_layer(&bytes, Budgets::FIRST_SLICE, &control()) {
        Ok(info) => assert_eq!(info.format, SourceFormat::Usdc),
        Err(error) => assert!(
            matches!(
                error.code,
                ErrorCode::UnsupportedProperty | ErrorCode::LimitExceeded
            ),
            "{error}"
        ),
    }
    // A truncated binary layer is an integrity error before anything else.
    let cut = bytes.get(..bytes.len() / 2).unwrap();
    assert_eq!(
        inspect_foreign_layer(cut, Budgets::FIRST_SLICE, &control())
            .unwrap_err()
            .code,
        ErrorCode::IntegrityError
    );
}

#[test]
fn layers_pinned_to_the_p02_2_mapping_are_refused() {
    // The digest of the P02.2 descriptor (master before R01): R01 only adds the wrapper.
    let old = "960c23cae54987cab86028cde90e360eb44ed64f8b6847f3c4f98b02d630ef56";
    let pinned = |name: &str, pin: &str| asset(name, pinned_text(pin));
    let mut input = initial();
    for name in ["authored.usda", "derived.usda"] {
        input.assets.retain(|a| a.name != name);
        input.assets.push(pinned(name, old));
    }
    input.manifest = input.manifest_digest().unwrap();
    assert_eq!(
        validate_layers(input.clone(), schema_identity().unwrap(), &control())
            .unwrap_err()
            .code,
        ErrorCode::UnsupportedVersion
    );
    assert_eq!(
        Author::open(input, targets(), schema_identity().unwrap(), &control())
            .err()
            .unwrap()
            .code,
        ErrorCode::UnsupportedVersion
    );
    // An unknown pin is still refused.
    let mut input = initial();
    input.assets.retain(|a| a.name != "authored.usda");
    input.assets.push(pinned("authored.usda", "abc"));
    input.manifest = input.manifest_digest().unwrap();
    let error = Author::open(input, targets(), schema_identity().unwrap(), &control())
        .err()
        .unwrap();
    assert_eq!(error.code, ErrorCode::UnsupportedVersion);
}

#[test]
fn an_arc_holder_that_is_not_a_plain_def_is_refused() {
    let (a, o) = attached();
    let assets = exported(&a, &o.context);
    let holder = source_path(WRAPPER);
    for specifier in [sdf::Specifier::Over, sdf::Specifier::Class] {
        {
            let layer_name = "authored.usda";
            let bad = tamper(&assets, layer_name, |d| {
                let p = sdf::Path::new(&holder).unwrap();
                d.set_field(&p, "specifier", sdf::Value::Specifier(specifier));
            });
            let error = validate(bad).unwrap_err();
            assert_eq!(
                error.code,
                ErrorCode::ForeignReadOnly,
                "{specifier:?}: {error}"
            );
        }
    }
    // An `over` of the holder in another owned layer is an opinion on the foreign root too.
    let bad = tamper(&assets, "derived.usda", |d| {
        over(d, &holder);
    });
    assert_eq!(validate(bad).unwrap_err().code, ErrorCode::ForeignReadOnly);
}

#[test]
fn half_faked_wrappers_are_refused_and_never_owned() {
    let (a, o) = attached();
    let assets = exported(&a, &o.context);
    let element = path(WRAPPER);
    let set = |d: &mut sdf::Data, name: &str, value: &str| {
        let p = sdf::Path::new(&format!("{element}.{name}")).unwrap();
        d.set_field(&p, "default", sdf::Value::token(value));
    };
    // Same arc, other geometry mode: it would skip the wrapper validator and the spec scan.
    let parametric = tamper(&assets, "authored.usda", |d| {
        set(d, "bim:geometryMode", "parametric");
    });
    assert_eq!(
        validate(parametric.clone()).unwrap_err().code,
        ErrorCode::ForeignReadOnly
    );
    let error = Author::open(
        request(parametric.clone(), &OWNED),
        targets(),
        schema_identity().unwrap(),
        &control(),
    )
    .err()
    .unwrap();
    assert_eq!(error.code, ErrorCode::ForeignReadOnly);
    // A reader (which does not run the U02 validator) still never calls it owned.
    let mut reader = Reader::default();
    let opened = reader
        .open(request(parametric, &OWNED), &control())
        .unwrap();
    let index = reader.element_index(&opened.context, &control()).unwrap();
    let id = beamcraft_layer_engine::index::ElementId::parse(WRAPPER).unwrap();
    assert_eq!(
        index.element(&id).unwrap().ownership,
        beamcraft_layer_engine::index::Ownership::Foreign
    );
    // A part kind at a reference path is rejected by the reader's U16 category check,
    // before the wrapper ownership validator runs.
    for (name, value) in [("bim:elementKind", "plate"), ("bim:elementKind", "beam")] {
        let bad = tamper(&assets, "authored.usda", |d| set(d, name, value));
        assert_eq!(
            validate(bad).unwrap_err().code,
            ErrorCode::InvalidId,
            "{value}"
        );
    }
    // Kind reference, a mode that is neither: the holder is stray.
    let bad = tamper(&assets, "authored.usda", |d| {
        set(d, "bim:geometryMode", "mesh")
    });
    assert_eq!(validate(bad).unwrap_err().code, ErrorCode::ForeignReadOnly);
    // An arc on a `Correction/Source` path below a prim that is no wrapper at all.
    let bad = tamper(&assets, "authored.usda", |d| {
        let holder = "/World/Fake/Correction/Source";
        def(d, holder);
        let p = sdf::Path::new(holder).unwrap();
        d.set_field(
            &p,
            "references",
            sdf::Value::ReferenceListOp(sdf::ListOp::prepended([sdf::Reference {
                asset_path: pin().layer_name(),
                prim_path: sdf::Path::new("/Site").unwrap(),
                layer_offset: sdf::LayerOffset::IDENTITY,
                custom_data: Default::default(),
            }])),
        );
    });
    assert_eq!(validate(bad).unwrap_err().code, ErrorCode::ForeignReadOnly);
    // The untouched layers remain valid.
    assert!(validate(assets).is_ok());
}
