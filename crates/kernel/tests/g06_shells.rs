// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! G06 independent checks of shell certification, orientation, topology and limits.
//!
//! Expected values come from voxel counting, closed-form formulas and the scalar
//! oracles in `common`; none call production geometry or topology code.

#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

use crate::common_mesh as common;

use beamcraft_kernel::diagnostic::Stage;
use beamcraft_kernel::topology::{
    Limits, Orientation, PointClass, analyze, analyze_with, classify_point,
};
use beamcraft_kernel::trimesh::{
    MAX_BLOB_BYTES, MAX_COORDINATE_MM, MAX_FACE_TABLE, MAX_TRIANGLES, MAX_VERTICES, TriMesh,
};
use beamcraft_kernel::verdict::{Fault, MeshCode, ShellStatus, Uncertainty};
use beamcraft_model::error::ErrorCode;
use common::*;

fn mesh(b: &Buf) -> TriMesh {
    TriMesh::from_buffers(&b.v, &b.t).unwrap()
}

fn status(b: &Buf) -> ShellStatus {
    analyze(&mesh(b)).unwrap().status()
}

fn ring(w: i32, d: i32, holes: &[[i32; 2]]) -> Vec<[i32; 3]> {
    let mut cells = Vec::new();
    for x in 0..w {
        for y in 0..d {
            if !holes.contains(&[x, y]) {
                cells.push([x, y, 0]);
            }
        }
    }
    cells
}

// ---------------------------------------------------------------- positive

#[test]
fn box_is_certified_with_closed_form_counts_and_volume() {
    let b = cuboid([0.0; 3], [10.0, 20.0, 5.0]);
    let a = analyze(&mesh(&b)).unwrap();
    assert_eq!(a.status(), ShellStatus::Certified);
    assert!(a.is_certified() && a.code().is_none());
    assert_eq!(
        (a.vertex_count(), a.triangle_count(), a.edge_count()),
        (8, 12, 18)
    );
    // V - E + F = 8 - 18 + 12 = 2 for one genus-0 shell.
    assert_eq!(a.euler_characteristic(), 2);
    let [shell] = a.shells() else {
        panic!("one shell")
    };
    assert_eq!(shell.genus, Some(0));
    assert!(shell.closed);
    assert_eq!(shell.orientation, Orientation::Outward);
    assert!(shell.signed_volume_mm3.unwrap().contains(10.0 * 20.0 * 5.0));
    assert_eq!(a.defects().boundary_edges, 0);
    assert_eq!(a.bounds().min, [0.0; 3]);
    assert_eq!(a.bounds().max, [10.0, 20.0, 5.0]);
}

#[test]
fn prism_is_certified_with_euler_two() {
    let a = analyze(&mesh(&prism(3.0, 4.0, 5.0))).unwrap();
    assert_eq!(a.status(), ShellStatus::Certified);
    // 6 vertices, 12 edges, 8 triangles: 6 - 12 + 8 = 2; volume = (3*4/2)*5 = 30.
    assert_eq!(
        (a.vertex_count(), a.edge_count(), a.triangle_count()),
        (6, 12, 8)
    );
    assert_eq!(a.euler_characteristic(), 2);
    assert!(a.shells()[0].signed_volume_mm3.unwrap().contains(30.0));
}

#[test]
fn box_with_a_tunnel_has_genus_one_by_euler() {
    // A 3x3x1 slab with the middle cell removed: one tunnel, genus 1.
    let cells = ring(3, 3, &[[1, 1]]);
    let b = voxels(&cells, 1.0, [0.0; 3]);
    let a = analyze(&mesh(&b)).unwrap();
    assert_eq!(a.status(), ShellStatus::Certified);
    let [shell] = a.shells() else {
        panic!("one shell")
    };
    assert_eq!(shell.genus, Some(1));
    assert_eq!(shell.euler_characteristic, 0);
    assert!(shell.signed_volume_mm3.unwrap().contains(8.0));
    // Independent Euler count from the buffer itself.
    let edges = census(&b.t).len() as i64;
    assert_eq!(b.v.len() as i64 - edges + b.t.len() as i64, 0);
    // Two tunnels: genus 2, characteristic -2.
    let two = voxels(&ring(5, 3, &[[1, 1], [3, 1]]), 1.0, [0.0; 3]);
    let a = analyze(&mesh(&two)).unwrap();
    assert_eq!(a.status(), ShellStatus::Certified);
    assert_eq!(a.shells()[0].genus, Some(2));
    assert_eq!(a.euler_characteristic(), -2);
}

#[test]
fn disjoint_shells_are_certified_with_euler_four() {
    let b = merge(
        &cuboid([0.0; 3], [2.0, 2.0, 2.0]),
        &cuboid([5.0, 0.0, 0.0], [8.0, 3.0, 4.0]),
    );
    let a = analyze(&mesh(&b)).unwrap();
    assert_eq!(a.status(), ShellStatus::Certified);
    assert_eq!(a.shells().len(), 2);
    assert_eq!(a.euler_characteristic(), 4);
    assert!(a.shells()[0].signed_volume_mm3.unwrap().contains(8.0));
    assert!(a.shells()[1].signed_volume_mm3.unwrap().contains(36.0));
    assert_eq!(a.triangle_shells().iter().filter(|s| **s == 1).count(), 12);
}

