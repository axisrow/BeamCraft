// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! T03 adapter: `ArrayElements` for a plate or a member, committed as one revision.
//!
//! Every copy is an independent duplicate (a P05 copy rule: fresh identity, freshly minted
//! operation records, the source's level), and the whole array is authored in one batch, so one
//! undo step takes every copy back. The source prims are never written; the declared footprint
//! is the union of the per-copy copy footprints and is checked against the layer engine's own
//! write set. Honesty limits: the copies join no assembly and take no connections (DV-REL-011),
//! as a single copy; a shared-definition source duplicates independently, it does not instance.

use crate::copy::{CopiedElement, creation_changes, not_found};
use crate::definition::material_lookup;
use crate::plate::{
    AUTHORED_LAYER, Base, DERIVED_LAYER, PlateError, RevisionSummary, control, decode, digest_of,
    evaluate_decoded, host_uuid, integrity, load_base, open_base, prior_op_with, replay_summary,
    summarize,
};
use crate::publication::{PublicationTail, prepare_and_publish};
use beamcraft_engine::footprint::Footprint;
use beamcraft_engine::transaction::PriorOp;
use beamcraft_engine::transform::{array_member_copies, array_plate_copies};
use beamcraft_host::revision::{CommitHook, RevisionStore, Uuid7};
use beamcraft_layer_engine::Reader;
use beamcraft_layer_engine::authored::{Author, Change, EditTargets, schema_identity};
use beamcraft_layer_engine::index::{ElementId as LayerElementId, LinkTarget};
use beamcraft_model::command::{ArrayElements, CommandError};
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId, ProjectId, RevisionId, WorkspaceId};

/// Inputs of one `array-linear` or `array-polar` call beyond the command itself.
#[derive(Debug, Clone)]
pub struct ArrayRequest {
    /// The logical operation.
    pub op_id: OpId,
    /// Identity of the revision this call creates.
    pub revision_id: RevisionId,
    /// The acting user (trusted context).
    pub actor: Uuid7,
    /// Project of a store that has no revision yet; an array in such a store is `NOT_FOUND`.
    pub project: ProjectId,
    /// Workspace of a store that has no revision yet.
    pub workspace: WorkspaceId,
    /// The command.
    pub command: ArrayElements,
}

/// Result of `array_elements`.
#[derive(Debug, Clone, PartialEq)]
pub struct Arrayed {
    /// True when the same operation was already committed and nothing was written.
    pub replayed: bool,
    /// The copies, in array order.
    pub elements: Vec<CopiedElement>,
    /// The revision that holds the result.
    pub revision: RevisionSummary,
    /// The declared footprint; `None` for a replay.
    pub footprint: Option<Footprint>,
    /// Names of the layers whose bytes changed (empty for a replay).
    pub changed_layers: Vec<String>,
}

/// The pinned profile a copy's creation reads (a member's, never a plate's).
fn copy_profile(copy: &CopiedElement) -> Option<ContentDigest> {
    match copy {
        CopiedElement::Plate(_) => None,
        CopiedElement::Member(r) => Some(r.authored.profile),
    }
}

/// The pinned material a copy's creation reads.
fn copy_material(copy: &CopiedElement) -> Option<ContentDigest> {
    match copy {
        CopiedElement::Plate(r) => r.authored.material,
        CopiedElement::Member(r) => r.authored.material,
    }
}

/// The identity of a copied record.
fn copy_id(copy: &CopiedElement) -> ElementId {
    match copy {
        CopiedElement::Plate(r) => r.authored.id,
        CopiedElement::Member(r) => r.authored.id,
    }
}

