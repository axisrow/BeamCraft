// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! F04 section 7 command arguments and their model-level admission.
//!
//! P01 implements `CreatePlate` only. This module holds what is true of the arguments
//! regardless of geometry: identity, label, finiteness, caps and the rigid placement. Contour
//! and thickness *geometry* (floor, winding, supported shapes) is evaluated by the engine with
//! the kernel. No JSON, envelope or canonical-byte form lives here yet: F04 sections 3 and 6
//! (`canonical`) remain planned, and [`CreatePlate::content_fingerprint`] is a
//! deterministic digest of the typed arguments, not RFC 8785 canonical bytes.

use crate::digest::{ContentDigest, Sha256};
use crate::error::ErrorCode;
pub use crate::grid::CreateGrid;
pub use crate::grid_update::UpdateGrid;
use crate::id::{ElementId, OpId};
use crate::limits::{
    BOLT_DIAMETER_CAP_MM, CONTOUR_POINT_CAP, COORDINATE_CAP_MM, DIMENSION_CAP_MM,
    MAX_BOLT_GROUP_SIZE, MAX_LABEL_BYTES, MAX_MEMBER_PATH_POINTS, MAX_WELD_LENGTH_MM,
    MAX_WELD_PATH_POINTS, MIN_CONTOUR_POINTS, MIN_LABEL_BYTES, MIN_MEMBER_PATH_POINTS,
    MIN_WELD_PATH_POINTS, MIN_WELD_SEGMENT_MM, ORIENTATION_NORM_TOLERANCE, ROLL_CAP_RAD,
    WELD_SIZE_CAP_MM,
};
use beamcraft_geom::canonical_f64_bytes;
use core::fmt;

/// The only contract version this implementation accepts.
pub const CONTRACT_VERSION: &str = "0.1.0";

/// A typed rejection: a stable F04 code, a fixed field token and a fixed safe message. It never
/// carries caller text, paths or credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandError {
    /// F10 rule for domain admission failures, absent for other errors.
    pub rule_id: Option<&'static str>,
    /// Stable F04 code.
    pub code: ErrorCode,
    /// Fixed schema token of the offending field, for example `contour_points_mm`.
    pub field: &'static str,
    /// Fixed ASCII explanation.
    pub message: &'static str,
}

impl CommandError {
    /// A rejection with the given code, field and message.
    pub const fn new(code: ErrorCode, field: &'static str, message: &'static str) -> Self {
        Self {
            code,
            field,
            message,
            rule_id: None,
        }
    }
    /// Retains the exact F10 rule through engine and CLI admission.
    pub const fn with_rule(mut self, rule_id: &'static str) -> Self {
        self.rule_id = Some(rule_id);
        self
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {} ({})", self.code, self.message, self.field)
    }
}

impl core::error::Error for CommandError {}

/// A rigid pose (F04 section 3): `world = R(q) * local + translation_mm`, quaternion in
/// `(x, y, z, w)` order. The components are authored as submitted; the accepted quaternion is
/// normalized only where it is used.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// Translation in millimetres, each component at most 1,000,000,000 in magnitude.
    pub translation_mm: [f64; 3],
    /// Quaternion components `(x, y, z, w)`, each in `[-1, 1]`, squared norm within 1e-6 of one.
    pub orientation_xyzw: [f64; 4],
}

impl Placement {
    /// No translation and the identity rotation.
    pub const IDENTITY: Self = Self {
        translation_mm: [0.0; 3],
        orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
    };

    /// Checks finiteness, the translation cap and the quaternion rules, in that order.
    ///
    /// Non-finite is `NON_FINITE_NUMBER`; a translation above the cap is `LIMIT_EXCEEDED`; a
    /// quaternion component outside `[-1, 1]` or a squared norm farther than 1e-6 from one is
    /// `INVALID_ARGUMENT`.
    pub fn validate(&self) -> Result<(), CommandError> {
        let finite = |values: &[f64], field: &'static str| {
            if values.iter().all(|v| v.is_finite()) {
                Ok(())
            } else {
                Err(
                    CommandError::new(ErrorCode::NonFiniteNumber, field, "value is not finite")
                        .with_rule("DV-ATTR-003"),
                )
            }
        };
        finite(&self.translation_mm, "placement.translation_mm")?;
        finite(&self.orientation_xyzw, "placement.orientation_xyzw")?;
        if self
            .translation_mm
            .iter()
            .any(|v| v.abs() > COORDINATE_CAP_MM)
        {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "placement.translation_mm",
                "translation component exceeds the coordinate cap",
            )
            .with_rule("DV-LIM-001"));
        }
        Self::validate_orientation(self.orientation_xyzw)
    }

    /// Checks a quaternion alone: non-finite is `NON_FINITE_NUMBER`; a component outside
    /// `[-1, 1]` or a squared norm farther than 1e-6 from one is `INVALID_ARGUMENT`.
    pub fn validate_orientation(xyzw: [f64; 4]) -> Result<(), CommandError> {
        if xyzw.iter().any(|v| !v.is_finite()) {
            return Err(CommandError::new(
                ErrorCode::NonFiniteNumber,
                "placement.orientation_xyzw",
                "value is not finite",
            )
            .with_rule("DV-ATTR-003"));
        }
        if xyzw.iter().any(|v| v.abs() > 1.0) {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "placement.orientation_xyzw",
                "quaternion component is outside [-1, 1]",
            )
            .with_rule("DV-LIM-003"));
        }
        let [x, y, z, w] = xyzw;
        let squared = ((x * x + y * y) + z * z) + w * w;
        if (squared - 1.0).abs() > ORIENTATION_NORM_TOLERANCE {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "placement.orientation_xyzw",
                "quaternion squared norm is not within 1e-6 of one",
            )
            .with_rule("DV-LIM-003"));
        }
        Ok(())
    }

    /// Whether the accepted rotation is the identity: `x = y = z = 0` (so `w` is `+-1` up to the
    /// accepted norm tolerance). `q` and `-q` are the same rotation.
    pub fn is_identity_rotation(&self) -> bool {
        let [x, y, z, _] = self.orientation_xyzw;
        x == 0.0 && y == 0.0 && z == 0.0
    }
}

/// Which closed half-space of the cutting plane survives (F04 section 7 `CutByPlane`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepSide {
    /// The side the plane normal points to (`normal . (p - point) >= 0`).
    Positive,
    /// The opposite side (`normal . (p - point) <= 0`).
    Negative,
}

impl KeepSide {
    /// The wire token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Positive => "positive",
            Self::Negative => "negative",
        }
    }

    /// Parses the wire token; anything else is `None`.
    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "positive" => Some(Self::Positive),
            "negative" => Some(Self::Negative),
            _ => None,
        }
    }
}

/// One owned `plane_cut` operation record of an element (F04 section 4, amendment A-2): the
/// plane parameters and the kept side, with the record identity the engine assigned it.
///
/// The plane is the exact set `normal . (p - point_mm) = 0` of the stored binary64 numbers; a
/// scaled normal describes the same plane. Whether the plane actually crosses the element (some
/// vertex strictly on each side, no face in the plane) is the kernel's decision at replay; a
/// tangent or missing plane refuses the command that authors or replays it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CutRecord {
    /// Full UUIDv7 of the record, unused in the project (`DUPLICATE_ID`).
    pub operation_id: OpId,
    /// The per-element ordinal the engine assigned in creation order; it names the cap face
    /// `cut:f.p<ordinal>` and is never reused or renumbered.
    pub ordinal: u16,
    /// A point of the cutting plane, element-local millimetres.
    pub point_mm: [f64; 3],
    /// A non-zero normal of the plane.
    pub normal: [f64; 3],
    /// The kept half-space.
    pub keep: KeepSide,
}

impl CutRecord {
    /// Admits what does not depend on geometry: finiteness, the coordinate cap for the point and
    /// every normal component, and a non-zero normal. A non-finite number is
    /// `NON_FINITE_NUMBER`; a coordinate above 1,000,000,000 mm is `LIMIT_EXCEEDED`; a zero
    /// normal is `INVALID_ARGUMENT`.
    pub fn validate(&self) -> Result<(), CommandError> {
        if self
            .point_mm
            .iter()
            .chain(self.normal.iter())
            .any(|v| !v.is_finite())
        {
            return Err(CommandError::new(
                ErrorCode::NonFiniteNumber,
                "cut",
                "value is not finite",
            )
            .with_rule("DV-ATTR-003"));
        }
        if self
            .point_mm
            .iter()
            .chain(self.normal.iter())
            .any(|v| v.abs() > COORDINATE_CAP_MM)
        {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "cut",
                "coordinate exceeds the coordinate cap",
            )
            .with_rule("DV-LIM-001"));
        }
        if self.normal.iter().all(|v| *v == 0.0) {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "cut",
                "the cutting plane needs a non-zero normal",
            ));
        }
        Ok(())
    }
}

/// F04 section 7 `CutByPlane`: adds one `plane_cut` record to a valid plate, beam or column and
/// updates its derived data in one transaction. `operation_id` must be unused in the project;
/// the engine assigns the next ordinal of the target.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CutByPlane {
    /// The element to cut.
    pub target_id: ElementId,
    /// Full UUIDv7 of the new record; unused in the project.
    pub operation_id: OpId,
    /// A point of the cutting plane, element-local millimetres.
    pub point_mm: [f64; 3],
    /// A non-zero normal of the plane.
    pub normal: [f64; 3],
    /// The kept half-space.
    pub keep: KeepSide,
}

impl CutByPlane {
    /// Admits what does not depend on geometry or model state: finiteness, the coordinate cap
    /// and a non-zero normal, as [`CutRecord::validate`].
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        CutRecord {
            operation_id: self.operation_id,
            ordinal: 0,
            point_mm: self.point_mm,
            normal: self.normal,
            keep: self.keep,
        }
        .validate()
    }

    /// SHA-256 of a versioned, length-prefixed binary encoding of the validated arguments, with
    /// signed zero normalized; the same deterministic digest form as
    /// [`CreatePlate::content_fingerprint`].
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        self.validate_arguments()?;
        let bad =
            || CommandError::new(ErrorCode::NonFiniteNumber, "cut", "value cannot be encoded");
        let mut bytes = Vec::with_capacity(128);
        bytes.extend_from_slice(b"BeamCraft/A03/CutByPlane/v1\0");
        bytes.extend_from_slice(self.target_id.as_bytes());
        bytes.extend_from_slice(self.operation_id.as_bytes());
        bytes.push(match self.keep {
            KeepSide::Positive => 0,
            KeepSide::Negative => 1,
        });
        for v in self.point_mm.into_iter().chain(self.normal) {
            bytes.extend_from_slice(&canonical_f64_bytes(v).map_err(|_| bad())?);
        }
        Sha256::digest(&bytes).map_err(|_| {
            CommandError::new(ErrorCode::LimitExceeded, "content", "content is too large")
                .with_rule("DV-LIM-001")
        })
    }
}

/// Where the thickness of a plate is laid off from its contour plane (A12.1, the F04 plate
/// amendment section 1). The contour lies in the element-local plane `z = 0`; the side names the
/// band of `z` that carries the material. The side is the axis of the element-local frame, not a
/// world direction: a rotated placement turns it with the plate. Volume, area and mass do not
/// depend on it; the mesh does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlateSide {
    /// Material between `z = 0` and `z = +t`. The default, and what every plate authored before
    /// the field existed reads as.
    Top,
    /// Material between `z = -t/2` and `z = +t/2`, half on each side of the contour plane.
    Middle,
    /// Material between `z = -t` and `z = 0`, laid off in `-Z`.
    Bottom,
}

impl PlateSide {
    /// The wire token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Middle => "middle",
            Self::Bottom => "bottom",
        }
    }

    /// Parses the wire token.
    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "top" => Some(Self::Top),
            "middle" => Some(Self::Middle),
            "bottom" => Some(Self::Bottom),
            _ => None,
        }
    }

    /// The element-local `z` of the material's lower face for a plate of the given thickness:
    /// `0`, `-t/2` or `-t`. Halving a finite `f64` and negating it are exact, so the offset is
    /// the authored thickness unchanged.
    pub const fn offset_mm(self, thickness_mm: f64) -> f64 {
        match self {
            Self::Top => 0.0,
            Self::Middle => -thickness_mm / 2.0,
            Self::Bottom => -thickness_mm,
        }
    }
}

/// The treatment of one contour vertex (A12.2, the F04 plate amendment section 2). The authored
/// contour keeps the sharp corner point; the treatment is an authored parameter, so a rounded
/// corner stays editable by its radius and is never a stored polygon.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CornerTreatment {
    /// The corner is replaced by a straight cut. `d1_mm` is measured from the vertex along the
    /// edge towards the **previous** vertex, `d2_mm` along the edge towards the **next** vertex;
    /// equal values give the symmetric cut of a single authored distance.
    Chamfer {
        /// Cut distance along the previous edge, in millimetres.
        d1_mm: f64,
        /// Cut distance along the next edge, in millimetres.
        d2_mm: f64,
    },
    /// The corner is replaced by a circular arc of the given radius, tangent to both edges.
    Round {
        /// Arc radius in millimetres.
        radius_mm: f64,
    },
}

impl CornerTreatment {
    /// The wire token of the treatment kind.
    pub const fn kind(self) -> &'static str {
        match self {
            Self::Chamfer { .. } => "chamfer",
            Self::Round { .. } => "round",
        }
    }
}

/// One contour vertex's corner treatment (A12.2): the vertex is named by its position in the
/// authored contour (amendment section 3, decision O-1: no stable vertex ids).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlateCorner {
    /// Position of the vertex in the authored contour, `0` to point count - 1.
    pub vertex: usize,
    /// The treatment authored on it.
    pub treatment: CornerTreatment,
}

impl PlateCorner {
    /// Admits one corner: the parameters are finite (`NON_FINITE_NUMBER`), greater than zero
    /// (`INVALID_ARGUMENT`) and within the F04 coordinate cap (`LIMIT_EXCEEDED`).
    pub fn validate(&self) -> Result<(), CommandError> {
        let values = match self.treatment {
            CornerTreatment::Chamfer { d1_mm, d2_mm } => [d1_mm, d2_mm],
            CornerTreatment::Round { radius_mm } => [radius_mm, radius_mm],
        };
        if values.iter().any(|v| !v.is_finite()) {
            return Err(CommandError::new(
                ErrorCode::NonFiniteNumber,
                "corner",
                "corner treatment value is not finite",
            )
            .with_rule("DV-ATTR-003"));
        }
        if values.iter().any(|v| *v <= 0.0) {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "corner",
                "corner treatment value is not positive",
            ));
        }
        if values.iter().any(|v| *v > COORDINATE_CAP_MM) {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "corner",
                "corner treatment value exceeds the coordinate cap",
            )
            .with_rule("DV-LIM-001"));
        }
        Ok(())
    }
}

/// Replace the parameters of an existing cut record, preserving its identity and ordinal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UpdateCut {
    /// Target, record identity and replacement plane in element-local millimetres.
    pub cut: CutByPlane,
}
impl UpdateCut {
    /// Validate the replacement independently of the target's geometry.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        self.cut.validate_arguments()
    }
    /// Versioned request identity, distinct from creating the same plane.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        let mut bytes = b"BeamCraft/A03/UpdateCut/v1\0".to_vec();
        bytes.extend_from_slice(self.cut.content_fingerprint()?.as_bytes());
        Sha256::digest(&bytes).map_err(|_| {
            CommandError::new(
                ErrorCode::LimitExceeded,
                "content",
                "cut request exceeds its bound",
            )
        })
    }
}

/// Delete one owned cut record. The logical request identity is separate from the record UUID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoveCut {
    /// Part whose operation list is edited.
    pub target_id: ElementId,
    /// Existing cut record; never interpreted as this request's operation identity.
    pub operation_id: OpId,
}
impl RemoveCut {
    /// Stable typed request digest, distinct from cut creation and replacement.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        let mut bytes = b"BeamCraft/A03/RemoveCut/v1\0".to_vec();
        bytes.extend_from_slice(self.target_id.as_bytes());
        bytes.extend_from_slice(self.operation_id.as_bytes());
        Sha256::digest(&bytes).map_err(|_| {
            CommandError::new(
                ErrorCode::LimitExceeded,
                "content",
                "cut request exceeds its bound",
            )
        })
    }
}

/// F04 section 7 `CreatePlate`. `material_digest` pins an existing immutable material definition;
/// whether the digest resolves (`NOT_FOUND`) is the engine's decision, not an argument check.
#[derive(Debug, Clone, PartialEq)]
pub struct CreatePlate {
    /// The new element's full identity; creation also checks that it is unused.
    pub new_id: ElementId,
    /// Label, 1 to 256 UTF-8 bytes without control characters. Not an identity.
    pub name: String,
    /// Authored integer part class (P15); zero is the default.
    pub class: i32,
    /// Element-local contour, 3 to 1024 points, closure implicit.
    pub contour_points_mm: Vec<[f64; 3]>,
    /// Extrusion thickness in millimetres.
    pub thickness_mm: f64,
    /// Where the thickness is laid off from the contour plane (A12.1); absent means `top`.
    pub thickness_side: Option<PlateSide>,
    /// Corner treatments of authored contour vertices (A12.2); empty means every corner is
    /// sharp. Each vertex is named at most once; the geometry it must really turn is the
    /// engine's decision.
    pub corners: Vec<PlateCorner>,
    /// Rigid placement.
    pub placement: Placement,
    /// Optional pinned material definition.
    pub material_digest: Option<ContentDigest>,
    /// Through-hole polygons in the plate's local XY, each at least three points, clockwise as
    /// seen from +Z (the cutter convention of the kernel). Whether a hole is strictly inside the
    /// outline and clear of the others is the kernel's decision. Empty means a solid plate.
    pub holes_mm: Vec<Vec<[f64; 2]>>,
}

/// Whether `name` is a valid element label (F04 section 3, `DV-BASE-003`).
pub fn label_is_valid(name: &str) -> bool {
    (MIN_LABEL_BYTES..=MAX_LABEL_BYTES).contains(&name.len())
        && !name.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}')
}

/// The explicit member kind of F04 section 7; it is independent of the USD model kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberKind {
    /// A straight beam.
    Beam,
    /// A straight column.
    Column,
}

impl MemberKind {
    /// The `bim:elementKind` token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Beam => "beam",
            Self::Column => "column",
        }
    }
}

/// Where the section of a beam or column sits across the member width (P14), along the rolled
/// local `X_m` of the F06 section 4 frame. The names are frame-relative, not world directions: a
/// rotated placement turns them with the member. `left` puts the section's bounding box in the
/// `-X_m` half of the frame, `right` in the `+X_m` half and `centre` straddles the axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SectionPlanePosition {
    /// The bounding box straddles the axis, from `-w/2` to `+w/2`. The default, and what every
    /// member authored before the field existed reads as.
    #[default]
    Centre,
    /// The bounding box occupies `-X_m`: from `-w` to `0`.
    Left,
    /// The bounding box occupies `+X_m`: from `0` to `+w`.
    Right,
}

impl SectionPlanePosition {
    /// The wire token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Centre => "centre",
            Self::Left => "left",
            Self::Right => "right",
        }
    }

    /// Parses the wire token.
    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "centre" => Some(Self::Centre),
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            _ => None,
        }
    }

    /// Where the bounding-box centre moves along `X_m` for a section of the given width: `0`,
    /// `-w/2` or `+w/2`. Halving a finite `f64` and negating it are exact.
    pub const fn half_shift_mm(self, width_mm: f64) -> f64 {
        match self {
            Self::Centre => 0.0,
            Self::Left => -width_mm / 2.0,
            Self::Right => width_mm / 2.0,
        }
    }
}

/// Where the section of a beam or column sits along the member height (P14), along the rolled
/// local `Y_m` of the F06 section 4 frame. The names are frame-relative, not world directions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SectionDepthPosition {
    /// The bounding box straddles the axis, from `-h/2` to `+h/2`. The default.
    #[default]
    Centre,
    /// The bounding box occupies `-Y_m`: from `-h` to `0`.
    Front,
    /// The bounding box occupies `+Y_m`: from `0` to `+h`.
    Back,
}

impl SectionDepthPosition {
    /// The wire token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Centre => "centre",
            Self::Front => "front",
            Self::Back => "back",
        }
    }

    /// Parses the wire token.
    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "centre" => Some(Self::Centre),
            "front" => Some(Self::Front),
            "back" => Some(Self::Back),
            _ => None,
        }
    }

    /// Where the bounding-box centre moves along `Y_m` for a section of the given height: `0`,
    /// `-h/2` or `+h/2`. Halving a finite `f64` and negating it are exact.
    pub const fn half_shift_mm(self, height_mm: f64) -> f64 {
        match self {
            Self::Centre => 0.0,
            Self::Front => -height_mm / 2.0,
            Self::Back => height_mm / 2.0,
        }
    }
}

