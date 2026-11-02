// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The plate family behind the discipline hook (#795): an extruded plate (P01, A02, A04, A10,
//! A12) and a lofted plate (A11). Each body is the arm it replaced in the platform's `edit.rs`
//! and `validate.rs`, moved unchanged, so the bytes written and the refusals returned are the
//! same. The facade constructors in [`super`] build the platform `Shape` from these hooks.

use super::read::{MAX_HOLE_POINTS, MAX_HOLES, cut_values};
use super::write::{CUT_FIELDS, author_bends, author_corners, author_cuts};
use crate::model::command::{
    BendRecord, BendSpec, CornerTreatment, CutRecord, HoleRecord, MAX_BENDS, PlateCorner, PlateSide,
};
use beamcraft_layer_engine::authored::seam::author::{attr, author_record_history, points};
use beamcraft_layer_engine::authored::seam::read::{cap, count_points, finite};
use beamcraft_layer_engine::authored::{AuthorCx, ShapeHook, ValidateCx, cut_history, invalid};
use beamcraft_layer_engine::{Diagnostic, limit};
use beamcraft_model::prim::ElementCategory;
use openusd::sdf;
use std::any::Any;

/// The authored shape of an extruded plate: the element-local contour preserved exactly, the
/// thickness and the operation records. The wrapper does not evaluate a solid.
#[derive(Debug, Clone, PartialEq)]
pub struct PlateShape {
    /// Ordered explicit contour, at most 1024 points and subject to tighter stage budgets.
    pub contour_mm: Vec<[f64; 3]>,
    /// Explicit extrusion thickness in millimetres.
    pub thickness_mm: f64,
    /// Where the thickness is laid off from the contour plane (A12.1); absence is the `top`
    /// default and stores no attribute.
    pub thickness_side: Option<PlateSide>,
    /// Corner treatments of authored contour vertices (A12.2); empty means every corner sharp
    /// and stores no attribute. The contour keeps its sharp points.
    pub corners: Vec<PlateCorner>,
    /// The bend operation records (A10); empty means a flat plate. The contour of a bent plate
    /// is its developed mid-surface outline.
    pub bends: Vec<BendRecord>,
    /// Through-hole polygons in plate-local XY (`z` is zero), each at least three points,
    /// stored as one flat point array plus the vertex counts. Empty means a solid plate.
    pub holes: Vec<Vec<[f64; 3]>>,
    /// Through-hole operation records (A04) authored with the element. An ordinary create
    /// carries none (F04 section 4); a copy authors the source's records with fresh operation
    /// ids and the same ordinals.
    pub hole_records: Vec<HoleRecord>,
    /// The owned `plane_cut` operation records (A03, amendment A-2) in ascending ordinal order,
    /// stored as five parallel attributes. Empty means an uncut plate.
    pub cuts: Vec<CutRecord>,
}

/// The authored shape of a lofted plate (A11): a plate between two face contours that need not
/// be coplanar, stored as two element-local point arrays of the same vertex count. The sheet
/// thickness is the distance the loft spans; the plate carries no thickness attribute.
#[derive(Debug, Clone, PartialEq)]
pub struct LoftedPlateShape {
    /// Ordered start face contour.
    pub start_mm: Vec<[f64; 3]>,
    /// Ordered end face contour, the same vertex count as the start contour.
    pub end_mm: Vec<[f64; 3]>,
}

impl ShapeHook for PlateShape {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn same_as(&self, other: &dyn ShapeHook) -> bool {
        other.as_any().downcast_ref::<Self>() == Some(self)
    }

