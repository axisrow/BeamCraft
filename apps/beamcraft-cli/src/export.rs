// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! E02: IFC4 export of one committed revision (contract `docs/contracts/e01-ifc-export-v0.1.md`).
//!
//! The adapter opens a verified revision through [`inspect_revision`] (so the kernel has already
//! re-evaluated every plate, beam and column and compared its mesh with the stored one,
//! `IFCX-GEO-005`), resolves everything into a [`beamcraft_ifc::ExportModel`], and hands it to the
//! pure writer. It changes nothing in the store (`IFCX-SCOPE-004`). The file is published through
//! [`beamcraft_host::output::publish_file`], so a failed export leaves no file.
//!
//! The report lists every element with its GlobalId, representation, quantity freshness and
//! omissions, plus the findings of the contract in its deterministic order.

use crate::plate::{
    ElementMeta, Inspection, PlacementRead, PlateError, inspect_revision, kernel_mesh,
};
use beamcraft_engine::quantity::Freshness;
use beamcraft_host::output::{Installed, OutputError, Staged, resolve_destination, stage_file};
use beamcraft_host::revision::{Revision, RevisionError, RevisionStore};
use beamcraft_ifc::{
    Declared, ElementData, Entity, ExportModel, LevelData, Limits, MaterialData, Mesh, Pose,
    QuantityData, Subject, WriteError, global_id, guid, step::SinkError, storey_global_id,
    write_ifc,
};
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, Uuid7};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::definition::DefinitionRecord;

/// The only contract version.
pub const CONTRACT_VERSION: &str = "0.1.0";
/// The only schema identifier (IFC4 ADD2 TC1).
pub const SCHEMA: &str = "IFC4";
/// The only output format.
pub const FORMAT: &str = "step";
/// Findings listed in a report (`IFCX-LIM-004`).
pub const MAX_LISTED_FINDINGS: usize = 256;
/// Findings counted at most (F10 section 6.3).
const MAX_COUNTED_FINDINGS: usize = 1_000_000;

fn error(code: ErrorCode, message: &'static str) -> PlateError {
    PlateError::new(code, message)
}

/// The options of one export (E01 section 15.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRequest {
    /// Exactly `0.1.0`.
    pub contract_version: String,
    /// Exactly `IFC4`.
    pub schema: String,
    /// Exactly `step`.
    pub format: String,
    /// An explicit revision; the published head when `None`.
    pub revision: Option<Uuid7>,
    /// The optional `IfcProject.Name`.
    pub project_name: Option<String>,
    /// Refuse when a plate, beam or column would leave without its mesh or placement.
    pub strict: bool,
    /// Replace an existing output file.
    pub overwrite: bool,
}

impl Default for ExportRequest {
    fn default() -> Self {
        Self {
            contract_version: CONTRACT_VERSION.into(),
            schema: SCHEMA.into(),
            format: FORMAT.into(),
            revision: None,
            project_name: None,
            strict: false,
            overwrite: false,
        }
    }
}

/// Severity of a finding, in the order of F10 section 6.4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Refuses the export.
    Error,
    /// A loss the export carries.
    Warning,
    /// Information.
    Info,
}

impl Severity {
    /// The report token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Info => "info",
        }
    }
}

/// One finding of the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// The `IFC_` token.
    pub code: &'static str,
    /// The F04 code, only for a finding that is fatal in strict mode.
    pub f04_code: Option<ErrorCode>,
    /// Severity.
    pub severity: Severity,
    /// The element, when the finding belongs to one.
    pub element_id: Option<ElementId>,
    /// A fixed schema field token.
    pub field: &'static str,
    /// Fixed English text that never echoes input.
    pub message: &'static str,
}

/// One element of the revision in the report.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementReport {
    /// The ElementId.
    pub element_id: ElementId,
    /// The 22-character GlobalId, `None` when the element was not exported.
    pub ifc_global_id: Option<String>,
    /// The element-kind token.
    pub kind: String,
    /// Whether it is in the file.
    pub exported: bool,
    /// `IfcBeam`, `IfcColumn`, `IfcPlate` or `IfcBuildingElementProxy`.
    pub entity: Option<&'static str>,
    /// `Tessellation` or `None` (no representation).
    pub representation: Option<&'static str>,
    /// Volume freshness token (plate, beam, column).
    pub volume: Option<&'static str>,
    /// Mass freshness token (plate, beam, column).
    pub mass: Option<&'static str>,
    /// The quantities left out of the file.
    pub omissions: Vec<&'static str>,
}

/// Counts of the report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// Elements of the revision (levels excluded).
    pub elements_total: usize,
    /// Elements in the file.
    pub exported: usize,
    /// Elements not in the file.
    pub not_exported: usize,
    /// Elements written as proxies.
    pub proxies: usize,
    /// Exported elements without a representation.
    pub without_representation: usize,
}

