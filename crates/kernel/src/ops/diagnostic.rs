// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Typed failures of the pilot cut operations.
//!
//! The `GEO_` tokens that already exist in F06 section 3 keep their meaning and F04 mapping.
//! Two tokens are **proposed** here because F06 0.1.0 has no operation vocabulary:
//! `GEO_OP_UNSUPPORTED` (a valid request outside the pilot scope) and `GEO_OP_NO_EFFECT` (a
//! request that would leave the body unchanged or empty). A contract amendment must adopt
//! them before an engine may persist them. No failure carries geometry, a coordinate value or
//! user text, and a failed call never returns a partial result.

use crate::diagnostic::Stage;
use crate::profile::diagnostic::{ProfileCode, ProfileDiagnostic};
use crate::verdict::{MeshCode, MeshError};
use beamcraft_model::error::ErrorCode;

/// Stable geometry code of a failed operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpCode {
    /// NaN or infinity in a submitted value.
    NonFinite,
    /// A zero or negative quantity (a zero plane normal).
    NotPositive,
    /// A value, count or budget above its cap.
    AboveLimit,
    /// A derived vertex, triangle or byte count above the F06 section 6 cap.
    BufferLimit,
    /// A derived face or shell count above the F06 section 6 cap.
    TopologyLimit,
    /// An invalid cutter contour under F04 section 3.
    ContourInvalid,
    /// A valid contour configuration outside the supported scope (nested island, several sections).
    UnsupportedContour,
    /// The body is open, non-manifold or inconsistently oriented.
    MeshNotClosed,
    /// The body is closed but wound inward.
    MeshInward,
    /// The body has no certificate of a valid, non-degenerate, self-intersection-free shell.
    MeshNotCertified,
    /// A body or face-table input disagrees with its own structure.
    BlobInvalid,
    /// A numeric certificate or exact predicate could not decide; nothing was guessed.
    NumericUncertain,
    /// The operation's own result failed its self-checks.
    SelfCheckFailed,
    /// Cancellation observed before the result was sealed.
    Cancelled,
    /// Proposed `GEO_OP_UNSUPPORTED`: a valid request outside the pilot scope.
    OpUnsupported,
    /// Proposed `GEO_OP_NO_EFFECT`: the request would not change, or would empty, the body.
    OpNoEffect,
}

impl OpCode {
    /// Stable wire token (F06 section 3 plus the two proposed `GEO_OP_` tokens).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NonFinite => "GEO_NON_FINITE",
            Self::NotPositive => "GEO_NOT_POSITIVE",
            Self::AboveLimit => "GEO_ABOVE_LIMIT",
            Self::BufferLimit => "GEO_BUFFER_LIMIT",
            Self::TopologyLimit => "GEO_TOPOLOGY_LIMIT",
            Self::ContourInvalid => "GEO_CONTOUR_INVALID",
            Self::UnsupportedContour => "GEO_UNSUPPORTED_CONTOUR",
            Self::MeshNotClosed => "GEO_MESH_NOT_CLOSED",
            Self::MeshInward => "GEO_MESH_INWARD",
            Self::MeshNotCertified => "GEO_MESH_NOT_CERTIFIED",
            Self::BlobInvalid => "GEO_BLOB_INVALID",
            Self::NumericUncertain => "GEO_NUMERIC_UNCERTAIN",
            Self::SelfCheckFailed => "GEO_SELF_CHECK_FAILED",
            Self::Cancelled => "GEO_CANCELLED",
            Self::OpUnsupported => "GEO_OP_UNSUPPORTED",
            Self::OpNoEffect => "GEO_OP_NO_EFFECT",
        }
    }

    /// F04 mapping of the existing codes; the proposed tokens map conservatively.
    pub const fn f04_code(self) -> ErrorCode {
        match self {
            Self::NonFinite => ErrorCode::NonFiniteNumber,
            Self::NotPositive | Self::ContourInvalid | Self::OpNoEffect => {
                ErrorCode::InvalidArgument
            }
            Self::AboveLimit | Self::BufferLimit | Self::TopologyLimit => ErrorCode::LimitExceeded,
            Self::BlobInvalid => ErrorCode::IntegrityError,
            Self::UnsupportedContour
            | Self::MeshNotClosed
            | Self::MeshInward
            | Self::MeshNotCertified
            | Self::NumericUncertain
            | Self::SelfCheckFailed
            | Self::OpUnsupported => ErrorCode::UnsupportedGeometry,
            Self::Cancelled => ErrorCode::Cancelled,
        }
    }

    /// F06 overall status string of a call that failed with this code.
    pub const fn status(self) -> &'static str {
        match self {
            Self::AboveLimit | Self::BufferLimit | Self::TopologyLimit => "limit_exceeded",
            Self::Cancelled => "cancelled",
            Self::NonFinite
            | Self::NotPositive
            | Self::ContourInvalid
            | Self::BlobInvalid
            | Self::OpNoEffect => "invalid_input",
            Self::UnsupportedContour
            | Self::MeshNotClosed
            | Self::MeshInward
            | Self::MeshNotCertified
            | Self::NumericUncertain
            | Self::SelfCheckFailed
            | Self::OpUnsupported => "unsupported",
        }
    }
}

