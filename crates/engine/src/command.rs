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
//! # What P01 supports, and what it does not
//!
//! F04 section 3 (P-1) admits any planar contour and evaluates a four-point rectangle in its own
//! body frame. Three paths evaluate a contour here:
//!
//! * The origin rectangle of F04-POS-01, `(0,0,0), (w,0,0), (w,l,0), (0,l,0)`, is the G02 box
//!   (its bytes and digests are unchanged since P01). Its clockwise mirror is `INVALID_ARGUMENT`.
//! * Every other contour in the plane `z = 0` is a G05 extrusion (A02): the kernel decides
//!   winding, self-intersection, touching and the local-size floor with exact predicates, so a
//!   clockwise or self-crossing outline is `INVALID_ARGUMENT` and any simple counter-clockwise
//!   polygon is evaluated as authored (nothing reordered, merged or repaired).
//!
//! * An exactly rectangular contour outside `z = 0` is evaluated in its ordered body frame,
//!   with canonical box vertices in element-local coordinates. Holes on this path are unsupported.
//!
//! Ordered 3D validity is checked before support: outside-band nonplanarity, clockwise winding,
//! intersections and touches are `INVALID_ARGUMENT`. Other contours outside `z = 0`, including
//! valid near-planar input, remain `UNSUPPORTED_GEOMETRY` in this evaluated-body API. Use
//! [`crate::plate_draft`] to retain supported authored-only states. Rigid placements are applied by the snapshot and export consumers; the body stays local.
//!
//! Through holes (A04) apply to the existing `z = 0` bodies. A box reports width and length in
//! its body frame; a G05 contour reports its element-local XY bounding-box extents.

use crate::footprint::Footprint;
use crate::solid::SolidMesh;
use beamcraft_geom::tolerance::TOLERANCE_POLICY_VERSION;
use beamcraft_kernel::diagnostic::Failure;
use beamcraft_kernel::measure::analytic_mass_kg;
use beamcraft_kernel::ops::{
    Body, CutPlane, HoleCutter, Keep, OpCode, OpFailure, OpResult, plane_cut, through_hole,
};
use beamcraft_kernel::profile::diagnostic::{ProfileFailure, ProfileField};
use beamcraft_kernel::profile::{Cavity, Point2, extrude};
use beamcraft_kernel::trimesh::TriMesh;
use beamcraft_kernel::{PlateResult, RectangularPlate, rectangular_plate};
use beamcraft_model::command::{
    AddHole, CommandError, CreatePlate, CutRecord, HoleRecord, MAX_HOLES, Placement, PlateCorner,
    PlateSide,
};
use beamcraft_model::definition::MaterialSpec;
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId};

/// Version label of the volume algorithm in a [`Quantities`] record.
pub const VOLUME_ALGORITHM: &str = "kernel-g02-rectangular-plate-volume/1;tolerance-policy-3";

/// Version label of the volume algorithm of a plate with a G05 contour and no hole: the analytic
/// `net_area * thickness` of the extruded simple polygon.
pub const CONTOUR_VOLUME_ALGORITHM: &str = "kernel-g05-contour-plate-volume/1;tolerance-policy-3";

/// Version label of the volume algorithm of a plate whose contour carries corner treatments
/// (A12.2): the analytic `net_area * thickness` of the derived outline, faceted to the reported
/// chord deviation, so an analytical area of the rounded shape is never claimed.
pub const TREATED_VOLUME_ALGORITHM: &str =
    "kernel-a12-corner-treatment-volume/1;tolerance-policy-3";

/// Version label of the volume algorithm of a plate with through holes: the midpoint of the G06
/// enclosure of the certified result of the G09 hole operation.
pub const HOLED_VOLUME_ALGORITHM: &str = "kernel-g09-through-hole-volume/1;tolerance-policy-3";

/// Version label of the volume algorithm of an element with a plane cut (A03): the midpoint of
/// the G06 enclosure of the certified result of the kernel plane cut, taken after any holes.
pub const CUT_VOLUME_ALGORITHM: &str = "kernel-a03-plane-cut-volume/1;tolerance-policy-3";

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
    /// Authored integer part class (P15); zero is the default.
    pub class: i32,
    /// Element-local contour as submitted, in millimetres.
    pub contour_points_mm: Vec<[f64; 3]>,
    /// Thickness in millimetres.
    pub thickness_mm: f64,
    /// Where the thickness is laid off from the contour plane (A12.1); absence reads as `top`.
    pub thickness_side: Option<PlateSide>,
    /// Corner treatments of authored contour vertices (A12.2); empty means every corner sharp.
    /// The contour keeps its sharp points; these are authored parameters, evaluated to the
    /// treated outline on every evaluation.
    pub corners: Vec<PlateCorner>,
    /// Rigid placement as submitted.
    pub placement: Placement,
    /// Pinned physical material definition; absence means the material and density are unknown.
    pub material: Option<ContentDigest>,
    /// Through-hole polygons in the plate's local XY, as authored. Empty means a solid plate.
    pub holes_mm: Vec<Vec<[f64; 2]>>,
    /// Through-hole operation records (A04), ordered by ordinal. The pilot's create-authored
    /// polygons above are deprecated attributes (A-2); every `AddHole` lands here.
    pub hole_records: Vec<HoleRecord>,
    /// The owned `plane_cut` operation records (A03, amendment A-2), replayed in ascending
    /// ordinal order after every hole. Empty means an uncut element.
    pub cuts: Vec<CutRecord>,
}

impl AuthoredPlate {
    /// The authored fields of an accepted command (the command's arguments, unchanged).
    pub fn from_command(command: &CreatePlate) -> Self {
        Self {
            id: command.new_id,
            name: command.name.clone(),
            class: command.class,
            contour_points_mm: command.contour_points_mm.clone(),
            thickness_mm: command.thickness_mm,
            thickness_side: command.thickness_side,
            corners: command.corners.clone(),
            placement: command.placement,
            material: command.material_digest,
            holes_mm: command.holes_mm.clone(),
            hole_records: Vec::new(),
            cuts: Vec::new(),
        }
    }

    /// The `CreatePlate` that would author exactly this state; used to compare content. A
    /// drafted plate never carries operation records (they are refused on unsupported
    /// geometry), so none are carried back.
    pub fn to_command(&self) -> CreatePlate {
        CreatePlate {
            new_id: self.id,
            name: self.name.clone(),
            class: self.class,
            contour_points_mm: self.contour_points_mm.clone(),
            thickness_mm: self.thickness_mm,
            thickness_side: self.thickness_side,
            corners: self.corners.clone(),
            placement: self.placement,
            material_digest: self.material,
            holes_mm: self.holes_mm.clone(),
        }
    }