/// The written file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputInfo {
    /// Bytes.
    pub bytes: u64,
    /// SHA-256 of the file.
    pub sha256: ContentDigest,
}

/// `ok` or `ok_with_losses`; a failed export is an error, not a report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Nothing raised and every element exported.
    Ok,
    /// A warning was raised or an element was not exported.
    OkWithLosses,
}

impl Status {
    /// The report token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::OkWithLosses => "ok_with_losses",
        }
    }
}

/// The deterministic report of one export (E01 section 15.4).
#[derive(Debug, Clone, PartialEq)]
pub struct ExportReport {
    /// The ProjectId stored in the revision.
    pub project_id: Uuid7,
    /// The exported revision.
    pub revision_id: Uuid7,
    /// `ok` or `ok_with_losses`.
    pub status: Status,
    /// The listed findings, at most [`MAX_LISTED_FINDINGS`], in the F10 order.
    pub findings: Vec<Finding>,
    /// All findings, counted up to one million.
    pub total_findings: usize,
    /// Whether `findings` is shorter than `total_findings`.
    pub truncated: bool,
    /// Counts.
    pub counts: Counts,
    /// LevelId and GlobalId of every level.
    pub levels: Vec<(Uuid7, String)>,
    /// Every element of the revision, in ElementId order.
    pub elements: Vec<ElementReport>,
    /// The written file.
    pub output: Option<OutputInfo>,
}

/// Whether the request is one this contract version serves (`IFCX-VER-001`, `-002`).
pub fn validate_request(request: &ExportRequest) -> Result<(), PlateError> {
    if request.contract_version != CONTRACT_VERSION || request.schema != SCHEMA {
        return Err(error(
            ErrorCode::UnsupportedVersion,
            "only contract version 0.1.0 and schema IFC4 are supported",
        ));
    }
    if request.format != FORMAT {
        return Err(error(
            ErrorCode::UnsupportedProperty,
            "only the clear-text STEP format is produced",
        ));
    }
    if let Some(name) = &request.project_name
        && !beamcraft_model::command::label_is_valid(name)
    {
        return Err(error(
            ErrorCode::InvalidArgument,
            "the project name is not a valid label",
        ));
    }
    Ok(())
}

/// The `IFC_` token a refusal carries (section 13).
pub fn ifc_code_of(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::UnsupportedVersion => "IFC_VERSION_UNSUPPORTED",
        ErrorCode::UnsupportedProperty => "IFC_FORMAT_UNSUPPORTED",
        ErrorCode::UnsupportedCommand => "IFC_IMPORT_UNSUPPORTED",
        ErrorCode::InvalidId => "IFC_ID_INVALID",
        ErrorCode::NotFound => "IFC_NOT_FOUND",
        ErrorCode::DependencyUnavailable => "IFC_INPUT_UNAVAILABLE",
        ErrorCode::DanglingRelationship => "IFC_LEVEL_DANGLING",
        ErrorCode::DuplicateId => "IFC_ID_DUPLICATE",
        ErrorCode::IntegrityError => "IFC_INTEGRITY",
        ErrorCode::LimitExceeded => "IFC_LIMIT",
        ErrorCode::BackendUnavailable => "IFC_OUTPUT_FAILED",
        ErrorCode::Cancelled => "IFC_CANCELLED",
        ErrorCode::UnsupportedGeometry => "IFC_GEOMETRY_ABSENT",
        _ => "IFC_ARGUMENT_INVALID",
    }
}

fn open_revision(store: &RevisionStore, wanted: Option<Uuid7>) -> Result<Revision, PlateError> {
    let Some(wanted) = wanted else {
        return store.open().map_err(|e| match e {
            RevisionError::NoRevision => error(
                ErrorCode::DependencyUnavailable,
                "the store has no revision to export",
            ),
            other => PlateError::from(other),
        });
    };
    let sequence = store
        .sequence_of(
            &beamcraft_host::revision::Uuid7::parse(&wanted.to_string())
                .map_err(|_| error(ErrorCode::InvalidId, "the revision id is not a UUIDv7"))?,
        )?
        .ok_or_else(|| error(ErrorCode::NotFound, "no committed revision has that id"))?;
    Ok(store.open_at(sequence)?)
}

fn revision_id(revision: &Revision) -> Result<Uuid7, PlateError> {
    Uuid7::parse(revision.info().revision().as_str())
        .map_err(|_| error(ErrorCode::InvalidId, "the revision id is not a UUIDv7"))
}

