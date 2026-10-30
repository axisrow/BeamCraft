// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A11 lofted plate commands: create one through the same atomic store/verifier as the
//! evaluated plates, and read the committed lofted plates of the verified head back.

use crate::definition::material_lookup;
use crate::plate::{
    AUTHORED_LAYER, Base, DERIVED_LAYER, DerivedMesh, PlateError, check_footprint, control, decode,
    digest_of, host_uuid, integrity, load_base, mesh_change, open_base, prior_op_with,
    replay_summary, summarize,
};
use crate::publication::{PublishRequest, publish_revision};
use beamcraft_engine::footprint::Footprint;
use beamcraft_engine::loft_plate::{
    AuthoredLoftedPlate, LOFT_VOLUME_ALGORITHM, create_lofted_plate,
};
use beamcraft_engine::quantity::{StoredRecord, lofted_plate_record};
use beamcraft_engine::transaction::PriorOp;
use beamcraft_host::revision::{CommitHook, CommitOutcome, RevisionError, RevisionStore, Uuid7};
use beamcraft_layer_engine::Reader;
use beamcraft_layer_engine::authored::{
    Author, Change, EditTargets, Element, Placement as LayerPlacement, Prepare, Shape,
    schema_identity,
};
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId, ProjectId, RevisionId, WorkspaceId};

/// Inputs of one `create-lofted-plate` call beyond the command itself.
#[derive(Debug, Clone)]
pub struct LoftRequest {
    /// The logical operation. A retry with the same id and identical content is a replay.
    pub op_id: OpId,
    /// Identity of the revision this call creates. Only used when the call creates one.
    pub revision_id: RevisionId,
    /// The creating actor (trusted context, `bim:createdBy`).
    pub actor: Uuid7,
    /// Project of a store that has no revision yet. Ignored otherwise: the store's wins.
    pub project: ProjectId,
    /// Workspace of a store that has no revision yet. Ignored otherwise.
    pub workspace: WorkspaceId,
    /// The command.
    pub command: beamcraft_model::command::CreateLoftedPlate,
}

/// Result of `create_lofted_plate`.
#[derive(Debug, Clone)]
pub struct CreatedLoft {
    /// True when the same operation was already committed and nothing was written.
    pub replayed: bool,
    /// The element the command created (or created earlier).
    pub plate: AuthoredLoftedPlate,
    /// Volume of the certified body in cubic millimetres.
    pub volume_mm3: f64,
    /// Certified surface area in square millimetres, when the kernel measured it.
    pub area_mm2: Option<f64>,
    /// SHA-256 of the sealed mesh blob.
    pub mesh_digest: String,
    /// Triangles of the certified mesh.
    pub triangles: usize,
    /// The revision that holds it.
    pub revision: crate::plate::RevisionSummary,
    /// The footprint the engine declared; `None` for a replay, which applies nothing.
    pub footprint: Option<Footprint>,
}

/// Creates a lofted plate and publishes the next revision, or replays the operation.
///
/// Steps, each failing closed: open and verify the head; decide replay from the host ledger
/// and the head state; admit the command with the engine (which evaluates the kernel loft,
/// so a twisted or self-intersecting input refuses before anything is written); author with
/// U02; check the layer engine's writes against the declared footprint; commit through
/// U06/U07 with the strict verifier.
pub fn create_lofted(
    store: &RevisionStore,
    request: &LoftRequest,
    hook: &mut dyn CommitHook,
) -> Result<CreatedLoft, PlateError> {
    request.command.validate_arguments()?;
    let control = control();
    let (head, base) = load_base(store, &request.project, &request.workspace)?;
    let open_request = open_base(&base)?;
    let mut reader = Reader::default();
    let opened = reader.open(open_request.clone(), &control)?;
    let decoded = decode(&reader, &opened, &control)?;
    let op = host_uuid(&request.op_id.to_string())?;
    let (prior, recorded) = prior_op_with(store, &op, || {
        match decoded
            .lofted
            .iter()
            .find(|(p, _)| p.id == request.command.new_id)
        {
            Some((plate, _)) => Ok(plate.to_command().content_fingerprint()?),
            None => digest_of(b"no accepted request with this element id"),
        }
    })?;
    let Base {
        project,
        workspace,
        generation: base_generation,
        expected,
        ..
    } = base;
    if let PriorOp::Accepted { fingerprint } = prior {
        return if request.command.content_fingerprint()? == fingerprint {
            let authored = decoded
                .lofted
                .iter()
                .find(|(p, _)| p.id == request.command.new_id)
                .map(|(p, _)| p)
                .ok_or_else(|| integrity("replayed element is absent from the head revision"))?;
            let quantities = authored_quantities(&decoded, &authored.id)?;
            Ok(CreatedLoft {
                replayed: true,
                plate: authored.clone(),
                volume_mm3: quantities.as_ref().map_or(0.0, |q| q.volume_mm3),
                area_mm2: None,
                mesh_digest: mesh_shape_key(&decoded, &authored.id)?,
                triangles: mesh_triangles(&decoded, &authored.id)?,
                revision: replay_summary(&head, recorded)?,
                footprint: None,
            })
        } else {
            Err(CommandError::new(
                ErrorCode::OpIdReuse,
                "op_id",
                "operation id was accepted with different content",
            )
            .into())
        };
    }
    let outcome = create_lofted_plate(
        &request.command,
        &|id| {
            decoded.elements.contains_key(id)
                || decoded.levels.levels().any(|(level, _)| level == id)
        },
        &material_lookup(&decoded.definitions),
    )?;
    let record = lofted_plate_record(
        &outcome.authored,
        LOFT_VOLUME_ALGORITHM,
        &outcome.quantities,
    );
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
    let placement = outcome.authored.placement;
    let prepared = author.prepare(
        Prepare {
            context: opened.context.clone(),
            op_id: request.op_id.to_string(),
            actor: request.actor.to_string(),
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
                    shape: Shape::LoftedPlate {
                        start_mm: outcome.authored.start_points_mm.clone(),
                        end_mm: outcome.authored.end_points_mm.clone(),
                    },
                    placement: LayerPlacement {
                        translation_mm: placement.translation_mm,
                        orientation_xyzw: placement.orientation_xyzw,
                    },
                    material: outcome.authored.material,
                }),
                mesh_change(&outcome.authored.id, &outcome.body.solid, &record)?,
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
            op,
            base_generation,
            expected: &expected,
        },
        hook,
    )?;
    Ok(CreatedLoft {
        replayed: committed.outcome == CommitOutcome::Replayed,
        plate: outcome.authored,
        volume_mm3: outcome.body.volume_mm3,
        area_mm2: outcome.body.area_mm2,
        mesh_digest: outcome.body.solid.digest().to_string(),
        triangles: outcome.body.solid.triangles().len(),
        revision: summarize(&committed.info),
        footprint: Some(outcome.footprint),
    })
}

