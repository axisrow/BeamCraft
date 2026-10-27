// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! P03/P12 adapter: the straight member (`CreateStraightMember`) and the polybeam
//! (`CreatePolybeam`), from command to committed USD revision.
//!
//! Same wiring and the same honesty limits as [`crate::plate`]: one implicit personal workspace,
//! the layer engine in this process (no LEW-03 containment), and create fused with commit. A
//! member pins its required profile and optional material by the full digest of an immutable
//! definition published earlier ([`crate::definition`]); the engine resolves both from the head
//! and the kernel builds the body. The two command families share the commit tail
//! (`finish_member`).

use crate::definition::{material_lookup, profile_lookup};
use crate::plate::{
    AUTHORED_LAYER, Base, DERIVED_LAYER, PlateError, Recorded, RevisionSummary, check_footprint,
    control, decode, digest_of, evaluate_decoded, host_uuid, load_base, member_record, mesh_change,
    open_base, prior_op_with, replay_summary, summarize,
};
use crate::publication::{PublishRequest, publish_revision};
use beamcraft_engine::footprint::Footprint;
use beamcraft_engine::member::{
    AuthoredMember, MemberAdmission, admit_create_member, admit_create_polybeam,
};
use beamcraft_engine::query::committed::{MemberRecord, ids};
use beamcraft_host::revision::{
    CommitHook, CommitOutcome, ExpectedBase, Revision, RevisionStore, Uuid7,
};
use beamcraft_layer_engine::OpenRequest;
use beamcraft_layer_engine::Reader;
use beamcraft_layer_engine::authored::{
    Author, Change, EditTargets, Element, MemberKind as LayerKind, Placement as LayerPlacement,
    Prepare, Shape, schema_identity,
};
use beamcraft_model::command::{CreatePolybeam, CreateStraightMember, MemberKind};
use beamcraft_model::digest::ContentDigest;
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

/// Inputs of one `create_polybeam` call beyond the command itself.
#[derive(Debug, Clone)]
pub struct CreatePolybeamRequest {
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
    pub command: CreatePolybeam,
}

/// Result of `create_member` and `create_polybeam`.
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

/// The replay fingerprint of a committed member for one command family: the digest of the
/// command that would author it again, or the unseen sentinel when the committed member was
/// authored by the other family (a straight-member retry never replays a polybeam and the
/// other way round).
fn replay_fingerprint(
    committed: &AuthoredMember,
    straight: bool,
) -> Result<ContentDigest, PlateError> {
    let command = if straight {
        committed.to_command().map(|c| c.content_fingerprint())
    } else {
        committed
            .to_polybeam_command()
            .map(|c| c.content_fingerprint())
    };
    match command.transpose()? {
        Some(fingerprint) => Ok(fingerprint),
        None => digest_of(b"no accepted request with this element id"),
    }
}

/// The opened base and its commit expectations, as the shared commit tail takes them.
struct MemberBase {
    base_request: OpenRequest,
    project: String,
    workspace: String,
    generation: u64,
    expected: ExpectedBase,
}

impl MemberBase {
    fn open(base: Base) -> Result<(Self, OpenRequest), PlateError> {
        let base_request = open_base(&base)?;
        let Base {
            project,
            workspace,
            generation,
            expected,
            ..
        } = base;
        Ok((
            Self {
                base_request: base_request.clone(),
                project,
                workspace,
                generation,
                expected,
            },
            base_request,
        ))
    }
}

/// Which create request the commit tail serves, for its op and revision ids.
enum MemberCreate<'a> {
    Straight(&'a CreateMemberRequest),
    Polybeam(&'a CreatePolybeamRequest),
}

impl MemberCreate<'_> {
    fn op_id(&self) -> OpId {
        match self {
            Self::Straight(request) => request.op_id,
            Self::Polybeam(request) => request.op_id,
        }
    }

    fn revision_id(&self) -> &RevisionId {
        match self {
            Self::Straight(request) => &request.revision_id,
            Self::Polybeam(request) => &request.revision_id,
        }
    }

    fn actor(&self) -> &Uuid7 {
        match self {
            Self::Straight(request) => &request.actor,
            Self::Polybeam(request) => &request.actor,
        }
    }
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
    let (member_base, base_request) = MemberBase::open(base)?;
    let (head_plates, head_members, definitions) = {
        let mut reader = Reader::default();
        let opened = reader.open(base_request, &control)?;
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
            // A committed polybeam member is never the replay target of a straight-member
            // command: its fingerprint falls back to the unseen sentinel.
            Some(record) => replay_fingerprint(&record.authored, true),
            None => digest_of(b"no accepted request with this element id"),
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
    finish_member(
        store,
        admission,
        head_members,
        &head,
        recorded,
        member_base,
        MemberCreate::Straight(request),
        op_host,
        hook,
    )
}

/// Creates one polybeam (P12) and publishes it as the next revision of the store, or replays
/// it. The steps and honesty limits are [`create_member`]'s; the engine sweeps the one pinned
/// section along the polyline and mitres every interior vertex (G12).
pub fn create_polybeam(
    store: &RevisionStore,
    request: &CreatePolybeamRequest,
    hook: &mut dyn CommitHook,
) -> Result<CreatedMember, PlateError> {
    let control = control();
    let (head, base) = load_base(store, &request.project, &request.workspace)?;
    let (member_base, base_request) = MemberBase::open(base)?;
    let (head_plates, head_members, definitions) = {
        let mut reader = Reader::default();
        let opened = reader.open(base_request, &control)?;
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
            // A committed straight member is never the replay target of a polybeam command.
            Some(record) => replay_fingerprint(&record.authored, false),
            None => digest_of(b"no accepted request with this element id"),
        }
    })?;
    let mut existing = ids(&head_plates);
    existing.extend(head_members.iter().map(|m| m.authored.id));
    let admission = admit_create_polybeam(
        request.op_id,
        &request.command,
        prior,
        &|id: &ElementId| existing.contains(id),
        &profile_lookup(&definitions),
        &material_lookup(&definitions),
    )?;
    finish_member(
        store,
        admission,
        head_members,
        &head,
        recorded,
        member_base,
        MemberCreate::Polybeam(request),
        op_host,
        hook,
    )
}

