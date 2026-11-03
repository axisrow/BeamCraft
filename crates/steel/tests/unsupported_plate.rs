// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! Original synthetic F04 authored-only admission and replay regressions.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

use beamcraft_model::command::Placement;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId};
use steel::engine::command::{create_plate, no_materials};
use steel::engine::plate_draft::{
    DraftAdmission, PlateDraft, admit_create_plate_draft, create_plate_draft,
    evaluate_committed_drafts,
};
use steel::engine::transaction::PriorOp;
use steel::model::command::CreatePlate;

fn command(s: f64) -> CreatePlate {
    CreatePlate {
        new_id: ElementId::parse("019a7bf0-0000-7000-8000-000000000005").unwrap(),
        name: "Near-planar original".into(),
        class: 0,
        thickness_mm: 10.,
        thickness_side: None,
        corners: Vec::new(),
        contour_points_mm: vec![
            [999000000., 0., 0.],
            [999393216., 0., s],
            [999393216., 524288., 0.],
            [999000000., 524288., s],
        ],
        placement: Placement {
            translation_mm: [0., 1e9, 0.],
            ..Placement::IDENTITY
        },
        material_digest: None,
        holes_mm: Vec::new(),
    }
}

#[test]
fn valid_authored_input_is_distinct_from_unsupported_solid_evaluation() {
    let command = command(0.75);
    assert_eq!(
        create_plate(&command, &|_| false, &no_materials)
            .unwrap_err()
            .code,
        ErrorCode::UnsupportedGeometry
    );
    let PlateDraft::Unsupported(plate) =
        create_plate_draft(&command, &|_| false, &no_materials).unwrap()
    else {
        panic!("expected unsupported authored input")
    };
    assert_eq!(plate.authored.to_command(), command);
    assert_eq!(plate.reason, "contour_not_exactly_planar");
    let (evaluated, unsupported) =
        evaluate_committed_drafts(std::slice::from_ref(&plate.authored), &no_materials).unwrap();
    assert!(evaluated.is_empty());
    assert_eq!(unsupported, vec![*plate]);
}

#[test]
fn wrongly_built_contours_are_kept_degenerate_and_hard_failures_still_refuse() {
    // A reversed contour (A-2 D-7's example) is retained without derived data, with the stable
    // winding reason and the unmodified authored inputs.
    let mut reversed = command(0.75);
    reversed.contour_points_mm.reverse();
    let PlateDraft::Unsupported(kept) =
        create_plate_draft(&reversed, &|_| false, &no_materials).unwrap()
    else {
        panic!("expected the reversed contour to be retained degenerate")
    };
    assert!(kept.degenerate);
    assert_eq!(kept.reason, "contour_winding_clockwise");
    assert_eq!(kept.authored.to_command(), reversed);
    // A contour outside the planarity band is the same retention class, its own reason.
    let PlateDraft::Unsupported(kept) =
        create_plate_draft(&command(0.875), &|_| false, &no_materials).unwrap()
    else {
        panic!("expected the off-band contour to be retained degenerate")
    };
    assert!(kept.degenerate);
    assert_eq!(kept.reason, "contour_outside_planarity_band");
    // Hard failures never retain: non-finite input, a duplicate id, holes beside the fault,
    // and an edge below the geometric floor (the authored layer's storage floor cannot hold
    // such a contour, so it never enters the degenerate state).
    assert_eq!(
        create_plate_draft(&command(f64::NAN), &|_| false, &no_materials)
            .unwrap_err()
            .code,
        ErrorCode::NonFiniteNumber
    );
    assert_eq!(
        create_plate_draft(&command(0.75), &|_| true, &no_materials)
            .unwrap_err()
            .code,
        ErrorCode::DuplicateId
    );
    let mut holed = command(0.75);
    holed.contour_points_mm.reverse();
    holed.holes_mm = vec![vec![[10.0, 10.0], [30.0, 10.0], [30.0, 30.0]]];
    assert_eq!(
        create_plate_draft(&holed, &|_| false, &no_materials)
            .unwrap_err()
            .code,
        ErrorCode::InvalidArgument
    );
    let tiny = CreatePlate {
        contour_points_mm: vec![
            [0., 0., 0.],
            [0.0000005, 0., 0.],
            [1000., 500., 0.],
            [0., 500., 0.],
        ],
        ..command(0.0)
    };
    assert_eq!(
        create_plate_draft(&tiny, &|_| false, &no_materials)
            .unwrap_err()
            .code,
        ErrorCode::InvalidArgument
    );
    // Supported-admission retention stays distinct: a valid contour with an identity rotation
    // of the world placement is unsupported, not degenerate.
    let mut rotated = command(0.75);
    rotated.placement.orientation_xyzw = [0., 0., 1., 0.];
    let PlateDraft::Unsupported(kept) =
        create_plate_draft(&rotated, &|_| false, &no_materials).unwrap()
    else {
        panic!("expected the rotated placement to stay in the unsupported class")
    };
    assert!(!kept.degenerate);
    assert_eq!(kept.reason, "contour_not_exactly_planar");
}