/// One committed lofted plate of the verified head, with its stored quantity record.
#[derive(Debug, Clone)]
pub struct LoftedView {
    /// Full element identity.
    pub id: ElementId,
    /// Label.
    pub name: String,
    /// Authored part class.
    pub class: i32,
    /// Stored volume in cubic millimetres, when the derived layer authors one.
    pub volume_mm3: Option<f64>,
    /// Stored volume status token.
    pub volume_status: Option<String>,
    /// Stored mass in kg, when the material authors a density and the record is stored.
    pub mass_kg: Option<f64>,
    /// Stored mass status token.
    pub mass_status: Option<String>,
    /// Stored algorithm label.
    pub algorithm: Option<String>,
    /// Triangles of the stored mesh.
    pub triangles: usize,
}

/// The committed lofted plates of the verified head (empty for a store without a revision).
pub fn lofted_plates(store: &RevisionStore) -> Result<Vec<LoftedView>, PlateError> {
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
    let mut reader = Reader::default();
    let opened = reader.open(open_base(&base)?, &control)?;
    let decoded = decode(&reader, &opened, &control)?;
    let mut views = decoded
        .lofted
        .iter()
        .map(|(authored, mesh)| {
            let stored = mesh.as_ref().and_then(|m| m.stored.clone());
            Ok(LoftedView {
                id: authored.id,
                name: authored.name.clone(),
                class: authored.class,
                volume_mm3: stored.as_ref().map(|q| q.volume_mm3),
                volume_status: stored.as_ref().map(|q| q.volume_status.clone()),
                mass_kg: stored.as_ref().and_then(|q| q.mass_kg),
                mass_status: stored.as_ref().map(|q| q.mass_status.clone()),
                algorithm: stored.as_ref().map(|q| q.algorithm.clone()),
                triangles: mesh_triangles(&decoded, &authored.id)?,
            })
        })
        .collect::<Result<Vec<_>, PlateError>>()?;
    views.sort_by_key(|v| v.id);
    Ok(views)
}

/// The stored quantity record of one decoded element's mesh.
fn authored_quantities(
    decoded: &crate::plate::Decoded,
    id: &ElementId,
) -> Result<Option<StoredRecord>, PlateError> {
    Ok(decoded
        .lofted
        .iter()
        .find(|(p, _)| &p.id == id)
        .and_then(|(_, mesh)| mesh.as_ref())
        .and_then(|mesh| mesh.stored.clone()))
}

/// The stored triangle count of one decoded element's mesh; zero when none is stored.
fn mesh_triangles(decoded: &crate::plate::Decoded, id: &ElementId) -> Result<usize, PlateError> {
    Ok(decoded
        .lofted
        .iter()
        .find(|(p, _)| &p.id == id)
        .and_then(|(_, mesh)| mesh.as_ref())
        .map(|mesh: &DerivedMesh| mesh.counts.len())
        .unwrap_or(0))
}

/// The stored shape key (the mesh digest at publish) of one decoded element.
fn mesh_shape_key(decoded: &crate::plate::Decoded, id: &ElementId) -> Result<String, PlateError> {
    Ok(decoded
        .lofted
        .iter()
        .find(|(p, _)| &p.id == id)
        .and_then(|(_, mesh)| mesh.as_ref())
        .map(|mesh: &DerivedMesh| mesh.shape_key.clone())
        .unwrap_or_default())
}