    /// Every hole polygon in replay order: the create-authored polygons of the deprecated A04
    /// pilot attributes first, then the operation records in ordinal order (F04 section 4).
    /// The plane cut, when present, applies after all of them.
    pub fn hole_polygons_in_replay_order(&self) -> Vec<Vec<[f64; 2]>> {
        let mut holes = self.holes_mm.clone();
        holes.extend(self.hole_records.iter().map(|r| r.polygon_mm.clone()));
        holes
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
    /// The sealed canonical box of an origin or framed rectangle, before any operation.
    /// Existing non-origin contours in `z = 0` retain their G05 extrusion and have none.
    pub result: Option<PlateResult>,
    /// The certified mesh of the plate as authored: the body, or the body with its through holes.
    pub solid: SolidMesh,
    /// Volume of the plate before any hole, in cubic millimetres: the exact box, or the analytic
    /// `net_area * thickness` of the contour polygon.
    pub gross_volume_mm3: f64,
    /// Width along body X for a box; element-local X bounding-box extent for a G05 contour.
    pub width_mm: f64,
    /// Length along body Y for a box; element-local Y bounding-box extent for a G05 contour.
    pub length_mm: f64,
    /// Thickness along body Z, in millimetres.
    pub thickness_mm: f64,
    /// Number of through holes applied.
    pub hole_count: usize,
    /// Whether the authored plane cut was applied after the holes (A03).
    pub cut_applied: bool,
    /// Whether a corner treatment was applied to the authored contour (A12.2). A treated plate
    /// never takes the canonical box path, even when the sharp contour is the origin rectangle.
    pub corner_treated: bool,
    /// The evaluated outline of a G05 contour plate in the element-local plane `z = 0`: the
    /// treated outline when corners carry treatments (A12.2), the authored contour otherwise.
    /// Empty for a canonical rectangular box. Numbering compares this outline, so two plates
    /// whose geometry differs share no part number.
    pub outline_mm: Vec<[f64; 2]>,
    /// The largest chord deviation any faceted arc of a treated outline keeps, in millimetres
    /// (A12.2); `0.0` when no corner is rounded. The quantities are those of the faceted body.
    pub arc_deviation_mm: f64,
}

impl PlateBody {
    /// The algorithm label of this body's volume record.
    pub const fn algorithm(&self) -> &'static str {
        if self.cut_applied {
            CUT_VOLUME_ALGORITHM
        } else if self.hole_count != 0 {
            HOLED_VOLUME_ALGORITHM
        } else if self.result.is_some() {
            VOLUME_ALGORITHM
        } else if self.corner_treated {
            TREATED_VOLUME_ALGORITHM
        } else {
            CONTOUR_VOLUME_ALGORITHM
        }
    }

    /// Whether the outline uses a canonical rectangular box rather than a G05 contour.
    pub const fn is_rectangular_box(&self) -> bool {
        self.result.is_some()
    }