fn mesh_of(record: &beamcraft_engine::solid::SolidMesh) -> Result<Mesh, PlateError> {
    let bad = || error(ErrorCode::IntegrityError, "a mesh index is out of range");
    let derived = kernel_mesh(record)?;
    let index = |v: i64| u32::try_from(v).map_err(|_| bad());
    let triangles = derived
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|&[a, b, c]| Ok([index(a)?, index(b)?, index(c)?]))
        .collect::<Result<Vec<_>, PlateError>>()?;
    Ok(Mesh {
        points: derived.points,
        triangles,
        closed: true,
    })
}

fn freshness(stored: bool, assessment: Freshness, expected_status: &'static str) -> &'static str {
    if !stored {
        "missing"
    } else if assessment != Freshness::Current {
        "stale"
    } else {
        expected_status
    }
}

struct Built {
    model: ExportModel,
    report_elements: Vec<ElementReport>,
    findings: Vec<Finding>,
    levels: Vec<(Uuid7, String)>,
}

/// Everything about one plate, beam or column the model needs, resolved from the inspection.
struct Body<'a> {
    entity: Entity,
    mesh: Result<Mesh, PlateError>,
    placement: beamcraft_model::command::Placement,
    material: Option<ContentDigest>,
    profile: Option<ContentDigest>,
    holes: Option<u32>,
    quantities: &'a beamcraft_engine::command::Quantities,
}

fn bodies(inspection: &Inspection) -> BTreeMap<ElementId, Body<'_>> {
    let mut out = BTreeMap::new();
    for r in &inspection.plates {
        out.insert(
            r.authored.id,
            Body {
                entity: Entity::Plate,
                mesh: mesh_of(&r.body.solid),
                placement: r.authored.placement,
                material: r.authored.material,
                profile: None,
                holes: u32::try_from(r.authored.holes_mm.len()).ok(),
                quantities: &r.quantities,
            },
        );
    }
    for r in &inspection.members {
        let placement = r.authored.placement;
        out.insert(
            r.authored.id,
            Body {
                entity: match r.authored.kind {
                    beamcraft_model::command::MemberKind::Beam => Entity::Beam,
                    beamcraft_model::command::MemberKind::Column => Entity::Column,
                },
                mesh: mesh_of(&r.body.solid),
                placement,
                material: r.authored.material,
                profile: Some(r.authored.profile),
                holes: None,
                quantities: &r.quantities,
            },
        );
    }
    out
}

fn declared_of(definition: &DefinitionRecord) -> Option<Declared> {
    let section = match definition {
        DefinitionRecord::RectangleProfile { section, .. }
        | DefinitionRecord::SectionProfile { section, .. } => section,
        DefinitionRecord::Material { .. } | DefinitionRecord::Fastener { .. } => return None,
    };
    let declared = Declared {
        designation: section.designation.clone(),
        standard: section.standard.clone(),
        area_mm2: section.area_mm2,
        mass_per_meter_kg: section.mass_per_meter_kg,
        surface_per_meter_m2: section.surface_per_meter_m2,
    };
    (!declared.is_empty()).then_some(declared)
}

fn build(
    inspection: &Inspection,
    request: &ExportRequest,
    revision: &Revision,
) -> Result<Built, PlateError> {
    let project_text = inspection.project_record.as_deref().ok_or_else(|| {
        error(
            ErrorCode::DependencyUnavailable,
            "the revision records no ProjectId",
        )
    })?;
    let project_id = Uuid7::parse(project_text)
        .map_err(|_| error(ErrorCode::InvalidId, "the stored ProjectId is not a UUIDv7"))?;
    let levels: Vec<LevelData> = inspection
        .levels
        .levels()
        .map(|(id, record)| LevelData {
            id: id.uuid(),
            name: record.name.clone(),
            elevation_mm: record.elevation_mm,
        })
        .collect();
    let level_ids: BTreeSet<Uuid7> = levels.iter().map(|l| l.id).collect();
    let bodies = bodies(inspection);
    let mut elements = Vec::new();
    let mut report_elements = Vec::new();
    let mut findings = Vec::new();
    let mut used_materials = BTreeSet::new();
    let find = |code, f04, severity, id, field, message| Finding {
        code,
        f04_code: f04,
        severity,
        element_id: Some(id),
        field,
        message,
    };
    for (id, meta) in &inspection.elements {
        if meta.kind == "grid" {
            continue;
        }
        let mut row = ElementReport {
            element_id: *id,
            ifc_global_id: None,
            kind: meta.kind.clone(),
            exported: false,
            entity: None,
            representation: None,
            volume: None,
            mass: None,
            omissions: Vec::new(),
        };
        if meta.deactivated {
            findings.push(find(
                "IFC_ELEMENT_DEACTIVATED",
                None,
                Severity::Info,
                *id,
                "lifecycle",
                "a deactivated element is not exported",
            ));
            report_elements.push(row);
            continue;
        }
        let level = inspection.levels.level_of(id);
        if let Some(level) = level
            && !level_ids.contains(&level.uuid())
        {
            return Err(error(
                ErrorCode::DanglingRelationship,
                "an element is assigned to a level the revision does not carry",
            ));
        }
        let data = match bodies.get(id) {
            Some(body) => {
                body_element(inspection, *id, meta, body, level, &mut row, &mut findings)?
            }
            None => proxy_element(*id, meta, level, &mut row, &mut findings),
        };
        if let Some(material) = data.material {
            used_materials.insert(material);
        }
        row.exported = true;
        row.ifc_global_id = Some(global_id(&data.id).as_str().to_owned());
        row.entity = Some(data.entity.ifc_name());
        row.representation = data.mesh.as_ref().map(|_| "Tessellation");
        elements.push(data);
        report_elements.push(row);
    }
    let mut materials = Vec::new();
    for digest in used_materials {
        let Some(DefinitionRecord::Material {
            density_kg_m3,
            grade,
            standard,
            ..
        }) = inspection.definitions.get(&digest)
        else {
            return Err(error(
                ErrorCode::NotFound,
                "a pinned material does not resolve to a definition",
            ));
        };
        materials.push(MaterialData {
            digest,
            grade: grade.clone(),
            standard: standard.clone(),
            density_kg_m3: *density_kg_m3,
        });
    }
    let level_ids = levels
        .iter()
        .map(|l| {
            (
                l.id,
                storey_global_id(&project_id, &l.id).as_str().to_owned(),
            )
        })
        .collect();
    Ok(Built {
        model: ExportModel {
            project_id,
            project_name: request.project_name.clone(),
            revision_unix_ms: revision_id(revision)?.unix_ms(),
            levels,
            materials,
            elements,
        },
        report_elements,
        findings,
        levels: level_ids,
    })
}

