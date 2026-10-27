// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A01 adapter: `CreateAssembly`, `AttachToAssembly` and `DetachFromAssembly`, each committed as
//! the next revision, plus a read of the assembly hierarchy of the head.
//!
//! An assembly is an element of kind `assembly` with its own UUIDv7, label and identity placement,
//! and no shape, material or derived body. Membership is the one `bim:partOf` relationship that a
//! part (plate, beam, column or nested assembly) authors towards its assembly: the record lives
//! with its source, so attaching or detaching writes only the part. The engine
//! ([`beamcraft_engine::assembly`]) decides kinds, cardinality, self-links and cycles from the head
//! model; the layer engine re-checks the same rules on the candidate and the strict verifier on
//! the committed bytes.
//!
//! Honesty limits: replay of an `op_id` is decided from the revision of that operation, as for
//! bolts, levels and welds: the request is the accepted one only if the state it asks for holds in
//! that revision and did not hold in the one before. The head is not consulted, so a later
//! operation does not break a replay. A revision at or below the compaction floor of a compacted
//! history cannot be checked this way and answers `NOT_FOUND`, not a replay. Copying a part
//! that has a `part-of` record creates a loose duplicate (`DV-REL-011`); a multi-element copy
//! is not implemented. Deleting an assembly that still has parts is `DEPENDENCY_CONFLICT`; there
//! is no cascade. An assembly cannot be renamed or moved yet.

use crate::plate::{
    AUTHORED_LAYER, Base, DERIVED_LAYER, PlateError, RevisionSummary, base_of, check_footprint,
    control, host_uuid, integrity, load_base, open_base, prior_op_with, property, replay_summary,
    summarize, text,
};
use crate::publication::{PublishRequest, changed_layers, publish_revision};
use beamcraft_engine::assembly::{Membership, PartKind};
use beamcraft_engine::footprint::Footprint;
use beamcraft_engine::transaction::PriorOp;
use beamcraft_host::revision::{CommitHook, CommitOutcome, RevisionError, RevisionStore, Uuid7};
use beamcraft_layer_engine::authored::{
    Author, Change, EditTargets, Element, Placement as LayerPlacement, Prepare, Shape,
    schema_identity, validate_layers,
};
use beamcraft_layer_engine::{Control, OpenRequest, Prim, Reader};
use beamcraft_model::command::{
    AttachToAssembly, CommandError, CreateAssembly, DetachFromAssembly, ExplodeAssembly,
    SetAssemblyMainPart,
};
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId, ProjectId, RevisionId, WorkspaceId};
use std::collections::BTreeMap;

/// Identities of one A01 call beyond the command itself.
#[derive(Debug, Clone)]
pub struct AssemblyContext {
    /// The logical operation.
    pub op_id: OpId,
    /// Identity of the revision this call creates.
    pub revision_id: RevisionId,
    /// The acting user (trusted context).
    pub actor: Uuid7,
    /// Project of a store that has no revision yet.
    pub project: ProjectId,
    /// Workspace of a store that has no revision yet.
    pub workspace: WorkspaceId,
}

/// One assembly of the head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssemblyView {
    /// Identity.
    pub id: ElementId,
    /// Label.
    pub name: String,
    /// The assembly it is a part of, when nested.
    pub parent: Option<ElementId>,
    /// Direct parts, in identity order.
    pub parts: Vec<ElementId>,
    /// The main part, when the assembly has named one (A16.1).
    pub main: Option<ElementId>,
}

/// Result of one A01 command.
#[derive(Debug, Clone, PartialEq)]
pub struct AssemblyChange {
    /// True when the same operation was already committed and nothing was written.
    pub replayed: bool,
    /// The revision that holds the result.
    pub revision: RevisionSummary,
    /// The declared footprint; `None` for a replay.
    pub footprint: Option<Footprint>,
    /// Names of the layers whose bytes changed (empty for a replay).
    pub changed_layers: Vec<String>,
    /// The assemblies of the resulting head.
    pub assemblies: Vec<AssemblyView>,
}

