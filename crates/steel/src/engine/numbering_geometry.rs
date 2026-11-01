// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The numbering geometry of a member: its evaluated solid in its first section frame.

use crate::engine::member::{AuthoredMember, MemberBody};
use beamcraft_engine::numbering::{NumberingError, PartGeometry};
use beamcraft_geom::{MemberFrame, Point3};

/// Evaluated member geometry in its first section frame, without scene placement. The
/// final solid includes sweeps and replayed cuts, rather than just the authored axis.
pub fn member_geometry(
    authored: &AuthoredMember,
    body: &MemberBody,
) -> Result<PartGeometry, NumberingError> {
    let (Some([sx, sy, sz]), Some([ex, ey, ez])) = (
        authored.axis_points_mm.first(),
        authored.axis_points_mm.get(1),
    ) else {
        return Err(NumberingError("member axis has no first segment"));
    };
    let frame = MemberFrame::new(
        Point3::new(*sx, *sy, *sz),
        Point3::new(*ex, *ey, *ez),
        body.width_mm,
        body.height_mm,
        authored.roll_rad,
    )
    .map_err(|_| NumberingError("member numbering frame is invalid"))?;
    PartGeometry::from_solid(&body.solid, frame.frame())
}