/// The authored position of the section about the member axis (P14): where the section's
/// bounding-box centre sits in the rolled element-local section plane and where the solid
/// physically starts and ends along the axis. The axis points stay the modelling line; the
/// offsets move the material. All values are element-local and turn with the roll and the
/// placement. The rotation about the axis is the existing `roll_rad`; it composes with this
/// position the same way it composes with the frame. The default value is the centred, unshifted
/// position: the body every member authored before the field existed reads as, bit for bit.
///
/// The shift of the bounding-box centre is `plane.half_shift_mm(w) + plane_offset_mm` along the
/// rolled `X_m` and `depth.half_shift_mm(h) + depth_offset_mm` along the rolled `Y_m`. The solid
/// spans `p1 + start_offset_mm * u .. p2 + end_offset_mm * u` along the unit axis `u`, so its
/// length is the axis length moved by `end_offset_mm - start_offset_mm` and must stay positive.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SectionPlacement {
    /// Across-width position of the section.
    pub plane: SectionPlanePosition,
    /// Extra shift along `X_m` in millimetres, applied after [`SectionPlanePosition`].
    pub plane_offset_mm: f64,
    /// Along-height position of the section.
    pub depth: SectionDepthPosition,
    /// Extra shift along `Y_m` in millimetres, applied after [`SectionDepthPosition`].
    pub depth_offset_mm: f64,
    /// Offset of the solid's start face along `+u` in millimetres.
    pub start_offset_mm: f64,
    /// Offset of the solid's end face along `+u` in millimetres.
    pub end_offset_mm: f64,
}

impl SectionPlacement {
    /// True when nothing is displaced. A member with the default position evaluates exactly as
    /// it did before the field existed: the same mesh bytes, quantities and signatures.
    pub fn is_default(&self) -> bool {
        self.plane == SectionPlanePosition::Centre
            && self.depth == SectionDepthPosition::Centre
            && self.plane_offset_mm == 0.0
            && self.depth_offset_mm == 0.0
            && self.start_offset_mm == 0.0
            && self.end_offset_mm == 0.0
    }
}

/// F04 section 7 `CreateStraightMember`: a straight member with a pinned profile (required) and
/// an optional pinned material. Whether the digests resolve (`NOT_FOUND`) and the shape (axis
/// length, supported section) are the engine's decisions, not argument checks.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateStraightMember {
    /// The new element's full identity.
    pub new_id: ElementId,
    /// Beam or column.
    pub kind: MemberKind,
    /// Label, 1 to 256 UTF-8 bytes without control characters.
    pub name: String,
    /// Authored integer part class (P15); zero is the default.
    pub class: i32,
    /// The two element-local axis points `p1` and `p2`.
    pub axis_points_mm: [[f64; 3]; 2],
    /// Pinned profile definition.
    pub profile_digest: ContentDigest,
    /// Rigid placement.
    pub placement: Placement,
    /// Explicit roll about the member axis; `None` is zero.
    pub roll_rad: Option<f64>,
    /// The authored position of the section about the axis (P14); absence is the centred
    /// default and stores no attributes.
    pub section_placement: Option<SectionPlacement>,
    /// Optional pinned material definition.
    pub material_digest: Option<ContentDigest>,
}

impl CreateStraightMember {
    /// Admits what does not depend on geometry: label, finiteness, the coordinate and roll caps
    /// and the placement. A non-finite number is `NON_FINITE_NUMBER`; a coordinate above
    /// 1,000,000,000 mm or a roll above 2 pi is `LIMIT_EXCEEDED`.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        if !label_is_valid(&self.name) {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "name",
                "label must be 1 to 256 bytes without control characters",
            )
            .with_rule("DV-BASE-003"));
        }
        let roll = self.roll_rad.unwrap_or(0.0);
        if !roll.is_finite() || self.axis_points_mm.iter().flatten().any(|v| !v.is_finite()) {
            return Err(CommandError::new(
                ErrorCode::NonFiniteNumber,
                "axis_points_mm",
                "value is not finite",
            )
            .with_rule("DV-ATTR-003"));
        }
        if self
            .axis_points_mm
            .iter()
            .flatten()
            .any(|v| v.abs() > COORDINATE_CAP_MM)
        {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "axis_points_mm",
                "coordinate exceeds the coordinate cap",
            )
            .with_rule("DV-LIM-001"));
        }
        if roll.abs() > ROLL_CAP_RAD {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "roll_rad",
                "roll exceeds 2 pi",
            )
            .with_rule("DV-LIM-001"));
        }
        if let Some(placement) = &self.section_placement {
            let offsets = [
                placement.plane_offset_mm,
                placement.depth_offset_mm,
                placement.start_offset_mm,
                placement.end_offset_mm,
            ];
            if offsets.iter().any(|v| !v.is_finite()) {
                return Err(CommandError::new(
                    ErrorCode::NonFiniteNumber,
                    "section_placement",
                    "value is not finite",
                )
                .with_rule("DV-ATTR-003"));
            }
            if offsets.iter().any(|v| v.abs() > DIMENSION_CAP_MM) {
                return Err(CommandError::new(
                    ErrorCode::LimitExceeded,
                    "section_placement",
                    "offset exceeds the dimension cap",
                )
                .with_rule("DV-LIM-001"));
            }
        }
        self.placement.validate()
    }

    /// SHA-256 of a versioned, length-prefixed binary encoding of the validated arguments, with
    /// signed zero normalized; like [`CreatePlate::content_fingerprint`] it is not the F04
    /// canonical form.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        self.validate_arguments()?;
        let bad = || {
            CommandError::new(
                ErrorCode::NonFiniteNumber,
                "axis_points_mm",
                "value cannot be encoded",
            )
            .with_rule("DV-ATTR-003")
        };
        let mut bytes = Vec::with_capacity(256);
        bytes.extend_from_slice(b"BeamCraft/P03/CreateStraightMember/v5\0");
        bytes.extend_from_slice(self.new_id.as_bytes());
        bytes.push(match self.kind {
            MemberKind::Beam => 0,
            MemberKind::Column => 1,
        });
        let name = self.name.as_bytes();
        bytes.extend_from_slice(&(name.len() as u64).to_le_bytes());
        bytes.extend_from_slice(name);
        bytes.extend_from_slice(&self.class.to_le_bytes());
        bytes.extend_from_slice(self.profile_digest.as_bytes());
        match &self.material_digest {
            Some(digest) => {
                bytes.push(1);
                bytes.extend_from_slice(digest.as_bytes());
            }
            None => bytes.push(0),
        }
        match self.roll_rad {
            Some(roll) => {
                bytes.push(1);
                bytes.extend_from_slice(&canonical_f64_bytes(roll).map_err(|_| bad())?);
            }
            None => bytes.push(0),
        }
        match &self.section_placement {
            None => bytes.push(0),
            Some(p) => {
                bytes.push(1);
                bytes.push(match p.plane {
                    SectionPlanePosition::Centre => 0,
                    SectionPlanePosition::Left => 1,
                    SectionPlanePosition::Right => 2,
                });
                bytes.push(match p.depth {
                    SectionDepthPosition::Centre => 0,
                    SectionDepthPosition::Front => 1,
                    SectionDepthPosition::Back => 2,
                });
                for v in [
                    p.plane_offset_mm,
                    p.depth_offset_mm,
                    p.start_offset_mm,
                    p.end_offset_mm,
                ] {
                    bytes.extend_from_slice(&canonical_f64_bytes(v).map_err(|_| bad())?);
                }
            }
        }
        let mut scalars = self
            .axis_points_mm
            .iter()
            .flatten()
            .copied()
            .chain(self.placement.translation_mm)
            .chain(self.placement.orientation_xyzw);
        scalars.try_for_each(|v| {
            bytes.extend_from_slice(&canonical_f64_bytes(v).map_err(|_| bad())?);
            Ok::<(), CommandError>(())
        })?;
        Sha256::digest(&bytes).map_err(|_| {
            CommandError::new(ErrorCode::LimitExceeded, "content", "content is too large")
                .with_rule("DV-LIM-001")
        })
    }
}

/// P13 curved member. `member.path_points_mm` is exactly start, on-arc, end.
/// The three points retain the arc's plane and direction; they are never stored as facets.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateCurvedBeam {
    /// Shared identity, profile, material and pose, with three authored arc points.
    pub member: CreatePolybeam,
}
impl CreateCurvedBeam {
    /// Admit bounded, finite inputs. Circle existence is an engineering check.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        self.member.validate_arguments()?;
        if self.member.path_points_mm.len() != 3 {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "arc_points_mm",
                "an arc requires exactly three points",
            ));
        }
        Ok(())
    }
    /// Request identity in a separate command domain from a three-point polybeam.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        self.validate_arguments()?;
        let mut bytes = b"BeamCraft/P13/CreateCurvedBeam/v1\0".to_vec();
        bytes.extend_from_slice(self.member.content_fingerprint()?.as_bytes());
        Sha256::digest(&bytes).map_err(|_| {
            CommandError::new(
                ErrorCode::LimitExceeded,
                "command",
                "request encoding exceeds capacity",
            )
        })
    }
}

/// P12 `CreatePolybeam`: a member along a polyline path with one section along the whole path.
/// The section is the pinned profile (required), the optional material is a pinned definition,
/// and the path is at least three element-local points whose interior vertices the kernel
/// mitres; a two-point path is a straight member, not a polybeam. Whether the digests resolve
/// (`NOT_FOUND`), the chords are long enough and the section is supported are the engine's
/// decisions, not argument checks.
#[derive(Debug, Clone, PartialEq)]
pub struct CreatePolybeam {
    /// The new element's full identity.
    pub new_id: ElementId,
    /// Beam or column.
    pub kind: MemberKind,
    /// Label, 1 to 256 UTF-8 bytes without control characters.
    pub name: String,
    /// Authored integer part class (P15); zero is the default.
    pub class: i32,
    /// The ordered element-local path points, `MIN_MEMBER_PATH_POINTS` to
    /// `MAX_MEMBER_PATH_POINTS` of them.
    pub path_points_mm: Vec<[f64; 3]>,
    /// Pinned profile definition, one section along the whole path.
    pub profile_digest: ContentDigest,
    /// Rigid placement.
    pub placement: Placement,
    /// Optional pinned material definition.
    pub material_digest: Option<ContentDigest>,
}

impl CreatePolybeam {
    /// Admits what does not depend on geometry: label, point count, finiteness, the coordinate
    /// cap and the placement. A non-finite number is `NON_FINITE_NUMBER`; fewer than
    /// `MIN_MEMBER_PATH_POINTS` points is `INVALID_ARGUMENT` (a two-point path is a straight
    /// member), more than `MAX_MEMBER_PATH_POINTS` points or a coordinate above the coordinate
    /// cap is `LIMIT_EXCEEDED`.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        if !label_is_valid(&self.name) {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "name",
                "label must be 1 to 256 bytes without control characters",
            )
            .with_rule("DV-BASE-003"));
        }
        if self.path_points_mm.len() < MIN_MEMBER_PATH_POINTS {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "path_points_mm",
                "a polybeam path needs at least three points",
            ));
        }
        if self.path_points_mm.len() > MAX_MEMBER_PATH_POINTS {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "path_points_mm",
                "path must have 2 to 1024 points",
            )
            .with_rule("DV-LIM-001"));
        }
        if self.path_points_mm.iter().flatten().any(|v| !v.is_finite()) {
            return Err(CommandError::new(
                ErrorCode::NonFiniteNumber,
                "path_points_mm",
                "value is not finite",
            )
            .with_rule("DV-ATTR-003"));
        }
        if self
            .path_points_mm
            .iter()
            .flatten()
            .any(|v| v.abs() > COORDINATE_CAP_MM)
        {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "path_points_mm",
                "coordinate exceeds the coordinate cap",
            )
            .with_rule("DV-LIM-001"));
        }
        self.placement.validate()
    }

    /// SHA-256 of a versioned, length-prefixed binary encoding of the validated arguments, with
    /// signed zero normalized; like [`CreateStraightMember::content_fingerprint`] it is not the
    /// F04 canonical form.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        self.validate_arguments()?;
        let bad = || {
            CommandError::new(
                ErrorCode::NonFiniteNumber,
                "path_points_mm",
                "value cannot be encoded",
            )
            .with_rule("DV-ATTR-003")
        };
        let mut bytes = Vec::with_capacity(256);
        bytes.extend_from_slice(b"BeamCraft/P12/CreatePolybeam/v1\0");
        bytes.extend_from_slice(self.new_id.as_bytes());
        bytes.push(match self.kind {
            MemberKind::Beam => 0,
            MemberKind::Column => 1,
        });
        let name = self.name.as_bytes();
        bytes.extend_from_slice(&(name.len() as u64).to_le_bytes());
        bytes.extend_from_slice(name);
        bytes.extend_from_slice(&self.class.to_le_bytes());
        bytes.extend_from_slice(self.profile_digest.as_bytes());
        match &self.material_digest {
            Some(digest) => {
                bytes.push(1);
                bytes.extend_from_slice(digest.as_bytes());
            }
            None => bytes.push(0),
        }
        bytes.extend_from_slice(&(self.path_points_mm.len() as u64).to_le_bytes());
        self.path_points_mm
            .iter()
            .flatten()
            .copied()
            .chain(self.placement.translation_mm)
            .chain(self.placement.orientation_xyzw)
            .try_for_each(|v| {
                bytes.extend_from_slice(&canonical_f64_bytes(v).map_err(|_| bad())?);
                Ok::<(), CommandError>(())
            })?;
        Sha256::digest(&bytes).map_err(|_| {
            CommandError::new(ErrorCode::LimitExceeded, "content", "content is too large")
                .with_rule("DV-LIM-001")
        })
    }
}

impl CreatePlate {
    /// Admits what does not depend on geometry: label, contour count, finiteness, coordinate and
    /// dimension caps, then the placement. The count is checked before any traversal.
    ///
    /// A count below 3 is `INVALID_ARGUMENT`, above 1024 `LIMIT_EXCEEDED`; a non-finite number is
    /// `NON_FINITE_NUMBER`; a coordinate above 1,000,000,000 mm or a thickness above
    /// 1,000,000 mm is `LIMIT_EXCEEDED`. Floor, winding, planarity and supported shape are the
    /// engine's.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        if !label_is_valid(&self.name) {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "name",
                "label must be 1 to 256 bytes without control characters",
            )
            .with_rule("DV-BASE-003"));
        }
        let count = self.contour_points_mm.len();
        if count > CONTOUR_POINT_CAP {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "contour_points_mm",
                "contour has more than 1024 points",
            )
            .with_rule("DV-LIM-001"));
        }
        if count < MIN_CONTOUR_POINTS {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "contour_points_mm",
                "contour needs at least 3 points",
            ));
        }
        if !self.thickness_mm.is_finite()
            || self
                .contour_points_mm
                .iter()
                .flatten()
                .any(|v| !v.is_finite())
        {
            return Err(CommandError::new(
                ErrorCode::NonFiniteNumber,
                "contour_points_mm",
                "value is not finite",
            )
            .with_rule("DV-ATTR-003"));
        }
        if self
            .contour_points_mm
            .iter()
            .flatten()
            .any(|v| v.abs() > COORDINATE_CAP_MM)
        {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "contour_points_mm",
                "coordinate exceeds the coordinate cap",
            )
            .with_rule("DV-LIM-001"));
        }
        if self.thickness_mm > DIMENSION_CAP_MM {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "thickness_mm",
                "thickness exceeds the dimension cap",
            )
            .with_rule("DV-LIM-001"));
        }
        self.placement.validate()?;
        self.validate_holes()?;
        self.validate_corners()?;
        Ok(())
    }

    /// Admits the corner treatments (A12.2): at most one per contour vertex, each naming a real
    /// vertex of this contour. The values are bounded by [`PlateCorner::validate`]; whether the
    /// treated corner really turns is the engine's decision.
    fn validate_corners(&self) -> Result<(), CommandError> {
        let count = self.contour_points_mm.len();
        if self.corners.len() > count {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "corner",
                "more corner treatments than contour vertices",
            )
            .with_rule("DV-LIM-001"));
        }
        let mut seen = Vec::new();
        for corner in &self.corners {
            if corner.vertex >= count {
                return Err(CommandError::new(
                    ErrorCode::InvalidArgument,
                    "corner",
                    "corner names no vertex of the contour",
                ));
            }
            if seen.contains(&corner.vertex) {
                return Err(CommandError::new(
                    ErrorCode::InvalidArgument,
                    "corner",
                    "a contour vertex carries more than one corner treatment",
                ));
            }
            seen.push(corner.vertex);
            corner.validate()?;
        }
        Ok(())
    }

    fn validate_holes(&self) -> Result<(), CommandError> {
        if self.holes_mm.len() > MAX_HOLES {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "holes_mm",
                "more than 64 holes",
            )
            .with_rule("DV-LIM-001"));
        }
        if self.holes_mm.iter().any(|h| h.len() > HOLE_POINT_CAP) {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "holes_mm",
                "a hole has more than 1024 points",
            )
            .with_rule("DV-LIM-001"));
        }
        if self.holes_mm.iter().any(|h| h.len() < MIN_CONTOUR_POINTS) {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "holes_mm",
                "a hole needs at least 3 points",
            ));
        }
        let coordinates = || self.holes_mm.iter().flatten().flatten();
        if coordinates().any(|v| !v.is_finite()) {
            return Err(CommandError::new(
                ErrorCode::NonFiniteNumber,
                "holes_mm",
                "value is not finite",
            )
            .with_rule("DV-ATTR-003"));
        }
        if coordinates().any(|v| v.abs() > COORDINATE_CAP_MM) {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "holes_mm",
                "coordinate exceeds the coordinate cap",
            )
            .with_rule("DV-LIM-001"));
        }
        Ok(())
    }

    /// SHA-256 of a versioned, length-prefixed binary encoding of the validated arguments, with
    /// signed zero normalized. Two commands with the same fingerprint have identical content;
    /// it is *not* the F04 canonical form and must not be sent over a wire.
    ///
    /// Fails (`LIMIT_EXCEEDED`, `NON_FINITE_NUMBER`) on arguments that do not validate, so a
    /// fingerprint exists only for admissible content.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        self.validate_arguments()?;
        let bad = || {
            CommandError::new(
                ErrorCode::NonFiniteNumber,
                "contour_points_mm",
                "value cannot be encoded",
            )
            .with_rule("DV-ATTR-003")
        };
        let mut bytes = Vec::with_capacity(64 + self.contour_points_mm.len() * 24);
        bytes.extend_from_slice(b"BeamCraft/P04/CreatePlate/v8\0");
        bytes.extend_from_slice(self.new_id.as_bytes());
        let name = self.name.as_bytes();
        bytes.extend_from_slice(&(name.len() as u64).to_le_bytes());
        bytes.extend_from_slice(name);
        bytes.extend_from_slice(&self.class.to_le_bytes());
        bytes.push(match self.thickness_side {
            None => 0,
            Some(PlateSide::Top) => 1,
            Some(PlateSide::Middle) => 2,
            Some(PlateSide::Bottom) => 3,
        });
        bytes.extend_from_slice(&(self.corners.len() as u64).to_le_bytes());
        for corner in &self.corners {
            bytes.extend_from_slice(&(corner.vertex as u64).to_le_bytes());
            bytes.push(match corner.treatment {
                CornerTreatment::Chamfer { .. } => 1,
                CornerTreatment::Round { .. } => 2,
            });
            let values = match corner.treatment {
                CornerTreatment::Chamfer { d1_mm, d2_mm } => [d1_mm, d2_mm],
                CornerTreatment::Round { radius_mm } => [radius_mm, radius_mm],
            };
            for v in values {
                bytes.extend_from_slice(&canonical_f64_bytes(v).map_err(|_| bad())?);
            }
        }
        bytes.extend_from_slice(&(self.contour_points_mm.len() as u64).to_le_bytes());
        let mut scalars = self
            .contour_points_mm
            .iter()
            .flatten()
            .copied()
            .chain([self.thickness_mm])
            .chain(self.placement.translation_mm)
            .chain(self.placement.orientation_xyzw);
        scalars.try_for_each(|v| {
            bytes.extend_from_slice(&canonical_f64_bytes(v).map_err(|_| bad())?);
            Ok::<(), CommandError>(())
        })?;
        match &self.material_digest {
            Some(digest) => {
                bytes.push(1);
                bytes.extend_from_slice(digest.as_bytes());
            }
            None => bytes.push(0),
        }
        bytes.extend_from_slice(&(self.holes_mm.len() as u64).to_le_bytes());
        for hole in &self.holes_mm {
            bytes.extend_from_slice(&(hole.len() as u64).to_le_bytes());
            for v in hole.iter().flatten() {
                bytes.extend_from_slice(&canonical_f64_bytes(*v).map_err(|_| bad())?);
            }
        }
        Sha256::digest(&bytes).map_err(|_| {
            CommandError::new(ErrorCode::LimitExceeded, "content", "content is too large")
                .with_rule("DV-LIM-001")
        })
    }
}

