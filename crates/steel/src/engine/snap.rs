// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The snap axes of the steel members of a committed revision.

use beamcraft_engine::error::ViewError;
use beamcraft_engine::query::feature::add;
use beamcraft_engine::query::pick::{MAX_SCANNED_ELEMENTS, MemberSnapAxis, input, limit};
use beamcraft_engine::solid::rotate_vector;

/// Compute reference axes from evaluated records, without inferring them from a section mesh.
pub fn member_snap_axes(
    records: &[crate::engine::committed::MemberRecord],
) -> Result<Vec<MemberSnapAxis>, ViewError> {
    if records.len() > MAX_SCANNED_ELEMENTS {
        return Err(limit());
    }
    let mut budget = 1_048_576usize;
    for record in records {
        let count = record.authored.axis_points_mm.len();
        if !(2..=beamcraft_model::limits::CONTOUR_POINT_CAP).contains(&count) {
            return Err(input());
        }
        budget = budget.checked_sub(count).ok_or_else(limit)?;
    }
    Ok(records
        .iter()
        .map(|record| {
            let placement = &record.authored.placement;
            MemberSnapAxis {
                element_id: record.authored.id,
                points_mm: record
                    .authored
                    .axis_points_mm
                    .iter()
                    .map(|p| {
                        add(
                            rotate_vector(*p, placement.orientation_xyzw),
                            placement.translation_mm,
                        )
                    })
                    .collect(),
            }
        })
        .collect())
}
