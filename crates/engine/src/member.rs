// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! F04 command application for `CreateStraightMember` (P03) and `CreatePolybeam` (P12): the
//! member counterparts of [`crate::command::create_plate`].
//!
//! A member is a beam or a column with a required pinned profile and an optional pinned
//! material. The engine never touches USD: it validates the command, resolves the pinned
//! definitions through caller lookups, asks the kernel for the body (the section is centred on
//! the axis, width along the rolled `X_m`, height along `Y_m`; F06 sections 2, 4 and 6) and
//! returns the authored state, the body, the quantities and the declared footprint.
//!
//! Members keep their axis, roll, cuts and mesh in element-local coordinates. A rigid placement
//! is carried by the scene instance and USD Xform; it never changes shape or mass keys. The
//! world section orientation is the placement rotation applied after the local axis/roll frame.
//! Placement admission remains mandatory even though it does not enter body evaluation.
//!
//! A member with exactly two axis points is straight (P03); one with more is a polybeam (P12):
//! the one section is swept along the polyline, every interior vertex is mitred (G12), and the
//! volume is the effective section area times the path length.

use crate::command::{
    AreaSource, CUT_VOLUME_ALGORITHM, MaterialInfo, MaterialLookup, Quantities, Volumes, map_cut,
    quantities_with_material, replay_cuts,
};
use crate::footprint::Footprint;
use crate::solid::SolidMesh;
use crate::transaction::PriorOp;
use beamcraft_geom::{MemberFrame, Point3};
use beamcraft_kernel::diagnostic::{Failure, Field};
use beamcraft_kernel::ops::{Body, OpFailure};
use beamcraft_kernel::section::{
    DETAILED_MIN_SAGITTA_MM, Representation, SectionFailure, SectionMember, section_member,
};
use beamcraft_kernel::sweep::{Sweep, SweepFailure, SweepPath, sweep};
use beamcraft_kernel::{StraightMember, straight_member};
use beamcraft_model::command::{
    CommandError, CreatePolybeam, CreateStraightMember, CutRecord, MemberKind, Placement,
};
use beamcraft_model::definition::{
    DeclaredAreaWarning, RectangleProfileSpec, SectionProfileSpec, SectionProperties,
};
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId};
use beamcraft_model::section::{CustomSection, SectionShape};

/// Version label of the volume algorithm in the [`Quantities`] record of a rectangle member:
/// the effective section area (the declared catalogue area, else `width * height`) times the
/// axis length.
pub const MEMBER_VOLUME_ALGORITHM: &str =
    "pmass-member-area-x-length/1;rectangle;tolerance-policy-3";

/// Version label of the volume algorithm of a member of an A07.1 section: the effective section
/// area (the declared catalogue area, else the exact closed-form area with true arcs) times the
/// axis length. The mesh is still the detailed polygon outline; its volume is not this number.
pub const SECTION_VOLUME_ALGORITHM: &str =
    "pmass-member-area-x-length/1;section-exact-area;tolerance-policy-3";

/// Version label of the volume algorithm of a polybeam (P12): the effective section area (the
/// declared catalogue area, else the exact area) times the polyline path length. The G12 seal
/// cross-checks the mesh against the analytic truncated-prism value, which drops the mitres for
/// a section centred on the path.
pub const POLYBEAM_VOLUME_ALGORITHM: &str =
    "pmass-member-area-x-path-length/1;polybeam;tolerance-policy-3";

/// Labels that earlier slices stored for members. A stored record that carries one is a
/// superseded value: it reads as `stale` and is recomputed by the next edit that rewrites the
/// record; it is not corruption.
pub const SUPERSEDED_MEMBER_ALGORITHMS: [&str; 2] = [
    "kernel-p03-straight-member-volume/1;tolerance-policy-3",
    "kernel-a07-section-member-volume/1;detailed-arcs-max(0.25mm,0.5%r);tolerance-policy-3",
];

/// The content of a resolved immutable profile definition.
#[derive(Debug, Clone, PartialEq)]
pub enum ProfileInfo {
    /// The first-scope centred rectangle.
    Rectangle {
        /// Section width in millimetres.
        width_mm: f64,
        /// Section height in millimetres.
        height_mm: f64,
        /// The section area the definition declares in mm2, if any (the catalogue area).
        declared_area_mm2: Option<f64>,
    },
    /// An A07.1 section family or custom outline.
    Section {
        /// The shape.
        shape: SectionShape,
        /// The section area the definition declares in mm2, if any (the catalogue area).
        declared_area_mm2: Option<f64>,
    },
}

impl ProfileInfo {
    /// Width and height of the section's bounding box in millimetres.
    pub fn extent_mm(&self) -> (f64, f64) {
        match self {
            Self::Rectangle {
                width_mm,
                height_mm,
                ..
            } => (*width_mm, *height_mm),
            Self::Section { shape, .. } => shape.extent_mm(),
        }
    }

    /// The declared (catalogue) area in mm2, if the definition carries one.
    pub const fn declared_area_mm2(&self) -> Option<f64> {
        match self {
            Self::Rectangle {
                declared_area_mm2, ..
            }
            | Self::Section {
                declared_area_mm2, ..
            } => *declared_area_mm2,
        }
    }

    /// The exact area computed from the parameters in mm2: `width * height` for a rectangle, the
    /// closed form with true arcs for a family, the shoelace area of a custom outline.
    pub fn exact_area_mm2(&self) -> f64 {
        match self {
            Self::Rectangle {
                width_mm,
                height_mm,
                ..
            } => width_mm * height_mm,
            Self::Section { shape, .. } => shape.exact_area_mm2(),
        }
    }

    /// The area a member's volume uses and where it came from: the declared catalogue area when
    /// the definition carries one, else the exact computed area. There is never no area.
    pub fn effective_area(&self) -> (f64, AreaSource) {
        match self.declared_area_mm2() {
            Some(area) => (area, AreaSource::Catalogue),
            None => (self.exact_area_mm2(), AreaSource::Computed),
        }
    }
}

/// Resolves a pinned profile digest. A digest with no definition is `NOT_FOUND`; a digest that
/// names a definition of another kind is `INVALID_ARGUMENT`.
pub type ProfileLookup<'a> = &'a dyn Fn(&ContentDigest) -> Result<ProfileInfo, CommandError>;

/// A lookup that knows no definition: any pinned profile is `NOT_FOUND`.
pub fn no_profiles(_: &ContentDigest) -> Result<ProfileInfo, CommandError> {
    Err(CommandError::new(
        ErrorCode::NotFound,
        "profile_digest",
        "profile definition is not in the model",
    ))
}

