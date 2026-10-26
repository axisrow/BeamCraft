// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! Original synthetic F04 authored-only admission and replay regressions.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

use beamcraft_engine::command::{create_plate, no_materials};
use beamcraft_engine::plate_draft::{
    DraftAdmission, PlateDraft, admit_create_plate_draft, create_plate_draft,
    evaluate_committed_drafts,
};
use beamcraft_engine::transaction::PriorOp;
use beamcraft_model::command::{CreatePlate, Placement};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId};

fn command(s: f64) -> CreatePlate {
    CreatePlate {
        new_id: ElementId::parse("019a7bf0-0000-7000-8000-000000000005").unwrap(),
        name: "Near-planar original".into(),
        thickness_mm: 10.,
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
fn invalid_inputs_and_identity_failures_never_become_unsupported_drafts() {
    assert_eq!(
        create_plate_draft(&command(0.875), &|_| false, &no_materials)
            .unwrap_err()
            .code,
        ErrorCode::InvalidArgument
    );
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
    let mut wrong = command(0.75);
    wrong.contour_points_mm.reverse();
    assert_eq!(
        create_plate_draft(&wrong, &|_| false, &no_materials)
            .unwrap_err()
            .code,
        ErrorCode::InvalidArgument
    );
    let mut wrong = command(0.75);
    wrong.placement.orientation_xyzw = [0., 0., 1., 0.];
    assert_eq!(
        create_plate_draft(&wrong, &|_| false, &no_materials)
            .unwrap_err()
            .code,
        ErrorCode::UnsupportedGeometry
    );
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
    let mut plate = beamcraft_engine::command::AuthoredPlate::from_command(&command(0.75));
    plate.contour_points_mm = vec![[0.; 3]; 1025];
    assert_eq!(
        evaluate_committed_drafts(&[plate], &no_materials)
            .unwrap_err()
            .code,
        ErrorCode::LimitExceeded
    );
    let mut plate = beamcraft_engine::command::AuthoredPlate::from_command(&command(0.75));
    plate.name = "x".repeat(257);
    assert_eq!(
        evaluate_committed_drafts(&[plate], &no_materials)
            .unwrap_err()
            .code,
        ErrorCode::InvalidArgument
    );
}
