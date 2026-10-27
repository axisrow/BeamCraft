// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! T01.7 bounded display-only grid tubes, independent of engineering bodies and quantities.
use beamcraft_model::{
    command::CommandError,
    digest::{ContentDigest, Sha256},
    grid::Grid,
};
/// Local-space mesh of disconnected 50 mm diameter, sixteen-sided guide tubes.
#[derive(Debug, Clone, PartialEq)]
pub struct GridGuideMesh {
    /// Geometry-only digest; names, labels and the inherited frame do not change it.
    pub digest: ContentDigest,
    /// Local millimetre points, centred on Z=0.
    pub points_mm: Vec<[f32; 3]>,
    /// Polygon sizes, including tube end caps.
    pub face_counts: Vec<i32>,
    /// Checked point indices.
    pub indices: Vec<i32>,
}
/// Tessellate each admitted finite grid line, with no engineering feature or quantity records.
pub fn guide_mesh(grid: &Grid) -> Result<GridGuideMesh, CommandError> {
    let lines = crate::grid_query::segments(grid)?;
    let mut points = Vec::with_capacity(lines.len() * 32);
    let mut counts = Vec::with_capacity(lines.len() * 18);
    let mut indices = Vec::with_capacity(lines.len() * 96);
    for line in lines {
        let [ax, ay] = line.start;
        let [bx, by] = line.end;
        let dx = bx - ax;
        let dy = by - ay;
        let len = dx.hypot(dy);
        let base = i32::try_from(points.len()).map_err(|_| {
            CommandError::new(
                beamcraft_model::error::ErrorCode::LimitExceeded,
                "grid.guide",
                "grid guide is too large",
            )
        })?;
        for [x, y] in [line.start, line.end] {
            for i in 0..16 {
                let angle = f64::from(i) * std::f64::consts::TAU / 16.;
                let (s, c) = angle.sin_cos();
                points.push([
                    (x - 25. * dy / len * c) as f32,
                    (y + 25. * dx / len * c) as f32,
                    (25. * s) as f32,
                ]);
            }
        }
        for i in 0..16 {
            let j = (i + 1) % 16;
            counts.push(4);
            indices.extend([base + i, base + j, base + 16 + j, base + 16 + i]);
        }
        counts.extend([16, 16]);
        indices.extend((0..16).rev().map(|i| base + i));
        indices.extend((0..16).map(|i| base + 16 + i));
    }
    let mut bytes = b"BeamCraft/T01/GridGuide/v1\0".to_vec();
    for p in &points {
        for v in p {
            bytes.extend_from_slice(&v.to_bits().to_le_bytes());
        }
    }
    for n in counts.iter().chain(&indices) {
        bytes.extend_from_slice(&n.to_le_bytes());
    }
    let digest = Sha256::digest(&bytes).map_err(|_| {
        CommandError::new(
            beamcraft_model::error::ErrorCode::LimitExceeded,
            "grid.guide",
            "grid guide is too large",
        )
    })?;
    Ok(GridGuideMesh {
        digest,
        points_mm: points,
        face_counts: counts,
        indices,
    })
}
