// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The member family behind the discipline hook (#795): a beam or a column (P03, P12, P13, P14)
//! and the edits of its path, roll and profile. Each body is the arm it replaced in the
//! platform's `edit.rs` and `validate.rs`, moved unchanged, so the bytes written and the
//! refusals returned are the same. The facade constructors in [`super`] build the platform
//! `Shape` and `Change` from these hooks.

use super::read::cut_values;
use super::write::{CUT_FIELDS, MEMBER_PLACEMENT};
use crate::model::command::{CutRecord, SectionPlacement};
use beamcraft_layer_engine::Diagnostic;
use beamcraft_layer_engine::authored::seam::author::{attr, attribute, points, relationship};
use beamcraft_layer_engine::authored::seam::read::{cap, finite};
use beamcraft_layer_engine::authored::{
    AuthorCx, ChangeAuthorCx, ChangeHook, ChangeValidateCx, Footprint, Group, MaterialOpinion,
    Opinion, ShapeHook, ValidateCx, invalid,
};
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::limits::MAX_MEMBER_PATH_POINTS;
use beamcraft_model::prim::ElementCategory;
use openusd::sdf;
use std::any::Any;

/// Closed member vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberKind {
    /// Straight beam.
    Beam,
    /// Straight column.
    Column,
}

/// The authored shape of a member (P03 straight, P12 polybeam), with a required pinned profile.
#[derive(Debug, Clone, PartialEq)]
pub struct MemberShape {
    /// Three authored axis points describe a circular arc (P13).
    pub curved: bool,
    /// Explicit beam or column kind.
    pub kind: MemberKind,
    /// Ordered element-local path points: exactly two for a straight member, at least three for
    /// a polybeam.
    pub axis_mm: Vec<[f64; 3]>,
    /// Immutable profile content digest.
    pub profile: ContentDigest,
    /// Missing roll has no authored default; engineering default is zero. A polybeam has no
    /// authored roll.
    pub roll_rad: Option<f64>,
    /// Position of the section about the axis (P14); `None` is the centred default and stores
    /// no attributes.
    pub placement: Option<SectionPlacement>,
    /// The owned `plane_cut` operation records (A03, amendment A-2) in ascending ordinal order.
    /// Empty means an uncut member.
    pub cuts: Vec<CutRecord>,
}

impl ShapeHook for MemberShape {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn same_as(&self, other: &dyn ShapeHook) -> bool {
        other.as_any().downcast_ref::<Self>() == Some(self)
    }