fn names(meta: &ElementMeta) -> (String, String, String) {
    (
        meta.name.clone().unwrap_or_default(),
        meta.schema_version
            .clone()
            .unwrap_or_else(|| "unknown".into()),
        meta.geometry_mode
            .clone()
            .unwrap_or_else(|| "unknown".into()),
    )
}

fn body_element(
    inspection: &Inspection,
    id: ElementId,
    meta: &ElementMeta,
    body: &Body<'_>,
    level: Option<ElementId>,
    row: &mut ElementReport,
    findings: &mut Vec<Finding>,
) -> Result<ElementData, PlateError> {
    let pose = Pose::from_quaternion(
        body.placement.translation_mm,
        body.placement.orientation_xyzw,
    )
    .ok_or_else(|| {
        error(
            ErrorCode::InvalidArgument,
            "a stored placement is not a rigid pose",
        )
    })?;
    let mesh = match &body.mesh {
        Ok(mesh) => Some(mesh.clone()),
        Err(_) => {
            findings.push(Finding {
                code: "IFC_GEOMETRY_ABSENT",
                f04_code: Some(ErrorCode::UnsupportedGeometry),
                severity: Severity::Warning,
                element_id: Some(id),
                field: "mesh",
                message: "the element has no usable mesh and leaves without a representation",
            });
            None
        }
    };
    let report = inspection.quantities.get(&id).ok_or_else(|| {
        error(
            ErrorCode::IntegrityError,
            "an element has no quantity assessment",
        )
    })?;
    let stored = report.stored.is_some();
    let volume = freshness(
        stored,
        report.assessment.volume,
        report.expected.volume_status,
    );
    let mass = freshness(stored, report.assessment.mass, report.expected.mass_status);
    row.volume = Some(volume);
    row.mass = Some(mass);
    let mut note = |name: &'static str| {
        row.omissions.push(name);
        findings.push(Finding {
            code: "IFC_QUANTITY_OMITTED",
            f04_code: None,
            severity: Severity::Warning,
            element_id: Some(id),
            field: name,
            message: "a quantity is not current and is left out of the file",
        });
    };
    let net_volume = if volume == "current" {
        let v = body.quantities.volume_m3;
        if !v.is_finite() || v <= 0.0 {
            return Err(error(
                ErrorCode::IntegrityError,
                "a current volume is not finite and positive",
            ));
        }
        Some(v)
    } else {
        note("volume");
        None
    };
    let net_weight = match (mass, body.quantities.mass_kg) {
        ("current", Some(kg)) if kg.is_finite() => Some(kg),
        ("current", _) => {
            return Err(error(
                ErrorCode::IntegrityError,
                "a current mass is not finite",
            ));
        }
        _ => {
            note("mass");
            None
        }
    };
    // Gross quantities only for an element without operations (IFCX-QTY-004).
    let without_operations = body.holes.unwrap_or(0) == 0;
    let quantities = QuantityData {
        volume_status: volume.into(),
        mass_status: mass.into(),
        shape_signature: report.stored.as_ref().map(|s| s.shape_signature),
        mass_signature: report.stored.as_ref().map(|s| s.mass_signature),
        algorithm: report.stored.as_ref().map(|s| s.algorithm.clone()),
        net_volume_m3: net_volume,
        gross_volume_m3: net_volume.filter(|_| without_operations),
        net_weight_kg: net_weight,
        gross_weight_kg: net_weight.filter(|_| without_operations),
    };
    let declared = match body.profile {
        Some(digest) => Some(
            inspection
                .definitions
                .get(&digest)
                .map(declared_of)
                .ok_or_else(|| error(ErrorCode::NotFound, "a pinned profile does not resolve"))?,
        )
        .flatten(),
        None => None,
    };
    let (name, schema_version, geometry_mode) = names(meta);
    Ok(ElementData {
        id: id.uuid(),
        entity: body.entity,
        kind_token: meta.kind.clone(),
        name,
        schema_version,
        geometry_mode,
        pose: Some(pose),
        mesh,
        level: level.map(|l| l.uuid()),
        material: body.material,
        profile: body.profile,
        hole_count: body.holes,
        quantities: Some(quantities),
        declared,
        assembly: meta.part_of.map(|a| a.uuid()),
    })
}

