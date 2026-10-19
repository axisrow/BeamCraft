// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! F04 command application for `CreatePlate` (P01): engineering validation, kernel evaluation
//! and the derived results of one accepted command.
//!
//! The engine never touches USD, files or a clock. It turns a typed command into an
//! [`AuthoredPlate`] (what the model layer must carry), a sealed kernel body (what the derived
//! layer must carry), quantities and a declared footprint. The adapter that writes USD and the
//! host that publishes revisions live in other crates and are told only these results.
//!
//! # What this slice supports, and what it does not
//!
//! F04 section 3 (P-1) admits any planar contour. The origin rectangle `(0,0,0), (w,0,0),
//! (w,l,0), (0,l,0)` keeps its G02 evaluation (and its stored meshes and quantities) unchanged.
//! Every other closed counter-clockwise outline in the plate-local XY plane is built with the
//! A07.1 profile machinery: the G05 outline admission (winding, simplicity, floors) and
//! extrusion along `+Z` by the thickness, certified as one closed shell by G06, with the A04
//! through holes cut by G09 exactly as on the rectangle. A clockwise outline, a self-intersecting
//! one, one below the area floor or outside the caps is `INVALID_ARGUMENT` or the kernel's own
//! code; nothing is guessed. A contour at a constant nonzero `z` is valid F04 input in its own
//! body frame but is not evaluated here: as in the first slice it is `UNSUPPORTED_GEOMETRY`,
//! because the plate body frame is the local XY plane and the placement carries translations.
//! A placement with a
//! rotation other than the identity is valid input but unsupported for evaluation here
//! (`UNSUPPORTED_GEOMETRY`): the view contract carries translations only.

use crate::footprint::Footprint;
use crate::solid::SolidMesh;
use beamcraft_geom::tolerance::TOLERANCE_POLICY_VERSION;
use beamcraft_geom::{MathError, Point3};
use beamcraft_kernel::diagnostic::Failure;
use beamcraft_kernel::measure::analytic_mass_kg;
use beamcraft_kernel::ops::{Body, HoleCutter, OpCode, OpFailure, OpResult, through_hole};
use beamcraft_kernel::profile::diagnostic::{Defect, ProfileFailure, ProfileField};
use beamcraft_kernel::profile::{Cavity, Point2, extrude};
use beamcraft_kernel::{RectangularPlate, rectangular_plate};
use beamcraft_model::command::{CommandError, CreatePlate, Placement};
use beamcraft_model::definition::MaterialSpec;
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;

/// Version label of the volume algorithm in a [`Quantities`] record.
pub const VOLUME_ALGORITHM: &str = "kernel-g02-rectangular-plate-volume/1;tolerance-policy-3";

/// Version label of the volume algorithm of a plate with through holes: the midpoint of the G06
/// enclosure of the certified result of the G09 hole operation.
pub const HOLED_VOLUME_ALGORITHM: &str = "kernel-g09-through-hole-volume/1;tolerance-policy-3";

/// Version label of the volume algorithm of a contour plate without holes: the analytic
/// `net_area * thickness` of the G05 extrusion of the outline (A02, experimental).
pub const CONTOUR_VOLUME_ALGORITHM: &str = "kernel-g05-contour-plate-volume/1;tolerance-policy-3";

const _: () = assert!(TOLERANCE_POLICY_VERSION == 3);

/// The physical content of a resolved immutable material definition.
///
/// The adapter that reads the committed model hands this over; the engine never reads USD. A
/// density of `None` is unknown and stays unknown.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaterialInfo {
    /// Density in kg/m3, when the definition authors one.
    pub density_kg_m3: Option<f64>,
}

/// Resolves a pinned material digest. A digest with no definition is `NOT_FOUND`; a digest that
/// names a definition of another kind is `INVALID_ARGUMENT`.
pub type MaterialLookup<'a> = &'a dyn Fn(&ContentDigest) -> Result<MaterialInfo, CommandError>;

/// A lookup that knows no definition: any pinned material is `NOT_FOUND`.
pub fn no_materials(_: &ContentDigest) -> Result<MaterialInfo, CommandError> {
    Err(CommandError::new(
        ErrorCode::NotFound,
        "material_digest",
        "material definition is not in the model",
    ))
}

/// The authored state of one plate: exactly the inputs F04 section 4 stores in the model layer.
/// Nothing derived is in it.
#[derive(Debug, Clone, PartialEq)]
pub struct AuthoredPlate {
    /// Full element identity.
    pub id: ElementId,
    /// Label.
    pub name: String,
    /// Element-local contour as submitted, in millimetres.
    pub contour_points_mm: Vec<[f64; 3]>,
    /// Thickness in millimetres.
    pub thickness_mm: f64,
    /// Rigid placement as submitted.
    pub placement: Placement,
    /// Pinned physical material definition; absence means the material and density are unknown.
    pub material: Option<ContentDigest>,
    /// Through-hole polygons in the plate's local XY, as authored. Empty means a solid plate.
    pub holes_mm: Vec<Vec<[f64; 2]>>,
}

impl AuthoredPlate {
    /// The authored fields of an accepted command (the command's arguments, unchanged).
    pub fn from_command(command: &CreatePlate) -> Self {
        Self {
            id: command.new_id,
            name: command.name.clone(),
            contour_points_mm: command.contour_points_mm.clone(),
            thickness_mm: command.thickness_mm,
            placement: command.placement,
            material: command.material_digest,
            holes_mm: command.holes_mm.clone(),
        }
    }

    /// The `CreatePlate` that would author exactly this state; used to compare content.
    pub fn to_command(&self) -> CreatePlate {
        CreatePlate {
            new_id: self.id,
            name: self.name.clone(),
            contour_points_mm: self.contour_points_mm.clone(),
            thickness_mm: self.thickness_mm,
            placement: self.placement,
            material_digest: self.material,
            holes_mm: self.holes_mm.clone(),
        }
    }
}

/// Status of one quantity (F04 section 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuantityStatus {
    /// A supported computation over matching authored inputs.
    Current,
    /// No value: an input such as density is absent.
    Unknown,
}

impl QuantityStatus {
    /// The wire token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Unknown => "unknown",
        }
    }
}

/// How the cross-section area of a member was obtained (P-Mass).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AreaSource {
    /// The area the pinned profile definition declares (a library entry or `--area-mm2`).
    Catalogue,
    /// The exact area computed from the section's parameters (closed form, true arcs).
    Computed,
}

impl AreaSource {
    /// The wire token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Catalogue => "catalogue",
            Self::Computed => "computed",
        }
    }
}