/// Most through holes of one plate.
pub const MAX_HOLES: usize = 64;
/// Most points of one hole polygon.
pub const HOLE_POINT_CAP: usize = 1024;

/// The hole of F04 section 7 `AddHole`: a clockwise polygon as submitted, or a generator whose
/// parameters name the polygon the command authors. The stored polygon is always authoritative
/// (F04 section 4); a generator is resolved at the input edge by [`HoleSpec::polygon`].
#[derive(Debug, Clone, PartialEq)]
pub enum HoleSpec {
    /// The clockwise polygon itself, 3 to 1024 points in plate-local XY, closure implicit.
    /// Winding, self-intersection, placement inside the outline and clearance from the other
    /// holes are the kernel's decision, with exact predicates.
    Polygon(Vec<[f64; 2]>),
    /// A round hole: the regular inscribed `N`-gon of F06 section 2 (A-2), with `N` decided
    /// from the radius against the fixed threshold table of [`HoleSpec::polygon`].
    Round {
        /// Centre in plate-local XY, each coordinate within the coordinate cap.
        centre_mm: [f64; 2],
        /// Diameter in millimetres, finite and greater than zero.
        diameter_mm: f64,
    },
    /// An axis-aligned rectangular hole from one corner, four clockwise points.
    Rectangle {
        /// The first corner in plate-local XY, each coordinate within the coordinate cap.
        corner_mm: [f64; 2],
        /// Extent along +X, finite and greater than zero.
        width_mm: f64,
        /// Extent along +Y, finite and greater than zero.
        height_mm: f64,
    },
}

/// The largest radius each permitted segment count serves (F06 section 2, A-2): the row at
/// index `k` is the largest radius whose chord deviation `r * (1 - cos(pi / N))` stays within
/// 0.05 mm for `N = 16 + 4k`, so the decision compares the radius with these fixed numbers and
/// no platform `cos` result decides it. The values are `0.05 / (1 - cos(pi / N))`.
const ROUND_HOLE_RADIUS_LIMITS: [f64; 61] = [
    2.602_171_722_995_434,     // N=16
    4.061_190_969_939_727_5,   // N=20
    5.844_440_655_460_139,     // N=24
    7.951_919_378_839_592,     // N=28
    1.038_362_655_579_763_1e1, // N=32
    1.313_956_190_743_764_2e1, // N=36
    1.621_972_528_694_607e1,   // N=40
    1.962_411_661_105_958e1,   // N=44
    2.335_273_582_969_88e1,    // N=48
    2.740_558_291_127_616_6e1, // N=52
    3.178_265_783_506_647_3e1, // N=56
    3.648_396_058_701_346e1,   // N=60
    4.150_949_115_731_939_5e1, // N=64
    4.685_924_953_897_042e1,   // N=68
    5.253_323_572_685_768e1,   // N=72
    5.853_144_971_716_463_4e1, // N=76
    6.485_389_150_701_836e1,   // N=80
    7.150_056_109_420_748e1,   // N=84
    7.847_145_847_701_5e1,     // N=88
    8.576_658_365_409_166e1,   // N=92
    9.338_593_662_435_471e1,   // N=96
    1.013_295_173_869_295_7e2, // N=100
    1.095_973_259_411_158_3e2, // N=104
    1.181_893_622_863_406_5e2, // N=108
    1.271_056_264_221_183_9e2, // N=112
    1.363_461_183_480_881e2,   // N=116
    1.459_108_380_638_777_5e2, // N=120
    1.557_997_855_692_493_7e2, // N=124
    1.660_129_608_639_590_2e2, // N=128
    1.765_503_639_477_979e2,   // N=132
    1.874_119_948_205_258e2,   // N=136
    1.985_978_534_820_98e2,    // N=140
    2.101_079_399_324_427_5e2, // N=144
    2.219_422_541_712_727_7e2, // N=148
    2.341_007_961_985_759_2e2, // N=152
    2.465_835_660_143_244_5e2, // N=156
    2.593_905_636_183_075_7e2, // N=160
    2.725_217_890_107_020_4e2, // N=164
    2.859_772_421_911_766_3e2, // N=168
    2.997_569_231_598_656e2,   // N=172
    3.138_608_319_165_718e2,   // N=176
    3.282_889_684_614_987_7e2, // N=180
    3.430_413_327_942_437e2,   // N=184
    3.581_179_249_152_984e2,   // N=188
    3.735_187_448_240_587e2,   // N=192
    3.892_437_925_207_74e2,    // N=196
    4.052_930_680_054_579_4e2, // N=200
    4.216_665_712_780_745e2,   // N=204
    4.383_643_023_387_047e2,   // N=208
    4.553_862_611_870_239e2,   // N=212
    4.727_324_478_229_255_6e2, // N=216
    4.904_028_622_470_072_5e2, // N=220
    5.083_975_044_590_468_4e2, // N=224
    5.267_163_744_590_734e2,   // N=228
    5.453_594_722_465_092e2,   // N=232
    5.643_267_978_213_062e2,   // N=236
    5.836_183_511_843_763e2,   // N=240
    6.032_341_323_355_112e2,   // N=244
    6.231_741_412_739_907e2,   // N=248
    6.434_383_780_007_807e2,   // N=252
    6.640_268_425_141_733e2,   // N=256
];

/// The first non-finite hole coordinate refuses the command (`NON_FINITE_NUMBER`).
fn hole_values_finite(mut values: impl Iterator<Item = f64>) -> Result<(), CommandError> {
    values.all(|v| v.is_finite()).then_some(()).ok_or_else(|| {
        CommandError::new(ErrorCode::NonFiniteNumber, "hole", "value is not finite")
            .with_rule("DV-ATTR-003")
    })
}

/// The first hole coordinate above the coordinate cap refuses the command (`LIMIT_EXCEEDED`).
fn hole_values_in_cap(mut values: impl Iterator<Item = f64>) -> Result<(), CommandError> {
    values
        .all(|v| v.abs() <= COORDINATE_CAP_MM)
        .then_some(())
        .ok_or_else(|| {
            CommandError::new(
                ErrorCode::LimitExceeded,
                "hole",
                "coordinate exceeds the coordinate cap",
            )
            .with_rule("DV-LIM-001")
        })
}

impl HoleSpec {
    /// Admits what does not depend on geometry: finiteness, positivity of generator sizes and
    /// the polygon caps. A non-finite number is `NON_FINITE_NUMBER`; a cap or a non-positive
    /// size is `LIMIT_EXCEEDED` or `INVALID_ARGUMENT` as for `CreatePlate.holes_mm`.
    pub fn validate(&self) -> Result<(), CommandError> {
        match self {
            Self::Polygon(points) => {
                if points.len() > HOLE_POINT_CAP {
                    return Err(CommandError::new(
                        ErrorCode::LimitExceeded,
                        "hole",
                        "a hole has more than 1024 points",
                    )
                    .with_rule("DV-LIM-001"));
                }
                if points.len() < MIN_CONTOUR_POINTS {
                    return Err(CommandError::new(
                        ErrorCode::InvalidArgument,
                        "hole",
                        "a hole needs at least 3 points",
                    ));
                }
                hole_values_finite(points.iter().flatten().copied())?;
                hole_values_in_cap(points.iter().flatten().copied())
            }
            Self::Round {
                centre_mm,
                diameter_mm,
            } => {
                hole_values_finite(centre_mm.iter().copied().chain([*diameter_mm]))?;
                hole_values_in_cap(centre_mm.iter().copied())?;
                if *diameter_mm <= 0.0 {
                    return Err(CommandError::new(
                        ErrorCode::InvalidArgument,
                        "hole",
                        "a hole diameter must be positive",
                    ));
                }
                Ok(())
            }
            Self::Rectangle {
                corner_mm,
                width_mm,
                height_mm,
            } => {
                hole_values_finite(corner_mm.iter().copied().chain([*width_mm, *height_mm]))?;
                hole_values_in_cap(corner_mm.iter().copied())?;
                if *width_mm <= 0.0 || *height_mm <= 0.0 {
                    return Err(CommandError::new(
                        ErrorCode::InvalidArgument,
                        "hole",
                        "a hole width and height must be positive",
                    ));
                }
                Ok(())
            }
        }
    }

    /// The clockwise polygon this spec authors, at the input edge (the vertices use the
    /// platform `sin` and `cos` and are the doubles they round to). A round hole of radius
    /// `r` is the regular inscribed `N`-gon of F06 section 2: the smallest multiple of four,
    /// at least 16 and at most 256, whose chord deviation stays within 0.05 mm. A radius
    /// above the last threshold needs `N > 256` and is `UNSUPPORTED_GEOMETRY` (field
    /// `hole.radius`), because no `Approximated` status exists in 0.1.x.
    pub fn polygon(&self) -> Result<Vec<[f64; 2]>, CommandError> {
        self.validate()?;
        match self {
            Self::Polygon(points) => Ok(points.clone()),
            Self::Round {
                centre_mm,
                diameter_mm,
            } => {
                let radius = diameter_mm / 2.0;
                let index = ROUND_HOLE_RADIUS_LIMITS
                    .iter()
                    .position(|limit| radius <= *limit)
                    .ok_or_else(|| {
                        CommandError::new(
                            ErrorCode::UnsupportedGeometry,
                            "hole.radius",
                            "the radius needs more than 256 segments",
                        )
                    })?;
                let segments = 16 + 4 * index;
                Ok((0..segments)
                    .map(|k| {
                        let theta = -core::f64::consts::TAU * (k as f64) / (segments as f64);
                        [
                            centre_mm[0] + radius * theta.cos(),
                            centre_mm[1] + radius * theta.sin(),
                        ]
                    })
                    .collect())
            }
            Self::Rectangle {
                corner_mm: [x, y],
                width_mm,
                height_mm,
            } => Ok(vec![
                [*x, *y],
                [*x, y + height_mm],
                [x + width_mm, y + height_mm],
                [x + width_mm, *y],
            ]),
        }
    }
}

/// What one through-cut record of a plate authors (A13): a fastener hole (A04) or a polygon
/// cut. Both replay through the same certified kernel subtraction and share the per-element
/// ordinal counter and the `cut:f.h<ordinal>` wall ids; the kind keeps the operations distinct
/// for numbering and reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoleKind {
    /// A through hole authored by `AddHole` (A04).
    Hole,
    /// A polygon cut authored by `CutByPolygon` or derived from `CutByPart` (A13).
    PolygonCut,
}

impl HoleKind {
    /// The wire token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hole => "hole",
            Self::PolygonCut => "polygon_cut",
        }
    }

    /// Parses the wire token; anything else is `None`.
    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "hole" => Some(Self::Hole),
            "polygon_cut" => Some(Self::PolygonCut),
            _ => None,
        }
    }
}

/// One through-hole operation record of a plate (F04 section 4, A04): an owned record with an
/// engine-assigned ordinal and the authoritative clockwise polygon. The records of one element
/// are always replayed from the base shape in ordinal order, so no stored intermediate mesh can
/// go stale. Ordinals are never reused or renumbered; `UpdateHole` and `RemoveHole`, later
/// slices of A04, address a record by its `operation_id`. Polygon cuts (A13) share the record
/// form and the ordinal counter, told apart by `kind`.
#[derive(Debug, Clone, PartialEq)]
pub struct HoleRecord {
    /// Full UUIDv7 of the record, unused in the project (`DUPLICATE_ID`).
    pub operation_id: OpId,
    /// The per-element ordinal in creation order, starting at zero. It names the record's
    /// FeatureIds (`cut:f.h<ordinal>...`).
    pub ordinal: usize,
    /// The authoritative clockwise polygon in plate-local XY, as generated at the input edge.
    pub polygon_mm: Vec<[f64; 2]>,
    /// Whether the record is a hole (A04) or a polygon cut (A13).
    pub kind: HoleKind,
}

/// Replace one existing hole's authoritative polygon, preserving its identity and ordinal.
#[derive(Debug, Clone, PartialEq)]
pub struct UpdateHole {
    /// Target, existing record UUID and replacement hole specification.
    pub hole: AddHole,
}
impl UpdateHole {
    /// Admit the shape arguments independently of model state.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        self.hole.validate_arguments()
    }
    /// Stable typed request identity, separate from creation.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        let mut b = b"BeamCraft/A04/UpdateHole/v1\0".to_vec();
        b.extend_from_slice(self.hole.content_fingerprint()?.as_bytes());
        Sha256::digest(&b).map_err(|_| {
            CommandError::new(
                ErrorCode::LimitExceeded,
                "content",
                "hole request exceeds its bound",
            )
        })
    }
}

/// Remove an existing hole by record identity, never by an unstable polygon index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoveHole {
    /// Target owned plate.
    pub target_id: ElementId,
    /// Existing hole record.
    pub operation_id: OpId,
}
impl RemoveHole {
    /// Canonical typed removal digest.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        let mut b = b"BeamCraft/A04/RemoveHole/v1\0".to_vec();
        b.extend_from_slice(self.target_id.as_bytes());
        b.extend_from_slice(self.operation_id.as_bytes());
        Sha256::digest(&b).map_err(|_| {
            CommandError::new(
                ErrorCode::LimitExceeded,
                "content",
                "hole request exceeds its bound",
            )
        })
    }
}

/// F04 section 7 `AddHole`: one through-hole operation record on a valid plate (A04). The
/// record's `operation_id` is also the logical command id: a retry with identical content
/// replays the recorded result, different content under an accepted id is `OP_ID_REUSE`, and
/// an id that already names a record of the project is `DUPLICATE_ID`. The engine assigns the
/// ordinal from the plate's per-element counter; other hole types (member holes, slots, blind
/// holes) are explicitly unsupported and refused as F06 section 2 failures.
#[derive(Debug, Clone, PartialEq)]
pub struct AddHole {
    /// The plate that will own the record; a member target is `UNSUPPORTED_GEOMETRY`.
    pub target_id: ElementId,
    /// Full UUIDv7 of the record, unused in the project.
    pub operation_id: OpId,
    /// The hole to author.
    pub hole: HoleSpec,
}

impl AddHole {
    /// Admits the arguments that do not depend on geometry or state.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        self.hole.validate()
    }

    /// The clockwise polygon this command authors; the stored polygon is authoritative.
    pub fn polygon(&self) -> Result<Vec<[f64; 2]>, CommandError> {
        self.hole.polygon()
    }

    /// SHA-256 of a versioned, length-prefixed binary encoding of the validated arguments, with
    /// signed zero normalized, as for [`CreatePlate::content_fingerprint`]. It is *not* the F04
    /// canonical form and must not be sent over a wire. Fails on arguments that do not
    /// validate, so a fingerprint exists only for admissible content.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        self.validate_arguments()?;
        let bad = || {
            CommandError::new(
                ErrorCode::NonFiniteNumber,
                "hole",
                "value cannot be encoded",
            )
            .with_rule("DV-ATTR-003")
        };
        let polygon = self.polygon()?;
        let mut bytes = Vec::with_capacity(64 + polygon.len() * 16);
        bytes.extend_from_slice(b"BeamCraft/A04/AddHole/v1\0");
        bytes.extend_from_slice(self.target_id.as_bytes());
        bytes.extend_from_slice(self.operation_id.as_bytes());
        match &self.hole {
            HoleSpec::Polygon(_) => bytes.push(0),
            HoleSpec::Round {
                centre_mm,
                diameter_mm,
            } => {
                bytes.push(1);
                for v in centre_mm.iter().copied().chain([*diameter_mm]) {
                    bytes.extend_from_slice(&canonical_f64_bytes(v).map_err(|_| bad())?);
                }
            }
            HoleSpec::Rectangle {
                corner_mm,
                width_mm,
                height_mm,
            } => {
                bytes.push(2);
                for v in corner_mm.iter().copied().chain([*width_mm, *height_mm]) {
                    bytes.extend_from_slice(&canonical_f64_bytes(v).map_err(|_| bad())?);
                }
            }
        }
        bytes.extend_from_slice(&(polygon.len() as u64).to_le_bytes());
        for v in polygon.iter().flatten() {
            bytes.extend_from_slice(&canonical_f64_bytes(*v).map_err(|_| bad())?);
        }
        Sha256::digest(&bytes).map_err(|_| {
            CommandError::new(ErrorCode::LimitExceeded, "content", "content is too large")
                .with_rule("DV-LIM-001")
        })
    }
}

/// F04 section 7 `CutByLine` (A13): cuts a valid plate, beam or column with the plane through
/// the line `point_a_mm`–`point_b_mm` and the given axis direction, keeping one side. The plane
/// is derived once at admission and stored as an ordinary `plane_cut` record; the line itself is
/// not kept in the record.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CutByLine {
    /// The element to cut.
    pub target_id: ElementId,
    /// Full UUIDv7 of the new record; unused in the project.
    pub operation_id: OpId,
    /// First point of the cutting line, element-local millimetres.
    pub point_a_mm: [f64; 3],
    /// Second point of the cutting line, element-local millimetres; equal to `point_a_mm` is
    /// `INVALID_ARGUMENT`.
    pub point_b_mm: [f64; 3],
    /// A non-zero direction the cutting plane contains besides the line; parallel to the line
    /// is `INVALID_ARGUMENT`.
    pub axis: [f64; 3],
    /// The kept half-space of the derived plane.
    pub keep: KeepSide,
}

impl CutByLine {
    /// Admits what does not depend on geometry or model state: finiteness, the coordinate cap,
    /// distinct line points and a non-zero axis. Whether the derived plane actually crosses the
    /// element is the kernel's decision at replay.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        let values = [self.point_a_mm, self.point_b_mm, self.axis];
        if values.iter().flatten().any(|v| !v.is_finite()) {
            return Err(CommandError::new(
                ErrorCode::NonFiniteNumber,
                "cut_by_line",
                "value is not finite",
            )
            .with_rule("DV-ATTR-003"));
        }
        if values.iter().flatten().any(|v| v.abs() > COORDINATE_CAP_MM) {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "cut_by_line",
                "coordinate exceeds the coordinate cap",
            )
            .with_rule("DV-LIM-001"));
        }
        if self.point_a_mm == self.point_b_mm {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "cut_by_line",
                "the two line points must differ",
            ));
        }
        if self.axis == [0.0; 3] {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "cut_by_line",
                "the cutting plane needs a non-zero axis",
            ));
        }
        Ok(())
    }

    /// The derived plane as a `plane_cut` record geometry: the plane contains the line and the
    /// axis, so its normal is `normalize(cross(b - a, axis))`. An axis parallel to the line is
    /// `INVALID_ARGUMENT`.
    pub fn plane(&self) -> Result<([f64; 3], [f64; 3]), CommandError> {
        let dir = [
            self.point_b_mm[0] - self.point_a_mm[0],
            self.point_b_mm[1] - self.point_a_mm[1],
            self.point_b_mm[2] - self.point_a_mm[2],
        ];
        let n = [
            dir[1] * self.axis[2] - dir[2] * self.axis[1],
            dir[2] * self.axis[0] - dir[0] * self.axis[2],
            dir[0] * self.axis[1] - dir[1] * self.axis[0],
        ];
        let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if !length.is_finite() || length == 0.0 {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "cut_by_line",
                "the axis must not be parallel to the line",
            ));
        }
        Ok((
            self.point_a_mm,
            [n[0] / length, n[1] / length, n[2] / length],
        ))
    }

    /// SHA-256 of a versioned, length-prefixed binary encoding of the validated arguments, as
    /// for [`CutByPlane::content_fingerprint`].
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        fingerprint_cut(
            b"BeamCraft/A13/CutByLine/v1",
            self.target_id,
            self.operation_id,
            self.validate_arguments(),
            |bytes| {
                for value in self
                    .point_a_mm
                    .into_iter()
                    .chain(self.point_b_mm)
                    .chain(self.axis)
                {
                    push_f64(bytes, value)?;
                }
                Ok(())
            },
        )
    }
}

/// F04 section 7 `CutByPolygon` (A13): cuts a valid plate with the prism of a closed polygon in
/// plate-local XY swept through the whole thickness, removing the material inside it. The
/// polygon admission is the kernel's (clockwise, simple, strictly inside the outline, clear of
/// every other record), exactly as for a hole; the record is a cut, not a hole.
#[derive(Debug, Clone, PartialEq)]
pub struct CutByPolygon {
    /// The plate to cut; a member target is `UNSUPPORTED_GEOMETRY` (the kernel subtracts
    /// Z-prisms only).
    pub target_id: ElementId,
    /// Full UUIDv7 of the new record; unused in the project.
    pub operation_id: OpId,
    /// The clockwise polygon in plate-local XY, 3 to 1024 points, closure implicit.
    pub polygon_mm: Vec<[f64; 2]>,
}