/// The authored state of one member: exactly the inputs F04 section 4 stores in the model layer.
#[derive(Debug, Clone, PartialEq)]
pub struct AuthoredMember {
    /// Full element identity.
    pub id: ElementId,
    /// Beam or column.
    pub kind: MemberKind,
    /// Label.
    pub name: String,
    /// Authored integer part class (P15); zero is the default.
    pub class: i32,
    /// Element-local path points in order: exactly two for a straight member, more for a
    /// polybeam.
    pub axis_points_mm: Vec<[f64; 3]>,
    /// Pinned profile definition.
    pub profile: ContentDigest,
    /// Explicit roll; `None` is zero. A polybeam has no authored roll: the G12 sweep carries the
    /// section parallel-transported.
    pub roll_rad: Option<f64>,
    /// Rigid placement as submitted.
    pub placement: Placement,
    /// Pinned material definition, if any.
    pub material: Option<ContentDigest>,
    /// The owned `plane_cut` operation records (A03, amendment A-2), replayed in ascending
    /// ordinal order. Empty means an uncut member.
    pub cuts: Vec<CutRecord>,
}

impl AuthoredMember {
    /// The authored fields of an accepted straight-member command.
    pub fn from_command(command: &CreateStraightMember) -> Self {
        Self {
            id: command.new_id,
            kind: command.kind,
            name: command.name.clone(),
            class: command.class,
            axis_points_mm: vec![command.axis_points_mm[0], command.axis_points_mm[1]],
            profile: command.profile_digest,
            roll_rad: command.roll_rad,
            placement: command.placement,
            material: command.material_digest,
            cuts: Vec::new(),
        }
    }

    /// The authored fields of an accepted polybeam command.
    pub fn from_polybeam(command: &CreatePolybeam) -> Self {
        Self {
            id: command.new_id,
            kind: command.kind,
            name: command.name.clone(),
            class: command.class,
            axis_points_mm: command.path_points_mm.clone(),
            profile: command.profile_digest,
            roll_rad: None,
            placement: command.placement,
            material: command.material_digest,
            cuts: Vec::new(),
        }
    }

    /// The `CreateStraightMember` that would author exactly this state, or `None` for a
    /// polybeam member.
    pub fn to_command(&self) -> Option<CreateStraightMember> {
        let [p1, p2] = self.axis_points_mm.as_slice() else {
            return None;
        };
        Some(CreateStraightMember {
            new_id: self.id,
            kind: self.kind,
            name: self.name.clone(),
            class: self.class,
            axis_points_mm: [*p1, *p2],
            profile_digest: self.profile,
            placement: self.placement,
            roll_rad: self.roll_rad,
            material_digest: self.material,
        })
    }

    /// The `CreatePolybeam` that would author exactly this state, or `None` for a straight
    /// member.
    pub fn to_polybeam_command(&self) -> Option<CreatePolybeam> {
        if self.axis_points_mm.len() < 3 {
            return None;
        }
        Some(CreatePolybeam {
            new_id: self.id,
            kind: self.kind,
            name: self.name.clone(),
            class: self.class,
            path_points_mm: self.axis_points_mm.clone(),
            profile_digest: self.profile,
            placement: self.placement,
            material_digest: self.material,
        })
    }
}

/// A member evaluated by the kernel: the sealed body and its section.
#[derive(Debug, Clone, PartialEq)]
pub struct MemberBody {
    /// The certified mesh, as every body offers it to the derived layer and the view. For a
    /// section its arcs are polygons, so `solid.volume_mm3()` (the mesh volume) can differ from
    /// [`Self::volume_mm3`] by the arc approximation: at most about `pi/3 * r * s` of area per
    /// quarter arc of radius `r` and sagitta `s <= max(0.25 mm, 0.5 % r)`, times the length.
    pub solid: SolidMesh,
    /// The resolved profile the body was built from.
    pub profile: ProfileInfo,
    /// Width of the section's bounding box in millimetres.
    pub width_mm: f64,
    /// Height of the section's bounding box in millimetres.
    pub height_mm: f64,
    /// The axis length in millimetres.
    pub axis_length_mm: f64,
    /// The effective section area in mm2 (declared catalogue area, else the exact area).
    pub section_area_mm2: f64,
    /// Where the effective area came from.
    pub area_source: AreaSource,
    /// Gross volume in cubic millimetres: `section_area_mm2 * axis_length_mm`. A member has no
    /// operation yet, so this is also its net volume.
    pub volume_mm3: f64,
    /// Version label of the volume algorithm.
    pub algorithm: &'static str,
}

fn map_kernel(failure: &Failure) -> CommandError {
    failure.diagnostics().next().map_or(
        CommandError::new(
            ErrorCode::UnsupportedGeometry,
            "geometry",
            "geometry evaluation failed",
        ),
        |d| {
            let field = match d.field {
                Field::Axis => "axis_points_mm",
                Field::Section => "profile",
                Field::Roll => "roll_rad",
                _ => "geometry",
            };
            CommandError::new(d.code.f04_code(), field, "geometry evaluation failed")
        },
    )
}

fn map_section(failure: &SectionFailure) -> CommandError {
    match failure {
        SectionFailure::Frame(f) => map_kernel(f),
        other => CommandError::new(other.f04_code(), "profile", "section evaluation failed"),
    }
}

/// Maps a G12 sweep refusal to the command field that caused it.
fn map_sweep(failure: &SweepFailure) -> CommandError {
    let (field, message) = match failure {
        SweepFailure::Section(_) | SweepFailure::Profile(_) => {
            ("profile", "section evaluation failed")
        }
        SweepFailure::Path(_) => ("path_points_mm", "polybeam path evaluation failed"),
        SweepFailure::Body(_) => ("geometry", "polybeam body certification failed"),
    };
    CommandError::new(failure.f04_code(), field, message)
}

/// Total polyline path length in millimetres. A stored axis is untrusted input, so a
/// non-finite chord or sum is a refusal, never a silent quantity.
fn path_length_mm(points: &[[f64; 3]]) -> Result<f64, CommandError> {
    let mut total = 0.0;
    for (a, b) in points.iter().zip(points.iter().skip(1)) {
        let [ax, ay, az] = *a;
        let [bx, by, bz] = *b;
        let d = [bx - ax, by - ay, bz - az];
        let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if !length.is_finite() {
            return Err(CommandError::new(
                ErrorCode::NonFiniteNumber,
                "axis_points_mm",
                "a path chord has no finite length",
            ));
        }
        total += length;
    }
    if !total.is_finite() {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "axis_points_mm",
            "the path length exceeds the finite range",
        ));
    }
    Ok(total)
}

/// The centred rectangle of a first-scope profile as an admitted custom outline, the section
/// the G12 sweep polygon is built from. The admission is the rectangle's own rule: the values
/// are re-checked because a committed definition is untrusted input.
fn rectangle_outline(
    width_mm: f64,
    height_mm: f64,
    declared_area_mm2: Option<f64>,
) -> Result<SectionShape, CommandError> {
    RectangleProfileSpec {
        width_mm,
        height_mm,
        section: SectionProperties {
            area_mm2: declared_area_mm2,
            ..SectionProperties::default()
        },
    }
    .validate()?;
    let (x, y) = (width_mm / 2.0, height_mm / 2.0);
    Ok(SectionShape::Custom(CustomSection {
        outer: vec![[-x, -y], [x, -y], [x, y], [-x, y]],
        cavities: Vec::new(),
    }))
}

