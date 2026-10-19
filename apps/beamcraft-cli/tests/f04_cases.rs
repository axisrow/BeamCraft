// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original harness for the frozen original acceptance descriptions of F04 0.1.0
// (`docs/contracts/model-command-cases-v0.1.json`); no third-party material.

//! Drives every F04 `CreatePlate` case that can be decided without a workspace through the real
//! engine admission (`admit_create_plate`) and checks the expected code or outcome. The case file
//! is decoded with the bounded protocol JSON decoder; the arguments are mapped to the typed
//! command here, so a non-number is `INVALID_ARGUMENT` like the contract says. Cases that need
//! the envelope, a workspace or an authenticated actor are listed as skipped with the reason, and
//! the cases where the A02 slice deliberately differs from F04 are listed as deviations: the test
//! fails if either list drifts silently.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_engine::transaction::{Admission, PriorOp, admit_create_plate};
use beamcraft_model::command::{CreatePlate, Placement};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId};
use beamcraft_protocol::json::{self, Json};
use beamcraft_protocol::limits::MAXIMA;
use std::collections::{BTreeMap, BTreeSet};

/// Cases that need the F04 envelope, a workspace or a trusted caller: not implemented in P01.
const SKIPPED: &[(&str, &str)] = &[
    (
        "F04-NEG-07",
        "envelope contract_version (no envelope decoder yet)",
    ),
    (
        "F04-NEG-08",
        "base_revision precondition (needs P06 workspaces)",
    ),
    (
        "F04-NEG-09",
        "workspace generation precondition (needs P06 workspaces)",
    ),
    (
        "F04-NEG-17",
        "authenticated actor / PERMISSION_DENIED (no trusted context yet)",
    ),
    (
        "F04-NEG-18",
        "envelope expected_generation decoding (no envelope decoder yet)",
    ),
];

/// Accepted by F04, not evaluated by the A02 slice with UNSUPPORTED_GEOMETRY (the code the
/// first slice used, so no valid input gains a new refusal class): a contour that touches any
/// nonzero `z` stays unevaluated because the plate body frame is the local XY plane and the
/// placement carries translations; F04's exact planarity tolerance is the G06 policy.
const UNSUPPORTED_IN_A02: &[&str] = &["F04-POS-18", "F04-POS-28"];

/// Rejected by F04 with INVALID_ARGUMENT (non-planar contour) but reported by this slice as
/// UNSUPPORTED_GEOMETRY, like the first slice did: the exact planarity refusal needs the G06
/// tolerance policy and is not an argument check here.
const WRONG_CODE_IN_A02: &[&str] = &["F04-NEG-29", "F04-NEG-38"];

fn code_of(name: &str) -> ErrorCode {
    ErrorCode::parse(name).unwrap()
}

fn number(value: &Json) -> Option<f64> {
    match value {
        Json::Number(_) => value.float().ok(),
        _ => None,
    }
}

fn numbers(value: &Json, count: usize) -> Option<Vec<f64>> {
    let items = value.array().ok()?;
    let parsed: Vec<f64> = items.iter().map(number).collect::<Option<_>>()?;
    (parsed.len() == count).then_some(parsed)
}

/// Maps the JSON arguments to the typed command, or the F04 code of the structural failure.
fn command_of(arguments: &Json) -> Result<CreatePlate, ErrorCode> {
    let new_id = arguments
        .get("new_id")
        .ok()
        .and_then(|v| v.string().ok())
        .ok_or(ErrorCode::MalformedCommand)?;
    let new_id = ElementId::parse(new_id).map_err(|_| ErrorCode::InvalidId)?;
    let name = arguments
        .get("name")
        .ok()
        .and_then(|v| v.string().ok())
        .ok_or(ErrorCode::InvalidArgument)?;
    let contour = arguments
        .get("contour_points_mm")
        .ok()
        .and_then(|v| v.array().ok())
        .ok_or(ErrorCode::InvalidArgument)?;
    let contour = contour
        .iter()
        .map(|p| numbers(p, 3).map(|v| [v[0], v[1], v[2]]))
        .collect::<Option<Vec<_>>>()
        .ok_or(ErrorCode::InvalidArgument)?;
    let thickness = arguments
        .get("thickness_mm")
        .ok()
        .and_then(number)
        .ok_or(ErrorCode::InvalidArgument)?;
    let placement = arguments
        .get("placement")
        .map_err(|_| ErrorCode::InvalidArgument)?;
    let translation = placement
        .get("translation_mm")
        .ok()
        .and_then(|v| numbers(v, 3))
        .ok_or(ErrorCode::InvalidArgument)?;
    let orientation = placement
        .get("orientation_xyzw")
        .ok()
        .and_then(|v| numbers(v, 4))
        .ok_or(ErrorCode::InvalidArgument)?;
    Ok(CreatePlate {
        new_id,
        name: name.to_owned(),
        contour_points_mm: contour,
        thickness_mm: thickness,
        placement: Placement {
            translation_mm: [translation[0], translation[1], translation[2]],
            orientation_xyzw: [
                orientation[0],
                orientation[1],
                orientation[2],
                orientation[3],
            ],
        },
        material_digest: None,
        holes_mm: Vec::new(),
    })
}

#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Applied,
    Replayed,
    Rejected(ErrorCode),
}

