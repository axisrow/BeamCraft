// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Display rules for a foreign reference: what a user looks at and snaps to.
//!
//! A reference is only displayed, so nothing in it is a reason to refuse the view. Every prim keeps
//! the full USD transform stack of its ancestors, any `Mesh` is drawn as its polygon cage (a
//! subdivision scheme, holes and creases are ignored) and `Cube` as a box; whatever cannot be
//! drawn (other gprims, a mesh without usable points or faces, a surface beyond the display
//! budgets) is left out instead of refusing the rest. The R02 measurement does not use this
//! module: it keeps its strict profile (`reference_surfaces`).

// The matrices are fixed 4 x 4 arrays indexed by loop counters bounded by 4, so no index can be
// out of range.
#![allow(clippy::indexing_slicing)]

use crate::plate::{property, text};
use beamcraft_engine::limits::{MAX_BLOB_TRIANGLES, MAX_BLOB_VERTICES};
use beamcraft_engine::scene::ReferenceGeometry;
use beamcraft_layer_engine::authored::ReferenceView;
use beamcraft_layer_engine::{Prim, Value};
use std::collections::BTreeMap;

/// A row-vector transform: a point is `[x, y, z, 1] * M`, the translation is the last row.
type Matrix = [[f64; 4]; 4];

const IDENTITY: Matrix = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

/// Most vertices and triangles one wrapper's display takes.
const BUDGET_VERTICES: usize = MAX_BLOB_VERTICES as usize;
const BUDGET_TRIANGLES: usize = MAX_BLOB_TRIANGLES as usize;