fn proxy_element(
    id: ElementId,
    meta: &ElementMeta,
    level: Option<ElementId>,
    row: &mut ElementReport,
    findings: &mut Vec<Finding>,
) -> ElementData {
    findings.push(Finding {
        code: "IFC_KIND_PROXY",
        f04_code: None,
        severity: Severity::Warning,
        element_id: Some(id),
        field: "kind",
        message: "the kind has no IFC mapping and is exported as a proxy",
    });
    row.omissions.clear();
    let pose = match meta.placement {
        PlacementRead::Absent => None,
        PlacementRead::Rigid(p) => {
            let usable = p
                .validate()
                .ok()
                .and_then(|()| Pose::from_quaternion(p.translation_mm, p.orientation_xyzw));
            if usable.is_none() {
                findings.push(placement_absent(id));
            }
            usable
        }
        PlacementRead::Other => {
            findings.push(placement_absent(id));
            None
        }
    };
    // The published mesh of an unmapped kind is exported as it is (`Closed` unset, never
    // re-verified: the kernel does not know the kind), but only with a pose to place it by.
    let mesh = match (&meta.mesh, &pose) {
        (Some(published), Some(_)) => Some(Mesh {
            points: published.points.clone(),
            triangles: published.triangles.clone(),
            closed: false,
        }),
        (Some(_), None) => {
            if matches!(meta.placement, PlacementRead::Absent) {
                findings.push(placement_absent(id));
            }
            None
        }
        (None, _) => None,
    };
    let (name, schema_version, geometry_mode) = names(meta);
    ElementData {
        id: id.uuid(),
        entity: Entity::Proxy,
        kind_token: meta.kind.clone(),
        name,
        schema_version,
        geometry_mode,
        pose,
        mesh,
        level: level.map(|l| l.uuid()),
        material: None,
        profile: None,
        hole_count: None,
        quantities: None,
        declared: None,
        assembly: meta.part_of.map(|a| a.uuid()),
    }
}

fn placement_absent(id: ElementId) -> Finding {
    Finding {
        code: "IFC_PLACEMENT_ABSENT",
        f04_code: Some(ErrorCode::UnsupportedGeometry),
        severity: Severity::Warning,
        element_id: Some(id),
        field: "placement",
        message: "the placement is not a rigid pose and is not exported",
    }
}

fn rank(f: &Finding) -> (Severity, &'static str, String, &'static str) {
    (
        f.severity,
        f.code,
        f.element_id.map(|e| e.to_string()).unwrap_or_default(),
        f.field,
    )
}

/// The first finding, in finding order, that is fatal in strict mode: only an element-level
/// finding that carries an F04 code (`IFCX-ERR-005`).
fn first_fatal(findings: &[Finding]) -> Option<&Finding> {
    findings
        .iter()
        .filter(|f| f.f04_code.is_some())
        .min_by_key(|f| rank(f))
}

fn write_error(error_value: WriteError) -> PlateError {
    match error_value {
        WriteError::Invalid(m) => error(ErrorCode::InvalidArgument, m),
        WriteError::Limit(m) => error(ErrorCode::LimitExceeded, m),
        WriteError::Integrity(m) => error(ErrorCode::IntegrityError, m),
        WriteError::Sink(SinkError::CapExceeded) => error(
            ErrorCode::LimitExceeded,
            "the output would exceed the size bound",
        ),
        WriteError::Sink(SinkError::Io(_)) => error(
            ErrorCode::BackendUnavailable,
            "the output could not be written",
        ),
    }
}