impl CutByPolygon {
    /// Admits the polygon shape (count, finiteness, caps) as [`HoleSpec::validate`] does.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        HoleSpec::Polygon(self.polygon_mm.clone()).validate()
    }

    /// The clockwise polygon this command authors; the stored polygon is authoritative.
    pub fn polygon(&self) -> Result<Vec<[f64; 2]>, CommandError> {
        HoleSpec::Polygon(self.polygon_mm.clone()).polygon()
    }

    /// SHA-256 of a versioned, length-prefixed binary encoding of the validated arguments, as
    /// for [`CutByPlane::content_fingerprint`].
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        let polygon = self.polygon()?;
        fingerprint_cut(
            b"BeamCraft/A13/CutByPolygon/v1",
            self.target_id,
            self.operation_id,
            self.validate_arguments(),
            |bytes| {
                for value in polygon.iter().flatten().copied() {
                    push_f64(bytes, value)?;
                }
                Ok(())
            },
        )
    }
}

/// F04 section 7 `CutByPart` (A13): cuts a plate with another plate, removing the material
/// inside the tool's prism. The tool's contour and reach are derived once at admission in the
/// target's local frame and stored as a polygon-cut record; moving the tool afterwards never
/// updates the cut. A tool whose relative rotation is not about the target normal, or whose
/// prism does not cross the target, is refused.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CutByPart {
    /// The plate to cut.
    pub target_id: ElementId,
    /// Full UUIDv7 of the new record; unused in the project.
    pub operation_id: OpId,
    /// The plate used as the cutter.
    pub tool_id: ElementId,
}

impl CutByPart {
    /// Nothing to admit beyond identity; everything else is the engine's decision.
    pub const fn validate_arguments(&self) -> Result<(), CommandError> {
        Ok(())
    }

    /// SHA-256 of a versioned, length-prefixed binary encoding of the validated arguments, as
    /// for [`CutByPlane::content_fingerprint`].
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        fingerprint_cut(
            b"BeamCraft/A13/CutByPart/v1",
            self.target_id,
            self.operation_id,
            self.validate_arguments(),
            |bytes| {
                bytes.extend_from_slice(self.tool_id.as_bytes());
                Ok(())
            },
        )
    }
}

/// F04 section 7 `FitEndToPart` (A13): trims a plate, beam or column flush with the face of
/// another plate. The engine fits to the tool face nearest the target's extent midpoint and
/// keeps the side the target body lies on; the derived plane is stored as an ordinary
/// `plane_cut` record. Moving the tool afterwards never updates the fit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FitEndToPart {
    /// The element to fit.
    pub target_id: ElementId,
    /// Full UUIDv7 of the new record; unused in the project.
    pub operation_id: OpId,
    /// The plate whose face the end is fitted to.
    pub tool_id: ElementId,
}

impl FitEndToPart {
    /// Nothing to admit beyond identity; everything else is the engine's decision.
    pub const fn validate_arguments(&self) -> Result<(), CommandError> {
        Ok(())
    }

    /// SHA-256 of a versioned, length-prefixed binary encoding of the validated arguments, as
    /// for [`CutByPlane::content_fingerprint`].
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        fingerprint_cut(
            b"BeamCraft/A13/FitEndToPart/v1",
            self.target_id,
            self.operation_id,
            self.validate_arguments(),
            |bytes| {
                bytes.extend_from_slice(self.tool_id.as_bytes());
                Ok(())
            },
        )
    }
}

/// F04 section 7 `ChamferPart` (A13): chamfers one corner of a valid plate by cutting it with
/// the plane through the two points `first_distance_mm` and `second_distance_mm` back along the
/// two contour edges at the vertex, keeping the far side. The derived plane is stored as an
/// ordinary `plane_cut` record; the vertex distances are not kept in the record.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChamferPart {
    /// The plate to chamfer.
    pub target_id: ElementId,
    /// Full UUIDv7 of the new record; unused in the project.
    pub operation_id: OpId,
    /// Index of the contour vertex to chamfer, into the stored contour order; out of range is
    /// `NOT_FOUND` at the engine.
    pub vertex_index: usize,
    /// Distance back along the edge from the previous vertex, in millimetres.
    pub first_distance_mm: f64,
    /// Distance back along the edge to the next vertex, in millimetres.
    pub second_distance_mm: f64,
}

impl ChamferPart {
    /// Admits the distances: finite, positive, within the coordinate cap. Whether they fit the
    /// edges and whether the derived plane crosses the element are the engine's and the
    /// kernel's decisions.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        for (name, value) in [
            ("first_distance_mm", self.first_distance_mm),
            ("second_distance_mm", self.second_distance_mm),
        ] {
            if !value.is_finite() {
                return Err(CommandError::new(
                    ErrorCode::NonFiniteNumber,
                    name,
                    "value is not finite",
                )
                .with_rule("DV-ATTR-003"));
            }
            if value <= 0.0 {
                return Err(CommandError::new(
                    ErrorCode::InvalidArgument,
                    name,
                    "a chamfer distance must be positive",
                ));
            }
            if value > COORDINATE_CAP_MM {
                return Err(CommandError::new(
                    ErrorCode::LimitExceeded,
                    name,
                    "distance exceeds the coordinate cap",
                )
                .with_rule("DV-LIM-001"));
            }
        }
        Ok(())
    }

    /// SHA-256 of a versioned, length-prefixed binary encoding of the validated arguments, as
    /// for [`CutByPlane::content_fingerprint`].
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        fingerprint_cut(
            b"BeamCraft/A13/ChamferPart/v1",
            self.target_id,
            self.operation_id,
            self.validate_arguments(),
            |bytes| {
                push_f64(bytes, self.first_distance_mm)?;
                push_f64(bytes, self.second_distance_mm)?;
                push_f64(bytes, self.vertex_index as f64)
            },
        )
    }
}

/// F04 section 7 `SplitPart` (A13): splits a valid plate, beam or column in two with a plane.
/// The target keeps the `keep` half-space and gains the matching `plane_cut` record; a new
/// element with the given identity receives a copy of the target's authored state and the
/// complementary cut. One command, one revision, one undo unit.
#[derive(Debug, Clone, PartialEq)]
pub struct SplitPart {
    /// The element to split.
    pub target_id: ElementId,
    /// Full identity of the new element holding the other half; unused in the project.
    pub new_id: ElementId,
    /// Full UUIDv7 of the new record; unused in the project.
    pub operation_id: OpId,
    /// Label of the new element.
    pub new_name: String,
    /// A point of the splitting plane, element-local millimetres.
    pub point_mm: [f64; 3],
    /// A non-zero normal of the splitting plane.
    pub normal: [f64; 3],
    /// The half-space the target keeps; the new element keeps the other.
    pub keep: KeepSide,
}

impl SplitPart {
    /// Admits the label and the plane arguments as `CutByPlane` does.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        if !label_is_valid(&self.new_name) {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "new_name",
                "label must be 1 to 256 bytes without control characters",
            )
            .with_rule("DV-BASE-003"));
        }
        CutRecord {
            operation_id: self.operation_id,
            ordinal: 0,
            point_mm: self.point_mm,
            normal: self.normal,
            keep: self.keep,
        }
        .validate()
    }

    /// SHA-256 of a versioned, length-prefixed binary encoding of the validated arguments, as
    /// for [`CutByPlane::content_fingerprint`].
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        fingerprint_cut(
            b"BeamCraft/A13/SplitPart/v1",
            self.target_id,
            self.operation_id,
            self.validate_arguments(),
            |bytes| {
                bytes.extend_from_slice(self.new_id.as_bytes());
                for value in self.point_mm.into_iter().chain(self.normal) {
                    push_f64(bytes, value)?;
                }
                // The label bytes, length-prefixed as `CreatePlate` does, so same-length
                // renames are never one recorded content.
                let name = self.new_name.as_bytes();
                bytes.extend_from_slice(&(name.len() as u64).to_le_bytes());
                bytes.extend_from_slice(name);
                push_f64(
                    bytes,
                    match self.keep {
                        KeepSide::Positive => 0.0,
                        KeepSide::Negative => 1.0,
                    },
                )
            },
        )
    }
}

/// F04 section 7 `CombineParts` (A13): combines two straight members with the same pinned
/// profile, orientation and material into one longer member along the same axis. The survivor
/// keeps its identity and extends over the union of the two axis ranges, which must overlap or
/// touch; the consumed element is deleted. The effect is the resulting authored state and the
/// revision ledger entry; the journal, not a per-element record, carries the operation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CombineParts {
    /// The member that survives and extends.
    pub survivor_id: ElementId,
    /// The member consumed by the combination; it is deleted.
    pub consumed_id: ElementId,
    /// Full UUIDv7 of the operation; unused in the project.
    pub operation_id: OpId,
}

impl CombineParts {
    /// The two members must differ; everything else is the engine's decision.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        if self.survivor_id == self.consumed_id {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "consumed_id",
                "a member cannot be combined with itself",
            ));
        }
        Ok(())
    }

    /// SHA-256 of a versioned, length-prefixed binary encoding of the validated arguments, as
    /// for [`CutByPlane::content_fingerprint`].
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        fingerprint_cut(
            b"BeamCraft/A13/CombineParts/v1",
            self.survivor_id,
            self.operation_id,
            self.validate_arguments(),
            |bytes| {
                bytes.extend_from_slice(self.consumed_id.as_bytes());
                Ok(())
            },
        )
    }
}

/// The shared fingerprint form of the A13 commands: a version tag, the primary element, the
/// record identity and a caller-provided tail, digested as [`CutByPlane`] does. A failed
/// admission or an inexpressible value has no fingerprint.
fn fingerprint_cut(
    tag: &[u8],
    element: ElementId,
    operation: OpId,
    admitted: Result<(), CommandError>,
    tail: impl FnOnce(&mut Vec<u8>) -> Result<(), CommandError>,
) -> Result<ContentDigest, CommandError> {
    admitted?;
    let bad = || {
        CommandError::new(ErrorCode::NonFiniteNumber, "cut", "value cannot be encoded")
            .with_rule("DV-ATTR-003")
    };
    let mut bytes = Vec::with_capacity(96);
    bytes.extend_from_slice(tag);
    bytes.push(0);
    bytes.extend_from_slice(element.as_bytes());
    bytes.extend_from_slice(operation.as_bytes());
    tail(&mut bytes).map_err(|_| bad())?;
    Sha256::digest(&bytes).map_err(|_| {
        CommandError::new(ErrorCode::LimitExceeded, "content", "content is too large")
            .with_rule("DV-LIM-001")
    })
}

/// Appends the canonical bytes of one f64 to a fingerprint buffer.
fn push_f64(bytes: &mut Vec<u8>, value: f64) -> Result<(), CommandError> {
    let bad = CommandError::new(ErrorCode::NonFiniteNumber, "cut", "value cannot be encoded")
        .with_rule("DV-ATTR-003");
    bytes.extend_from_slice(&canonical_f64_bytes(value).map_err(|_| bad)?);
    Ok(())
}

/// A typed patch of supported authored fields (F04 section 7 `UpdateElement`). A field that is
/// `None` is left unchanged. `material` distinguishes no change (`None`), clearing the reference
/// (`Some(None)`) and pinning a definition (`Some(Some(digest))`). There is no raw USD setter.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ElementPatch {
    /// Complete replacement of a plate's sparse corner treatment list; empty clears it.
    pub corners: Option<Vec<PlateCorner>>,
    /// Complete replacement of a plate's authored contour (A12.3): every point, in order. A
    /// member has no contour. Whether the new contour is a valid plate shape is the engine's
    /// decision; the patch carries it whole, like a creation.
    pub contour_points_mm: Option<Vec<[f64; 3]>>,
    /// New label.
    pub name: Option<String>,
    /// New authored integer part class (P15).
    pub class: Option<i32>,
    /// New material reference, or its explicit removal.
    pub material: Option<Option<ContentDigest>>,
    /// New plate thickness in millimetres (a plate only).
    pub thickness_mm: Option<f64>,
    /// New thickness side of a plate (A12.1); `Some` names the side to lay the thickness off
    /// from the contour plane. A plate always has a side (absent means `top`), so there is no
    /// clearing.
    pub thickness_side: Option<PlateSide>,
    /// New placement translation in millimetres, absolute; the rotation is kept (a move).
    pub translation_mm: Option<[f64; 3]>,
    /// New placement rotation, absolute. Replaces the stored quaternion (the identity resets a
    /// part to the unrotated pose); the translation is kept. The quaternion follows the `Placement` rules and is validated.
    pub orientation_xyzw: Option<[f64; 4]>,
    /// D2: new level assignment of a plate, beam or column, or its explicit removal
    /// (`Some(None)`). Whether the level exists is the engine's decision.
    pub level: Option<Option<ElementId>>,
    /// P12: the complete replacement of a member's path points, two for a straight member and
    /// at least three for a polybeam. The whole path is replaced; there is no per-point setter.
    pub axis_points_mm: Option<Vec<[f64; 3]>>,
}

/// F04 section 7 `UpdateElement`: the target element and a typed patch.
#[derive(Debug, Clone, PartialEq)]
pub struct UpdateElement {
    /// The element to change.
    pub target_id: ElementId,
    /// The patch.
    pub patch: ElementPatch,
}

impl UpdateElement {
    /// Admits the patch: it must change something, the label follows the F04 rule, the
    /// thickness is finite (`NON_FINITE_NUMBER`) and at most 1,000,000 mm (`LIMIT_EXCEEDED`), and
    /// the translation is finite and within the coordinate cap. That a field applies to the
    /// target's kind and the geometry are the engine's decisions.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        let patch = &self.patch;
        if let Some(corners) = &patch.corners {
            if corners.len() > CONTOUR_POINT_CAP {
                return Err(CommandError::new(
                    ErrorCode::LimitExceeded,
                    "corner",
                    "too many corner treatments",
                ));
            }
            let mut seen = Vec::new();
            for corner in corners {
                corner.validate()?;
                if corner.vertex >= CONTOUR_POINT_CAP || seen.contains(&corner.vertex) {
                    return Err(CommandError::new(
                        ErrorCode::InvalidArgument,
                        "corner",
                        "duplicate or out-of-range corner vertex",
                    ));
                }
                seen.push(corner.vertex);
            }
        }
        if let Some(points) = &patch.contour_points_mm {
            let count = points.len();
            if count > CONTOUR_POINT_CAP {
                return Err(CommandError::new(
                    ErrorCode::LimitExceeded,
                    "contour_points_mm",
                    "contour has more than 1024 points",
                )
                .with_rule("DV-LIM-001"));
            }
            if count < MIN_CONTOUR_POINTS {
                return Err(CommandError::new(
                    ErrorCode::InvalidArgument,
                    "contour_points_mm",
                    "contour needs at least 3 points",
                ));
            }
            if points.iter().flatten().any(|v| !v.is_finite()) {
                return Err(CommandError::new(
                    ErrorCode::NonFiniteNumber,
                    "contour_points_mm",
                    "value is not finite",
                )
                .with_rule("DV-ATTR-003"));
            }
            if points.iter().flatten().any(|v| v.abs() > COORDINATE_CAP_MM) {
                return Err(CommandError::new(
                    ErrorCode::LimitExceeded,
                    "contour_points_mm",
                    "coordinate exceeds the coordinate cap",
                )
                .with_rule("DV-LIM-001"));
            }
        }
        if patch.corners.is_none()
            && patch.contour_points_mm.is_none()
            && patch.class.is_none()
            && patch.name.is_none()
            && patch.material.is_none()
            && patch.thickness_mm.is_none()
            && patch.thickness_side.is_none()
            && patch.translation_mm.is_none()
            && patch.orientation_xyzw.is_none()
            && patch.level.is_none()
            && patch.axis_points_mm.is_none()
        {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "changes",
                "the patch changes nothing",
            ));
        }
        if let Some(translation) = patch.translation_mm {
            if translation.iter().any(|v| !v.is_finite()) {
                return Err(CommandError::new(
                    ErrorCode::NonFiniteNumber,
                    "translation_mm",
                    "value is not finite",
                )
                .with_rule("DV-ATTR-003"));
            }
            if translation.iter().any(|v| v.abs() > COORDINATE_CAP_MM) {
                return Err(CommandError::new(
                    ErrorCode::LimitExceeded,
                    "translation_mm",
                    "translation component exceeds the coordinate cap",
                )
                .with_rule("DV-LIM-001"));
            }
        }
        if let Some(xyzw) = patch.orientation_xyzw {
            Placement::validate_orientation(xyzw)?;
        }
        if patch.name.as_deref().is_some_and(|n| !label_is_valid(n)) {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "name",
                "label must be 1 to 256 bytes without control characters",
            )
            .with_rule("DV-BASE-003"));
        }
        if let Some(t) = patch.thickness_mm {
            if !t.is_finite() {
                return Err(CommandError::new(
                    ErrorCode::NonFiniteNumber,
                    "thickness_mm",
                    "value is not finite",
                )
                .with_rule("DV-ATTR-003"));
            }
            if t > DIMENSION_CAP_MM {
                return Err(CommandError::new(
                    ErrorCode::LimitExceeded,
                    "thickness_mm",
                    "thickness exceeds the dimension cap",
                )
                .with_rule("DV-LIM-001"));
            }
        }
        if let Some(points) = &patch.axis_points_mm {
            if points.len() > MAX_MEMBER_PATH_POINTS {
                return Err(CommandError::new(
                    ErrorCode::LimitExceeded,
                    "axis_points_mm",
                    "path has more than 1024 points",
                )
                .with_rule("DV-LIM-001"));
            }
            if points.iter().flatten().any(|v| !v.is_finite()) {
                return Err(CommandError::new(
                    ErrorCode::NonFiniteNumber,
                    "axis_points_mm",
                    "value is not finite",
                )
                .with_rule("DV-ATTR-003"));
            }
            if points.iter().flatten().any(|v| v.abs() > COORDINATE_CAP_MM) {
                return Err(CommandError::new(
                    ErrorCode::LimitExceeded,
                    "axis_points_mm",
                    "coordinate exceeds the coordinate cap",
                )
                .with_rule("DV-LIM-001"));
            }
        }
        Ok(())
    }

    /// SHA-256 of a versioned, length-prefixed encoding of the validated request.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        self.validate_arguments()?;
        let mut bytes = Vec::with_capacity(128);
        bytes.extend_from_slice(b"BeamCraft/P04/UpdateElement/v2\0");
        bytes.extend_from_slice(self.target_id.as_bytes());
        match &self.patch.name {
            Some(name) => {
                bytes.push(1);
                bytes.extend_from_slice(&(name.len() as u64).to_le_bytes());
                bytes.extend_from_slice(name.as_bytes());
            }
            None => bytes.push(0),
        }
        match self.patch.class {
            Some(class) => {
                bytes.push(1);
                bytes.extend_from_slice(&class.to_le_bytes());
            }
            None => bytes.push(0),
        }
        match &self.patch.material {
            None => bytes.push(0),
            Some(None) => bytes.push(1),
            Some(Some(digest)) => {
                bytes.push(2);
                bytes.extend_from_slice(digest.as_bytes());
            }
        }
        match self.patch.thickness_mm {
            Some(t) => {
                bytes.push(1);
                bytes.extend_from_slice(&canonical_f64_bytes(t).map_err(|_| {
                    CommandError::new(
                        ErrorCode::NonFiniteNumber,
                        "thickness_mm",
                        "value cannot be encoded",
                    )
                    .with_rule("DV-ATTR-003")
                })?);
            }
            None => bytes.push(0),
        }
        // Appended only when present, so the fingerprints of earlier patches are unchanged.
        if let Some(translation) = self.patch.translation_mm {
            bytes.push(2);
            for v in translation {
                bytes.extend_from_slice(&canonical_f64_bytes(v).map_err(|_| {
                    CommandError::new(
                        ErrorCode::NonFiniteNumber,
                        "translation_mm",
                        "value cannot be encoded",
                    )
                    .with_rule("DV-ATTR-003")
                })?);
            }
        }
        // Appended only when present, so the fingerprints of earlier patches are unchanged.
        match self.patch.level {
            None => {}
            Some(None) => bytes.extend_from_slice(&[3, 0]),
            Some(Some(level)) => {
                bytes.extend_from_slice(&[3, 1]);
                bytes.extend_from_slice(level.as_bytes());
            }
        }
        // Appended only when present, so the fingerprints of earlier patches are unchanged.
        if let Some(xyzw) = self.patch.orientation_xyzw {
            bytes.extend_from_slice(&[4, 1]);
            for v in xyzw {
                bytes.extend_from_slice(&canonical_f64_bytes(v).map_err(|_| {
                    CommandError::new(
                        ErrorCode::NonFiniteNumber,
                        "placement.orientation_xyzw",
                        "value cannot be encoded",
                    )
                    .with_rule("DV-ATTR-003")
                })?);
            }
        }
        // Appended only when present, so the fingerprints of earlier patches are unchanged.
        if let Some(side) = self.patch.thickness_side {
            bytes.push(5);
            bytes.push(match side {
                PlateSide::Top => 1,
                PlateSide::Middle => 2,
                PlateSide::Bottom => 3,
            });
        }
        // Appended only when present, so the fingerprints of earlier patches are unchanged.
        if let Some(points) = &self.patch.axis_points_mm {
            bytes.extend_from_slice(&[6, 1]);
            bytes.extend_from_slice(&(points.len() as u64).to_le_bytes());
            for v in points.iter().flatten().copied() {
                bytes.extend_from_slice(&canonical_f64_bytes(v).map_err(|_| {
                    CommandError::new(
                        ErrorCode::NonFiniteNumber,
                        "axis_points_mm",
                        "value cannot be encoded",
                    )
                    .with_rule("DV-ATTR-003")
                })?);
            }
        }
        if let Some(corners) = &self.patch.corners {
            bytes.extend_from_slice(b"A12.3/corner-patch/v1\0");
            bytes.extend_from_slice(&(corners.len() as u64).to_le_bytes());
            for corner in corners {
                bytes.extend_from_slice(&(corner.vertex as u64).to_le_bytes());
                bytes.push(match corner.treatment {
                    CornerTreatment::Chamfer { .. } => 1,
                    CornerTreatment::Round { .. } => 2,
                });
                let values = match corner.treatment {
                    CornerTreatment::Chamfer { d1_mm, d2_mm } => [d1_mm, d2_mm],
                    CornerTreatment::Round { radius_mm } => [radius_mm, radius_mm],
                };
                for v in values {
                    bytes.extend_from_slice(&v.to_le_bytes());
                }
            }
        }
        // Appended only when present, so the fingerprints of earlier patches are unchanged.
        if let Some(points) = &self.patch.contour_points_mm {
            bytes.extend_from_slice(b"A12.3/contour-patch/v1\0");
            bytes.extend_from_slice(&(points.len() as u64).to_le_bytes());
            for v in points.iter().flatten().copied() {
                bytes.extend_from_slice(&canonical_f64_bytes(v).map_err(|_| {
                    CommandError::new(
                        ErrorCode::NonFiniteNumber,
                        "contour_points_mm",
                        "value cannot be encoded",
                    )
                    .with_rule("DV-ATTR-003")
                })?);
            }
        }
        Sha256::digest(&bytes).map_err(|_| {
            CommandError::new(ErrorCode::LimitExceeded, "content", "content is too large")
                .with_rule("DV-LIM-001")
        })
    }
}