/// The replay or the committed outcome of one admitted member create: the tail shared by
/// [`create_member`] and [`create_polybeam`]. A replay re-reads the head's record; an apply
/// authors the U02 changes (class, element, mesh and quantities), checks the layer engine's
/// write set against the declared footprint and commits through U06/U07 with `plate`'s
/// verifier.
#[allow(clippy::too_many_arguments)]
fn finish_member(
    store: &RevisionStore,
    admission: MemberAdmission,
    head_members: Vec<MemberRecord>,
    head: &Option<Revision>,
    recorded: Option<Recorded>,
    member_base: MemberBase,
    create: MemberCreate<'_>,
    op_host: Uuid7,
    hook: &mut dyn CommitHook,
) -> Result<CreatedMember, PlateError> {
    let transaction = match admission {
        MemberAdmission::Replay => {
            let member = head_members
                .into_iter()
                .find(|r| r.authored.id == new_id_of(&create))
                .ok_or_else(|| {
                    crate::plate::integrity("replayed element is absent from the head revision")
                })?;
            return Ok(CreatedMember {
                replayed: true,
                member,
                revision: replay_summary(head, recorded)?,
                footprint: None,
            });
        }
        MemberAdmission::Apply(transaction) => transaction,
    };
    let outcome = &transaction.outcome;
    let record = MemberRecord {
        authored: outcome.authored.clone(),
        body: outcome.body.clone(),
        quantities: outcome.quantities,
    };
    let schema = schema_identity()?;
    let (mut author, opened) = Author::open(
        member_base.base_request,
        EditTargets {
            authored: AUTHORED_LAYER.into(),
            derived: DERIVED_LAYER.into(),
        },
        schema,
        &control(),
    )?;
    let placement = outcome.authored.placement;
    let [qx, qy, qz, qw] = placement.orientation_xyzw;
    let prepared = author.prepare(
        Prepare {
            context: opened.context.clone(),
            op_id: create.op_id().to_string(),
            actor: create.actor().to_string(),
            schema,
            resources: Vec::new(),
            changes: vec![
                Change::PartClass {
                    id: outcome.authored.id.to_string(),
                    class: outcome.authored.class,
                },
                Change::Create(Element {
                    id: outcome.authored.id.to_string(),
                    name: outcome.authored.name.clone(),
                    shape: Shape::Member {
                        kind: match outcome.authored.kind {
                            MemberKind::Beam => LayerKind::Beam,
                            MemberKind::Column => LayerKind::Column,
                        },
                        axis_mm: outcome.authored.axis_points_mm.clone(),
                        profile: outcome.authored.profile,
                        roll_rad: outcome.authored.roll_rad,
                        placement: outcome.authored.section_placement,
                        cuts: Vec::new(),
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
        &control(),
    )?;
    check_footprint(&prepared, &outcome.footprint)?;
    let committed = publish_revision(
        store,
        &PublishRequest {
            footprint: &outcome.footprint,
            fingerprint: outcome.fingerprint,
            prepared: &prepared,
            schema,
            project: &member_base.project,
            workspace: &member_base.workspace,
            revision_id: create.revision_id(),
            op: op_host,
            base_generation: member_base.generation,
            expected: &member_base.expected,
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

/// The element id a create command targets.
fn new_id_of(create: &MemberCreate<'_>) -> ElementId {
    match create {
        MemberCreate::Straight(request) => request.command.new_id,
        MemberCreate::Polybeam(request) => request.command.new_id,
    }
}