/// The quantity records of one plate or member. Mass is `current` only with a resolved material
/// that authors a density; otherwise it is `unknown` and carries no value: density is never
/// guessed. The input signature of F06 section 7 is not implemented (`keys` are planned), so no
/// signature is claimed.
///
/// **Gross and net (P-Mass).** The gross volume is the nominal body before any operation: for a
/// member the effective section area times the axis length, for a plate the outline area times
/// the thickness. The net volume is the body after the operations (through holes today). With no
/// operation the two are the same value. The removed volume is *measured* on the kernel meshes as
/// the volume before an operation minus the volume after it, so the arc approximation of a
/// detailed outline cancels between the two meshes; `net = gross - removed`.
/// `volume_mm3` and `mass_kg` are the NET values, kept under their old names so existing readers
/// do not change.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quantities {
    /// NET volume in mm3 (the alias the first quantity slices exposed).
    pub volume_mm3: f64,
    /// NET volume in m3: the mm3 value divided by 1e9.
    pub volume_m3: f64,
    /// GROSS volume in mm3: the nominal body before any operation.
    pub volume_gross_mm3: f64,
    /// Volume removed by the operations in mm3, measured on the meshes; `0.0` without operations.
    pub removed_volume_mm3: f64,
    /// Status of the volume record.
    pub volume_status: QuantityStatus,
    /// Density in kg/m3 used for the mass, when the material authors one.
    pub density_kg_m3: Option<f64>,
    /// NET mass in kg, present exactly when `mass_status` is `current`.
    pub mass_kg: Option<f64>,
    /// GROSS mass in kg, present exactly when `mass_status` is `current`.
    pub mass_gross_kg: Option<f64>,
    /// Status of the mass record.
    pub mass_status: QuantityStatus,
    /// Algorithm version of the volume record.
    pub algorithm: &'static str,
    /// Effective section area in mm2 of a member; `None` for a plate.
    pub section_area_mm2: Option<f64>,
    /// Where the effective area came from; `None` for a plate.
    pub area_source: Option<AreaSource>,
}

/// A plate evaluated by the kernel: the sealed body and its supported dimensions.
#[derive(Debug, Clone, PartialEq)]
pub struct PlateBody {
    /// The certified mesh of the plate as authored: the box, the contour extrusion, with or
    /// without its through holes.
    pub solid: SolidMesh,
    /// The volume-algorithm label before holes: the G02 box or the G05 contour extrusion. A
    /// plate with holes reports the G09 label instead.
    pub volume_algorithm: &'static str,
    /// GROSS volume in mm3: the outline area times the thickness of the uncut body, exact for
    /// the polygon (the P-Mass nominal volume, before any hole).
    pub volume_gross_mm3: f64,
    /// Width along local X of the outline's bounding box, in millimetres.
    pub width_mm: f64,
    /// Length along local Y of the outline's bounding box, in millimetres.
    pub length_mm: f64,
    /// Thickness along local Z, in millimetres.
    pub thickness_mm: f64,
    /// Number of through holes applied.
    pub hole_count: usize,
}

impl PlateBody {
    /// The algorithm label of this body's volume record.
    pub const fn algorithm(&self) -> &'static str {
        if self.hole_count == 0 {
            self.volume_algorithm
        } else {
            HOLED_VOLUME_ALGORITHM
        }
    }

    fn volumes(&self) -> Volumes {
        // The gross body is the exact outline extrusion (the box or the polygon prism), so the
        // gross volume is its analytic area times the thickness and equals the mesh before the
        // holes; the holes remove what the two meshes differ by.
        Volumes::measured(
            self.volume_gross_mm3,
            self.volume_gross_mm3,
            self.solid.volume_mm3(),
        )
    }

    /// The quantity records of this body with no material: volume only, mass `unknown`.
    pub fn quantities(&self) -> Quantities {
        quantities_without_material(self.volumes(), self.algorithm())
    }

    /// The quantity records of this body with the physical material of the element.
    ///
    /// The density is re-admitted here because a committed definition is untrusted input. Mass is
    /// `fl(fl(V_mm3 * density) / 1e9)` (F06 section 2); an absent density keeps it `unknown`.
    pub fn quantities_for(
        &self,
        material: Option<MaterialInfo>,
    ) -> Result<Quantities, CommandError> {
        quantities_with_material(self.volumes(), self.algorithm(), material)
    }
}

/// The three volumes of a body: nominal (gross), removed by operations and net.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Volumes {
    pub(crate) gross_mm3: f64,
    pub(crate) removed_mm3: f64,
    pub(crate) net_mm3: f64,
}

impl Volumes {
    /// A body with no operation: net is gross.
    pub(crate) const fn unchanged(gross_mm3: f64) -> Self {
        Self {
            gross_mm3,
            removed_mm3: 0.0,
            net_mm3: gross_mm3,
        }
    }

    /// The rule for operations: `removed` is the volume of the mesh before the operations minus
    /// that of the mesh after them, and `net = gross - removed`. When the gross body is the mesh
    /// before (a box is exact) the two are the same number, so the measured volume after is used
    /// as is and no rounding is added.
    pub(crate) fn measured(gross_mm3: f64, mesh_before_mm3: f64, mesh_after_mm3: f64) -> Self {
        let removed_mm3 = mesh_before_mm3 - mesh_after_mm3;
        let net_mm3 = if gross_mm3 == mesh_before_mm3 {
            mesh_after_mm3
        } else {
            gross_mm3 - removed_mm3
        };
        Self {
            gross_mm3,
            removed_mm3,
            net_mm3,
        }
    }
}

pub(crate) fn quantities_without_material(volumes: Volumes, algorithm: &'static str) -> Quantities {
    Quantities {
        volume_mm3: volumes.net_mm3,
        volume_m3: volumes.net_mm3 / 1_000_000_000.0,
        volume_gross_mm3: volumes.gross_mm3,
        removed_volume_mm3: volumes.removed_mm3,
        volume_status: QuantityStatus::Current,
        density_kg_m3: None,
        mass_kg: None,
        mass_gross_kg: None,
        mass_status: QuantityStatus::Unknown,
        algorithm,
        section_area_mm2: None,
        area_source: None,
    }
}

pub(crate) fn quantities_with_material(
    volumes: Volumes,
    algorithm: &'static str,
    material: Option<MaterialInfo>,
) -> Result<Quantities, CommandError> {
    let mut quantities = quantities_without_material(volumes, algorithm);
    let Some(density) = material.and_then(|m| m.density_kg_m3) else {
        return Ok(quantities);
    };
    MaterialSpec {
        density_kg_m3: Some(density),
        ..MaterialSpec::default()
    }
    .validate()?;
    let mass = |volume_mm3: f64| {
        analytic_mass_kg(volume_mm3, Some(density)).map_err(|code| {
            CommandError::new(
                code.f04_code(),
                "density_kg_m3",
                "mass could not be computed",
            )
        })
    };
    let net = mass(volumes.net_mm3)?;
    let gross = mass(volumes.gross_mm3)?;
    quantities.density_kg_m3 = Some(density);
    quantities.mass_kg = net.kg;
    quantities.mass_gross_kg = gross.kg;
    quantities.mass_status = if net.kg.is_some() && gross.kg.is_some() {
        QuantityStatus::Current
    } else {
        QuantityStatus::Unknown
    };
    Ok(quantities)
}