impl From<MeshCode> for OpCode {
    fn from(code: MeshCode) -> Self {
        match code {
            MeshCode::NonFinite => Self::NonFinite,
            MeshCode::AboveLimit => Self::AboveLimit,
            MeshCode::BufferLimit => Self::BufferLimit,
            MeshCode::TopologyLimit => Self::TopologyLimit,
            MeshCode::BlobInvalid => Self::BlobInvalid,
            MeshCode::NotClosed => Self::MeshNotClosed,
            MeshCode::Inward => Self::MeshInward,
            MeshCode::NotCertified => Self::MeshNotCertified,
            MeshCode::NumericUncertain => Self::NumericUncertain,
            MeshCode::Cancelled => Self::Cancelled,
        }
    }
}

impl From<ProfileCode> for OpCode {
    fn from(code: ProfileCode) -> Self {
        match code {
            ProfileCode::NonFinite => Self::NonFinite,
            ProfileCode::NotPositive | ProfileCode::BelowMinimum => Self::NotPositive,
            ProfileCode::AboveLimit => Self::AboveLimit,
            ProfileCode::ContourInvalid => Self::ContourInvalid,
            ProfileCode::UnsupportedContour => Self::UnsupportedContour,
            ProfileCode::BufferLimit => Self::BufferLimit,
            ProfileCode::TopologyLimit => Self::TopologyLimit,
            ProfileCode::NumericUncertain => Self::NumericUncertain,
            ProfileCode::SelfCheckFailed => Self::SelfCheckFailed,
            ProfileCode::Cancelled => Self::Cancelled,
        }
    }
}

/// Fixed schema token naming the failing input or part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpField {
    /// The input body.
    Body,
    /// The face table or normals submitted with the body.
    FaceTable,
    /// The cutting plane.
    Plane,
    /// The through-hole cutter.
    Cutter,
    /// The planar section of a plane cut.
    Section,
    /// The re-triangulated cap of a hole.
    Cap,
    /// The operation budget.
    Limits,
    /// The candidate result mesh.
    Mesh,
    /// The result volume.
    Volume,
    /// Host cancellation request.
    Cancellation,
}

impl OpField {
    /// Fixed ASCII token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Body => "body",
            Self::FaceTable => "face_table",
            Self::Plane => "plane",
            Self::Cutter => "cutter",
            Self::Section => "section",
            Self::Cap => "cap",
            Self::Limits => "limits",
            Self::Mesh => "mesh",
            Self::Volume => "volume_mm3",
            Self::Cancellation => "cancellation",
        }
    }
}

/// Fixed classification of why a request was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpDefect {
    /// No finer classification applies.
    None,
    /// The body has more than one shell.
    BodyShells,
    /// The hole pilot needs a straight prism with two horizontal caps.
    BodyNotPrism,
    /// The member hole meets a curved or sloped wall face inside the cutter range (G14.2).
    BodyWallUnsupported,
    /// The caps of the prism are not one face each, or a cap has no outline.
    BodyCapFaces,
    /// The cap outlines of the body are not one outer loop plus holes.
    BodyLoops,
    /// A face id or normal of the face table is invalid.
    FaceTableEntry,
    /// The face table and the mesh disagree in size or duplicate an id.
    FaceTableSize,
    /// The ordinal is above its cap (999).
    Ordinal,
    /// The body mesh is not in element-local space.
    BodySpace,
    /// The whole body lies strictly on the kept side: nothing would be removed.
    PlaneOutside,
    /// The whole body lies strictly on the removed side: nothing would be kept.
    PlaneEmpty,
    /// The plane only touches the body (vertex, edge or face contact) without crossing it.
    PlaneTangent,
    /// A face of the body lies exactly in the plane.
    PlaneCoplanarFace,
    /// The section touches itself at a vertex or edge, so it is not a set of simple loops.
    SectionPinched,
    /// The section has more than one outer loop.
    SectionMultiple,
    /// The section is not admissible as a cap polygon (see the attached diagnostic).
    SectionNotAdmissible,
    /// The kept surface does not close on the plane.
    SectionOpen,
    /// The cutter does not reach beyond both faces of the plate.
    CutterNotThrough,
    /// The cutter ends exactly on a face of the plate.
    CutterTangent,
    /// The cutter protrudes past a face by less than the geometric floor.
    CutterMargin,
    /// The cutter range is empty or reversed.
    CutterRange,
    /// The candidate has a collapsed, zero-area or sub-floor feature.
    ResultDegenerate,
    /// The candidate meets itself.
    ResultOverlapping,
    /// The candidate is not a certified closed outward shell.
    ResultNotCertified,
    /// The certification of the candidate could not decide.
    ResultUncertain,
    /// The candidate has more than one shell.
    ResultShells,
    /// The candidate's Euler characteristic or genus is not the expected one.
    ResultTopology,
    /// The candidate's volume disagrees with the analytic volume.
    ResultVolume,
    /// The output would exceed the triangle budget.
    BudgetTriangles,
    /// A cap or section has more loops than the loop budget.
    BudgetLoops,
    /// A cap profile has more points than the point budget.
    BudgetCapPoints,
    /// The exact-predicate budget was used up.
    BudgetExact,
    /// A value left the range the exact arithmetic supports.
    NumericRange,
}

