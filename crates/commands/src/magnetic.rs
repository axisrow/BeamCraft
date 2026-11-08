// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! T01.8 magnetic grid bindings through the atomic revision pipeline: `bind-grid` stores a
//! member's axis in its grid's local frame and `unbind-grid` materializes the world axis back,
//! so a shared-grid edit never rewrites the member (contract §7.5, decisions 2026-11-08).

use crate::grid::{GridContext, pinned_selector, snapshot};
use crate::plate::{decode, evaluate_decoded, part_mesh_change, prior_op_with, replay_summary};
use crate::publication::{PublishRequest, publish_revision};
use beamcraft_engine::footprint::{Entry, Footprint, Group, Target};
use beamcraft_host::revision::{CommitHook, CommitOutcome, RevisionStore};
use beamcraft_layer_engine::Reader;
use beamcraft_layer_engine::authored::{
    Change, FollowBinding, FollowRecord, Prepare, schema_identity,
};
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::grid::{BindGrid, Grid, UnbindGrid};
use beamcraft_model::id::ElementId;
use session::Refusal;
use session::pipeline::{
    Base, RevisionSummary, control, host_uuid, integrity, load_base, open_base, summarize,
};
use steel::engine::transaction::PriorOp;

/// The committed or replayed result of a magnetic binding command.
#[derive(Debug, Clone, PartialEq)]
pub struct BindChange {
    /// Whether the operation was already committed.
    pub replayed: bool,
    /// Revision holding the result.
    pub revision: RevisionSummary,
    /// Declared writes, absent on replay.
    pub footprint: Option<Footprint>,
}

fn replay(
    store: &RevisionStore,
    context: &GridContext,
    fingerprint: beamcraft_model::digest::ContentDigest,
) -> Result<Option<BindChange>, Refusal> {
    let (head, _base) = load_base(store, &context.project, &context.workspace)?;
    let op = host_uuid(&context.op_id.to_string())?;
    let (prior, recorded) = prior_op_with(store, &op, || Ok(fingerprint))?;
    if let PriorOp::Accepted { .. } = prior {
        let Some((_, sequence, _)) = &recorded else {
            return Err(integrity("accepted binding operation has no revision"));
        };
        let revision = store.open_at(*sequence)?;
        let evidence = crate::command_evidence::read(&revision)?
            .ok_or_else(|| integrity("binding operation has no request evidence"))?;
        if evidence.fingerprint != fingerprint {
            return Err(CommandError::new(
                ErrorCode::OpIdReuse,
                "op_id",
                "operation id was accepted with different content",
            )
            .into());
        }
        return Ok(Some(BindChange {
            replayed: true,
            revision: replay_summary(&head, recorded)?,
            footprint: None,
        }));
    }
    Ok(None)
}

/// The head member's committed record, read through the real decoded picture with its
/// world-derived body.
fn head_member(
    store: &RevisionStore,
    context: &GridContext,
    target: &ElementId,
) -> Result<steel::engine::committed::MemberRecord, Refusal> {
    let c = control();
    let (_, base) = load_base(store, &context.project, &context.workspace)?;
    let request = open_base(&base)?;
    let mut reader = Reader::default();
    let opened = reader.open(request, &c)?;
    let decoded = decode(&reader, &opened, &c)?;
    let (_, members) = evaluate_decoded(&decoded)?;
    let record = members
        .iter()
        .find(|r| r.authored.id == *target)
        .ok_or_else(|| CommandError::new(ErrorCode::NotFound, "target_id", "member not found"))?;
    Ok(record.clone())
}

/// The head grid, read through the verified grid records.
fn head_grid(
    store: &RevisionStore,
    context: &GridContext,
    grid: &ElementId,
) -> Result<Grid, Refusal> {
    let c = control();
    let (_, base) = load_base(store, &context.project, &context.workspace)?;
    let request = open_base(&base)?;
    let prims = snapshot(request, &c)?;
    beamcraft_layer_engine::authored::grid::read_grids(&prims)?
        .into_iter()
        .find(|g| g.id == *grid)
        .ok_or_else(|| CommandError::new(ErrorCode::NotFound, "grid", "grid not found").into())
}

fn member_entries(
    target: &ElementId,
    groups: impl IntoIterator<Item = Group>,
) -> impl Iterator<Item = Entry> {
    groups.into_iter().map(|group| Entry {
        target: Target::Element(*target),
        group,
    })
}