/// The supported elements, labels of assemblies and membership of a pinned model.
struct Model {
    membership: Membership,
    names: BTreeMap<ElementId, String>,
    /// Every element identity, whatever its kind.
    ids: Vec<ElementId>,
}

pub(crate) fn read_prims(
    reader: &Reader,
    context: &beamcraft_layer_engine::Context,
    control: &Control,
) -> Result<Vec<Prim>, PlateError> {
    let mut prims = Vec::new();
    let mut cursor = None;
    loop {
        let page = reader.read(context, cursor.as_ref(), control)?;
        prims.extend(page.items);
        match page.next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    Ok(prims)
}

fn model_of(request: &OpenRequest, control: &Control) -> Result<Model, PlateError> {
    // Stored bytes are verified, not interpreted: the structural rules (A01-V01 among them) run
    // before the hierarchy is read, so a damaged graph is refused rather than reported or replayed.
    validate_layers(request.clone(), schema_identity()?, control)?;
    let mut reader = Reader::default();
    let opened = reader.open(request.clone(), control)?;
    let prims = read_prims(&reader, &opened.context, control)?;
    let by_path: BTreeMap<&str, ElementId> = prims
        .iter()
        .filter_map(|p| {
            let id = ElementId::parse(p.element_id.as_deref()?).ok()?;
            Some((p.path.as_str(), id))
        })
        .collect();
    let mut model = Model {
        membership: Membership::default(),
        names: BTreeMap::new(),
        ids: by_path.values().copied().collect(),
    };
    for prim in &prims {
        let Some(id) = by_path.get(prim.path.as_str()).copied() else {
            continue;
        };
        let kind = match property(prim, "bim:elementKind").and_then(text) {
            Some("plate") => PartKind::Plate,
            Some("beam") => PartKind::Beam,
            Some("column") => PartKind::Column,
            Some("assembly") => PartKind::Assembly,
            _ => continue,
        };
        model.membership.insert(id, kind);
        if kind == PartKind::Assembly {
            let name = property(prim, "bim:name")
                .and_then(text)
                .ok_or_else(|| integrity("assembly has no label"))?;
            model.names.insert(id, name.to_owned());
        }
        if let Some(r) = prim.relationships.iter().find(|r| r.name == "bim:partOf") {
            match r.targets.as_slice() {
                [] => {}
                [target] => {
                    let parent = by_path
                        .get(target.as_str())
                        .copied()
                        .ok_or_else(|| integrity("part-of target is not an element"))?;
                    model.membership.set_parent(id, parent);
                }
                _ => return Err(integrity("element has more than one part-of target")),
            }
        }
        if let Some(r) = prim.relationships.iter().find(|r| r.name == "bim:mainPart") {
            match r.targets.as_slice() {
                [] => {}
                [target] => {
                    let part = by_path
                        .get(target.as_str())
                        .copied()
                        .ok_or_else(|| integrity("main-part target is not an element"))?;
                    model.membership.set_main_part(id, part);
                }
                _ => return Err(integrity("assembly has more than one main part")),
            }
        }
    }
    Ok(model)
}

fn views(model: &Model) -> Vec<AssemblyView> {
    model
        .membership
        .assemblies()
        .map(|id| AssemblyView {
            id,
            name: model.names.get(&id).cloned().unwrap_or_default(),
            parent: model.membership.parent(&id),
            parts: model.membership.parts(&id),
            main: model.membership.main_part(&id),
        })
        .collect()
}

/// Whether the pinned model holds an element that is the main part of its assembly (for
/// `delete-element`).
pub(crate) fn is_main_part(
    request: &OpenRequest,
    id: &ElementId,
    control: &Control,
) -> Result<bool, PlateError> {
    let model = model_of(request, control)?;
    Ok(model
        .membership
        .assemblies()
        .any(|a| model.membership.main_part(&a) == Some(*id)))
}

/// Whether the pinned model holds an assembly of this identity (for `delete-element`).
pub(crate) fn is_assembly(
    request: &OpenRequest,
    id: &ElementId,
    control: &Control,
) -> Result<bool, PlateError> {
    Ok(model_of(request, control)?.membership.kind(id) == Some(PartKind::Assembly))
}

/// The assemblies of the verified head (empty for a store without a revision).
pub fn assemblies(store: &RevisionStore) -> Result<Vec<AssemblyView>, PlateError> {
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

/// The planned command of [`apply`]: the layer change, the footprint, and how to recognise its
/// result in the head for a replay.
struct Plan {
    changes: Vec<Change>,
    footprint: Footprint,
}

fn reuse() -> PlateError {
    CommandError::new(
        ErrorCode::OpIdReuse,
        "op_id",
        "operation id was accepted with different content",
    )
    .into()
}

/// Whether `holds` is true in the revision at `sequence` and was not in the one before it.
fn introduced_by(
    store: &RevisionStore,
    sequence: u64,
    holds: &dyn Fn(&Model) -> bool,
    control: &Control,
) -> Result<bool, PlateError> {
    let model_at = |sequence: u64| -> Result<Model, PlateError> {
        let revision = store.open_at(sequence)?;
        model_of(&open_base(&base_of(&revision)?)?, control)
    };
    if !holds(&model_at(sequence)?) {
        return Ok(false);
    }
    Ok(match sequence.checked_sub(1).filter(|s| *s > 0) {
        Some(before) => !holds(&model_at(before)?),
        None => true,
    })
}

/// The shared path of every A01 command: open the head, decide replay from the ledger and the
/// revision of the accepted operation (see [`introduced_by`]), plan against the head model,
/// author, check the footprint, commit.
fn apply(
    store: &RevisionStore,
    context: &AssemblyContext,
    fingerprint: ContentDigest,
    needs_head: bool,
    in_head: &dyn Fn(&Model) -> bool,
    plan: &dyn Fn(&Model) -> Result<Plan, PlateError>,
    hook: &mut dyn CommitHook,
) -> Result<AssemblyChange, PlateError> {
    let control = control();
    let (head, base) = load_base(store, &context.project, &context.workspace)?;
    if needs_head && head.is_none() {
        return Err(CommandError::new(
            ErrorCode::NotFound,
            "part_id",
            "the part is not in the head revision",
        )
        .into());
    }
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
    // The ledger records that the operation committed, not what it asked for. The request is the
    // accepted one only if the revision of that operation is where its effect first appears: the
    // state it asks for holds there and did not hold in the revision before. The head cannot
    // decide this, since it may hold the same state through another operation.
    let (prior, recorded) = prior_op_with(store, &op_host, || Ok(fingerprint))?;
    if let PriorOp::Accepted { .. } = prior {
        let Some((_, sequence, _)) = &recorded else {
            return Err(integrity("replay without a recorded revision"));
        };
        if !introduced_by(store, *sequence, in_head, &control)? {
            return Err(reuse());
        }
        return Ok(AssemblyChange {
            replayed: true,
            revision: replay_summary(&head, recorded)?,
            footprint: None,
            changed_layers: Vec::new(),
            assemblies: views(&model),
        });
    }
    let Plan { changes, footprint } = plan(&model)?;
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
    Ok(AssemblyChange {
        replayed: false,
        revision: summarize(&committed.info),
        footprint: Some(footprint),
        changed_layers,
        assemblies: assemblies(store)?,
    })
}

/// Creates one assembly and publishes the next revision, or replays the operation.
pub fn create_assembly(
    store: &RevisionStore,
    context: &AssemblyContext,
    command: &CreateAssembly,
    hook: &mut dyn CommitHook,
) -> Result<AssemblyChange, PlateError> {
    let fingerprint = command.content_fingerprint()?;
    apply(
        store,
        context,
        fingerprint,
        false,
        &|m| m.names.get(&command.new_id) == Some(&command.name),
        &|m| {
            if m.ids.contains(&command.new_id) {
                return Err(CommandError::new(
                    ErrorCode::DuplicateId,
                    "new_id",
                    "the identity is already in use",
                )
                .into());
            }
            Ok(Plan {
                changes: vec![Change::Create(Element {
                    id: command.new_id.to_string(),
                    name: command.name.clone(),
                    shape: Shape::Assembly,
                    placement: LayerPlacement {
                        translation_mm: [0.0; 3],
                        orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
                    },
                    material: None,
                })],
                footprint: Footprint::for_create(command.new_id, None)?,
            })
        },
        hook,
    )
}

/// Makes a part a member of an assembly and publishes the next revision, or replays.
pub fn attach(
    store: &RevisionStore,
    context: &AssemblyContext,
    command: &AttachToAssembly,
    hook: &mut dyn CommitHook,
) -> Result<AssemblyChange, PlateError> {
    let fingerprint = command.content_fingerprint()?;
    apply(
        store,
        context,
        fingerprint,
        true,
        &|m| m.membership.parent(&command.part_id) == Some(command.assembly_id),
        &|m| {
            let decision = m.membership.attach(command)?;
            Ok(Plan {
                changes: vec![Change::PartOf {
                    id: command.part_id.to_string(),
                    assembly: Some(command.assembly_id.to_string()),
                }],
                footprint: decision.footprint,
            })
        },
        hook,
    )
}

/// Ends the membership of a part and publishes the next revision, or replays.
pub fn detach(
    store: &RevisionStore,
    context: &AssemblyContext,
    command: &DetachFromAssembly,
    hook: &mut dyn CommitHook,
) -> Result<AssemblyChange, PlateError> {
    let fingerprint = command.content_fingerprint()?;
    apply(
        store,
        context,
        fingerprint,
        true,
        &|m| {
            m.membership.kind(&command.part_id).is_some()
                && m.membership.parent(&command.part_id).is_none()
        },
        &|m| {
            let decision = m.membership.detach(command)?;
            Ok(Plan {
                changes: vec![Change::PartOf {
                    id: command.part_id.to_string(),
                    assembly: None,
                }],
                footprint: decision.footprint,
            })
        },
        hook,
    )
}

/// Names the main part of an assembly and publishes the next revision, or replays. The record
/// lives on the assembly, so only the assembly is written.
pub fn set_main_part(
    store: &RevisionStore,
    context: &AssemblyContext,
    command: &SetAssemblyMainPart,
    hook: &mut dyn CommitHook,
) -> Result<AssemblyChange, PlateError> {
    let fingerprint = command.content_fingerprint()?;
    apply(
        store,
        context,
        fingerprint,
        true,
        &|m| m.membership.main_part(&command.assembly_id) == Some(command.part_id),
        &|m| {
            let decision = m.membership.set_main(command)?;
            Ok(Plan {
                changes: vec![Change::MainPart {
                    id: command.assembly_id.to_string(),
                    part: command.part_id.to_string(),
                }],
                footprint: decision.footprint,
            })
        },
        hook,
    )
}

/// Dissolves one assembly level and publishes the next revision, or replays: the assembly is
/// deleted, its direct parts become loose, and the parts themselves do not change.
pub fn explode(
    store: &RevisionStore,
    context: &AssemblyContext,
    command: &ExplodeAssembly,
    hook: &mut dyn CommitHook,
) -> Result<AssemblyChange, PlateError> {
    let fingerprint = command.content_fingerprint()?;
    apply(
        store,
        context,
        fingerprint,
        true,
        &|m| !m.ids.contains(&command.assembly_id),
        &|m| {
            let decision = m.membership.explode(command)?;
            let mut changes: Vec<Change> = decision
                .parts
                .iter()
                .map(|p| Change::PartOf {
                    id: p.to_string(),
                    assembly: None,
                })
                .collect();
            changes.push(Change::Delete {
                id: command.assembly_id.to_string(),
            });
            Ok(Plan {
                changes,
                footprint: decision.footprint,
            })
        },
        hook,
    )
}
