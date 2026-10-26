// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original logical-case adapter and USD fixtures, MPL-2.0.

//! Executes an explicit subset of logical F10 cases through the real USD index and copy planner.

#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
use beamcraft_layer_engine::{index::*, *};
use beamcraft_model::digest::Sha256;
use std::time::{Duration, Instant};

fn eid(id: &str) -> ElementId {
    ElementId::parse(id).unwrap()
}
fn el(id: &str, attrs: &str) -> String {
    format!(
        "def Xform \"PRT_{}\" {{ custom string bim:id = \"{id}\"\n{attrs}\n}}\n",
        id.replace('-', "")
    )
}
fn link(name: &str, id: &str) -> String {
    format!(
        "custom rel {name} = </World/Steel/Parts/PRT_{}>\n",
        id.replace('-', "")
    )
}
fn world(elements: &str) -> String {
    format!(
        "#usda 1.0\n(metersPerUnit=0.001\nupAxis=\"Z\")\ndef Xform \"World\" {{ def Scope \"Steel\" {{ def Scope \"Parts\" {{ {elements} }} }} }}"
    )
}
fn built(body: String) -> Result<ElementIndex, Diagnostic> {
    let hash = |bytes: &[u8]| Sha256::digest(bytes).unwrap();
    let mut request = OpenRequest {
        identity: Identity {
            project: "019a7bf0-0000-7000-8000-000000000001".into(),
            workspace: "019a7bf0-0000-7000-8000-000000000002".into(),
            revision: hash(b"case"),
            generation: 0,
        },
        root: "root.usda".into(),
        assets: vec![Asset {
            name: "root.usda".into(),
            digest: hash(body.as_bytes()),
            bytes: body.into_bytes(),
        }],
        owned_layers: ["root.usda".into()].into_iter().collect(),
        manifest: hash(b"placeholder"),
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
    let opened = reader.open(request, &control)?;
    reader.element_index(&opened.context, &control)
}

#[test]
fn supported_f10_logical_cases_drive_the_real_index_and_copy_planner() {
    use beamcraft_json::{Json, Limits};
    let fixture = beamcraft_json::decode(
        include_bytes!("../../../tests/fixtures/contracts/f10-domain-vocabulary-cases-v0.1.json"),
        &Limits {
            bytes: 1024 * 1024,
            ..Limits::DEFAULT
        },
    )
    .unwrap();
    // This is an explicit executable subset, not a claim of full F10 acceptance.
    let supported = [
        "DVC-BASE-004-P",
        "DVC-BASE-004-N",
        "DVC-REL-011-P",
        "DVC-REL-011-N",
    ];
    let mut ran = Vec::new();
    for case in fixture.get("cases").unwrap().array().unwrap() {
        let id = case.get("case_id").unwrap().string().unwrap();
        if !supported.contains(&id) {
            continue;
        }
        ran.push(id);
        let given = case.get("given").unwrap();
        let mut body = String::new();
        for element in given.get("elements").unwrap().array().unwrap() {
            let element_id = element.get("id").unwrap().string().unwrap();
            let kind = element.get("kind").unwrap().string().unwrap();
            let mut attrs = format!("custom token bim:elementKind = \"{kind}\"\n");
            if let Ok(value) = element.get("is_deactivated") {
                attrs.push_str(&match value {
                    Json::Bool(value) => format!("custom bool bim:isDeactivated = {value}\n"),
                    Json::String(value) => {
                        format!("custom string bim:isDeactivated = \"{value}\"\n")
                    }
                    _ => panic!("unsupported fixture deactivation value"),
                });
            }
            for rel in given.get("relationships").unwrap().array().unwrap() {
                if rel.get("source").unwrap().string().unwrap() == element_id {
                    let name = rel.get("type").unwrap().string().unwrap();
                    attrs.push_str(&link(
                        &format!("bim:{name}"),
                        rel.get("target").unwrap().string().unwrap(),
                    ));
                }
            }
            body.push_str(&el(element_id, &attrs));
        }
        let result = built(world(&body));
        let action = case.get("action").unwrap();
        let result = if action.get("type").unwrap().string().unwrap() == "copy" {
            result.and_then(|index| {
                index
                    .copy_plan(
                        &eid(action.get("source").unwrap().string().unwrap()),
                        &eid(action.get("new_id").unwrap().string().unwrap()),
                    )
                    .map(|_| ())
            })
        } else {
            result.map(|_| ())
        };
        let expected = case.get("expected").unwrap();
        let findings = expected.get("findings").unwrap().array().unwrap();
        if let Some(finding) = findings.first() {
            let error = result.unwrap_err();
            assert_eq!(
                error.rule_id,
                Some(finding.get("rule_id").unwrap().string().unwrap()),
                "{id}"
            );
            assert_eq!(
                error.code.as_str(),
                finding.get("code").unwrap().string().unwrap(),
                "{id}"
            );
        } else {
            result.unwrap();
        }
    }
    assert_eq!(ran, supported);
}