    fn kind_token(&self) -> &'static str {
        match self.kind {
            MemberKind::Beam => "beam",
            MemberKind::Column => "column",
        }
    }

    fn category(&self) -> ElementCategory {
        ElementCategory::Part
    }

    fn author_cuts(&self, data: &mut sdf::Data, path: &str) -> Result<(), Diagnostic> {
        // A fresh uncut element authors no cut attributes; the empty arrays belong to the
        // change path, where they mask weaker opinions (A03).
        if self.cuts.is_empty() {
            return Ok(());
        }
        super::write::author_cuts(data, path, &self.cuts, None)
    }

    fn classed(&self) -> bool {
        true
    }

    fn author(&self, cx: &mut AuthorCx<'_>) -> Result<(), Diagnostic> {
        let (data, p) = (&mut *cx.data, cx.path);
        attr(
            data,
            p,
            "bim:axisPointsMm",
            "double3[]",
            points(&self.axis_mm),
        )?;
        if self.curved {
            attr(
                data,
                p,
                "steel:member:pathType",
                "token",
                sdf::Value::token("arc"),
            )?;
        }
        relationship(
            data,
            p,
            "bim:profile",
            MaterialOpinion::Target(self.profile),
        )?;
        if let Some(n) = self.roll_rad {
            attr(
                data,
                p,
                "steel:member:rollRad",
                "double",
                sdf::Value::Double(n),
            )?;
        }
        if let Some(placement) = &self.placement {
            attr(
                data,
                p,
                "steel:member:planePosition",
                "token",
                sdf::Value::token(placement.plane.as_str()),
            )?;
            attr(
                data,
                p,
                "steel:member:planeOffsetMm",
                "double",
                sdf::Value::Double(placement.plane_offset_mm),
            )?;
            attr(
                data,
                p,
                "steel:member:depthPosition",
                "token",
                sdf::Value::token(placement.depth.as_str()),
            )?;
            attr(
                data,
                p,
                "steel:member:depthOffsetMm",
                "double",
                sdf::Value::Double(placement.depth_offset_mm),
            )?;
            attr(
                data,
                p,
                "steel:member:startOffsetMm",
                "double",
                sdf::Value::Double(placement.start_offset_mm),
            )?;
            attr(
                data,
                p,
                "steel:member:endOffsetMm",
                "double",
                sdf::Value::Double(placement.end_offset_mm),
            )?;
        }
        Ok(())
    }

    fn attributes(&self) -> Vec<(&'static str, &'static str)> {
        let mut out = Vec::new();
        if self.curved {
            out.push(("steel:member:pathType", "token"));
        }
        out.push(("bim:axisPointsMm", "double3[]"));
        out.push(("steel:member:rollRad", "double"));
        if self.placement.is_some() {
            out.extend_from_slice(MEMBER_PLACEMENT);
        }
        out.push(("bim:profile", "rel"));
        if !self.cuts.is_empty() {
            out.extend_from_slice(CUT_FIELDS);
        }
        out
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) -> Result<(), Diagnostic> {
        let axis_mm = &self.axis_mm;
        if axis_mm.len() < 2 || axis_mm.len() > MAX_MEMBER_PATH_POINTS {
            return Err(invalid());
        }
        if self.curved && (axis_mm.len() != 3 || self.roll_rad.is_some()) {
            return Err(invalid());
        }
        cap(axis_mm.iter().flatten().copied(), 1e9)?;
        if let Some(n) = self.roll_rad {
            cap([n], std::f64::consts::TAU)?;
        }
        if let Some(p) = &self.placement {
            cap(
                [
                    p.plane_offset_mm,
                    p.depth_offset_mm,
                    p.start_offset_mm,
                    p.end_offset_mm,
                ],
                1e6,
            )?;
        }
        cut_values(&self.cuts)?;
        cx.reads.insert(Footprint {
            identity: self.profile.to_string(),
            group: Group::Relationship,
        });
        Ok(())
    }

    fn check_finite(&self) -> Result<(), Diagnostic> {
        finite(
            self.axis_mm
                .iter()
                .flatten()
                .copied()
                .chain(self.roll_rad.iter().copied())
                .chain(self.placement.iter().flat_map(|p| {
                    [
                        p.plane_offset_mm,
                        p.depth_offset_mm,
                        p.start_offset_mm,
                        p.end_offset_mm,
                    ]
                }))
                .chain(
                    self.cuts
                        .iter()
                        .flat_map(|c| c.point_mm.into_iter().chain(c.normal)),
                ),
        )
    }
}

/// The groups of an edit of a shape input: read the lifecycle and the inputs, write the inputs
/// and the derived result.
const SHAPE_INPUT_GROUPS: (&[Group], &[Group]) = (
    &[Group::Lifecycle, Group::ShapeInputs],
    &[Group::ShapeInputs, Group::Derived],
);

/// Replace the member's path points as a whole (P03 two, P12 three or more).
#[derive(Debug, Clone, PartialEq)]
pub struct SetAxis {
    /// Full target UUIDv7.
    pub id: String,
    /// Ordered element-local path points.
    pub points_mm: Vec<[f64; 3]>,
}

/// Change the optional roll opinion in the selected authored layer.
#[derive(Debug, Clone, PartialEq)]
pub struct SetRoll {
    /// Full target UUIDv7.
    pub id: String,
    /// Exact authored opinion.
    pub opinion: Opinion<f64>,
}

