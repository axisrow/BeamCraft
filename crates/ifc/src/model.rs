// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The plain input of the writer (E01 section 15.2, item `model`).
//!
//! The caller resolves everything: placement vectors, mesh points and triangles, quantities with
//! their freshness, names, digests, declared values and levels. The writer derives no geometry,
//! calls no kernel and reads no store. Lists must already be in the contract order: elements by
//! ascending [`ElementId`](beamcraft_model::id::ElementId), levels by ascending id and materials
//! by ascending digest; the writer checks this and refuses an input that is not.

use beamcraft_model::digest::ContentDigest;
use beamcraft_model::id::Uuid7;

/// Bounds of one export (E01 section 14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Elements per file (`IFCX-LIM-001`).
    pub max_elements: usize,
    /// Levels per project (`IFCX-LIM-005`).
    pub max_levels: usize,
    /// Vertices of one mesh (`IFCX-LIM-005`).
    pub max_vertices: usize,
    /// Triangles of one mesh (`IFCX-LIM-005`).
    pub max_triangles: usize,
    /// Output bytes (`IFCX-LIM-002`).
    pub max_bytes: u64,
}

impl Limits {
    /// The limits of the contract.
    pub const DEFAULT: Self = Self {
        max_elements: 65_536,
        max_levels: 1_024,
        max_vertices: 262_144,
        max_triangles: 524_288,
        max_bytes: 268_435_456,
    };
}

impl Default for Limits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The IFC entity an element becomes (`IFCX-SCOPE-001`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entity {
    /// `IfcBeam`, predefined type `.BEAM.`.
    Beam,
    /// `IfcColumn`, predefined type `.COLUMN.`.
    Column,
    /// `IfcPlate`, predefined type `.NOTDEFINED.`.
    Plate,
    /// `IfcBuildingElementProxy`, predefined type `.NOTDEFINED.`.
    Proxy,
}

impl Entity {
    /// The IFC entity name in upper case, as written in the file.
    pub const fn step_name(self) -> &'static str {
        match self {
            Self::Beam => "IFCBEAM",
            Self::Column => "IFCCOLUMN",
            Self::Plate => "IFCPLATE",
            Self::Proxy => "IFCBUILDINGELEMENTPROXY",
        }
    }

    /// The IFC entity name in the schema's spelling, for reports.
    pub const fn ifc_name(self) -> &'static str {
        match self {
            Self::Beam => "IfcBeam",
            Self::Column => "IfcColumn",
            Self::Plate => "IfcPlate",
            Self::Proxy => "IfcBuildingElementProxy",
        }
    }

    /// The predefined-type enumeration literal.
    pub const fn predefined(self) -> &'static str {
        match self {
            Self::Beam => ".BEAM.",
            Self::Column => ".COLUMN.",
            Self::Plate | Self::Proxy => ".NOTDEFINED.",
        }
    }

    const fn qto_name(self) -> Option<&'static str> {
        match self {
            Self::Beam => Some("Qto_BeamBaseQuantities"),
            Self::Column => Some("Qto_ColumnBaseQuantities"),
            Self::Plate => Some("Qto_PlateBaseQuantities"),
            Self::Proxy => None,
        }
    }

    pub(crate) const fn quantity_set(self) -> Option<&'static str> {
        self.qto_name()
    }
}

/// An element placement as an `IfcAxis2Placement3D` (`IFCX-PLC-001`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    /// `Location`, millimetres.
    pub translation_mm: [f64; 3],
    /// `Axis`: the local +Z in world directions.
    pub axis: [f64; 3],
    /// `RefDirection`: the local +X in world directions.
    pub ref_direction: [f64; 3],
}

impl Pose {
    /// The pose of a rigid placement `world = R(q) * local + translation` with the quaternion in
    /// `(x, y, z, w)` order (`IFCX-PLC-004`). `q` is normalized first, so `q` and `-q` give the
    /// same numbers, and negative zero becomes zero. `None` for a non-finite component or a
    /// zero quaternion.
    pub fn from_quaternion(translation_mm: [f64; 3], xyzw: [f64; 4]) -> Option<Self> {
        let [x, y, z, w] = xyzw;
        let norm = (((x * x + y * y) + z * z) + w * w).sqrt();
        if !norm.is_finite() || norm == 0.0 || translation_mm.iter().any(|v| !v.is_finite()) {
            return None;
        }
        let (x, y, z, w) = (x / norm, y / norm, z / norm, w / norm);
        let clean = |v: f64| v + 0.0;
        let axis = [
            clean(2.0 * (x * z + w * y)),
            clean(2.0 * (y * z - w * x)),
            clean(1.0 - 2.0 * (x * x + y * y)),
        ];
        let ref_direction = [
            clean(1.0 - 2.0 * (y * y + z * z)),
            clean(2.0 * (x * y + w * z)),
            clean(2.0 * (x * z - w * y)),
        ];
        Some(Self {
            translation_mm: translation_mm.map(clean),
            axis,
            ref_direction,
        })
    }
}