/// F04 section 7 `DeleteElement` (P08): removes one owned element. It is refused while another
/// element still links to it (`DEPENDENCY_CONFLICT`); there is no cascade.
#[derive(Debug, Clone, PartialEq)]
pub struct DeleteElement {
    /// The element to delete.
    pub target_id: ElementId,
}

impl DeleteElement {
    /// SHA-256 of a versioned encoding of the request. Whether the element exists and whether
    /// something links to it are the engine's and the layer engine's decisions.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        let mut bytes = Vec::with_capacity(64);
        bytes.extend_from_slice(b"BeamCraft/P08/DeleteElement/v1\0");
        bytes.extend_from_slice(self.target_id.as_bytes());
        Sha256::digest(&bytes).map_err(|_| {
            CommandError::new(ErrorCode::LimitExceeded, "content", "content is too large")
                .with_rule("DV-LIM-001")
        })
    }
}

/// F04 section 7 `CopyElement` (P05): a new element with a fresh identity and the same authored
/// inputs as the source, optionally renamed and translated. The source is only read.
#[derive(Debug, Clone, PartialEq)]
pub struct CopyElement {
    /// The element to copy.
    pub source_id: ElementId,
    /// Identity of the copy.
    pub new_id: ElementId,
    /// Label of the copy; the source label when `None`.
    pub name: Option<String>,
    /// Translation added to the source placement, in millimetres.
    pub offset_mm: [f64; 3],
}

impl CopyElement {
    /// Admits the request: the identities differ (`DUPLICATE_ID`), the label follows the F04 rule,
    /// the offset is finite (`NON_FINITE_NUMBER`) and within the coordinate cap
    /// (`LIMIT_EXCEEDED`). That the source exists and the resulting placement stays in range are
    /// the engine's decisions.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        if self.source_id == self.new_id {
            return Err(CommandError::new(
                ErrorCode::DuplicateId,
                "new_id",
                "the copy identity equals the source",
            )
            .with_rule("DV-BASE-002"));
        }
        if self.name.as_deref().is_some_and(|n| !label_is_valid(n)) {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "name",
                "label must be 1 to 256 bytes without control characters",
            )
            .with_rule("DV-BASE-003"));
        }
        if self.offset_mm.iter().any(|v| !v.is_finite()) {
            return Err(CommandError::new(
                ErrorCode::NonFiniteNumber,
                "offset_mm",
                "value is not finite",
            )
            .with_rule("DV-ATTR-003"));
        }
        if self.offset_mm.iter().any(|v| v.abs() > COORDINATE_CAP_MM) {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "offset_mm",
                "offset component exceeds the coordinate cap",
            )
            .with_rule("DV-LIM-001"));
        }
        Ok(())
    }

    /// SHA-256 of a versioned, length-prefixed encoding of the validated request.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        self.validate_arguments()?;
        let mut bytes = Vec::with_capacity(128);
        bytes.extend_from_slice(b"BeamCraft/P05/CopyElement/v1\0");
        bytes.extend_from_slice(self.source_id.as_bytes());
        bytes.extend_from_slice(self.new_id.as_bytes());
        match &self.name {
            Some(name) => {
                bytes.push(1);
                bytes.extend_from_slice(&(name.len() as u64).to_le_bytes());
                bytes.extend_from_slice(name.as_bytes());
            }
            None => bytes.push(0),
        }
        for v in self.offset_mm {
            bytes.extend_from_slice(&canonical_f64_bytes(v).map_err(|_| {
                CommandError::new(
                    ErrorCode::NonFiniteNumber,
                    "offset_mm",
                    "value cannot be encoded",
                )
                .with_rule("DV-ATTR-003")
            })?);
        }
        Sha256::digest(&bytes).map_err(|_| {
            CommandError::new(ErrorCode::LimitExceeded, "content", "content is too large")
                .with_rule("DV-LIM-001")
        })
    }
}

/// A01 `CreateAssembly`: a new `assembly` element (domain vocabulary section 2.3), a grouping
/// target of `part-of` with no shape, material or derived body of its own. It carries the common
/// base only; its placement is the identity in this slice.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateAssembly {
    /// Identity of the new assembly.
    pub new_id: ElementId,
    /// Explicit user label.
    pub name: String,
}

impl CreateAssembly {
    /// Admits the request: the label follows the F04 rule (`DV-BASE-003`). Whether the identity
    /// is free (`DUPLICATE_ID`) is the engine's decision.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        if !label_is_valid(&self.name) {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "name",
                "label must be 1 to 256 bytes without control characters",
            )
            .with_rule("DV-BASE-003"));
        }
        Ok(())
    }

    /// SHA-256 of a versioned, length-prefixed encoding of the validated request.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        self.validate_arguments()?;
        let mut bytes = Vec::with_capacity(64 + self.name.len());
        bytes.extend_from_slice(b"BeamCraft/A01/CreateAssembly/v1\0");
        bytes.extend_from_slice(self.new_id.as_bytes());
        bytes.extend_from_slice(&(self.name.len() as u64).to_le_bytes());
        bytes.extend_from_slice(self.name.as_bytes());
        Sha256::digest(&bytes).map_err(|_| {
            CommandError::new(ErrorCode::LimitExceeded, "content", "content is too large")
                .with_rule("DV-LIM-001")
        })
    }
}

/// A01 `AttachToAssembly`: authors the one `part-of` record of `part_id` (domain vocabulary
/// section 5.1), so the part becomes a member of `assembly_id`. The record lives with its source,
/// the part; the assembly is only read.
#[derive(Debug, Clone, PartialEq)]
pub struct AttachToAssembly {
    /// The part: a plate, beam, column or (for nesting) another assembly.
    pub part_id: ElementId,
    /// The target assembly.
    pub assembly_id: ElementId,
}

impl AttachToAssembly {
    /// Admits the request: a self-link is `INVALID_ARGUMENT` (`DV-REL-006`). Kinds, cardinality
    /// and cycles (`DV-REL-003` to `DV-REL-005`) need the model and are the engine's decisions.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        if self.part_id == self.assembly_id {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "assembly_id",
                "an element cannot be a part of itself",
            )
            .with_rule("DV-REL-006"));
        }
        Ok(())
    }

    /// SHA-256 of a versioned encoding of the validated request.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        self.validate_arguments()?;
        let mut bytes = Vec::with_capacity(80);
        bytes.extend_from_slice(b"BeamCraft/A01/AttachToAssembly/v1\0");
        bytes.extend_from_slice(self.part_id.as_bytes());
        bytes.extend_from_slice(self.assembly_id.as_bytes());
        Sha256::digest(&bytes).map_err(|_| {
            CommandError::new(ErrorCode::LimitExceeded, "content", "content is too large")
                .with_rule("DV-LIM-001")
        })
    }
}

/// A16.1 `SetAssemblyMainPart`: names `part_id` as the assembly's one main part (A16.1). The
/// record is stored on the assembly as its `bim:mainPart` relationship; the part is only read.
#[derive(Debug, Clone, PartialEq)]
pub struct SetAssemblyMainPart {
    /// The assembly whose main part is named.
    pub assembly_id: ElementId,
    /// The part: a direct member of the assembly (plate, beam, column or nested assembly).
    pub part_id: ElementId,
}

impl SetAssemblyMainPart {
    /// SHA-256 of a versioned encoding of the request. That the part exists and is a direct
    /// member of the assembly, and that the main part is unique, are the engine's decisions.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        let mut bytes = Vec::with_capacity(80);
        bytes.extend_from_slice(b"BeamCraft/A16/SetAssemblyMainPart/v1\0");
        bytes.extend_from_slice(self.assembly_id.as_bytes());
        bytes.extend_from_slice(self.part_id.as_bytes());
        Sha256::digest(&bytes).map_err(|_| {
            CommandError::new(ErrorCode::LimitExceeded, "content", "content is too large")
                .with_rule("DV-LIM-001")
        })
    }
}

/// A16 `ExplodeAssembly`: dissolves one assembly level. The assembly element is deleted and its
/// direct parts lose their `part-of` records; geometry, placement and attributes of the parts do
/// not change, and a nested sub-assembly stays an assembly with its own parts (decision of
/// 2026-10-21: explode means dissolve).
#[derive(Debug, Clone, PartialEq)]
pub struct ExplodeAssembly {
    /// The assembly to dissolve.
    pub assembly_id: ElementId,
}

impl ExplodeAssembly {
    /// SHA-256 of a versioned encoding of the request. That the target exists and is an assembly
    /// is the engine's decision.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        let mut bytes = Vec::with_capacity(64);
        bytes.extend_from_slice(b"BeamCraft/A16/ExplodeAssembly/v1\0");
        bytes.extend_from_slice(self.assembly_id.as_bytes());
        Sha256::digest(&bytes).map_err(|_| {
            CommandError::new(ErrorCode::LimitExceeded, "content", "content is too large")
                .with_rule("DV-LIM-001")
        })
    }
}

/// A01 `DetachFromAssembly`: removes the `part-of` record of `part_id`. The former assembly is
/// not written and stays, even when it has no parts left.
#[derive(Debug, Clone, PartialEq)]
pub struct DetachFromAssembly {
    /// The part whose membership ends.
    pub part_id: ElementId,
}

impl DetachFromAssembly {
    /// SHA-256 of a versioned encoding of the request. Whether the part exists and is a member
    /// of an assembly are the engine's decisions.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        let mut bytes = Vec::with_capacity(64);
        bytes.extend_from_slice(b"BeamCraft/A01/DetachFromAssembly/v1\0");
        bytes.extend_from_slice(self.part_id.as_bytes());
        Sha256::digest(&bytes).map_err(|_| {
            CommandError::new(ErrorCode::LimitExceeded, "content", "content is too large")
                .with_rule("DV-LIM-001")
        })
    }
}

/// D2 `CreateLevel`: a new `level` element (a storey): a name and an elevation in millimetres.
/// A level has no geometry and no placement of its own; plates and members are assigned to it
/// through `UpdateElement` (`ElementPatch::level`), and the elevation moves no element.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateLevel {
    /// Identity of the new level.
    pub new_id: ElementId,
    /// Explicit user label, 1 to 256 UTF-8 bytes without control characters.
    pub name: String,
    /// Elevation in millimetres: finite and at most 1,000,000,000 in magnitude (the coordinate
    /// cap). Negative elevations are allowed (a basement).
    pub elevation_mm: f64,
}

impl CreateLevel {
    /// Admits the request: the label follows the F04 rule (`INVALID_ARGUMENT`), the elevation is
    /// finite (`NON_FINITE_NUMBER`) and within the coordinate cap (`LIMIT_EXCEEDED`). Whether the
    /// identity is free (`DUPLICATE_ID`) is the engine's decision.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        if !label_is_valid(&self.name) {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "name",
                "label must be 1 to 256 bytes without control characters",
            )
            .with_rule("DV-BASE-003"));
        }
        if !self.elevation_mm.is_finite() {
            return Err(CommandError::new(
                ErrorCode::NonFiniteNumber,
                "elevation_mm",
                "value is not finite",
            )
            .with_rule("DV-ATTR-003"));
        }
        if self.elevation_mm.abs() > COORDINATE_CAP_MM {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "elevation_mm",
                "elevation exceeds the coordinate cap",
            )
            .with_rule("DV-LIM-001"));
        }
        Ok(())
    }

    /// SHA-256 of a versioned, length-prefixed encoding of the validated request.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        self.validate_arguments()?;
        let mut bytes = Vec::with_capacity(72 + self.name.len());
        bytes.extend_from_slice(b"BeamCraft/D2/CreateLevel/v1\0");
        bytes.extend_from_slice(self.new_id.as_bytes());
        bytes.extend_from_slice(&(self.name.len() as u64).to_le_bytes());
        bytes.extend_from_slice(self.name.as_bytes());
        bytes.extend_from_slice(&canonical_f64_bytes(self.elevation_mm).map_err(|_| {
            CommandError::new(
                ErrorCode::NonFiniteNumber,
                "elevation_mm",
                "value cannot be encoded",
            )
            .with_rule("DV-ATTR-003")
        })?);
        Sha256::digest(&bytes).map_err(|_| {
            CommandError::new(ErrorCode::LimitExceeded, "content", "content is too large")
                .with_rule("DV-LIM-001")
        })
    }
}

/// The weld types A06 supports. Every other type (butt, plug, spot and so on) is not modelled and
/// has no token here, so it cannot be requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeldType {
    /// A fillet weld.
    Fillet,
}

/// Where a weld is made. It is always authored explicitly; there is no default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeldLocation {
    /// Made in the workshop.
    Workshop,
    /// Made on site.
    Site,
}

/// The side of the joint a weld is on, relative to the main part's reference side. It is always
/// authored explicitly; there is no default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeldSide {
    /// The reference (arrow) side.
    Arrow,
    /// The opposite side.
    Other,
    /// Both sides.
    Both,
}

impl WeldType {
    /// The stored token.
    pub const fn as_token(self) -> &'static str {
        match self {
            Self::Fillet => "fillet",
        }
    }
    /// Parses an exact token.
    pub fn parse(token: &str) -> Option<Self> {
        (token == "fillet").then_some(Self::Fillet)
    }
}

impl WeldLocation {
    /// The stored token.
    pub const fn as_token(self) -> &'static str {
        match self {
            Self::Workshop => "workshop",
            Self::Site => "site",
        }
    }
    /// Parses an exact token.
    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "workshop" => Some(Self::Workshop),
            "site" => Some(Self::Site),
            _ => None,
        }
    }
}

impl WeldSide {
    /// The stored token.
    pub const fn as_token(self) -> &'static str {
        match self {
            Self::Arrow => "arrow",
            Self::Other => "other",
            Self::Both => "both",
        }
    }
    /// Parses an exact token.
    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "arrow" => Some(Self::Arrow),
            "other" => Some(Self::Other),
            "both" => Some(Self::Both),
            _ => None,
        }
    }
}

/// The authored properties of one weld (A06). No solid geometry is derived from them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeldSpec {
    /// The weld type.
    pub weld_type: WeldType,
    /// Nominal size in millimetres: finite, greater than 0 and at most [`WELD_SIZE_CAP_MM`].
    pub size_mm: f64,
    /// Workshop or site.
    pub location: WeldLocation,
    /// Arrow side, other side or both.
    pub side: WeldSide,
}

/// The contour symbol of a weld (A15): the finish of its face as the drawing symbol shows it.
/// `none` is an authored absence; there is no default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeldContour {
    /// No contour symbol.
    None,
    /// Flush contour.
    Flush,
    /// Concave contour.
    Concave,
    /// Convex contour.
    Convex,
}

impl WeldContour {
    /// The stored token.
    pub const fn as_token(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Flush => "flush",
            Self::Concave => "concave",
            Self::Convex => "convex",
        }
    }
    /// Parses an exact token.
    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "none" => Some(Self::None),
            "flush" => Some(Self::Flush),
            "concave" => Some(Self::Concave),
            "convex" => Some(Self::Convex),
            _ => None,
        }
    }
}

/// The symbol data of one weld (A15): what the drawing symbol states besides the type and size.
/// Everything is authored explicitly; the stored bytes keep exactly what was authored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeldSymbol {
    /// All-around symbol: the weld runs around the whole joint perimeter. Symbol data only; the
    /// solid shape stays the authored path.
    pub all_around: bool,
    /// Contour symbol of the weld face.
    pub contour: WeldContour,
}

/// The optional geometry of one weld (A15): the path it runs along and its symbol data. The path
/// is a world-space polyline in millimetres; the type and size of the parent [`WeldSpec`] shape
/// its solid. At least [`MIN_WELD_PATH_POINTS`] and at most [`MAX_WELD_PATH_POINTS`] points, each
/// within the coordinate cap, consecutive points at least [`MIN_WELD_SEGMENT_MM`] apart and a
/// total length of at most [`MAX_WELD_LENGTH_MM`].
#[derive(Debug, Clone, PartialEq)]
pub struct WeldGeometry {
    /// Ordered world points of the path in millimetres.
    pub path_mm: Vec<[f64; 3]>,
    /// The symbol data.
    pub symbol: WeldSymbol,
}

/// A06 `CreateWeld`: a new `connection` element (domain vocabulary section 2.3) that is a weld. Its
/// one `connects` record (section 5.1) names exactly two distinct parts in order, the main part
/// first; the record lives with the weld, so the parts are only read. It carries the common base
/// and the weld properties; it has no body, material or derived geometry, and its placement is
/// the identity in this slice.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateWeld {
    /// Identity of the new weld.
    pub new_id: ElementId,
    /// Explicit user label.
    pub name: String,
    /// The main part: a plate, beam or column.
    pub main_id: ElementId,
    /// The part welded to it: a plate, beam or column.
    pub secondary_id: ElementId,
    /// The weld properties.
    pub weld: WeldSpec,
    /// Optional geometry (A15): the path the weld runs along and its symbol data. `None` is an
    /// A06 weld without a body; `Some` derives the solid and stores the symbol data.
    pub geometry: Option<WeldGeometry>,
}

