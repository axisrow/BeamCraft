// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Shared synthetic-mesh generators and independent scalar oracles for G06 tests.
//!
//! Nothing here calls production geometry code: expected values come from voxel
//! counting, closed-form box/prism formulas, plain f64 loops and `std` collections.

#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used
)]

use std::collections::{BTreeMap, BTreeSet};

pub type Verts = Vec<[f64; 3]>;
pub type Tris = Vec<[u32; 3]>;

/// A mesh as plain buffers.
#[derive(Clone, Debug)]
pub struct Buf {
    pub v: Verts,
    pub t: Tris,
}

/// Face corner offsets per outward direction, counter-clockwise seen from outside.
const FACES: [([i32; 3], [[i32; 3]; 4]); 6] = [
    ([1, 0, 0], [[1, 0, 0], [1, 1, 0], [1, 1, 1], [1, 0, 1]]),
    ([-1, 0, 0], [[0, 0, 0], [0, 0, 1], [0, 1, 1], [0, 1, 0]]),
    ([0, 1, 0], [[0, 1, 0], [0, 1, 1], [1, 1, 1], [1, 1, 0]]),
    ([0, -1, 0], [[0, 0, 0], [1, 0, 0], [1, 0, 1], [0, 0, 1]]),
    ([0, 0, 1], [[0, 0, 1], [1, 0, 1], [1, 1, 1], [0, 1, 1]]),
    ([0, 0, -1], [[0, 0, 0], [0, 1, 0], [1, 1, 0], [1, 0, 0]]),
];

/// Surface of a set of unit voxels, scaled and offset; shared corners share indices.
pub fn voxels(cells: &[[i32; 3]], scale: f64, offset: [f64; 3]) -> Buf {
    let set: BTreeSet<[i32; 3]> = cells.iter().copied().collect();
    let mut index: BTreeMap<[i32; 3], u32> = BTreeMap::new();
    let mut v: Verts = Vec::new();
    let mut t: Tris = Vec::new();
    for c in &set {
        for (dir, quad) in FACES {
            let n = [c[0] + dir[0], c[1] + dir[1], c[2] + dir[2]];
            if set.contains(&n) {
                continue;
            }
            let ids: Vec<u32> = quad
                .iter()
                .map(|o| {
                    let key = [c[0] + o[0], c[1] + o[1], c[2] + o[2]];
                    *index.entry(key).or_insert_with(|| {
                        v.push([
                            f64::from(key[0]) * scale + offset[0],
                            f64::from(key[1]) * scale + offset[1],
                            f64::from(key[2]) * scale + offset[2],
                        ]);
                        (v.len() - 1) as u32
                    })
                })
                .collect();
            t.push([ids[0], ids[1], ids[2]]);
            t.push([ids[0], ids[2], ids[3]]);
        }
    }
    Buf { v, t }
}

/// Exposed unit faces of a voxel set (surface area in unit squares).
pub fn exposed_faces(cells: &[[i32; 3]]) -> usize {
    let set: BTreeSet<[i32; 3]> = cells.iter().copied().collect();
    set.iter()
        .map(|c| {
            FACES
                .iter()
                .filter(|(d, _)| !set.contains(&[c[0] + d[0], c[1] + d[1], c[2] + d[2]]))
                .count()
        })
        .sum()
}

/// An axis-aligned box as the F06 12-triangle mesh (vertex `i` = bits x, y, z).
pub fn cuboid(lo: [f64; 3], hi: [f64; 3]) -> Buf {
    let v: Verts = (0..8)
        .map(|i| {
            [
                if i & 1 == 0 { lo[0] } else { hi[0] },
                if i & 2 == 0 { lo[1] } else { hi[1] },
                if i & 4 == 0 { lo[2] } else { hi[2] },
            ]
        })
        .collect();
    let t: Tris = [
        [0, 4, 6],
        [0, 6, 2],
        [1, 3, 7],
        [1, 7, 5],
        [0, 1, 5],
        [0, 5, 4],
        [2, 6, 7],
        [2, 7, 3],
        [0, 2, 3],
        [0, 3, 1],
        [4, 5, 7],
        [4, 7, 6],
    ]
    .to_vec();
    Buf { v, t }
}

