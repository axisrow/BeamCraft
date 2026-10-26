// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The structural model for BeamCraft.

pub mod changeset;
pub mod command;
pub mod definition;
pub mod digest;
pub mod error;
pub mod grid;
pub mod grid_edit;
pub mod grid_update;
pub mod id;
pub mod limits;
pub mod prim;
pub mod reference;
pub mod section;

pub use error::ErrorCode;

use beamcraft_geom::Point3;

/// A straight steel beam defined by its axis and a profile name.
#[derive(Debug, Clone, PartialEq)]
pub struct Beam {
    /// Profile designation, e.g. `"IPE300"`.
    pub profile: String,
    /// Start of the beam axis.
    pub start: Point3,
    /// End of the beam axis.
    pub end: Point3,
}

impl Beam {
    /// Length of the beam axis, in millimetres.
    pub fn length(&self) -> f64 {
        self.start.distance_to(self.end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_follows_the_axis() {
        let beam = Beam {
            profile: "IPE300".to_owned(),
            start: Point3::ORIGIN,
            end: Point3::new(6000.0, 0.0, 0.0),
        };
        assert!((beam.length() - 6000.0).abs() < 1e-9);
    }
}
