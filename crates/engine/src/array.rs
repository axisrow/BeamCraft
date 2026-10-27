// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! T03 bounded patterns; count always means new copies, excluding the original.
use crate::transform::{MAX_GROUP, RigidDelta};
use beamcraft_model::{command::CommandError, error::ErrorCode};
fn count(n: u32) -> Result<(), CommandError> {
    if n == 0 || n as usize > MAX_GROUP {
        Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "count",
            "array needs 1..128 new copies",
        ))
    } else {
        Ok(())
    }
}
/// Translations at step, twice step, ... relative to each original source.
pub fn linear(step: [f64; 3], copies: u32) -> Result<Vec<RigidDelta>, CommandError> {
    count(copies)?;
    (1..=copies)
        .map(|i| RigidDelta::translation(step.map(|v| v * f64::from(i))))
        .collect()
}
/// Rotations at angle step, twice angle step, ... radians around the same world axis.
pub fn polar(
    center: [f64; 3],
    axis: [f64; 3],
    angle: f64,
    copies: u32,
) -> Result<Vec<RigidDelta>, CommandError> {
    count(copies)?;
    (1..=copies)
        .map(|i| RigidDelta::rotation(center, axis, angle * f64::from(i)))
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    use beamcraft_model::command::Placement;
    #[test]
    fn count_excludes_original_and_all_offsets_are_from_source() {
        let deltas = linear([10., 0., 0.], 3).unwrap();
        let p = Placement {
            translation_mm: [5., 0., 0.],
            ..Placement::IDENTITY
        };
        assert_eq!(
            deltas
                .iter()
                .map(|d| d.apply(p).unwrap().translation_mm[0])
                .collect::<Vec<_>>(),
            vec![15., 25., 35.]
        );
        assert!(linear([0.; 3], 0).is_err());
        assert!(polar([0.; 3], [0., 0., 1.], 1., 129).is_err());
    }
}