/// Right triangular prism: legs `a`, `b` in the XY plane, height `h` in Z.
pub fn prism(a: f64, b: f64, h: f64) -> Buf {
    let v = vec![
        [0.0, 0.0, 0.0],
        [a, 0.0, 0.0],
        [0.0, b, 0.0],
        [0.0, 0.0, h],
        [a, 0.0, h],
        [0.0, b, h],
    ];
    let t = vec![
        [0, 2, 1],
        [3, 4, 5],
        [0, 1, 4],
        [0, 4, 3],
        [0, 3, 5],
        [0, 5, 2],
        [1, 2, 5],
        [1, 5, 4],
    ];
    Buf { v, t }
}

/// Appends `other` to `base` (disjoint index ranges).
pub fn merge(base: &Buf, other: &Buf) -> Buf {
    let shift = base.v.len() as u32;
    let mut v = base.v.clone();
    v.extend(other.v.iter().copied());
    let mut t = base.t.clone();
    t.extend(
        other
            .t
            .iter()
            .map(|[a, b, c]| [a + shift, b + shift, c + shift]),
    );
    Buf { v, t }
}

/// Reverses every triangle's winding.
pub fn inverted(m: &Buf) -> Buf {
    Buf {
        v: m.v.clone(),
        t: m.t.iter().map(|[a, b, c]| [*a, *c, *b]).collect(),
    }
}

/// Independent signed volume: sum of tetrahedra against the first vertex, in f64.
pub fn signed_volume(v: &Verts, t: &Tris) -> f64 {
    let o = v[0];
    let mut sum = 0.0;
    for [a, b, c] in t {
        let p = |i: u32| {
            let q = v[i as usize];
            [q[0] - o[0], q[1] - o[1], q[2] - o[2]]
        };
        let (a, b, c) = (p(*a), p(*b), p(*c));
        sum += a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
            + a[2] * (b[0] * c[1] - b[1] * c[0]);
    }
    sum / 6.0
}

