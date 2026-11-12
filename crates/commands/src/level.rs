// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D2 adapter: `CreateLevel` and `UpdateLevel` committed as the next revision, plus a read of the
//! levels of the head. The assignment of a plate, beam or column to a level is the level field of
//! `UpdateElement` (see [`crate::edit`]); deleting a level is `delete-element`.
//!
//! A level is an element of kind `level` with its own UUIDv7 under `/World/Levels`, a label and an
//! `bim:elevationMm`, and no placement, material or derived body. Assignment is the one
//! `bim:level` relationship that a plate, beam or column authors towards its level: the record
//! lives with its source, so assigning writes only that element. A level has no geometric effect:
//! nothing here rebuilds or moves a body.
//!
//! Honesty limits: replay of an `op_id` is decided from the revision the host ledger recorded for
//! it, as in `weld`: a creation is a replay only when that revision introduced a level of that
//! id with the same name and elevation; another request under the same id is `OP_ID_REUSE`. An
//! update replays from the request evidence of its revision. A new elevation moves no assigned
//! element, and a level is not copied. Deleting a level that
//! elements use is `DEPENDENCY_CONFLICT`; there is no cascade.

use crate::assembly::read_prims;
use crate::coordination::pinned_owns_element;
use crate::plate::{check_footprint, number, prior_op_with, property, replay_summary, text};
use crate::publication::{PublishRequest, changed_layers, publish_revision};
use beamcraft_engine::footprint::Footprint;
use beamcraft_engine::level::{HolderKind, LevelRecord, Levels};
use beamcraft_host::revision::{CommitHook, CommitOutcome, RevisionError, RevisionStore, Uuid7};
use beamcraft_layer_engine::authored::{
    Change, Element, Placement as LayerPlacement, Prepare, Shape, schema_identity, validate_layers,
};
use beamcraft_layer_engine::{Control, OpenRequest, Prim, Reader};
use beamcraft_model::command::{CommandError, CreateLevel, UpdateLevel};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId, ProjectId, RevisionId, WorkspaceId};
use beamcraft_model::level::CreateLevels;
use session::Refusal;
use session::pipeline::{
    Base, RevisionSummary, base_of, control, digest_of, host_uuid, integrity, load_base, open_base,
    summarize,
};
use std::collections::BTreeMap;
use steel::engine::transaction::PriorOp;

/// Identities of one D2 call beyond the command itself.
#[derive(Debug, Clone)]
pub struct LevelContext {
    /// The logical operation.
    pub op_id: OpId,
    /// Identity of the revision this call creates.
    pub revision_id: RevisionId,
    /// The acting user (trusted context).
    pub actor: Uuid7,
    /// Project of a store that has no revision yet; its first revision records it.
    pub project: ProjectId,
    /// Workspace of a store that has no revision yet.
    pub workspace: WorkspaceId,
}

/// One level of the head.
#[derive(Debug, Clone, PartialEq)]
pub struct LevelView {
    /// Identity.
    pub id: ElementId,
    /// Label.
    pub name: String,
    /// Elevation in millimetres.
    pub elevation_mm: f64,
    /// The plates, beams and columns assigned to it, in identity order.
    pub elements: Vec<ElementId>,
}

/// Result of one D2 creation.
#[derive(Debug, Clone, PartialEq)]
pub struct LevelChange {
    /// True when the same operation was already committed and nothing was written.
    pub replayed: bool,
    /// The revision that holds the result.
    pub revision: RevisionSummary,
    /// The declared footprint; `None` for a replay.
    pub footprint: Option<Footprint>,
    /// Names of the layers whose bytes changed (empty for a replay).
    pub changed_layers: Vec<String>,
    /// The levels of the resulting head.
    pub levels: Vec<LevelView>,
    /// The beams and columns an `update-level --move-elements` carried to the new elevation, in
    /// identity order; empty for any other call and for a replay.
    pub moved: Vec<ElementId>,
}