/// Exports one committed revision of `store` to the file `out` and returns the report.
///
/// Nothing is written to the store. A refusal returns the error and leaves no file.
pub fn export_ifc(
    store: &RevisionStore,
    request: &ExportRequest,
    out: &Path,
) -> Result<ExportReport, PlateError> {
    export_with_limits(store, request, out, &Limits::DEFAULT)
}

/// [`export_ifc`] with explicit bounds (tests use small ones).
pub fn export_with_limits(
    store: &RevisionStore,
    request: &ExportRequest,
    out: &Path,
    limits: &Limits,
) -> Result<ExportReport, PlateError> {
    let prepared = prepare_export_with_limits(store, request, out, limits)?;
    let report = prepared.report.clone();
    prepared.install()?;
    Ok(report)
}

/// An export whose file is complete and synced but not yet installed under its name, with its
/// report. Dropping it leaves no file; [`PreparedExport::install`] publishes it. Staging first
/// lets a caller stage further outputs (the report file) and install only when all succeeded.
pub struct PreparedExport {
    /// The report of the export.
    pub report: ExportReport,
    staged: Staged,
}

impl PreparedExport {
    /// Installs the file under its final name.
    pub fn install(self) -> Result<Installed, PlateError> {
        self.staged.install().map_err(|e| error(e.code, e.message))
    }
}

/// Refuses a destination that lies inside the source store (a file there could shadow or break
/// the store, and `--overwrite` could replace a record or blob) and two destinations that are
/// the same file. Run before anything is created.
pub fn check_destinations(
    store: &RevisionStore,
    out: &Path,
    report: Option<&Path>,
) -> Result<(), PlateError> {
    let refuse = |e: OutputError| error(e.code, e.message);
    let root = store.blobs().canonical_root();
    let first = resolve_destination(out).map_err(refuse)?;
    let second = report
        .map(|path| resolve_destination(path).map_err(refuse))
        .transpose()?;
    for destination in std::iter::once(&first).chain(second.as_ref()) {
        if destination.directory.starts_with(root) {
            return Err(error(
                ErrorCode::InvalidArgument,
                "an output may not be written inside the store it is exported from",
            ));
        }
    }
    if second.as_ref().is_some_and(|s| s.same_file(&first)) {
        return Err(error(
            ErrorCode::InvalidArgument,
            "the IFC output and the report output are the same file",
        ));
    }
    Ok(())
}

/// [`export_ifc`] up to, but not including, the install of the file.
pub fn prepare_export(
    store: &RevisionStore,
    request: &ExportRequest,
    out: &Path,
) -> Result<PreparedExport, PlateError> {
    prepare_export_with_limits(store, request, out, &Limits::DEFAULT)
}

/// [`prepare_export`] with explicit bounds.
pub fn prepare_export_with_limits(
    store: &RevisionStore,
    request: &ExportRequest,
    out: &Path,
    limits: &Limits,
) -> Result<PreparedExport, PlateError> {
    validate_request(request)?;
    check_destinations(store, out, None)?;
    let revision = open_revision(store, request.revision)?;
    let inspection = inspect_revision(&revision)?;
    inspection.require_evaluated_geometry()?;
    let mut built = build(&inspection, request, &revision)?;
    if request.strict
        && let Some(fatal) = first_fatal(&built.findings)
        && let Some(code) = fatal.f04_code
    {
        return Err(error(code, fatal.message));
    }
    let mut outcome = None;
    let staged = stage_file(out, request.overwrite, |file| {
        match write_ifc(&built.model, limits, &mut *file) {
            Ok((_, written)) => {
                outcome = Some(written);
                Ok(())
            }
            Err(e) => {
                let e = write_error(e);
                Err(OutputError {
                    code: e.code,
                    message: e.message,
                })
            }
        }
    })
    .map_err(|e| error(e.code, e.message))?;
    let written = outcome
        .ok_or_else(|| error(ErrorCode::BackendUnavailable, "the output was not written"))?;
    for t in &written.truncations {
        let (element_id, field) = match &t.subject {
            Subject::Element(id) => (ElementId::from_uuid(*id).into(), t.field),
            _ => (None, t.field),
        };
        built.findings.push(Finding {
            code: "IFC_NAME_TRUNCATED",
            f04_code: None,
            severity: Severity::Warning,
            element_id,
            field,
            message: "a text longer than 255 characters was cut in an IfcLabel attribute",
        });
    }
    built.findings.sort_by_key(rank);
    let total = built.findings.len().min(MAX_COUNTED_FINDINGS);
    built.findings.truncate(MAX_LISTED_FINDINGS);
    let counts = Counts {
        elements_total: built.report_elements.len(),
        exported: built.report_elements.iter().filter(|e| e.exported).count(),
        not_exported: built.report_elements.iter().filter(|e| !e.exported).count(),
        proxies: built
            .report_elements
            .iter()
            .filter(|e| e.entity == Some("IfcBuildingElementProxy"))
            .count(),
        without_representation: built
            .report_elements
            .iter()
            .filter(|e| e.exported && e.representation.is_none())
            .count(),
    };
    let lossy = total > 0 || counts.not_exported > 0;
    Ok(PreparedExport {
        staged,
        report: ExportReport {
            project_id: built.model.project_id,
            revision_id: revision_id(&revision)?,
            status: if lossy {
                Status::OkWithLosses
            } else {
                Status::Ok
            },
            truncated: built.findings.len() < total,
            findings: built.findings,
            total_findings: total,
            counts,
            levels: built.levels,
            elements: built.report_elements,
            output: Some(OutputInfo {
                bytes: written.bytes,
                sha256: written.sha256,
            }),
        },
    })
}

