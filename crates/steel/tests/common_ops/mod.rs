// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Independent oracles for the G09 operation tests.
//!
//! Nothing here calls production geometry code: expected volumes come from closed forms,
//! voxel counting and plain f64 loops; the blob, topology and orientation checks reread the
//! public buffers with the G05/G06 test readers and `std` collections.

#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used
)]

use crate::common::{mesh_volume, parse_blob, topology};
use crate::common_mesh::{self, Buf};
use beamcraft_kernel::ops::{Body, OpResult};
use beamcraft_kernel::trimesh::TriMesh;
use beamcraft_kernel::{RectangularPlate, rectangular_plate};

/// A G02 plate as a body (the six `plate:` ids).
pub fn plate(w: f64, l: f64, t: f64) -> Body {
    let plate = rectangular_plate(RectangularPlate {
        width_mm: w,
        length_mm: l,
        thickness_mm: t,
    })
    .unwrap();
    Body::from_plate(&plate).unwrap()
}

/// Face of each triangle of `common_mesh::cuboid` (two triangles per face).
pub const CUBOID_FACES: [u16; 12] = [0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5];
/// Outward unit normals of the cuboid faces in `CUBOID_FACES` order.
pub const CUBOID_NORMALS: [[f64; 3]; 6] = [
    [-1.0, 0.0, 0.0],
    [1.0, 0.0, 0.0],
    [0.0, -1.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, 0.0, -1.0],
    [0.0, 0.0, 1.0],
];

/// An axis-aligned box of any origin as a body (`box:f.x0` ... `box:f.z1`).
pub fn cuboid(lo: [f64; 3], hi: [f64; 3]) -> Body {
    let buf = common_mesh::cuboid(lo, hi);
    let ids = [
        "box:f.x0", "box:f.x1", "box:f.y0", "box:f.y1", "box:f.z0", "box:f.z1",
    ];
    body_of(&buf, &CUBOID_FACES, &ids, &CUBOID_NORMALS.map(Some))
}

/// Builds a body from plain buffers, through the real blob parser.
pub fn body_of(buf: &Buf, faces: &[u16], ids: &[&str], normals: &[Option<[f64; 3]>]) -> Body {
    let blob = common_mesh::blob(&buf.v, &buf.t, faces, ids.len() as u32, 0);
    let mesh = TriMesh::from_blob(&blob).unwrap();
    Body::from_mesh(
        mesh,
        ids.iter().map(|s| (*s).to_owned()).collect(),
        normals.to_vec(),
        None,
    )
    .unwrap()
}

/// The same body with its vertices and triangles renumbered and rotated at random.
pub fn renumbered(body: &Body, rng: &mut common_mesh::Rng) -> Body {
    let mesh = body.mesh();
    let n = mesh.vertices().len();
    let mut perm: Vec<usize> = (0..n).collect();
    for i in (1..n).rev() {
        perm.swap(i, rng.below(i as u64 + 1) as usize);
    }
    let mut v = vec![[0.0; 3]; n];
    for (old, &new) in perm.iter().enumerate() {
        v[new] = mesh.vertices()[old];
    }
    let mut items: Vec<([u32; 3], u16)> = mesh
        .triangles()
        .iter()
        .zip(mesh.faces())
        .map(|([a, b, c], f)| {
            let mut tri = [
                perm[*a as usize] as u32,
                perm[*b as usize] as u32,
                perm[*c as usize] as u32,
            ];
            tri.rotate_left(rng.below(3) as usize);
            (tri, *f)
        })
        .collect();
    for i in (1..items.len()).rev() {
        items.swap(i, rng.below(i as u64 + 1) as usize);
    }
    let t: Vec<[u32; 3]> = items.iter().map(|(t, _)| *t).collect();
    let faces: Vec<u16> = items.iter().map(|(_, f)| *f).collect();
    let blob = common_mesh::blob(&v, &t, &faces, mesh.face_count(), 0);
    Body::from_mesh(
        TriMesh::from_blob(&blob).unwrap(),
        body.face_table().to_vec(),
        body.face_normals().to_vec(),
        Some(body.tolerance()),
    )
    .unwrap()
}

