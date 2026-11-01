// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Independent readers and oracles for the G05 tests.
//!
//! Nothing here calls production math: areas, volumes, topology and the blob layout are
//! recomputed from the public buffers by separate scalar code and closed-form formulas.

#![allow(
    dead_code,
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used
)]

use beamcraft_kernel::profile::{Cavity, Extrusion, Point2};
use std::collections::{BTreeMap, BTreeSet};

pub fn p(x: f64, y: f64) -> Point2 {
    Point2 { x, y }
}

/// Counter-clockwise axis-aligned rectangle.
pub fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<Point2> {
    vec![p(x0, y0), p(x1, y0), p(x1, y1), p(x0, y1)]
}

/// Clockwise axis-aligned rectangle (a cavity).
pub fn hole_rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Cavity {
    Cavity::polygon(vec![p(x0, y0), p(x0, y1), p(x1, y1), p(x1, y0)])
}

pub fn shoelace(points: &[Point2]) -> f64 {
    // Relative to the first point: absolute coordinates near 1e9 would cancel catastrophically.
    let o = points[0];
    let n = points.len();
    let mut sum = 0.0;
    for i in 0..n {
        let (a, b) = (points[i], points[(i + 1) % n]);
        sum += (a.x - o.x) * (b.y - o.y) - (b.x - o.x) * (a.y - o.y);
    }
    0.5 * sum
}

/// Net area (outer minus cavity areas), from signed areas.
pub fn net_area(outer: &[Point2], cavities: &[Cavity]) -> f64 {
    shoelace(outer) + cavities.iter().map(|c| shoelace(c.points())).sum::<f64>()
}

/// Area of a regular n-gon with vertices on a circle of radius r (closed form).
pub fn regular_polygon_area(n: u32, r: f64) -> f64 {
    0.5 * f64::from(n) * r * r * (std::f64::consts::TAU / f64::from(n)).sin()
}

pub struct Blob {
    pub vertices: Vec<[f64; 3]>,
    pub triangles: Vec<[u32; 3]>,
    pub faces: Vec<u16>,
    pub face_count: u32,
    pub space: u32,
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

/// Reads a BCMESH01 blob by the F06 section 6 table only, checking every structural rule.
pub fn parse_blob(bytes: &[u8]) -> Blob {
    assert_eq!(&bytes[0..8], b"BCMESH01");
    assert_eq!(u32_at(bytes, 8), 1, "layout version");
    let v = u32_at(bytes, 12) as usize;
    let t = u32_at(bytes, 16) as usize;
    let k = u32_at(bytes, 20);
    let space = u32_at(bytes, 24);
    assert_eq!(u32_at(bytes, 28), 0, "reserved");
    let pad8 = |n: usize| n.div_ceil(8) * 8;
    assert_eq!(
        bytes.len(),
        32 + 24 * v + pad8(12 * t) + pad8(2 * t),
        "total length"
    );
    let mut at = 32;
    let mut vertices = Vec::new();
    for _ in 0..v {
        let mut c = [0.0f64; 3];
        for slot in &mut c {
            let raw = bytes[at..at + 8].try_into().unwrap();
            *slot = f64::from_le_bytes(raw);
            assert!(slot.is_finite());
            assert!(
                !(*slot == 0.0 && slot.is_sign_negative()),
                "negative zero in blob"
            );
            at += 8;
        }
        vertices.push(c);
    }
    let mut triangles = Vec::new();
    for _ in 0..t {
        triangles.push([
            u32_at(bytes, at),
            u32_at(bytes, at + 4),
            u32_at(bytes, at + 8),
        ]);
        at += 12;
    }
    let pad = pad8(12 * t) - 12 * t;
    assert!(bytes[at..at + pad].iter().all(|b| *b == 0), "index padding");
    at += pad;
    let mut faces = Vec::new();
    for _ in 0..t {
        faces.push(u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap()));
        at += 2;
    }
    assert!(bytes[at..].iter().all(|b| *b == 0), "face padding");
    for tri in &triangles {
        assert!(tri.iter().all(|i| (*i as usize) < v));
    }
    assert!(faces.iter().all(|f| u32::from(*f) < k));
    Blob {
        vertices,
        triangles,
        faces,
        face_count: k,
        space,
    }
}

pub struct Topology {
    pub vertices_used: usize,
    pub undirected_edges: usize,
    pub closed: bool,
    pub components: usize,
}