    fn volumes(&self) -> Volumes {
        // The gross body is the exact outline extrusion (the box or the polygon prism), so the
        // gross volume is its analytic area times the thickness and equals the mesh before the
        // holes and the cut; the operations remove what the two meshes differ by, so the net
        // volume of a cut or holed body is its own certified mesh volume.
        Volumes::measured(
            self.gross_volume_mm3,
            self.gross_volume_mm3,
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

/// The element-local Z of a plate's lower face (A12.1): the authored side laid off from the
/// contour plane; absence is the `top` default.
fn side_offset(authored: &AuthoredPlate) -> f64 {
    authored
        .thickness_side
        .map_or(0.0, |side| side.offset_mm(authored.thickness_mm))
}

/// The certified plain body of a plate laid off by `offset_mm` along the element-local Z
/// (A12.1 thickness side), with its mesh for the base result: the kernel blob is translated
/// by the pure translation of the production rigid map and re-certified with the face table,
/// normals and tolerance of the body it certifies. `None` for a zero offset, so the default
/// `top` side keeps the kernel results bit-identical to A02. The offset is the authored
/// thickness halved or negated, both exact, so every vertex moves by exactly that amount and
/// winding, adjacency, the face table and the measured volume are preserved; a refusal is
/// therefore `UNSUPPORTED_GEOMETRY`, field `thickness_side`, and never an input rejection.
fn laid_off(
    blob: &[u8],
    face_table: &[String],
    face_normals: &[Option<[f64; 3]>],
    tolerance: beamcraft_geom::tolerance::Tolerance,
    volume_mm3: f64,
    offset_mm: f64,
) -> Result<Option<(Body, SolidMesh)>, CommandError> {
    if offset_mm == 0.0 {
        return Ok(None);
    }
    use beamcraft_geom::{Frame3, Point3, RigidTransform, Vec3};
    let refused = || {
        CommandError::new(
            ErrorCode::UnsupportedGeometry,
            "thickness_side",
            "the laid-off plate body could not be certified",
        )
    };
    let plain = TriMesh::from_blob(blob).map_err(|_| refused())?;
    let axis = |v: [f64; 3]| Vec3::new(v[0], v[1], v[2]).map_err(|_| refused());
    let frame = Frame3::try_new(
        axis([1.0, 0.0, 0.0])?,
        axis([0.0, 1.0, 0.0])?,
        axis([0.0, 0.0, 1.0])?,
    )
    .map_err(|_| refused())?;
    let translation = RigidTransform::from_body_frame(frame, Point3::new(0.0, 0.0, offset_mm))
        .map_err(|_| refused())?;
    let translated = plain.placed(&translation).map_err(|_| refused())?;
    let body = Body::from_mesh(
        translated.clone(),
        face_table.to_vec(),
        face_normals.to_vec(),
        Some(tolerance),
    )
    .map_err(|_| refused())?;
    let solid = SolidMesh::from_admitted(&translated, face_table.to_vec(), volume_mm3)
        .ok_or_else(refused)?;
    Ok(Some((body, solid)))
}

/// How a contour relates to the first-slice shape.
enum Contour {
    /// `(0,0,0), (w,0,0), (w,l,0), (0,l,0)` with `w, l > 0`.
    OriginRectangle { width: f64, length: f64 },
    /// The same rectangle listed clockwise.
    ClockwiseOriginRectangle,
    /// Anything else.
    Other,
}

fn classify(points: &[[f64; 3]]) -> Contour {
    let [p0, p1, p2, p3] = match points {
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
    if zero(p1[0]) && zero(p1[2]) && p1[1] > 0.0 {
        let (l, w) = (p1[1], p2[0]);
        if p2[1] == l && zero(p2[2]) && w > 0.0 && p3[0] == w && zero(p3[1]) && zero(p3[2]) {
            return Contour::ClockwiseOriginRectangle;
        }
    }
    Contour::Other
}

/// Evaluates an authored plate with the kernel: engineering validation in the F04 order that
/// P01 implements, then the sealed rectangular body.
///
/// Order: count, finiteness, coordinate and dimension caps, local size, minima, ordered area,
/// planarity, winding and exact projected simplicity, then the supported shape (see
/// the module documentation). The caller is expected to have run
/// [`CreatePlate::validate_arguments`] for label, placement and the argument-level caps; this
/// function repeats the checks it depends on and never panics on any input.
pub fn evaluate_plate(authored: &AuthoredPlate) -> Result<PlateBody, CommandError> {
    let points = &authored.contour_points_mm;
    let contour = beamcraft_kernel::plate_contour::admit(points, authored.thickness_mm)?;
    let offset = side_offset(authored);
    match classify(points) {
        // A treated origin rectangle (A12.2) is no longer the G02 box: the treatments replace
        // corners, so it evaluates as the contour below and takes the treated label.
        Contour::OriginRectangle { width, length } if authored.corners.is_empty() => {
            let result = rectangular_plate(RectangularPlate {
                width_mm: width,
                length_mm: length,
                thickness_mm: authored.thickness_mm,
            })
            .map_err(|f| map_kernel(&f))?;
            let shifted = laid_off(
                result.mesh().blob(),
                &result
                    .mesh()
                    .face_table()
                    .iter()
                    .map(|s| (*s).to_owned())
                    .collect::<Vec<_>>(),
                &result
                    .mesh()
                    .face_normals()
                    .iter()
                    .map(|n| Some([n.x(), n.y(), n.z()]))
                    .collect::<Vec<_>>(),
                result.tolerance(),
                result.volume_mm3(),
                offset,
            )?;
            let base = shifted
                .as_ref()
                .map_or_else(|| SolidMesh::from_box(&result), |(_, mesh)| mesh.clone());
            let holes = authored.hole_polygons_in_replay_order();
            let solid = apply_holes_and_cut(
                || match &shifted {
                    Some((body, _)) => Ok(body.clone()),
                    None => Body::from_plate(&result),
                },
                base,
                authored.thickness_mm,
                offset,
                &holes,
                &authored.cuts,
            )?;
            Ok(PlateBody {
                gross_volume_mm3: result.volume_mm3(),
                result: Some(result),
                solid,
                width_mm: width,
                length_mm: length,
                thickness_mm: authored.thickness_mm,
                hole_count: holes.len(),
                cut_applied: !authored.cuts.is_empty(),
                corner_treated: false,
                outline_mm: Vec::new(),
                arc_deviation_mm: 0.0,
            })
        }
        // A treated origin rectangle falls through to the contour evaluation below (A12.2).
        Contour::OriginRectangle { .. } => evaluate_contour(authored),
        Contour::ClockwiseOriginRectangle => Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "contour_points_mm",
            "contour winding is clockwise",
        )),
        Contour::Other if points.iter().any(|p| p[2] != 0.0) => {
            // A corner treatment is authored on the z = 0 contour plane (the amendment); the
            // framed body has no corner of that frame to treat.
            if !authored.corners.is_empty() {
                return Err(CommandError::new(
                    ErrorCode::UnsupportedGeometry,
                    "corner",
                    "corner treatments on a framed plate are not evaluated",
                ));
            }
            let [width, length] = contour.rectangle_mm.ok_or_else(|| {
                CommandError::new(
                    ErrorCode::UnsupportedGeometry,
                    "contour_points_mm",
                    "only exactly rectangular contours outside z = 0 are evaluated",
                )
            })?;
            // Existing hole coordinates are element-local XY, not the new body frame.
            // Refuse rather than silently rotate or relocate the authored cutters.
            if !authored.holes_mm.is_empty() {
                return Err(CommandError::new(
                    ErrorCode::UnsupportedGeometry,
                    "holes_mm",
                    "holes on a framed plate are not evaluated",
                ));
            }
            if !authored.hole_records.is_empty() {
                return Err(CommandError::new(
                    ErrorCode::UnsupportedGeometry,
                    "hole",
                    "hole records on a framed plate are not evaluated",
                ));
            }
            let result =
                beamcraft_kernel::plate_contour::rectangular_body(points, authored.thickness_mm)?;
            let shifted = laid_off(
                result.mesh().blob(),
                &result
                    .mesh()
                    .face_table()
                    .iter()
                    .map(|s| (*s).to_owned())
                    .collect::<Vec<_>>(),
                &result
                    .mesh()
                    .face_normals()
                    .iter()
                    .map(|n| Some([n.x(), n.y(), n.z()]))
                    .collect::<Vec<_>>(),
                result.tolerance(),
                result.volume_mm3(),
                offset,
            )?;
            let base = shifted
                .as_ref()
                .map_or_else(|| SolidMesh::from_box(&result), |(_, mesh)| mesh.clone());
            let solid = apply_holes_and_cut(
                || match &shifted {
                    Some((body, _)) => Ok(body.clone()),
                    None => Body::from_plate(&result),
                },
                base,
                authored.thickness_mm,
                offset,
                &[],
                &authored.cuts,
            )?;
            Ok(PlateBody {
                gross_volume_mm3: result.volume_mm3(),
                result: Some(result),
                solid,
                width_mm: width,
                length_mm: length,
                thickness_mm: authored.thickness_mm,
                hole_count: 0,
                cut_applied: !authored.cuts.is_empty(),
                corner_treated: false,
                outline_mm: Vec::new(),
                arc_deviation_mm: 0.0,
            })
        }
        Contour::Other => evaluate_contour(authored),
    }
}

fn map_profile(failure: &ProfileFailure) -> CommandError {
    failure.diagnostics().first().map_or(
        CommandError::new(
            ErrorCode::UnsupportedGeometry,
            "contour_points_mm",
            "contour evaluation failed",
        ),
        |d| {
            let field = match d.field {
                ProfileField::Thickness => "thickness_mm",
                _ => "contour_points_mm",
            };
            CommandError::new(d.code.f04_code(), field, "contour evaluation failed")
        },
    )
}

/// Evaluates a contour that is not the origin rectangle (A02), or any contour carrying corner
/// treatments (A12.2): the G05 extrusion of a simple polygon that lies in the plane `z = 0`,
/// counter-clockwise seen from `+Z`.
///
/// The contour is used exactly as authored (F04 P-1) unless a vertex carries a corner
/// treatment, in which case the treated outline is derived first (in `f64`, arcs faceted to
/// the reported deviation) and the kernel validates the derived polygon with its exact
/// predicates, as for any contour: nothing is reordered, merged or repaired, and a clockwise,
/// self-crossing or backtracking outline is `INVALID_ARGUMENT`. Rectangles outside `z = 0`
/// take the separate framed-box path in `evaluate_plate`.
fn evaluate_contour(authored: &AuthoredPlate) -> Result<PlateBody, CommandError> {
    if authored.contour_points_mm.iter().any(|p| p[2] != 0.0) {
        return Err(CommandError::new(
            ErrorCode::UnsupportedGeometry,
            "contour_points_mm",
            "only a contour in the plane z = 0 is evaluated in this slice",
        ));
    }
    let (outline, arc_deviation) =
        crate::corners::treated_outline(&authored.contour_points_mm, &authored.corners)?;
    let outer: Vec<Point2> = outline.iter().map(|[x, y]| Point2::new(*x, *y)).collect();
    let extrusion = extrude(&outer, &[], authored.thickness_mm).map_err(|f| map_profile(&f))?;
    let offset = side_offset(authored);
    let shifted = laid_off(
        extrusion.mesh().blob(),
        extrusion.mesh().face_table(),
        &extrusion
            .mesh()
            .face_normals()
            .iter()
            .map(|n| n.map(|v| [v.x(), v.y(), v.z()]))
            .collect::<Vec<_>>(),
        extrusion.tolerance(),
        extrusion.volume_mm3(),
        offset,
    )?;
    let base = match shifted.as_ref() {
        Some((_, mesh)) => mesh.clone(),
        None => SolidMesh::from_extrusion(&extrusion).ok_or_else(|| {
            CommandError::new(
                ErrorCode::UnsupportedGeometry,
                "contour_points_mm",
                "the contour mesh could not be digested",
            )
        })?,
    };
    let holes = authored.hole_polygons_in_replay_order();
    let solid = apply_holes_and_cut(
        || match &shifted {
            Some((body, _)) => Ok(body.clone()),
            None => Body::from_extrusion(&extrusion),
        },
        base,
        authored.thickness_mm,
        offset,
        &holes,
        &authored.cuts,
    )?;
    let extent = |axis: fn(&Point2) -> f64| {
        let low = outer.iter().map(axis).fold(f64::INFINITY, f64::min);
        let high = outer.iter().map(axis).fold(f64::NEG_INFINITY, f64::max);
        high - low
    };
    Ok(PlateBody {
        result: None,
        gross_volume_mm3: extrusion.volume_mm3(),
        solid,
        width_mm: extent(|p| p.x),
        length_mm: extent(|p| p.y),
        thickness_mm: authored.thickness_mm,
        hole_count: holes.len(),
        cut_applied: !authored.cuts.is_empty(),
        corner_treated: !authored.corners.is_empty(),
        outline_mm: outline.clone(),
        arc_deviation_mm: arc_deviation,
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

/// The engine's typed refusal for a plane cut that the kernel would not apply.
pub(crate) fn map_cut(failure: &OpFailure) -> CommandError {
    let message = match failure.code {
        OpCode::OpNoEffect => "the cutting plane misses the element entirely",
        OpCode::OpUnsupported => {
            "the cutting plane is tangent to the element or its scope is unsupported"
        }
        _ => "the plane cut failed",
    };
    CommandError::new(failure.code.f04_code(), "cut", message)
}

/// The certified result of one plane-cut record over `body`: it keeps the record's half-space and
/// names the cap face `cut:f.p<ordinal>`.
pub(crate) fn apply_cut(body: &Body, cut: &CutRecord) -> Result<OpResult, CommandError> {
    let plane = CutPlane::new(cut.point_mm, cut.normal);
    let keep = match cut.keep {
        beamcraft_model::command::KeepSide::Positive => Keep::Positive,
        beamcraft_model::command::KeepSide::Negative => Keep::Negative,
    };
    plane_cut(body, &plane, keep, cut.ordinal).map_err(|f| map_cut(&f))
}

/// The certified mesh of the cut records replayed in ascending order over `body`, each keeping
/// its own half-space. An empty list leaves `base` untouched (a member path that has not decided
/// between the plain and the cut body yet).
pub(crate) fn replay_cuts(
    body: &Body,
    base: SolidMesh,
    cuts: &[CutRecord],
) -> Result<SolidMesh, CommandError> {
    let mut body = body.clone();
    let mut last = None;
    for cut in cuts {
        let applied = apply_cut(&body, cut)?;
        body = applied.body().clone();
        last = Some(applied);
    }
    match last {
        Some(applied) => Ok(SolidMesh::from_operation(&applied)),
        None => Ok(base),
    }
}

/// The certified mesh of a plate with its through holes. Each hole is one kernel cutter, in
/// order, so the wall ids of the first hole are `h0`. With no holes it is the plain body `base`.
///
/// The cutters span the body's own `z` band (A12.1): `offset_mm` is the element-local Z of the
/// lower face, `0` for the default `top`. The kernel decides, with exact predicates, whether a
/// hole lies strictly inside the outline, is clear of the others and is a clockwise simple
/// polygon; a refusal is `INVALID_ARGUMENT` for a bad polygon and the kernel's own code
/// otherwise, and nothing is applied. The plane-cut records (A03) then replay in ascending
/// ordinal order, each keeping its own half-space; a cut the kernel would not apply refuses the
/// whole command.
fn apply_holes_and_cut(
    body: impl FnOnce() -> Result<Body, OpFailure>,
    base: SolidMesh,
    thickness_mm: f64,
    offset_mm: f64,
    holes: &[Vec<[f64; 2]>],
    cuts: &[CutRecord],
) -> Result<SolidMesh, CommandError> {
    let mut body = body().map_err(|f| map_operation(&f))?;
    let mut last = None;
    for (ordinal, hole) in holes.iter().enumerate() {
        let cavity = Cavity::polygon(hole.iter().map(|[x, y]| Point2::new(*x, *y)).collect());
        let ordinal = u16::try_from(ordinal).map_err(|_| {
            CommandError::new(ErrorCode::LimitExceeded, "holes_mm", "too many holes")
        })?;
        let cutter = HoleCutter::new(
            cavity,
            offset_mm - CUTTER_OVERSHOOT_MM,
            offset_mm + thickness_mm + CUTTER_OVERSHOOT_MM,
            ordinal,
        );
        let applied = through_hole(&body, &cutter).map_err(|f| map_operation(&f))?;
        body = applied.body().clone();
        last = Some(applied);
    }
    for cut in cuts {
        // Each cut replays on the previous result, so the cap face ids follow the earlier
        // operations and the ordinals name the caps in creation order.
        let applied = apply_cut(&body, cut)?;
        body = applied.body().clone();
        last = Some(applied);
    }
    match last {
        Some(applied) => Ok(SolidMesh::from_operation(&applied)),
        None => Ok(base),
    }
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
/// A rigid placement changes the scene pose, never the element-local body.
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
    let quantities = body.quantities_for(material)?;
    Ok(CreatePlateOutcome {
        fingerprint: command.content_fingerprint()?,
        authored,
        body,
        quantities,
        footprint,
    })
}

/// Applies `AddHole` to the head authored state of a plate: argument admission, an unused
/// record id, the next per-element ordinal and the full replay of the base shape with every
/// hole (F04 sections 4 and 7). Nothing is published; the caller commits the returned state,
/// its body and the footprint of a shape edit as one atomic command.
///
/// `operation_id_used` reports whether the project already holds a record with the id on any
/// element: a record id is unused in the project (`DUPLICATE_ID`), not only on the target. A
/// hole the kernel refuses (winding, placement, clearance) refuses the whole command, and the
/// head state is returned unchanged. The total polygon count of the plate stays within
/// [`MAX_HOLES`], the present implementation admission, until G10 replaces it with measured
/// kernel capacity.
pub fn add_hole(
    head: &AuthoredPlate,
    command: &AddHole,
    operation_id_used: &dyn Fn(OpId) -> bool,
) -> Result<(AuthoredPlate, PlateBody), CommandError> {
    add_hole_at(head, command, operation_id_used, None)
}
/// Append a hole using the persistent counter rather than reusing a retired semantic ordinal.
pub fn add_hole_after(
    head: &AuthoredPlate,
    command: &AddHole,
    operation_id_used: &dyn Fn(OpId) -> bool,
    next_ordinal: usize,
) -> Result<(AuthoredPlate, PlateBody), CommandError> {
    add_hole_at(head, command, operation_id_used, Some(next_ordinal))
}
fn add_hole_at(
    head: &AuthoredPlate,
    command: &AddHole,
    operation_id_used: &dyn Fn(OpId) -> bool,
    next_ordinal: Option<usize>,
) -> Result<(AuthoredPlate, PlateBody), CommandError> {
    command.validate_arguments()?;
    if operation_id_used(command.operation_id) {
        return Err(CommandError::new(
            ErrorCode::DuplicateId,
            "operation_id",
            "operation id already names a record of the project",
        ));
    }
    if head.hole_polygons_in_replay_order().len() >= MAX_HOLES {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "hole",
            "more than 64 holes on one plate",
        )
        .with_rule("DV-LIM-001"));
    }
    if head.id != command.target_id {
        return Err(CommandError::new(
            ErrorCode::NotFound,
            "target_id",
            "hole target not found",
        ));
    }
    let minimum = head
        .hole_records
        .last()
        .map_or(Some(0), |r| r.ordinal.checked_add(1))
        .ok_or_else(|| {
            CommandError::new(
                ErrorCode::LimitExceeded,
                "hole.ordinal",
                "hole ordinal exhausted",
            )
        })?;
    let ordinal = next_ordinal.unwrap_or(minimum);
    if ordinal > 999 {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "hole.ordinal",
            "hole ordinal capacity exhausted",
        ));
    }
    if ordinal < minimum {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "hole.ordinal",
            "hole counter regressed",
        ));
    }
    let mut next = head.clone();
    next.hole_records.push(HoleRecord {
        operation_id: command.operation_id,
        ordinal,
        polygon_mm: command.polygon()?,
        kind: beamcraft_model::command::HoleKind::Hole,
    });
    let body = evaluate_plate(&next).map_err(|error| {
        // The target was valid, so a refusal names the new hole, not the plate's fields.
        if error.field == "holes_mm" {
            CommandError::new(error.code, "hole", error.message)
        } else {
            error
        }
    })?;
    Ok((next, body))
}