/// The body of a polybeam: the one section swept along the polyline path (G12), every interior
/// vertex mitred. The quantity of record stays analytic — the effective section area times the
/// path length; a cut replaces both mesh and volume with the certified replay result.
fn evaluate_polybeam_body(
    authored: &AuthoredMember,
    profile: &ProfileInfo,
    points: &[[f64; 3]],
) -> Result<(SolidMesh, f64, &'static str, Option<f64>), CommandError> {
    let shape = match profile {
        ProfileInfo::Rectangle {
            width_mm,
            height_mm,
            declared_area_mm2,
        } => rectangle_outline(*width_mm, *height_mm, *declared_area_mm2)?,
        ProfileInfo::Section { shape, .. } => shape.clone(),
    };
    let swept = sweep(&Sweep {
        path: SweepPath::Polyline(
            points
                .iter()
                .map(|p| Point3::new(p[0], p[1], p[2]))
                .collect(),
        ),
        shape,
        // The detailed outline of the section, like a straight member of the same section.
        representation: Representation::Detailed,
        // Unused by a pure polyline, which has no curve; a positive value is required anyway.
        chord_deviation_mm: DETAILED_MIN_SAGITTA_MM,
    })
    .map_err(|failure| map_sweep(&failure))?;
    let path_length_mm = path_length_mm(points)?;
    if authored.cuts.is_empty() {
        Ok((
            SolidMesh::from_sweep(&swept),
            path_length_mm,
            POLYBEAM_VOLUME_ALGORITHM,
            None,
        ))
    } else {
        let solid = replay_cuts(swept.body(), SolidMesh::from_sweep(&swept), &authored.cuts)?;
        let volume = solid.volume_mm3();
        Ok((solid, path_length_mm, CUT_VOLUME_ALGORITHM, Some(volume)))
    }
}

/// Net area in mm2 of the detailed polygon outline of a section shape, as the kernel generates
/// it for the mesh (shoelace of the polygon, radii as chords on the arcs). It is a display and
/// geometry figure only: quantities use the exact area ([`ProfileInfo::effective_area`]).
pub fn section_outline_area_mm2(shape: &SectionShape) -> Result<f64, CommandError> {
    let result = section_member(&SectionMember {
        start: Point3::new(0.0, 0.0, 0.0),
        end: Point3::new(0.0, 0.0, 1000.0),
        shape: shape.clone(),
        representation: Representation::Detailed,
        roll_rad: None,
    })
    .map_err(|failure| map_section(&failure))?;
    Ok(result.net_area_mm2())
}

/// The warning for a declared section area that differs from the EXACT area of the section by
/// more than 1 percent (owner decision D3: a warning, never a refusal; the comparison is with the
/// closed-form area, not the polygon of the mesh). `None` when no area is declared, when it
/// agrees, or when the shape is not admitted (the declared value stays authoritative and the
/// failure surfaces where the body is built). The `kernel_mm2` field of the warning carries the
/// exact area.
pub fn declared_section_area_warning(spec: &SectionProfileSpec) -> Option<DeclaredAreaWarning> {
    spec.section.area_mm2?;
    spec.shape.validate().ok()?;
    spec.declared_area_warning(spec.shape.exact_area_mm2())
}

/// Evaluates an authored member with its resolved profile. The profile values are re-admitted
/// because a committed definition is untrusted input. A rectangle keeps the P03 box builder (and
/// its exact bytes); a section family or custom outline is the kernel's detailed A07.1 outline
/// extruded along the axis for the mesh. The volume is `effective area x axis length`, where the
/// effective area is the declared catalogue area when the profile carries one, else the exact
/// computed area (P-Mass).
///
/// Placement is validated but excluded from the local body and its keys. The scene instance
/// applies the complete rigid transform for bounds, picking, features and display.
pub fn evaluate_member(
    authored: &AuthoredMember,
    profile: ProfileInfo,
) -> Result<MemberBody, CommandError> {
    authored.placement.validate()?;
    let points: Vec<Point3> = authored
        .axis_points_mm
        .iter()
        .map(|p| Point3::new(p[0], p[1], p[2]))
        .collect();
    let (width_mm, height_mm) = profile.extent_mm();
    SectionProperties {
        area_mm2: profile.declared_area_mm2(),
        ..SectionProperties::default()
    }
    .validate()?;
    let (solid, axis_length_mm, algorithm, cut_volume_mm3) = if points.len() > 2 {
        evaluate_polybeam_body(authored, &profile, &authored.axis_points_mm)?
    } else {
        let Some((start, end)) = points.first().zip(points.last()).map(|(a, b)| (*a, *b)) else {
            return Err(CommandError::new(
                ErrorCode::UnsupportedGeometry,
                "axis_points_mm",
                "the member axis has no ends",
            ));
        };
        match &profile {
            ProfileInfo::Rectangle {
                width_mm,
                height_mm,
                declared_area_mm2,
            } => {
                // The declared area is admitted with the rectangle's own rule (it must agree with
                // width x height within 0.1 percent), so a direct engine caller cannot pair a
                // rectangle with an area that would give a different volume.
                RectangleProfileSpec {
                    width_mm: *width_mm,
                    height_mm: *height_mm,
                    section: SectionProperties {
                        area_mm2: *declared_area_mm2,
                        ..SectionProperties::default()
                    },
                }
                .validate()?;
                let result = straight_member(StraightMember {
                    start,
                    end,
                    width_mm: *width_mm,
                    height_mm: *height_mm,
                    roll_rad: authored.roll_rad,
                })
                .map_err(|failure| map_kernel(&failure))?;
                // The kernel succeeded, so the frame exists; the same frame gives the same length.
                let frame = MemberFrame::new(start, end, *width_mm, *height_mm, authored.roll_rad)
                    .map_err(|_| {
                        CommandError::new(
                            ErrorCode::UnsupportedGeometry,
                            "axis_points_mm",
                            "member frame could not be built",
                        )
                    })?;
                // The cuts replace both the mesh and the analytic volume with the certified replay
                // result; without one the P-Mass `area x length` volume stands.
                let (solid, cut_volume) = if authored.cuts.is_empty() {
                    (SolidMesh::from_box(&result), None)
                } else {
                    let body = Body::from_plate(&result).map_err(|f: OpFailure| map_cut(&f))?;
                    let solid = replay_cuts(&body, SolidMesh::from_box(&result), &authored.cuts)?;
                    let volume = solid.volume_mm3();
                    (solid, Some(volume))
                };
                (
                    solid,
                    frame.scale().axis_length_mm(),
                    if cut_volume.is_some() {
                        CUT_VOLUME_ALGORITHM
                    } else {
                        MEMBER_VOLUME_ALGORITHM
                    },
                    cut_volume,
                )
            }
            ProfileInfo::Section { shape, .. } => {
                shape.validate()?;
                let result = section_member(&SectionMember {
                    start,
                    end,
                    shape: shape.clone(),
                    representation: Representation::Detailed,
                    roll_rad: authored.roll_rad,
                })
                .map_err(|failure| map_section(&failure))?;
                if authored.cuts.is_empty() {
                    (
                        SolidMesh::from_section(&result),
                        result.axis_length_mm(),
                        SECTION_VOLUME_ALGORITHM,
                        None,
                    )
                } else {
                    let solid = replay_cuts(
                        result.body(),
                        SolidMesh::from_section(&result),
                        &authored.cuts,
                    )?;
                    let volume = solid.volume_mm3();
                    (
                        solid,
                        result.axis_length_mm(),
                        CUT_VOLUME_ALGORITHM,
                        Some(volume),
                    )
                }
            }
        }
    };
    let (section_area_mm2, area_source) = profile.effective_area();
    // A cut member's volume is the certified cut result (A03); otherwise the P-Mass gross
    // volume `effective area x axis length` (no operation on an uncut member yet).
    let volume_mm3 = cut_volume_mm3.unwrap_or(section_area_mm2 * axis_length_mm);
    if !volume_mm3.is_finite() || volume_mm3 <= 0.0 {
        return Err(CommandError::new(
            ErrorCode::UnsupportedGeometry,
            "profile",
            "section area or volume is not a positive finite number",
        ));
    }
    Ok(MemberBody {
        solid,
        profile,
        width_mm,
        height_mm,
        axis_length_mm,
        section_area_mm2,
        area_source,
        volume_mm3,
        algorithm,
    })
}