/// Independent closed 2-manifold and connectivity check from the index table only.
pub fn topology(triangles: &[[u32; 3]], vertex_count: usize) -> Topology {
    let mut directed: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    for [a, b, c] in triangles {
        for e in [(*a, *b), (*b, *c), (*c, *a)] {
            *directed.entry(e).or_default() += 1;
        }
    }
    let closed = directed
        .iter()
        .all(|(&(a, b), &n)| n == 1 && directed.get(&(b, a)) == Some(&1));
    let undirected: BTreeSet<(u32, u32)> = directed
        .keys()
        .map(|&(a, b)| (a.min(b), a.max(b)))
        .collect();
    let mut adjacent: Vec<Vec<usize>> = vec![Vec::new(); vertex_count];
    for &(a, b) in &undirected {
        adjacent[a as usize].push(b as usize);
        adjacent[b as usize].push(a as usize);
    }
    let used: BTreeSet<u32> = triangles.iter().flatten().copied().collect();
    let mut seen = vec![false; vertex_count];
    let mut components = 0;
    for start in &used {
        if seen[*start as usize] {
            continue;
        }
        components += 1;
        let mut stack = vec![*start as usize];
        seen[*start as usize] = true;
        while let Some(v) = stack.pop() {
            for &w in &adjacent[v] {
                if !seen[w] {
                    seen[w] = true;
                    stack.push(w);
                }
            }
        }
    }
    Topology {
        vertices_used: used.len(),
        undirected_edges: undirected.len(),
        closed,
        components,
    }
}

/// Divergence-theorem volume about the first vertex, `(1/6) * sum(det)`.
pub fn mesh_volume(vertices: &[[f64; 3]], triangles: &[[u32; 3]]) -> f64 {
    let r = vertices[0];
    let mut sum = 0.0;
    for t in triangles {
        let [a, b, c] = t.map(|i| {
            let v = vertices[i as usize];
            [v[0] - r[0], v[1] - r[1], v[2] - r[2]]
        });
        let cross = [
            b[1] * c[2] - b[2] * c[1],
            b[2] * c[0] - b[0] * c[2],
            b[0] * c[1] - b[1] * c[0],
        ];
        sum += a[0] * cross[0] + a[1] * cross[1] + a[2] * cross[2];
    }
    sum / 6.0
}

/// CR-TOL-001-style relative comparison used only by tests (never by the kernel).
pub fn close(a: f64, b: f64, rel: f64) -> bool {
    (a - b).abs() <= rel * a.abs().max(b.abs()) + 1e-12
}