/// Replace an owned hole in memory and replay all holes/cuts before any publication.
/// The original authored state is unchanged on every refusal.
pub fn update_hole(
    head: &AuthoredPlate,
    command: &beamcraft_model::command::UpdateHole,
) -> Result<(AuthoredPlate, PlateBody), CommandError> {
    command.validate_arguments()?;
    if head.id != command.hole.target_id {
        return Err(CommandError::new(
            ErrorCode::NotFound,
            "target_id",
            "hole target not found",
        ));
    }
    let mut next = head.clone();
    let record = next
        .hole_records
        .iter_mut()
        .find(|r| r.operation_id == command.hole.operation_id)
        .ok_or_else(|| {
            CommandError::new(
                ErrorCode::NotFound,
                "operation_id",
                "hole record not found on target",
            )
        })?;
    // An A13 polygon cut shares the record list but is not an A04 hole: the adapter refuses
    // it with a typed result instead of editing one.
    if record.kind != beamcraft_model::command::HoleKind::Hole {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "operation_id",
            "the record is a polygon cut, not a hole",
        ));
    }
    record.polygon_mm = command.hole.polygon()?;
    let body = evaluate_plate(&next).map_err(|e| {
        if e.field == "holes_mm" {
            CommandError::new(e.code, "hole", e.message)
        } else {
            e
        }
    })?;
    Ok((next, body))
}