fn mul(a: &Matrix, b: &Matrix) -> Matrix {
    let mut out = [[0.0; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            out[i][j] = (0..4).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

/// The inverse by Gauss-Jordan elimination, or `None` for a singular matrix.
fn invert(m: &Matrix) -> Option<Matrix> {
    let mut a = *m;
    let mut inv = IDENTITY;
    for col in 0..4 {
        let pivot = (col..4).max_by(|x, y| a[*x][col].abs().total_cmp(&a[*y][col].abs()))?;
        if a[pivot][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, pivot);
        inv.swap(col, pivot);
        let scale = a[col][col];
        for j in 0..4 {
            a[col][j] /= scale;
            inv[col][j] /= scale;
        }
        for row in 0..4 {
            if row != col {
                let factor = a[row][col];
                for j in 0..4 {
                    a[row][j] -= factor * a[col][j];
                    inv[row][j] -= factor * inv[col][j];
                }
            }
        }
    }
    Some(inv)
}

fn translation(v: [f64; 3]) -> Matrix {
    let mut m = IDENTITY;
    m[3] = [v[0], v[1], v[2], 1.0];
    m
}

fn scale(v: [f64; 3]) -> Matrix {
    let mut m = IDENTITY;
    m[0][0] = v[0];
    m[1][1] = v[1];
    m[2][2] = v[2];
    m
}

/// A rotation about one axis, in degrees.
fn rotation(axis: char, degrees: f64) -> Matrix {
    let (s, c) = degrees.to_radians().sin_cos();
    let mut m = IDENTITY;
    match axis {
        'X' => {
            m[1][1] = c;
            m[1][2] = s;
            m[2][1] = -s;
            m[2][2] = c;
        }
        'Y' => {
            m[0][0] = c;
            m[0][2] = -s;
            m[2][0] = s;
            m[2][2] = c;
        }
        _ => {
            m[0][0] = c;
            m[0][1] = s;
            m[1][0] = -s;
            m[1][1] = c;
        }
    }
    m
}

fn orientation(w: f64, x: f64, y: f64, z: f64) -> Option<Matrix> {
    let length = (w * w + x * x + y * y + z * z).sqrt();
    if !length.is_finite() || length < 1e-12 {
        return None;
    }
    let (w, x, y, z) = (w / length, x / length, y / length, z / length);
    let mut m = IDENTITY;
    // The column-vector rotation matrix, transposed into the row-vector convention.
    m[0][0] = 1.0 - 2.0 * (y * y + z * z);
    m[1][0] = 2.0 * (x * y - z * w);
    m[2][0] = 2.0 * (x * z + y * w);
    m[0][1] = 2.0 * (x * y + z * w);
    m[1][1] = 1.0 - 2.0 * (x * x + z * z);
    m[2][1] = 2.0 * (y * z - x * w);
    m[0][2] = 2.0 * (x * z - y * w);
    m[1][2] = 2.0 * (y * z + x * w);
    m[2][2] = 1.0 - 2.0 * (x * x + y * y);
    Some(m)
}

/// The scalars of a numeric value, flattened (a matrix or a quaternion is an array of numbers).
fn numbers(value: &Value) -> Option<Vec<f64>> {
    match value {
        Value::Number(n) => Some(vec![*n]),
        Value::Int(i) => Some(vec![*i as f64]),
        Value::Uint(u) => Some(vec![*u as f64]),
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.extend(numbers(item)?);
            }
            Some(out)
        }
        _ => None,
    }
}

fn triple(values: &[f64]) -> Option<[f64; 3]> {
    match values {
        [x, y, z] => Some([*x, *y, *z]),
        [n] => Some([*n; 3]),
        _ => None,
    }
}

/// The matrix of one `xformOp:*` attribute, or `None` when it cannot be read: such an operation is
/// then skipped rather than refusing the reference.
fn operation(p: &Prim, op: &str) -> Option<Matrix> {
    let (inverted, name) = match op.strip_prefix("!invert!") {
        Some(rest) => (true, rest),
        None => (false, op),
    };
    let kind = name.strip_prefix("xformOp:")?.split(':').next()?;
    let values = numbers(property(p, name)?)?;
    let matrix = match kind {
        "translate" => translation(triple(&values)?),
        "scale" => scale(triple(&values)?),
        "rotateX" | "rotateY" | "rotateZ" => {
            let axis = kind.chars().last()?;
            rotation(axis, *values.first()?)
        }
        "orient" => match values.as_slice() {
            [w, x, y, z] => orientation(*w, *x, *y, *z)?,
            _ => return None,
        },
        "transform" => {
            if values.len() != 16 {
                return None;
            }
            let mut m = IDENTITY;
            for (row, chunk) in m.iter_mut().zip(values.chunks(4)) {
                for (cell, v) in row.iter_mut().zip(chunk) {
                    *cell = *v;
                }
            }
            m
        }
        _ => {
            let order = kind.strip_prefix("rotate")?;
            let [rx, ry, rz] = triple(&values)?;
            if !matches!(order, "XYZ" | "XZY" | "YXZ" | "YZX" | "ZXY" | "ZYX") {
                return None;
            }
            // The first named axis is applied first.
            order.chars().fold(IDENTITY, |acc, axis| {
                let degrees = match axis {
                    'X' => rx,
                    'Y' => ry,
                    _ => rz,
                };
                mul(&acc, &rotation(axis, degrees))
            })
        }
    };
    if inverted {
        invert(&matrix)
    } else {
        Some(matrix)
    }
}

/// The local transform of a prim and whether it resets the parent's stack. Operations not named in
/// `xformOpOrder` are inactive; the first listed is the outermost.
fn local_matrix(p: &Prim) -> (Matrix, bool) {
    let Some(Value::Array(order)) = property(p, "xformOpOrder") else {
        return (IDENTITY, false);
    };
    let mut matrix = IDENTITY;
    let mut reset = false;
    for entry in order.iter().take(64) {
        let Some(op) = text(entry) else { continue };
        if op == "!resetXformStack!" {
            matrix = IDENTITY;
            reset = true;
        } else if let Some(op) = operation(p, op) {
            matrix = mul(&op, &matrix);
        }
    }
    (matrix, reset)
}

fn finite(m: &Matrix) -> bool {
    m.iter().flatten().all(|v| v.is_finite())
}

fn point(value: &Value) -> Option<[f64; 3]> {
    match value {
        Value::Array(_) => triple(&numbers(value)?),
        _ => None,
    }
}

fn index(value: &Value) -> Option<u32> {
    match value {
        Value::Int(i) => u32::try_from(*i).ok(),
        Value::Uint(i) => u32::try_from(*i).ok(),
        _ => None,
    }
}

fn items<'a>(p: &'a Prim, name: &str, cap: usize) -> Option<&'a [Value]> {
    match property(p, name) {
        Some(Value::Array(values)) if values.len() <= cap => Some(values),
        _ => None,
    }
}

/// A `Mesh` as its polygon cage, or `None` when it has no usable points or faces or would take
/// the display over its budget. `budget` is the vertices and triangles taken so far.
fn mesh(p: &Prim, budget: &mut (usize, usize)) -> Option<ReferenceGeometry> {
    let points: Vec<[f64; 3]> = items(p, "points", BUDGET_VERTICES)?
        .iter()
        .map(point)
        .collect::<Option<_>>()?;
    let counts: Vec<u32> = items(p, "faceVertexCounts", BUDGET_TRIANGLES)?
        .iter()
        .map(index)
        .collect::<Option<_>>()?;
    let indices: Vec<u32> = items(p, "faceVertexIndices", BUDGET_TRIANGLES * 3)?
        .iter()
        .map(index)
        .collect::<Option<_>>()?;
    let corners: u64 = counts.iter().map(|c| u64::from(*c)).sum();
    if points.is_empty()
        || counts.is_empty()
        || corners != indices.len() as u64
        || indices
            .iter()
            .any(|i| usize::try_from(*i).ok().is_none_or(|i| i >= points.len()))
    {
        return None;
    }
    let triangles: usize = counts
        .iter()
        .map(|c| usize::try_from(c.saturating_sub(2)).unwrap_or(0))
        .sum();
    if triangles == 0
        || budget.0.saturating_add(points.len()) > BUDGET_VERTICES
        || budget.1.saturating_add(triangles) > BUDGET_TRIANGLES
    {
        return None;
    }
    budget.0 += points.len();
    budget.1 += triangles;
    Some(ReferenceGeometry {
        points,
        face_vertex_counts: counts,
        face_vertex_indices: indices,
        translation: [0.0; 3],
        left_handed: matches!(property(p, "orientation"), Some(Value::Text(t)) if t == "leftHanded"),
    })
}

