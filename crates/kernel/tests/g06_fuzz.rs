// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! G06 deterministic hostile-input fuzzing: no panic, no hang, no false certification.
//!
//! All generators are seeded xorshift streams, so every failure reproduces. The
//! independent oracles in `common` decide topology and signed volume.

#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

use crate::common_mesh as common;

use beamcraft_kernel::measure::measure;
use beamcraft_kernel::topology::{Analysis, Limits, analyze, analyze_with};
use beamcraft_kernel::trimesh::TriMesh;
use beamcraft_kernel::verdict::{MeshCode, ShellStatus};
use common::*;

const SPECIAL_F64: [f64; 12] = [
    f64::NAN,
    f64::INFINITY,
    f64::NEG_INFINITY,
    -0.0,
    0.0,
    5e-324,
    f64::MIN_POSITIVE,
    1.0e300,
    -1.0e300,
    4.0e9,
    4.000_000_000_000_001e9,
    1.0,
];

fn small_limits() -> Limits {
    Limits {
        max_candidate_pairs: 20_000,
        max_index_nodes: 4096,
        max_exact_predicates: 20_000,
    }
}

/// Oracle checks that must hold whenever production says `Certified`.
fn assert_sound_certificate(m: &TriMesh, a: &Analysis) {
    let v: Verts = m.vertices().to_vec();
    let t: Tris = m.triangles().to_vec();
    assert_eq!(topology_verdict(&t), "closed_manifold");
    let comps = components(&t);
    assert_eq!(comps.len(), a.shells().len());
    let mut total = 0.0;
    for c in &comps {
        let sub: Tris = c.iter().map(|i| t[*i]).collect();
        let volume = signed_volume(&v, &sub);
        assert!(volume > 0.0, "certified shell with volume {volume}");
        total += volume;
    }
    let q = match measure(m, a) {
        Ok(q) => q,
        Err(e) => panic!("{e:?} {m:?}"),
    };
    let measured = q.volume_mm3().unwrap();
    // The f64 oracle can lose digits to cancellation; allow a generous relative band.
    let scale: f64 = v.iter().flatten().map(|x| x.abs()).fold(1.0, f64::max);
    assert!(
        (measured.midpoint() - total).abs() <= 1e-9 * total.abs() + 1e-9 * scale.powi(3) * 1e-6,
        "{measured:?} vs {total}"
    );
    assert!(measured.lo > 0.0 && q.area_mm2().unwrap().lo > 0.0);
}

fn run(m: &TriMesh) -> (String, bool) {
    let first = analyze_with(m, small_limits(), |_| false);
    let second = analyze_with(m, small_limits(), |_| false);
    assert_eq!(format!("{first:?}"), format!("{second:?}"), "deterministic");
    match first {
        Ok(a) => {
            if a.is_certified() {
                assert_sound_certificate(m, &a);
            }
            // Any refusal carries a stable code; certified carries none.
            assert_eq!(a.code().is_none(), a.is_certified());
            (a.status().as_str().to_owned(), a.is_certified())
        }
        Err(e) => {
            assert!(!e.code.as_str().is_empty() && e.fault.as_str().len() <= 64);
            assert!(matches!(
                e.code,
                MeshCode::TopologyLimit | MeshCode::AboveLimit | MeshCode::NumericUncertain
            ));
            (format!("error:{}", e.code.as_str()), false)
        }
    }
}

fn mutate_blob(base: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut b = base.to_vec();
    for _ in 0..=rng.below(3) {
        match rng.below(7) {
            0 => {
                let (i, bit) = (rng.below(b.len() as u64) as usize, rng.below(8));
                b[i] ^= 1 << bit;
            }
            1 if b.len() >= 8 => {
                let at = rng.below(b.len() as u64 - 7) as usize;
                let special = SPECIAL_F64[rng.below(SPECIAL_F64.len() as u64) as usize];
                b[at..at + 8].copy_from_slice(&special.to_le_bytes());
            }
            2 if b.len() >= 4 => {
                let at = rng.below(b.len() as u64 - 3) as usize;
                let word =
                    [0u32, 1, 7, 8, 12, 4096, 4097, 8192, 8193, u32::MAX][rng.below(10) as usize];
                b[at..at + 4].copy_from_slice(&word.to_le_bytes());
            }
            3 => {
                let keep = rng.below(b.len() as u64 + 1) as usize;
                b.truncate(keep);
                if b.is_empty() {
                    b.push(0);
                }
            }
            4 => {
                for _ in 0..rng.below(40) {
                    b.push(rng.next() as u8);
                }
            }
            5 => {
                // Header-field edits keep the body, so length checks are exercised.
                let field = [8usize, 12, 16, 20, 24, 28][rng.below(6) as usize];
                if b.len() >= field + 4 {
                    let word = rng.below(20) as u32;
                    b[field..field + 4].copy_from_slice(&word.to_le_bytes());
                }
            }
            _ => {
                let at = rng.below(b.len() as u64) as usize;
                b[at] = rng.next() as u8;
            }
        }
    }
    b
}