#[test]
fn sixteen_shells_are_accepted_and_seventeen_hit_the_topology_cap() {
    let boxes = |n: usize| {
        (0..n)
            .map(|i| cuboid([3.0 * i as f64, 0.0, 0.0], [3.0 * i as f64 + 2.0, 2.0, 2.0]))
            .reduce(|a, b| merge(&a, &b))
            .unwrap()
    };
    let a = analyze(&mesh(&boxes(16))).unwrap();
    assert_eq!((a.status(), a.shells().len()), (ShellStatus::Certified, 16));
    let e = analyze(&mesh(&boxes(17))).unwrap_err();
    assert_eq!((e.code, e.fault), (MeshCode::TopologyLimit, Fault::Shells));
    assert_eq!(e.code.f04_code(), ErrorCode::LimitExceeded);
}

#[test]
fn relabelled_and_reordered_copies_classify_identically() {
    let sources = [
        voxels(&ring(3, 3, &[[1, 1]]), 1.0, [0.0; 3]),
        merge(
            &cuboid([0.0; 3], [2.0; 3]),
            &cuboid([4.0, 0.0, 0.0], [6.0, 3.0, 1.0]),
        ),
        prism(3.0, 4.0, 5.0),
    ];
    let mut rng = Rng(0x5eed_0001);
    for src in &sources {
        let base = analyze(&mesh(src)).unwrap();
        for _ in 0..6 {
            let s = shuffled(src, &mut rng);
            let a = analyze(&mesh(&s)).unwrap();
            assert_eq!(a.status(), base.status());
            assert_eq!(a.defects(), base.defects());
            assert_eq!(a.euler_characteristic(), base.euler_characteristic());
            assert_eq!(a.edge_count(), base.edge_count());
            let (mut x, mut y): (Vec<_>, Vec<_>) = (
                a.shells().iter().map(|s| (s.triangles, s.genus)).collect(),
                base.shells()
                    .iter()
                    .map(|s| (s.triangles, s.genus))
                    .collect(),
            );
            x.sort();
            y.sort();
            assert_eq!(x, y);
            // Exact sums make the volume enclosure independent of order.
            let mut vx: Vec<_> = a
                .shells()
                .iter()
                .map(|s| s.signed_volume_mm3.unwrap().lo.to_bits())
                .collect();
            let mut vy: Vec<_> = base
                .shells()
                .iter()
                .map(|s| s.signed_volume_mm3.unwrap().lo.to_bits())
                .collect();
            vx.sort_unstable();
            vy.sort_unstable();
            assert_eq!(vx, vy);
        }
    }
}

#[test]
fn repeat_analysis_is_identical_and_blobs_round_trip() {
    let b = voxels(&ring(4, 3, &[[1, 1]]), 0.25, [3.5, -2.0, 7.0]);
    let m = mesh(&b);
    let (a1, a2) = (analyze(&m).unwrap(), analyze(&m).unwrap());
    assert_eq!(format!("{a1:?}"), format!("{a2:?}"));
    let bytes = m.to_blob();
    assert_eq!(bytes, mesh(&b).to_blob());
    let again = TriMesh::from_blob(&bytes).unwrap();
    assert_eq!(again, m);
    assert_eq!(again.to_blob(), bytes);
    // The blob equals the independent writer's bytes.
    assert_eq!(bytes, blob(&b.v, &b.t, &vec![0; b.t.len()], 1, 0));
}

#[test]
fn seeded_voxel_sets_match_an_independent_verdict() {
    let mut rng = Rng(0xc0ff_ee11);
    let (mut certified, mut manifold_negative, mut non_manifold) = (0, 0, 0);
    for _ in 0..120 {
        let mut cells = Vec::new();
        for x in 0..3 {
            for y in 0..3 {
                for z in 0..3 {
                    if rng.chance(17, 20) {
                        cells.push([x, y, z]);
                    }
                }
            }
        }
        if cells.is_empty() {
            continue;
        }
        let b = voxels(&cells, 1.0, [0.0; 3]);
        let Ok(m) = TriMesh::from_buffers(&b.v, &b.t) else {
            continue;
        };
        let result = analyze(&m);
        let comps = components(&b.t);
        if comps.len() > 16 {
            assert_eq!(result.unwrap_err().code, MeshCode::TopologyLimit);
            continue;
        }
        let a = result.unwrap();
        match topology_verdict(&b.t) {
            "non_manifold" => {
                non_manifold += 1;
                assert_eq!(a.status(), ShellStatus::NonManifold, "{cells:?}");
            }
            "closed_manifold" => {
                let negative = comps.iter().any(|c| {
                    let t: Tris = c.iter().map(|i| b.t[*i]).collect();
                    signed_volume(&b.v, &t) < 0.0
                });
                if negative {
                    manifold_negative += 1;
                    assert_eq!(a.status(), ShellStatus::Inward, "{cells:?}");
                } else {
                    certified += 1;
                    assert_eq!(a.status(), ShellStatus::Certified, "{cells:?}");
                    assert_eq!(a.shells().len(), comps.len());
                    let total: f64 = a
                        .shells()
                        .iter()
                        .map(|s| s.signed_volume_mm3.unwrap())
                        .map(|e| e.lo)
                        .sum();
                    assert!((total - cells.len() as f64).abs() < 1e-9);
                }
            }
            other => panic!("generator produced {other}"),
        }
    }
    assert!(
        certified > 20 && non_manifold > 2,
        "{certified} {non_manifold} {manifold_negative}"
    );
}