impl CreateWeld {
    /// Admits the request: the label (`DV-BASE-003`), two distinct parts (`DV-REL-006`,
    /// `DV-REL-007`), a weld that is not one of its own parts, and a finite size in
    /// `(0, WELD_SIZE_CAP_MM]`. Whether the parts exist and their kinds are the engine's decisions.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        if !label_is_valid(&self.name) {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "name",
                "label must be 1 to 256 bytes without control characters",
            )
            .with_rule("DV-BASE-003"));
        }
        if self.main_id == self.secondary_id {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "secondary_id",
                "a weld connects two distinct parts",
            )
            .with_rule("DV-REL-007"));
        }
        if self.new_id == self.main_id || self.new_id == self.secondary_id {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "new_id",
                "a weld cannot connect itself",
            )
            .with_rule("DV-REL-006"));
        }
        let size = self.weld.size_mm;
        if !size.is_finite() {
            return Err(CommandError::new(
                ErrorCode::NonFiniteNumber,
                "size_mm",
                "weld size must be finite",
            )
            .with_rule("DV-ATTR-003"));
        }
        if size <= 0.0 || size > WELD_SIZE_CAP_MM {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "size_mm",
                "weld size must be greater than 0 and at most 1000 mm",
            ));
        }
        self.validate_geometry()
    }

    /// Admits the optional geometry (A15): a bounded path of finite world points within the
    /// coordinate cap, consecutive points at least [`MIN_WELD_SEGMENT_MM`] apart and a total
    /// length of at most [`MAX_WELD_LENGTH_MM`]. Absent geometry passes.
    fn validate_geometry(&self) -> Result<(), CommandError> {
        let Some(geometry) = &self.geometry else {
            return Ok(());
        };
        if geometry.path_mm.len() < MIN_WELD_PATH_POINTS
            || geometry.path_mm.len() > MAX_WELD_PATH_POINTS
        {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "path_mm",
                "a weld path has 2 to 1024 points",
            ));
        }
        let mut total = 0.0;
        for (point, next) in geometry.path_mm.iter().zip(geometry.path_mm.iter().skip(1)) {
            for coordinate in point.iter().chain(next.iter()) {
                if !coordinate.is_finite() {
                    return Err(CommandError::new(
                        ErrorCode::NonFiniteNumber,
                        "path_mm",
                        "a weld path point must be finite",
                    )
                    .with_rule("DV-ATTR-003"));
                }
                if coordinate.abs() > COORDINATE_CAP_MM {
                    return Err(CommandError::new(
                        ErrorCode::InvalidArgument,
                        "path_mm",
                        "a weld path point exceeds the coordinate cap",
                    ));
                }
            }
            let segment = (next[0] - point[0]).powi(2)
                + (next[1] - point[1]).powi(2)
                + (next[2] - point[2]).powi(2);
            if !segment.is_finite() {
                return Err(CommandError::new(
                    ErrorCode::NonFiniteNumber,
                    "path_mm",
                    "a weld path segment must be finite",
                )
                .with_rule("DV-ATTR-003"));
            }
            if segment < MIN_WELD_SEGMENT_MM * MIN_WELD_SEGMENT_MM {
                return Err(CommandError::new(
                    ErrorCode::InvalidArgument,
                    "path_mm",
                    "consecutive weld path points must be at least 0.001 mm apart",
                ));
            }
            total += segment.sqrt();
        }
        if total > MAX_WELD_LENGTH_MM {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "path_mm",
                "the weld path is longer than 1000000 mm",
            ));
        }
        Ok(())
    }

    /// SHA-256 of a versioned, length-prefixed encoding of the validated request.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        self.validate_arguments()?;
        let mut bytes = Vec::with_capacity(128 + self.name.len());
        bytes.extend_from_slice(b"BeamCraft/A06/CreateWeld/v1\0");
        bytes.extend_from_slice(self.new_id.as_bytes());
        bytes.extend_from_slice(&(self.name.len() as u64).to_le_bytes());
        bytes.extend_from_slice(self.name.as_bytes());
        bytes.extend_from_slice(self.main_id.as_bytes());
        bytes.extend_from_slice(self.secondary_id.as_bytes());
        for token in [
            self.weld.weld_type.as_token(),
            self.weld.location.as_token(),
            self.weld.side.as_token(),
        ] {
            // Tokens are short constants; the length prefix keeps the encoding unambiguous.
            bytes.extend_from_slice(&(token.len() as u64).to_le_bytes());
            bytes.extend_from_slice(token.as_bytes());
        }
        let size = canonical_f64_bytes(self.weld.size_mm).map_err(|_| {
            CommandError::new(
                ErrorCode::NonFiniteNumber,
                "size_mm",
                "weld size must be finite",
            )
            .with_rule("DV-ATTR-003")
        })?;
        bytes.extend_from_slice(&size);
        if let Some(geometry) = &self.geometry {
            // A second domain keeps every A06 request (geometry absent) at its exact v1 bytes.
            bytes.extend_from_slice(b"BeamCraft/A15/CreateWeldGeometry/v1\0");
            let count = u64::try_from(geometry.path_mm.len()).map_err(|_| {
                CommandError::new(
                    ErrorCode::LimitExceeded,
                    "path_mm",
                    "the weld path is too long",
                )
            })?;
            bytes.extend_from_slice(&count.to_le_bytes());
            for point in &geometry.path_mm {
                for coordinate in point {
                    bytes.extend_from_slice(&canonical_f64_bytes(*coordinate).map_err(|_| {
                        CommandError::new(
                            ErrorCode::NonFiniteNumber,
                            "path_mm",
                            "a weld path point must be finite",
                        )
                        .with_rule("DV-ATTR-003")
                    })?);
                }
            }
            bytes.extend_from_slice(&[u8::from(geometry.symbol.all_around)]);
            let contour = geometry.symbol.contour.as_token();
            bytes.extend_from_slice(&(contour.len() as u64).to_le_bytes());
            bytes.extend_from_slice(contour.as_bytes());
        }
        Sha256::digest(&bytes).map_err(|_| {
            CommandError::new(ErrorCode::LimitExceeded, "content", "content is too large")
                .with_rule("DV-LIM-001")
        })
    }
}

/// The authored properties of one bolt group (A05). The group is `count` bolts in a straight
/// world line `origin + direction * (pitch * i)`, `i` in `0..count`. No solid geometry is derived
/// from them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoltSpec {
    /// Nominal bolt diameter in millimetres: finite, greater than 0 and at most
    /// [`BOLT_DIAMETER_CAP_MM`].
    pub diameter_mm: f64,
    /// Bolts in the group, 1 to [`MAX_BOLT_GROUP_SIZE`].
    pub count: usize,
    /// Centre-to-centre distance along the line in millimetres: finite, greater than 0 and at
    /// most the dimension cap.
    pub pitch_mm: f64,
    /// World position of the first bolt in millimetres, each component within the coordinate cap.
    pub origin_mm: [f64; 3],
    /// Unit world direction of the line: finite, squared norm within
    /// [`ORIENTATION_NORM_TOLERANCE`] of one. Stored normalized; builders pass a unit vector.
    pub direction_mm: [f64; 3],
}

/// A05 `CreateBolt`: a new `connection` element (domain vocabulary section 2.3) that is a bolt
/// group. Its one `connects` record (section 5.1) names exactly two distinct parts in order, the
/// main part first; the record lives with the group, so the parts are only read. It carries the
/// common base and the bolt properties; it has no body, material or derived geometry, and its
/// placement is the identity in this slice.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateBolt {
    /// Identity of the new bolt group.
    pub new_id: ElementId,
    /// Explicit user label.
    pub name: String,
    /// The main part: a plate, beam or column.
    pub main_id: ElementId,
    /// The part bolted to it: a plate, beam or column.
    pub secondary_id: ElementId,
    /// The bolt properties.
    pub bolt: BoltSpec,
}

impl CreateBolt {
    /// Admits the request: the label (`DV-BASE-003`), two distinct parts (`DV-REL-006`,
    /// `DV-REL-007`), a group that is not one of its own parts, a finite diameter in
    /// `(0, BOLT_DIAMETER_CAP_MM]`, a count of 1 to [`MAX_BOLT_GROUP_SIZE`], a finite pitch in
    /// `(0, dimension cap]`, a finite origin within the coordinate cap and a finite unit
    /// direction. Whether the parts exist, their kinds and whether every bolt position lies on
    /// both parts are the engine's decisions.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        if !label_is_valid(&self.name) {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "name",
                "label must be 1 to 256 bytes without control characters",
            )
            .with_rule("DV-BASE-003"));
        }
        if self.main_id == self.secondary_id {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "secondary_id",
                "a bolt group connects two distinct parts",
            )
            .with_rule("DV-REL-007"));
        }
        if self.new_id == self.main_id || self.new_id == self.secondary_id {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "new_id",
                "a bolt group cannot connect itself",
            )
            .with_rule("DV-REL-006"));
        }
        let bolt = &self.bolt;
        let scalars = [bolt.diameter_mm, bolt.pitch_mm];
        if scalars.iter().any(|v| !v.is_finite()) || bolt.origin_mm.iter().any(|v| !v.is_finite()) {
            return Err(CommandError::new(
                ErrorCode::NonFiniteNumber,
                "bolt",
                "bolt diameter, pitch or origin is not finite",
            )
            .with_rule("DV-ATTR-003"));
        }
        if bolt.direction_mm.iter().any(|v| !v.is_finite()) {
            return Err(CommandError::new(
                ErrorCode::NonFiniteNumber,
                "direction_mm",
                "direction is not finite",
            )
            .with_rule("DV-ATTR-003"));
        }
        if bolt.diameter_mm <= 0.0 || bolt.diameter_mm > BOLT_DIAMETER_CAP_MM {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "diameter_mm",
                "bolt diameter must be greater than 0 and at most 1000 mm",
            ));
        }
        if bolt.count == 0 {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "count",
                "a bolt group has at least one bolt",
            ));
        }
        if bolt.count > MAX_BOLT_GROUP_SIZE {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "count",
                "a bolt group has more than 512 bolts",
            )
            .with_rule("DV-LIM-001"));
        }
        if bolt.pitch_mm <= 0.0 || bolt.pitch_mm > DIMENSION_CAP_MM {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "pitch_mm",
                "bolt pitch must be greater than 0 and at most the dimension cap",
            ));
        }
        if bolt.origin_mm.iter().any(|v| v.abs() > COORDINATE_CAP_MM) {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "origin_mm",
                "origin component exceeds the coordinate cap",
            )
            .with_rule("DV-LIM-001"));
        }
        let squared = bolt.direction_mm.iter().map(|v| v * v).sum::<f64>();
        if (squared - 1.0).abs() > ORIENTATION_NORM_TOLERANCE {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "direction_mm",
                "direction must be a unit vector",
            ));
        }
        Ok(())
    }

    /// SHA-256 of a versioned, length-prefixed encoding of the validated request.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        self.validate_arguments()?;
        let bad = || {
            CommandError::new(
                ErrorCode::NonFiniteNumber,
                "bolt",
                "a bolt property cannot be encoded",
            )
            .with_rule("DV-ATTR-003")
        };
        let mut bytes = Vec::with_capacity(160 + self.name.len());
        bytes.extend_from_slice(b"BeamCraft/A05/CreateBolt/v1\0");
        bytes.extend_from_slice(self.new_id.as_bytes());
        bytes.extend_from_slice(&(self.name.len() as u64).to_le_bytes());
        bytes.extend_from_slice(self.name.as_bytes());
        bytes.extend_from_slice(self.main_id.as_bytes());
        bytes.extend_from_slice(self.secondary_id.as_bytes());
        for v in [self.bolt.diameter_mm, self.bolt.pitch_mm] {
            bytes.extend_from_slice(&canonical_f64_bytes(v).map_err(|_| bad())?);
        }
        bytes.extend_from_slice(&(self.bolt.count as u64).to_le_bytes());
        for v in self
            .bolt
            .origin_mm
            .iter()
            .copied()
            .chain(self.bolt.direction_mm)
        {
            bytes.extend_from_slice(&canonical_f64_bytes(v).map_err(|_| bad())?);
        }
        Sha256::digest(&bytes).map_err(|_| {
            CommandError::new(ErrorCode::LimitExceeded, "content", "content is too large")
                .with_rule("DV-LIM-001")
        })
    }
}

/// A08 `ApproveMark`: a person approves the mark of one element, freezing its value. Whether the
/// element exists, carries a mark and is not already approved are the engine's decisions.
#[derive(Debug, Clone, PartialEq)]
pub struct ApproveMark {
    /// The element whose mark is approved.
    pub target_id: ElementId,
}

impl ApproveMark {
    /// SHA-256 of a versioned encoding of the request.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        let mut bytes = Vec::with_capacity(64);
        bytes.extend_from_slice(b"BeamCraft/A08/ApproveMark/v1\0");
        bytes.extend_from_slice(self.target_id.as_bytes());
        Sha256::digest(&bytes).map_err(|_| {
            CommandError::new(ErrorCode::LimitExceeded, "content", "content is too large")
                .with_rule("DV-LIM-001")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plate() -> CreatePlate {
        CreatePlate {
            new_id: ElementId::parse("019a7bf0-0000-7000-8000-000000000005").unwrap(),
            name: "Synthetic plate".into(),
            class: 0,
            contour_points_mm: vec![
                [0.0, 0.0, 0.0],
                [1000.0, 0.0, 0.0],
                [1000.0, 500.0, 0.0],
                [0.0, 500.0, 0.0],
            ],
            thickness_mm: 10.0,
            thickness_side: None,
            corners: Vec::new(),
            placement: Placement::IDENTITY,
            material_digest: None,
            holes_mm: Vec::new(),
        }
    }

    fn code(plate: &CreatePlate) -> ErrorCode {
        plate.validate_arguments().unwrap_err().code
    }

    #[test]
    fn the_thickness_side_has_wire_tokens_exact_offsets_and_its_own_fingerprint() {
        // The tokens round-trip; anything else does not parse.
        for (side, token) in [
            (PlateSide::Top, "top"),
            (PlateSide::Middle, "middle"),
            (PlateSide::Bottom, "bottom"),
        ] {
            assert_eq!(side.as_str(), token);
            assert_eq!(PlateSide::parse(token), Some(side));
        }
        assert_eq!(PlateSide::parse("above"), None);
        // The lower face sits at 0, -t/2 and -t; halving and negating are exact.
        assert_eq!(PlateSide::Top.offset_mm(10.0), 0.0);
        assert_eq!(PlateSide::Middle.offset_mm(10.0), -5.0);
        assert_eq!(PlateSide::Bottom.offset_mm(10.0), -10.0);
        // Absence is the top default in content: an older plate and an explicit top plate
        // evaluate the same body, but the authored arguments stay distinguishable.
        let base = plate();
        let top = CreatePlate {
            thickness_side: Some(PlateSide::Top),
            ..base.clone()
        };
        let middle = CreatePlate {
            thickness_side: Some(PlateSide::Middle),
            ..base.clone()
        };
        assert_ne!(
            base.content_fingerprint().unwrap(),
            middle.content_fingerprint().unwrap()
        );
        assert_ne!(
            top.content_fingerprint().unwrap(),
            middle.content_fingerprint().unwrap()
        );
        assert_ne!(
            base.content_fingerprint().unwrap(),
            top.content_fingerprint().unwrap()
        );
    }

    #[test]
    fn corner_treatments_are_admitted_per_vertex_and_in_the_fingerprint() {
        // A12.2: one treatment per vertex, each naming a real vertex, each with bounded values.
        let chamfer = |vertex, d1, d2| {
            let mut c = plate();
            c.corners = vec![PlateCorner {
                vertex,
                treatment: CornerTreatment::Chamfer {
                    d1_mm: d1,
                    d2_mm: d2,
                },
            }];
            c
        };
        let round = |vertex, r| {
            let mut c = plate();
            c.corners = vec![PlateCorner {
                vertex,
                treatment: CornerTreatment::Round { radius_mm: r },
            }];
            c
        };
        assert!(chamfer(0, 100.0, 100.0).validate_arguments().is_ok());
        assert!(round(3, 50.0).validate_arguments().is_ok());
        // A vertex that does not exist, and a vertex named twice.
        assert_eq!(code(&chamfer(4, 100.0, 100.0)), ErrorCode::InvalidArgument);
        let mut twice = plate();
        twice.corners = vec![
            PlateCorner {
                vertex: 1,
                treatment: CornerTreatment::Round { radius_mm: 10.0 },
            },
            PlateCorner {
                vertex: 1,
                treatment: CornerTreatment::Round { radius_mm: 20.0 },
            },
        ];
        assert_eq!(code(&twice), ErrorCode::InvalidArgument);
        // Zero, negative, non-finite and over the cap (the amendment's negative case 1).
        assert_eq!(code(&chamfer(0, 0.0, 100.0)), ErrorCode::InvalidArgument);
        assert_eq!(code(&round(0, -1.0)), ErrorCode::InvalidArgument);
        assert_eq!(code(&round(0, f64::NAN)), ErrorCode::NonFiniteNumber);
        assert_eq!(
            code(&round(0, COORDINATE_CAP_MM * 2.0)),
            ErrorCode::LimitExceeded
        );
        // Different treatments fingerprint differently, and the empty list differs from a
        // treated plate (the v8 tag covers the new block).
        let base = plate().content_fingerprint().unwrap();
        let a = chamfer(0, 100.0, 100.0).content_fingerprint().unwrap();
        let b = chamfer(0, 100.0, 50.0).content_fingerprint().unwrap();
        let c = round(0, 100.0).content_fingerprint().unwrap();
        let other_vertex = chamfer(1, 100.0, 100.0).content_fingerprint().unwrap();
        assert_ne!(base, a);
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, other_vertex);
    }

    fn hole() -> Vec<[f64; 2]> {
        vec![
            [100.0, 100.0],
            [100.0, 200.0],
            [200.0, 200.0],
            [200.0, 100.0],
        ]
    }

    fn add_hole() -> AddHole {
        AddHole {
            target_id: ElementId::parse("019a7bf0-0000-7000-8000-000000000006").unwrap(),
            operation_id: OpId::parse("019a7bf0-0000-7000-8000-000000000007").unwrap(),
            hole: HoleSpec::Polygon(hole()),
        }
    }

    fn hole_code(command: &AddHole) -> ErrorCode {
        command.validate_arguments().unwrap_err().code
    }

    #[test]
    fn holes_have_caps_codes_and_part_of_the_fingerprint() {
        let solid = plate().content_fingerprint().unwrap();
        let mut p = plate();
        p.holes_mm = vec![hole()];
        assert!(p.validate_arguments().is_ok());
        assert_ne!(p.content_fingerprint().unwrap(), solid);
        let mut moved = p.clone();
        moved.holes_mm[0][0][0] = 101.0;
        assert_ne!(
            moved.content_fingerprint().unwrap(),
            p.content_fingerprint().unwrap()
        );
        let mut bad = plate();
        bad.holes_mm = vec![vec![[1.0, 1.0], [2.0, 2.0]]];
        assert_eq!(code(&bad), ErrorCode::InvalidArgument);
        bad.holes_mm = vec![vec![[f64::NAN, 1.0], [2.0, 2.0], [3.0, 1.0]]];
        assert_eq!(code(&bad), ErrorCode::NonFiniteNumber);
        bad.holes_mm = vec![vec![[2e9, 1.0], [2.0, 2.0], [3.0, 1.0]]];
        assert_eq!(code(&bad), ErrorCode::LimitExceeded);
        bad.holes_mm = vec![hole(); MAX_HOLES + 1];
        assert_eq!(code(&bad), ErrorCode::LimitExceeded);
        bad.holes_mm = vec![vec![[0.0, 0.0]; HOLE_POINT_CAP + 1]];
        assert_eq!(code(&bad), ErrorCode::LimitExceeded);
        bad.holes_mm = vec![hole(); MAX_HOLES];
        assert!(bad.validate_arguments().is_ok());
    }

    #[test]
    fn an_add_hole_admits_generators_and_fingerprints_its_content() {
        let command = add_hole();
        assert!(command.validate_arguments().is_ok());
        // The polygon is authoritative; the generator parameters are part of the content.
        assert_eq!(command.polygon().unwrap(), hole());
        let fingerprint = command.content_fingerprint().unwrap();
        assert_eq!(command.content_fingerprint().unwrap(), fingerprint);
        let mut moved = command.clone();
        moved.hole = HoleSpec::Polygon(vec![
            [100.0, 100.0],
            [100.0, 201.0],
            [200.0, 200.0],
            [200.0, 100.0],
        ]);
        assert_ne!(moved.content_fingerprint().unwrap(), fingerprint);
        // The same polygon from a generator is still a different command.
        let mut from_generator = command.clone();
        from_generator.hole = HoleSpec::Rectangle {
            corner_mm: [100.0, 100.0],
            width_mm: 100.0,
            height_mm: 100.0,
        };
        assert_ne!(from_generator.content_fingerprint().unwrap(), fingerprint);
        assert_eq!(from_generator.polygon().unwrap(), hole());
    }

    #[test]
    fn a_round_hole_decides_n_from_the_fixed_threshold_table() {
        let round = |diameter: f64| HoleSpec::Round {
            centre_mm: [250.0, 250.0],
            diameter_mm: diameter,
        };
        // The examples of F06 section 2: about 5.2 mm gives N = 16, 100 mm gives 72 and
        // 500 mm gives 160.
        assert_eq!(round(5.2).polygon().unwrap().len(), 16);
        assert_eq!(round(100.0).polygon().unwrap().len(), 72);
        assert_eq!(round(500.0).polygon().unwrap().len(), 160);
        // A radius above a threshold needs the next multiple of four.
        assert_eq!(round(5.3).polygon().unwrap().len(), 20);
        // A radius above the last threshold needs more than 256 segments and is refused,
        // because no Approximated status exists.
        let error = round(1329.0).polygon().unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedGeometry);
        assert_eq!(error.field, "hole.radius");
        // Vertices start on +X of the centre and run clockwise (below it next).
        let polygon = round(100.0).polygon().unwrap();
        assert_eq!(polygon[0], [300.0, 250.0]);
        assert!(polygon[1][1] < 250.0);
    }