#[test]
fn hostile_blobs_never_panic_or_certify_unsoundly() {
    let plate = cuboid([0.0; 3], [10.0, 20.0, 1.0]);
    let torus = voxels(
        &[
            [0, 0, 0],
            [1, 0, 0],
            [2, 0, 0],
            [0, 1, 0],
            [2, 1, 0],
            [0, 2, 0],
            [1, 2, 0],
            [2, 2, 0],
        ],
        1.5,
        [1.0; 3],
    );
    let bases = [
        blob(
            &plate.v,
            &plate.t,
            &[0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5],
            6,
            0,
        ),
        blob(&torus.v, &torus.t, &vec![0; torus.t.len()], 1, 1),
    ];
    let mut rng = Rng(0xfeed_f00d_1234_5678);
    let mut outcomes = std::collections::BTreeMap::<String, usize>::new();
    for round in 0..1600 {
        let bytes = mutate_blob(&bases[round % 2], &mut rng);
        match TriMesh::from_blob(&bytes) {
            Err(e) => {
                *outcomes
                    .entry(format!("blob:{}", e.code.as_str()))
                    .or_default() += 1;
                // Every structural failure is a stable code with a stable F04 mapping.
                assert!(matches!(
                    e.code,
                    MeshCode::BlobInvalid
                        | MeshCode::BufferLimit
                        | MeshCode::TopologyLimit
                        | MeshCode::AboveLimit
                ));
                let _ = e.code.f04_code();
            }
            Ok(m) => {
                // Accepted blobs are canonical and re-encode to the same bytes.
                assert_eq!(m.to_blob(), bytes);
                let (label, _) = run(&m);
                *outcomes.entry(label).or_default() += 1;
            }
        }
    }
    assert!(outcomes.len() >= 5, "{outcomes:?}");
    assert!(outcomes.keys().any(|k| k.starts_with("blob:")));
}

fn random_buffers(rng: &mut Rng) -> (Verts, Tris) {
    let nv = 1 + rng.below(10) as usize;
    let nt = 1 + rng.below(14) as usize;
    let v: Verts = (0..nv)
        .map(|_| {
            [0, 1, 2].map(|_| {
                if rng.chance(1, 12) {
                    SPECIAL_F64[rng.below(SPECIAL_F64.len() as u64) as usize]
                } else {
                    // Small integers keep coincidences and coplanarity likely.
                    (rng.below(5) as f64) - 2.0
                }
            })
        })
        .collect();
    let t: Tris = (0..nt)
        .map(|_| {
            [0, 1, 2].map(|_| {
                if rng.chance(1, 25) {
                    nv as u32 + rng.below(3) as u32
                } else {
                    rng.below(nv as u64) as u32
                }
            })
        })
        .collect();
    (v, t)
}

#[test]
fn random_soups_never_panic_or_certify_unsoundly() {
    let mut rng = Rng(0x0dd_ba11_5eed);
    let mut seen = std::collections::BTreeMap::<String, usize>::new();
    for _ in 0..3000 {
        let (v, t) = random_buffers(&mut rng);
        match TriMesh::from_buffers(&v, &t) {
            Err(e) => {
                *seen.entry(format!("buf:{}", e.code.as_str())).or_default() += 1;
            }
            Ok(m) => {
                let (label, certified) = run(&m);
                *seen.entry(label).or_default() += 1;
                if !certified {
                    continue;
                }
            }
        }
    }
    assert!(
        seen.keys().any(|k| k.starts_with("buf:GEO_NON_FINITE")),
        "{seen:?}"
    );
    assert!(
        seen.keys().any(|k| k.starts_with("buf:GEO_BLOB_INVALID")),
        "{seen:?}"
    );
    assert!(seen.values().sum::<usize>() == 3000);
}