// ---------------------------------------------------------------- negative topology

#[test]
fn open_mesh_never_certifies() {
    // F06-NEG-18: remove triangle (0, 2, 3) of the plate table.
    let mut b = cuboid([0.0; 3], [10.0, 20.0, 1.0]);
    let removed = b.t.iter().position(|t| *t == [0, 2, 3]).unwrap();
    b.t.remove(removed);
    let a = analyze(&mesh(&b)).unwrap();
    assert_eq!(a.status(), ShellStatus::Open);
    assert_eq!(a.defects().boundary_edges, 3);
    assert_eq!(a.euler_characteristic(), 8 - 18 + 11);
    assert!(!a.is_certified());
    assert_eq!(a.shells()[0].orientation, Orientation::Unknown);
    assert!(a.shells()[0].signed_volume_mm3.is_none() && !a.shells()[0].closed);
    let code = a.code().unwrap();
    assert_eq!(
        (code, code.as_str()),
        (MeshCode::NotClosed, "GEO_MESH_NOT_CLOSED")
    );
    assert_eq!(code.f04_code(), ErrorCode::UnsupportedGeometry);
}

#[test]
fn duplicate_triangle_is_non_manifold() {
    let mut b = cuboid([0.0; 3], [1.0; 3]);
    b.t.push(b.t[3]);
    let a = analyze(&mesh(&b)).unwrap();
    assert_eq!(a.status(), ShellStatus::NonManifold);
    assert_eq!(a.defects().duplicate_triangles, 1);
    // Same winding makes three uses of each of its edges.
    assert_eq!(a.defects().non_manifold_edges, 3);
    // A reversed duplicate is also a duplicate and never closes the shell.
    let mut c = cuboid([0.0; 3], [1.0; 3]);
    let [x, y, z] = c.t[3];
    c.t.push([x, z, y]);
    assert_eq!(status(&c), ShellStatus::NonManifold);
}

#[test]
fn three_faces_on_an_edge_is_non_manifold() {
    let b = Buf {
        v: vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, -1.0, 0.0],
        ],
        t: vec![[0, 1, 2], [1, 0, 3], [0, 1, 4]],
    };
    let a = analyze(&mesh(&b)).unwrap();
    assert_eq!(a.status(), ShellStatus::NonManifold);
    assert_eq!(a.defects().non_manifold_edges, 1);
    assert_eq!(topology_verdict(&b.t), "non_manifold");
}

#[test]
fn boxes_touching_along_an_edge_or_at_a_vertex_are_non_manifold_or_overlapping() {
    // Two voxels sharing only an edge: that edge has four triangles.
    let edge = voxels(&[[0, 0, 0], [1, 1, 0]], 1.0, [0.0; 3]);
    assert_eq!(topology_verdict(&edge.t), "non_manifold");
    assert_eq!(status(&edge), ShellStatus::NonManifold);
    // Two voxels sharing only a corner: every edge is fine, the vertex is pinched.
    let corner = voxels(&[[0, 0, 0], [1, 1, 1]], 1.0, [0.0; 3]);
    assert_eq!(topology_verdict(&corner.t), "non_manifold");
    let a = analyze(&mesh(&corner)).unwrap();
    assert_eq!(a.status(), ShellStatus::NonManifold);
    assert_eq!(a.defects().non_manifold_vertices, 1);
    assert_eq!(a.defects().non_manifold_edges, 0);
}

#[test]
fn one_flipped_triangle_is_inconsistent_orientation() {
    for victim in [0, 5, 11] {
        let mut b = cuboid([0.0; 3], [4.0, 5.0, 6.0]);
        b.t[victim].swap(1, 2);
        let a = analyze(&mesh(&b)).unwrap();
        assert_eq!(a.status(), ShellStatus::InconsistentOrientation);
        assert_eq!(a.defects().inconsistent_edges, 3);
        assert_eq!(a.code(), Some(MeshCode::NotClosed));
        assert_eq!(topology_verdict(&b.t), "inconsistent");
    }
}

