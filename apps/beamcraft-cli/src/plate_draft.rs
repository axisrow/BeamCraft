// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Atomic authored-only plate revisions for valid unsupported contours.

use crate::definition::material_lookup;
use crate::plate::{
    self, AUTHORED_LAYER, CreateRequest, Created, DERIVED_LAYER, PlateError, RevisionSummary,
    check_footprint, control, decode, digest_of, host_uuid, integrity, load_base, open_base,
    prior_op_with, replay_summary, summarize,
};
use crate::publication::{PublishRequest, publish_revision};
use beamcraft_engine::footprint::Footprint;
use beamcraft_engine::plate_draft::{
    DraftAdmission, PlateDraft, UnsupportedPlate, admit_create_plate_draft,
};
use beamcraft_host::revision::{CommitHook, CommitOutcome, RevisionStore};
use beamcraft_layer_engine::Reader;
use beamcraft_layer_engine::authored::{
    Author, Change, EditTargets, Element, Placement, Prepare, Shape, schema_identity,
};

/// Saved unsupported input with its committed revision and replay result.
#[derive(Debug, Clone, PartialEq)]
pub struct CreatedUnsupported {
    /// Original authored data and the reason geometry is unavailable.
    pub plate: UnsupportedPlate,
    /// Whether this operation was already committed.
    pub replayed: bool,
    /// Revision holding this authored result.
    pub revision: RevisionSummary,
    /// Writes of a new operation; absent for a replay.
    pub footprint: Option<Footprint>,
}

/// A saved authored plate, with either evaluated or explicitly unsupported geometry.
#[derive(Debug, Clone, PartialEq)]
pub enum CreatedDraft {
    /// Complete solid and quantity records.
    Evaluated(Box<Created>),
    /// Authored input only; no partial derived result.
    Unsupported(Box<CreatedUnsupported>),
}

/// Creates an authored plate draft through the same atomic store/verifier as evaluated plates.
/// The evaluated-only `plate::create_plate` API retains its refusal-without-mutation behaviour.
pub fn create_plate_draft(
    store: &RevisionStore,
    request: &CreateRequest,
    hook: &mut dyn CommitHook,
) -> Result<CreatedDraft, PlateError> {
    let control = control();
    let (head, base) = load_base(store, &request.project, &request.workspace)?;
    let open_request = open_base(&base)?;
    let mut reader = Reader::default();
    let opened = reader.open(open_request.clone(), &control)?;
    let decoded = decode(&reader, &opened, &control)?;
    let op = host_uuid(&request.op_id.to_string())?;
    let (prior, recorded) = prior_op_with(store, &op, || {
        match decoded
            .plates
            .iter()
            .find(|(p, _)| p.id == request.command.new_id)
        {
            Some((plate, _)) => Ok(plate.to_command().content_fingerprint()?),
            None => digest_of(b"no accepted request with this element id"),
        }
    })?;
    let admission = admit_create_plate_draft(
        request.op_id,
        &request.command,
        prior,
        &|id| {
            decoded.elements.contains_key(id)
                || decoded.levels.levels().any(|(level, _)| level == id)
        },
        &material_lookup(&decoded.definitions),
    )?;
    let transaction = match admission {
        DraftAdmission::Replay => {
            let authored = decoded
                .plates
                .iter()
                .find(|(p, _)| p.id == request.command.new_id)
                .map(|(p, _)| p)
                .ok_or_else(|| integrity("replayed element is absent"))?;
            let state = beamcraft_engine::plate_draft::create_plate_draft(
                &authored.to_command(),
                &|_| false,
                &material_lookup(&decoded.definitions),
            )?;
            match state {
                PlateDraft::Evaluated(_) => {
                    return plate::create_plate(store, request, hook)
                        .map(|p| CreatedDraft::Evaluated(Box::new(p)));
                }
                PlateDraft::Unsupported(plate) => {
                    if let Some(head) = &head {
                        plate::inspect_revision(head)?;
                    }
                    return Ok(CreatedDraft::Unsupported(Box::new(CreatedUnsupported {
                        plate: *plate,
                        replayed: true,
                        revision: replay_summary(&head, recorded)?,
                        footprint: None,
                    })));
                }
            }
        }
        DraftAdmission::Apply(transaction) => *transaction,
    };
    let PlateDraft::Unsupported(plate) = transaction.outcome else {
        return plate::create_plate(store, request, hook)
            .map(|p| CreatedDraft::Evaluated(Box::new(p)));
    };
    if let Some(head) = &head {
        plate::inspect_revision(head)?;
    }
    let schema = schema_identity()?;
    let (mut author, opened) = Author::open(
        open_request,
        EditTargets {
            authored: AUTHORED_LAYER.into(),
            derived: DERIVED_LAYER.into(),
        },
        schema,
        &control,
    )?;
    let authored = &plate.authored;
    let prepared = author.prepare(
        Prepare {
            context: opened.context,
            op_id: request.op_id.to_string(),
            actor: request.actor.to_string(),
            schema,
            resources: Vec::new(),
            changes: vec![
                Change::PartClass {
                    id: authored.id.to_string(),
                    class: authored.class,
                },
                Change::Create(Element {
                    id: authored.id.to_string(),
                    name: authored.name.clone(),
                    shape: Shape::Plate {
                        contour_mm: authored.contour_points_mm.clone(),
                        thickness_mm: authored.thickness_mm,
                        thickness_side: authored.thickness_side,
                        // An unsupported plate is never evaluated, so it holds no treatments
                        // and no records.
                        corners: Vec::new(),
                        bends: Vec::new(),
                        holes: Vec::new(),
                        hole_records: Vec::new(),
                        cuts: Vec::new(),
                    },
                    placement: Placement {
                        translation_mm: authored.placement.translation_mm,
                        orientation_xyzw: authored.placement.orientation_xyzw,
                    },
                    material: authored.material,
                }),
            ],
        },
        &control,
    )?;
    check_footprint(&prepared, &transaction.footprint)?;
    let committed = publish_revision(
        store,
        &PublishRequest {
            footprint: &transaction.footprint,
            fingerprint: transaction.fingerprint,
            prepared: &prepared,
            schema,
            project: &base.project,
            workspace: &base.workspace,
            revision_id: &request.revision_id,
            op,
            base_generation: base.generation,
            expected: &base.expected,
        },
        hook,
    )?;
    Ok(CreatedDraft::Unsupported(Box::new(CreatedUnsupported {
        plate: *plate,
        replayed: committed.outcome == CommitOutcome::Replayed,
        revision: summarize(&committed.info),
        footprint: Some(transaction.footprint),
    })))
}