#[test]
fn surgery_on_valid_shells_is_never_certified_and_is_classified() {
    let torus = voxels(
        &[
            [0, 0, 0],
            [1, 0, 0],
            [2, 0, 0],
            [0, 1, 0],
            [2, 1, 0],
            [0, 2, 0],
            [1, 2, 0],
            [2, 2, 0],
        ],
        1.0,
        [0.0; 3],
    );
    let bases = [
        cuboid([0.0; 3], [3.0, 4.0, 5.0]),
        prism(3.0, 4.0, 5.0),
        torus,
        voxels(&[[0, 0, 0], [1, 0, 0], [1, 1, 0], [1, 1, 1]], 1.0, [0.0; 3]),
    ];
    let mut rng = Rng(0xabad_1dea);
    for base in &bases {
        assert_eq!(
            analyze(&TriMesh::from_buffers(&base.v, &base.t).unwrap())
                .unwrap()
                .status(),
            ShellStatus::Certified
        );
        for _ in 0..60 {
            let mut b = base.clone();
            let victim = rng.below(b.t.len() as u64) as usize;
            let (expected, rewired) = match rng.below(4) {
                0 => {
                    b.t.remove(victim);
                    (Some(ShellStatus::Open), false)
                }
                1 => {
                    b.t.push(b.t[victim]);
                    (Some(ShellStatus::NonManifold), false)
                }
                2 => {
                    b.t[victim].swap(1, 2);
                    (Some(ShellStatus::InconsistentOrientation), false)
                }
                _ => {
                    // Rewire one corner to another vertex: three edges lose their partners.
                    let corner = rng.below(3) as usize;
                    let new = rng.below(b.v.len() as u64) as u32;
                    if b.t[victim][corner] == new {
                        continue;
                    }
                    b.t[victim][corner] = new;
                    (None, true)
                }
            };
            let m = TriMesh::from_buffers(&b.v, &b.t).unwrap();
            let a = analyze(&m).unwrap();
            assert!(!a.is_certified(), "{b:?}");
            if let Some(want) = expected {
                // Removing a triangle can leave a pinched vertex or other defects of
                // higher precedence only for non-manifold fans; the oracle decides.
                let oracle = topology_verdict(&b.t);
                let status = a.status();
                match (want, oracle) {
                    (ShellStatus::Open, "open") => assert_eq!(status, ShellStatus::Open),
                    (ShellStatus::Open, "non_manifold") => {
                        assert_eq!(status, ShellStatus::NonManifold)
                    }
                    (ShellStatus::NonManifold, _) => assert_eq!(status, ShellStatus::NonManifold),
                    (ShellStatus::InconsistentOrientation, "inconsistent") => {
                        assert_eq!(status, ShellStatus::InconsistentOrientation);
                    }
                    (ShellStatus::InconsistentOrientation, "non_manifold") => {
                        assert_eq!(status, ShellStatus::NonManifold);
                    }
                    other => panic!("unexpected {other:?} {status:?}"),
                }
            } else {
                assert!(rewired);
                assert_ne!(topology_verdict(&b.t), "closed_manifold");
            }
        }
    }
}

#[test]
fn perturbed_vertices_keep_closed_topology_and_never_certify_unsoundly() {
    let base = voxels(
        &[
            [0, 0, 0],
            [1, 0, 0],
            [2, 0, 0],
            [0, 1, 0],
            [2, 1, 0],
            [0, 2, 0],
            [1, 2, 0],
            [2, 2, 0],
        ],
        1.0,
        [0.0; 3],
    );
    let mut rng = Rng(0x1357_9bdf);
    let (mut certified, mut other) = (0, 0);
    for _ in 0..300 {
        let mut b = base.clone();
        for _ in 0..=rng.below(2) {
            let i = rng.below(b.v.len() as u64) as usize;
            let axis = rng.below(3) as usize;
            b.v[i][axis] += match rng.below(4) {
                0 => 0.5,
                1 => -1.5,
                2 => 2f64.powi(-40),
                _ => 3.0,
            };
        }
        let m = TriMesh::from_buffers(&b.v, &b.t).unwrap();
        let (label, ok) = run(&m);
        if ok {
            certified += 1;
        } else {
            other += 1;
            assert!(
                ["overlapping", "inward", "degenerate", "uncertain"].contains(&label.as_str()),
                "{label}"
            );
        }
    }
    assert!(certified > 20 && other > 20, "{certified} {other}");
}

#[test]
fn maximum_size_hostile_soups_terminate() {
    let mut rng = Rng(0x600d_c0de);
    let verts: Verts = (0..4096)
        .map(|_| [0, 1, 2].map(|_| rng.below(4000) as f64 * 0.25 - 500.0))
        .collect();
    let tris: Tris = (0..8192)
        .map(|_| [0, 1, 2].map(|_| rng.below(4096) as u32))
        .collect();
    let m = TriMesh::from_buffers(&verts, &tris).unwrap();
    let a = analyze(&m);
    assert!(a.is_err() || !a.unwrap().is_certified());
    // A dense overlapping stack of closed boxes hits a budget instead of running away.
    let boxes = (0..16)
        .map(|i| cuboid([i as f64 * 0.1; 3], [10.0 + i as f64 * 0.1; 3]))
        .reduce(|a, b| merge(&a, &b))
        .unwrap();
    let m = TriMesh::from_buffers(&boxes.v, &boxes.t).unwrap();
    let a = analyze_with(
        &m,
        Limits {
            max_candidate_pairs: 50,
            ..Limits::DEFAULT
        },
        |_| false,
    )
    .unwrap();
    assert!(!a.is_certified());
}