    fn kind_token(&self) -> &'static str {
        "plate"
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
        author_cuts(data, path, &self.cuts, None)
    }

    fn classed(&self) -> bool {
        true
    }

    fn charge(
        &self,
        count: &mut dyn FnMut(usize, usize) -> Result<(), Diagnostic>,
    ) -> Result<(), Diagnostic> {
        let Self {
            contour_mm, holes, ..
        } = self;
        count(contour_mm.len().checked_mul(3).ok_or_else(limit)?, 8)?;
        let hole_points = holes.iter().map(Vec::len).sum::<usize>();
        count(hole_points.checked_mul(3).ok_or_else(limit)?, 8)?;
        count(holes.len(), 4)?;
        Ok(())
    }

    fn author(&self, cx: &mut AuthorCx<'_>) -> Result<(), Diagnostic> {
        let Self {
            contour_mm,
            thickness_mm,
            thickness_side,
            corners,
            bends,
            holes,
            hole_records,
            ..
        } = self;
        let data = &mut *cx.data;
        let p = cx.path.to_owned();
        attr(
            data,
            &p,
            "steel:plate:contourPointsMm",
            "double3[]",
            points(contour_mm),
        )?;
        attr(
            data,
            &p,
            "steel:plate:thicknessMm",
            "double",
            sdf::Value::Double(*thickness_mm),
        )?;
        if let Some(side) = thickness_side {
            attr(
                data,
                &p,
                "steel:plate:thicknessSide",
                "token",
                sdf::Value::token(side.as_str()),
            )?;
        }
        if !corners.is_empty() {
            author_corners(data, &p, corners)?;
        }
        if !bends.is_empty() {
            author_bends(data, &p, bends)?;
        }
        if !holes.is_empty() {
            let flat: Vec<[f64; 3]> = holes.iter().flatten().copied().collect();
            let counts = holes
                .iter()
                .map(|h| i32::try_from(h.len()).map_err(|_| limit()))
                .collect::<Result<Vec<_>, _>>()?;
            attr(
                data,
                &p,
                "steel:plate:holePointsMm",
                "double3[]",
                points(&flat),
            )?;
            attr(
                data,
                &p,
                "steel:plate:holeVertexCounts",
                "int[]",
                sdf::Value::IntVec(counts),
            )?;
        }
        if !hole_records.is_empty() {
            author_record_history(
                data,
                &p,
                None,
                &hole_records
                    .iter()
                    .map(|r| (r.operation_id, r.ordinal))
                    .collect::<Vec<_>>(),
                cut_history::Family::PlateHole,
            )?;
            let mut flat: Vec<[f64; 3]> = Vec::new();
            let mut counts = Vec::new();
            let mut ids = Vec::new();
            let mut ordinals = Vec::new();
            let mut kinds = Vec::new();
            let mut owners = Vec::new();
            for record in hole_records {
                flat.extend(record.polygon_mm.iter().map(|[x, y]| [*x, *y, 0.0]));
                counts.push(i32::try_from(record.polygon_mm.len()).map_err(|_| limit())?);
                ids.push(record.operation_id.to_string());
                ordinals.push(i32::try_from(record.ordinal).map_err(|_| limit())?);
                kinds.push(record.kind.as_str().into());
                owners.push(match record.owner {
                    Some(group) => group.to_string(),
                    None => String::new(),
                });
            }
            attr(data, &p, "steel:hole:pointsMm", "double3[]", points(&flat))?;
            attr(
                data,
                &p,
                "steel:hole:vertexCounts",
                "int[]",
                sdf::Value::IntVec(counts),
            )?;
            attr(
                data,
                &p,
                "steel:hole:operationIds",
                "string[]",
                sdf::Value::StringVec(ids),
            )?;
            attr(
                data,
                &p,
                "steel:hole:ordinals",
                "int[]",
                sdf::Value::IntVec(ordinals),
            )?;
            attr(
                data,
                &p,
                "steel:hole:kinds",
                "token[]",
                sdf::Value::TokenVec(kinds),
            )?;
            attr(
                data,
                &p,
                "steel:hole:generatedBy",
                "string[]",
                sdf::Value::StringVec(owners),
            )?;
        }
        Ok(())
    }

    fn attributes(&self) -> Vec<(&'static str, &'static str)> {
        let Self {
            holes,
            cuts,
            hole_records,
            bends,
            ..
        } = self;
        let mut out = Vec::new();
        let mut add = |name: &'static str, ty: &'static str, _place: u8| out.push((name, ty));
        add("steel:plate:contourPointsMm", "double3[]", 0);
        add("steel:plate:thicknessMm", "double", 0);
        if !bends.is_empty() {
            for (name, ty) in [
                ("steel:plate:bendOperationId", "string[]"),
                ("steel:plate:bendKind", "token[]"),
                ("steel:plate:bendLineAMm", "double3[]"),
                ("steel:plate:bendLineBMm", "double3[]"),
                ("steel:plate:bendRadiusMm", "double[]"),
                ("steel:plate:bendRadiusBMm", "double[]"),
                ("steel:plate:bendAngleRad", "double[]"),
            ] {
                add(name, ty, 0);
            }
        }
        if !holes.is_empty() {
            add("steel:plate:holePointsMm", "double3[]", 0);
            add("steel:plate:holeVertexCounts", "int[]", 0);
        }
        if !cuts.is_empty() {
            for &(name, ty) in CUT_FIELDS {
                add(name, ty, 0);
            }
        }
        if !hole_records.is_empty() {
            add("steel:hole:pointsMm", "double3[]", 0);
            add("steel:hole:vertexCounts", "int[]", 0);
            add("steel:hole:operationIds", "string[]", 0);
            add("steel:hole:ordinals", "int[]", 0);
            add("steel:hole:generatedBy", "string[]", 0);
            add(cut_history::HOLE_NEXT, "int", 0);
            add(cut_history::HOLE_RETIRED, "string[]", 0);
        }
        out
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) -> Result<(), Diagnostic> {
        let b = cx.budgets;
        let Self {
            contour_mm,
            thickness_mm,
            corners,
            bends,
            holes,
            hole_records,
            cuts,
            ..
        } = self;
        count_points(contour_mm, b)?;
        cap(contour_mm.iter().flatten().copied(), 1e9)?;
        cap([*thickness_mm], 1e6)?;
        // Corner treatments (A12.2): at most one per vertex, each naming a
        // vertex of this contour with bounded values; whether the corner
        // really turns and the treatments fit the edges are the engine's.
        if corners.len() > contour_mm.len() {
            return Err(limit());
        }
        let mut seen = Vec::new();
        for corner in corners {
            if corner.vertex >= contour_mm.len() || seen.contains(&corner.vertex) {
                return Err(invalid());
            }
            seen.push(corner.vertex);
            let values = match corner.treatment {
                CornerTreatment::Chamfer { d1_mm, d2_mm } => [d1_mm, d2_mm],
                CornerTreatment::Round { radius_mm } => [radius_mm, radius_mm],
            };
            if values.iter().any(|v| *v <= 0.0) {
                return Err(invalid());
            }
            cap(values, 1e9)?;
        }
        if holes.len() + hole_records.len() > MAX_HOLES {
            return Err(limit());
        }
        for hole in holes {
            if hole.len() > MAX_HOLE_POINTS {
                return Err(limit());
            }
            if hole.len() < 3 || hole.iter().any(|p| p[2] != 0.0) {
                return Err(invalid());
            }
            cap(hole.iter().flatten().copied(), 1e9)?;
        }
        for record in hole_records {
            // Record polygons are planar XY pairs by type; winding, placement
            // and clearance are the engine's replay decision.
            if record.polygon_mm.len() > MAX_HOLE_POINTS || record.polygon_mm.len() < 3 {
                return Err(invalid());
            }
            cap(record.polygon_mm.iter().flatten().copied(), 1e9)?;
        }
        cut_values(cuts)?;
        // Bend records (A10): structural bounds only — finiteness and caps for
        // the line points and radii, a finite angle inside (-pi, pi); whether
        // the records form one supported bent body is the engine's replay
        // decision.
        if bends.len() > MAX_BENDS {
            return Err(limit());
        }
        for bend in bends {
            let (lines, radii, angle): ([[f64; 3]; 2], &[f64], Option<f64>) = match bend.spec {
                BendSpec::Fold {
                    a_mm,
                    b_mm,
                    radius_mm,
                    angle_rad,
                } => ([a_mm, b_mm], &[radius_mm][..], Some(angle_rad)),
                BendSpec::Roll {
                    a_mm,
                    b_mm,
                    radius_mm,
                    radius_b_mm,
                } => (
                    [a_mm, b_mm],
                    match radius_b_mm {
                        Some(rb) => &[radius_mm, rb][..],
                        None => &[radius_mm][..],
                    },
                    None,
                ),
            };
            cap(lines.iter().flatten().copied(), 1e9)?;
            if radii.iter().any(|v| *v <= 0.0) {
                return Err(invalid());
            }
            cap(radii.iter().copied(), 1e9)?;
            // A cone with equal radii is a cylinder, and the stored encoding
            // keeps one radius for a cylinder: equal radii would read back as
            // a cylinder and refuse their own sector blank.
            if let BendSpec::Roll {
                radius_mm,
                radius_b_mm: Some(radius_b),
                ..
            } = bend.spec
                && radius_b == radius_mm
            {
                return Err(invalid());
            }
            if let Some(angle) = angle
                && (!angle.is_finite() || angle == 0.0 || angle.abs() >= core::f64::consts::PI)
            {
                return Err(invalid());
            }
        }
        Ok(())
    }

    fn check_finite(&self) -> Result<(), Diagnostic> {
        let Self {
            contour_mm,
            thickness_mm,
            corners,
            bends,
            holes,
            hole_records,
            cuts,
            ..
        } = self;
        finite(
            contour_mm
                .iter()
                .flatten()
                .copied()
                .chain(holes.iter().flatten().flatten().copied())
                .chain(
                    hole_records
                        .iter()
                        .flat_map(|r| r.polygon_mm.iter().flatten().copied()),
                )
                .chain([*thickness_mm])
                .chain(
                    cuts.iter()
                        .flat_map(|c| c.point_mm.into_iter().chain(c.normal)),
                )
                .chain(corners.iter().flat_map(|c| match c.treatment {
                    CornerTreatment::Chamfer { d1_mm, d2_mm } => [d1_mm, d2_mm],
                    CornerTreatment::Round { radius_mm } => [radius_mm, radius_mm],
                }))
                .chain(bends.iter().flat_map(|b| {
                    match b.spec {
                        BendSpec::Fold {
                            a_mm,
                            b_mm,
                            radius_mm,
                            angle_rad,
                        } => a_mm
                            .into_iter()
                            .chain(b_mm)
                            .chain([radius_mm, angle_rad])
                            .collect::<Vec<_>>(),
                        BendSpec::Roll {
                            a_mm,
                            b_mm,
                            radius_mm,
                            radius_b_mm,
                        } => a_mm
                            .into_iter()
                            .chain(b_mm)
                            .chain([radius_mm, radius_b_mm.unwrap_or(radius_mm)])
                            .collect::<Vec<_>>(),
                    }
                })),
        )
    }
}