/// The quantity records of a member body with the physical material of the element: gross is the
/// effective area times the axis length, and net is the same (no operation on a member yet).
pub fn member_quantities(
    body: &MemberBody,
    material: Option<MaterialInfo>,
) -> Result<Quantities, CommandError> {
    let mut quantities = quantities_with_material(
        Volumes::unchanged(body.volume_mm3),
        body.algorithm,
        material,
    )?;
    quantities.section_area_mm2 = Some(body.section_area_mm2);
    quantities.area_source = Some(body.area_source);
    Ok(quantities)
}

/// Everything one accepted `CreateStraightMember` produces for the layers and the host.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateMemberOutcome {
    /// The authored state for the model layer.
    pub authored: AuthoredMember,
    /// The evaluated body for the derived layer.
    pub body: MemberBody,
    /// Quantity records.
    pub quantities: Quantities,
    /// Declared read and write sets.
    pub footprint: Footprint,
    /// Digest of the typed arguments, for replay and reuse decisions.
    pub fingerprint: ContentDigest,
}

/// Applies `CreateStraightMember` to an isolated draft in memory: argument admission, duplicate
/// id, the pinned profile (required) and material (optional) in that order, then geometry.
/// Nothing is published.
pub fn create_straight_member(
    command: &CreateStraightMember,
    existing: &dyn Fn(&ElementId) -> bool,
    profiles: ProfileLookup<'_>,
    materials: MaterialLookup<'_>,
) -> Result<CreateMemberOutcome, CommandError> {
    command.validate_arguments()?;
    if existing(&command.new_id) {
        return Err(CommandError::new(
            ErrorCode::DuplicateId,
            "new_id",
            "element id is already owned",
        ));
    }
    let profile = profiles(&command.profile_digest)?;
    let material = command
        .material_digest
        .as_ref()
        .map(materials)
        .transpose()?;
    let footprint = Footprint::for_create_member(
        command.new_id,
        command.profile_digest,
        command.material_digest,
    )?;
    let authored = AuthoredMember::from_command(command);
    let body = evaluate_member(&authored, profile)?;
    let quantities = member_quantities(&body, material)?;
    Ok(CreateMemberOutcome {
        fingerprint: command.content_fingerprint()?,
        authored,
        body,
        quantities,
        footprint,
    })
}

/// One accepted member command, bound to its operation.
#[derive(Debug, Clone, PartialEq)]
pub struct MemberTransaction {
    /// The operation id bound to this application.
    pub op_id: OpId,
    /// Authored state, body, quantities, footprint and fingerprint.
    pub outcome: CreateMemberOutcome,
}

/// The decision of [`admit_create_member`].
#[derive(Debug, Clone, PartialEq)]
pub enum MemberAdmission {
    /// A new operation: apply it.
    Apply(Box<MemberTransaction>),
    /// The same operation with identical content was already accepted: change nothing.
    Replay,
}

/// Admits one `CreateStraightMember`: an already accepted `op_id` is decided first (identical
/// content is a replay, different content `OP_ID_REUSE`), a new one is validated and evaluated.
pub fn admit_create_member(
    op_id: OpId,
    command: &CreateStraightMember,
    prior: PriorOp,
    existing: &dyn Fn(&ElementId) -> bool,
    profiles: ProfileLookup<'_>,
    materials: MaterialLookup<'_>,
) -> Result<MemberAdmission, CommandError> {
    match prior {
        PriorOp::Accepted { fingerprint } => {
            if command.content_fingerprint()? == fingerprint {
                Ok(MemberAdmission::Replay)
            } else {
                Err(CommandError::new(
                    ErrorCode::OpIdReuse,
                    "op_id",
                    "operation id was accepted with different content",
                ))
            }
        }
        PriorOp::Unseen => Ok(MemberAdmission::Apply(Box::new(MemberTransaction {
            op_id,
            outcome: create_straight_member(command, existing, profiles, materials)?,
        }))),
    }
}

/// Everything one accepted `CreatePolybeam` produces for the layers and the host; the outcome
/// record is the straight member's.
pub type CreatePolybeamOutcome = CreateMemberOutcome;

/// Applies `CreatePolybeam` to an isolated draft in memory: argument admission, duplicate id,
/// the pinned profile (required) and material (optional) in that order, then the swept
/// geometry. Nothing is published.
pub fn create_polybeam(
    command: &CreatePolybeam,
    existing: &dyn Fn(&ElementId) -> bool,
    profiles: ProfileLookup<'_>,
    materials: MaterialLookup<'_>,
) -> Result<CreatePolybeamOutcome, CommandError> {
    command.validate_arguments()?;
    if existing(&command.new_id) {
        return Err(CommandError::new(
            ErrorCode::DuplicateId,
            "new_id",
            "element id is already owned",
        ));
    }
    let profile = profiles(&command.profile_digest)?;
    let material = command
        .material_digest
        .as_ref()
        .map(materials)
        .transpose()?;
    let footprint = Footprint::for_create_member(
        command.new_id,
        command.profile_digest,
        command.material_digest,
    )?;
    let authored = AuthoredMember::from_polybeam(command);
    let body = evaluate_member(&authored, profile)?;
    let quantities = member_quantities(&body, material)?;
    Ok(CreatePolybeamOutcome {
        fingerprint: command.content_fingerprint()?,
        authored,
        body,
        quantities,
        footprint,
    })
}