fn publish(
    store: &RevisionStore,
    context: &GridContext,
    fingerprint: beamcraft_model::digest::ContentDigest,
    footprint: Footprint,
    changes: Vec<Change>,
    hook: &mut dyn CommitHook,
) -> Result<BindChange, Refusal> {
    let c = control();
    let (_, base) = load_base(store, &context.project, &context.workspace)?;
    let request = open_base(&base)?;
    let schema = schema_identity()?;
    let (mut author, opened) = crate::scope::open_author(store, request, false, schema, &c)?;
    let prepared = author.prepare(
        Prepare {
            context: opened.context.clone(),
            op_id: context.op_id.to_string(),
            actor: context.actor.to_string(),
            schema,
            resources: Vec::new(),
            changes,
        },
        &c,
    )?;
    let Base {
        project,
        workspace,
        generation: base_generation,
        expected,
        ..
    } = base;
    let committed = publish_revision(
        store,
        &PublishRequest {
            footprint: &footprint,
            fingerprint,
            prepared: &prepared,
            schema,
            project: &project,
            workspace: &workspace,
            revision_id: &context.revision_id,
            op: host_uuid(&context.op_id.to_string())?,
            base_generation,
            expected: &expected,
        },
        hook,
    )?;
    if committed.outcome == CommitOutcome::Replayed {
        return Err(integrity("unseen binding command reported as replayed"));
    }
    Ok(BindChange {
        replayed: false,
        revision: summarize(&committed.info),
        footprint: Some(footprint),
    })
}

/// Binds a placed member to a local grid (T01.8), or replays the exact prior binding. The
/// member's axis becomes grid-local; a pinned coordination grid is never followed.
pub fn bind_grid(
    store: &RevisionStore,
    context: &GridContext,
    command: &BindGrid,
    hook: &mut dyn CommitHook,
) -> Result<BindChange, Refusal> {
    command.validate_arguments()?;
    let fingerprint = command.content_fingerprint()?;
    if let Some(replayed) = replay(store, context, fingerprint)? {
        return Ok(replayed);
    }
    if pinned_selector(&command.grid.to_string())?.is_some() {
        return Err(Refusal {
            rule_id: Some("PC-OWN-002"),
            code: ErrorCode::ForeignReadOnly,
            message: "the pinned coordination grid is read-only and is never followed",
        });
    }
    let member = head_member(store, context, &command.target_id)?;
    let grid = head_grid(store, context, &command.grid)?;
    let local_axis_mm =
        steel::engine::member::grid_local_axis(&grid, &member.authored.axis_points_mm);
    let reads = member_entries(
        &command.target_id,
        [Group::Lifecycle, Group::ShapeInputs, Group::Relationship],
    )
    .chain([
        Entry {
            target: Target::Element(command.grid),
            group: Group::Lifecycle,
        },
        Entry {
            target: Target::Element(command.grid),
            group: Group::Placement,
        },
    ])
    .collect::<Vec<_>>();
    let footprint = Footprint::from_entries(
        &reads,
        &member_entries(
            &command.target_id,
            [Group::ShapeInputs, Group::Relationship],
        )
        .collect::<Vec<_>>(),
    )?;
    publish(
        store,
        context,
        fingerprint,
        footprint,
        vec![Change::Follow {
            id: command.target_id.to_string(),
            binding: FollowBinding::Bind(FollowRecord {
                grid: command.grid.to_string(),
                local_axis_mm,
            }),
        }],
        hook,
    )
}

/// Releases a member's grid binding (T01.8) and materializes the current world axis, or
/// replays the exact prior release. An unbound member is refused.
pub fn unbind_grid(
    store: &RevisionStore,
    context: &GridContext,
    command: &UnbindGrid,
    hook: &mut dyn CommitHook,
) -> Result<BindChange, Refusal> {
    command.validate_arguments()?;
    let fingerprint = command.content_fingerprint()?;
    if let Some(replayed) = replay(store, context, fingerprint)? {
        return Ok(replayed);
    }
    let member = head_member(store, context, &command.target_id)?;
    let follow = member.authored.follow.clone().ok_or_else(|| {
        CommandError::new(
            ErrorCode::InvalidArgument,
            "target_id",
            "the member is not bound to a grid",
        )
    })?;
    let grid = head_grid(store, context, &follow.grid)?;
    let world = steel::engine::member::grid_world_axis(&grid, &follow.local_axis_mm);
    // The release re-materializes the world axis and refreshes the derived mesh with it:
    // the stored mesh stops lagging the frame the moment the member stops following it.
    // The record's body is evaluated from the current derived world, identical to the
    // released one, so its mesh and quantities are the fresh truth.
    let mesh = part_mesh_change(&member)?;
    let reads = member_entries(
        &command.target_id,
        [
            Group::Lifecycle,
            Group::ShapeInputs,
            Group::Relationship,
            Group::Placement,
        ],
    )
    .chain([
        Entry {
            target: Target::Element(follow.grid),
            group: Group::Lifecycle,
        },
        Entry {
            target: Target::Element(follow.grid),
            group: Group::Placement,
        },
    ])
    .collect::<Vec<_>>();
    let footprint = Footprint::from_entries(
        &reads,
        &member_entries(
            &command.target_id,
            [Group::ShapeInputs, Group::Relationship, Group::Derived],
        )
        .collect::<Vec<_>>(),
    )?;
    publish(
        store,
        context,
        fingerprint,
        footprint,
        vec![
            Change::Follow {
                id: command.target_id.to_string(),
                binding: FollowBinding::Release(world),
            },
            mesh,
        ],
        hook,
    )
}
