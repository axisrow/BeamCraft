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
    MAX_BOLT_GROUP_SIZE, MAX_LABEL_BYTES, MIN_CONTOUR_POINTS, MIN_LABEL_BYTES,
    ORIENTATION_NORM_TOLERANCE, ROLL_CAP_RAD, WELD_SIZE_CAP_MM,
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

/// F04 section 7 `CreatePlate`. `material_digest` pins an existing immutable material definition;
/// whether the digest resolves (`NOT_FOUND`) is the engine's decision, not an argument check.
#[derive(Debug, Clone, PartialEq)]
pub struct CreatePlate {
    /// The new element's full identity; creation also checks that it is unused.
    pub new_id: ElementId,
    /// Label, 1 to 256 UTF-8 bytes without control characters. Not an identity.
    pub name: String,
    /// Element-local contour, 3 to 1024 points, closure implicit.
    pub contour_points_mm: Vec<[f64; 3]>,
    /// Extrusion thickness in millimetres.
    pub thickness_mm: f64,
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
    /// The two element-local axis points `p1` and `p2`.
    pub axis_points_mm: [[f64; 3]; 2],
    /// Pinned profile definition.
    pub profile_digest: ContentDigest,
    /// Rigid placement.
    pub placement: Placement,
    /// Explicit roll about the member axis; `None` is zero.
    pub roll_rad: Option<f64>,
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
        bytes.extend_from_slice(b"BeamCraft/P03/CreateStraightMember/v3\0");
        bytes.extend_from_slice(self.new_id.as_bytes());
        bytes.push(match self.kind {
            MemberKind::Beam => 0,
            MemberKind::Column => 1,
        });
        let name = self.name.as_bytes();
        bytes.extend_from_slice(&(name.len() as u64).to_le_bytes());
        bytes.extend_from_slice(name);
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
        bytes.extend_from_slice(b"BeamCraft/P04/CreatePlate/v5\0");
        bytes.extend_from_slice(self.new_id.as_bytes());
        let name = self.name.as_bytes();
        bytes.extend_from_slice(&(name.len() as u64).to_le_bytes());
        bytes.extend_from_slice(name);
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

/// A typed patch of supported authored fields (F04 section 7 `UpdateElement`). A field that is
/// `None` is left unchanged. `material` distinguishes no change (`None`), clearing the reference
/// (`Some(None)`) and pinning a definition (`Some(Some(digest))`). There is no raw USD setter.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ElementPatch {
    /// New label.
    pub name: Option<String>,
    /// New material reference, or its explicit removal.
    pub material: Option<Option<ContentDigest>>,
    /// New plate thickness in millimetres (a plate only).
    pub thickness_mm: Option<f64>,
    /// New placement translation in millimetres, absolute; the rotation is kept (a move).
    pub translation_mm: Option<[f64; 3]>,
    /// New placement rotation, absolute. Replaces the stored quaternion (the identity resets a
    /// member to the unrotated pose; a plate refuses a non-identity rotation, as at creation);
    /// the translation is kept. The quaternion follows the `Placement` rules and is validated.
    pub orientation_xyzw: Option<[f64; 4]>,
    /// D2: new level assignment of a plate, beam or column, or its explicit removal
    /// (`Some(None)`). Whether the level exists is the engine's decision.
    pub level: Option<Option<ElementId>>,
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
        if patch.name.is_none()
            && patch.material.is_none()
            && patch.thickness_mm.is_none()
            && patch.translation_mm.is_none()
            && patch.orientation_xyzw.is_none()
            && patch.level.is_none()
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
        Ok(())
    }

    /// SHA-256 of a versioned, length-prefixed encoding of the validated request.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        self.validate_arguments()?;
        let mut bytes = Vec::with_capacity(128);
        bytes.extend_from_slice(b"BeamCraft/P04/UpdateElement/v1\0");
        bytes.extend_from_slice(self.target_id.as_bytes());
        match &self.patch.name {
            Some(name) => {
                bytes.push(1);
                bytes.extend_from_slice(&(name.len() as u64).to_le_bytes());
                bytes.extend_from_slice(name.as_bytes());
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
            ));
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
            contour_points_mm: vec![
                [0.0, 0.0, 0.0],
                [1000.0, 0.0, 0.0],
                [1000.0, 500.0, 0.0],
                [0.0, 500.0, 0.0],
            ],
            thickness_mm: 10.0,
            placement: Placement::IDENTITY,
            material_digest: None,
            holes_mm: Vec::new(),
        }
    }

    fn code(plate: &CreatePlate) -> ErrorCode {
        plate.validate_arguments().unwrap_err().code
    }

    fn hole() -> Vec<[f64; 2]> {
        vec![
            [100.0, 100.0],
            [100.0, 200.0],
            [200.0, 200.0],
            [200.0, 100.0],
        ]
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
            axis_points_mm: [[0.0; 3], [6000.0, 0.0, 0.0]],
            profile_digest: digest(b"profile"),
            placement: Placement::IDENTITY,
            roll_rad: None,
            material_digest: None,
        }
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