/// Arrays every copy of one source part and publishes the next revision, or replays the
/// operation.
///
/// Steps, each failing closed: admit the request; open and verify the head; find the source;
/// build every copy's authored state at its stepped placement (the bodies are the source's);
/// decide replay from the host ledger; refuse a copy identity that is already in use; author
/// all copies with U02 in one batch; check the layer engine's writes against the declared
/// footprint; commit through U06/U07 with the strict verifier.
pub fn array_elements(
    store: &RevisionStore,
    request: &ArrayRequest,
    hook: &mut dyn CommitHook,
) -> Result<Arrayed, PlateError> {
    let command = &request.command;
    command.validate_arguments()?;
    let control = control();
    let (head, base) = load_base(store, &request.project, &request.workspace)?;
    if head.is_none() {
        return Err(not_found());
    }
    let before: std::collections::BTreeMap<String, _> = base
        .assets
        .iter()
        .map(|a| (a.name.clone(), a.digest))
        .collect();
    let base_request = open_base(&base)?;
    let Base {
        project,
        workspace,
        generation: base_generation,
        expected,
        ..
    } = base;
    let (plates, members, definitions, levels, instances) = {
        let mut reader = Reader::default();
        let opened = reader.open(base_request.clone(), &control)?;
        let decoded = decode(&reader, &opened, &control)?;
        let (plates, members) = evaluate_decoded(&decoded)?;
        if decoded
            .plates
            .iter()
            .any(|(p, _)| p.id == command.source_id)
            && !plates.iter().any(|p| p.authored.id == command.source_id)
        {
            return Err(CommandError::new(
                ErrorCode::DependencyUnavailable,
                "source_id",
                "arraying an unsupported plate is not implemented",
            )
            .into());
        }
        (
            plates,
            members,
            decoded.definitions,
            decoded.levels,
            decoded.instances,
        )
    };
    let materials = material_lookup(&definitions);
    let canonical = match instances.definition(command.source_id) {
        Ok(id) => id,
        Err(e) if e.code == ErrorCode::NotFound => command.source_id,
        Err(e) => return Err(e.into()),
    };
    // The engine builds every authored copy; the element-local bodies and quantities are the
    // source's, byte for byte, exactly as for one copy.
    let copies: Vec<CopiedElement> = if let Some(source) =
        plates.iter().find(|r| r.authored.id == command.source_id)
    {
        array_plate_copies(&source.authored, command)?
            .into_iter()
            .map(|authored| {
                CopiedElement::Plate(Box::new(beamcraft_engine::query::committed::PlateRecord {
                    body: source.body.clone(),
                    quantities: source.quantities,
                    authored,
                }))
            })
            .collect()
    } else if let Some(source) = members.iter().find(|r| r.authored.id == command.source_id) {
        array_member_copies(&source.authored, command)?
            .into_iter()
            .map(|authored| {
                CopiedElement::Member(Box::new(beamcraft_engine::query::committed::MemberRecord {
                    body: source.body.clone(),
                    quantities: source.quantities,
                    authored,
                }))
            })
            .collect()
    } else if crate::weld::is_weld(&base_request, &command.source_id, &control)? {
        return Err(CommandError::new(
            ErrorCode::DependencyConflict,
            "source_id",
            "a weld has element links that an array does not carry over",
        )
        .with_rule("DV-REL-011")
        .into());
    } else if crate::bolt::is_bolt(&base_request, &command.source_id, &control)? {
        return Err(CommandError::new(
            ErrorCode::DependencyConflict,
            "source_id",
            "a bolt group has element links that an array does not carry over",
        )
        .with_rule("DV-REL-011")
        .into());
    } else if crate::level::is_level(&base_request, &command.source_id, &control)? {
        return Err(CommandError::new(
            ErrorCode::UnsupportedProperty,
            "source_id",
            "a level is not arrayed",
        )
        .into());
    } else {
        return Err(not_found());
    };
    for copy in &copies {
        if let Some(digest) = copy_material(copy) {
            materials(&digest)?;
        }
    }
    // The array is a replay when the head holds every copy of the source's current state and
    // none of them is itself an instance.
    let held: Vec<Option<CopiedElement>> = command
        .new_ids
        .iter()
        .map(|id| {
            plates
                .iter()
                .find(|r| r.authored.id == *id)
                .map(|r| CopiedElement::Plate(Box::new(r.clone())))
                .or_else(|| {
                    members
                        .iter()
                        .find(|r| r.authored.id == *id)
                        .map(|r| CopiedElement::Member(Box::new(r.clone())))
                })
        })
        .collect();
    let same_state = |a: &CopiedElement, b: &CopiedElement| match (a, b) {
        (CopiedElement::Plate(a), CopiedElement::Plate(b)) => a.authored == b.authored,
        (CopiedElement::Member(a), CopiedElement::Member(b)) => a.authored == b.authored,
        _ => false,
    };
    let complete = held.iter().zip(&copies).all(|(head_copy, copy)| {
        head_copy
            .as_ref()
            .is_some_and(|h| same_state(h, copy) && !instances.is_instance(copy_id(h)))
    });
    let any_held = held.iter().any(Option::is_some);

    let op_host = host_uuid(&request.op_id.to_string())?;
    let fingerprint = command.content_fingerprint()?;
    let (prior, recorded) = prior_op_with(store, &op_host, || {
        if complete {
            Ok(fingerprint)
        } else {
            digest_of(b"no accepted request matches the head state")
        }
    })?;
    match prior {
        PriorOp::Accepted { fingerprint: seen } if seen == fingerprint => {
            let elements = held
                .iter()
                .map(|h| {
                    h.clone()
                        .ok_or_else(|| integrity("replayed element is absent from the head"))
                })
                .collect::<Result<Vec<_>, PlateError>>()?;
            return Ok(Arrayed {
                replayed: true,
                elements,
                revision: replay_summary(&head, recorded)?,
                footprint: None,
                changed_layers: Vec::new(),
            });
        }
        PriorOp::Accepted { .. } => {
            return Err(CommandError::new(
                ErrorCode::OpIdReuse,
                "op_id",
                "operation id was accepted with different content",
            )
            .into());
        }
        PriorOp::Unseen => {}
    }
    if any_held {
        return Err(CommandError::new(
            ErrorCode::DuplicateId,
            "new_ids",
            "a copy identity is already in use",
        )
        .into());
    }

    // The copies stay on the source's level (D2), as a single copy does.
    let level = levels.level_of(&command.source_id);
    let mut footprint: Option<Footprint> = None;
    for (id, copy) in command.new_ids.iter().zip(&copies) {
        let mut one = Footprint::for_copy(
            command.source_id,
            *id,
            copy_profile(copy),
            copy_material(copy),
        )?;
        if canonical != command.source_id {
            one = one.union(Footprint::for_copy(
                canonical,
                *id,
                copy_profile(copy),
                copy_material(copy),
            )?)?;
        }
        if let Some(level) = level {
            one = one.union(Footprint::for_level(*id, Some(level))?)?;
        }
        footprint = Some(match footprint {
            Some(all) => all.union(one)?,
            None => one,
        });
    }
    let footprint = footprint.ok_or_else(|| integrity("the array produced no copies"))?;
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
    // The links the adapter does not carry over must not be dropped silently: the layer engine
    // plans every copy, dropping assembly membership and connections (DV-REL-011) and keeping
    // the definition and level links.
    let index = author.element_index(&opened.context, &control)?;
    let is_level_link = |l: &beamcraft_layer_engine::index::PlannedLink| {
        l.name == "bim:level" && matches!(l.target, LinkTarget::Element(_))
    };
    for id in &command.new_ids {
        let plan = index.copy_plan(
            &LayerElementId::parse(&command.source_id.to_string())?,
            &LayerElementId::parse(&id.to_string())?,
        )?;
        if plan
            .links
            .iter()
            .any(|l| !matches!(l.target, LinkTarget::Definition(_)) && !is_level_link(l))
        {
            return Err(CommandError::new(
                ErrorCode::DependencyConflict,
                "source_id",
                "the source has element links that an array does not carry over yet",
            )
            .with_rule("DV-REL-011")
            .into());
        }
    }
    let mut changes = Vec::new();
    for copy in &copies {
        changes.extend(creation_changes(copy)?);
    }
    if let Some(level) = level {
        for id in &command.new_ids {
            changes.push(Change::Level {
                id: id.to_string(),
                level: Some(level.to_string()),
            });
        }
    }
    let (committed, changed_layers) = prepare_and_publish(
        &mut author,
        &opened,
        changes,
        &footprint,
        &before,
        PublicationTail {
            store,
            control: &control,
            schema,
            fingerprint,
            project: &project,
            workspace: &workspace,
            revision_id: &request.revision_id,
            op: op_host,
            op_id: request.op_id.to_string(),
            actor: request.actor.to_string(),
            base_generation,
            expected: &expected,
        },
        hook,
    )?;
    Ok(Arrayed {
        replayed: false,
        elements: copies,
        revision: summarize(&committed.info),
        footprint: Some(footprint),
        changed_layers,
    })
}