    #[test]
    fn an_add_hole_carries_the_caps_codes() {
        let mut bad = add_hole();
        bad.hole = HoleSpec::Polygon(vec![[1.0, 1.0], [2.0, 2.0]]);
        assert_eq!(hole_code(&bad), ErrorCode::InvalidArgument);
        bad.hole = HoleSpec::Polygon(vec![[f64::NAN, 1.0], [2.0, 2.0], [3.0, 1.0]]);
        assert_eq!(hole_code(&bad), ErrorCode::NonFiniteNumber);
        bad.hole = HoleSpec::Polygon(vec![[2e9, 1.0], [2.0, 2.0], [3.0, 1.0]]);
        assert_eq!(hole_code(&bad), ErrorCode::LimitExceeded);
        bad.hole = HoleSpec::Polygon(vec![[0.0, 0.0]; HOLE_POINT_CAP + 1]);
        assert_eq!(hole_code(&bad), ErrorCode::LimitExceeded);
        bad.hole = HoleSpec::Round {
            centre_mm: [0.0, 0.0],
            diameter_mm: 0.0,
        };
        assert_eq!(hole_code(&bad), ErrorCode::InvalidArgument);
        bad.hole = HoleSpec::Round {
            centre_mm: [0.0, 0.0],
            diameter_mm: f64::NAN,
        };
        assert_eq!(hole_code(&bad), ErrorCode::NonFiniteNumber);
        bad.hole = HoleSpec::Rectangle {
            corner_mm: [0.0, 0.0],
            width_mm: 0.0,
            height_mm: 1.0,
        };
        assert_eq!(hole_code(&bad), ErrorCode::InvalidArgument);
        bad.hole = HoleSpec::Rectangle {
            corner_mm: [2e9, 0.0],
            width_mm: 1.0,
            height_mm: 1.0,
        };
        assert_eq!(hole_code(&bad), ErrorCode::LimitExceeded);
    }

    fn cut() -> CutByPlane {
        CutByPlane {
            target_id: ElementId::parse("019a7bf0-0000-7000-8000-000000000005").unwrap(),
            operation_id: OpId::parse("019a7bf0-0000-7000-8000-0000000000c7").unwrap(),
            point_mm: [250.0, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
            keep: KeepSide::Negative,
        }
    }

    #[test]
    fn a_cut_command_has_caps_codes_and_a_content_fingerprint() {
        let c = cut();
        assert!(c.validate_arguments().is_ok());
        let base = c.content_fingerprint().unwrap();
        assert_eq!(base, c.content_fingerprint().unwrap());
        let mut moved = c;
        moved.point_mm[0] = 251.0;
        assert_ne!(moved.content_fingerprint().unwrap(), base);
        let mut flipped = c;
        flipped.keep = KeepSide::Positive;
        assert_ne!(flipped.content_fingerprint().unwrap(), base);
        // A zero normal is not a plane; finiteness and the coordinate cap come first.
        let zero = CutByPlane {
            normal: [0.0; 3],
            ..cut()
        };
        assert_eq!(
            zero.validate_arguments().unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        let nan = CutByPlane {
            point_mm: [f64::NAN, 0.0, 0.0],
            ..cut()
        };
        assert_eq!(
            nan.validate_arguments().unwrap_err().code,
            ErrorCode::NonFiniteNumber
        );
        let wide = CutByPlane {
            point_mm: [2e9, 0.0, 0.0],
            ..cut()
        };
        assert_eq!(
            wide.validate_arguments().unwrap_err().code,
            ErrorCode::LimitExceeded
        );
        assert_eq!(KeepSide::parse("positive"), Some(KeepSide::Positive));
        assert_eq!(KeepSide::parse("sideways"), None);
    }

    #[test]
    fn a_rectangle_is_admitted() {
        assert!(plate().validate_arguments().is_ok());
    }

    #[test]
    fn labels_follow_the_f04_rule() {
        for (name, ok) in [
            ("a", true),
            ("x".repeat(256).as_str(), true),
            ("x".repeat(257).as_str(), false),
            ("", false),
            ("line\nbreak", false),
            ("nul\0", false),
            ("del\u{7f}", false),
            ("c1\u{85}", true),
            ("é".repeat(128).as_str(), true),
            ("é".repeat(129).as_str(), false),
        ] {
            let mut p = plate();
            p.name = name.into();
            assert_eq!(p.validate_arguments().is_ok(), ok, "{name:?}");
        }
    }

    #[test]
    fn contour_count_finiteness_and_caps_have_their_codes() {
        let mut p = plate();
        p.contour_points_mm.truncate(2);
        assert_eq!(code(&p), ErrorCode::InvalidArgument); // F04-NEG-31
        let mut p = plate();
        p.contour_points_mm = vec![[0.0; 3]; 1025];
        assert_eq!(code(&p), ErrorCode::LimitExceeded);
        let mut p = plate();
        p.contour_points_mm[1][0] = f64::NAN;
        assert_eq!(code(&p), ErrorCode::NonFiniteNumber);
        let mut p = plate();
        p.contour_points_mm[2][1] = f64::NEG_INFINITY;
        assert_eq!(code(&p), ErrorCode::NonFiniteNumber);
        let mut p = plate();
        p.thickness_mm = f64::INFINITY;
        assert_eq!(code(&p), ErrorCode::NonFiniteNumber);
        let mut p = plate();
        p.contour_points_mm[2][1] = 1_000_000_000.5;
        assert_eq!(code(&p), ErrorCode::LimitExceeded);
        let mut p = plate();
        p.contour_points_mm[2][1] = 1_000_000_000.0;
        assert!(p.validate_arguments().is_ok());
        let mut p = plate();
        p.thickness_mm = 1_000_000.5;
        assert_eq!(code(&p), ErrorCode::LimitExceeded);
    }

    #[test]
    fn placement_rules_match_the_f04_cases() {
        let q = |x: f64, y: f64, z: f64, w: f64| {
            let mut p = plate();
            p.placement.orientation_xyzw = [x, y, z, w];
            p.validate_arguments()
        };
        assert!(q(0.0, 0.0, 0.0, 1.0).is_ok());
        assert!(q(0.0, 0.0, 0.0, -1.0).is_ok()); // F04-POS-12
        assert!(q(0.0, 0.0, 0.0, 0.99999995).is_ok()); // F04-POS-11
        let s = core::f64::consts::FRAC_1_SQRT_2;
        assert!(q(0.0, 0.0, s, s).is_ok());
        for (x, y, z, w) in [
            (0.0, 0.0, 0.0, 0.5),      // F04-NEG-22
            (0.0, 0.0, 0.0, 1.5),      // F04-NEG-23
            (0.0, 0.0, 0.0, 0.0),      // F04-NEG-24
            (0.0, 0.0, 0.0, 0.999999), // F04-NEG-25
        ] {
            assert_eq!(q(x, y, z, w).unwrap_err().code, ErrorCode::InvalidArgument);
        }
        assert_eq!(
            q(0.0, f64::NAN, 0.0, 1.0).unwrap_err().code,
            ErrorCode::NonFiniteNumber
        );
        let mut p = plate();
        p.placement.translation_mm = [1_000_000_000.0, 0.0, 0.0]; // F04-POS-09
        assert!(p.validate_arguments().is_ok());
        p.placement.translation_mm = [0.0, -1_000_000_001.0, 0.0];
        assert_eq!(code(&p), ErrorCode::LimitExceeded);
        p.placement.translation_mm = [0.0, 0.0, f64::INFINITY];
        assert_eq!(code(&p), ErrorCode::NonFiniteNumber);
    }

    #[test]
    fn identity_rotation_ignores_the_sign_of_w() {
        let mut p = Placement::IDENTITY;
        assert!(p.is_identity_rotation());
        p.orientation_xyzw = [0.0, 0.0, 0.0, -1.0];
        assert!(p.is_identity_rotation());
        let s = core::f64::consts::FRAC_1_SQRT_2;
        p.orientation_xyzw = [0.0, 0.0, s, s];
        assert!(!p.is_identity_rotation());
    }

    #[test]
    fn the_fingerprint_tracks_content_and_ignores_signed_zero() {
        let a = plate().content_fingerprint().unwrap();
        assert_eq!(a, plate().content_fingerprint().unwrap());
        let mut zero = plate();
        zero.contour_points_mm[0][0] = -0.0;
        assert_eq!(zero.content_fingerprint().unwrap(), a);
        for mutate in [
            |p: &mut CreatePlate| p.thickness_mm = 10.5,
            |p: &mut CreatePlate| p.name = "Other".into(),
            |p: &mut CreatePlate| p.contour_points_mm[1][0] = 750.0,
            |p: &mut CreatePlate| p.placement.translation_mm[2] = 1.0,
            |p: &mut CreatePlate| {
                p.new_id = ElementId::parse("019a7bf0-0000-7000-8000-000000000006").unwrap();
            },
        ] {
            let mut p = plate();
            mutate(&mut p);
            assert_ne!(p.content_fingerprint().unwrap(), a);
        }
        let mut bad = plate();
        bad.thickness_mm = f64::NAN;
        assert!(bad.content_fingerprint().is_err());
    }

    #[test]
    fn a_material_reference_is_admitted_and_part_of_the_fingerprint() {
        let digest = |text: &[u8]| crate::digest::Sha256::digest(text).unwrap();
        let none = plate().content_fingerprint().unwrap();
        let mut a = plate();
        a.material_digest = Some(digest(b"a"));
        assert!(a.validate_arguments().is_ok());
        let mut b = plate();
        b.material_digest = Some(digest(b"b"));
        let fingerprints = [
            none,
            a.content_fingerprint().unwrap(),
            b.content_fingerprint().unwrap(),
        ];
        assert_ne!(fingerprints[0], fingerprints[1]);
        assert_ne!(fingerprints[1], fingerprints[2]);
        assert_ne!(fingerprints[0], fingerprints[2]);
    }

    fn member() -> CreateStraightMember {
        let digest = |text: &[u8]| crate::digest::Sha256::digest(text).unwrap();
        CreateStraightMember {
            new_id: ElementId::parse("019a7bf0-0000-7000-8000-000000000007").unwrap(),
            kind: MemberKind::Beam,
            name: "Synthetic beam".into(),
            class: 0,
            axis_points_mm: [[0.0; 3], [6000.0, 0.0, 0.0]],
            profile_digest: digest(b"profile"),
            placement: Placement::IDENTITY,
            roll_rad: None,
            section_placement: None,
            material_digest: None,
        }
    }

    #[test]
    fn the_section_placement_tokens_half_shifts_and_default_are_exact() {
        use SectionDepthPosition as D;
        use SectionPlanePosition as P;
        for (token, plane) in [
            ("centre", P::Centre),
            ("left", P::Left),
            ("right", P::Right),
        ] {
            assert_eq!(P::parse(token), Some(plane));
            assert_eq!(plane.as_str(), token);
        }
        for (token, depth) in [
            ("centre", D::Centre),
            ("front", D::Front),
            ("back", D::Back),
        ] {
            assert_eq!(D::parse(token), Some(depth));
            assert_eq!(depth.as_str(), token);
        }
        assert_eq!(P::parse("above"), None);
        assert_eq!(D::parse("top"), None);
        // Left and front are the negative halves; half shifts are exact.
        assert_eq!(P::Centre.half_shift_mm(200.0), 0.0);
        assert_eq!(P::Left.half_shift_mm(200.0), -100.0);
        assert_eq!(P::Right.half_shift_mm(200.0), 100.0);
        assert_eq!(D::Front.half_shift_mm(300.0), -150.0);
        assert_eq!(D::Back.half_shift_mm(300.0), 150.0);
        assert!(SectionPlacement::default().is_default());
        let neg_zero = SectionPlacement {
            plane_offset_mm: -0.0,
            ..SectionPlacement::default()
        };
        assert!(neg_zero.is_default(), "-0.0 is 0.0");
        let shifted = SectionPlacement {
            plane_offset_mm: 0.5,
            ..SectionPlacement::default()
        };
        assert!(!shifted.is_default());
    }

    #[test]
    fn a_member_section_placement_has_the_argument_rules_and_its_own_fingerprint() {
        fn placed(mutate: impl FnOnce(&mut SectionPlacement)) -> SectionPlacement {
            let mut p = SectionPlacement::default();
            mutate(&mut p);
            p
        }
        let base = member().content_fingerprint().unwrap();
        let mut m = member();
        m.section_placement = Some(placed(|p| p.plane = SectionPlanePosition::Left));
        assert!(m.validate_arguments().is_ok());
        assert_ne!(m.content_fingerprint().unwrap(), base);
        let left = m.content_fingerprint().unwrap();
        m.section_placement = Some(placed(|p| p.plane = SectionPlanePosition::Right));
        assert_ne!(m.content_fingerprint().unwrap(), left);
        for mutate in [
            |p: &mut SectionPlacement| p.plane_offset_mm = f64::NAN,
            |p: &mut SectionPlacement| p.start_offset_mm = f64::INFINITY,
            |p: &mut SectionPlacement| p.end_offset_mm = f64::NAN,
        ] {
            let mut m = member();
            m.section_placement = Some(placed(mutate));
            assert_eq!(
                m.validate_arguments().unwrap_err().code,
                ErrorCode::NonFiniteNumber
            );
        }
        let mut m = member();
        m.section_placement = Some(placed(|p| p.plane_offset_mm = 1_000_000.5));
        assert_eq!(
            m.validate_arguments().unwrap_err().code,
            ErrorCode::LimitExceeded
        );
        // An explicit default placement is valid authored input of its own.
        let mut m = member();
        m.section_placement = Some(SectionPlacement::default());
        assert!(m.validate_arguments().is_ok());
        assert_ne!(m.content_fingerprint().unwrap(), base);
    }

    #[test]
    fn a_member_has_the_f04_argument_rules() {
        assert!(member().validate_arguments().is_ok());
        let code = |m: CreateStraightMember| m.validate_arguments().unwrap_err().code;
        let mut m = member();
        m.name = "line\nbreak".into();
        assert_eq!(code(m), ErrorCode::InvalidArgument);
        let mut m = member();
        m.axis_points_mm[1][2] = f64::NAN;
        assert_eq!(code(m), ErrorCode::NonFiniteNumber);
        let mut m = member();
        m.roll_rad = Some(f64::INFINITY);
        assert_eq!(code(m), ErrorCode::NonFiniteNumber);
        let mut m = member();
        m.axis_points_mm[0][0] = 1_000_000_000.5;
        assert_eq!(code(m), ErrorCode::LimitExceeded);
        let mut m = member();
        m.roll_rad = Some(core::f64::consts::TAU);
        assert!(m.validate_arguments().is_ok());
        m.roll_rad = Some(6.3);
        assert_eq!(code(m), ErrorCode::LimitExceeded);
        let mut m = member();
        m.placement.orientation_xyzw = [0.0, 0.0, 0.0, 0.5];
        assert_eq!(code(m), ErrorCode::InvalidArgument);
    }

    #[test]
    fn the_member_fingerprint_tracks_every_input() {
        let base = member().content_fingerprint().unwrap();
        let digest = |text: &[u8]| crate::digest::Sha256::digest(text).unwrap();
        for mutate in [
            |m: &mut CreateStraightMember| m.kind = MemberKind::Column,
            |m: &mut CreateStraightMember| m.name = "Other".into(),
            |m: &mut CreateStraightMember| m.axis_points_mm[1][0] = 6001.0,
            |m: &mut CreateStraightMember| m.roll_rad = Some(0.5),
            |m: &mut CreateStraightMember| m.placement.translation_mm[1] = 1.0,
        ] {
            let mut m = member();
            mutate(&mut m);
            assert_ne!(m.content_fingerprint().unwrap(), base);
        }
        let mut m = member();
        m.profile_digest = digest(b"other profile");
        assert_ne!(m.content_fingerprint().unwrap(), base);
        let mut m = member();
        m.material_digest = Some(digest(b"material"));
        assert_ne!(m.content_fingerprint().unwrap(), base);
        // Signed zero and an explicit zero roll are not the same authored input, but -0.0 is 0.0.
        let mut m = member();
        m.axis_points_mm[0][0] = -0.0;
        assert_eq!(m.content_fingerprint().unwrap(), base);
    }

    fn polybeam() -> CreatePolybeam {
        CreatePolybeam {
            new_id: ElementId::parse("019a7bf0-0000-7000-8000-000000000017").unwrap(),
            kind: MemberKind::Beam,
            name: "Polybeam".into(),
            class: 0,
            path_points_mm: vec![[0.0; 3], [3000.0, 0.0, 0.0], [3000.0, 4000.0, 0.0]],
            profile_digest: crate::digest::Sha256::digest(b"profile").unwrap(),
            placement: Placement::IDENTITY,
            material_digest: None,
        }
    }

    #[test]
    fn a_polybeam_has_the_f04_argument_rules() {
        use crate::limits::MAX_MEMBER_PATH_POINTS;
        assert!(polybeam().validate_arguments().is_ok());
        let code = |m: CreatePolybeam| m.validate_arguments().unwrap_err().code;
        // Two points author a straight member (P03), never a polybeam.
        let mut m = polybeam();
        m.path_points_mm.truncate(2);
        assert_eq!(code(m), ErrorCode::InvalidArgument);
        let mut m = polybeam();
        m.path_points_mm = vec![[0.0; 3]; MAX_MEMBER_PATH_POINTS + 1];
        assert_eq!(code(m), ErrorCode::LimitExceeded);
        let mut m = polybeam();
        m.path_points_mm[1][2] = f64::NAN;
        assert_eq!(code(m), ErrorCode::NonFiniteNumber);
        let mut m = polybeam();
        m.path_points_mm[2][1] = 1_000_000_000.5;
        assert_eq!(code(m), ErrorCode::LimitExceeded);
        let mut m = polybeam();
        m.name = "line\nbreak".into();
        assert_eq!(code(m), ErrorCode::InvalidArgument);
        let mut m = polybeam();
        m.placement.translation_mm[1] = f64::INFINITY;
        assert_eq!(code(m), ErrorCode::NonFiniteNumber);
    }

    #[test]
    fn the_polybeam_fingerprint_tracks_the_whole_path() {
        let base = polybeam().content_fingerprint().unwrap();
        for mutate in [
            |m: &mut CreatePolybeam| m.kind = MemberKind::Column,
            |m: &mut CreatePolybeam| m.name = "Other".into(),
            |m: &mut CreatePolybeam| {
                m.path_points_mm[1] = [3000.0, 1000.0, 0.0];
            },
            |m: &mut CreatePolybeam| m.path_points_mm.push([6000.0, 4000.0, 0.0]),
            |m: &mut CreatePolybeam| m.placement.translation_mm[1] = 1.0,
        ] {
            let mut m = polybeam();
            mutate(&mut m);
            assert_ne!(m.content_fingerprint().unwrap(), base);
        }
    }

    fn copy() -> CopyElement {
        CopyElement {
            source_id: ElementId::parse("019a7bf0-0000-7000-8000-000000000005").unwrap(),
            new_id: ElementId::parse("019a7bf0-0000-7000-8000-000000000008").unwrap(),
            name: None,
            offset_mm: [100.0, 0.0, 0.0],
        }
    }

    #[test]
    fn a_copy_has_the_f04_argument_rules() {
        assert!(copy().validate_arguments().is_ok());
        let code = |c: CopyElement| c.validate_arguments().unwrap_err().code;
        let mut c = copy();
        c.new_id = c.source_id;
        assert_eq!(code(c), ErrorCode::DuplicateId);
        let mut c = copy();
        c.name = Some(String::new());
        assert_eq!(code(c), ErrorCode::InvalidArgument);
        let mut c = copy();
        c.offset_mm[2] = f64::NAN;
        assert_eq!(code(c), ErrorCode::NonFiniteNumber);
        let mut c = copy();
        c.offset_mm[0] = 1_000_000_001.0;
        assert_eq!(code(c), ErrorCode::LimitExceeded);
    }

    #[test]
    fn the_copy_fingerprint_tracks_every_input() {
        let base = copy().content_fingerprint().unwrap();
        for mutate in [
            |c: &mut CopyElement| c.name = Some("Other".into()),
            |c: &mut CopyElement| c.offset_mm[1] = 1.0,
            |c: &mut CopyElement| {
                c.new_id = ElementId::parse("019a7bf0-0000-7000-8000-000000000009").unwrap()
            },
            |c: &mut CopyElement| {
                c.source_id = ElementId::parse("019a7bf0-0000-7000-8000-000000000004").unwrap()
            },
        ] {
            let mut c = copy();
            mutate(&mut c);
            assert_ne!(c.content_fingerprint().unwrap(), base);
        }
    }

    fn id(n: u8) -> ElementId {
        ElementId::parse(&format!("019a7bf0-0000-7000-8000-0000000000{n:02x}")).unwrap()
    }

    #[test]
    fn an_orientation_patch_has_argument_rules_and_a_fingerprint() {
        let update = |orientation: Option<[f64; 4]>| UpdateElement {
            target_id: id(5),
            patch: ElementPatch {
                orientation_xyzw: orientation,
                name: Some("Beam".into()),
                ..ElementPatch::default()
            },
        };
        let quarter = [0.0, 0.0, 0.5_f64.sqrt(), 0.5_f64.sqrt()];
        let base = update(None).content_fingerprint().unwrap();
        assert_ne!(update(Some(quarter)).content_fingerprint().unwrap(), base);
        // Setting the identity is a real (clearing) change with its own fingerprint.
        assert_ne!(
            update(Some([0.0, 0.0, 0.0, 1.0]))
                .content_fingerprint()
                .unwrap(),
            base
        );
        let code = |u: UpdateElement| u.validate_arguments().unwrap_err().code;
        assert_eq!(
            code(update(Some([0.0, 0.0, 0.5, 0.5]))),
            ErrorCode::InvalidArgument
        );
        assert_eq!(code(update(Some([0.0; 4]))), ErrorCode::InvalidArgument);
        assert_eq!(
            code(update(Some([f64::NAN, 0.0, 0.0, 1.0]))),
            ErrorCode::NonFiniteNumber
        );
        assert!(update(Some(quarter)).validate_arguments().is_ok());
        // The fingerprint covers the quaternion bytes themselves.
        let negated = [-quarter[0], -quarter[1], -quarter[2], -quarter[3]];
        assert_ne!(
            update(Some(negated)).content_fingerprint().unwrap(),
            update(Some(quarter)).content_fingerprint().unwrap()
        );
    }

    #[test]
    fn non_finite_orientation_reports_the_domain_rule_in_validation_and_encoding() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            for component in 0..4 {
                let mut orientation = [0.0, 0.0, 0.0, 1.0];
                orientation[component] = value;
                let update = UpdateElement {
                    target_id: id(5),
                    patch: ElementPatch {
                        orientation_xyzw: Some(orientation),
                        ..ElementPatch::default()
                    },
                };
                for error in [
                    Placement::validate_orientation(orientation).unwrap_err(),
                    update.validate_arguments().unwrap_err(),
                    update.content_fingerprint().unwrap_err(),
                ] {
                    assert_eq!(error.code, ErrorCode::NonFiniteNumber);
                    assert_eq!(error.field, "placement.orientation_xyzw");
                    assert_eq!(error.rule_id, Some("DV-ATTR-003"));
                }
            }
        }
    }