/// Remove one owned hole and re-evaluate all remaining holes and cuts.
pub fn remove_hole(
    head: &AuthoredPlate,
    command: &beamcraft_model::command::RemoveHole,
) -> Result<(AuthoredPlate, PlateBody), CommandError> {
    if head.id != command.target_id {
        return Err(CommandError::new(
            ErrorCode::NotFound,
            "target_id",
            "hole target not found",
        ));
    }
    let record = head
        .hole_records
        .iter()
        .find(|r| r.operation_id == command.operation_id)
        .ok_or_else(|| {
            CommandError::new(
                ErrorCode::NotFound,
                "operation_id",
                "hole record not found on target",
            )
        })?;
    // The same kind guard as `update_hole`: retiring a polygon cut is not an A04 removal.
    if record.kind != beamcraft_model::command::HoleKind::Hole {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "operation_id",
            "the record is a polygon cut, not a hole",
        ));
    }
    let mut next = head.clone();
    next.hole_records
        .retain(|r| r.operation_id != command.operation_id);
    let body = evaluate_plate(&next)?;
    Ok((next, body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use beamcraft_model::command::HoleSpec;

    fn command(contour: Vec<[f64; 3]>, thickness: f64) -> CreatePlate {
        CreatePlate {
            new_id: ElementId::parse("019a7bf0-0000-7000-8000-000000000005").unwrap(),
            name: "Synthetic plate".into(),
            class: 0,
            contour_points_mm: contour,
            thickness_mm: thickness,
            thickness_side: None,
            corners: Vec::new(),
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

    fn cut_record(ordinal: u16, point: [f64; 3], keep: KeepSide) -> CutRecord {
        CutRecord {
            operation_id: OpId::parse("019a7bf0-0000-7000-8000-0000000000c7").unwrap(),
            ordinal,
            point_mm: point,
            normal: [1.0, 0.0, 0.0],
            keep,
        }
    }

    use beamcraft_model::command::KeepSide;
    use beamcraft_model::id::OpId;

    #[test]
    fn cut_records_replay_in_ordinal_order_with_their_kept_side() {
        // A plate cut to x <= 250 keeps 250 * 500 * 10 = 1,250,000 mm3; the net volume is the
        // certified cut mesh itself (the gross stays the full outline).
        let authored = AuthoredPlate {
            id: ElementId::parse("019a7bf0-0000-7000-8000-000000000005").unwrap(),
            name: "Cut plate".into(),
            contour_points_mm: rect(1000.0, 500.0),
            thickness_mm: 10.0,
            thickness_side: None,
            corners: Vec::new(),
            placement: Placement::IDENTITY,
            material: None,
            class: 0,
            holes_mm: Vec::new(),
            hole_records: Vec::new(),
            cuts: vec![cut_record(0, [250.0, 0.0, 0.0], KeepSide::Negative)],
        };
        let body = evaluate_plate(&authored).unwrap();
        assert!((body.quantities().volume_mm3 - 1_250_000.0).abs() <= 1e-6);
        assert_eq!(body.algorithm(), CUT_VOLUME_ALGORITHM);
        assert!(body.cut_applied);
        assert!(body.solid.triangles().len() > 12);
        // Keeping the other side mirrors the volume.
        let flipped = AuthoredPlate {
            cuts: vec![cut_record(3, [250.0, 0.0, 0.0], KeepSide::Positive)],
            ..authored.clone()
        };
        let body = evaluate_plate(&flipped).unwrap();
        assert!((body.quantities().volume_mm3 - 3_750_000.0).abs() <= 1e-6);
        // The cap face is named by the record's ordinal, so the same plane at a later ordinal
        // gives the same body under a different name.
        assert!(
            body.solid
                .scene_mesh()
                .face_table
                .iter()
                .any(|id| id == "cut:f.p3")
        );
        // Two records replay in order: x <= 250 then x >= 150 keeps the 100 mm middle strip.
        let both = AuthoredPlate {
            cuts: vec![
                cut_record(0, [250.0, 0.0, 0.0], KeepSide::Negative),
                cut_record(1, [150.0, 0.0, 0.0], KeepSide::Positive),
            ],
            ..authored.clone()
        };
        let body = evaluate_plate(&both).unwrap();
        assert!((body.quantities().volume_mm3 - 500_000.0).abs() <= 1e-6);
        // A cut after a hole keeps both effects: the hole lies inside the kept part.
        let square = |x: f64, y: f64, s: f64| vec![[x, y], [x, y + s], [x + s, y + s], [x + s, y]];
        let both = AuthoredPlate {
            holes_mm: vec![square(100.0, 100.0, 100.0)],
            cuts: vec![cut_record(0, [250.0, 0.0, 0.0], KeepSide::Negative)],
            ..authored.clone()
        };
        let body = evaluate_plate(&both).unwrap();
        assert!((body.quantities().volume_mm3 - 1_150_000.0).abs() <= 1e-6);
        // A scaled normal describes the same plane and the same body.
        let mut scaled = cut_record(0, [250.0, 0.0, 0.0], KeepSide::Negative);
        scaled.normal = [1000.0, 0.0, 0.0];
        let body = evaluate_plate(&AuthoredPlate {
            cuts: vec![scaled],
            ..authored.clone()
        })
        .unwrap();
        assert!((body.quantities().volume_mm3 - 1_250_000.0).abs() <= 1e-6);
    }

    #[test]
    fn a_cut_the_kernel_refuses_applies_nothing() {
        let authored = AuthoredPlate {
            id: ElementId::parse("019a7bf0-0000-7000-8000-000000000005").unwrap(),
            name: "Cut plate".into(),
            contour_points_mm: rect(1000.0, 500.0),
            thickness_mm: 10.0,
            thickness_side: None,
            corners: Vec::new(),
            placement: Placement::IDENTITY,
            material: None,
            class: 0,
            holes_mm: Vec::new(),
            hole_records: Vec::new(),
            cuts: Vec::new(),
        };
        let code_of = |cut: CutRecord| {
            evaluate_plate(&AuthoredPlate {
                cuts: vec![cut],
                ..authored.clone()
            })
            .unwrap_err()
            .code
        };
        // The plane misses the body entirely.
        assert_eq!(
            code_of(cut_record(0, [2000.0, 0.0, 0.0], KeepSide::Negative)),
            ErrorCode::InvalidArgument
        );
        // The plane only touches the bottom face: a tangent cut, not a trim.
        assert_eq!(
            code_of(CutRecord {
                normal: [0.0, 0.0, 1.0],
                ..cut_record(0, [0.0, 0.0, 0.0], KeepSide::Negative)
            }),
            ErrorCode::UnsupportedGeometry
        );
        // Argument-level failures come first: a zero normal never reaches the kernel.
        assert_eq!(
            code_of(CutRecord {
                normal: [0.0; 3],
                ..cut_record(0, [0.0, 0.0, 0.0], KeepSide::Negative)
            }),
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            code_of(cut_record(0, [f64::NAN, 0.0, 0.0], KeepSide::Negative)),
            ErrorCode::NonFiniteNumber
        );
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
        assert_eq!(out.body.result.as_ref().unwrap().volume_mm3(), 5_000_000.0);
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
    fn the_thickness_side_lays_the_material_off_the_contour_plane() {
        let side_command = |side: Option<PlateSide>| {
            AuthoredPlate::from_command(&CreatePlate {
                thickness_side: side,
                ..command(rect(1000.0, 500.0), 10.0)
            })
        };
        let z_range = |body: &PlateBody| {
            let z = || body.solid.vertices().iter().map(|p| p[2]);
            (
                z().fold(f64::INFINITY, f64::min),
                z().fold(f64::NEG_INFINITY, f64::max),
            )
        };
        // Absence is the top default, bit-identical to an explicit top.
        let default = evaluate_plate(&side_command(None)).unwrap();
        let top = evaluate_plate(&side_command(Some(PlateSide::Top))).unwrap();
        assert_eq!(default.solid, top.solid);
        for body in [&default, &top] {
            let (low, high) = z_range(body);
            assert_eq!((low, high), (0.0, 10.0));
        }
        // Middle is half on each side of the contour plane, bottom is laid off in -Z; the
        // volume does not depend on the side.
        let middle = evaluate_plate(&side_command(Some(PlateSide::Middle))).unwrap();
        let bottom = evaluate_plate(&side_command(Some(PlateSide::Bottom))).unwrap();
        assert_eq!(z_range(&middle), (-5.0, 5.0));
        assert_eq!(z_range(&bottom), (-10.0, 0.0));
        let volume = default.quantities().volume_mm3;
        assert_eq!(top.quantities().volume_mm3, volume);
        assert_eq!(middle.quantities().volume_mm3, volume);
        assert_eq!(bottom.quantities().volume_mm3, volume);
        assert_eq!(default.quantities().volume_gross_mm3, 5_000_000.0);
        // A contour plate follows the same rule in its own evaluation path.
        let outline = vec![
            [0.0, 0.0, 0.0],
            [600.0, 0.0, 0.0],
            [600.0, 400.0, 0.0],
            [300.0, 600.0, 0.0],
            [0.0, 400.0, 0.0],
        ];
        let contour = |side: Option<PlateSide>| {
            AuthoredPlate::from_command(&CreatePlate {
                contour_points_mm: outline.clone(),
                thickness_mm: 8.0,
                thickness_side: side,
                ..command(Vec::new(), 8.0)
            })
        };
        let flat = evaluate_plate(&contour(None)).unwrap();
        assert_eq!(z_range(&flat), (0.0, 8.0));
        let laid = evaluate_plate(&contour(Some(PlateSide::Bottom))).unwrap();
        assert_eq!(z_range(&laid), (-8.0, 0.0));
        assert_eq!(laid.quantities().volume_mm3, flat.quantities().volume_mm3);
    }

    #[test]
    fn a_cut_and_a_hole_replay_on_the_laid_off_body() {
        let side_command = |side: Option<PlateSide>| AuthoredPlate {
            cuts: Vec::new(),
            holes_mm: Vec::new(),
            ..AuthoredPlate::from_command(&CreatePlate {
                thickness_side: side,
                ..command(rect(1000.0, 500.0), 10.0)
            })
        };
        let z_cut = |point: [f64; 3], keep: KeepSide| CutRecord {
            operation_id: OpId::parse("019a7bf0-0000-7000-8000-0000000000c7").unwrap(),
            ordinal: 0,
            point_mm: point,
            normal: [0.0, 0.0, 1.0],
            keep,
        };
        // The same record at z = +5 keeps z <= 5: a top plate loses its upper half.
        let top = evaluate_plate(&AuthoredPlate {
            cuts: vec![z_cut([0.0, 0.0, 5.0], KeepSide::Negative)],
            ..side_command(None)
        })
        .unwrap();
        assert!((top.quantities().volume_mm3 - 2_500_000.0).abs() <= 1e-6);
        // The bottom plate lies entirely below the plane, so the cut misses it and the
        // command is refused: changing the side changes what a fixed cut removes.
        let missed = evaluate_plate(&AuthoredPlate {
            cuts: vec![z_cut([0.0, 0.0, 5.0], KeepSide::Negative)],
            ..side_command(Some(PlateSide::Bottom))
        });
        assert_eq!(missed.unwrap_err().code, ErrorCode::InvalidArgument);
        // A through hole pierces the laid-off body: the middle plate keeps the exact net
        // volume of its own z band.
        let square = |x: f64, y: f64, s: f64| vec![[x, y], [x, y + s], [x + s, y + s], [x + s, y]];
        let holed = evaluate_plate(&AuthoredPlate {
            holes_mm: vec![square(100.0, 100.0, 100.0)],
            ..side_command(Some(PlateSide::Middle))
        })
        .unwrap();
        assert_eq!(holed.hole_count, 1);
        assert!((holed.quantities().volume_mm3 - 4_900_000.0).abs() <= 1e-6);
        let (low, high) = {
            let z = holed.solid.vertices().iter().map(|p| p[2]);
            (
                z.clone().fold(f64::INFINITY, f64::min),
                z.fold(f64::NEG_INFINITY, f64::max),
            )
        };
        assert_eq!((low, high), (-5.0, 5.0));
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
    fn an_add_hole_record_replays_and_keeps_its_ordinal_and_volume() {
        let square = |x: f64, y: f64, s: f64| vec![[x, y], [x, y + s], [x + s, y + s], [x + s, y]];
        let op =
            |n: u8| OpId::parse(&format!("019a7bf0-0000-7000-8000-0000000000{n:02x}")).unwrap();
        let head = AuthoredPlate::from_command(&command(rect(1000.0, 500.0), 10.0));
        let record = |operation_id, polygon| AddHole {
            target_id: head.id,
            operation_id,
            hole: HoleSpec::Polygon(polygon),
        };
        let unused = |_: OpId| false;
        let (next, body) =
            add_hole(&head, &record(op(6), square(100.0, 100.0, 100.0)), &unused).unwrap();
        assert_eq!(next.hole_records.len(), 1);
        assert_eq!(next.hole_records[0].ordinal, 0);
        assert_eq!(next.hole_records[0].operation_id, op(6));
        assert_eq!(body.hole_count, 1);
        assert_eq!(body.quantities_for(None).unwrap().volume_mm3, 4_900_000.0);
        // A second record appends with the next ordinal and is replayed after the first.
        let (next, body) =
            add_hole(&next, &record(op(7), square(400.0, 200.0, 50.0)), &unused).unwrap();
        assert_eq!(next.hole_records[1].ordinal, 1);
        assert_eq!(body.hole_count, 2);
        assert_eq!(body.quantities_for(None).unwrap().volume_mm3, 4_875_000.0);
        // A record id used anywhere in the project refuses the command.
        let error = add_hole(&next, &record(op(6), square(700.0, 300.0, 50.0)), &|id| {
            id == op(6)
        })
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::DuplicateId);
        assert_eq!(error.field, "operation_id");
        // A hole the kernel refuses names the hole argument and applies nothing.
        let mut ccw = square(100.0, 100.0, 100.0);
        ccw.reverse();
        let error = add_hole(&next, &record(op(8), ccw), &unused).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidArgument);
        assert_eq!(error.field, "hole");
        // The present implementation admission keeps the total polygon count within the
        // create-plate cap until G10 replaces it with measured kernel capacity.
        let mut full = AuthoredPlate::from_command(&command(rect(1000.0, 500.0), 10.0));
        full.holes_mm = vec![square(100.0, 100.0, 10.0); MAX_HOLES];
        let error =
            add_hole(&full, &record(op(9), square(300.0, 300.0, 10.0)), &unused).unwrap_err();
        assert_eq!(error.code, ErrorCode::LimitExceeded);
    }

    #[test]
    fn an_add_hole_record_follows_a_pilot_hole_and_is_refused_on_a_framed_plate() {
        let square = |x: f64, y: f64, s: f64| vec![[x, y], [x, y + s], [x + s, y + s], [x + s, y]];
        let op = OpId::parse("019a7bf0-0000-7000-8000-000000000006").unwrap();
        let mut head = AuthoredPlate::from_command(&command(rect(1000.0, 500.0), 10.0));
        head.holes_mm = vec![square(100.0, 100.0, 100.0)];
        let record = AddHole {
            target_id: head.id,
            operation_id: op,
            hole: HoleSpec::Polygon(square(400.0, 200.0, 50.0)),
        };
        let (next, body) = add_hole(&head, &record, &|_| false).unwrap();
        assert_eq!(body.hole_count, 2);
        assert_eq!(body.quantities_for(None).unwrap().volume_mm3, 4_875_000.0);
        assert_eq!(next.hole_polygons_in_replay_order().len(), 2);
        // A framed (out-of-plane) rectangle never takes holes, records included.
        let mut framed = AuthoredPlate::from_command(&command(
            vec![
                [0.0, 0.0, 50.0],
                [1000.0, 0.0, 50.0],
                [1000.0, 500.0, 50.0],
                [0.0, 500.0, 50.0],
            ],
            10.0,
        ));
        framed.hole_records = next.hole_records;
        let error = evaluate_plate(&framed).unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedGeometry);
        assert_eq!(error.field, "hole");
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

    fn l_shape() -> Vec<[f64; 3]> {
        // 1000 x 600 with a 600 x 300 notch at the top right: area 600_000 - 180_000 = 420_000.
        vec![
            [0.0, 0.0, 0.0],
            [1000.0, 0.0, 0.0],
            [1000.0, 300.0, 0.0],
            [400.0, 300.0, 0.0],
            [400.0, 600.0, 0.0],
            [0.0, 600.0, 0.0],
        ]
    }

    #[test]
    fn a_rectangular_contour_in_another_plane_is_evaluated_and_nonplanarity_is_invalid() {
        let moved = vec![
            [10.0, 20.0, 30.0],
            [10.0, 1020.0, 30.0],
            [-490.0, 1020.0, 30.0],
            [-490.0, 20.0, 30.0],
        ];
        let out = create_plate(&command(moved, 10.0), &none).unwrap(); // F04-POS-18
        assert_eq!(out.body.width_mm, 1000.0);
        assert_eq!(out.body.length_mm, 500.0);
        assert_eq!(out.quantities.volume_mm3, 5_000_000.0);
        let tilted = vec![
            [0.0, 0.0, 0.0],
            [1000.0, 0.0, 0.0],
            [1000.0, 500.0, 20.0],
            [0.0, 500.0, 0.0],
        ];
        assert_eq!(code(&command(tilted, 10.0)), ErrorCode::InvalidArgument);
    }

    #[test]
    fn a_simple_contour_is_extruded_with_the_exact_polygon_volume() {
        let out = create_plate(&command(l_shape(), 10.0), &none).unwrap();
        assert!(out.body.result.is_none());
        assert!(!out.body.is_rectangular_box());
        // Independent closed form: 420_000 mm2 * 10 mm.
        assert_eq!(out.quantities.volume_mm3, 4_200_000.0);
        assert_eq!(out.quantities.volume_gross_mm3, 4_200_000.0);
        assert_eq!(out.quantities.removed_volume_mm3, 0.0);
        assert_eq!(out.quantities.algorithm, CONTOUR_VOLUME_ALGORITHM);
        assert_eq!((out.body.width_mm, out.body.length_mm), (1000.0, 600.0));
        // The mesh is closed: the divergence-theorem volume of the stored triangles agrees.
        let solid = &out.body.solid;
        let signed: f64 = solid
            .triangles()
            .iter()
            .map(|t| {
                let v = |i: u32| solid.vertices()[i as usize];
                let (a, b, c) = (v(t[0]), v(t[1]), v(t[2]));
                (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                    + a[2] * (b[0] * c[1] - b[1] * c[0]))
                    / 6.0
            })
            .sum();
        assert!(
            (signed - 4_200_000.0).abs() <= 1e-6 * 4_200_000.0,
            "{signed}"
        );
        // The same contour gives the same bytes.
        let again = create_plate(&command(l_shape(), 10.0), &none).unwrap();
        assert_eq!(again.body.solid.digest(), solid.digest());
        assert_eq!(again.fingerprint, out.fingerprint);
        // A pentagon (F04-POS-20) and a rectangle that is not at the origin or starts at another
        // corner (F04-POS-19) are contours too.
        let pentagon = vec![
            [0.0, 0.0, 0.0],
            [1000.0, 0.0, 0.0],
            [1200.0, 250.0, 0.0],
            [500.0, 600.0, 0.0],
            [0.0, 500.0, 0.0],
        ];
        assert!(create_plate(&command(pentagon, 10.0), &none).is_ok());
        let turned = vec![
            [1000.0, 500.0, 0.0],
            [0.0, 500.0, 0.0],
            [0.0, 0.0, 0.0],
            [1000.0, 0.0, 0.0],
        ];
        let out = create_plate(&command(turned, 10.0), &none).unwrap();
        assert_eq!(out.quantities.volume_mm3, 5_000_000.0);
        let shifted = vec![
            [100.0, 100.0, 0.0],
            [1100.0, 100.0, 0.0],
            [1100.0, 600.0, 0.0],
            [100.0, 600.0, 0.0],
        ];
        let out = create_plate(&command(shifted, 10.0), &none).unwrap();
        assert!(!out.body.is_rectangular_box());
        assert_eq!(out.quantities.volume_mm3, 5_000_000.0);
    }

    #[test]
    fn a_wrong_contour_is_an_invalid_argument_and_nothing_is_built() {
        let mut clockwise = l_shape();
        clockwise.reverse();
        assert_eq!(code(&command(clockwise, 10.0)), ErrorCode::InvalidArgument);
        // F04-NEG-30: a bow tie crosses itself.
        let bow_tie = vec![
            [0.0, 0.0, 0.0],
            [1000.0, 500.0, 0.0],
            [0.0, 500.0, 0.0],
            [800.0, 0.0, 0.0],
        ];
        assert_eq!(code(&command(bow_tie, 10.0)), ErrorCode::InvalidArgument);
        // A vertex touching an edge, and a contour that doubles back on itself.
        let touching = vec![
            [0.0, 0.0, 0.0],
            [1000.0, 0.0, 0.0],
            [1000.0, 500.0, 0.0],
            [500.0, 0.0, 0.0],
            [0.0, 500.0, 0.0],
        ];
        assert_eq!(code(&command(touching, 10.0)), ErrorCode::InvalidArgument);
        let flat = vec![[0.0, 0.0, 0.0], [500.0, 0.0, 0.0], [1000.0, 0.0, 0.0]];
        assert_eq!(code(&command(flat, 10.0)), ErrorCode::InvalidArgument);
        // The thickness floor applies to a contour as it does to a rectangle.
        assert_eq!(code(&command(l_shape(), 0.0)), ErrorCode::InvalidArgument);
    }

    #[test]
    fn through_holes_apply_to_a_contour_plate() {
        let mut c = command(l_shape(), 10.0);
        // A clockwise 100 x 100 hole well inside the lower arm of the L.
        c.holes_mm = vec![vec![
            [100.0, 100.0],
            [100.0, 200.0],
            [200.0, 200.0],
            [200.0, 100.0],
        ]];
        let out = create_plate(&c, &none).unwrap();
        assert_eq!(out.body.hole_count, 1);
        assert_eq!(out.quantities.algorithm, HOLED_VOLUME_ALGORITHM);
        assert_eq!(out.quantities.volume_gross_mm3, 4_200_000.0);
        assert!(
            (out.quantities.volume_mm3 - 4_100_000.0).abs() <= 1e-3,
            "{}",
            out.quantities.volume_mm3
        );
        assert!((out.quantities.removed_volume_mm3 - 100_000.0).abs() <= 1e-3);
        // A hole that leaves the L through the notch is refused, nothing is applied.
        c.holes_mm = vec![vec![
            [350.0, 250.0],
            [350.0, 400.0],
            [450.0, 400.0],
            [450.0, 250.0],
        ]];
        assert_eq!(code(&c), ErrorCode::InvalidArgument);
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
        assert!(create_plate(&c, &none).is_ok());
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
                class: 0,
                contour_points_mm: contour,
                thickness_mm: 10.0,
                thickness_side: None,
                corners: Vec::new(),
                placement: Placement::IDENTITY,
                material: None,
                holes_mm: Vec::new(),
                hole_records: Vec::new(),
                cuts: Vec::new(),
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
    #[test]
    fn hole_updates_keep_record_order_and_identity_and_reject_bad_geometry() {
        let head = AuthoredPlate::from_command(&command(rect(1000.0, 500.0), 10.0));
        let id = OpId::parse("019a7bf0-0000-7000-8000-00000000ad01").unwrap();
        let c = AddHole {
            target_id: head.id,
            operation_id: id,
            hole: HoleSpec::Rectangle {
                corner_mm: [100.0, 100.0],
                width_mm: 100.0,
                height_mm: 100.0,
            },
        };
        let (head, _) = add_hole(&head, &c, &|_| false).unwrap();
        let edit = beamcraft_model::command::UpdateHole {
            hole: AddHole {
                hole: HoleSpec::Rectangle {
                    corner_mm: [100.0, 100.0],
                    width_mm: 200.0,
                    height_mm: 100.0,
                },
                ..c
            },
        };
        let (next, body) = update_hole(&head, &edit).unwrap();
        assert_eq!(next.hole_records.first().unwrap().ordinal, 0);
        assert_eq!(next.hole_records.first().unwrap().operation_id, id);
        assert_eq!(body.quantities_for(None).unwrap().volume_mm3, 4_800_000.0);
        assert_ne!(
            edit.content_fingerprint().unwrap(),
            edit.hole.content_fingerprint().unwrap()
        );
        let mut bad = edit.clone();
        bad.hole.operation_id = OpId::parse("019a7bf0-0000-7000-8000-00000000ad02").unwrap();
        assert!(update_hole(&head, &bad).is_err());
        assert_eq!(head.hole_records.first().unwrap().polygon_mm.len(), 4);
    }
    #[test]
    fn hole_removal_preserves_ordinal_history_and_large_ordinals_refuse_without_overflow() {
        let head = AuthoredPlate::from_command(&command(rect(1000.0, 500.0), 10.0));
        let id = OpId::parse("019a7bf0-0000-7000-8000-00000000af01").unwrap();
        let c = AddHole {
            target_id: head.id,
            operation_id: id,
            hole: HoleSpec::Rectangle {
                corner_mm: [100.0, 100.0],
                width_mm: 100.0,
                height_mm: 100.0,
            },
        };
        let (p, _) = add_hole_after(&head, &c, &|_| false, 7).unwrap();
        let removal = beamcraft_model::command::RemoveHole {
            target_id: p.id,
            operation_id: id,
        };
        let (next, body) = remove_hole(&p, &removal).unwrap();
        assert!(next.hole_records.is_empty());
        assert_eq!(body.quantities_for(None).unwrap().volume_mm3, 5_000_000.0);
        assert!(remove_hole(&next, &removal).is_err());
        assert!(add_hole_after(&head, &c, &|_| false, 1000).is_err());
        let mut extreme = p;
        extreme.hole_records.first_mut().unwrap().ordinal = usize::MAX;
        assert!(add_hole(&extreme, &c, &|_| false).is_err());
    }
    #[test]
    fn the_a04_adapter_refuses_an_a13_polygon_cut_record_with_a_typed_result() {
        // A polygon cut lives in the same record list with kind PolygonCut; update-hole and
        // remove-hole must never edit or retire one through the hole vocabulary.
        let head = AuthoredPlate::from_command(&command(rect(1000.0, 500.0), 10.0));
        let id = OpId::parse("019a7bf0-0000-7000-8000-00000000af11").unwrap();
        let mut with_cut = head.clone();
        with_cut.hole_records.push(HoleRecord {
            operation_id: id,
            ordinal: 0,
            polygon_mm: vec![
                [100.0, 100.0],
                [200.0, 100.0],
                [200.0, 200.0],
                [100.0, 200.0],
            ],
            kind: beamcraft_model::command::HoleKind::PolygonCut,
        });
        let edit = beamcraft_model::command::UpdateHole {
            hole: AddHole {
                target_id: with_cut.id,
                operation_id: id,
                hole: HoleSpec::Rectangle {
                    corner_mm: [100.0, 100.0],
                    width_mm: 50.0,
                    height_mm: 50.0,
                },
            },
        };
        assert_eq!(
            update_hole(&with_cut, &edit).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        let removal = beamcraft_model::command::RemoveHole {
            target_id: with_cut.id,
            operation_id: id,
        };
        assert_eq!(
            remove_hole(&with_cut, &removal).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(with_cut.hole_records.len(), 1);
    }
}
