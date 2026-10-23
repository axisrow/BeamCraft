// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! P03 adapter: the first straight member (beam or column), from `CreateStraightMember` to a
//! committed USD revision.
//!
//! Same wiring and the same honesty limits as [`crate::plate`]: one implicit personal workspace,
//! the layer engine in this process (no LEW-03 containment), and create fused with commit. A
//! member pins its required profile and optional material by the full digest of an immutable
//! definition published earlier ([`crate::definition`]); the engine resolves both from the head
//! and the kernel builds the body.

use crate::definition::{material_lookup, profile_lookup};
use crate::plate::{
    AUTHORED_LAYER, Base, DERIVED_LAYER, PlateError, PublishRequest, RevisionSummary,
    check_footprint, control, decode, evaluate_decoded, host_uuid, load_base, member_record,
    mesh_change, open_base, prior_op_with, publish_revision, replay_summary, summarize,
};
use beamcraft_engine::footprint::Footprint;
use beamcraft_engine::member::{MemberAdmission, admit_create_member};
use beamcraft_engine::query::committed::{MemberRecord, ids};
use beamcraft_host::revision::{CommitHook, CommitOutcome, RevisionStore, Uuid7};
use beamcraft_layer_engine::Reader;
use beamcraft_layer_engine::authored::{
    Author, Change, EditTargets, Element, MemberKind as LayerKind, Placement as LayerPlacement,
    Prepare, Shape, schema_identity,
};
use beamcraft_model::command::{CreateStraightMember, MemberKind};
use beamcraft_model::id::{ElementId, OpId, ProjectId, RevisionId, WorkspaceId};

/// Inputs of one `create_member` call beyond the command itself.
#[derive(Debug, Clone)]
pub struct CreateMemberRequest {
    /// The logical operation. A retry with the same id and identical content is a replay.
    pub op_id: OpId,
    /// Identity of the revision this call creates.
    pub revision_id: RevisionId,
    /// The creating actor (trusted context, `bim:createdBy`).
    pub actor: Uuid7,
    /// Project of a store that has no revision yet. Ignored otherwise.
    pub project: ProjectId,
    /// Workspace of a store that has no revision yet. Ignored otherwise.
    pub workspace: WorkspaceId,
    /// The command.
    pub command: CreateStraightMember,
}

/// Result of `create_member`.
#[derive(Debug, Clone, PartialEq)]
pub struct CreatedMember {
    /// True when the same operation was already committed and nothing was written.
    pub replayed: bool,
    /// The element the command created (or created earlier).
    pub member: MemberRecord,
    /// The revision that holds it.
    pub revision: RevisionSummary,
    /// The footprint the engine declared; `None` for a replay.
    pub footprint: Option<Footprint>,
}

/// Creates one beam or column and publishes it as the next revision of the store, or replays it.
///
/// Steps, each failing closed: open and verify the head; decide replay from the host ledger;
/// engine admission (`CreateStraightMember` validation, the pinned profile and material resolved
/// from the head's definitions, kernel body, footprint); U02 authoring over the head's layers;
/// check that the layer engine's write set is inside the engine's footprint; publish the layer
/// blobs and commit through U06/U07 with the verifier of `plate` (which re-evaluates every plate
/// and member with the kernel).
pub fn create_member(
    store: &RevisionStore,
    request: &CreateMemberRequest,
    hook: &mut dyn CommitHook,
) -> Result<CreatedMember, PlateError> {
    let control = control();
    let (head, base) = load_base(store, &request.project, &request.workspace)?;
    let base_request = open_base(&base)?;
    let Base {
        project,
        workspace,
        generation: base_generation,
        expected,
        ..
    } = base;
    let (head_plates, head_members, definitions) = {
        let mut reader = Reader::default();
        let opened = reader.open(base_request.clone(), &control)?;
        let decoded = decode(&reader, &opened, &control)?;
        let (plates, members) = evaluate_decoded(&decoded)?;
        (plates, members, decoded.definitions)
    };
    let op_host = host_uuid(&request.op_id.to_string())?;
    let (prior, recorded) = prior_op_with(store, &op_host, || {
        match head_members
            .iter()
            .find(|r| r.authored.id == request.command.new_id)
        {
            Some(record) => Ok(record.authored.to_command().content_fingerprint()?),
            None => crate::plate::digest_of(b"no accepted request with this element id"),
        }
    })?;
    let mut existing = ids(&head_plates);
    existing.extend(head_members.iter().map(|m| m.authored.id));
    let admission = admit_create_member(
        request.op_id,
        &request.command,
        prior,
        &|id: &ElementId| existing.contains(id),
        &profile_lookup(&definitions),
        &material_lookup(&definitions),
    )?;
    let transaction = match admission {
        MemberAdmission::Replay => {
            let member = head_members
                .into_iter()
                .find(|r| r.authored.id == request.command.new_id)
                .ok_or_else(|| {
                    crate::plate::integrity("replayed element is absent from the head revision")
                })?;
            return Ok(CreatedMember {
                replayed: true,
                member,
                revision: replay_summary(&head, recorded)?,
                footprint: None,
            });
        }
        MemberAdmission::Apply(transaction) => *transaction,
    };
    let outcome = &transaction.outcome;
    let record = MemberRecord {
        authored: outcome.authored.clone(),
        body: outcome.body.clone(),
        quantities: outcome.quantities,
    };
    let schema = schema_identity()?;
    let (mut author, opened) = Author::open(
        base_request,
        EditTargets {
            authored: AUTHORED_LAYER.into(),
            derived: DERIVED_LAYER.into(),
        },
        schema,
        &control,
    )?;
    let placement = outcome.authored.placement;
    let [qx, qy, qz, qw] = placement.orientation_xyzw;
    let [p1, p2] = outcome.authored.axis_points_mm;
    let prepared = author.prepare(
        Prepare {
            context: opened.context.clone(),
            op_id: request.op_id.to_string(),
            actor: request.actor.to_string(),
            schema,
            resources: Vec::new(),
            changes: vec![
                Change::Create(Element {
                    id: outcome.authored.id.to_string(),
                    name: outcome.authored.name.clone(),
                    shape: Shape::Member {
                        kind: match outcome.authored.kind {
                            MemberKind::Beam => LayerKind::Beam,
                            MemberKind::Column => LayerKind::Column,
                        },
                        axis_mm: [p1, p2],
                        profile: outcome.authored.profile,
                        roll_rad: outcome.authored.roll_rad,
                        cut: outcome.authored.cut,
                    },
                    placement: LayerPlacement {
                        translation_mm: placement.translation_mm,
                        orientation_xyzw: [qx, qy, qz, qw],
                    },
                    material: outcome.authored.material,
                }),
                mesh_change(
                    &outcome.authored.id,
                    &record.body.solid,
                    &member_record(&record),
                )?,
            ],
        },
        &control,
    )?;
    check_footprint(&prepared, &outcome.footprint)?;
    let committed = publish_revision(
        store,
        &PublishRequest {
            footprint: &outcome.footprint,
            fingerprint: outcome.fingerprint,
            prepared: &prepared,
            schema,
            project: &project,
            workspace: &workspace,
            revision_id: &request.revision_id,
            op: op_host,
            base_generation,
            expected: &expected,
        },
        hook,
    )?;
    Ok(CreatedMember {
        replayed: committed.outcome == CommitOutcome::Replayed,
        member: record,
        revision: summarize(&committed.info),
        footprint: Some(outcome.footprint.clone()),
    })
}