/// The published mesh in element-local millimetres (`IFCX-GEO-002`).
#[derive(Debug, Clone, PartialEq)]
pub struct Mesh {
    /// Points in their stored order.
    pub points: Vec<[f32; 3]>,
    /// Triangles as zero-based vertex indices in their stored order; the writer adds one.
    pub triangles: Vec<[u32; 3]>,
    /// Whether the shell is certified closed (plate, beam, column); unset in the file otherwise.
    pub closed: bool,
}

/// The stored quantity records of a plate, beam or column and their freshness (E01 section 8).
#[derive(Debug, Clone, PartialEq)]
pub struct QuantityData {
    /// `current`, `stale`, `missing`, `unknown` or `unsupported`.
    pub volume_status: String,
    /// `current`, `stale`, `missing`, `unknown` or `unsupported`.
    pub mass_status: String,
    /// Shape signature of the stored record.
    pub shape_signature: Option<ContentDigest>,
    /// Mass signature of the stored record.
    pub mass_signature: Option<ContentDigest>,
    /// Algorithm label of the stored record.
    pub algorithm: Option<String>,
    /// Volume in cubic metres, only when current (`NetVolume`).
    pub net_volume_m3: Option<f64>,
    /// Volume in cubic metres of an element without operations (`GrossVolume`).
    pub gross_volume_m3: Option<f64>,
    /// Mass in kilograms, only when current (`NetWeight`).
    pub net_weight_kg: Option<f64>,
    /// Mass in kilograms of an element without operations (`GrossWeight`).
    pub gross_weight_kg: Option<f64>,
}

/// Declared section values of a profile definition (E01 section 9.2), written as authored.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Declared {
    /// Declared designation.
    pub designation: Option<String>,
    /// Declared standard.
    pub standard: Option<String>,
    /// Declared area in mm2.
    pub area_mm2: Option<f64>,
    /// Declared mass per metre in kg.
    pub mass_per_meter_kg: Option<f64>,
    /// Declared surface per metre in m2.
    pub surface_per_meter_m2: Option<f64>,
}

impl Declared {
    /// Whether nothing is declared (the property set is then not written).
    pub fn is_empty(&self) -> bool {
        self.designation.is_none()
            && self.standard.is_none()
            && self.area_mm2.is_none()
            && self.mass_per_meter_kg.is_none()
            && self.surface_per_meter_m2.is_none()
    }
}

/// One exported element, with every value resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementData {
    /// The ElementId (`bim:id`).
    pub id: Uuid7,
    /// The entity it becomes.
    pub entity: Entity,
    /// The element-kind token (`plate`, `beam`, ... or the token of a proxy).
    pub kind_token: String,
    /// `bim:name`.
    pub name: String,
    /// `bim:schemaVersion`.
    pub schema_version: String,
    /// `bim:geometryMode`.
    pub geometry_mode: String,
    /// The placement, when the element has a rigid one.
    pub pose: Option<Pose>,
    /// The mesh, when the element has a usable one. Requires `pose`.
    pub mesh: Option<Mesh>,
    /// The level it belongs to.
    pub level: Option<Uuid7>,
    /// The pinned material definition.
    pub material: Option<ContentDigest>,
    /// The pinned profile definition (members).
    pub profile: Option<ContentDigest>,
    /// Number of through holes (plates).
    pub hole_count: Option<u32>,
    /// Quantity records (plate, beam, column).
    pub quantities: Option<QuantityData>,
    /// Declared section values (members).
    pub declared: Option<Declared>,
    /// The assembly this element is a part of (`AssemblyId`).
    pub assembly: Option<Uuid7>,
}

/// A level (`IfcBuildingStorey`).
#[derive(Debug, Clone, PartialEq)]
pub struct LevelData {
    /// The LevelId.
    pub id: Uuid7,
    /// Name.
    pub name: String,
    /// Elevation in millimetres, written as stored.
    pub elevation_mm: f64,
}

/// A pinned material definition used by at least one exported element.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialData {
    /// Full content digest of the definition.
    pub digest: ContentDigest,
    /// Declared grade (`Name`), or `Unspecified` when absent.
    pub grade: Option<String>,
    /// Declared standard (`Description`).
    pub standard: Option<String>,
    /// Density in kg/m3, only when authored.
    pub density_kg_m3: Option<f64>,
}

/// Everything the writer needs for one file.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportModel {
    /// The stored ProjectId (`IFCX-PRJ-001`).
    pub project_id: Uuid7,
    /// The optional project name.
    pub project_name: Option<String>,
    /// Unix milliseconds of the RevisionId, for the header time stamp (`IFCX-DET-005`).
    pub revision_unix_ms: u64,
    /// Levels by ascending LevelId.
    pub levels: Vec<LevelData>,
    /// Materials by ascending digest.
    pub materials: Vec<MaterialData>,
    /// Elements by ascending ElementId.
    pub elements: Vec<ElementData>,
}