#[test]
fn fully_inverted_shell_is_inward_with_negative_signed_volume() {
    // F06-NEG-20: reversing every triangle keeps the edges paired and flips the sign.
    let b = inverted(&cuboid([0.0; 3], [100.0, 50.0, 10.0]));
    let a = analyze(&mesh(&b)).unwrap();
    assert_eq!(a.status(), ShellStatus::Inward);
    let s = &a.shells()[0];
    assert!(s.closed && s.orientation == Orientation::Inward);
    assert!(s.signed_volume_mm3.unwrap().contains(-50_000.0));
    assert_eq!(a.euler_characteristic(), 2);
    assert_eq!(a.code().unwrap().as_str(), "GEO_MESH_INWARD");
    assert_eq!(a.code().unwrap().f04_code(), ErrorCode::UnsupportedGeometry);
    // A cavity-like inward shell inside an outward one is refused as well.
    let outer = cuboid([0.0; 3], [10.0; 3]);
    let nested = merge(&outer, &inverted(&cuboid([2.0; 3], [4.0; 3])));
    assert_eq!(status(&nested), ShellStatus::Inward);
}

#[test]
fn degenerate_triangles_are_reported_not_certified() {
    // Repeated index.
    let mut b = cuboid([0.0; 3], [10.0; 3]);
    b.t.push([0, 0, 1]);
    let a = analyze(&mesh(&b)).unwrap();
    assert_eq!(a.status(), ShellStatus::Degenerate);
    assert_eq!(a.defects().collapsed_triangles, 1);
    // Exactly collinear corners, with distinct indices.
    let mut c = cuboid([0.0; 3], [10.0; 3]);
    c.v.push([5.0, 0.0, 0.0]);
    c.t.push([0, 8, 1]);
    let a = analyze(&mesh(&c)).unwrap();
    assert_eq!(a.status(), ShellStatus::Degenerate);
    assert_eq!(a.defects().zero_area_triangles, 1);
    // Unused vertex.
    let mut d = cuboid([0.0; 3], [10.0; 3]);
    d.v.push([3.0, 3.0, 3.0]);
    let a = analyze(&mesh(&d)).unwrap();
    assert_eq!(
        (a.status(), a.defects().unused_vertices),
        (ShellStatus::Degenerate, 1)
    );
    // A tetrahedron with one edge of 0.0005 mm, below eps_geom = 0.001 mm (strict).
    let tetra = Buf {
        v: vec![
            [0.0, 0.0, 0.0],
            [0.0005, 0.0, 0.0],
            [0.0, 10.0, 0.0],
            [0.0, 0.0, 10.0],
        ],
        t: vec![[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]],
    };
    let a = analyze(&mesh(&tetra)).unwrap();
    assert_eq!(a.status(), ShellStatus::Degenerate);
    assert!(a.defects().short_edge_triangles >= 2);
    assert_eq!(a.code(), Some(MeshCode::NotCertified));
    // The same tetrahedron with that edge exactly at the floor is sound.
    let ok = Buf {
        v: vec![
            [0.0, 0.0, 0.0],
            [0.001, 0.0, 0.0],
            [0.0, 10.0, 0.0],
            [0.0, 0.0, 10.0],
        ],
        t: tetra.t.clone(),
    };
    assert_eq!(status(&ok), ShellStatus::Certified);
}

// ---------------------------------------------------------------- overlap and uncertainty

#[test]
fn overlapping_closed_boxes_are_never_exact() {
    // F06-NEG-19: closed boxes whose divergence sum (2000) is not the union (1500).
    let b = merge(
        &cuboid([0.0; 3], [10.0; 3]),
        &cuboid([5.0, 0.0, 0.0], [15.0, 10.0, 10.0]),
    );
    assert_eq!(
        b.v.len() as i64 - census(&b.t).len() as i64 + b.t.len() as i64,
        4
    );
    assert!((signed_volume(&b.v, &b.t) - 2000.0).abs() < 1e-9);
    let a = analyze(&mesh(&b)).unwrap();
    assert_eq!(a.status(), ShellStatus::Overlapping);
    assert_eq!(a.code().unwrap().as_str(), "GEO_MESH_NOT_CERTIFIED");
    assert_eq!(a.code().unwrap().f04_code(), ErrorCode::UnsupportedGeometry);
    assert_eq!(a.euler_characteristic(), 4);
}

#[test]
fn contact_between_shells_is_overlap_and_gaps_are_not() {
    let a = cuboid([0.0; 3], [10.0; 3]);
    let touching_face = merge(&a, &cuboid([10.0, 0.0, 0.0], [20.0, 10.0, 10.0]));
    assert_eq!(status(&touching_face), ShellStatus::Overlapping);
    let touching_edge = merge(&a, &cuboid([10.0, 10.0, 0.0], [20.0, 20.0, 10.0]));
    assert_eq!(status(&touching_edge), ShellStatus::Overlapping);
    let touching_vertex = merge(&a, &cuboid([10.0, 10.0, 10.0], [20.0, 20.0, 20.0]));
    assert_eq!(status(&touching_vertex), ShellStatus::Overlapping);
    // One representable step apart is disjoint (exact predicates, no epsilon).
    let gap = f64::from_bits(10.0f64.to_bits() + 1);
    let apart = merge(&a, &cuboid([gap, 0.0, 0.0], [20.0, 10.0, 10.0]));
    assert_eq!(status(&apart), ShellStatus::Certified);
}

