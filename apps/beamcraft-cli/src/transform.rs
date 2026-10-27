// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! T03 group operations: prepare every affected part once, then publish one revision.
use crate::{
    copy::CopiedElement,
    plate::{self, AUTHORED_LAYER, DERIVED_LAYER, PlateError, RevisionSummary},
    publication::{PublishRequest, publish_revision},
};
use beamcraft_engine::{
    footprint::Footprint,
    transform::{RigidDelta, copy_id, validate_ids},
};
use beamcraft_host::ledger::Outcome;
use beamcraft_host::revision::{CommitHook, RevisionStore, Uuid7};
use beamcraft_layer_engine::{
    Reader,
    authored::{
        Author, Change, EditTargets, Element, MemberKind as LayerKind, Placement as LayerPlacement,
        Prepare, Shape, schema_identity,
    },
    index::{ElementId as LayerId, LinkTarget},
};
use beamcraft_model::{
    command::{CopyElement, MemberKind, Placement},
    error::ErrorCode,
    id::{ElementId, OpId, ProjectId, RevisionId, WorkspaceId},
};
/// Typed arguments, shared by the CLI and other headless callers.
#[derive(Debug, Clone)]
pub struct TransformRequest {
    /// Logical operation identity.
    pub op_id: OpId,
    /// Revision to publish.
    pub revision_id: RevisionId,
    /// Actor identity.
    pub actor: Uuid7,
    /// Fallback project for an empty store (which transformations refuse).
    pub project: ProjectId,
    /// Workspace identity.
    pub workspace: WorkspaceId,
    /// Explicit sorted unique owned part IDs.
    pub ids: Vec<ElementId>,
    /// Proper world transform.
    pub delta: RigidDelta,
    /// Create new parts instead of changing selected placements.
    pub copy: bool,
    /// New parts share the source's canonical definition.
    pub instance: bool,
}
/// One atomic group's published result.
#[derive(Debug, Clone)]
pub struct Transformed {
    /// A repeat of an already accepted request.
    pub replayed: bool,
    /// Source and resulting element IDs in selection order.
    pub elements: Vec<(ElementId, ElementId)>,
    /// Published or replayed revision.
    pub revision: RevisionSummary,
}
fn error(code: ErrorCode, message: &'static str) -> PlateError {
    PlateError::new(code, message)
}
fn pose(e: &CopiedElement) -> Placement {
    match e {
        CopiedElement::Plate(p) => p.authored.placement,
        CopiedElement::Member(p) => p.authored.placement,
    }
}
fn set_pose(e: &mut CopiedElement, p: Placement) {
    match e {
        CopiedElement::Plate(r) => r.authored.placement = p,
        CopiedElement::Member(r) => r.authored.placement = p,
    }
}
fn layer_pose(p: Placement) -> LayerPlacement {
    LayerPlacement {
        translation_mm: p.translation_mm,
        orientation_xyzw: p.orientation_xyzw,
    }
}
fn create_changes(
    e: &CopiedElement,
    canonical: Option<ElementId>,
) -> Result<Vec<Change>, PlateError> {
    let (id, name, class, shape, placement, material, mesh) = match e {
        CopiedElement::Plate(r) => {
            let a = &r.authored;
            (
                a.id,
                a.name.clone(),
                a.class,
                Shape::Plate {
                    contour_mm: a.contour_points_mm.clone(),
                    thickness_mm: a.thickness_mm,
                    holes: a
                        .holes_mm
                        .iter()
                        .map(|h| h.iter().map(|[x, y]| [*x, *y, 0.]).collect())
                        .collect(),
                    hole_records: a.hole_records.clone(),
                    cuts: a.cuts.clone(),
                },
                a.placement,
                a.material,
                plate::mesh_change(&a.id, &r.body.solid, &plate::plate_record(r))?,
            )
        }
        CopiedElement::Member(r) => {
            let a = &r.authored;
            (
                a.id,
                a.name.clone(),
                a.class,
                Shape::Member {
                    kind: match a.kind {
                        MemberKind::Beam => LayerKind::Beam,
                        MemberKind::Column => LayerKind::Column,
                    },
                    axis_mm: a.axis_points_mm,
                    profile: a.profile,
                    roll_rad: a.roll_rad,
                    cuts: a.cuts.clone(),
                },
                a.placement,
                a.material,
                plate::mesh_change(&a.id, &r.body.solid, &plate::member_record(r))?,
            )
        }
    };
    let (shape, material) = if let Some(definition) = canonical {
        let kind = match e {
            CopiedElement::Plate(_) => beamcraft_model::instance::PartKind::Plate,
            CopiedElement::Member(r) => match r.authored.kind {
                MemberKind::Beam => beamcraft_model::instance::PartKind::Beam,
                MemberKind::Column => beamcraft_model::instance::PartKind::Column,
            },
        };
        (
            Shape::Instance {
                kind,
                definition: definition.to_string(),
            },
            None,
        )
    } else {
        (shape, material)
    };
    Ok(vec![
        Change::Create(Element {
            id: id.to_string(),
            name,
            shape,
            placement: layer_pose(placement),
            material,
        }),
        Change::PartClass {
            id: id.to_string(),
            class,
        },
        mesh,
    ])
}
/// Apply one group transform with stable retry identities, evidence and a single commit.
pub fn transform_elements(
    store: &RevisionStore,
    request: &TransformRequest,
    hook: &mut dyn CommitHook,
) -> Result<Transformed, PlateError> {
    transform_with_mirror(store, request, None, None, hook)
}
/// Reflect a group as independent copies. The supplied rigid delta must be identity.
pub fn mirror_elements(
    store: &RevisionStore,
    request: &TransformRequest,
    plane: beamcraft_engine::mirror::MirrorPlane,
    hook: &mut dyn CommitHook,
) -> Result<Transformed, PlateError> {
    if !request.copy || request.instance || request.delta != RigidDelta::translation([0.; 3])? {
        return Err(error(
            ErrorCode::InvalidArgument,
            "mirror needs independent copies and an identity rigid delta",
        ));
    }
    transform_with_mirror(store, request, Some(plane), None, hook)
}
/// Copy a group for each admitted array transform; one atomic revision and undo unit.
pub fn array_elements(
    store: &RevisionStore,
    request: &TransformRequest,
    deltas: &[RigidDelta],
    hook: &mut dyn CommitHook,
) -> Result<Transformed, PlateError> {
    if !request.copy || request.delta != RigidDelta::translation([0.; 3])? {
        return Err(error(
            ErrorCode::InvalidArgument,
            "arrays require copying with an identity base delta",
        ));
    }
    if deltas.is_empty()
        || request.ids.len().saturating_mul(deltas.len()) > beamcraft_engine::transform::MAX_GROUP
    {
        return Err(error(
            ErrorCode::LimitExceeded,
            "array produces more than 128 new parts or is empty",
        ));
    }
    transform_with_mirror(store, request, None, Some(deltas), hook)
}
fn transform_with_mirror(
    store: &RevisionStore,
    request: &TransformRequest,
    mirror: Option<beamcraft_engine::mirror::MirrorPlane>,
    array: Option<&[RigidDelta]>,
    hook: &mut dyn CommitHook,
) -> Result<Transformed, PlateError> {
    validate_ids(&request.ids)?;
    if request.instance && !request.copy {
        return Err(error(
            ErrorCode::InvalidArgument,
            "instance mode requires copying",
        ));
    }
    let mut fingerprint =
        request
            .delta
            .fingerprint(&request.ids, request.copy, request.instance)?;
    if let Some(plane) = mirror {
        let mut bytes = b"BeamCraft/T03/MirrorGroup/v1\0".to_vec();
        bytes.extend(fingerprint.as_bytes());
        for v in plane.parameters() {
            bytes.extend(if v == 0.0 {
                0.0_f64.to_le_bytes()
            } else {
                v.to_le_bytes()
            });
        }
        fingerprint = plate::digest_of(&bytes)?;
    }

    if let Some(deltas) = array {
        let mut bytes = b"BeamCraft/T03/ArrayGroup/v1\0".to_vec();
        bytes.extend(fingerprint.as_bytes());
        for delta in deltas {
            bytes.extend(
                delta
                    .fingerprint(&request.ids, true, request.instance)?
                    .as_bytes(),
            );
        }
        fingerprint = plate::digest_of(&bytes)?;
    }
    let deltas = array.map_or_else(|| vec![request.delta], |d| d.to_vec());
    let mut elements = Vec::new();
    for (slot, _) in deltas.iter().enumerate() {
        let slot = u32::try_from(slot)
            .map_err(|_| error(ErrorCode::LimitExceeded, "too many array slots"))?;
        for id in &request.ids {
            elements.push((
                *id,
                if request.copy {
                    copy_id(request.op_id, *id, slot)?
                } else {
                    *id
                },
            ));
        }
    }
    let op = plate::host_uuid(&request.op_id.to_string())?;
    // Read the committed request evidence, not current source state: later source edits do not
    // turn a retry into a second relative transformation.
    match store.query_outcome(&op)? {
        Outcome::Committed { sequence, .. } => {
            let revision = store.open_at(sequence)?;
            let recorded = crate::command_evidence::read(&revision)?.ok_or_else(|| {
                error(
                    ErrorCode::OpIdReuse,
                    "operation has no compatible request evidence",
                )
            })?;
            if recorded.fingerprint != fingerprint {
                return Err(error(
                    ErrorCode::OpIdReuse,
                    "operation identity was accepted with different arguments",
                ));
            }
            return Ok(Transformed {
                replayed: true,
                elements,
                revision: plate::summarize(revision.info()),
            });
        }
        Outcome::Unknown => {
            return Err(error(
                ErrorCode::OutcomeUnknown,
                "operation outcome requires recovery",
            ));
        }
        Outcome::Aborted { .. } | Outcome::NotFound => {}
    }
    let control = plate::control();
    let (head, base) = plate::load_base(store, &request.project, &request.workspace)?;
    if head.is_none() {
        return Err(error(
            ErrorCode::NotFound,
            "transform needs a model revision",
        ));
    }
    let open = plate::open_base(&base)?;
    let mut reader = Reader::default();
    let opened = reader.open(open.clone(), &control)?;
    let decoded = plate::decode(&reader, &opened, &control)?;
    let (plates, members) = plate::evaluate_decoded(&decoded)?;
    let mut records = std::collections::BTreeMap::new();
    for r in plates {
        records.insert(r.authored.id, CopiedElement::Plate(Box::new(r)));
    }
    for r in members {
        records.insert(r.authored.id, CopiedElement::Member(Box::new(r)));
    }
    if request.instance
        && decoded
            .instances
            .links()
            .count()
            .saturating_add(elements.len())
            > beamcraft_model::instance::MAX_PART_INSTANCES
    {
        return Err(error(
            ErrorCode::LimitExceeded,
            "linked instance limit exceeded",
        ));
    }
    let schema = schema_identity()?;
    let (mut author, opened) = Author::open(
        open,
        EditTargets {
            authored: AUTHORED_LAYER.into(),
            derived: DERIVED_LAYER.into(),
        },
        schema,
        &control,
    )?;
    let index = author.element_index(&opened.context, &control)?;
    let mut footprint = Footprint::default();
    let mut changes = Vec::new();
    for (index_in_plan, (source, target)) in elements.iter().enumerate() {
        let source_record = records.get(source).ok_or_else(|| {
            error(
                ErrorCode::UnsupportedProperty,
                "selection contains a missing or unsupported part",
            )
        })?;
        let delta = deltas
            .get(index_in_plan / request.ids.len())
            .ok_or_else(|| error(ErrorCode::InvalidArgument, "array slot is absent"))?;
        let transformed = delta.apply(pose(source_record))?;
        if request.copy {
            let plan = index.copy_plan(
                &LayerId::parse(&source.to_string())?,
                &LayerId::parse(&target.to_string())?,
            )?;
            if plan
                .links
                .iter()
                .any(|l| !matches!(l.target, LinkTarget::Definition(_)) && l.name != "bim:level")
            {
                return Err(error(
                    ErrorCode::DependencyConflict,
                    "copy contains unsupported external relationships",
                ));
            }
            let cmd = CopyElement {
                source_id: *source,
                new_id: *target,
                name: None,
                offset_mm: [0.; 3],
            };
            let mut next = match source_record {
                CopiedElement::Plate(r) => {
                    let mut r = r.clone();
                    r.authored = if request.instance {
                        beamcraft_engine::instance::copy_plate_instance(&r.authored, &cmd)?
                    } else {
                        beamcraft_engine::copy::copy_plate(&r.authored, &cmd)?
                    };
                    CopiedElement::Plate(r)
                }
                CopiedElement::Member(r) => {
                    let mut r = r.clone();
                    r.authored = if request.instance {
                        beamcraft_engine::instance::copy_member_instance(&r.authored, &cmd)?
                    } else {
                        beamcraft_engine::copy::copy_member(&r.authored, &cmd)?
                    };
                    CopiedElement::Member(r)
                }
            };
            set_pose(&mut next, transformed);
            if let Some(plane) = mirror {
                let materials = crate::definition::material_lookup(&decoded.definitions);
                match &mut next {
                    CopiedElement::Plate(r) => {
                        r.authored = plane.plate(&r.authored)?;
                        r.body = beamcraft_engine::command::evaluate_plate(&r.authored)?;
                        r.quantities = r.body.quantities_for(
                            r.authored.material.as_ref().map(&materials).transpose()?,
                        )?;
                    }
                    CopiedElement::Member(r) => {
                        r.authored = plane.member(&r.authored)?;
                        let profiles = crate::definition::profile_lookup(&decoded.definitions);
                        r.body = beamcraft_engine::member::evaluate_member(
                            &r.authored,
                            profiles(&r.authored.profile)?,
                        )?;
                        r.quantities = beamcraft_engine::member::member_quantities(
                            &r.body,
                            r.authored.material.as_ref().map(&materials).transpose()?,
                        )?;
                    }
                }
            }
            let (profile, material) = match &next {
                CopiedElement::Plate(r) => (None, r.authored.material),
                CopiedElement::Member(r) => (Some(r.authored.profile), r.authored.material),
            };
            footprint =
                footprint.union(Footprint::for_copy(*source, *target, profile, material)?)?;
            let canonical = decoded.instances.definition(*source)?;
            if canonical != *source {
                footprint =
                    footprint.union(Footprint::for_copy(canonical, *target, profile, material)?)?;
            }
            // Preserve links inside an independent copied group, isolated from the original.
            // A linked copy explicitly retains the existing canonical definition instead.
            let definition = if request.instance {
                Some(canonical)
            } else if canonical != *source && request.ids.binary_search(&canonical).is_ok() {
                let slot = u32::try_from(index_in_plan / request.ids.len())
                    .map_err(|_| error(ErrorCode::LimitExceeded, "too many array slots"))?;
                let copied_owner = copy_id(request.op_id, canonical, slot)?;
                footprint = footprint.union(Footprint::for_copy(
                    copied_owner,
                    *target,
                    profile,
                    material,
                )?)?;
                Some(copied_owner)
            } else {
                None
            };
            changes.extend(create_changes(&next, definition)?);
            if let Some(level) = decoded.levels.level_of(source) {
                footprint = footprint.union(Footprint::for_level(*target, Some(level))?)?;
                changes.push(Change::Level {
                    id: target.to_string(),
                    level: Some(level.to_string()),
                });
            }
        } else {
            if pose(source_record) == transformed {
                return Err(error(
                    ErrorCode::InvalidArgument,
                    "transform changes no placement",
                ));
            }
            footprint = footprint.union(Footprint::for_update(
                *source, false, false, false, true, None,
            )?)?;
            changes.push(Change::Placement {
                id: source.to_string(),
                placement: layer_pose(transformed),
            });
        }
    }
    let prepared = author.prepare(
        Prepare {
            context: opened.context.clone(),
            op_id: request.op_id.to_string(),
            actor: request.actor.to_string(),
            schema,
            resources: Vec::new(),
            changes,
        },
        &control,
    )?;
    plate::check_footprint(&prepared, &footprint)?;
    let committed = publish_revision(
        store,
        &PublishRequest {
            footprint: &footprint,
            fingerprint,
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
    Ok(Transformed {
        replayed: false,
        elements,
        revision: plate::summarize(&committed.info),
    })
}
