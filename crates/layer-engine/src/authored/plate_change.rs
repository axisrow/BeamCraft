// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The edits of a plate, its holes and its cuts behind the discipline hook (#795): the contour,
//! the thickness and its side, the corner treatments, the bend records, the cut records and the
//! plate and member hole records. Each body is the arm it replaces in `edit.rs` and
//! `validate.rs`, moved unchanged, so the bytes written and the refusals returned are the same.

use super::edit::{
    CUT_FIELDS, MHOLE_FIELDS, attr, author_bends, author_corners, author_cuts,
    author_record_history, points,
};
use super::hook::{ChangeAuthorCx, ChangeHook, ChangeValidateCx, HookedChange};
use super::validate::{cap, count_points, cut_values, finite};
use super::*;
use openusd::sdf;
use std::any::Any;

/// The groups of an edit of a shape input: read the lifecycle and the inputs, write the inputs
/// and the derived result.
const SHAPE_INPUT_GROUPS: (&[Group], &[Group]) = (
    &[Group::Lifecycle, Group::ShapeInputs],
    &[Group::ShapeInputs, Group::Derived],
);

macro_rules! plate_change {
    ($(#[$meta:meta])* $name:ident { $field:ident : $ty:ty }, $ctor:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq)]
        pub struct $name {
            /// Full target UUIDv7.
            pub id: String,
            /// The new value.
            pub $field: $ty,
        }

        impl Change {
            #[doc = concat!("The edit [`", stringify!($name), "`].")]
            pub fn $ctor(id: String, $field: $ty) -> Self {
                Self::Discipline(HookedChange::new($name { id, $field }))
            }
        }
    };
}

macro_rules! common {
    ($key:literal, |$kind:ident| $kinds:expr) => {
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

        fn targets_kind(&self, $kind: &str) -> bool {
            $kinds
        }

        fn groups(&self) -> (&'static [Group], &'static [Group]) {
            SHAPE_INPUT_GROUPS
        }

        fn invalidates_derived(&self) -> bool {
            true
        }
    };
}

plate_change!(
    /// Replace the complete contour; engineering validity is a host prerequisite.
    SetContour { points_mm: Vec<[f64; 3]> }, contour
);
plate_change!(
    /// Replace a required plate thickness.
    SetThickness { mm: f64 }, thickness
);
plate_change!(
    /// Complete sparse replacement of the plate's corner treatments (A12.2): the list authors
    /// every treated contour vertex as a whole, an empty list clears them all back to sharp.
    SetCorners { corners: Vec<PlateCorner> }, corners
);
plate_change!(
    /// Complete replacement of the plate's bend records (A10).
    SetBends { bends: Vec<BendRecord> }, bends
);
plate_change!(
    /// Replace the plate's thickness side (A12.1).
    SetThicknessSide { side: PlateSide }, thickness_side
);
plate_change!(
    /// Replace the owned cut records of a plate, beam or column (A03).
    SetCuts { cuts: Vec<CutRecord> }, cuts
);
plate_change!(
    /// Replace the plate's hole records (A04).
    SetHoleRecords { records: Vec<HoleRecord> }, hole_records
);
plate_change!(
    /// Replace the member's hole records (A04.2).
    SetMemberHoles { records: Vec<MemberHoleRecord> }, member_holes
);

impl ChangeHook for SetContour {
    common!("steel:plate:contourPointsMm", |k| k == "plate");

    fn charge(
        &self,
        count: &mut dyn FnMut(usize, usize) -> Result<(), Diagnostic>,
    ) -> Result<(), Diagnostic> {
        count(self.points_mm.len().checked_mul(3).ok_or_else(limit)?, 8)
    }

    fn validate(&self, cx: &mut ChangeValidateCx<'_>) -> Result<(), Diagnostic> {
        count_points(&self.points_mm, cx.budgets)?;
        cap(self.points_mm.iter().flatten().copied(), 1e9)
    }

    fn check_finite(&self) -> Result<(), Diagnostic> {
        finite(self.points_mm.iter().flatten().copied())
    }

    fn author(&self, cx: &mut ChangeAuthorCx<'_>) -> Result<(), Diagnostic> {
        attr(
            cx.data,
            cx.path,
            "steel:plate:contourPointsMm",
            "double3[]",
            points(&self.points_mm),
        )
    }

    fn attributes(&self) -> Vec<(&'static str, &'static str)> {
        vec![("steel:plate:contourPointsMm", "double3[]")]
    }
}

impl ChangeHook for SetThickness {
    common!("steel:plate:thicknessMm", |k| k == "plate");

    fn validate(&self, _cx: &mut ChangeValidateCx<'_>) -> Result<(), Diagnostic> {
        cap([self.mm], 1e6)
    }

    fn check_finite(&self) -> Result<(), Diagnostic> {
        finite([self.mm])
    }

    fn author(&self, cx: &mut ChangeAuthorCx<'_>) -> Result<(), Diagnostic> {
        attr(
            cx.data,
            cx.path,
            "steel:plate:thicknessMm",
            "double",
            sdf::Value::Double(self.mm),
        )
    }

    fn attributes(&self) -> Vec<(&'static str, &'static str)> {
        vec![("steel:plate:thicknessMm", "double")]
    }
}

impl ChangeHook for SetCorners {
    common!("steel:plate:cornerVertex", |k| k == "plate");

    fn validate(&self, _cx: &mut ChangeValidateCx<'_>) -> Result<(), Diagnostic> {
        let corners = &self.corners;
        if corners.len() > 1024 {
            return Err(limit());
        }
        let mut seen = Vec::new();
        for corner in corners {
            if corner.vertex >= 1024 || seen.contains(&corner.vertex) {
                return Err(invalid());
            }
            seen.push(corner.vertex);
            corner.validate().map_err(|_| invalid())?;
        }
        Ok(())
    }

    fn check_finite(&self) -> Result<(), Diagnostic> {
        Ok(())
    }

    fn author(&self, cx: &mut ChangeAuthorCx<'_>) -> Result<(), Diagnostic> {
        author_corners(cx.data, cx.path, &self.corners)
    }

    fn attributes(&self) -> Vec<(&'static str, &'static str)> {
        vec![
            ("steel:plate:cornerVertex", "int[]"),
            ("steel:plate:cornerKind", "token[]"),
            ("steel:plate:cornerD1Mm", "double[]"),
            ("steel:plate:cornerD2Mm", "double[]"),
            ("steel:plate:cornerRadiusMm", "double[]"),
        ]
    }
}

impl ChangeHook for SetBends {
    common!("steel:plate:bendOperationId", |k| k == "plate");

    fn validate(&self, _cx: &mut ChangeValidateCx<'_>) -> Result<(), Diagnostic> {
        let bends = &self.bends;
        // Structural bounds only: finiteness, caps, positive radii and a finite angle
        // inside (-pi, pi); whether the records form one supported bent body is the
        // engine's replay decision.
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
            // A cone with equal radii is a cylinder, and the stored encoding keeps one
            // radius for a cylinder: equal radii would read back as a cylinder and
            // refuse their own sector blank.
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
        Ok(())
    }

    fn author(&self, cx: &mut ChangeAuthorCx<'_>) -> Result<(), Diagnostic> {
        author_bends(cx.data, cx.path, &self.bends)
    }

    fn attributes(&self) -> Vec<(&'static str, &'static str)> {
        vec![
            ("steel:plate:bendOperationId", "string[]"),
            ("steel:plate:bendKind", "token[]"),
            ("steel:plate:bendLineAMm", "double3[]"),
            ("steel:plate:bendLineBMm", "double3[]"),
            ("steel:plate:bendRadiusMm", "double[]"),
            ("steel:plate:bendRadiusBMm", "double[]"),
            ("steel:plate:bendAngleRad", "double[]"),
        ]
    }
}

impl ChangeHook for SetThicknessSide {
    common!("steel:plate:thicknessSide", |k| matches!(
        k,
        "plate" | "beam" | "column"
    ));

    fn validate(&self, _cx: &mut ChangeValidateCx<'_>) -> Result<(), Diagnostic> {
        Ok(())
    }

    fn check_finite(&self) -> Result<(), Diagnostic> {
        Ok(())
    }

    fn author(&self, cx: &mut ChangeAuthorCx<'_>) -> Result<(), Diagnostic> {
        attr(
            cx.data,
            cx.path,
            "steel:plate:thicknessSide",
            "token",
            sdf::Value::token(self.side.as_str()),
        )
    }

    fn attributes(&self) -> Vec<(&'static str, &'static str)> {
        vec![("steel:plate:thicknessSide", "token")]
    }
}

impl ChangeHook for SetCuts {
    common!("steel:cut", |k| matches!(k, "plate" | "beam" | "column"));

    fn validate(&self, _cx: &mut ChangeValidateCx<'_>) -> Result<(), Diagnostic> {
        cut_values(&self.cuts)
    }

    fn check_finite(&self) -> Result<(), Diagnostic> {
        finite(
            self.cuts
                .iter()
                .flat_map(|c| c.point_mm.into_iter().chain(c.normal)),
        )
    }

    fn author(&self, cx: &mut ChangeAuthorCx<'_>) -> Result<(), Diagnostic> {
        author_cuts(
            cx.data,
            cx.path,
            &self.cuts,
            cx.prims.iter().find(|old| old.path == cx.path),
        )
    }

    fn attributes(&self) -> Vec<(&'static str, &'static str)> {
        CUT_FIELDS.to_vec()
    }
}

impl ChangeHook for SetHoleRecords {
    common!("steel:hole", |k| k == "plate");

    // The hole records write no derived field of their own.
    fn declares_mesh(&self) -> bool {
        false
    }

    fn validate(&self, _cx: &mut ChangeValidateCx<'_>) -> Result<(), Diagnostic> {
        Ok(())
    }

    fn check_finite(&self) -> Result<(), Diagnostic> {
        Ok(())
    }

    fn author(&self, cx: &mut ChangeAuthorCx<'_>) -> Result<(), Diagnostic> {
        let (authored, prims) = (&mut *cx.data, cx.prims);
        let p = cx.path.to_owned();
        let records = &self.records;
        author_record_history(
            authored,
            &p,
            prims.iter().find(|old| old.path == p),
            &records
                .iter()
                .map(|r| (r.operation_id, r.ordinal))
                .collect::<Vec<_>>(),
            cut_history::Family::PlateHole,
        )?;
        // The six record attributes, always together: the authoritative polygons of
        // every record, replayed from the base shape in ordinal order, with the id,
        // the engine-assigned ordinal, the kind (hole or polygon cut) and the
        // generating bolt group of each (empty for a hand-placed record, A14.4).
        let mut flat = Vec::new();
        let mut counts = Vec::new();
        let mut ids = Vec::new();
        let mut ordinals = Vec::new();
        let mut kinds = Vec::new();
        let mut owners = Vec::new();
        for record in records {
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
        attr(
            authored,
            &p,
            "steel:hole:pointsMm",
            "double3[]",
            points(&flat),
        )?;
        attr(
            authored,
            &p,
            "steel:hole:vertexCounts",
            "int[]",
            sdf::Value::IntVec(counts),
        )?;
        attr(
            authored,
            &p,
            "steel:hole:operationIds",
            "string[]",
            sdf::Value::StringVec(ids),
        )?;
        attr(
            authored,
            &p,
            "steel:hole:ordinals",
            "int[]",
            sdf::Value::IntVec(ordinals),
        )?;
        attr(
            authored,
            &p,
            "steel:hole:kinds",
            "token[]",
            sdf::Value::TokenVec(kinds),
        )?;
        attr(
            authored,
            &p,
            "steel:hole:generatedBy",
            "string[]",
            sdf::Value::StringVec(owners),
        )?;
        Ok(())
    }

    fn attributes(&self) -> Vec<(&'static str, &'static str)> {
        vec![
            ("steel:hole:pointsMm", "double3[]"),
            ("steel:hole:vertexCounts", "int[]"),
            ("steel:hole:operationIds", "string[]"),
            ("steel:hole:ordinals", "int[]"),
            ("steel:hole:generatedBy", "string[]"),
            (cut_history::HOLE_NEXT, "int"),
            (cut_history::HOLE_RETIRED, "string[]"),
        ]
    }
}

impl ChangeHook for SetMemberHoles {
    common!("steel:mhole", |k| matches!(k, "beam" | "column"));

    fn validate(&self, _cx: &mut ChangeValidateCx<'_>) -> Result<(), Diagnostic> {
        Ok(())
    }

    fn check_finite(&self) -> Result<(), Diagnostic> {
        Ok(())
    }

    fn author(&self, cx: &mut ChangeAuthorCx<'_>) -> Result<(), Diagnostic> {
        let (authored, prims) = (&mut *cx.data, cx.prims);
        let p = cx.path.to_owned();
        let records = &self.records;
        author_record_history(
            authored,
            &p,
            prims.iter().find(|old| old.path == p),
            &records
                .iter()
                .map(|r| (r.operation_id, r.ordinal))
                .collect::<Vec<_>>(),
            cut_history::Family::MemberHole,
        )?;
        // The nine record attributes, always together: the authoritative polygons of
        // every record and their member-frame axis points, directions, extents, ids and
        // ordinals (A04.2). A through extent stores zero range values.
        let mut flat = Vec::new();
        let mut counts = Vec::new();
        let mut ids = Vec::new();
        let mut ordinals = Vec::new();
        let mut axis_points = Vec::new();
        let mut directions = Vec::new();
        let mut extents = Vec::new();
        let mut from = Vec::new();
        let mut to = Vec::new();
        for record in records {
            flat.extend(record.polygon_mm.iter().map(|[x, y]| [*x, *y, 0.0]));
            counts.push(i32::try_from(record.polygon_mm.len()).map_err(|_| limit())?);
            ids.push(record.operation_id.to_string());
            ordinals.push(i32::try_from(record.ordinal).map_err(|_| limit())?);
            axis_points.push(record.axis_point_mm);
            directions.push(record.axis_direction);
            extents.push(record.extent.as_str().into());
            let (start, end) = match record.extent {
                HoleExtent::Through => (0.0, 0.0),
                HoleExtent::Range { from_mm, to_mm } => (from_mm, to_mm),
            };
            from.push(start);
            to.push(end);
        }
        for (name, ty, v) in [
            ("steel:mhole:pointsMm", "double3[]", points(&flat)),
            (
                "steel:mhole:vertexCounts",
                "int[]",
                sdf::Value::IntVec(counts),
            ),
            (
                "steel:mhole:operationIds",
                "string[]",
                sdf::Value::StringVec(ids),
            ),
            (
                "steel:mhole:ordinals",
                "int[]",
                sdf::Value::IntVec(ordinals),
            ),
            (
                "steel:mhole:axisPointsMm",
                "double3[]",
                points(&axis_points),
            ),
            (
                "steel:mhole:axisDirections",
                "double3[]",
                points(&directions),
            ),
            (
                "steel:mhole:extents",
                "token[]",
                sdf::Value::TokenVec(extents),
            ),
            (
                "steel:mhole:extentFromMm",
                "double[]",
                sdf::Value::DoubleVec(from),
            ),
            (
                "steel:mhole:extentToMm",
                "double[]",
                sdf::Value::DoubleVec(to),
            ),
        ] {
            attr(authored, &p, name, ty, v)?;
        }
        Ok(())
    }

    fn attributes(&self) -> Vec<(&'static str, &'static str)> {
        MHOLE_FIELDS.to_vec()
    }
}