/// The kinds, levels and level records of a pinned set of prims. Elements of foreign or unknown
/// kinds count as used identities and nothing else.
pub(crate) fn levels_of(prims: &[Prim]) -> Result<Levels, Refusal> {
    let by_path: BTreeMap<&str, ElementId> = prims
        .iter()
        .filter_map(|p| {
            let id = ElementId::parse(p.element_id.as_deref()?).ok()?;
            Some((p.path.as_str(), id))
        })
        .collect();
    let mut levels = Levels::default();
    for prim in prims {
        let Some(id) = by_path.get(prim.path.as_str()).copied() else {
            continue;
        };
        // Only owned 0.1.0 parametric elements are interpreted, as the structural validator does;
        // newer, foreign or other-mode content stays inert and counts as a used identity only.
        let supported = !prim.sources.is_empty()
            && prim.sources.iter().all(|s| s.owned)
            && property(prim, "bim:schemaVersion").and_then(text) == Some("0.1.0")
            && property(prim, "bim:geometryMode").and_then(text) == Some("parametric");
        let holder = match property(prim, "bim:elementKind")
            .and_then(text)
            .filter(|_| supported)
        {
            Some("plate") => HolderKind::Plate,
            Some("beam") => HolderKind::Beam,
            Some("column") => HolderKind::Column,
            Some("level") => {
                let name = property(prim, "bim:name")
                    .and_then(text)
                    .ok_or_else(|| integrity("level has no label"))?;
                let elevation_mm = property(prim, "bim:elevationMm")
                    .and_then(number)
                    .ok_or_else(|| integrity("level has no elevation"))?;
                levels.insert_level(
                    id,
                    LevelRecord {
                        name: name.to_owned(),
                        elevation_mm,
                    },
                );
                continue;
            }
            _ => {
                levels.insert_other(id);
                continue;
            }
        };
        levels.insert_holder(id, holder);
        if let Some(r) = prim.relationships.iter().find(|r| r.name == "bim:level") {
            match r.targets.as_slice() {
                [] => {}
                [target] => {
                    let level = by_path
                        .get(target.as_str())
                        .copied()
                        .ok_or_else(|| integrity("level target is not an element"))?;
                    levels.set_level(id, level);
                }
                _ => return Err(integrity("element has more than one level target")),
            }
        }
    }
    Ok(levels)
}

pub(crate) fn model_of(request: &OpenRequest, control: &Control) -> Result<Levels, Refusal> {
    // Stored bytes are verified, not interpreted: the structural rules (D2-V01 among them) run
    // before the levels are read, so a damaged graph is refused rather than reported or replayed.
    validate_layers(request.clone(), schema_identity()?, control)?;
    let mut reader = Reader::default();
    let opened = reader.open(request.clone(), control)?;
    levels_of(&read_prims(&reader, &opened.context, control)?)
}

/// The levels of a model with the elements that use them.
pub fn views(levels: &Levels) -> Vec<LevelView> {
    levels
        .levels()
        .map(|(id, record)| LevelView {
            id: *id,
            name: record.name.clone(),
            elevation_mm: record.elevation_mm,
            elements: levels.elements_of(id),
        })
        .collect()
}

/// Whether the pinned model holds a level of this identity (for `delete-element`).
pub(crate) fn is_level(
    request: &OpenRequest,
    id: &ElementId,
    control: &Control,
) -> Result<bool, Refusal> {
    Ok(model_of(request, control)?.level(id).is_some())
}