impl ShapeHook for LoftedPlateShape {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn same_as(&self, other: &dyn ShapeHook) -> bool {
        other.as_any().downcast_ref::<Self>() == Some(self)
    }

    // A lofted plate is a plate: its kind token and part path are the plate's, and the shape
    // token `steel:plate:form` discriminates the record form.
    fn kind_token(&self) -> &'static str {
        "plate"
    }

    fn category(&self) -> ElementCategory {
        ElementCategory::Part
    }

    fn classed(&self) -> bool {
        true
    }

    fn charge(
        &self,
        count: &mut dyn FnMut(usize, usize) -> Result<(), Diagnostic>,
    ) -> Result<(), Diagnostic> {
        let Self { start_mm, end_mm } = self;
        count(
            start_mm
                .len()
                .checked_add(end_mm.len())
                .ok_or_else(limit)?
                .checked_mul(3)
                .ok_or_else(limit)?,
            8,
        )?;
        Ok(())
    }

    fn author(&self, cx: &mut AuthorCx<'_>) -> Result<(), Diagnostic> {
        let Self { start_mm, end_mm } = self;
        let data = &mut *cx.data;
        let p = cx.path.to_owned();
        attr(
            data,
            &p,
            "steel:plate:form",
            "token",
            sdf::Value::token("lofted"),
        )?;
        attr(
            data,
            &p,
            "steel:plate:loftStartMm",
            "double3[]",
            points(start_mm),
        )?;
        attr(
            data,
            &p,
            "steel:plate:loftEndMm",
            "double3[]",
            points(end_mm),
        )?;
        Ok(())
    }

    fn attributes(&self) -> Vec<(&'static str, &'static str)> {
        vec![
            ("steel:plate:form", "token"),
            ("steel:plate:loftStartMm", "double3[]"),
            ("steel:plate:loftEndMm", "double3[]"),
        ]
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) -> Result<(), Diagnostic> {
        let b = cx.budgets;
        let Self { start_mm, end_mm } = self;
        // Both contours of a lofted plate (A11): the shared contour bounds;
        // the equal vertex count, floor, winding and planarity are the
        // engine's, and the certification refuses a twisted correspondence.
        for contour in [start_mm, end_mm] {
            count_points(contour, b)?;
            cap(contour.iter().flatten().copied(), 1e9)?;
        }
        Ok(())
    }

    fn check_finite(&self) -> Result<(), Diagnostic> {
        finite(
            self.start_mm
                .iter()
                .flatten()
                .copied()
                .chain(self.end_mm.iter().flatten().copied()),
        )
    }
}
