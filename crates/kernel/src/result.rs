// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Sealed G02 result; no state publication or cache identity is implied.

use crate::mesh::PlateMesh;
use beamcraft_geom::tolerance::Tolerance;

/// Successful rectangular primitive evaluation, available only after self-checks.
#[derive(Debug, Clone, PartialEq)]
pub struct PlateResult {
    /// The sealed mesh.
    pub mesh: PlateMesh,
    /// The analytic volume in cubic millimetres.
    pub volume_mm3: f64,
    /// The local-size policy.
    pub tolerance: Tolerance,
}

impl PlateResult {
    /// Closed canonical triangle mesh, in element-local millimetres.
    pub const fn mesh(&self) -> &PlateMesh {
        &self.mesh
    }
    /// Analytic volume `(width*length)*thickness` in cubic millimetres.
    pub const fn volume_mm3(&self) -> f64 {
        self.volume_mm3
    }
    /// Analytic volume divided by exactly representable 1,000,000,000.
    pub fn volume_m3(&self) -> f64 {
        self.volume_mm3 / 1_000_000_000.0
    }
    /// Local-size policy used for dimension admission.
    pub const fn tolerance(&self) -> Tolerance {
        self.tolerance
    }
}