/// The GlobalId of an ElementId text, for `ifc-guid --id` (`INVALID_ID` for anything that is not
/// a canonical lowercase UUIDv7, with no repair, `IFCX-GUID-002`).
pub fn ifc_guid_of(text: &str) -> Result<String, PlateError> {
    let id = Uuid7::parse(text)
        .map_err(|_| error(ErrorCode::InvalidId, "the id is not a canonical UUIDv7"))?;
    Ok(global_id(&id).as_str().to_owned())
}

/// The UUID text behind a GlobalId, for `ifc-guid --decode` (`INVALID_ARGUMENT` for anything else,
/// `IFCX-GUID-003`). The 128-bit value is printed even when it is not a valid UUIDv7.
pub fn ifc_guid_decode(text: &str) -> Result<String, PlateError> {
    let bytes = guid::decode(text)
        .map_err(|_| error(ErrorCode::InvalidArgument, "not an IfcGloballyUniqueId"))?;
    let mut out = String::with_capacity(36);
    for (i, b) in bytes.iter().enumerate() {
        if matches!(i, 4 | 6 | 8 | 10) {
            out.push('-');
        }
        out.push_str(&format!("{b:02x}"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use beamcraft_model::command::Placement;

    fn id(last: u8) -> ElementId {
        ElementId::parse(&format!("019a7bf0-0000-7000-8000-0000000000{last:02x}")).unwrap()
    }

    fn meta(placement: PlacementRead) -> ElementMeta {
        ElementMeta {
            kind: "weld".into(),
            name: Some("W".into()),
            schema_version: Some("0.1.0".into()),
            geometry_mode: Some("parametric".into()),
            deactivated: false,
            placement,
            part_of: None,
            mesh: None,
        }
    }

    fn triangle() -> crate::plate::PublishedMesh {
        crate::plate::PublishedMesh {
            points: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            triangles: vec![[0, 1, 2]],
        }
    }

    fn proxy_with_mesh(placement: PlacementRead) -> (ElementData, Vec<Finding>) {
        let mut meta = meta(placement);
        meta.mesh = Some(triangle());
        let mut row = ElementReport {
            element_id: id(1),
            ifc_global_id: None,
            kind: "weld".into(),
            exported: false,
            entity: None,
            representation: None,
            volume: None,
            mass: None,
            omissions: Vec::new(),
        };
        let mut findings = Vec::new();
        let data = proxy_element(id(1), &meta, None, &mut row, &mut findings);
        (data, findings)
    }

    #[test]
    fn a_proxy_keeps_its_published_mesh_when_it_has_a_pose() {
        let (data, findings) = proxy_with_mesh(PlacementRead::Rigid(Placement::IDENTITY));
        let mesh = data.mesh.unwrap();
        assert_eq!(mesh.triangles, [[0, 1, 2]]);
        assert!(!mesh.closed, "an unmapped kind is never certified closed");
        assert!(data.pose.is_some());
        assert_eq!(
            findings.iter().map(|f| f.code).collect::<Vec<_>>(),
            ["IFC_KIND_PROXY"]
        );
    }

    #[test]
    fn a_published_mesh_without_a_pose_is_dropped_and_listed() {
        for placement in [PlacementRead::Absent, PlacementRead::Other] {
            let (data, findings) = proxy_with_mesh(placement);
            assert!(data.mesh.is_none() && data.pose.is_none());
            assert!(findings.iter().any(|f| f.code == "IFC_PLACEMENT_ABSENT"));
        }
    }

    fn proxy(placement: PlacementRead) -> (ElementData, Vec<Finding>) {
        let mut row = ElementReport {
            element_id: id(1),
            ifc_global_id: None,
            kind: "weld".into(),
            exported: false,
            entity: None,
            representation: None,
            volume: None,
            mass: None,
            omissions: Vec::new(),
        };
        let mut findings = Vec::new();
        let data = proxy_element(id(1), &meta(placement), None, &mut row, &mut findings);
        (data, findings)
    }

    #[test]
    fn a_rigid_pose_is_exported_and_listed_only_as_a_proxy() {
        let (data, findings) = proxy(PlacementRead::Rigid(Placement::IDENTITY));
        assert!(data.pose.is_some() && data.mesh.is_none());
        assert_eq!(
            findings.iter().map(|f| f.code).collect::<Vec<_>>(),
            ["IFC_KIND_PROXY"]
        );
        let (data, findings) = proxy(PlacementRead::Absent);
        assert!(data.pose.is_none());
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn a_pose_that_is_not_rigid_is_not_exported_and_is_fatal_only_in_strict_mode() {
        let broken = Placement {
            translation_mm: [f64::NAN, 0.0, 0.0],
            ..Placement::IDENTITY
        };
        for placement in [PlacementRead::Other, PlacementRead::Rigid(broken)] {
            let (data, findings) = proxy(placement);
            assert!(data.pose.is_none() && data.mesh.is_none());
            let codes: Vec<_> = findings.iter().map(|f| f.code).collect();
            assert_eq!(codes, ["IFC_KIND_PROXY", "IFC_PLACEMENT_ABSENT"]);
            let fatal = first_fatal(&findings).unwrap();
            assert_eq!(fatal.code, "IFC_PLACEMENT_ABSENT");
            assert_eq!(fatal.f04_code, Some(ErrorCode::UnsupportedGeometry));
        }
    }

    #[test]
    fn findings_without_an_f04_code_are_never_fatal() {
        let (_, findings) = proxy(PlacementRead::Absent);
        assert!(first_fatal(&findings).is_none());
    }

    #[test]
    fn the_first_fatal_finding_follows_the_finding_order() {
        let mk = |code: &'static str, element: u8| Finding {
            code,
            f04_code: Some(ErrorCode::UnsupportedGeometry),
            severity: Severity::Warning,
            element_id: Some(id(element)),
            field: "x",
            message: "m",
        };
        let findings = [mk("IFC_PLACEMENT_ABSENT", 2), mk("IFC_GEOMETRY_ABSENT", 9)];
        assert_eq!(first_fatal(&findings).unwrap().code, "IFC_GEOMETRY_ABSENT");
    }

    #[test]
    fn requests_are_validated_before_anything_is_read() {
        let ok = ExportRequest::default();
        assert!(validate_request(&ok).is_ok());
        let code = |r: ExportRequest| validate_request(&r).unwrap_err().code;
        assert_eq!(
            code(ExportRequest {
                contract_version: "0.2.0".into(),
                ..ok.clone()
            }),
            ErrorCode::UnsupportedVersion
        );
        assert_eq!(
            code(ExportRequest {
                schema: "IFC4X3".into(),
                ..ok.clone()
            }),
            ErrorCode::UnsupportedVersion
        );
        assert_eq!(
            code(ExportRequest {
                format: "ifcjson".into(),
                ..ok.clone()
            }),
            ErrorCode::UnsupportedProperty
        );
        assert_eq!(
            code(ExportRequest {
                project_name: Some(String::new()),
                ..ok
            }),
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn every_refusal_code_has_its_ifc_token() {
        for (code, token) in [
            (ErrorCode::UnsupportedVersion, "IFC_VERSION_UNSUPPORTED"),
            (ErrorCode::UnsupportedProperty, "IFC_FORMAT_UNSUPPORTED"),
            (ErrorCode::InvalidId, "IFC_ID_INVALID"),
            (ErrorCode::NotFound, "IFC_NOT_FOUND"),
            (ErrorCode::DependencyUnavailable, "IFC_INPUT_UNAVAILABLE"),
            (ErrorCode::DanglingRelationship, "IFC_LEVEL_DANGLING"),
            (ErrorCode::DuplicateId, "IFC_ID_DUPLICATE"),
            (ErrorCode::IntegrityError, "IFC_INTEGRITY"),
            (ErrorCode::LimitExceeded, "IFC_LIMIT"),
            (ErrorCode::BackendUnavailable, "IFC_OUTPUT_FAILED"),
            (ErrorCode::Cancelled, "IFC_CANCELLED"),
            (ErrorCode::InvalidArgument, "IFC_ARGUMENT_INVALID"),
        ] {
            assert_eq!(ifc_code_of(code), token);
        }
    }

    #[test]
    fn freshness_tokens_follow_the_stored_record() {
        assert_eq!(freshness(false, Freshness::Missing, "current"), "missing");
        assert_eq!(freshness(true, Freshness::Stale, "current"), "stale");
        assert_eq!(freshness(true, Freshness::Current, "current"), "current");
        assert_eq!(freshness(true, Freshness::Current, "unknown"), "unknown");
    }
}
