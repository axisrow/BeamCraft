// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Bounded, read-only queries against a complete pinned set of project grids.
use beamcraft_model::{
    command::CommandError,
    error::ErrorCode,
    grid::{Grid, GridFamily},
    id::ElementId,
};

/// A line in grid-local coordinates, with its persistent identity and label.
#[derive(Debug, Clone, PartialEq)]
pub struct GridSegment {
    /// Axis identity.
    pub axis_id: ElementId,
    /// Explicit label.
    pub label: String,
    /// Family (U is zero, V is one).
    pub family: u8,
    /// Local start.
    pub start: [f64; 2],
    /// Local end.
    pub end: [f64; 2],
}
/// A vertical grid-line plane.
#[derive(Debug, Clone, PartialEq)]
pub struct GridPlane {
    /// Grid identity.
    pub grid_id: ElementId,
    /// Axis identity.
    pub axis_id: ElementId,
    /// World point on the plane, at zero height.
    pub point_world_mm: [f64; 3],
    /// Horizontal unit normal.
    pub normal_world: [f64; 3],
}
fn bad(code: ErrorCode, message: &'static str) -> CommandError {
    CommandError::new(code, "grid.query", message).with_rule("T01-V01")
}
/// Validate the bounded source and resolve a canonical id first, or an unambiguous exact name.
pub fn resolve_grid<'a>(grids: &'a [Grid], selector: &str) -> Result<&'a Grid, CommandError> {
    validate(grids)?;
    if let Ok(id) = ElementId::parse(selector) {
        return grids
            .iter()
            .find(|g| g.id == id)
            .ok_or_else(|| bad(ErrorCode::NotFound, "GRID_UNKNOWN"));
    }
    let mut matches = grids.iter().filter(|g| g.name == selector);
    let grid = matches
        .next()
        .ok_or_else(|| bad(ErrorCode::NotFound, "GRID_UNKNOWN"))?;
    if matches.next().is_some() {
        return Err(bad(ErrorCode::InvalidArgument, "GRID_NAME_AMBIGUOUS"));
    }
    Ok(grid)
}
/// Admit no more than 64 grids and the model's bounded families.
pub fn validate(grids: &[Grid]) -> Result<(), CommandError> {
    if grids.len() > 64 {
        return Err(bad(ErrorCode::LimitExceeded, "grid query exceeds 64 grids"));
    }
    let mut ids = std::collections::BTreeSet::new();
    for g in grids {
        g.validate()?;
        if !ids.insert(g.id) || g.axis_ids().any(|id| !ids.insert(id)) {
            return Err(bad(
                ErrorCode::DuplicateId,
                "grid query contains duplicate identities",
            ));
        }
    }
    Ok(())
}
/// Expand at most 400 admitted lines without changing their identities.
pub fn segments(grid: &Grid) -> Result<Vec<GridSegment>, CommandError> {
    grid.validate()?;
    let mut out = Vec::new();
    for (family, f) in [(0, &grid.u), (1, &grid.v)] {
        match f {
            GridFamily::Regular(f) => {
                for ((id, label), p) in f.axis_ids.iter().zip(&f.labels).zip(&f.positions_mm) {
                    let [a, b] = f.span_mm;
                    let (start, end) = if family == 0 {
                        ([*p, a], [*p, b])
                    } else {
                        ([a, *p], [b, *p])
                    };
                    out.push(GridSegment {
                        axis_id: *id,
                        label: label.clone(),
                        family,
                        start,
                        end,
                    });
                }
            }
            GridFamily::Drawn(lines) => {
                for l in lines {
                    out.push(GridSegment {
                        axis_id: l.axis_id,
                        label: l.label.clone(),
                        family,
                        start: l.start_mm,
                        end: l.end_mm,
                    });
                }
            }
        }
    }
    Ok(out)
}
/// Transform local XY to world XY using the stored normalized quaternion.
pub fn local_to_world(g: &Grid, p: [f64; 2]) -> [f64; 2] {
    let [z, w] = g.orientation_zw;
    let n = z * z + w * w;
    let c = (w * w - z * z) / n;
    let s = 2.0 * z * w / n;
    let [x, y] = p;
    let [tx, ty] = g.translation_mm;
    [tx + c * x - s * y, ty + s * x + c * y]
}
fn cross([ax, ay]: [f64; 2], [bx, by]: [f64; 2]) -> f64 {
    ax * by - ay * bx
}
fn sub([ax, ay]: [f64; 2], [bx, by]: [f64; 2]) -> [f64; 2] {
    [ax - bx, ay - by]
}
/// Unique segment intersection, including endpoints; collinear or parallel pairs are undefined.
pub fn intersection(a: &GridSegment, b: &GridSegment) -> Option<[f64; 2]> {
    if a.family == b.family {
        return None;
    }
    let r = sub(a.end, a.start);
    let s = sub(b.end, b.start);
    let den = cross(r, s);
    // Reject a numerically unresolved direction instead of inventing a remote crossing.
    let [rx, ry] = r;
    let [sx, sy] = s;
    if den.abs() <= 16.0 * f64::EPSILON * (rx * sy).abs().max((ry * sx).abs()) {
        return None;
    }
    let q = sub(b.start, a.start);
    let t = cross(q, s) / den;
    let u = cross(q, r) / den;
    if !(0.0..=1.0).contains(&t) || !(0.0..=1.0).contains(&u) {
        return None;
    }
    let [x, y] = a.start;
    let [dx, dy] = r;
    Some([x + t * dx, y + t * dy])
}
/// Resolve labels from different families in either order to a world XY intersection.
pub fn grid_point(grid: &Grid, first: &str, second: &str) -> Result<[f64; 2], CommandError> {
    let lines = segments(grid)?;
    let find = |label| {
        lines
            .iter()
            .find(|l| l.label == label)
            .ok_or_else(|| bad(ErrorCode::NotFound, "GRID_AXIS_UNKNOWN"))
    };
    let p = intersection(find(first)?, find(second)?)
        .ok_or_else(|| bad(ErrorCode::InvalidArgument, "GRID_INTERSECTION_UNDEFINED"))?;
    Ok(local_to_world(grid, p))
}
/// Resolve an axis label to its vertical world plane.
pub fn grid_line_plane(grid: &Grid, label: &str) -> Result<GridPlane, CommandError> {
    let lines = segments(grid)?;
    let line = lines
        .iter()
        .find(|l| l.label == label)
        .ok_or_else(|| bad(ErrorCode::NotFound, "GRID_AXIS_UNKNOWN"))?;
    let a = local_to_world(grid, line.start);
    let b = local_to_world(grid, line.end);
    let [dx, dy] = sub(b, a);
    let len = dx.hypot(dy);
    Ok(GridPlane {
        grid_id: grid.id,
        axis_id: line.axis_id,
        point_world_mm: [a[0], a[1], 0.0],
        normal_world: [-dy / len, dx / len, 0.0],
    })
}
/// Closest point on a finite segment, in local XY.
pub fn closest(line: &GridSegment, p: [f64; 2]) -> [f64; 2] {
    let [dx, dy] = sub(line.end, line.start);
    let [px, py] = sub(p, line.start);
    let t = ((px * dx + py * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    let [x, y] = line.start;
    [x + t * dx, y + t * dy]
}