fn run_case(case_id: &str, arguments: &Json, accepted: &BTreeMap<String, CreatePlate>) -> Verdict {
    let command = match command_of(arguments) {
        Ok(c) => c,
        Err(code) => return Verdict::Rejected(code),
    };
    let op = OpId::parse("019a7bf0-0000-7000-8000-000000000004").unwrap();
    let prior = match case_id {
        // The operation was accepted with the original POS-01 content.
        "F04-POS-04" | "F04-NEG-10" => PriorOp::Accepted {
            fingerprint: accepted["F04-POS-01"].content_fingerprint().unwrap(),
        },
        _ => PriorOp::Unseen,
    };
    let owned = command.new_id;
    let existing = move |id: &ElementId| case_id == "F04-NEG-03" && *id == owned;
    match admit_create_plate(
        op,
        &command,
        prior,
        &existing,
        &beamcraft_engine::command::no_materials,
    ) {
        Ok(Admission::Apply(_)) => Verdict::Applied,
        Ok(Admission::Replay) => Verdict::Replayed,
        Err(e) => Verdict::Rejected(e.code),
    }
}

#[test]
fn every_decidable_create_plate_case_matches_f04_or_is_a_declared_deviation() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/contracts/model-command-cases-v0.1.json"
    );
    let bytes = std::fs::read(path).unwrap();
    let file = json::decode(&bytes, &MAXIMA).unwrap();
    assert_eq!(
        file.get("contract_version").unwrap().string().unwrap(),
        "0.1.0"
    );
    let cases = file.get("cases").unwrap().array().unwrap();
    let plate_cases: Vec<&Json> = cases
        .iter()
        .filter(|c| c.get("command").unwrap().string().unwrap() == "CreatePlate")
        .collect();
    assert!(plate_cases.len() >= 40, "{}", plate_cases.len());

    let mut accepted = BTreeMap::new();
    let pos01 = plate_cases
        .iter()
        .find(|c| c.get("case_id").unwrap().string().unwrap() == "F04-POS-01")
        .unwrap();
    accepted.insert(
        "F04-POS-01".to_owned(),
        command_of(pos01.get("arguments").unwrap()).unwrap(),
    );

    let (mut passed, mut skipped, mut deviations) =
        (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    for case in &plate_cases {
        let id = case.get("case_id").unwrap().string().unwrap();
        let envelope = case.get("envelope_changes").unwrap();
        if SKIPPED.iter().any(|(s, _)| *s == id) {
            skipped.insert(id.to_owned());
            continue;
        }
        assert!(
            matches!(envelope, Json::Object(o) if o.is_empty()),
            "{id} changes the envelope but is not in the skip list"
        );
        let verdict = run_case(id, case.get("arguments").unwrap(), &accepted);
        let expected = case.get("expected").unwrap();
        let expected_verdict = match expected.get("code") {
            Ok(Json::String(code)) => Verdict::Rejected(code_of(code)),
            _ => match expected.get("outcome").unwrap().string().unwrap() {
                "accepted_draft" => Verdict::Applied,
                "recorded_result_replayed" => Verdict::Replayed,
                other => panic!("{id}: unknown outcome {other}"),
            },
        };
        if UNSUPPORTED_IN_A02.contains(&id) || WRONG_CODE_IN_A02.contains(&id) {
            assert_eq!(
                verdict,
                Verdict::Rejected(ErrorCode::UnsupportedGeometry),
                "{id} is a declared deviation and must stay UNSUPPORTED_GEOMETRY"
            );
            assert_ne!(
                verdict, expected_verdict,
                "{id} no longer deviates: update the lists"
            );
            deviations.insert(id.to_owned());
        } else {
            assert_eq!(verdict, expected_verdict, "{id}");
            passed.insert(id.to_owned());
        }
    }
    println!("PASSED ({}): {passed:?}", passed.len());
    println!("DEVIATIONS ({}): {deviations:?}", deviations.len());
    println!("SKIPPED ({}): {skipped:?}", skipped.len());
    assert_eq!(skipped.len(), SKIPPED.len());
    assert_eq!(
        deviations.len(),
        UNSUPPORTED_IN_A02.len() + WRONG_CODE_IN_A02.len()
    );
    // Cases that must be among the passing ones: the accepted first-scope plate, replay and reuse,
    // the minimum, the largest plate, large translation, quaternion tolerance, and the codes.
    for id in [
        "F04-POS-01",
        "F04-POS-04",
        "F04-POS-08",
        "F04-POS-09",
        "F04-POS-11",
        "F04-POS-12",
        "F04-POS-14",
        "F04-POS-17",
        // The A02 contour plate closed the former P01 deviations: the pentagon and the translated
        // rectangles (including the POS-19 square, which is listed counter-clockwise after all)
        // are evaluated, and the self-intersecting contour is refused with the F04 code. The
        // non-planar NEG-29/38 stay declared wrong-code deviations (UNSUPPORTED instead of
        // INVALID) with the off-plane POS-18/28.
        "F04-POS-19",
        "F04-POS-20",
        "F04-POS-25",
        "F04-POS-26",
        "F04-NEG-30",
        "F04-POS-27",
        "F04-NEG-01",
        "F04-NEG-02",
        "F04-NEG-03",
        "F04-NEG-04",
        "F04-NEG-05",
        "F04-NEG-06",
        "F04-NEG-10",
        "F04-NEG-19",
        "F04-NEG-22",
        "F04-NEG-23",
        "F04-NEG-24",
        "F04-NEG-25",
        "F04-NEG-27",
        "F04-NEG-28",
        "F04-NEG-31",
        "F04-NEG-32",
        "F04-NEG-34",
        "F04-NEG-35",
    ] {
        assert!(passed.contains(id), "{id} should pass: {passed:?}");
    }
}