/// The surfaces of one wrapper under the display rules.
pub(super) fn display_meshes(
    view: &ReferenceView,
    by_path: &BTreeMap<&str, &Prim>,
) -> Vec<ReferenceGeometry> {
    let prefix = format!("{}/", view.source_root);
    let mut ancestors: BTreeMap<&str, (Matrix, bool)> = BTreeMap::new();
    let mut meshes = Vec::new();
    let mut budget = (0usize, 0usize);
    for (path, p) in by_path
        .range(view.source_root.as_str()..)
        .take_while(|(path, _)| **path == view.source_root || path.starts_with(&prefix))
    {
        let (parent, hidden) = if *path == view.source_root {
            (IDENTITY, false)
        } else {
            let Some((parent, _)) = path.rsplit_once('/') else {
                continue;
            };
            ancestors.get(parent).copied().unwrap_or((IDENTITY, true))
        };
        let invisible =
            matches!(property(p, "visibility"), Some(Value::Text(t)) if t == "invisible");
        let off_purpose =
            property(p, "purpose").is_some_and(|v| !matches!(text(v), Some("default" | "render")));
        let hidden = hidden || !p.active || invisible || off_purpose;
        let (local, reset) = local_matrix(p);
        let world = if reset { local } else { mul(&local, &parent) };
        let world = if finite(&world) { world } else { IDENTITY };
        ancestors.insert(path, (world, hidden));
        if hidden {
            continue;
        }
        let geometry = match p.type_name.as_deref() {
            Some("Cube") => {
                let size = match property(p, "size") {
                    Some(Value::Number(n)) if n.is_finite() && *n > 0.0 => *n,
                    None => 2.0,
                    _ => continue,
                };
                ReferenceGeometry::cube(size, [0.0; 3]).ok()
            }
            Some("Mesh") => mesh(p, &mut budget),
            _ => None,
        };
        let Some(mut geometry) = geometry else {
            continue;
        };
        let det = world[0][0] * (world[1][1] * world[2][2] - world[1][2] * world[2][1])
            - world[0][1] * (world[1][0] * world[2][2] - world[1][2] * world[2][0])
            + world[0][2] * (world[1][0] * world[2][1] - world[1][1] * world[2][0]);
        for q in &mut geometry.points {
            let [x, y, z] = *q;
            *q = std::array::from_fn(|j| {
                x * world[0][j] + y * world[1][j] + z * world[2][j] + world[3][j]
            });
        }
        if det < 0.0 {
            geometry.left_handed = !geometry.left_handed;
        }
        if geometry.points.iter().flatten().all(|v| v.is_finite()) {
            meshes.push(geometry);
        }
    }
    meshes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f64; 3], b: [f64; 3]) {
        assert!(
            a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9),
            "{a:?} != {b:?}"
        );
    }

    fn apply(m: &Matrix, p: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|j| p[0] * m[0][j] + p[1] * m[1][j] + p[2] * m[2][j] + m[3][j])
    }

    #[test]
    fn the_first_listed_operation_is_the_outermost() {
        // [translate, rotateZ]: the point is rotated first, then translated.
        let m = mul(&rotation('Z', 90.0), &translation([10.0, 0.0, 0.0]));
        close(apply(&m, [1.0, 0.0, 0.0]), [10.0, 1.0, 0.0]);
    }

    #[test]
    fn rotations_orientations_and_inverses_agree() {
        close(
            apply(&rotation('X', 90.0), [0.0, 1.0, 0.0]),
            [0.0, 0.0, 1.0],
        );
        close(
            apply(&rotation('Y', 90.0), [0.0, 0.0, 1.0]),
            [1.0, 0.0, 0.0],
        );
        let half = std::f64::consts::FRAC_1_SQRT_2;
        let q = orientation(half, 0.0, 0.0, half).unwrap();
        close(apply(&q, [1.0, 0.0, 0.0]), [0.0, 1.0, 0.0]);
        let m = mul(&scale([2.0, 3.0, 4.0]), &translation([1.0, 2.0, 3.0]));
        let back = mul(&m, &invert(&m).unwrap());
        close(apply(&back, [5.0, 6.0, 7.0]), [5.0, 6.0, 7.0]);
        assert!(invert(&scale([0.0, 1.0, 1.0])).is_none());
    }
}