/// Replace the required immutable member profile.
#[derive(Debug, Clone, PartialEq)]
pub struct SetProfile {
    /// Full target UUIDv7.
    pub id: String,
    /// Verified profile content digest.
    pub digest: ContentDigest,
}

macro_rules! member_change_common {
    ($key:literal) => {
        fn as_any(&self) -> &dyn Any {
            self
        }

        fn same_as(&self, other: &dyn ChangeHook) -> bool {
            other.as_any().downcast_ref::<Self>() == Some(self)
        }

        fn target(&self) -> &str {
            &self.id
        }

        fn group_key(&self) -> &'static str {
            $key
        }

        fn targets_kind(&self, kind: &str) -> bool {
            matches!(kind, "beam" | "column")
        }

        fn groups(&self) -> (&'static [Group], &'static [Group]) {
            SHAPE_INPUT_GROUPS
        }

        fn invalidates_derived(&self) -> bool {
            true
        }
    };
}

impl ChangeHook for SetAxis {
    member_change_common!("bim:axisPointsMm");

    fn validate(&self, _cx: &mut ChangeValidateCx<'_>) -> Result<(), Diagnostic> {
        cap(self.points_mm.iter().flatten().copied(), 1e9)
    }

    fn check_finite(&self) -> Result<(), Diagnostic> {
        finite(self.points_mm.iter().flatten().copied())
    }

    fn author(&self, cx: &mut ChangeAuthorCx<'_>) -> Result<(), Diagnostic> {
        attr(
            cx.data,
            cx.path,
            "bim:axisPointsMm",
            "double3[]",
            points(&self.points_mm),
        )
    }

    fn attributes(&self) -> Vec<(&'static str, &'static str)> {
        vec![("bim:axisPointsMm", "double3[]")]
    }
}

impl ChangeHook for SetRoll {
    member_change_common!("steel:member:rollRad");

    fn validate(&self, _cx: &mut ChangeValidateCx<'_>) -> Result<(), Diagnostic> {
        match self.opinion {
            Opinion::Value(n) => cap([n], std::f64::consts::TAU),
            _ => Ok(()),
        }
    }

    fn check_finite(&self) -> Result<(), Diagnostic> {
        match self.opinion {
            Opinion::Value(n) => finite([n]),
            _ => Ok(()),
        }
    }

    fn author(&self, cx: &mut ChangeAuthorCx<'_>) -> Result<(), Diagnostic> {
        let opinion = match &self.opinion {
            Opinion::Value(n) => Opinion::Value(sdf::Value::Double(*n)),
            Opinion::Absent => Opinion::Absent,
            Opinion::Block => Opinion::Block,
            Opinion::Remove => Opinion::Remove,
        };
        attribute(
            cx.data,
            cx.path,
            "steel:member:rollRad",
            "double",
            opinion,
            true,
        )
    }

    fn attributes(&self) -> Vec<(&'static str, &'static str)> {
        vec![("steel:member:rollRad", "double")]
    }
}

impl ChangeHook for SetProfile {
    member_change_common!("bim:profile");

    fn validate(&self, cx: &mut ChangeValidateCx<'_>) -> Result<(), Diagnostic> {
        cx.reads.insert(Footprint {
            identity: self.digest.to_string(),
            group: Group::Relationship,
        });
        Ok(())
    }

    fn check_finite(&self) -> Result<(), Diagnostic> {
        Ok(())
    }

    fn author(&self, cx: &mut ChangeAuthorCx<'_>) -> Result<(), Diagnostic> {
        relationship(
            cx.data,
            cx.path,
            "bim:profile",
            MaterialOpinion::Target(self.digest),
        )
    }

    fn attributes(&self) -> Vec<(&'static str, &'static str)> {
        vec![("bim:profile", "rel")]
    }
}