/// Everything one accepted `CreatePlate` produces for the layers and the host.
#[derive(Debug, Clone, PartialEq)]
pub struct CreatePlateOutcome {
    /// The authored state for the model layer.
    pub authored: AuthoredPlate,
    /// The evaluated body for the derived layer.
    pub body: PlateBody,
    /// Quantity records.
    pub quantities: Quantities,
    /// Declared read and write sets; callers cannot narrow them.
    pub footprint: Footprint,
    /// Digest of the typed arguments, for replay and reuse decisions.
    pub fingerprint: ContentDigest,
}

fn map_math(error: MathError, field: &'static str) -> CommandError {
    let (code, message) = match error {
        MathError::NonFinite => (ErrorCode::NonFiniteNumber, "value is not finite"),
        MathError::AboveLimit => (ErrorCode::LimitExceeded, "value exceeds its limit"),
        MathError::InvalidPointCount => (
            ErrorCode::InvalidArgument,
            "contour needs at least 3 points",
        ),
        MathError::NotPositive | MathError::BelowMinimum => (
            ErrorCode::InvalidArgument,
            "dimension is not above the geometric floor",
        ),
        MathError::AxisTooShort | MathError::ZeroAxis | MathError::InvalidFrame => {
            (ErrorCode::InvalidArgument, "degenerate geometry")
        }
        MathError::NumericUncertain => (
            ErrorCode::UnsupportedGeometry,
            "numeric result is uncertain",
        ),
    };
    CommandError::new(code, field, message)
}

fn map_kernel(failure: &Failure) -> CommandError {
    failure.diagnostics().next().map_or(
        CommandError::new(
            ErrorCode::UnsupportedGeometry,
            "contour_points_mm",
            "geometry evaluation failed",
        ),
        |d| {
            let field = match d.field {
                beamcraft_kernel::diagnostic::Field::Width
                | beamcraft_kernel::diagnostic::Field::Length => "contour_points_mm",
                beamcraft_kernel::diagnostic::Field::Thickness => "thickness_mm",
                _ => "geometry",
            };
            CommandError::new(d.code.f04_code(), field, "geometry evaluation failed")
        },
    )
}

/// How a contour relates to the supported shapes.
enum Contour {
    /// `(0,0,0), (w,0,0), (w,l,0), (0,l,0)` with `w, l > 0`: the G02 box.
    OriginRectangle { width: f64, length: f64 },
    /// Any other closed outline: the A02 contour extrusion.
    Other,
}

fn classify(points: &[[f64; 3]]) -> Contour {
    // The closure of a contour is implicit, so the origin rectangle may be listed starting at
    // any of its corners. The cycle is rotated onto the origin corner first, so the same closed
    // rectangle keeps its G02 evaluation, quantities and mesh digest whatever the cyclic start;
    // a rotation never changes the polygon the points describe.
    let start = points
        .iter()
        .position(|p| p[0] == 0.0 && p[1] == 0.0 && p[2] == 0.0);
    let rotated: Vec<[f64; 3]> = match start {
        Some(start) if start > 0 => points
            .iter()
            .skip(start)
            .chain(points.iter().take(start))
            .copied()
            .collect(),
        _ => points.to_vec(),
    };
    let [p0, p1, p2, p3] = match rotated.as_slice() {
        [a, b, c, d] => [*a, *b, *c, *d],
        _ => return Contour::Other,
    };
    let zero = |v: f64| v == 0.0;
    if !(zero(p0[0]) && zero(p0[1]) && zero(p0[2])) {
        return Contour::Other;
    }
    if zero(p1[1]) && zero(p1[2]) && p1[0] > 0.0 {
        let (w, l) = (p1[0], p2[1]);
        if p2[0] == w && zero(p2[2]) && l > 0.0 && zero(p3[0]) && p3[1] == l && zero(p3[2]) {
            return Contour::OriginRectangle {
                width: w,
                length: l,
            };
        }
    }
    // The clockwise mirror is a valid `Other` outline that the G05 admission refuses by winding,
    // like every other clockwise contour.
    Contour::Other
}

/// Evaluates an authored plate with the kernel: engineering validation in the F04 order that
/// P01 implements, then the sealed rectangular body or the A02 contour extrusion.
///
/// Order: count and finiteness and coordinate caps (`contour_points_mm`), local size `D` and its
/// cap, thickness range and floor, repeated consecutive points, then the supported shape (see
/// the module documentation). The caller is expected to have run
/// [`CreatePlate::validate_arguments`] for label, placement and the argument-level caps; this
/// function repeats the checks it depends on and never panics on any input.
pub fn evaluate_plate(authored: &AuthoredPlate) -> Result<PlateBody, CommandError> {
    let points = &authored.contour_points_mm;
    let raw: Vec<Point3> = points
        .iter()
        .map(|[x, y, z]| Point3::new(*x, *y, *z))
        .collect();
    let tolerance = beamcraft_geom::tolerance::Tolerance::for_plate(&raw)
        .map_err(|e| map_math(e, "contour_points_mm"))?;
    tolerance
        .check_dimension(authored.thickness_mm)
        .map_err(|e| map_math(e, "thickness_mm"))?;
    for (index, a) in raw.iter().enumerate() {
        let b = raw
            .get(index.saturating_add(1))
            .or_else(|| raw.first())
            .copied()
            .unwrap_or(*a);
        if tolerance
            .coincident(*a, b)
            .map_err(|e| map_math(e, "contour_points_mm"))?
        {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "contour_points_mm",
                "repeated consecutive contour points",
            ));
        }
    }
    match classify(points) {
        Contour::OriginRectangle { width, length } => {
            let result = rectangular_plate(RectangularPlate {
                width_mm: width,
                length_mm: length,
                thickness_mm: authored.thickness_mm,
            })
            .map_err(|f| map_kernel(&f))?;
            let solid = if authored.holes_mm.is_empty() {
                SolidMesh::from_box(&result)
            } else {
                let body = Body::from_plate(&result).map_err(|f| map_operation(&f))?;
                let applied = cut_holes(body, authored.thickness_mm, &authored.holes_mm)?;
                SolidMesh::from_operation(&applied)
            };
            Ok(PlateBody {
                solid,
                volume_algorithm: VOLUME_ALGORITHM,
                // The same product the G02 box reports as its analytic volume.
                volume_gross_mm3: result.volume_mm3(),
                width_mm: width,
                length_mm: length,
                thickness_mm: authored.thickness_mm,
                hole_count: authored.holes_mm.len(),
            })
        }
        Contour::Other => evaluate_contour_plate(authored),
    }
}

