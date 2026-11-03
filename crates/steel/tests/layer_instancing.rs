// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic layers, MPL-2.0; existing test-data attribution applies.

//! U11 instancing of shared geometry: every derived body is an immutable, content-addressed
//! asset prim under `/World/Definitions/Geometry/GEO_<sha256>`, and every element's
//! `Geometry` prim is an `instanceable` reference to it. Equal bodies share one asset, so
//! many copies compose one prototype; an edit publishes a new asset only for the changed
//! body and never rewrites a shared one.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use beamcraft_layer_engine::{authored::*, *};
use beamcraft_model::digest::{ContentDigest, Sha256};
use beamcraft_model::error::ErrorCode;
use std::time::{Duration, Instant};
use steel::layer as steel_layer;
use steel::layer::*;

const ACTOR: &str = "0199e7a0-0000-7000-8000-000000000005";
const OP: &str = "0199e7a0-0000-7000-8000-000000000006";
fn id(n: usize) -> String {
    format!("0199e7a0-0000-7000-8000-{n:012x}")
}
fn digest(bytes: &[u8]) -> ContentDigest {
    Sha256::digest(bytes).unwrap()
}
fn control() -> Control {
    steel_layer::install();
    Control {
        cancellation: Cancellation::default(),
        deadline: Instant::now() + Duration::from_secs(120),
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
            asset("authored.usda", "#usda 1.0\n"),
            asset("derived.usda", "#usda 1.0\n"),
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
fn plate(n: usize) -> Change {
    Change::Create(Element {
        id: id(n),
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
/// One synthetic body; identical bodies carry one identical engine shape key, so their
/// canonical asset content — key plus buffers — matches exactly.
fn body(n: usize, height: f32) -> Mesh {
    Mesh {
        id: id(n),
        shape_key: digest(format!("shape-{height}").as_bytes()),
        points_mm: vec![
            [0.0, 0.0, 0.0],
            [100.0, 0.0, 0.0],
            [100.0, 100.0, height],
            [0.0, 100.0, height],
        ],
        face_counts: vec![4],
        indices: vec![0, 1, 2, 3],
        quantities: None,
        reduced: None,
        proxy: None,
        axis: None,
    }
}
/// Prepares `changes` over `author`'s live view and returns the candidate request bytes.
fn run(author: &mut Author, opened: &Opened, changes: Vec<Change>) -> (OpenRequest, Prepared) {
    let schema = schema_identity().unwrap();
    let prepared = author
        .prepare(
            Prepare {
                context: opened.context.clone(),
                op_id: OP.into(),
                actor: ACTOR.into(),
                schema,
                resources: Vec::new(),
                changes,
            },
            &control(),
        )
        .unwrap();
    let mut request = empty();
    request.assets = prepared.layers.clone();
    // Prepared.layers are exactly the owned assets of the candidate (U15 adds element files
    // and the referenced asset layer to the owned set).
    request.owned_layers = prepared
        .layers
        .iter()
        .map(|a| a.name.clone())
        .collect::<std::collections::BTreeSet<_>>();
    request.manifest = request.manifest_digest().unwrap();
    (request, prepared)
}
fn install(author: &mut Author, opened: &Opened, prepared: &Prepared) -> Opened {
    author
        .install(
            &opened.context,
            &prepared.token,
            &prepared.op_id,
            Decision::Accepted,
            &control(),
        )
        .unwrap()
}
/// The bytes of one owned layer of the candidate, as UTF-8 (U15 splits the derived scope:
/// bodies live in the referenced asset layer, element prims in per-element files).
fn layer_text(head: &OpenRequest, name: &str) -> String {
    std::str::from_utf8(&head.assets.iter().find(|a| a.name == name).unwrap().bytes)
        .unwrap()
        .to_owned()
}
/// The text of every per-element derived layer of the candidate, in manifest order.
fn element_texts(head: &OpenRequest) -> Vec<String> {
    head.assets
        .iter()
        .filter(|a| element_layer_id(&a.name).is_some())
        .map(|a| layer_text(head, &a.name))
        .collect()
}
fn snapshot(author: &Author, context: &Context) -> Vec<Prim> {
    let mut prims = Vec::new();
    let mut cursor = None;
    loop {
        let page = author.read(context, cursor.as_ref(), &control()).unwrap();
        prims.extend(page.items);
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    prims
}

#[test]
fn identical_bodies_in_one_batch_share_one_immutable_asset() {
    let (mut author, opened) = Author::open(
        empty(),
        targets(),
        None,
        schema_identity().unwrap(),
        &control(),
    )
    .unwrap();
    let (head, prepared) = run(
        &mut author,
        &opened,
        vec![
            plate(1),
            plate(2),
            Change::Mesh(body(1, 5.0)),
            Change::Mesh(body(2, 5.0)),
        ],
    );
    // The shared body asset lives once in the referenced asset layer (U15); the per-element
    // instanceable references compose it from there.
    let assets = layer_text(&head, "derived/geometry.usda");
    assert_eq!(assets.matches("def Mesh \"GEO_").count(), 1, "{assets}");
    let elements = element_texts(&head);
    assert_eq!(elements.len(), 2, "{elements:?}");
    assert_eq!(
        elements
            .iter()
            .map(|t| t.matches("instanceable = true").count())
            .sum::<usize>(),
        2
    );
    let asset_digest = mesh_asset_digest(&body(1, 5.0)).unwrap();
    let asset_path = format!("/World/Definitions/Geometry/GEO_{asset_digest}");
    assert!(
        elements
            .iter()
            .all(|t| t.contains(&format!("references = @{}@<{asset_path}>", "geometry.usda")))
    );
    // The element's own prim authors no buffers.
    assert!(elements.iter().all(|t| !t.contains("/Geometry.points")));
    // The writes carry the asset's content identity (U11 evidence).
    let identity = asset_digest.to_string();
    assert!(prepared.writes.contains(&Footprint {
        identity: identity.clone(),
        group: Group::Derived,
    }));
    assert!(prepared.reads.contains(&Footprint {
        identity,
        group: Group::Derived,
    }));
    let live = install(&mut author, &opened, &prepared);
    // The composed snapshot carries the shared buffers through both references.
    let composed = snapshot(&author, &live.context);
    for n in [1, 2] {
        let wrapper = composed
            .iter()
            .find(|p| {
                p.path == format!("/World/Steel/Parts/PRT_{}/Geometry", id(n).replace('-', ""))
            })
            .unwrap();
        assert!(wrapper.instanceable);
    }
    let geometry = composed
        .iter()
        .find(|p| p.path == "/World/Steel/Parts/PRT_0199e7a0000070008000000000000001/Geometry")
        .unwrap();
    let asset_prim = composed.iter().find(|p| p.path == asset_path).unwrap();
    assert_eq!(
        geometry
            .properties
            .iter()
            .find(|p| p.name == "points")
            .map(|p| p.value.as_ref().unwrap()),
        asset_prim
            .properties
            .iter()
            .find(|p| p.name == "points")
            .map(|p| p.value.as_ref().unwrap())
    );
    // A tampered asset name no longer recomposes from its content (the bodies live in the
    // referenced asset layer, U15).
    // A tampered asset name no longer recomposes from its content (U15): the body lives in
    // the referenced asset layer, and every per-element reference is relabelled with it, so
    // the naming still recomposes and the digest identity is what fails.
    let other = digest(b"other").to_string();
    let mut forged = head.clone();
    for a in forged.assets.iter_mut() {
        if a.name == "derived/geometry.usda" || element_layer_id(&a.name).is_some() {
            let text = String::from_utf8_lossy(&a.bytes).into_owned();
            let needle = format!("GEO_{asset_digest}");
            println!(
                "U15DBG {} contains={} len={}",
                a.name,
                text.contains(&needle),
                text.len()
            );
            let relabelled = text.replace(&needle, &format!("GEO_{other}"));
            *a = asset(&a.name, &relabelled);
        }
    }
    forged.manifest = forged.manifest_digest().unwrap();
    let mut reader = Reader::default();
    assert_eq!(
        reader
            .open_validated(forged, schema_identity().unwrap(), &control())
            .err()
            .unwrap()
            .code,
        ErrorCode::InvalidId
    );
}

#[test]
fn a_new_body_publishes_one_new_asset_and_never_rewrites_the_shared_one() {
    let (mut author, opened) = Author::open(
        empty(),
        targets(),
        None,
        schema_identity().unwrap(),
        &control(),
    )
    .unwrap();
    let (head, prepared) = run(
        &mut author,
        &opened,
        vec![plate(1), Change::Mesh(body(1, 5.0))],
    );
    let before = layer_text(&head, "derived/geometry.usda");
    assert_eq!(before.matches("def Mesh \"GEO_").count(), 1);
    let live = install(&mut author, &opened, &prepared);
    // A different body for the second element: one new asset, the first untouched.
    let (head2, prepared2) = run(
        &mut author,
        &live,
        vec![plate(2), Change::Mesh(body(2, 12.0))],
    );
    let after = layer_text(&head2, "derived/geometry.usda");
    assert_eq!(after.matches("def Mesh \"GEO_").count(), 2);
    let old = mesh_asset_digest(&body(1, 5.0)).unwrap();
    let new = mesh_asset_digest(&body(2, 12.0)).unwrap();
    // The first asset's spec is carried verbatim, not rewritten.
    let start = before.find(&format!("def Mesh \"GEO_{old}")).unwrap();
    let end = before[start..].find("bim:shapeKey").unwrap() + start;
    let old_spec = before[start..end].to_owned();
    assert!(after.contains(&old_spec));
    assert!(after.contains(&format!("GEO_{new}")));
    let live = install(&mut author, &live, &prepared2);
    // Deleting the last referent keeps the immutable asset, like every pinned definition,
    // while its wrapper goes away.
    let (_, prepared3) = run(&mut author, &live, vec![Change::Delete { id: id(1) }]);
    let live = install(&mut author, &live, &prepared3);
    let composed = snapshot(&author, &live.context);
    assert!(
        !composed
            .iter()
            .any(|p| p.path == "/World/Steel/Parts/PRT_0199e7a0000070008000000000000001/Geometry")
    );
    assert!(composed.iter().any(|p| p.path.contains("/GEO_")));
}

#[test]
fn a_thousand_copies_share_one_prototype() {
    let (mut author, mut opened) = Author::open(
        empty(),
        targets(),
        None,
        schema_identity().unwrap(),
        &control(),
    )
    .unwrap();
    let mut next = 1usize;
    for _ in 0..10 {
        let mut changes = Vec::new();
        for _ in 0..100 {
            changes.push(plate(next));
            changes.push(Change::Mesh(body(next, 5.0)));
            next += 1;
        }
        let (head, prepared) = run(&mut author, &opened, changes);
        opened = install(&mut author, &opened, &prepared);
        let _ = head;
    }
    let exported = author.export(&opened.context, &control()).unwrap();
    let layer_named = |name: &str| {
        std::str::from_utf8(&exported.iter().find(|a| a.name == name).unwrap().bytes).unwrap()
    };
    let assets = layer_named("derived/geometry.usda");
    let element_texts: Vec<String> = exported
        .iter()
        .filter(|a| element_layer_id(&a.name).is_some())
        .map(|a| layer_named(&a.name).to_owned())
        .collect();
    // 1000 copies, one immutable geometry asset in the referenced layer (U15), 1000
    // instanceable references in the per-element files.
    assert_eq!(assets.matches("def Mesh \"GEO_").count(), 1, "{assets}");
    assert_eq!(
        element_texts
            .iter()
            .map(|t| t.matches("instanceable = true").count())
            .sum::<usize>(),
        1000
    );
    // The stage still opens, composes and validates within the model budgets.
    let mut request = empty();
    request.owned_layers = exported
        .iter()
        .map(|a| a.name.clone())
        .collect::<std::collections::BTreeSet<_>>();
    request.assets = exported;
    request.identity.generation += 1;
    request.manifest = request.manifest_digest().unwrap();
    let mut reader = Reader::default();
    let (opened, _) = reader
        .open_validated(request, schema_identity().unwrap(), &control())
        .unwrap();
    assert!(opened.prim_count > 2000);
}