#[test]
fn nested_outward_shells_overlap_and_disjoint_nesting_probes_are_robust() {
    let outer = cuboid([0.0; 3], [10.0; 3]);
    // The inner box's vertices are not on any outer face diagonal.
    let inner = cuboid([2.0, 3.0, 4.0], [3.0, 5.0, 7.0]);
    assert_eq!(status(&merge(&outer, &inner)), ShellStatus::Overlapping);
    // Same box beside the outer one: bounding boxes overlap only if nested.
    let beside = cuboid([11.0, 3.0, 4.0], [12.0, 5.0, 7.0]);
    assert_eq!(status(&merge(&outer, &beside)), ShellStatus::Certified);
    // Inside the bounding box of an L-shaped solid but outside the solid.
    let l = voxels(&[[0, 0, 0], [1, 0, 0], [0, 1, 0]], 1.0, [0.0; 3]);
    let in_notch = cuboid([1.25, 1.25, 0.25], [1.75, 1.75, 0.75]);
    assert_eq!(status(&merge(&l, &in_notch)), ShellStatus::Certified);
}

#[test]
fn exhausted_budgets_are_uncertain_never_exact() {
    let two = merge(
        &cuboid([0.0; 3], [10.0; 3]),
        &cuboid([20.0, 0.0, 0.0], [30.0, 10.0, 10.0]),
    );
    let m = mesh(&two);
    let pairs = analyze_with(
        &m,
        Limits {
            max_candidate_pairs: 1,
            ..Limits::DEFAULT
        },
        |_| false,
    )
    .unwrap();
    assert_eq!(
        pairs.status(),
        ShellStatus::Uncertain(Uncertainty::PairBudget)
    );
    let nodes = analyze_with(
        &m,
        Limits {
            max_index_nodes: 1,
            ..Limits::DEFAULT
        },
        |_| false,
    )
    .unwrap();
    assert_eq!(
        nodes.status(),
        ShellStatus::Uncertain(Uncertainty::IndexBudget)
    );
    // A zero node budget cannot certify even a mesh whose index is a single leaf.
    let tetra = Buf {
        v: vec![
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [0.0, 10.0, 0.0],
            [0.0, 0.0, 10.0],
        ],
        t: vec![[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]],
    };
    let tm = mesh(&tetra);
    assert_eq!(analyze(&tm).unwrap().status(), ShellStatus::Certified);
    let zero = analyze_with(
        &tm,
        Limits {
            max_index_nodes: 0,
            ..Limits::DEFAULT
        },
        |_| false,
    )
    .unwrap();
    assert_eq!(
        zero.status(),
        ShellStatus::Uncertain(Uncertainty::IndexBudget)
    );
    // Coplanar neighbours need the exact path, which a zero budget refuses.
    let exact = analyze_with(
        &m,
        Limits {
            max_exact_predicates: 0,
            ..Limits::DEFAULT
        },
        |_| false,
    )
    .unwrap();
    assert_eq!(
        exact.status(),
        ShellStatus::Uncertain(Uncertainty::PredicateBudget)
    );
    for a in [pairs, nodes, exact] {
        assert!(!a.is_certified());
        assert_eq!(a.code(), Some(MeshCode::NotCertified));
        assert_eq!(a.code().unwrap().f04_code(), ErrorCode::UnsupportedGeometry);
    }
    // With the production limits the same mesh certifies.
    assert_eq!(analyze(&m).unwrap().status(), ShellStatus::Certified);
}

#[test]
fn a_large_certified_slab_stays_within_budget() {
    let b = voxels(&ring(20, 20, &[]), 1.0, [0.0; 3]);
    assert!(b.t.len() > 1500);
    let a = analyze(&mesh(&b)).unwrap();
    assert_eq!(a.status(), ShellStatus::Certified);
    assert!(a.candidate_pairs_examined() > 0);
    assert!(a.exact_predicates_used() < Limits::DEFAULT.max_exact_predicates);
    assert!(a.shells()[0].signed_volume_mm3.unwrap().contains(400.0));
}

#[test]
fn mesh_size_is_capped_like_the_largest_body() {
    // D is the largest distance from vertex 0; above the 2,000,000 mm plate cap it is a limit error.
    let big = cuboid([0.0; 3], [1_500_000.0, 1_000.0, 1_000.0]);
    assert_eq!(status(&big), ShellStatus::Certified);
    let e = analyze(&mesh(&cuboid([0.0; 3], [2_500_000.0, 1.0, 1.0]))).unwrap_err();
    assert_eq!(
        (e.code, e.fault),
        (MeshCode::AboveLimit, Fault::Coordinates)
    );
    assert_eq!(e.code.f04_code(), ErrorCode::LimitExceeded);
    assert_eq!(e.status(), "limit_exceeded");
    // The same size passed as an explicit body tolerance is the caller's responsibility.
    let m = mesh(&cuboid([0.0; 3], [100.0, 200.0, 5.0]));
    let tol = m.tolerance().unwrap();
    assert!(tol.local_size_mm() > 223.0 && tol.eps_geom_mm() >= 0.001);
    let a = beamcraft_kernel::topology::analyze_with_tolerance(&m, tol, Limits::DEFAULT, |_| false)
        .unwrap();
    assert_eq!(a.tolerance(), tol);
}