/// Admits one `CreatePolybeam`: the replay decision of [`admit_create_member`] with the P12
/// content fingerprint.
pub fn admit_create_polybeam(
    op_id: OpId,
    command: &CreatePolybeam,
    prior: PriorOp,
    existing: &dyn Fn(&ElementId) -> bool,
    profiles: ProfileLookup<'_>,
    materials: MaterialLookup<'_>,
) -> Result<MemberAdmission, CommandError> {
    match prior {
        PriorOp::Accepted { fingerprint } => {
            if command.content_fingerprint()? == fingerprint {
                Ok(MemberAdmission::Replay)
            } else {
                Err(CommandError::new(
                    ErrorCode::OpIdReuse,
                    "op_id",
                    "operation id was accepted with different content",
                ))
            }
        }
        PriorOp::Unseen => Ok(MemberAdmission::Apply(Box::new(MemberTransaction {
            op_id,
            outcome: create_polybeam(command, existing, profiles, materials)?,
        }))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::no_materials;
    use crate::footprint::{Group, Target};
    use beamcraft_model::digest::Sha256;

    fn digest(text: &[u8]) -> ContentDigest {
        Sha256::digest(text).unwrap()
    }

    fn command(profile: ContentDigest) -> CreateStraightMember {
        CreateStraightMember {
            new_id: ElementId::parse("019a7bf0-0000-7000-8000-000000000007").unwrap(),
            kind: MemberKind::Beam,
            name: "Synthetic beam".into(),
            class: 0,
            axis_points_mm: [[0.0; 3], [6000.0, 0.0, 0.0]],
            profile_digest: profile,
            placement: Placement::IDENTITY,
            roll_rad: None,
            material_digest: None,
        }
    }

    fn none(_: &ElementId) -> bool {
        false
    }

    fn rectangle(
        digest: &ContentDigest,
        known: ContentDigest,
    ) -> Result<ProfileInfo, CommandError> {
        if *digest == known {
            Ok(ProfileInfo::Rectangle {
                width_mm: 200.0,
                height_mm: 300.0,
                declared_area_mm2: None,
            })
        } else {
            no_profiles(digest)
        }
    }

    #[test]
    fn a_declared_section_area_beyond_one_percent_of_the_outline_is_a_warning() {
        use beamcraft_model::definition::SectionProperties;
        use beamcraft_model::section::{ParametricSection, SectionFamily};
        let spec = |area: Option<f64>| SectionProfileSpec {
            shape: SectionShape::Parametric(
                ParametricSection::new(SectionFamily::Chs, vec![114.0, 5.0]).unwrap(),
            ),
            section: SectionProperties {
                area_mm2: area,
                ..SectionProperties::default()
            },
        };
        // The tube is a ring: pi / 4 * (114^2 - 104^2). The warning compares with that exact
        // area, not with the polygon of the mesh.
        let ring = std::f64::consts::FRAC_PI_4 * (114.0 * 114.0 - 104.0 * 104.0);
        let polygon = section_outline_area_mm2(&spec(None).shape).unwrap();
        assert!((polygon - ring).abs() < 0.01 * ring, "{polygon} vs {ring}");
        let exact = spec(None).shape.exact_area_mm2();
        assert!((exact - ring).abs() <= 1e-9 * ring);
        assert_eq!(declared_section_area_warning(&spec(None)), None);
        assert_eq!(
            declared_section_area_warning(&spec(Some(ring.round()))),
            None
        );
        // A declared area that matches the (slightly larger) polygon but is further than one
        // percent from the exact area would be flagged; the exact area is the reference.
        for declared in [1.05 * ring, 0.95 * ring] {
            let warning = declared_section_area_warning(&spec(Some(declared))).unwrap();
            assert_eq!(warning.declared_mm2, declared);
            assert_eq!(warning.kernel_mm2, exact);
            assert!(warning.relative_difference > 0.03 && warning.threshold == 0.01);
        }
        assert_eq!(
            declared_section_area_warning(&spec(Some(exact * 1.0099))),
            None
        );
        assert!(declared_section_area_warning(&spec(Some(exact * 1.0101))).is_some());
    }

    fn i_section(r: f64) -> SectionShape {
        use beamcraft_model::section::{ParametricSection, SectionFamily};
        SectionShape::Parametric(
            ParametricSection::new(SectionFamily::IShape, vec![298.0, 149.0, 5.5, 8.0, r]).unwrap(),
        )
    }

    fn member_with(profile: ProfileInfo, density: Option<f64>, length: f64) -> CreateMemberOutcome {
        let digest = digest(b"profile");
        let lookup = |d: &ContentDigest| {
            if *d == digest {
                Ok(profile.clone())
            } else {
                no_profiles(d)
            }
        };
        let materials = |_: &ContentDigest| {
            Ok(MaterialInfo {
                density_kg_m3: density,
            })
        };
        let mut c = command(digest);
        c.axis_points_mm = [[0.0; 3], [length, 0.0, 0.0]];
        c.material_digest = Some(self::digest(b"steel"));
        create_straight_member(&c, &none, &lookup, &materials).unwrap()
    }

    #[test]
    fn the_exact_area_replaces_the_polygon_and_the_old_value_overstated() {
        let exact = 3935.0 + (4.0 - core::f64::consts::PI) * 169.0;
        let out = member_with(
            ProfileInfo::Section {
                shape: i_section(13.0),
                declared_area_mm2: None,
            },
            Some(7850.0),
            6000.0,
        );
        let q = out.quantities;
        assert_eq!(q.area_source, Some(AreaSource::Computed));
        assert!((q.section_area_mm2.unwrap() - exact).abs() <= 1e-9 * exact);
        assert!((q.volume_mm3 - exact * 6000.0).abs() <= 1e-9 * exact * 6000.0);
        let mass = q.mass_kg.unwrap();
        assert!((mass - 192.168).abs() < 0.01, "{mass}");
        // Without operations gross and net are one value.
        assert_eq!(q.volume_gross_mm3, q.volume_mm3);
        assert_eq!(q.mass_gross_kg, q.mass_kg);
        assert_eq!(q.removed_volume_mm3, 0.0);
        // The mesh keeps the detailed polygon: its volume overstates the exact one (the old
        // quantity, 192.58 kg), by less than the arc bound of four fillets.
        let mesh = out.body.solid.volume_mm3();
        assert!(mesh > q.volume_mm3);
        assert!((mesh - q.volume_mm3) / q.volume_mm3 < 0.0025);
        let old_mass = mesh * 7850.0 / 1e9;
        assert!((old_mass - 192.58).abs() < 0.02, "{old_mass}");
    }

    #[test]
    fn a_declared_catalogue_area_wins_over_the_computed_one() {
        let out = member_with(
            ProfileInfo::Section {
                shape: i_section(13.0),
                declared_area_mm2: Some(4080.0),
            },
            Some(7850.0),
            6000.0,
        );
        let q = out.quantities;
        assert_eq!(q.area_source, Some(AreaSource::Catalogue));
        assert_eq!(q.section_area_mm2, Some(4080.0));
        assert_eq!(q.volume_mm3, 4080.0 * 6000.0);
        assert!((q.mass_kg.unwrap() - 192.168).abs() < 0.001);
        // A hostile declared area from a committed definition is refused, never used.
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e30] {
            let lookup = |_: &ContentDigest| {
                Ok(ProfileInfo::Section {
                    shape: i_section(13.0),
                    declared_area_mm2: Some(bad),
                })
            };
            assert!(
                create_straight_member(&command(digest(b"p")), &none, &lookup, &no_materials)
                    .is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_rectangle_is_width_times_height_unless_an_area_is_declared() {
        let computed = member_with(
            ProfileInfo::Rectangle {
                width_mm: 200.0,
                height_mm: 300.0,
                declared_area_mm2: None,
            },
            Some(7850.0),
            6000.0,
        );
        assert_eq!(computed.quantities.area_source, Some(AreaSource::Computed));
        assert_eq!(computed.quantities.section_area_mm2, Some(60_000.0));
        assert_eq!(computed.quantities.volume_mm3, 360_000_000.0);
        let declared = member_with(
            ProfileInfo::Rectangle {
                width_mm: 200.0,
                height_mm: 300.0,
                declared_area_mm2: Some(60_030.0),
            },
            Some(7850.0),
            6000.0,
        );
        assert_eq!(declared.quantities.area_source, Some(AreaSource::Catalogue));
        assert_eq!(declared.quantities.volume_mm3, 60_030.0 * 6000.0);
    }

    #[test]
    fn a_rectangle_refuses_a_declared_area_that_disagrees_with_width_times_height() {
        for declared in [1.0, 59_000.0, 61_000.0, 1e9] {
            let lookup = |_: &ContentDigest| {
                Ok(ProfileInfo::Rectangle {
                    width_mm: 200.0,
                    height_mm: 300.0,
                    declared_area_mm2: Some(declared),
                })
            };
            let error =
                create_straight_member(&command(digest(b"p")), &none, &lookup, &no_materials)
                    .unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidArgument, "{declared}");
        }
        // Within 0.1 percent it is accepted (the catalogue value wins).
        let lookup = |_: &ContentDigest| {
            Ok(ProfileInfo::Rectangle {
                width_mm: 200.0,
                height_mm: 300.0,
                declared_area_mm2: Some(60_030.0),
            })
        };
        assert!(
            create_straight_member(&command(digest(b"p")), &none, &lookup, &no_materials).is_ok()
        );
    }

    #[test]
    fn a_custom_outline_uses_its_shoelace_area() {
        use beamcraft_model::section::CustomSection;
        let out = member_with(
            ProfileInfo::Section {
                shape: SectionShape::Custom(CustomSection {
                    outer: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]],
                    cavities: vec![vec![[40.0, 40.0], [40.0, 60.0], [60.0, 60.0], [60.0, 40.0]]],
                }),
                declared_area_mm2: None,
            },
            None,
            1000.0,
        );
        assert_eq!(out.quantities.section_area_mm2, Some(9_600.0));
        assert_eq!(out.quantities.volume_mm3, 9_600_000.0);
        assert_eq!(out.quantities.mass_kg, None);
    }

    #[test]
    fn a_section_family_beam_is_the_kernel_detailed_outline_extruded() {
        use beamcraft_model::section::{ParametricSection, SectionFamily};
        let profile = digest(b"i-shape 300");
        let shape = SectionShape::Parametric(
            ParametricSection::new(SectionFamily::IShape, vec![300.0, 150.0, 7.0, 10.0, 15.0])
                .unwrap(),
        );
        let lookup = |d: &ContentDigest| {
            if *d == profile {
                Ok(ProfileInfo::Section {
                    shape: shape.clone(),
                    declared_area_mm2: None,
                })
            } else {
                no_profiles(d)
            }
        };
        let out = create_straight_member(&command(profile), &none, &lookup, &no_materials).unwrap();
        assert_eq!((out.body.width_mm, out.body.height_mm), (150.0, 300.0));
        assert_eq!(out.body.algorithm, SECTION_VOLUME_ALGORITHM);
        assert!(out.body.solid.vertices().len() > 8);
        // Area of the sharp I plus four root fillets, exactly; the mesh is only close to it.
        let area =
            2.0 * 150.0 * 10.0 + 280.0 * 7.0 + 4.0 * (1.0 - core::f64::consts::FRAC_PI_4) * 225.0;
        assert!((out.body.volume_mm3 / 6000.0 - area).abs() <= 1e-9 * area);
        assert!((out.body.solid.volume_mm3() / 6000.0 - area).abs() < 25.0);
        assert_eq!(out.quantities.volume_mm3, out.body.volume_mm3);
        // A bad committed definition is refused by the same admission as a command.
        let bad = |d: &ContentDigest| {
            let _ = d;
            Ok(ProfileInfo::Section {
                shape: SectionShape::Parametric(
                    ParametricSection::new(
                        SectionFamily::IShape,
                        vec![300.0, 150.0, 7.0, 200.0, 15.0],
                    )
                    .unwrap(),
                ),
                declared_area_mm2: None,
            })
        };
        assert_eq!(
            create_straight_member(&command(profile), &none, &bad, &no_materials)
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn a_beam_gets_its_volume_from_the_pinned_section_and_the_axis() {
        let profile = digest(b"rectangle 200x300");
        let lookup = |d: &ContentDigest| rectangle(d, profile);
        let out = create_straight_member(&command(profile), &none, &lookup, &no_materials).unwrap();
        // 200 x 300 mm section along a 6000 mm axis: 360,000,000 mm3 = 0.36 m3.
        assert_eq!(out.quantities.volume_mm3, 360_000_000.0);
        assert!((out.quantities.volume_m3 - 0.36).abs() <= 1e-12);
        assert_eq!(
            out.quantities.mass_status,
            crate::command::QuantityStatus::Unknown
        );
        assert_eq!(out.quantities.algorithm, MEMBER_VOLUME_ALGORITHM);
        assert_eq!(out.authored.to_command(), Some(command(profile)));
        assert_eq!(out.body.width_mm, 200.0);
        assert!(out.footprint.reads().iter().any(|e| {
            e.target == Target::Definition(profile) && e.group == Group::Relationship
        }));
    }

    #[test]
    fn a_plane_cut_trims_a_member_of_any_section() {
        use beamcraft_model::command::KeepSide;
        use beamcraft_model::id::OpId;
        use beamcraft_model::section::{ParametricSection, SectionFamily};
        let profile = ProfileInfo::Rectangle {
            width_mm: 200.0,
            height_mm: 300.0,
            declared_area_mm2: None,
        };
        let record = |point: [f64; 3]| CutRecord {
            operation_id: OpId::parse("019a7bf0-0000-7000-8000-0000000000c7").unwrap(),
            ordinal: 0,
            point_mm: point,
            normal: [1.0, 0.0, 0.0],
            keep: KeepSide::Negative,
        };
        let authored = AuthoredMember {
            id: ElementId::parse("019a7bf0-0000-7000-8000-000000000007").unwrap(),
            kind: MemberKind::Beam,
            name: "Cut beam".into(),
            axis_points_mm: vec![[0.0; 3], [6000.0, 0.0, 0.0]],
            profile: digest(b"profile"),
            roll_rad: None,
            placement: Placement::IDENTITY,
            material: None,
            class: 0,
            cuts: vec![record([3000.0, 0.0, 0.0])],
        };
        let body = evaluate_member(&authored, profile.clone()).unwrap();
        // The kept half is 3000 mm long: 3000 * 200 * 300 = 180,000,000 mm3 = 0.18 m3; the cut
        // result replaces the P-Mass `area x length` volume with the certified cut volume.
        let quantities = member_quantities(&body, None).unwrap();
        assert!((quantities.volume_mm3 - 180_000_000.0).abs() <= 1e-6);
        assert_eq!(quantities.algorithm, CUT_VOLUME_ALGORITHM);
        // A cut section member is the certified cut of the detailed outline, about half of it.
        let shape = SectionShape::Parametric(
            ParametricSection::new(SectionFamily::IShape, vec![300.0, 150.0, 7.0, 10.0, 15.0])
                .unwrap(),
        );
        let uncut = evaluate_member(
            &AuthoredMember {
                cuts: Vec::new(),
                ..authored.clone()
            },
            ProfileInfo::Section {
                shape: shape.clone(),
                declared_area_mm2: None,
            },
        )
        .unwrap();
        let cut = evaluate_member(
            &authored,
            ProfileInfo::Section {
                shape,
                declared_area_mm2: None,
            },
        )
        .unwrap();
        assert_eq!(cut.algorithm, CUT_VOLUME_ALGORITHM);
        // The uncut quantity is the P-Mass exact-area volume; the cut quantity is the certified
        // volume of the polygon mesh, whose arc approximation (sagitta at most max(0.25 mm,
        // 0.5% r)) moves the section area by tens of mm2 (~35 000 mm3 per half here).
        assert!(
            (cut.volume_mm3 - uncut.volume_mm3 / 2.0).abs() <= 50_000.0,
            "cut {} vs uncut/2 {}",
            cut.volume_mm3,
            uncut.volume_mm3 / 2.0
        );
        // A plane that misses the member entirely refuses the evaluation.
        let missing = AuthoredMember {
            cuts: vec![record([9000.0, 0.0, 0.0])],
            ..authored.clone()
        };
        assert_eq!(
            evaluate_member(&missing, profile).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn a_pinned_material_with_density_gives_the_mass() {
        let profile = digest(b"rectangle 200x300");
        let steel = digest(b"steel");
        let profiles = |d: &ContentDigest| rectangle(d, profile);
        let materials = |d: &ContentDigest| {
            if *d == steel {
                Ok(MaterialInfo {
                    density_kg_m3: Some(7850.0),
                })
            } else {
                no_materials(d)
            }
        };
        let mut c = command(profile);
        c.material_digest = Some(steel);
        let out = create_straight_member(&c, &none, &profiles, &materials).unwrap();
        // 0.36 m3 * 7850 kg/m3 = 2826 kg.
        assert!((out.quantities.mass_kg.unwrap() - 2826.0).abs() <= 1e-9);
    }

    #[test]
    fn the_profile_is_required_and_resolved_before_geometry() {
        let profile = digest(b"rectangle 200x300");
        let lookup = |d: &ContentDigest| rectangle(d, profile);
        let mut c = command(digest(b"absent"));
        let code = |c: &CreateStraightMember| {
            create_straight_member(c, &none, &lookup, &no_materials)
                .unwrap_err()
                .code
        };
        assert_eq!(code(&c), ErrorCode::NotFound);
        c.axis_points_mm[1] = [0.0; 3];
        assert_eq!(code(&c), ErrorCode::NotFound);
        c.profile_digest = profile;
        assert_eq!(code(&c), ErrorCode::InvalidArgument); // zero axis, F04 section 3 (G-03)
        let mut c = command(profile);
        c.material_digest = Some(digest(b"absent material"));
        assert_eq!(code(&c), ErrorCode::NotFound);
    }

    #[test]
    fn a_rigid_member_rotation_keeps_the_local_mesh_and_quantities() {
        let profile = digest(b"rectangle 200x300");
        let lookup = |d: &ContentDigest| rectangle(d, profile);
        let mut c = command(profile);
        let s = core::f64::consts::FRAC_1_SQRT_2;
        c.placement.orientation_xyzw = [0.0, 0.0, s, s];
        c.placement.translation_mm = [-50.0, 70.0, 1000.0];
        let rotated = create_straight_member(&c, &none, &lookup, &no_materials).unwrap();
        let identity = create_straight_member(
            &CreateStraightMember {
                placement: beamcraft_model::command::Placement::IDENTITY,
                ..c.clone()
            },
            &none,
            &lookup,
            &no_materials,
        )
        .unwrap();
        // Quantities and the analytic dimensions are invariant under a rigid rotation.
        assert_eq!(rotated.quantities, identity.quantities);
        assert_eq!(rotated.body.axis_length_mm, identity.body.axis_length_mm);
        assert_eq!(rotated.body.volume_mm3, identity.body.volume_mm3);
        assert_eq!(
            rotated.body.section_area_mm2,
            identity.body.section_area_mm2
        );
        assert_eq!(rotated.body.solid, identity.body.solid);
        // `q` and `-q` are the same rotation, and x = y = z = 0 keeps the mesh bytes.
        let mut flip = c.clone();
        flip.placement.orientation_xyzw = [0.0, 0.0, 0.0, -1.0];
        let flipped = create_straight_member(&flip, &none, &lookup, &no_materials).unwrap();
        assert_eq!(flipped.body.solid, identity.body.solid);

        // The remaining assertions in this regression concern the existing refusal order.
        let c = command(profile);
        let taken = |id: &ElementId| *id == c.new_id;
        assert_eq!(
            create_straight_member(&c, &taken, &lookup, &no_materials)
                .unwrap_err()
                .code,
            ErrorCode::DuplicateId
        );
        // A committed definition is untrusted: a profile outside the F04 caps never evaluates.
        for (w, h, expected) in [
            (0.0, 300.0, ErrorCode::InvalidArgument),
            (f64::NAN, 300.0, ErrorCode::NonFiniteNumber),
            (200.0, 2e6, ErrorCode::LimitExceeded),
        ] {
            let authored = AuthoredMember::from_command(&command(profile));
            let error = evaluate_member(
                &authored,
                ProfileInfo::Rectangle {
                    width_mm: w,
                    height_mm: h,
                    declared_area_mm2: None,
                },
            )
            .unwrap_err();
            assert_eq!(error.code, expected, "{w} x {h}");
        }
    }

    #[test]
    fn replay_and_reuse_follow_the_ledger() {
        let profile = digest(b"rectangle 200x300");
        let lookup = |d: &ContentDigest| rectangle(d, profile);
        let op = OpId::parse("019a7bf0-0000-7000-8000-000000000004").unwrap();
        let accepted = PriorOp::Accepted {
            fingerprint: command(profile).content_fingerprint().unwrap(),
        };
        assert_eq!(
            admit_create_member(
                op,
                &command(profile),
                accepted,
                &none,
                &lookup,
                &no_materials
            )
            .unwrap(),
            MemberAdmission::Replay
        );
        let mut changed = command(profile);
        changed.axis_points_mm[1][0] = 5000.0;
        assert_eq!(
            admit_create_member(op, &changed, accepted, &none, &lookup, &no_materials)
                .unwrap_err()
                .code,
            ErrorCode::OpIdReuse
        );
        assert!(matches!(
            admit_create_member(
                op,
                &command(profile),
                PriorOp::Unseen,
                &none,
                &lookup,
                &no_materials
            )
            .unwrap(),
            MemberAdmission::Apply(_)
        ));
    }

    fn polybeam_command(profile: ContentDigest) -> CreatePolybeam {
        CreatePolybeam {
            new_id: ElementId::parse("019a7bf0-0000-7000-8000-000000000017").unwrap(),
            kind: MemberKind::Beam,
            name: "L polybeam".into(),
            class: 0,
            // An L: a 3000 mm leg, a right-angle mitre, a 4000 mm leg.
            path_points_mm: vec![[0.0; 3], [3000.0, 0.0, 0.0], [3000.0, 4000.0, 0.0]],
            profile_digest: profile,
            placement: Placement::IDENTITY,
            material_digest: None,
        }
    }

    fn polybeam_with(profile: ProfileInfo, density: Option<f64>) -> CreateMemberOutcome {
        let digest = digest(b"profile");
        let lookup = |d: &ContentDigest| {
            if *d == digest {
                Ok(profile.clone())
            } else {
                no_profiles(d)
            }
        };
        let materials = |_: &ContentDigest| {
            Ok(MaterialInfo {
                density_kg_m3: density,
            })
        };
        let mut c = polybeam_command(digest);
        c.material_digest = Some(self::digest(b"steel"));
        create_polybeam(&c, &none, &lookup, &materials).unwrap()
    }

    #[test]
    fn a_polybeam_volume_is_area_times_path_length_across_the_mitre() {
        // P12 acceptance: the section is constant along the path and the mitre drops out of the
        // volume (G12 corollary 1). 200 x 300 over 3000 + 4000 mm.
        let area = 200.0 * 300.0;
        let length = 7000.0;
        let out = polybeam_with(
            ProfileInfo::Rectangle {
                width_mm: 200.0,
                height_mm: 300.0,
                declared_area_mm2: None,
            },
            Some(7850.0),
        );
        assert_eq!(out.body.algorithm, POLYBEAM_VOLUME_ALGORITHM);
        assert_eq!(out.body.axis_length_mm, length);
        assert!((out.body.volume_mm3 - area * length).abs() <= 1e-9 * area * length);
        assert_eq!(out.quantities.volume_mm3, out.body.volume_mm3);
        // The certified mesh agrees with the analytic mitre volume to the seal slack.
        let mesh = out.body.solid.volume_mm3();
        assert!(
            (mesh - area * length).abs() <= 1e-6 * area * length,
            "{mesh}"
        );
        assert_eq!(out.quantities.mass_kg, Some(area * length * 7850.0 / 1e9));
        // A straight member does not masquerade as a polybeam, and the command round-trips.
        let straight = AuthoredMember::from_command(&command(digest(b"profile")));
        assert!(straight.to_polybeam_command().is_none());
        assert_eq!(straight.to_command(), Some(command(digest(b"profile"))));
        assert!(out.authored.to_command().is_none());
        let mut same = polybeam_command(out.authored.profile);
        same.material_digest = Some(self::digest(b"steel"));
        assert_eq!(out.authored.to_polybeam_command().unwrap(), same);
    }

    #[test]
    fn a_polybeam_of_a_section_family_sweeps_its_detailed_outline() {
        let out = polybeam_with(
            ProfileInfo::Section {
                shape: i_section(13.0),
                declared_area_mm2: None,
            },
            Some(7850.0),
        );
        let exact = 3935.0 + (4.0 - core::f64::consts::PI) * 169.0;
        let (width_mm, height_mm) = i_section(13.0).extent_mm();
        assert_eq!(
            (out.body.width_mm, out.body.height_mm),
            (width_mm, height_mm)
        );
        assert!((out.body.volume_mm3 - exact * 7000.0).abs() <= 1e-9 * exact * 7000.0);
        // The mesh is the polygon outline (fillet chords overstate the exact area), but the
        // mitre fraction cancels over the centred section: the mesh volume is the polygon area
        // times the path length, within the polygon bound of the straight member.
        let mesh = out.body.solid.volume_mm3();
        assert!(mesh > out.body.volume_mm3, "{mesh}");
        assert!(
            (mesh - out.body.volume_mm3) / out.body.volume_mm3 < 0.0025,
            "{mesh}"
        );
    }

    #[test]
    fn a_polybeam_refuses_a_bad_path_or_profile_and_replays_by_content() {
        let profile = digest(b"rectangle 200x300");
        let lookup = |d: &ContentDigest| rectangle(d, profile);
        // A zero-length chord is the kernel's refusal, not an argument check.
        let mut c = polybeam_command(profile);
        c.path_points_mm = vec![[0.0; 3], [3000.0, 0.0, 0.0], [3000.0, 0.0, 0.0]];
        assert_eq!(
            create_polybeam(&c, &none, &lookup, &no_materials)
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        // An absent profile is `NOT_FOUND` before any geometry.
        let c = polybeam_command(digest(b"absent"));
        assert_eq!(
            create_polybeam(&c, &none, &lookup, &no_materials)
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
        // Replay follows the P12 content fingerprint.
        let op = OpId::parse("019a7bf0-0000-7000-8000-000000000004").unwrap();
        let accepted = PriorOp::Accepted {
            fingerprint: polybeam_command(profile).content_fingerprint().unwrap(),
        };
        assert_eq!(
            admit_create_polybeam(
                op,
                &polybeam_command(profile),
                accepted,
                &none,
                &lookup,
                &no_materials
            )
            .unwrap(),
            MemberAdmission::Replay
        );
        let mut changed = polybeam_command(profile);
        changed.path_points_mm[1][1] = 1000.0;
        assert_eq!(
            admit_create_polybeam(op, &changed, accepted, &none, &lookup, &no_materials)
                .unwrap_err()
                .code,
            ErrorCode::OpIdReuse
        );
    }

    #[test]
    fn a_polybeam_cut_is_the_certified_replay_volume() {
        use beamcraft_model::command::KeepSide;
        let profile = ProfileInfo::Rectangle {
            width_mm: 200.0,
            height_mm: 300.0,
            declared_area_mm2: None,
        };
        let mut authored = AuthoredMember::from_polybeam(&polybeam_command(digest(b"profile")));
        authored.cuts = vec![CutRecord {
            operation_id: OpId::parse("019a7bf0-0000-7000-8000-0000000000c7").unwrap(),
            ordinal: 0,
            point_mm: [4000.0, 1000.0, 0.0],
            normal: [0.0, 1.0, 0.0],
            keep: KeepSide::Negative,
        }];
        let body = evaluate_member(&authored, profile).unwrap();
        // The plane keeps the first leg and a 1000 mm stub of the second: the certified replay
        // volume replaces the analytic `area x path length`.
        assert_eq!(body.algorithm, CUT_VOLUME_ALGORITHM);
        let analytic = 200.0 * 300.0 * 4000.0;
        assert!(
            (body.volume_mm3 - analytic).abs() <= 1e-6 * analytic,
            "{}",
            body.volume_mm3
        );
    }
}