/// Applies `f` to every vertex and `g` to every face normal; `mirror` reverses the winding
/// (for reflections).
pub fn mapped(
    body: &Body,
    f: impl Fn([f64; 3]) -> [f64; 3],
    g: impl Fn([f64; 3]) -> [f64; 3],
    mirror: bool,
) -> Body {
    let mesh = body.mesh();
    let v: Vec<[f64; 3]> = mesh
        .vertices()
        .iter()
        .map(|p| f(*p).map(|x| if x == 0.0 { 0.0 } else { x }))
        .collect();
    let t: Vec<[u32; 3]> = mesh
        .triangles()
        .iter()
        .map(|[a, b, c]| if mirror { [*a, *c, *b] } else { [*a, *b, *c] })
        .collect();
    let blob = common_mesh::blob(&v, &t, mesh.faces(), mesh.face_count(), 0);
    Body::from_mesh(
        TriMesh::from_blob(&blob).unwrap(),
        body.face_table().to_vec(),
        body.face_normals()
            .iter()
            .map(|n| n.map(|n| g(n).map(|x| if x == 0.0 { 0.0 } else { x })))
            .collect(),
        Some(body.tolerance()),
    )
    .unwrap()
}

/// Volume of `{x in [0,l0]x[0,l1]x[0,l2] : n . x <= d}` for strictly positive `n`
/// (inclusion-exclusion over the box corners; a closed form independent of any mesh).
pub fn box_halfspace_volume(l: [f64; 3], n: [f64; 3], d: f64) -> f64 {
    let mut sum = 0.0;
    for mask in 0..8u32 {
        let mut shifted = d;
        let mut sign = 1.0;
        for axis in 0..3 {
            if mask & (1 << axis) != 0 {
                shifted -= n[axis] * l[axis];
                sign = -sign;
            }
        }
        if shifted > 0.0 {
            sum += sign * shifted * shifted * shifted;
        }
    }
    sum / (6.0 * n[0] * n[1] * n[2])
}

/// Everything about a sealed result that can be verified from its public buffers alone.
pub struct Facts {
    pub vertices: usize,
    pub edges: usize,
    pub triangles: usize,
    pub euler: i64,
    pub volume: f64,
}