impl OpDefect {
    /// Fixed ASCII token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::BodyShells => "body.shells",
            Self::BodyNotPrism => "body.not_prism",
            Self::BodyWallUnsupported => "body.wall_unsupported",
            Self::BodyCapFaces => "body.cap_faces",
            Self::BodyLoops => "body.loops",
            Self::FaceTableEntry => "face_table.entry",
            Self::FaceTableSize => "face_table.size",
            Self::Ordinal => "ordinal",
            Self::BodySpace => "body.space",
            Self::PlaneOutside => "plane.outside",
            Self::PlaneEmpty => "plane.empty",
            Self::PlaneTangent => "plane.tangent",
            Self::PlaneCoplanarFace => "plane.coplanar_face",
            Self::SectionPinched => "section.pinched",
            Self::SectionMultiple => "section.multiple",
            Self::SectionNotAdmissible => "section.not_admissible",
            Self::SectionOpen => "section.open",
            Self::CutterNotThrough => "cutter.not_through",
            Self::CutterTangent => "cutter.tangent",
            Self::CutterMargin => "cutter.margin",
            Self::CutterRange => "cutter.range",
            Self::ResultDegenerate => "result.degenerate",
            Self::ResultOverlapping => "result.overlapping",
            Self::ResultNotCertified => "result.not_certified",
            Self::ResultUncertain => "result.uncertain",
            Self::ResultShells => "result.shells",
            Self::ResultTopology => "result.topology",
            Self::ResultVolume => "result.volume",
            Self::BudgetTriangles => "budget.triangles",
            Self::BudgetLoops => "budget.loops",
            Self::BudgetCapPoints => "budget.cap_points",
            Self::BudgetExact => "budget.exact_predicates",
            Self::NumericRange => "numeric.range",
        }
    }
}

/// A refused or failed operation: no geometry, value or partial result is carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpFailure {
    /// Stable geometry code; the F04 mapping is [`OpCode::f04_code`].
    pub code: OpCode,
    /// The failing input or part.
    pub field: OpField,
    /// Fixed refinement of the reason.
    pub defect: OpDefect,
    /// Evaluation stage that observed the failure.
    pub stage: Stage,
    /// The first G05 admission diagnostic when a cap or section profile was refused. For a
    /// hole, its `cavity` index equals the number of existing holes for the new cutter.
    pub profile: Option<ProfileDiagnostic>,
}

impl OpFailure {
    /// Shared with the steel crate.
    pub const fn new(code: OpCode, field: OpField, defect: OpDefect, stage: Stage) -> Self {
        Self {
            code,
            field,
            defect,
            stage,
            profile: None,
        }
    }

    pub(crate) const fn cancelled(stage: Stage) -> Self {
        Self::new(
            OpCode::Cancelled,
            OpField::Cancellation,
            OpDefect::None,
            stage,
        )
    }

    /// Shared with the steel crate.
    pub const fn numeric(field: OpField, defect: OpDefect, stage: Stage) -> Self {
        Self::new(OpCode::NumericUncertain, field, defect, stage)
    }

    /// Shared with the steel crate.
    pub const fn unsupported(field: OpField, defect: OpDefect, stage: Stage) -> Self {
        Self::new(OpCode::OpUnsupported, field, defect, stage)
    }

    /// Shared with the steel crate.
    pub const fn self_check(defect: OpDefect, stage: Stage) -> Self {
        Self::new(OpCode::SelfCheckFailed, OpField::Mesh, defect, stage)
    }

    /// Shared with the steel crate.
    pub fn from_mesh(error: MeshError, field: OpField) -> Self {
        Self::new(OpCode::from(error.code), field, OpDefect::None, error.stage)
    }

    /// F06 overall status string.
    pub const fn status(&self) -> &'static str {
        self.code.status()
    }
}

impl core::fmt::Display for OpFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("cut operation failed")
    }
}

impl core::error::Error for OpFailure {}