// ---------------------------------------------------------------- point classification

#[test]
fn point_classification_on_a_tunnelled_solid() {
    let b = voxels(&ring(3, 3, &[[1, 1]]), 1.0, [0.0; 3]);
    let m = mesh(&b);
    let a = analyze(&m).unwrap();
    let at = |p: [f64; 3]| classify_point(&m, &a, p).unwrap();
    assert_eq!(at([0.3, 0.2, 0.7]), PointClass::Inside);
    assert_eq!(at([2.4, 1.3, 0.6]), PointClass::Inside);
    assert_eq!(at([1.3, 1.2, 0.6]), PointClass::Outside); // inside the tunnel
    assert_eq!(at([-0.3, 0.2, 0.7]), PointClass::Outside);
    assert_eq!(at([0.3, 0.2, 1.5]), PointClass::Outside);
    assert_eq!(at([0.0, 0.5, 0.5]), PointClass::OnSurface);
    assert_eq!(at([1.0, 1.0, 0.5]), PointClass::OnSurface);
    // An uncertified analysis refuses and a foreign mesh is detected.
    let open = {
        let mut o = cuboid([0.0; 3], [1.0; 3]);
        o.t.pop();
        mesh(&o)
    };
    let open_analysis = analyze(&open).unwrap();
    assert_eq!(
        classify_point(&open, &open_analysis, [0.5; 3])
            .unwrap_err()
            .code,
        MeshCode::NotCertified
    );
    assert_eq!(
        classify_point(&open, &a, [0.5; 3]).unwrap_err().fault,
        Fault::Analysis
    );
    assert_eq!(
        classify_point(&m, &a, [f64::NAN, 0.0, 0.0])
            .unwrap_err()
            .code,
        MeshCode::NonFinite
    );
}

// ---------------------------------------------------------------- structural validation

#[test]
fn buffers_reject_non_finite_out_of_range_and_oversized_input() {
    let ok = cuboid([0.0; 3], [1.0; 3]);
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut b = ok.clone();
        b.v[3][1] = bad;
        let e = TriMesh::from_buffers(&b.v, &b.t).unwrap_err();
        assert_eq!((e.code, e.fault), (MeshCode::NonFinite, Fault::Coordinates));
        assert_eq!(e.code.f04_code(), ErrorCode::NonFiniteNumber);
    }
    let mut far = ok.clone();
    far.v[0][2] = MAX_COORDINATE_MM;
    assert!(TriMesh::from_buffers(&far.v, &far.t).is_ok());
    far.v[0][2] = MAX_COORDINATE_MM.next_up();
    assert_eq!(
        TriMesh::from_buffers(&far.v, &far.t).unwrap_err().code,
        MeshCode::AboveLimit
    );
    let mut oor = ok.clone();
    oor.t[0][2] = 8;
    let e = TriMesh::from_buffers(&oor.v, &oor.t).unwrap_err();
    assert_eq!((e.code, e.fault), (MeshCode::BlobInvalid, Fault::Indices));
    assert_eq!(e.code.f04_code(), ErrorCode::IntegrityError);
    assert_eq!(
        TriMesh::from_buffers(&[], &ok.t).unwrap_err().fault,
        Fault::Vertices
    );
    assert_eq!(
        TriMesh::from_buffers(&ok.v, &[]).unwrap_err().fault,
        Fault::Triangles
    );
    // Negative zero is canonicalized in buffers.
    let mut nz = ok.clone();
    nz.v[0][0] = -0.0;
    assert_eq!(
        TriMesh::from_buffers(&nz.v, &nz.t).unwrap().vertices()[0][0].to_bits(),
        0
    );
}