/// The levels of the verified head (empty for a store without a revision), in ascending identity
/// order.
pub fn levels(store: &RevisionStore) -> Result<Vec<LevelView>, Refusal> {
    let control = control();
    let revision = match store.open() {
        Ok(revision) => revision,
        Err(RevisionError::NoRevision) => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let content = revision.manifest().content();
    let damaged = |_| integrity("head manifest identity is not a UUIDv7");
    let project = ProjectId::parse(content.project().as_str()).map_err(damaged)?;
    let workspace = WorkspaceId::parse(content.workspace().as_str()).map_err(damaged)?;
    let (_, base) = load_base(store, &project, &workspace)?;
    Ok(views(&model_of(&open_base(&base)?, &control)?))
}

/// The level `id` as the operation committed at `sequence` created it: present in that revision
/// and absent (as any element) from the revision before it. `None` when that revision did not
/// create such a level, so the operation id belongs to another request.
fn created_by(
    store: &RevisionStore,
    sequence: u64,
    id: &ElementId,
    control: &Control,
) -> Result<Option<LevelRecord>, Refusal> {
    let at = store.open_at(sequence)?;
    let model = model_of(&open_base(&base_of(&at)?)?, control)?;
    let Some(record) = model.level(id).cloned() else {
        return Ok(None);
    };
    if sequence > 1 {
        let before = store.open_at(sequence - 1)?;
        if model_of(&open_base(&base_of(&before)?)?, control)?.contains(id) {
            return Ok(None);
        }
    }
    Ok(Some(record))
}

/// Creates one level and publishes the next revision (the first one records the project), or
/// replays the operation.
///
/// Steps, each failing closed: admit the command; open and verify the head; decide replay from
/// the host ledger and the head state; refuse a used identity; author the level with U02; check
/// the layer engine's writes against the declared footprint; commit through U06/U07 with the
/// strict verifier.
pub fn create_level(
    store: &RevisionStore,
    context: &LevelContext,
    command: &CreateLevel,
    hook: &mut dyn CommitHook,
) -> Result<LevelChange, Refusal> {
    command.validate_arguments()?;
    let control = control();
    let (head, base) = load_base(store, &context.project, &context.workspace)?;
    let before: BTreeMap<String, _> = base
        .assets
        .iter()
        .map(|a| (a.name.clone(), a.digest))
        .collect();
    let base_request = open_base(&base)?;
    let model = model_of(&base_request, &control)?;
    let Base {
        project,
        workspace,
        generation: base_generation,
        expected,
        ..
    } = base;
    let op_host = host_uuid(&context.op_id.to_string())?;
    // The ledger proves that the operation committed; what it committed is read from its own
    // revision, never from the head, so another level that happens to match is not a replay.
    let (prior, recorded) = prior_op_with(store, &op_host, || {
        digest_of(b"the accepted request is decided from the operation's revision")
    })?;
    let accepted = match (&prior, &recorded) {
        (PriorOp::Accepted { .. }, Some((_, sequence, _))) => {
            created_by(store, *sequence, &command.new_id, &control)?
        }
        _ => None,
    };
    let same = accepted
        .as_ref()
        .is_some_and(|r| r.name == command.name && r.elevation_mm == command.elevation_mm);
    match prior {
        PriorOp::Accepted { .. } if same => {
            return Ok(LevelChange {
                replayed: true,
                revision: replay_summary(&head, recorded)?,
                footprint: None,
                changed_layers: Vec::new(),
                levels: views(&model),
                moved: Vec::new(),
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
    let creation = model.create(command)?;
    let schema = schema_identity()?;
    let (mut author, opened) =
        crate::scope::open_author(store, base_request, true, schema, &control)?;
    let prepared = author.prepare(
        Prepare {
            context: opened.context.clone(),
            op_id: context.op_id.to_string(),
            actor: context.actor.to_string(),
            schema,
            resources: Vec::new(),
            changes: vec![Change::Create(Element {
                id: command.new_id.to_string(),
                name: command.name.clone(),
                shape: Shape::Level {
                    elevation_mm: command.elevation_mm,
                },
                placement: LayerPlacement {
                    translation_mm: [0.0; 3],
                    orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
                },
                material: None,
            })],
        },
        &control,
    )?;
    check_footprint(&prepared, &creation.footprint)?;
    let changed_layers = changed_layers(&prepared, &before);
    let committed = publish_revision(
        store,
        &PublishRequest {
            footprint: &creation.footprint,
            fingerprint: command.content_fingerprint()?,
            prepared: &prepared,
            schema,
            project: &project,
            workspace: &workspace,
            revision_id: &context.revision_id,
            op: op_host,
            base_generation,
            expected: &expected,
        },
        hook,
    )?;
    if committed.outcome == CommitOutcome::Replayed {
        return Err(integrity("an unseen operation was reported as replayed"));
    }
    Ok(LevelChange {
        replayed: false,
        revision: summarize(&committed.info),
        footprint: Some(creation.footprint),
        changed_layers,
        levels: levels(store)?,
        moved: Vec::new(),
    })
}

/// Creates several levels in one revision, or replays the operation from its recorded request
/// evidence. The levels are decided together: one taken identity refuses them all and nothing is
/// written.
pub fn create_levels(
    store: &RevisionStore,
    context: &LevelContext,
    command: &CreateLevels,
    hook: &mut dyn CommitHook,
) -> Result<LevelChange, Refusal> {
    let fingerprint = command.content_fingerprint()?;
    let control = control();
    let (head, base) = load_base(store, &context.project, &context.workspace)?;
    let base_request = open_base(&base)?;
    let model = model_of(&base_request, &control)?;
    let op_host = host_uuid(&context.op_id.to_string())?;
    let (prior, recorded) = prior_op_with(store, &op_host, || Ok(fingerprint))?;
    if let PriorOp::Accepted { .. } = prior {
        let Some((_, sequence, _)) = &recorded else {
            return Err(integrity("accepted level operation has no revision"));
        };
        let evidence = crate::command_evidence::read(&store.open_at(*sequence)?)?
            .ok_or_else(|| integrity("level operation has no request evidence"))?;
        if evidence.fingerprint != fingerprint {
            return Err(CommandError::new(
                ErrorCode::OpIdReuse,
                "op_id",
                "operation id was accepted with different content",
            )
            .into());
        }
        return Ok(LevelChange {
            replayed: true,
            revision: replay_summary(&head, recorded)?,
            footprint: None,
            changed_layers: Vec::new(),
            levels: views(&model),
            moved: Vec::new(),
        });
    }
    let (creation, created) = model.create_all(command)?;
    let before: BTreeMap<String, _> = base
        .assets
        .iter()
        .map(|a| (a.name.clone(), a.digest))
        .collect();
    let Base {
        project,
        workspace,
        generation: base_generation,
        expected,
        ..
    } = base;
    let schema = schema_identity()?;
    let (mut author, opened) =
        crate::scope::open_author(store, base_request, true, schema, &control)?;
    let changes = created
        .iter()
        .map(|level| {
            Change::Create(Element {
                id: level.new_id.to_string(),
                name: level.name.clone(),
                shape: Shape::Level {
                    elevation_mm: level.elevation_mm,
                },
                placement: LayerPlacement {
                    translation_mm: [0.0; 3],
                    orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
                },
                material: None,
            })
        })
        .collect();
    let prepared = author.prepare(
        Prepare {
            context: opened.context.clone(),
            op_id: context.op_id.to_string(),
            actor: context.actor.to_string(),
            schema,
            resources: Vec::new(),
            changes,
        },
        &control,
    )?;
    check_footprint(&prepared, &creation.footprint)?;
    let changed_layers = changed_layers(&prepared, &before);
    let committed = publish_revision(
        store,
        &PublishRequest {
            footprint: &creation.footprint,
            fingerprint,
            prepared: &prepared,
            schema,
            project: &project,
            workspace: &workspace,
            revision_id: &context.revision_id,
            op: op_host,
            base_generation,
            expected: &expected,
        },
        hook,
    )?;
    if committed.outcome == CommitOutcome::Replayed {
        return Err(integrity("an unseen operation was reported as replayed"));
    }
    Ok(LevelChange {
        replayed: false,
        revision: summarize(&committed.info),
        footprint: Some(creation.footprint),
        changed_layers,
        levels: levels(store)?,
        moved: Vec::new(),
    })
}

/// Changes the name, the elevation or both of a level and publishes the next revision, or replays
/// the operation from its recorded request evidence.
pub fn update_level(
    store: &RevisionStore,
    context: &LevelContext,
    command: &UpdateLevel,
    hook: &mut dyn CommitHook,
) -> Result<LevelChange, Refusal> {
    let fingerprint = command.content_fingerprint()?;
    let control = control();
    let (head, base) = load_base(store, &context.project, &context.workspace)?;
    let base_request = open_base(&base)?;
    let model = model_of(&base_request, &control)?;
    let op_host = host_uuid(&context.op_id.to_string())?;
    let (prior, recorded) = prior_op_with(store, &op_host, || Ok(fingerprint))?;
    if let PriorOp::Accepted { .. } = prior {
        let Some((_, sequence, _)) = &recorded else {
            return Err(integrity("accepted level operation has no revision"));
        };
        let evidence = crate::command_evidence::read(&store.open_at(*sequence)?)?
            .ok_or_else(|| integrity("level operation has no request evidence"))?;
        if evidence.fingerprint != fingerprint {
            return Err(CommandError::new(
                ErrorCode::OpIdReuse,
                "op_id",
                "operation id was accepted with different content",
            )
            .into());
        }
        return Ok(LevelChange {
            replayed: true,
            revision: replay_summary(&head, recorded)?,
            footprint: None,
            changed_layers: Vec::new(),
            levels: views(&model),
            moved: Vec::new(),
        });
    }
    // A pinned coordination level is foreign bytes: no authority of this store reaches it.
    if !model.contains(&command.target_id) && pinned_owns_element(store, &command.target_id)? {
        return Err(CommandError::new(
            ErrorCode::UnsupportedProperty,
            "target_id",
            "the level belongs to a pinned coordination project and is read-only",
        )
        .into());
    }
    let update = model.update(command)?;
    // The beams and columns on the level follow a new elevation in the same revision, read from
    // the same pinned base.
    let follow = match (
        command.move_elements,
        update.elevation_mm,
        model.level(&command.target_id),
    ) {
        (true, Some(to_mm), Some(record)) => {
            crate::level_follow::follow_level(&base_request, &control, record.elevation_mm, to_mm)?
        }
        _ => crate::level_follow::Follow::default(),
    };
    if !follow.moved.is_empty() && !crate::scope::is_classic(store)? {
        return Err(CommandError::new(
            ErrorCode::UnsupportedProperty,
            "move_elements",
            "elements follow a level only in a store without section scopes",
        )
        .into());
    }
    let footprint = update.footprint.union(follow.footprint)?;
    let before: BTreeMap<String, _> = base
        .assets
        .iter()
        .map(|a| (a.name.clone(), a.digest))
        .collect();
    let Base {
        project,
        workspace,
        generation: base_generation,
        expected,
        ..
    } = base;
    let schema = schema_identity()?;
    let (mut author, opened) =
        crate::scope::open_author(store, base_request, true, schema, &control)?;
    let index = author.element_index(&opened.context, &control)?;
    let id = beamcraft_layer_engine::index::ElementId::parse(&command.target_id.to_string())?;
    author.authorize_write(&index, &id, &control)?;
    let mut changes = Vec::with_capacity(2usize.saturating_add(follow.changes.len()));
    if let Some(name) = &update.name {
        changes.push(Change::Label {
            id: command.target_id.to_string(),
            name: name.clone(),
        });
    }
    if let Some(elevation_mm) = update.elevation_mm {
        changes.push(Change::LevelElevation {
            id: command.target_id.to_string(),
            elevation_mm,
        });
    }
    changes.extend(follow.changes);
    let prepared = author.prepare(
        Prepare {
            context: opened.context.clone(),
            op_id: context.op_id.to_string(),
            actor: context.actor.to_string(),
            schema,
            resources: Vec::new(),
            changes,
        },
        &control,
    )?;
    check_footprint(&prepared, &footprint)?;
    let changed_layers = changed_layers(&prepared, &before);
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
            op: op_host,
            base_generation,
            expected: &expected,
        },
        hook,
    )?;
    if committed.outcome == CommitOutcome::Replayed {
        return Err(integrity("an unseen operation was reported as replayed"));
    }
    Ok(LevelChange {
        replayed: false,
        revision: summarize(&committed.info),
        footprint: Some(footprint),
        changed_layers,
        levels: levels(store)?,
        moved: follow.moved,
    })
}