/// Evaluates an arbitrary closed outline (A02, experimental): the outline in the plate-local XY
/// plane, the G05 admission and extrusion along `+Z` by the thickness, the G06 certification and
/// the A04 through holes, exactly as on the rectangle.
///
/// The analytic volume of the plain outline is `net_area * thickness`; with holes it is the
/// midpoint of the G06 enclosure of the certified cut result, like on the rectangle. A contour
/// that touches any nonzero `z` is valid F04 input in its own body frame, but this slice does
/// not evaluate it (`UNSUPPORTED_GEOMETRY`, as in the first slice, so no valid input gains a
/// new refusal class): the plate body frame is the local XY plane and the placement carries
/// translations, and F04's planarity tolerance is the G06 policy, not an argument check here.
fn evaluate_contour_plate(authored: &AuthoredPlate) -> Result<PlateBody, CommandError> {
    if authored.contour_points_mm.iter().any(|[_, _, z]| *z != 0.0) {
        return Err(CommandError::new(
            ErrorCode::UnsupportedGeometry,
            "contour_points_mm",
            "a contour off the plate's local XY plane is not evaluated in this slice; a translation belongs to the placement",
        ));
    }
    let outer: Vec<Point2> = authored
        .contour_points_mm
        .iter()
        .map(|[x, y, _]| Point2::new(*x, *y))
        .collect();
    let extrusion = extrude(&outer, &[], authored.thickness_mm)
        .map_err(|f| map_profile(&f, "contour_points_mm"))?;
    let solid = if authored.holes_mm.is_empty() {
        // The G06 certification of the same shell; the stored bytes are the extrusion's own.
        Body::from_extrusion(&extrusion).map_err(map_body)?;
        SolidMesh::from_extrusion(&extrusion)
    } else {
        let body = Body::from_extrusion(&extrusion).map_err(map_body)?;
        let applied = cut_holes(body, authored.thickness_mm, &authored.holes_mm)?;
        SolidMesh::from_operation(&applied)
    };
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for p in &outer {
        lo = [lo[0].min(p.x), lo[1].min(p.y)];
        hi = [hi[0].max(p.x), hi[1].max(p.y)];
    }
    Ok(PlateBody {
        solid,
        volume_algorithm: CONTOUR_VOLUME_ALGORITHM,
        // The exact polygon area times the thickness: the same value the plain mesh reports.
        volume_gross_mm3: extrusion.volume_mm3(),
        width_mm: hi[0] - lo[0],
        length_mm: hi[1] - lo[1],
        thickness_mm: authored.thickness_mm,
        hole_count: authored.holes_mm.len(),
    })
}

/// Millimetres by which a hole cutter reaches past both faces of the plate.
const CUTTER_OVERSHOOT_MM: f64 = 1.0;

fn map_operation(failure: &OpFailure) -> CommandError {
    let (code, message) = match failure.code {
        OpCode::ContourInvalid | OpCode::UnsupportedContour => (
            ErrorCode::InvalidArgument,
            "a hole is not a valid clockwise polygon strictly inside the outline and clear of the others",
        ),
        other => (other.f04_code(), "hole evaluation failed"),
    };
    CommandError::new(code, "holes_mm", message)
}

/// Maps the G06 body admission of the extruded outline. It is a fault of the outline, never of
/// a hole, so the field names `contour_points_mm` and not `holes_mm`.
fn map_body(failure: OpFailure) -> CommandError {
    CommandError::new(
        failure.code.f04_code(),
        "contour_points_mm",
        "the extruded outline could not be certified as one closed solid",
    )
}

/// Why the G05 outline admission refused a contour, in the kernel's own words.
fn contour_message(defect: Defect) -> &'static str {
    match defect {
        Defect::TooFewPoints => "the outline has fewer than three points",
        Defect::EdgeTooShort => "an outline edge is below the geometric floor",
        Defect::ZeroArea => "the outline has zero area",
        Defect::AreaTooSmall => "the outline area is below the geometric floor",
        Defect::WrongWinding => {
            "the outline is clockwise; it must be counter-clockwise seen from +Z"
        }
        Defect::SelfIntersection => "the outline intersects itself",
        Defect::Backtracking => "two adjacent outline edges fold back onto each other",
        Defect::NarrowFeature => "two non-adjacent outline edges are closer than the floor",
        _ => "the outline is not an admissible closed contour",
    }
}

/// Maps the G05 outline admission of a contour plate to a command error, keeping the kernel's
/// own diagnosis: the field names the refused quantity and the message says why.
fn map_profile(failure: &ProfileFailure, field: &'static str) -> CommandError {
    let Some(diagnostic) = failure.diagnostics().first() else {
        return CommandError::new(
            ErrorCode::UnsupportedGeometry,
            field,
            "the outline could not be evaluated",
        );
    };
    let (field, message) = match diagnostic.field {
        ProfileField::Thickness => (
            "thickness_mm",
            "the thickness is not above the outline's geometric floor",
        ),
        _ => (field, contour_message(diagnostic.defect)),
    };
    CommandError::new(diagnostic.code.f04_code(), field, message)
}

/// The certified mesh of a plate with its through holes. Each hole is one kernel cutter, in
/// order, so the wall ids of the first hole are `h0`. The caller supplies the certified body of
/// the uncut plate (the G02 box or the A02 contour extrusion).
///
/// The kernel decides, with exact predicates, whether a hole lies strictly inside the outline,
/// is clear of the others and is a clockwise simple polygon; a refusal is `INVALID_ARGUMENT`
/// for a bad polygon and the kernel's own code otherwise, and nothing is applied.
fn cut_holes(
    mut body: Body,
    thickness_mm: f64,
    holes: &[Vec<[f64; 2]>],
) -> Result<OpResult, CommandError> {
    let mut last = None;
    for (ordinal, hole) in holes.iter().enumerate() {
        let cavity = Cavity::polygon(hole.iter().map(|[x, y]| Point2::new(*x, *y)).collect());
        let ordinal = u16::try_from(ordinal).map_err(|_| {
            CommandError::new(ErrorCode::LimitExceeded, "holes_mm", "too many holes")
        })?;
        let cutter = HoleCutter::new(
            cavity,
            -CUTTER_OVERSHOOT_MM,
            thickness_mm + CUTTER_OVERSHOOT_MM,
            ordinal,
        );
        let applied = through_hole(&body, &cutter).map_err(|f| map_operation(&f))?;
        body = applied.body().clone();
        last = Some(applied);
    }
    last.ok_or_else(|| {
        CommandError::new(
            ErrorCode::UnsupportedGeometry,
            "holes_mm",
            "hole evaluation produced no result",
        )
    })
}