#[test]
fn caps_are_exact_at_the_limit_and_rejected_one_above() {
    let tri = |n: usize| -> Vec<[u32; 3]> {
        (0..n)
            .map(|i| {
                [
                    (i % 4096) as u32,
                    ((i + 1) % 4096) as u32,
                    ((i + 2) % 4096) as u32,
                ]
            })
            .collect()
    };
    let verts = |n: usize| -> Vec<[f64; 3]> { (0..n).map(|i| [i as f64, 0.0, 0.0]).collect() };
    assert!(TriMesh::from_buffers(&verts(MAX_VERTICES), &tri(MAX_TRIANGLES)).is_ok());
    let e = TriMesh::from_buffers(&verts(MAX_VERTICES + 1), &tri(12)).unwrap_err();
    assert_eq!((e.code, e.fault), (MeshCode::BufferLimit, Fault::Vertices));
    let e = TriMesh::from_buffers(&verts(MAX_VERTICES), &tri(MAX_TRIANGLES + 1)).unwrap_err();
    assert_eq!((e.code, e.fault), (MeshCode::BufferLimit, Fault::Triangles));
    // F06-POS-27: a blob exactly at every cap is structurally valid.
    let v = verts(MAX_VERTICES);
    let t = tri(MAX_TRIANGLES);
    let faces: Vec<u16> = (0..MAX_TRIANGLES)
        .map(|i| (i % MAX_FACE_TABLE) as u16)
        .collect();
    let bytes = blob(&v, &t, &faces, MAX_FACE_TABLE as u32, 0);
    assert_eq!(bytes.len(), MAX_BLOB_BYTES);
    let parsed = TriMesh::from_blob(&bytes).unwrap();
    assert_eq!(parsed.to_blob(), bytes);
    assert_eq!(parsed.face_count(), MAX_FACE_TABLE as u32);
    // Hostile worst case: a maximum soup of random triangles terminates and is refused.
    let mut rng = Rng(77);
    let soup: Vec<[u32; 3]> = (0..MAX_TRIANGLES)
        .map(|_| {
            [
                rng.below(4096) as u32,
                rng.below(4096) as u32,
                rng.below(4096) as u32,
            ]
        })
        .collect();
    let m = TriMesh::from_buffers(&v, &soup);
    if let Ok(m) = m {
        let r = analyze(&m);
        assert!(r.is_err() || !r.unwrap().is_certified());
    }
}

fn header(v: u32, t: u32, k: u32, space: u32) -> Vec<u8> {
    let mut out = b"BCMESH01".to_vec();
    for w in [1u32, v, t, k, space, 0] {
        out.extend(w.to_le_bytes());
    }
    out
}

#[test]
fn blob_header_cases_follow_the_f06_codes() {
    let code = |bytes: &[u8]| {
        let e = TriMesh::from_blob(bytes).unwrap_err();
        (e.code, e.fault)
    };
    // F06-NEG-11..14: counts are compared with their caps before any size.
    assert_eq!(
        code(&header(262_145, 12, 6, 0)),
        (MeshCode::BufferLimit, Fault::Vertices)
    );
    assert_eq!(
        code(&header(8, 524_289, 6, 0)),
        (MeshCode::BufferLimit, Fault::Triangles)
    );
    assert_eq!(
        code(&header(8, 12, 65_536, 0)),
        (MeshCode::TopologyLimit, Fault::FaceTable)
    );
    assert_eq!(
        code(&header(u32::MAX, u32::MAX, u32::MAX, 0)),
        (MeshCode::BufferLimit, Fault::Vertices)
    );
    // F06-NEG-30 and zero counts one by one.
    assert_eq!(code(&header(0, 0, 0, 0)).0, MeshCode::BlobInvalid);
    assert_eq!(
        code(&header(0, 12, 6, 0)),
        (MeshCode::BlobInvalid, Fault::Vertices)
    );
    assert_eq!(
        code(&header(8, 0, 6, 0)),
        (MeshCode::BlobInvalid, Fault::Triangles)
    );
    assert_eq!(
        code(&header(8, 12, 0, 0)),
        (MeshCode::BlobInvalid, Fault::FaceTable)
    );
    // Header-only blob of a valid-looking header: the length disagrees.
    assert_eq!(
        code(&header(8, 12, 6, 0)),
        (MeshCode::BlobInvalid, Fault::Bytes)
    );
    // Bad magic, layout, space, reserved, short input.
    let mut h = header(8, 12, 6, 0);
    h[0] = b'X';
    assert_eq!(code(&h), (MeshCode::BlobInvalid, Fault::Header));
    for (at, value) in [(8usize, 2u32), (24, 2), (28, 1)] {
        let mut h = header(8, 12, 6, 0);
        h[at..at + 4].copy_from_slice(&value.to_le_bytes());
        assert_eq!(code(&h), (MeshCode::BlobInvalid, Fault::Header));
    }
    assert_eq!(code(&[]).1, Fault::Header);
    assert_eq!(code(&b"BCMESH01"[..]).1, Fault::Header);
    assert_eq!(
        code(&vec![0u8; MAX_BLOB_BYTES + 1]),
        (MeshCode::BufferLimit, Fault::Bytes)
    );
}