/// Checks every structural promise of a successful extrusion against independent oracles.
pub fn assert_valid_extrusion(ex: &Extrusion, outer: &[Point2], cavities: &[Cavity], t: f64) {
    let mesh = ex.mesh();
    let n = outer.len() + cavities.iter().map(|c| c.points().len()).sum::<usize>();
    let blob = parse_blob(mesh.blob());
    assert_eq!(blob.space, 0);
    assert_eq!(blob.vertices.len(), 2 * n);
    assert_eq!(blob.vertices.len(), mesh.vertices().len());
    assert_eq!(blob.triangles.as_slice(), mesh.triangles());
    assert_eq!(blob.faces.as_slice(), mesh.triangle_faces());
    assert_eq!(blob.face_count as usize, mesh.face_table().len());
    assert_eq!(mesh.subdivision_scheme(), "none");
    assert_eq!(mesh.orientation(), "rightHanded");
    // Vertex layout: bottom copies, then top copies at z = t (negative zero normalized).
    let mut points: Vec<Point2> = outer.to_vec();
    for c in cavities {
        points.extend_from_slice(c.points());
    }
    for (i, q) in points.iter().enumerate() {
        assert_eq!(blob.vertices[i], [q.x + 0.0, q.y + 0.0, 0.0]);
        assert_eq!(blob.vertices[n + i], [q.x + 0.0, q.y + 0.0, t]);
    }
    let h = cavities.len();
    let topo = topology(&blob.triangles, blob.vertices.len());
    assert!(topo.closed, "directed edges are not paired");
    assert_eq!(topo.vertices_used, 2 * n);
    assert_eq!(topo.components, 1);
    let (v, e, f) = (2 * n, topo.undirected_edges, blob.triangles.len());
    assert_eq!(
        v as i64 - e as i64 + f as i64,
        2 - 2 * h as i64,
        "Euler for genus = cavities"
    );
    assert_eq!(f, 4 * n + 4 * h - 4, "triangle count");
    let area = net_area(outer, cavities);
    let expect = area * t;
    assert!(
        close(ex.volume_mm3(), expect, 1e-9),
        "analytic {} vs {}",
        ex.volume_mm3(),
        expect
    );
    assert!(close(ex.net_area_mm2(), area, 1e-9));
    assert!(close(ex.volume_m3(), expect / 1e9, 1e-9));
    let vm = mesh_volume(&blob.vertices, &blob.triangles);
    assert!(
        vm > 0.0 && close(vm, expect, 1e-6),
        "mesh volume {vm} vs {expect}"
    );
    // Outward winding of every cap and wall triangle.
    for (tri, face) in blob.triangles.iter().zip(&blob.faces) {
        let [a, b, c] = tri.map(|i| blob.vertices[i as usize]);
        let cross = [
            (b[1] - a[1]) * (c[2] - a[2]) - (b[2] - a[2]) * (c[1] - a[1]),
            (b[2] - a[2]) * (c[0] - a[0]) - (b[0] - a[0]) * (c[2] - a[2]),
            (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]),
        ];
        match (*face, mesh.face_normals()[*face as usize]) {
            (0, Some(nm)) => {
                assert_eq!((nm.x(), nm.y(), nm.z()), (0.0, 0.0, -1.0));
                assert!(cross[2] < 0.0 && a[2] == 0.0 && b[2] == 0.0 && c[2] == 0.0);
            }
            (1, Some(nm)) => {
                assert_eq!((nm.x(), nm.y(), nm.z()), (0.0, 0.0, 1.0));
                assert!(cross[2] > 0.0 && a[2] == t && b[2] == t && c[2] == t);
            }
            (_, Some(nm)) => {
                let dot = cross[0] * nm.x() + cross[1] * nm.y() + cross[2] * nm.z();
                assert!(dot > 0.0, "wall triangle faces inward");
                assert!((nm.x() * nm.x() + nm.y() * nm.y() + nm.z() * nm.z() - 1.0).abs() < 1e-12);
            }
            (_, None) => {}
        }
    }
    // Feature ids: charset, length, uniqueness, table sizes.
    let mut all = BTreeSet::new();
    for id in mesh
        .face_table()
        .iter()
        .chain(mesh.edge_table())
        .chain(mesh.vertex_table())
    {
        assert!(id.len() <= 64, "id too long");
        assert!(
            id.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b":.-".contains(&b)),
            "bad id {id}"
        );
        assert!(id.starts_with("extrusion:"));
        assert!(all.insert(id.clone()), "duplicate id {id}");
    }
    assert_eq!(mesh.vertex_table().len(), 2 * n);
    assert_eq!(mesh.edge_table().len(), 3 * n);
    assert_eq!(mesh.edge_vertices().len(), 3 * n);
    assert_eq!(mesh.face_roles().len(), mesh.face_table().len());
    assert_eq!(mesh.face_normals().len(), mesh.face_table().len());
    for face in 0..mesh.face_table().len() {
        assert!(
            blob.faces.iter().any(|f| *f as usize == face),
            "face {face} has no triangle"
        );
    }
    // An edge starts at the vertex named like it: `e.z0.o3` starts at `v.z0.o3`, `e.v.o3`
    // joins `v.z0.o3` to `v.z1.o3`.
    for (id, [a, b]) in mesh.edge_table().iter().zip(mesh.edge_vertices()) {
        let (va, vb) = (
            &mesh.vertex_table()[*a as usize],
            &mesh.vertex_table()[*b as usize],
        );
        let tail = id.strip_prefix("extrusion:e.").unwrap();
        if let Some(rest) = tail.strip_prefix("v.") {
            assert_eq!(va, &format!("extrusion:v.z0.{rest}"));
            assert_eq!(vb, &format!("extrusion:v.z1.{rest}"));
        } else {
            assert_eq!(va, &format!("extrusion:v.{tail}"));
            let level = &tail[..2];
            assert!(vb.starts_with(&format!("extrusion:v.{level}.")));
        }
    }
}

pub struct Rng(pub u64);

impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}