/// Independent surface area in f64.
pub fn area(v: &Verts, t: &Tris) -> f64 {
    t.iter()
        .map(|[a, b, c]| {
            let (a, b, c) = (v[*a as usize], v[*b as usize], v[*c as usize]);
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let w = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let x = [
                u[1] * w[2] - u[2] * w[1],
                u[2] * w[0] - u[0] * w[2],
                u[0] * w[1] - u[1] * w[0],
            ];
            0.5 * (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt()
        })
        .sum()
}

/// Edge census: undirected edge -> (forward uses, backward uses).
pub fn census(t: &Tris) -> BTreeMap<(u32, u32), (u32, u32)> {
    let mut map: BTreeMap<(u32, u32), (u32, u32)> = BTreeMap::new();
    for [a, b, c] in t {
        for (u, w) in [(*a, *b), (*b, *c), (*c, *a)] {
            let e = map.entry((u.min(w), u.max(w))).or_insert((0, 0));
            if u < w {
                e.0 += 1;
            } else {
                e.1 += 1;
            }
        }
    }
    map
}

/// Independent combinatorial verdict for a buffer without degenerate triangles.
///
/// Returns `"closed_manifold"`, `"open"`, `"non_manifold"` or `"inconsistent"`
/// following the documented precedence (non-manifold, open, inconsistent).
pub fn topology_verdict(t: &Tris) -> &'static str {
    let c = census(t);
    if c.values().any(|(f, b)| f + b >= 3) {
        return "non_manifold";
    }
    let mut around: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for (i, tri) in t.iter().enumerate() {
        for v in tri {
            around.entry(*v).or_default().push(i);
        }
    }
    for tris in around.values() {
        let share = |x: usize, y: usize| t[x].iter().filter(|a| t[y].contains(a)).count() >= 2;
        let mut seen = vec![false; tris.len()];
        let mut stack = vec![0usize];
        seen[0] = true;
        while let Some(i) = stack.pop() {
            for j in 0..tris.len() {
                if !seen[j] && share(tris[i], tris[j]) {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
        if seen.iter().any(|s| !s) {
            return "non_manifold";
        }
    }
    if c.values().any(|(f, b)| f + b == 1) {
        return "open";
    }
    if c.values().any(|(f, b)| *f != 1 || *b != 1) {
        return "inconsistent";
    }
    "closed_manifold"
}

/// Connected components by shared vertices, as lists of triangle indices.
pub fn components(t: &Tris) -> Vec<Vec<usize>> {
    fn find(p: &mut BTreeMap<u32, u32>, x: u32) -> u32 {
        let q = *p.entry(x).or_insert(x);
        if q == x {
            x
        } else {
            let r = find(p, q);
            p.insert(x, r);
            r
        }
    }
    let mut parent: BTreeMap<u32, u32> = BTreeMap::new();
    for [a, b, c] in t {
        for (x, y) in [(*a, *b), (*b, *c)] {
            let (rx, ry) = (find(&mut parent, x), find(&mut parent, y));
            parent.insert(rx, ry);
        }
    }
    let mut groups: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for (i, tri) in t.iter().enumerate() {
        let r = find(&mut parent, tri[0]);
        groups.entry(r).or_default().push(i);
    }
    let mut out: Vec<Vec<usize>> = groups.into_values().collect();
    out.sort_by_key(|g| g[0]);
    out
}

/// Tiny deterministic xorshift generator for seeded tests.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    pub fn chance(&mut self, num: u64, den: u64) -> bool {
        self.below(den) < num
    }
}

/// Permutes triangles, relabels vertices and rotates each triangle's start corner.
pub fn shuffled(m: &Buf, rng: &mut Rng) -> Buf {
    let n = m.v.len();
    let mut perm: Vec<usize> = (0..n).collect();
    for i in (1..n).rev() {
        perm.swap(i, rng.below(i as u64 + 1) as usize);
    }
    let mut v = vec![[0.0; 3]; n];
    for (old, &new) in perm.iter().enumerate() {
        v[new] = m.v[old];
    }
    let mut t: Tris =
        m.t.iter()
            .map(|[a, b, c]| {
                let mut tri = [
                    perm[*a as usize] as u32,
                    perm[*b as usize] as u32,
                    perm[*c as usize] as u32,
                ];
                tri.rotate_left(rng.below(3) as usize);
                tri
            })
            .collect();
    for i in (1..t.len()).rev() {
        t.swap(i, rng.below(i as u64 + 1) as usize);
    }
    Buf { v, t }
}

/// Little-endian BCMESH01 blob written independently of the production encoder.
pub fn blob(v: &Verts, t: &Tris, faces: &[u16], k: u32, space: u32) -> Vec<u8> {
    let pad8 = |n: usize| n.div_ceil(8) * 8;
    let mut out = b"BCMESH01".to_vec();
    for word in [1u32, v.len() as u32, t.len() as u32, k, space, 0] {
        out.extend(word.to_le_bytes());
    }
    for p in v {
        for x in p {
            out.extend(x.to_le_bytes());
        }
    }
    for tri in t {
        for i in tri {
            out.extend(i.to_le_bytes());
        }
    }
    out.resize(32 + 24 * v.len() + pad8(12 * t.len()), 0);
    for f in faces {
        out.extend(f.to_le_bytes());
    }
    out.resize(
        32 + 24 * v.len() + pad8(12 * t.len()) + pad8(2 * t.len()),
        0,
    );
    out
}