/// The contour of the origin rectangle of F04-POS-01: `(0,0,0), (w,0,0), (w,l,0), (0,l,0)`,
/// counter-clockwise seen from +Z. This is the one contour the kernel evaluates today, so a
/// dimension-based front end (the CLI's `--width --length`) builds it here instead of inventing
/// its own.
///
/// A non-finite dimension is `NON_FINITE_NUMBER`; zero or negative is `INVALID_ARGUMENT`. Caps
/// and the geometric floor are checked by [`create_plate`].
pub fn origin_rectangle_contour(
    width_mm: f64,
    length_mm: f64,
) -> Result<Vec<[f64; 3]>, CommandError> {
    if !width_mm.is_finite() || !length_mm.is_finite() {
        return Err(CommandError::new(
            ErrorCode::NonFiniteNumber,
            "contour_points_mm",
            "value is not finite",
        ));
    }
    if width_mm <= 0.0 || length_mm <= 0.0 {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "contour_points_mm",
            "dimension is not positive",
        ));
    }
    Ok(vec![
        [0.0, 0.0, 0.0],
        [width_mm, 0.0, 0.0],
        [width_mm, length_mm, 0.0],
        [0.0, length_mm, 0.0],
    ])
}

/// Applies `CreatePlate` to an isolated draft in memory: argument admission, engineering
/// validation, kernel evaluation, quantities and the declared footprint. Nothing is published.
///
/// `existing` is the set of element IDs already owned by the draft; a creation ID in it is
/// `DUPLICATE_ID`, checked after argument admission and before geometry as F04 section 8 orders.
/// A pinned material is resolved next (`materials`), so a missing definition is `NOT_FOUND`
/// before any geometry runs and its read is part of the footprint.
/// A placement rotation other than the identity is `UNSUPPORTED_GEOMETRY` (module documentation).
pub fn create_plate(
    command: &CreatePlate,
    existing: &dyn Fn(&ElementId) -> bool,
    materials: MaterialLookup<'_>,
) -> Result<CreatePlateOutcome, CommandError> {
    command.validate_arguments()?;
    if existing(&command.new_id) {
        return Err(CommandError::new(
            ErrorCode::DuplicateId,
            "new_id",
            "element id is already owned",
        ));
    }
    let material = command
        .material_digest
        .as_ref()
        .map(materials)
        .transpose()?;
    let footprint = Footprint::for_create(command.new_id, command.material_digest)?;
    let authored = AuthoredPlate::from_command(command);
    let body = evaluate_plate(&authored)?;
    if !command.placement.is_identity_rotation() {
        return Err(CommandError::new(
            ErrorCode::UnsupportedGeometry,
            "placement.orientation_xyzw",
            "rotated placement is not evaluated in this slice",
        ));
    }
    let quantities = body.quantities_for(material)?;
    Ok(CreatePlateOutcome {
        fingerprint: command.content_fingerprint()?,
        authored,
        body,
        quantities,
        footprint,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(contour: Vec<[f64; 3]>, thickness: f64) -> CreatePlate {
        CreatePlate {
            new_id: ElementId::parse("019a7bf0-0000-7000-8000-000000000005").unwrap(),
            name: "Synthetic plate".into(),
            contour_points_mm: contour,
            thickness_mm: thickness,
            placement: Placement::IDENTITY,
            material_digest: None,
            holes_mm: Vec::new(),
        }
    }

    fn rect(w: f64, l: f64) -> Vec<[f64; 3]> {
        vec![[0.0, 0.0, 0.0], [w, 0.0, 0.0], [w, l, 0.0], [0.0, l, 0.0]]
    }

    fn none(_: &ElementId) -> bool {
        false
    }

    // Shadows the glob import: these tests have no material definitions unless they say so.
    fn create_plate(
        command: &CreatePlate,
        existing: &dyn Fn(&ElementId) -> bool,
    ) -> Result<CreatePlateOutcome, CommandError> {
        super::create_plate(command, existing, &no_materials)
    }

    fn digest(text: &[u8]) -> ContentDigest {
        beamcraft_model::digest::Sha256::digest(text).unwrap()
    }

    #[test]
    fn a_pinned_material_with_density_gives_a_current_mass_and_is_in_the_footprint() {
        let known = digest(b"steel");
        let lookup = |d: &ContentDigest| {
            if *d == known {
                Ok(MaterialInfo {
                    density_kg_m3: Some(1000.0),
                })
            } else {
                no_materials(d)
            }
        };
        let mut c = command(rect(1000.0, 500.0), 10.0);
        c.material_digest = Some(known);
        let out = super::create_plate(&c, &none, &lookup).unwrap();
        // F04 section 5 analytic example: 0.005 m3 at 1000 kg/m3 is 5 kg.
        assert_eq!(out.quantities.mass_status, QuantityStatus::Current);
        assert_eq!(out.quantities.density_kg_m3, Some(1000.0));
        assert!((out.quantities.mass_kg.unwrap() - 5.0).abs() <= 1e-9);
        assert_eq!(out.authored.material, Some(known));
        assert_eq!(out.authored.to_command(), c);
        assert!(out.footprint.reads().iter().any(|e| {
            e.target == crate::footprint::Target::Definition(known)
                && e.group == crate::footprint::Group::Relationship
        }));
    }

    #[test]
    fn unknown_density_stays_unknown_and_a_missing_definition_is_not_found() {
        let known = digest(b"no-density");
        let lookup = |d: &ContentDigest| {
            if *d == known {
                Ok(MaterialInfo {
                    density_kg_m3: None,
                })
            } else {
                no_materials(d)
            }
        };
        let mut c = command(rect(1000.0, 500.0), 10.0);
        c.material_digest = Some(known);
        let out = super::create_plate(&c, &none, &lookup).unwrap();
        assert_eq!(out.quantities.mass_status, QuantityStatus::Unknown);
        assert_eq!(out.quantities.mass_kg, None);
        assert_eq!(out.quantities.density_kg_m3, None);
        c.material_digest = Some(digest(b"absent"));
        assert_eq!(code(&c), ErrorCode::NotFound);
        // The definition is resolved before geometry runs.
        c.thickness_mm = 0.0;
        assert_eq!(code(&c), ErrorCode::NotFound);
    }

    #[test]
    fn a_committed_density_is_admitted_again_before_it_makes_a_mass() {
        let body = evaluate_plate(&AuthoredPlate::from_command(&command(
            rect(1000.0, 500.0),
            10.0,
        )))
        .unwrap();
        for (density, expected) in [
            (f64::NAN, ErrorCode::NonFiniteNumber),
            (0.0, ErrorCode::InvalidArgument),
            (-7850.0, ErrorCode::InvalidArgument),
            (2_000_000.0, ErrorCode::LimitExceeded),
        ] {
            let error = body
                .quantities_for(Some(MaterialInfo {
                    density_kg_m3: Some(density),
                }))
                .unwrap_err();
            assert_eq!(error.code, expected, "{density}");
        }
        assert_eq!(
            body.quantities_for(None).unwrap().mass_status,
            QuantityStatus::Unknown
        );
    }

    fn code(c: &CreatePlate) -> ErrorCode {
        create_plate(c, &none).unwrap_err().code
    }

    #[test]
    fn the_first_scope_plate_is_created_with_independent_volume() {
        let out = create_plate(&command(rect(1000.0, 500.0), 10.0), &none).unwrap();
        // Independent closed form: 1000 * 500 * 10 mm3 = 5,000,000 mm3 = 0.005 m3 (F04 section 5).
        assert_eq!(out.quantities.volume_mm3, 5_000_000.0);
        assert!((out.quantities.volume_m3 - 0.005).abs() <= 1e-6 * 0.005);
        assert_eq!(out.quantities.volume_status, QuantityStatus::Current);
        assert_eq!(out.quantities.mass_status, QuantityStatus::Unknown);
        assert_eq!(out.body.width_mm, 1000.0);
        assert_eq!(out.body.length_mm, 500.0);
        assert_eq!(out.authored.thickness_mm, 10.0);
        assert!(!out.footprint.writes().is_empty());
    }

    #[test]
    fn rejections_carry_the_f04_codes_of_the_cases() {
        let ok = rect(1000.0, 500.0);
        assert_eq!(code(&command(ok.clone(), 0.0)), ErrorCode::InvalidArgument); // F04-NEG-04
        assert_eq!(code(&command(ok.clone(), -1.0)), ErrorCode::InvalidArgument); // (negative thickness; the UpdateElement case F04-NEG-14 is not run)
        assert_eq!(
            code(&command(rect(1_000_001.0, 500.0), 10.0)),
            ErrorCode::LimitExceeded
        ); // F04-NEG-05
        assert_eq!(
            code(&command(rect(1_000_000.001, 500.0), 10.0)),
            ErrorCode::LimitExceeded
        ); // F04-NEG-19
        assert_eq!(
            code(&command(ok.clone(), 0.0009)),
            ErrorCode::InvalidArgument
        ); // F04-NEG-27
        assert_eq!(
            code(&command(vec![[0.0; 3], [1000.0, 0.0, 0.0]], 10.0)),
            ErrorCode::InvalidArgument
        ); // F04-NEG-31
        let repeated = vec![
            [0.0, 0.0, 0.0],
            [1000.0, 0.0, 0.0],
            [1000.0, 0.0, 0.0],
            [1000.0, 500.0, 0.0],
            [0.0, 500.0, 0.0],
        ];
        assert_eq!(code(&command(repeated, 10.0)), ErrorCode::InvalidArgument); // F04-NEG-32
        let clockwise = vec![
            [0.0, 0.0, 0.0],
            [0.0, 500.0, 0.0],
            [1000.0, 500.0, 0.0],
            [1000.0, 0.0, 0.0],
        ];
        assert_eq!(code(&command(clockwise, 10.0)), ErrorCode::InvalidArgument); // F04-NEG-28
        let wide = vec![
            [0.0, 0.0, 0.0],
            [750_000.0, 0.0, 0.0],
            [1_500_000.0, 0.0, 0.0],
            [2_250_000.0, 0.0, 0.0],
            [2_250_000.0, 1000.0, 0.0],
            [1_500_000.0, 1000.0, 0.0],
            [750_000.0, 1000.0, 0.0],
            [0.0, 1000.0, 0.0],
        ];
        assert_eq!(code(&command(wide, 10.0)), ErrorCode::LimitExceeded); // F04-NEG-35
    }

    #[test]
    fn through_holes_are_cut_by_the_kernel_with_exact_volume_and_mass() {
        // Clockwise seen from +Z, as the kernel's cutter requires.
        let square = |x: f64, y: f64, s: f64| vec![[x, y], [x, y + s], [x + s, y + s], [x + s, y]];
        let mut c = command(rect(1000.0, 500.0), 10.0);
        c.holes_mm = vec![square(100.0, 100.0, 100.0), square(400.0, 200.0, 50.0)];
        let out = create_plate(&c, &none).unwrap();
        // (1000 * 500 - 100 * 100 - 50 * 50) * 10 = 4,875,000 mm3, closed form.
        assert!((out.quantities.volume_mm3 - 4_875_000.0).abs() <= 1e-6);
        assert_eq!(out.quantities.algorithm, HOLED_VOLUME_ALGORITHM);
        assert_eq!(out.body.hole_count, 2);
        assert_eq!(out.authored.holes_mm, c.holes_mm);
        assert_eq!(out.authored.to_command(), c);
        let solid = &out.body.solid;
        // Cap loops plus wall quads: far more than the 12 triangles of the plain box.
        assert!(solid.triangles().len() > 12);
        let plain = create_plate(&command(rect(1000.0, 500.0), 10.0), &none).unwrap();
        assert_ne!(solid.digest(), plain.body.solid.digest());
        assert_eq!(plain.body.solid.triangles().len(), 12);
        // The mesh volume agrees with the divergence theorem over the returned triangles.
        let v = solid.vertices();
        let signed: f64 = solid
            .triangles()
            .iter()
            .map(|[a, b, c]| {
                let (a, b, c) = (v[*a as usize], v[*b as usize], v[*c as usize]);
                let cross = [
                    b[1] * c[2] - b[2] * c[1],
                    b[2] * c[0] - b[0] * c[2],
                    b[0] * c[1] - b[1] * c[0],
                ];
                (a[0] * cross[0] + a[1] * cross[1] + a[2] * cross[2]) / 6.0
            })
            .sum();
        assert!((signed - 4_875_000.0).abs() <= 1e-3, "{signed}");
        // The hole does not change the box the dimensions come from.
        assert_eq!(out.body.width_mm, 1000.0);
        assert_eq!(plain.body.solid.volume_mm3(), 5_000_000.0);
        // Gross is the outline area times the thickness, exact and without holes; net is the
        // kernel's result; the removed volume is the difference of the two meshes.
        let q = out.quantities;
        assert_eq!(q.volume_gross_mm3, 5_000_000.0);
        assert!((q.volume_mm3 - 4_875_000.0).abs() <= 1e-6);
        assert!((q.removed_volume_mm3 - 125_000.0).abs() <= 1e-6);
        assert!((q.volume_gross_mm3 - q.removed_volume_mm3 - q.volume_mm3).abs() <= 1e-6);
        assert_eq!(
            plain.quantities.volume_gross_mm3,
            plain.quantities.volume_mm3
        );
        assert_eq!(plain.quantities.removed_volume_mm3, 0.0);
        assert_eq!(q.area_source, None);
    }

    #[test]
    fn gross_and_net_mass_follow_the_density() {
        let square = |x: f64, y: f64, s: f64| vec![[x, y], [x, y + s], [x + s, y + s], [x + s, y]];
        let mut c = command(rect(1000.0, 500.0), 10.0);
        c.holes_mm = vec![square(100.0, 100.0, 100.0), square(400.0, 200.0, 50.0)];
        let body = evaluate_plate(&AuthoredPlate::from_command(&c)).unwrap();
        let q = body
            .quantities_for(Some(MaterialInfo {
                density_kg_m3: Some(7850.0),
            }))
            .unwrap();
        // 5e6 mm3 = 0.005 m3 -> 39.25 kg gross; 4.875e6 mm3 -> 38.26875 kg net.
        assert!((q.mass_gross_kg.unwrap() - 39.25).abs() <= 1e-9);
        assert!((q.mass_kg.unwrap() - 38.268_75).abs() <= 1e-6);
        assert_eq!(q.mass_status, QuantityStatus::Current);
        let none = body.quantities_for(None).unwrap();
        assert_eq!((none.mass_kg, none.mass_gross_kg), (None, None));
        assert_eq!(none.volume_gross_mm3, 5_000_000.0);
    }

    #[test]
    fn a_hole_the_kernel_refuses_applies_nothing() {
        let square = |x: f64, y: f64, s: f64| vec![[x, y], [x, y + s], [x + s, y + s], [x + s, y]];
        let code_of = |holes: Vec<Vec<[f64; 2]>>| {
            let mut c = command(rect(1000.0, 500.0), 10.0);
            c.holes_mm = holes;
            create_plate(&c, &none).unwrap_err().code
        };
        // Counter-clockwise is not a cutter.
        let mut ccw = square(100.0, 100.0, 100.0);
        ccw.reverse();
        assert_eq!(code_of(vec![ccw]), ErrorCode::InvalidArgument);
        // Outside, crossing and touching the outline.
        assert_eq!(
            code_of(vec![square(2000.0, 100.0, 100.0)]),
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            code_of(vec![square(950.0, 100.0, 100.0)]),
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            code_of(vec![square(0.0, 100.0, 100.0)]),
            ErrorCode::InvalidArgument
        );
        // Overlapping another hole.
        assert_eq!(
            code_of(vec![
                square(100.0, 100.0, 100.0),
                square(150.0, 150.0, 100.0)
            ]),
            ErrorCode::InvalidArgument
        );
        // Argument-level failures come first.
        assert_eq!(
            code_of(vec![vec![[1.0, 1.0], [2.0, 2.0]]]),
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn the_inclusive_floor_is_the_scaled_geometric_minimum() {
        // F04-POS-17: 0.001 mm is exactly the absolute floor, so it is accepted.
        let out = create_plate(&command(rect(1000.0, 500.0), 0.001), &none).unwrap();
        assert!((out.quantities.volume_m3 - 5e-7).abs() <= 1e-6 * 5e-7);
        // F04-POS-27: the 3 x 4 plate has D = 5, so 5 * 2^-22 is far below 0.001.
        assert!(create_plate(&command(rect(3.0, 4.0), 0.001), &none).is_ok());
        // F04-POS-08: the largest first-scope plate.
        let big = create_plate(&command(rect(1e6, 1e6), 1e6), &none).unwrap();
        assert!((big.quantities.volume_m3 - 1e9).abs() <= 1e-6 * 1e9);
    }

    #[test]
    fn an_arbitrary_closed_outline_is_evaluated_with_its_exact_polygon_volume() {
        // A right triangle: area 300 * 400 / 2 = 60,000 mm2, times 10 mm = 600,000 mm3.
        let triangle = vec![[0.0, 0.0, 0.0], [300.0, 0.0, 0.0], [0.0, 400.0, 0.0]];
        let out = create_plate(&command(triangle.clone(), 10.0), &none).unwrap();
        assert!(
            (out.quantities.volume_mm3 - 600_000.0).abs() <= 1e-9,
            "{out:?}"
        );
        assert_eq!(out.quantities.algorithm, CONTOUR_VOLUME_ALGORITHM);
        assert_eq!(out.body.width_mm, 300.0);
        assert_eq!(out.body.length_mm, 400.0);
        // The mesh is a certified solid, not a box: two cap triangles and three quad walls.
        assert_eq!(out.body.solid.triangles().len(), 8);
        // The former F04-POS-20 pentagon is now evaluated, not guessed; the off-plane
        // rectangle of F04-POS-18 stays unsupported (refused in the rejection test below).
        let pentagon = vec![
            [0.0, 0.0, 0.0],
            [1000.0, 0.0, 0.0],
            [1200.0, 250.0, 0.0],
            [500.0, 600.0, 0.0],
            [0.0, 500.0, 0.0],
        ];
        let out = create_plate(&command(pentagon, 10.0), &none).unwrap();
        assert_eq!(out.quantities.algorithm, CONTOUR_VOLUME_ALGORITHM);
        assert_eq!(out.body.width_mm, 1200.0);
        assert_eq!(out.body.length_mm, 600.0);
    }

    #[test]
    fn the_origin_rectangle_is_recognised_at_any_cyclic_start() {
        let canonical = create_plate(&command(rect(1000.0, 500.0), 10.0), &none).unwrap();
        // The same closed rectangle listed from its second, third and fourth corner: the
        // rotation changes no geometry, so the G02 evaluation, algorithm label and mesh digest
        // stay the canonical ones.
        for shift in 1..4 {
            let mut rotated = rect(1000.0, 500.0);
            rotated.rotate_left(shift);
            let out = create_plate(&command(rotated, 10.0), &none).unwrap();
            assert_eq!(out.quantities.algorithm, VOLUME_ALGORITHM);
            assert_eq!(out.quantities.volume_mm3, canonical.quantities.volume_mm3);
            assert_eq!(out.body.solid.digest(), canonical.body.solid.digest());
            assert_eq!(out.body.solid.triangles().len(), 12);
        }
        // A translated rectangle is a different authored outline, so it stays on the contour
        // path: its mesh must match the contour as authored, and the placement is the vehicle
        // for translations.
        let moved = vec![
            [10.0, 20.0, 0.0],
            [1010.0, 20.0, 0.0],
            [1010.0, 520.0, 0.0],
            [10.0, 520.0, 0.0],
        ];
        let out = create_plate(&command(moved, 10.0), &none).unwrap();
        assert_eq!(out.quantities.algorithm, CONTOUR_VOLUME_ALGORITHM);
        assert_eq!(out.body.solid.triangles().len(), 12);
    }

    #[test]
    fn contour_rejections_use_the_f04_codes() {
        let code_of = |contour: Vec<[f64; 3]>| {
            create_plate(&command(contour, 10.0), &none)
                .unwrap_err()
                .code
        };
        // A clockwise outline is refused, the mirror rectangle included (F04-NEG-28).
        let clockwise = vec![
            [0.0, 0.0, 0.0],
            [0.0, 500.0, 0.0],
            [1000.0, 500.0, 0.0],
            [1000.0, 0.0, 0.0],
        ];
        assert_eq!(code_of(clockwise), ErrorCode::InvalidArgument);
        // A self-intersecting bowtie is refused.
        assert_eq!(
            code_of(vec![
                [0.0, 0.0, 0.0],
                [1000.0, 500.0, 0.0],
                [1000.0, 0.0, 0.0],
                [0.0, 500.0, 0.0],
            ]),
            ErrorCode::InvalidArgument
        );
        // Contour points off the local XY plane are valid F04 input in its own body frame, but
        // this slice does not evaluate them: `UNSUPPORTED_GEOMETRY`, as before A02 (the former
        // F04-POS-18 shape, and the non-planar F04-NEG-29 shape, whose exact F04 refusal code
        // needs the G06 planarity tolerance). A translation belongs to the placement.
        for off_plane in [
            vec![
                [0.0, 0.0, 5.0],
                [1000.0, 0.0, 5.0],
                [1000.0, 500.0, 5.0],
                [0.0, 500.0, 5.0],
            ],
            vec![
                [0.0, 0.0, 0.0],
                [1000.0, 0.0, 0.0],
                [1000.0, 500.0, 20.0],
                [0.0, 500.0, 0.0],
            ],
        ] {
            assert_eq!(code_of(off_plane), ErrorCode::UnsupportedGeometry);
        }
        // Fewer than three points stays INVALID_ARGUMENT (F04-NEG-31) on the contour path too.
        assert_eq!(
            code_of(vec![[0.0, 0.0, 0.0], [1000.0, 0.0, 0.0]]),
            ErrorCode::InvalidArgument
        );
        // More points than the model cap are refused before any traversal.
        assert_eq!(
            code_of(vec![[0.0, 0.0, 0.0]; 1025]),
            ErrorCode::LimitExceeded
        );
    }

    #[test]
    fn a_contour_plate_takes_through_holes_with_the_exact_volume() {
        let triangle = vec![[0.0, 0.0, 0.0], [300.0, 0.0, 0.0], [0.0, 400.0, 0.0]];
        let hole = vec![[50.0, 50.0], [50.0, 100.0], [100.0, 100.0], [100.0, 50.0]];
        let mut c = command(triangle, 10.0);
        c.holes_mm = vec![hole];
        let out = create_plate(&c, &none).unwrap();
        // (60,000 - 50 * 50) mm2, times 10 mm = 575,000 mm3.
        assert!(
            (out.quantities.volume_mm3 - 575_000.0).abs() <= 1e-6,
            "{out:?}"
        );
        assert_eq!(out.quantities.algorithm, HOLED_VOLUME_ALGORITHM);
        assert_eq!(out.body.hole_count, 1);
        // A hole the kernel refuses (outside the triangle) applies nothing.
        let outside = vec![
            [200.0, 200.0],
            [200.0, 300.0],
            [300.0, 300.0],
            [300.0, 200.0],
        ];
        let mut c = command(
            vec![[0.0, 0.0, 0.0], [300.0, 0.0, 0.0], [0.0, 400.0, 0.0]],
            10.0,
        );
        c.holes_mm = vec![outside];
        assert_eq!(
            create_plate(&c, &none).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn argument_errors_come_before_geometry() {
        let mut c = command(rect(1000.0, 500.0), 10.0);
        c.placement.orientation_xyzw = [0.0, 0.0, 0.0, 0.5];
        assert_eq!(code(&c), ErrorCode::InvalidArgument); // F04-NEG-22
        c.placement.orientation_xyzw = [0.0, 0.0, 0.0, 0.99999995];
        assert!(create_plate(&c, &none).is_ok()); // F04-POS-11
        c.placement.orientation_xyzw = [0.0, 0.0, 0.0, -1.0];
        assert!(create_plate(&c, &none).is_ok()); // F04-POS-12
        c.placement.translation_mm = [1e9, 0.0, 0.0];
        assert!(create_plate(&c, &none).is_ok()); // F04-POS-09
        let s = core::f64::consts::FRAC_1_SQRT_2;
        c.placement.orientation_xyzw = [0.0, 0.0, s, s];
        assert_eq!(code(&c), ErrorCode::UnsupportedGeometry);
        let mut nan = command(rect(1000.0, 500.0), 10.0);
        nan.contour_points_mm[1][0] = f64::NAN;
        assert_eq!(code(&nan), ErrorCode::NonFiniteNumber);
    }

    #[test]
    fn an_owned_id_is_a_duplicate_before_any_geometry() {
        let c = command(rect(1000.0, 500.0), 10.0);
        let taken = |id: &ElementId| *id == c.new_id;
        assert_eq!(
            create_plate(&c, &taken).unwrap_err().code,
            ErrorCode::DuplicateId
        ); // F04-NEG-03
        let mut bad = command(rect(1000.0, 500.0), 0.0);
        bad.new_id = c.new_id;
        assert_eq!(
            create_plate(&bad, &taken).unwrap_err().code,
            ErrorCode::DuplicateId
        );
    }

    #[test]
    fn evaluation_never_panics_on_hostile_contours() {
        for contour in [
            vec![],
            vec![[0.0; 3]],
            vec![[f64::MAX; 3]; 4],
            vec![[f64::NAN; 3]; 4],
            vec![[1e300, 0.0, 0.0], [-1e300, 0.0, 0.0], [0.0, 1e300, 0.0]],
            vec![[0.0; 3]; 1024],
        ] {
            let authored = AuthoredPlate {
                id: ElementId::parse("019a7bf0-0000-7000-8000-000000000005").unwrap(),
                name: "x".into(),
                contour_points_mm: contour,
                thickness_mm: 10.0,
                placement: Placement::IDENTITY,
                material: None,
                holes_mm: Vec::new(),
            };
            assert!(evaluate_plate(&authored).is_err());
        }
    }

    #[test]
    fn the_dimension_front_end_builds_the_origin_rectangle() {
        let contour = origin_rectangle_contour(1000.0, 500.0).unwrap();
        assert_eq!(contour, rect(1000.0, 500.0));
        for (w, l, expected) in [
            (0.0, 5.0, ErrorCode::InvalidArgument),
            (5.0, -1.0, ErrorCode::InvalidArgument),
            (f64::NAN, 5.0, ErrorCode::NonFiniteNumber),
            (5.0, f64::INFINITY, ErrorCode::NonFiniteNumber),
        ] {
            assert_eq!(origin_rectangle_contour(w, l).unwrap_err().code, expected);
        }
    }

    #[test]
    fn the_same_arguments_fingerprint_the_same_and_round_trip_through_authored_state() {
        let c = command(rect(1000.0, 500.0), 10.0);
        let out = create_plate(&c, &none).unwrap();
        assert_eq!(out.authored.to_command(), c);
        assert_eq!(
            out.fingerprint,
            out.authored.to_command().content_fingerprint().unwrap()
        );
    }
}