/// Rereads `res` with the independent readers and asserts every promise of the pilot.
pub fn assert_sealed(res: &OpResult) -> Facts {
    let blob = parse_blob(res.blob());
    let mesh = res.body().mesh();
    assert_eq!(blob.vertices, mesh.vertices());
    assert_eq!(blob.triangles, mesh.triangles());
    assert_eq!(blob.faces, mesh.faces());
    assert_eq!(blob.space, 0);
    // An independent encoder reproduces the exact bytes.
    assert_eq!(
        common_mesh::blob(
            &blob.vertices,
            &blob.triangles,
            &blob.faces,
            blob.face_count,
            0
        ),
        res.blob()
    );
    assert_eq!(res.subdivision_scheme(), "none");
    assert_eq!(res.orientation(), "rightHanded");
    // Canonical order: sorted vertices, rotated triangles sorted by face.
    assert!(blob.vertices.windows(2).all(|w| {
        w[0].iter()
            .zip(&w[1])
            .map(|(a, b)| a.total_cmp(b))
            .find(|o| o.is_ne())
            .is_some_and(|o| o.is_lt())
    }));
    for tri in &blob.triangles {
        assert!(tri[0] < tri[1] && tri[0] < tri[2]);
    }
    let keys: Vec<(u16, [u32; 3])> = blob
        .faces
        .iter()
        .copied()
        .zip(blob.triangles.iter().copied())
        .collect();
    assert!(keys.windows(2).all(|w| w[0] < w[1]));
    // Closed, one component, every vertex used.
    let topo = topology(&blob.triangles, blob.vertices.len());
    assert!(topo.closed, "result is not a closed 2-manifold");
    assert_eq!(topo.components, 1);
    assert_eq!(topo.vertices_used, blob.vertices.len());
    let euler =
        topo.vertices_used as i64 - topo.undirected_edges as i64 + blob.triangles.len() as i64;
    assert_eq!(euler, res.euler_characteristic());
    // Volume: independent f64 sum inside the certified enclosure (relative slack for the
    // f64 rounding of the independent sum itself).
    let volume = mesh_volume(&blob.vertices, &blob.triangles);
    let enc = res.volume_mm3();
    let slack = 1e-9 * volume.abs();
    assert!(volume > 0.0 && enc.lo - slack <= volume && volume <= enc.hi + slack);
    // Bounds are the exact vertex extents.
    for axis in 0..3 {
        let lo = blob
            .vertices
            .iter()
            .map(|p| p[axis])
            .fold(f64::INFINITY, f64::min);
        let hi = blob
            .vertices
            .iter()
            .map(|p| p[axis])
            .fold(f64::NEG_INFINITY, f64::max);
        assert_eq!(res.bounds().min[axis], lo);
        assert_eq!(res.bounds().max[axis], hi);
    }
    // Face table: one id and one normal per face, every face used, normals outward.
    let ids = res.face_table();
    assert_eq!(ids.len(), blob.face_count as usize);
    assert_eq!(res.face_normals().len(), ids.len());
    let mut sorted: Vec<&String> = ids.iter().collect();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len(), "face ids are unique");
    for id in ids {
        assert!(id.len() <= 22);
        assert!(id.bytes().all(|b| b.is_ascii_lowercase()
            || b.is_ascii_digit()
            || matches!(b, b':' | b'.' | b'-')));
        assert_eq!(id.matches(':').count(), 1);
    }
    for (face, normal) in res.face_normals().iter().enumerate() {
        let used = blob
            .faces
            .iter()
            .filter(|f| usize::from(**f) == face)
            .count();
        assert!(used > 0, "face {face} has no triangle");
        let Some(n) = normal else { continue };
        let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        assert!((length - 1.0).abs() < 1e-12, "unit normal");
        // A face's area vector must point along its normal. A sliver triangle on a rounded
        // cap can carry rounding-dominated direction, so single triangles are only held to the
        // normal when they are not negligible next to the face.
        let mut sum = [0.0f64; 3];
        let mut cross: Vec<[f64; 3]> = Vec::new();
        for (tri, f) in blob.triangles.iter().zip(&blob.faces) {
            if usize::from(*f) != face {
                continue;
            }
            let [a, b, c] = tri.map(|i| blob.vertices[i as usize]);
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let w = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let x = [
                u[1] * w[2] - u[2] * w[1],
                u[2] * w[0] - u[0] * w[2],
                u[0] * w[1] - u[1] * w[0],
            ];
            for (s, v) in sum.iter_mut().zip(x) {
                *s += v;
            }
            cross.push(x);
        }
        let total = (sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2]).sqrt();
        assert!(
            sum[0] * n[0] + sum[1] * n[1] + sum[2] * n[2] > 0.999_999 * total,
            "face {face} not outward"
        );
        for x in cross {
            let dot = x[0] * n[0] + x[1] * n[1] + x[2] * n[2];
            let norm = (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt();
            if norm > 1e-6 * total {
                assert!(dot > 0.99 * norm, "triangle of face {face} not outward");
            }
        }
    }
    Facts {
        vertices: blob.vertices.len(),
        edges: topo.undirected_edges,
        triangles: blob.triangles.len(),
        euler,
        volume,
    }
}

/// Counts cell centres of a grid inside a predicate; the independent voxel volume.
pub fn voxel_volume(
    lo: [f64; 3],
    hi: [f64; 3],
    cells: usize,
    inside: impl Fn([f64; 3]) -> bool,
) -> f64 {
    let h = [
        (hi[0] - lo[0]) / cells as f64,
        (hi[1] - lo[1]) / cells as f64,
        (hi[2] - lo[2]) / cells as f64,
    ];
    let mut count = 0u64;
    for i in 0..cells {
        for j in 0..cells {
            for k in 0..cells {
                let p = [
                    lo[0] + (i as f64 + 0.5) * h[0],
                    lo[1] + (j as f64 + 0.5) * h[1],
                    lo[2] + (k as f64 + 0.5) * h[2],
                ];
                if inside(p) {
                    count += 1;
                }
            }
        }
    }
    count as f64 * h[0] * h[1] * h[2]
}

/// Even-odd point-in-polygon in the XY plane (independent of the kernel's exact predicates).
pub fn in_polygon(poly: &[[f64; 2]], p: [f64; 2]) -> bool {
    let mut inside = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0])
        {
            inside = !inside;
        }
    }
    inside
}