#[test]
fn a_committed_degenerate_plate_reads_authored_only_and_repairs_into_a_body() {
    let mut reversed = command(0.75);
    reversed.contour_points_mm.reverse();
    let PlateDraft::Unsupported(kept) =
        create_plate_draft(&reversed, &|_| false, &no_materials).unwrap()
    else {
        panic!("expected retention")
    };
    // The committed read keeps the degenerate record authored-only, without derived data.
    let (evaluated, unsupported) =
        evaluate_committed_drafts(std::slice::from_ref(&kept.authored), &no_materials).unwrap();
    assert!(evaluated.is_empty());
    assert_eq!(unsupported, vec![*kept.clone()]);
    // Once fixed, the same record evaluates: the D-7 repair recomputes geometry and quantities.
    let mut fixed = kept.authored.clone();
    fixed.contour_points_mm = vec![
        [0., 0., 0.],
        [1000., 0., 0.],
        [1000., 500., 0.],
        [0., 500., 0.],
    ];
    let (evaluated, unsupported) =
        evaluate_committed_drafts(std::slice::from_ref(&fixed), &no_materials).unwrap();
    assert!(unsupported.is_empty());
    assert_eq!(evaluated.len(), 1);
    assert_eq!(evaluated[0].authored, fixed);
}

#[test]
fn unsupported_creation_keeps_the_replay_and_reuse_contract() {
    let op = OpId::parse("019a7bf0-0000-7000-8000-000000000004").unwrap();
    let cmd = command(0.75);
    assert!(matches!(
        admit_create_plate_draft(op, &cmd, PriorOp::Unseen, &|_| false, &no_materials).unwrap(),
        DraftAdmission::Apply(_)
    ));
    let prior = PriorOp::Accepted {
        fingerprint: cmd.content_fingerprint().unwrap(),
    };
    assert_eq!(
        admit_create_plate_draft(op, &cmd, prior, &|_| true, &no_materials).unwrap(),
        DraftAdmission::Replay
    );
    assert_eq!(
        admit_create_plate_draft(op, &command(0.5), prior, &|_| true, &no_materials)
            .unwrap_err()
            .code,
        ErrorCode::OpIdReuse
    );
}

#[test]
fn committed_draft_fields_are_capped_before_owned_command_copies() {
    let mut plate = steel::engine::command::AuthoredPlate::from_command(&command(0.75));
    plate.contour_points_mm = vec![[0.; 3]; 1025];
    assert_eq!(
        evaluate_committed_drafts(&[plate], &no_materials)
            .unwrap_err()
            .code,
        ErrorCode::LimitExceeded
    );
    let mut plate = steel::engine::command::AuthoredPlate::from_command(&command(0.75));
    plate.name = "x".repeat(257);
    assert_eq!(
        evaluate_committed_drafts(&[plate], &no_materials)
            .unwrap_err()
            .code,
        ErrorCode::InvalidArgument
    );
}