#[test]
fn blob_body_corruption_is_rejected_and_the_good_blob_is_not() {
    let b = cuboid([0.0; 3], [10.0, 20.0, 1.0]);
    let faces: Vec<u16> = [0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5].to_vec();
    let good = blob(&b.v, &b.t, &faces, 6, 0);
    assert_eq!(good.len(), 392);
    assert!(TriMesh::from_blob(&good).is_ok());
    let fault = |bytes: &[u8]| {
        let e = TriMesh::from_blob(bytes).unwrap_err();
        (e.code, e.fault)
    };
    assert_eq!(fault(&good[..391]), (MeshCode::BlobInvalid, Fault::Bytes)); // F06-NEG-15
    let mut longer = good.clone();
    longer.push(0);
    assert_eq!(fault(&longer), (MeshCode::BlobInvalid, Fault::Bytes));
    let mut c = good.clone();
    c[32..40].copy_from_slice(&f64::NAN.to_le_bytes());
    assert_eq!(fault(&c), (MeshCode::BlobInvalid, Fault::Coordinates));
    c[32..40].copy_from_slice(&f64::INFINITY.to_le_bytes());
    assert_eq!(fault(&c), (MeshCode::BlobInvalid, Fault::Coordinates));
    c[32..40].copy_from_slice(&(-0.0f64).to_le_bytes());
    assert_eq!(fault(&c), (MeshCode::BlobInvalid, Fault::Coordinates));
    c[32..40].copy_from_slice(&5.0e9f64.to_le_bytes());
    assert_eq!(fault(&c), (MeshCode::AboveLimit, Fault::Coordinates));
    let index_at = 32 + 24 * 8;
    let mut c = good.clone();
    c[index_at..index_at + 4].copy_from_slice(&8u32.to_le_bytes());
    assert_eq!(fault(&c), (MeshCode::BlobInvalid, Fault::Indices));
    let face_at = index_at + 12 * 12;
    let mut c = good.clone();
    c[face_at..face_at + 2].copy_from_slice(&6u16.to_le_bytes());
    assert_eq!(fault(&c), (MeshCode::BlobInvalid, Fault::Faces));
    // 12*12 = 144 index bytes need no padding but 24 face bytes do not either; use a
    // 5-triangle blob so both sections pad, then dirty the padding.
    let small = Buf {
        v: b.v.clone(),
        t: b.t[..5].to_vec(),
    };
    let sb = blob(&small.v, &small.t, &[0, 0, 0, 0, 0], 1, 0);
    assert!(TriMesh::from_blob(&sb).is_ok());
    let pad_at = 32 + 24 * 8 + 60;
    let mut c = sb.clone();
    c[pad_at] = 1;
    assert_eq!(fault(&c), (MeshCode::BlobInvalid, Fault::Faces));
    let mut c = sb.clone();
    let last = c.len() - 1;
    c[last] = 1;
    assert_eq!(fault(&c), (MeshCode::BlobInvalid, Fault::Faces));
}

// ---------------------------------------------------------------- cancellation

fn record_polls(m: &TriMesh) -> Vec<Stage> {
    let mut seen = Vec::new();
    analyze_with(m, Limits::DEFAULT, |s| {
        seen.push(s);
        false
    })
    .unwrap();
    seen
}

fn assert_cancelled_at_every_poll(m: &TriMesh) {
    let stages = record_polls(m);
    assert!(stages.len() >= 3);
    assert_eq!(stages.first(), Some(&Stage::Validation));
    assert_eq!(stages.last(), Some(&Stage::Quantities));
    for (k, stage) in stages.iter().enumerate() {
        let mut n = 0usize;
        let r = analyze_with(m, Limits::DEFAULT, |_| {
            let hit = n == k;
            n += 1;
            hit
        });
        let e = r.unwrap_err();
        assert_eq!(
            (e.code, e.fault, e.stage),
            (MeshCode::Cancelled, Fault::Cancellation, *stage),
            "poll {k}"
        );
        assert_eq!(e.code.f04_code(), ErrorCode::Cancelled);
        assert_eq!(n, k + 1, "no work and no poll after the cancelled one");
    }
    // The poll sequence is the same every time (determinism of observing points).
    assert_eq!(stages, record_polls(m));
}

#[test]
fn cancellation_at_every_poll_discards_everything() {
    assert_cancelled_at_every_poll(&mesh(&cuboid([0.0; 3], [1.0; 3])));
    let nested = merge(
        &cuboid([0.0; 3], [10.0; 3]),
        &cuboid([2.0, 3.0, 4.0], [3.0, 5.0, 7.0]),
    );
    assert_cancelled_at_every_poll(&mesh(&nested));
    let open = {
        let mut b = cuboid([0.0; 3], [1.0; 3]);
        b.t.pop();
        b
    };
    assert_cancelled_at_every_poll(&mesh(&open));
    // Large enough for polls inside the pair scan.
    let slab = voxels(&ring(8, 8, &[]), 1.0, [0.0; 3]);
    let stages = record_polls(&mesh(&slab));
    assert!(
        stages.iter().filter(|s| **s == Stage::Checks).count() >= 4,
        "{stages:?}"
    );
    assert_cancelled_at_every_poll(&mesh(&slab));
}

#[test]
fn analysis_of_each_pipeline_stage_has_the_documented_poll_order() {
    let stages = record_polls(&mesh(&cuboid([0.0; 3], [1.0; 3])));
    assert_eq!(
        stages[..3],
        [Stage::Validation, Stage::Build, Stage::Checks]
    );
    assert!(stages.ends_with(&[Stage::Quantities]));
    // A defective mesh stops before the index and the nested-shell probes.
    let mut open = cuboid([0.0; 3], [1.0; 3]);
    open.t.pop();
    assert_eq!(
        record_polls(&mesh(&open)),
        [
            Stage::Validation,
            Stage::Build,
            Stage::Checks,
            Stage::Quantities
        ]
    );
}