    #[test]
    fn a_move_patch_has_argument_rules_and_a_fingerprint() {
        let update = |translation: [f64; 3]| UpdateElement {
            target_id: id(5),
            patch: ElementPatch {
                translation_mm: Some(translation),
                ..ElementPatch::default()
            },
        };
        assert!(update([1.0, 2.0, 3.0]).validate_arguments().is_ok());
        let code = |u: UpdateElement| u.validate_arguments().unwrap_err().code;
        assert_eq!(
            code(update([f64::INFINITY, 0.0, 0.0])),
            ErrorCode::NonFiniteNumber
        );
        assert_eq!(
            code(update([0.0, 1_000_000_001.0, 0.0])),
            ErrorCode::LimitExceeded
        );
        let base = update([1.0, 2.0, 3.0]).content_fingerprint().unwrap();
        assert_ne!(update([1.0, 2.0, 4.0]).content_fingerprint().unwrap(), base);
        // A patch without a move keeps the fingerprint it had before moves existed.
        let rename = UpdateElement {
            target_id: id(5),
            patch: ElementPatch {
                name: Some("A".into()),
                ..ElementPatch::default()
            },
        };
        assert_ne!(rename.content_fingerprint().unwrap(), base);
    }

    #[test]
    fn a_thickness_side_patch_has_its_own_fingerprint() {
        // A retry that differs only in the side must not replay as the same operation (A12.1).
        let update = |side: Option<PlateSide>| UpdateElement {
            target_id: id(5),
            patch: ElementPatch {
                thickness_side: side,
                ..ElementPatch::default()
            },
        };
        assert!(update(Some(PlateSide::Bottom)).validate_arguments().is_ok());
        // A patch that names no field changes nothing and stays inadmissible, so the three
        // sides are compared against each other.
        let top = update(Some(PlateSide::Top)).content_fingerprint().unwrap();
        let middle = update(Some(PlateSide::Middle))
            .content_fingerprint()
            .unwrap();
        let bottom = update(Some(PlateSide::Bottom))
            .content_fingerprint()
            .unwrap();
        assert_ne!(top, middle);
        assert_ne!(top, bottom);
        assert_ne!(middle, bottom);
        // The side rides behind the append-only tags, so older patches keep their digests.
        let rename = UpdateElement {
            target_id: id(5),
            patch: ElementPatch {
                name: Some("A".into()),
                ..ElementPatch::default()
            },
        };
        assert_ne!(rename.content_fingerprint().unwrap(), bottom);
    }

    #[test]
    fn the_delete_fingerprint_names_the_target() {
        let a = DeleteElement { target_id: id(5) };
        let b = DeleteElement { target_id: id(6) };
        assert_eq!(
            a.content_fingerprint().unwrap(),
            a.clone().content_fingerprint().unwrap()
        );
        assert_ne!(
            a.content_fingerprint().unwrap(),
            b.content_fingerprint().unwrap()
        );
    }

    #[test]
    fn assembly_commands_admit_labels_refuse_self_links_and_fingerprint_their_content() {
        let a = ElementId::parse("019a7bf0-0000-7000-8000-000000200001").unwrap();
        let b = ElementId::parse("019a7bf0-0000-7000-8000-000000200002").unwrap();
        let create = CreateAssembly {
            new_id: a,
            name: "Frame A".into(),
        };
        assert!(create.validate_arguments().is_ok());
        let renamed = CreateAssembly {
            name: "Frame B".into(),
            ..create.clone()
        };
        assert_ne!(
            create.content_fingerprint().unwrap(),
            renamed.content_fingerprint().unwrap()
        );
        for bad in ["", "tab\there"] {
            let c = CreateAssembly {
                name: bad.into(),
                ..create.clone()
            };
            assert_eq!(
                c.validate_arguments().unwrap_err().code,
                ErrorCode::InvalidArgument
            );
        }
        let attach = AttachToAssembly {
            part_id: a,
            assembly_id: b,
        };
        assert!(attach.validate_arguments().is_ok());
        let swapped = AttachToAssembly {
            part_id: b,
            assembly_id: a,
        };
        assert_ne!(
            attach.content_fingerprint().unwrap(),
            swapped.content_fingerprint().unwrap()
        );
        let own = AttachToAssembly {
            part_id: a,
            assembly_id: a,
        };
        assert_eq!(
            own.content_fingerprint().unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        let detach = DetachFromAssembly { part_id: a };
        assert_ne!(
            detach.content_fingerprint().unwrap(),
            DetachFromAssembly { part_id: b }
                .content_fingerprint()
                .unwrap()
        );
        let main = SetAssemblyMainPart {
            assembly_id: b,
            part_id: a,
        };
        assert_ne!(
            main.content_fingerprint().unwrap(),
            SetAssemblyMainPart {
                assembly_id: b,
                part_id: b,
            }
            .content_fingerprint()
            .unwrap()
        );
        let explode = ExplodeAssembly { assembly_id: a };
        assert_ne!(
            explode.content_fingerprint().unwrap(),
            ExplodeAssembly { assembly_id: b }
                .content_fingerprint()
                .unwrap()
        );
    }

    #[test]
    fn create_weld_admits_two_distinct_parts_and_a_bounded_size() {
        let id =
            |n: u32| ElementId::parse(&format!("019a7bf0-0000-7000-8000-0000003{n:05}")).unwrap();
        let weld = CreateWeld {
            new_id: id(1),
            name: "W1".into(),
            main_id: id(2),
            secondary_id: id(3),
            weld: WeldSpec {
                weld_type: WeldType::Fillet,
                size_mm: 6.0,
                location: WeldLocation::Workshop,
                side: WeldSide::Arrow,
            },
            geometry: None,
        };
        assert!(weld.validate_arguments().is_ok());
        let base = weld.content_fingerprint().unwrap();
        // Every authored property and the part order are part of the content.
        let variants = [
            CreateWeld {
                secondary_id: id(2),
                main_id: id(3),
                ..weld.clone()
            },
            CreateWeld {
                weld: WeldSpec {
                    location: WeldLocation::Site,
                    ..weld.weld
                },
                ..weld.clone()
            },
            CreateWeld {
                weld: WeldSpec {
                    side: WeldSide::Both,
                    ..weld.weld
                },
                ..weld.clone()
            },
            CreateWeld {
                weld: WeldSpec {
                    size_mm: 8.0,
                    ..weld.weld
                },
                ..weld.clone()
            },
        ];
        for v in &variants {
            assert_ne!(v.content_fingerprint().unwrap(), base);
        }
        let refused = |w: CreateWeld| w.validate_arguments().unwrap_err().code;
        assert_eq!(
            refused(CreateWeld {
                secondary_id: id(2),
                ..weld.clone()
            }),
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            refused(CreateWeld {
                new_id: id(2),
                ..weld.clone()
            }),
            ErrorCode::InvalidArgument
        );
        for size in [0.0, -1.0, 1000.5] {
            assert_eq!(
                refused(CreateWeld {
                    weld: WeldSpec {
                        size_mm: size,
                        ..weld.weld
                    },
                    ..weld.clone()
                }),
                ErrorCode::InvalidArgument
            );
        }
        assert_eq!(
            refused(CreateWeld {
                weld: WeldSpec {
                    size_mm: f64::NAN,
                    ..weld.weld
                },
                ..weld.clone()
            }),
            ErrorCode::NonFiniteNumber
        );
        assert_eq!(
            refused(CreateWeld {
                name: String::new(),
                ..weld.clone()
            }),
            ErrorCode::InvalidArgument
        );
        for token in ["fillet", "workshop", "site", "arrow", "other", "both"] {
            let back = WeldType::parse(token)
                .map(WeldType::as_token)
                .or(WeldLocation::parse(token).map(WeldLocation::as_token))
                .or(WeldSide::parse(token).map(WeldSide::as_token));
            assert_eq!(back, Some(token));
        }
        assert!(WeldType::parse("butt").is_none() && WeldSide::parse("Arrow").is_none());
    }

    #[test]
    fn weld_geometry_admits_a_bounded_path_and_symbols_are_exact_tokens() {
        let id =
            |n: u32| ElementId::parse(&format!("019a7bf0-0000-7000-8000-0000003{n:05}")).unwrap();
        let weld = CreateWeld {
            new_id: id(1),
            name: "W1".into(),
            main_id: id(2),
            secondary_id: id(3),
            weld: WeldSpec {
                weld_type: WeldType::Fillet,
                size_mm: 6.0,
                location: WeldLocation::Workshop,
                side: WeldSide::Arrow,
            },
            geometry: Some(WeldGeometry {
                path_mm: vec![[0.0; 3], [100.0, 0.0, 0.0]],
                symbol: WeldSymbol {
                    all_around: false,
                    contour: WeldContour::None,
                },
            }),
        };
        assert!(weld.validate_arguments().is_ok());
        let plain = CreateWeld {
            geometry: None,
            ..weld.clone()
        };
        // A06 requests keep their exact v1 fingerprint; geometry is a second domain.
        assert_ne!(
            weld.content_fingerprint().unwrap(),
            plain.content_fingerprint().unwrap()
        );
        let same = CreateWeld {
            geometry: Some(WeldGeometry {
                path_mm: vec![[0.0; 3], [100.0, 0.0, 0.0]],
                symbol: WeldSymbol {
                    all_around: false,
                    contour: WeldContour::None,
                },
            }),
            ..weld.clone()
        };
        assert_eq!(
            weld.content_fingerprint().unwrap(),
            same.content_fingerprint().unwrap()
        );
        let variants = [
            CreateWeld {
                geometry: Some(WeldGeometry {
                    path_mm: vec![[0.0; 3], [200.0, 0.0, 0.0]],
                    ..weld.geometry.clone().unwrap()
                }),
                ..weld.clone()
            },
            CreateWeld {
                geometry: Some(WeldGeometry {
                    symbol: WeldSymbol {
                        all_around: true,
                        contour: WeldContour::None,
                    },
                    ..weld.geometry.clone().unwrap()
                }),
                ..weld.clone()
            },
            CreateWeld {
                geometry: Some(WeldGeometry {
                    symbol: WeldSymbol {
                        all_around: false,
                        contour: WeldContour::Concave,
                    },
                    ..weld.geometry.clone().unwrap()
                }),
                ..weld.clone()
            },
        ];
        for v in &variants {
            assert_ne!(
                v.content_fingerprint().unwrap(),
                weld.content_fingerprint().unwrap()
            );
        }
        let code = |w: CreateWeld| w.validate_arguments().unwrap_err().code;
        let with_path = |path: Vec<[f64; 3]>| CreateWeld {
            geometry: Some(WeldGeometry {
                path_mm: path,
                symbol: WeldSymbol {
                    all_around: false,
                    contour: WeldContour::None,
                },
            }),
            ..weld.clone()
        };
        for (path, expected) in [
            (vec![], ErrorCode::InvalidArgument),
            (vec![[0.0; 3]], ErrorCode::InvalidArgument),
            (vec![[0.0; 3], [0.0, 0.0, 0.0]], ErrorCode::InvalidArgument),
            (
                vec![[0.0; 3], [0.0, 0.0005, 0.0]],
                ErrorCode::InvalidArgument,
            ),
            (
                vec![[0.0; 3], [f64::NAN, 0.0, 0.0]],
                ErrorCode::NonFiniteNumber,
            ),
            (
                vec![[0.0; 3], [f64::INFINITY, 0.0, 0.0]],
                ErrorCode::NonFiniteNumber,
            ),
            (
                vec![[2.0e10, 0.0, 0.0], [0.0; 3]],
                ErrorCode::InvalidArgument,
            ),
            (
                vec![[0.0; 3], [1.0e6, 0.0, 0.0], [2.0e6, 0.0, 0.0]],
                ErrorCode::LimitExceeded,
            ),
        ] {
            assert_eq!(code(with_path(path)), expected);
        }
        for token in ["none", "flush", "concave", "convex"] {
            assert_eq!(
                WeldContour::parse(token).map(WeldContour::as_token),
                Some(token)
            );
        }
        assert!(WeldContour::parse("round").is_none() && WeldContour::parse("None").is_none());
    }

    #[test]
    fn create_level_admits_a_label_and_a_bounded_finite_elevation() {
        let level = CreateLevel {
            new_id: id(1),
            name: "Level 1".into(),
            elevation_mm: 3000.0,
        };
        assert!(level.validate_arguments().is_ok());
        let code = |l: CreateLevel| l.validate_arguments().unwrap_err().code;
        for elevation in [-1_000_000_000.0, 0.0, 1_000_000_000.0] {
            let l = CreateLevel {
                elevation_mm: elevation,
                ..level.clone()
            };
            assert!(l.validate_arguments().is_ok(), "{elevation}");
        }
        for elevation in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let l = CreateLevel {
                elevation_mm: elevation,
                ..level.clone()
            };
            assert_eq!(code(l), ErrorCode::NonFiniteNumber);
        }
        for elevation in [1_000_000_000.5, -1e300] {
            let l = CreateLevel {
                elevation_mm: elevation,
                ..level.clone()
            };
            assert_eq!(code(l), ErrorCode::LimitExceeded);
        }
        for name in ["", "tab\there"] {
            let l = CreateLevel {
                name: name.into(),
                ..level.clone()
            };
            assert_eq!(code(l), ErrorCode::InvalidArgument);
        }
        let base = level.content_fingerprint().unwrap();
        for changed in [
            CreateLevel {
                new_id: id(2),
                ..level.clone()
            },
            CreateLevel {
                name: "Level 2".into(),
                ..level.clone()
            },
            CreateLevel {
                elevation_mm: 3000.5,
                ..level.clone()
            },
        ] {
            assert_ne!(changed.content_fingerprint().unwrap(), base);
        }
        assert_eq!(level.clone().content_fingerprint().unwrap(), base);
    }

    #[test]
    fn a_level_patch_counts_as_a_change_and_is_part_of_the_fingerprint() {
        let patch = |level| UpdateElement {
            target_id: id(5),
            patch: ElementPatch {
                level,
                ..ElementPatch::default()
            },
        };
        assert!(patch(Some(Some(id(9)))).validate_arguments().is_ok());
        assert!(patch(Some(None)).validate_arguments().is_ok());
        assert_eq!(
            patch(None).validate_arguments().unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        let set = patch(Some(Some(id(9)))).content_fingerprint().unwrap();
        let other = patch(Some(Some(id(10)))).content_fingerprint().unwrap();
        let clear = patch(Some(None)).content_fingerprint().unwrap();
        assert!(set != other && set != clear && other != clear);
    }

    fn bolt() -> CreateBolt {
        CreateBolt {
            new_id: id(1),
            name: "B1".into(),
            main_id: id(2),
            secondary_id: id(3),
            bolt: BoltSpec {
                diameter_mm: 16.0,
                count: 4,
                pitch_mm: 80.0,
                origin_mm: [0.0, 200.0, 5.0],
                direction_mm: [1.0, 0.0, 0.0],
            },
        }
    }

    #[test]
    fn create_bolt_admits_two_distinct_parts_and_bounded_properties() {
        let group = bolt();
        assert!(group.validate_arguments().is_ok());
        let base = group.content_fingerprint().unwrap();
        // Every authored property and the part order are part of the content.
        let variants = [
            CreateBolt {
                secondary_id: id(2),
                main_id: id(3),
                ..group.clone()
            },
            CreateBolt {
                bolt: BoltSpec {
                    diameter_mm: 20.0,
                    ..group.bolt
                },
                ..group.clone()
            },
            CreateBolt {
                bolt: BoltSpec {
                    count: group.bolt.count + 1,
                    ..group.bolt
                },
                ..group.clone()
            },
            CreateBolt {
                bolt: BoltSpec {
                    pitch_mm: 90.0,
                    ..group.bolt
                },
                ..group.clone()
            },
            CreateBolt {
                bolt: BoltSpec {
                    origin_mm: [101.0, 200.0, 5.0],
                    ..group.bolt
                },
                ..group.clone()
            },
            CreateBolt {
                bolt: BoltSpec {
                    direction_mm: [0.0, 1.0, 0.0],
                    ..group.bolt
                },
                ..group.clone()
            },
        ];
        for v in &variants {
            assert_ne!(v.content_fingerprint().unwrap(), base);
        }
        let refused = |b: CreateBolt| b.validate_arguments().unwrap_err();
        assert_eq!(
            refused(CreateBolt {
                secondary_id: id(2),
                ..group.clone()
            })
            .code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            refused(CreateBolt {
                new_id: id(2),
                ..group.clone()
            })
            .code,
            ErrorCode::InvalidArgument
        );
        for diameter in [0.0, -1.0, 1000.5] {
            assert_eq!(
                refused(CreateBolt {
                    bolt: BoltSpec {
                        diameter_mm: diameter,
                        ..group.bolt
                    },
                    ..group.clone()
                })
                .code,
                ErrorCode::InvalidArgument
            );
        }
        for (count, expected) in [
            (0, ErrorCode::InvalidArgument),
            (MAX_BOLT_GROUP_SIZE + 1, ErrorCode::LimitExceeded),
        ] {
            assert_eq!(
                refused(CreateBolt {
                    bolt: BoltSpec {
                        count,
                        ..group.bolt
                    },
                    ..group.clone()
                })
                .code,
                expected
            );
        }
        assert!(
            CreateBolt {
                bolt: BoltSpec {
                    count: MAX_BOLT_GROUP_SIZE,
                    ..group.bolt
                },
                ..group.clone()
            }
            .validate_arguments()
            .is_ok()
        );
        assert_eq!(
            refused(CreateBolt {
                bolt: BoltSpec {
                    pitch_mm: 0.0,
                    ..group.bolt
                },
                ..group.clone()
            })
            .code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            refused(CreateBolt {
                bolt: BoltSpec {
                    pitch_mm: DIMENSION_CAP_MM + 1.0,
                    ..group.bolt
                },
                ..group.clone()
            })
            .code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            refused(CreateBolt {
                bolt: BoltSpec {
                    origin_mm: [1_000_000_001.0, 0.0, 0.0],
                    ..group.bolt
                },
                ..group.clone()
            })
            .code,
            ErrorCode::LimitExceeded
        );
        for field in [
            BoltSpec {
                diameter_mm: f64::NAN,
                ..group.bolt
            },
            BoltSpec {
                origin_mm: [0.0, f64::INFINITY, 0.0],
                ..group.bolt
            },
        ] {
            let e = refused(CreateBolt {
                bolt: field,
                ..group.clone()
            });
            assert_eq!(e.code, ErrorCode::NonFiniteNumber, "{}", e.field);
        }
        let e = refused(CreateBolt {
            bolt: BoltSpec {
                direction_mm: [2.0, 0.0, 0.0],
                ..group.bolt
            },
            ..group.clone()
        });
        assert_eq!(e.code, ErrorCode::InvalidArgument);
        assert_eq!(e.field, "direction_mm");
        assert_eq!(
            refused(CreateBolt {
                name: String::new(),
                ..group.clone()
            })
            .code,
            ErrorCode::InvalidArgument
        );
        // Signed zero and an equivalent unit direction fingerprint the same.
        let mut zero = bolt();
        zero.bolt.origin_mm[0] = -0.0;
        assert_eq!(zero.content_fingerprint().unwrap(), base);
        assert_eq!(group.clone().content_fingerprint().unwrap(), base);
    }
}
