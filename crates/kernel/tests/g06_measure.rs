// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! G06 independent checks of mesh measurements, statuses, mass rules and transforms.
//!
//! Expected values come from closed forms and voxel counting in `common`.

#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

use crate::common_mesh as common;

use beamcraft_geom::{Frame3, Point3, RigidTransform, Vec3};
use beamcraft_kernel::diagnostic::{Code, Stage};
use beamcraft_kernel::enclosure::Enclosure;
use beamcraft_kernel::measure::cross_check_plate;
use beamcraft_kernel::measure::{
    Basis, MassRecord, MeshMeasurement, QuantityStatus, analytic_mass_kg, mass_status, measure,
    measure_with_cancel,
};
use beamcraft_kernel::topology::{Limits, analyze, analyze_with};
use beamcraft_kernel::trimesh::TriMesh;
use beamcraft_kernel::verdict::{Fault, MeshCode, ShellStatus, Uncertainty};
use beamcraft_kernel::{RectangularPlate, rectangular_plate};
use common::*;

fn mesh(b: &Buf) -> TriMesh {
    TriMesh::from_buffers(&b.v, &b.t).unwrap()
}

fn measured(m: &TriMesh) -> MeshMeasurement {
    measure(m, &analyze(m).unwrap()).unwrap()
}

fn near(e: Enclosure, expected: f64, relative: f64) -> bool {
    (e.midpoint() - expected).abs() <= relative * expected.abs().max(1e-300)
        && e.width() <= 8.0 * relative * expected.abs().max(1e-300)
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

fn rotation(x: [f64; 3], y: [f64; 3], z: [f64; 3], origin: [f64; 3]) -> RigidTransform {
    let frame = Frame3::try_new(
        Vec3::new(x[0], x[1], x[2]).unwrap(),
        Vec3::new(y[0], y[1], y[2]).unwrap(),
        Vec3::new(z[0], z[1], z[2]).unwrap(),
    )
    .unwrap();
    RigidTransform::from_body_frame(frame, Point3::new(origin[0], origin[1], origin[2])).unwrap()
}

// ---------------------------------------------------------------- closed forms

#[test]
fn box_measurements_match_closed_forms() {
    let m = mesh(&cuboid([0.0; 3], [10.0, 20.0, 5.0]));
    let q = measured(&m);
    assert_eq!(q.shell_status(), ShellStatus::Certified);
    assert!(q.volume_mm3().unwrap().contains(1000.0));
    assert!(q.area_mm2().unwrap().contains(2.0 * (200.0 + 50.0 + 100.0)));
    let c = q.centroid_mm().unwrap();
    for (e, want) in c.iter().zip([5.0, 10.0, 2.5]) {
        assert!(e.contains(want), "{e:?} {want}");
    }
    assert_eq!(q.bounds().min, [0.0; 3]);
    assert_eq!(q.bounds().max, [10.0, 20.0, 5.0]);
    // Integer-valued quantities stay tight: exact operations keep point results.
    assert!(q.volume_mm3().unwrap().width() < 1e-12);
    // Unit conversions divide by the exact integers 1e9 and 1e6.
    assert!(q.volume_m3().unwrap().contains(1000.0 / 1e9));
    assert!(q.area_m2().unwrap().contains(700.0 / 1e6));
}

#[test]
fn prism_and_tunnel_measurements_match_closed_forms() {
    let q = measured(&mesh(&prism(3.0, 4.0, 5.0)));
    assert!(q.volume_mm3().unwrap().contains(30.0));
    // Two triangles ab/2 each, faces a*h and b*h, and the 5 x 5 hypotenuse face.
    assert!(q.area_mm2().unwrap().contains(12.0 + 15.0 + 20.0 + 25.0));
    // Centroid of a right prism: (a/3, b/3, h/2).
    let c = q.centroid_mm().unwrap();
    for (e, want) in c.iter().zip([1.0, 4.0 / 3.0, 2.5]) {
        assert!(
            (e.midpoint() - want).abs() < 1e-14 && e.width() < 1e-13,
            "{e:?} {want}"
        );
    }
    // Irrational area: legs 1 and 1 give 1 + (2 + sqrt 2) * 1 = 3 + sqrt 2.
    let q = measured(&mesh(&prism(1.0, 1.0, 1.0)));
    assert!(near(q.area_mm2().unwrap(), 3.0 + 2f64.sqrt(), 1e-14));
    // The tunnelled slab: 8 cells, exposed faces counted independently.
    let cells = ring(3, 3, &[[1, 1]]);
    let q = measured(&mesh(&voxels(&cells, 1.0, [0.0; 3])));
    assert!(q.volume_mm3().unwrap().contains(8.0));
    assert!(q.area_mm2().unwrap().contains(exposed_faces(&cells) as f64));
    assert_eq!(exposed_faces(&cells), 32);
    // The slab is symmetric about x = y = 1.5, z = 0.5.
    for (e, want) in q.centroid_mm().unwrap().iter().zip([1.5, 1.5, 0.5]) {
        assert!(e.contains(want), "{e:?}");
    }
}

#[test]
fn mesh_values_are_never_engineering_quantities() {
    let q = measured(&mesh(&cuboid([0.0; 3], [1.0; 3])));
    assert_eq!(q.basis(), Basis::MeshEnclosure);
    // F06 section 8: a mesh from any other source gets no engineering quantity.
    assert_eq!(q.quantity_status(), QuantityStatus::Unsupported);
    for density in [None, Some(QuantityStatus::Current)] {
        let mass = q.mass(density);
        assert_eq!((mass.status, mass.kg), (QuantityStatus::Unsupported, None));
    }
}

// ---------------------------------------------------------------- refusal

#[test]
fn open_inward_overlapping_and_uncertain_meshes_yield_no_value() {
    let mut open = cuboid([0.0; 3], [10.0, 20.0, 1.0]);
    open.t.pop();
    let overlap = merge(
        &cuboid([0.0; 3], [10.0; 3]),
        &cuboid([5.0, 0.0, 0.0], [15.0, 10.0, 10.0]),
    );
    let mut flipped = cuboid([0.0; 3], [2.0; 3]);
    flipped.t[0].swap(1, 2);
    let mut duplicate = cuboid([0.0; 3], [2.0; 3]);
    duplicate.t.push(duplicate.t[0]);
    let mut collapsed = cuboid([0.0; 3], [2.0; 3]);
    collapsed.t.push([0, 0, 1]);
    let cases: [(Buf, ShellStatus); 6] = [
        (open, ShellStatus::Open),
        (
            inverted(&cuboid([0.0; 3], [100.0, 50.0, 10.0])),
            ShellStatus::Inward,
        ),
        (overlap, ShellStatus::Overlapping),
        (flipped, ShellStatus::InconsistentOrientation),
        (duplicate, ShellStatus::NonManifold),
        (collapsed, ShellStatus::Degenerate),
    ];
    for (b, want) in cases {
        let m = mesh(&b);
        let q = measured(&m);
        assert_eq!(q.shell_status(), want);
        assert!(q.volume_mm3().is_none() && q.area_mm2().is_none() && q.centroid_mm().is_none());
        assert!(q.volume_m3().is_none() && q.area_m2().is_none());
        assert_eq!(q.quantity_status(), QuantityStatus::Unsupported);
        // Unsupported geometry outranks an absent density (F06-NEG-18).
        assert_eq!(q.mass(None).status, QuantityStatus::Unsupported);
        assert_eq!(q.mass(None).kg, None);
        // Bounds of the vertices stay available but are not a quantity.
        assert!(q.bounds().min[0] <= q.bounds().max[0]);
    }
    let two = merge(
        &cuboid([0.0; 3], [10.0; 3]),
        &cuboid([20.0, 0.0, 0.0], [30.0; 3]),
    );
    let m = mesh(&two);
    let a = analyze_with(
        &m,
        Limits {
            max_candidate_pairs: 1,
            ..Limits::DEFAULT
        },
        |_| false,
    )
    .unwrap();
    assert_eq!(a.status(), ShellStatus::Uncertain(Uncertainty::PairBudget));
    let q = measure(&m, &a).unwrap();
    assert!(q.volume_mm3().is_none() && q.centroid_mm().is_none());
    assert_eq!(q.shell_status(), a.status());
}

#[test]
fn an_analysis_of_another_mesh_is_rejected() {
    let (a, b) = (
        mesh(&cuboid([0.0; 3], [1.0; 3])),
        mesh(&cuboid([0.0; 3], [2.0; 3])),
    );
    let analysis = analyze(&a).unwrap();
    let e = measure(&b, &analysis).unwrap_err();
    assert_eq!((e.code, e.fault), (MeshCode::NotCertified, Fault::Analysis));
}

// ---------------------------------------------------------------- mass and status rules

#[test]
fn mass_status_follows_the_f06_precedence() {
    use QuantityStatus::{Current, Stale, Unknown, Unsupported};
    assert_eq!(mass_status(Unsupported, None), Unsupported);
    assert_eq!(mass_status(Unsupported, Some(Current)), Unsupported);
    // Unknown density keeps mass unknown even when volume is current.
    assert_eq!(mass_status(Current, None), Unknown);
    assert_eq!(mass_status(Stale, None), Unknown);
    assert_eq!(mass_status(Current, Some(Stale)), Stale);
    assert_eq!(mass_status(Stale, Some(Current)), Stale);
    assert_eq!(mass_status(Current, Some(Current)), Current);
    assert_eq!(mass_status(Unknown, Some(Current)), Unknown);
    for s in [Current, Stale, Unknown, Unsupported] {
        assert!(matches!(
            s.as_str(),
            "current" | "stale" | "unknown" | "unsupported"
        ));
    }
}

#[test]
fn analytic_mass_uses_the_written_order_and_never_defaults_density() {
    // W08: a 1000 x 500 x 10 plate at density 1000 has V = 5,000,000 mm3 and mass 5 kg.
    let m = analytic_mass_kg(5_000_000.0, Some(1000.0)).unwrap();
    assert_eq!((m.status, m.kg), (QuantityStatus::Current, Some(5.0)));
    // W09: 240,000,000 mm3 at density 2000 is 480 kg.
    assert_eq!(
        analytic_mass_kg(240_000_000.0, Some(2000.0)).unwrap().kg,
        Some(480.0)
    );
    let unknown = analytic_mass_kg(5_000_000.0, None).unwrap();
    assert_eq!(
        (unknown.status, unknown.kg),
        (QuantityStatus::Unknown, None)
    );
    assert_eq!(analytic_mass_kg(1.0, Some(f64::NAN)), Err(Code::NonFinite));
    assert_eq!(
        analytic_mass_kg(1.0, Some(f64::INFINITY)),
        Err(Code::NonFinite)
    );
    // A zero density is the massless auxiliary material: a current mass of zero.
    assert_eq!(
        analytic_mass_kg(1.0, Some(0.0)),
        Ok(MassRecord {
            status: QuantityStatus::Current,
            kg: Some(0.0)
        })
    );
    assert_eq!(analytic_mass_kg(1.0, Some(-5.0)), Err(Code::NotPositive));
    // W10: a subnormal density underflows; zero is never a current mass.
    assert_eq!(
        analytic_mass_kg(1e-9, Some(5e-324)),
        Err(Code::NumericUncertain)
    );
    // G11: a subnormal mass is refused too. 1 mm3 at 1e-310 kg/m3 is about 1e-319 kg, a
    // subnormal with a relative error near 1e-5, beyond the comparison rule.
    assert_eq!(
        analytic_mass_kg(1.0, Some(1e-310)),
        Err(Code::NumericUncertain)
    );
    // The smallest normal mass is still current.
    assert_eq!(
        analytic_mass_kg(1e9, Some(f64::MIN_POSITIVE)).unwrap().kg,
        Some(f64::MIN_POSITIVE)
    );
    assert_eq!(
        analytic_mass_kg(1e300, Some(1e300)),
        Err(Code::NumericUncertain)
    );
    assert_eq!(
        analytic_mass_kg(0.0, Some(1.0)),
        Err(Code::NumericUncertain)
    );
    assert_eq!(
        analytic_mass_kg(f64::NAN, None),
        Err(Code::NumericUncertain)
    );
}

// ---------------------------------------------------------------- rigid and scale invariance

#[test]
fn exact_rigid_motions_preserve_every_scalar_quantity() {
    let cells = ring(3, 3, &[[1, 1]]);
    let base = mesh(&voxels(&cells, 1.0, [0.0; 3]));
    let q0 = measured(&base);
    let (v0, a0) = (q0.volume_mm3().unwrap(), q0.area_mm2().unwrap());
    let motions = [
        // Pure translation by exactly representable amounts, including a large one.
        rotation(
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.25, -1.5, 3.0],
        ),
        rotation(
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0e9, -1.0e9, 5.0e8],
        ),
        // A quarter turn about Z, and a cyclic permutation of the axes (both exact).
        rotation(
            [0.0, 1.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [2.0, 0.0, 0.0],
        ),
        rotation(
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 0.0],
            [-7.0, 9.0, 0.5],
        ),
    ];
    for t in &motions {
        let moved = base.transformed(t).unwrap();
        let a = analyze(&moved).unwrap();
        assert_eq!(a.status(), ShellStatus::Certified);
        let q = measure(&moved, &a).unwrap();
        // The exact integer sums see the same polyhedron: identical volume bits.
        assert_eq!(q.volume_mm3().unwrap(), v0);
        assert!(q.area_mm2().unwrap().overlaps(a0) && q.area_mm2().unwrap().contains(32.0));
        assert_eq!(a.euler_characteristic(), 0);
        assert_eq!(a.shells()[0].genus, Some(1));
    }
    // The centroid moves exactly like its points under the quarter turn about Z.
    let t = rotation(
        [0.0, 1.0, 0.0],
        [-1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        [2.0, 0.0, 0.0],
    );
    let moved = base.transformed(&t).unwrap();
    let c = measured(&moved).centroid_mm().unwrap();
    // (x, y, z) -> (-y + 2, x, z); the centroid (1.5, 1.5, 0.5) maps to (0.5, 1.5, 0.5).
    for (e, want) in c.iter().zip([0.5, 1.5, 0.5]) {
        assert!(e.contains(want), "{e:?}");
    }
    assert_eq!(moved.space(), beamcraft_kernel::trimesh::Space::World);
}

#[test]
fn inexact_rotation_changes_quantities_only_within_rounding() {
    // 3-4-5 rotation about Z: the axes 0.6 and 0.8 are rounded, so the map is
    // rigid only to binary64 accuracy; quantities agree to a tiny relative bound.
    let t = rotation(
        [0.6, 0.8, 0.0],
        [-0.8, 0.6, 0.0],
        [0.0, 0.0, 1.0],
        [10.0, -20.0, 30.0],
    );
    let b = cuboid([0.0; 3], [10.0, 20.0, 5.0]);
    let moved = mesh(&b).transformed(&t).unwrap();
    let q = measured(&moved);
    assert_eq!(q.shell_status(), ShellStatus::Certified);
    assert!(near(q.volume_mm3().unwrap(), 1000.0, 1e-12));
    assert!(near(q.area_mm2().unwrap(), 700.0, 1e-12));
    // Independent rotated centroid: R * (5, 10, 2.5) + (10, -20, 30).
    let want = [
        0.6 * 5.0 - 0.8 * 10.0 + 10.0,
        0.8 * 5.0 + 0.6 * 10.0 - 20.0,
        2.5 + 30.0,
    ];
    for (e, w) in q.centroid_mm().unwrap().iter().zip(want) {
        assert!((e.midpoint() - w).abs() < 1e-12, "{e:?} {w}");
    }
}

#[test]
fn reflections_cannot_be_certified_as_rigid_and_mirrored_meshes_are_inward() {
    assert!(
        Frame3::try_new(Vec3::X, Vec3::Y, Vec3::new(0.0, 0.0, -1.0).unwrap()).is_err(),
        "a reflection is not a body frame"
    );
    let b = cuboid([1.0, 0.0, 0.0], [4.0, 5.0, 6.0]);
    let mirrored = Buf {
        v: b.v.iter().map(|[x, y, z]| [-x, *y, *z]).collect(),
        t: b.t.clone(),
    };
    let a = analyze(&mesh(&mirrored)).unwrap();
    assert_eq!(a.status(), ShellStatus::Inward);
    assert!(measured(&mesh(&mirrored)).volume_mm3().is_none());
}

#[test]
fn placement_beyond_the_coordinate_cap_is_refused() {
    let t = rotation(
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.0e9, 1.0e9, 1.0e9],
    );
    let far = mesh(&cuboid([0.0; 3], [1.0; 3]));
    let once = far.transformed(&t).unwrap();
    let twice = once.transformed(&t).unwrap();
    let thrice = twice.transformed(&t).unwrap();
    let e = thrice.transformed(&t).unwrap_err();
    assert_eq!(e.code, MeshCode::AboveLimit);
}

#[test]
fn exact_scale_factors_scale_volume_cubically_and_area_quadratically() {
    let cells = ring(3, 3, &[[1, 1]]);
    let q1 = measured(&mesh(&voxels(&cells, 1.0, [0.0; 3])));
    for k in [2.0, 3.0, 0.5, 8.0] {
        let q = measured(&mesh(&voxels(&cells, k, [0.0; 3])));
        assert!(q.volume_mm3().unwrap().contains(8.0 * k * k * k), "{k}");
        assert!(q.area_mm2().unwrap().contains(32.0 * k * k), "{k}");
        // V / L^3 and S / L^2 are invariant (the factors are exact in binary64).
        assert!(q.volume_mm3().unwrap().overlaps(Enclosure {
            lo: q1.volume_mm3().unwrap().lo * k * k * k,
            hi: q1.volume_mm3().unwrap().hi * k * k * k,
        }));
    }
}

#[test]
fn far_from_the_origin_exact_sums_do_not_cancel_away() {
    // A 2 mm cube 1e9 mm from the origin: floating-point volume sums of absolute
    // coordinates would lose everything; exact sums keep it.
    let b = cuboid(
        [1.0e9, -1.0e9, 5.0e8],
        [1.0e9 + 2.0, -1.0e9 + 2.0, 5.0e8 + 2.0],
    );
    let q = measured(&mesh(&b));
    assert_eq!(q.shell_status(), ShellStatus::Certified);
    assert!(q.volume_mm3().unwrap().contains(8.0));
    assert!(q.area_mm2().unwrap().contains(24.0));
    for (e, want) in q
        .centroid_mm()
        .unwrap()
        .iter()
        .zip([1.0e9 + 1.0, -1.0e9 + 1.0, 5.0e8 + 1.0])
    {
        assert!(e.contains(want), "{e:?}");
    }
}

// ---------------------------------------------------------------- cancellation

#[test]
fn measurement_cancellation_at_every_poll_returns_nothing() {
    let slab = mesh(&voxels(&ring(12, 12, &[]), 1.0, [0.0; 3]));
    assert!(slab.triangles().len() > 512);
    let a = analyze(&slab).unwrap();
    let mut stages = Vec::new();
    measure_with_cancel(&slab, &a, |s| {
        stages.push(s);
        false
    })
    .unwrap();
    assert_eq!(stages[0], Stage::Validation);
    assert!(stages[1..].iter().all(|s| *s == Stage::Quantities));
    assert!(stages.len() >= 3);
    for (k, stage) in stages.iter().enumerate() {
        let mut n = 0usize;
        let e = measure_with_cancel(&slab, &a, |_| {
            let hit = n == k;
            n += 1;
            hit
        })
        .unwrap_err();
        assert_eq!((e.code, e.stage), (MeshCode::Cancelled, *stage));
        assert_eq!(n, k + 1);
    }
    // A refused mesh polls once, at entry.
    let mut open = cuboid([0.0; 3], [1.0; 3]);
    open.t.pop();
    let m = mesh(&open);
    let a = analyze(&m).unwrap();
    let mut n = 0;
    measure_with_cancel(&m, &a, |_| {
        n += 1;
        false
    })
    .unwrap();
    assert_eq!(n, 1);
}

// ---------------------------------------------------------------- G02 cross-check

#[test]
fn sealed_plates_agree_with_the_independent_mesh_check() {
    for (w, l, t) in [
        (1000.0, 500.0, 10.0),
        (1.0, 1.0, 1.0),
        (23.0, 47.0, 3.0),
        (0.001, 0.001, 0.001),
        (1_000_000.0, 1_000_000.0, 1_000_000.0),
        (0.1, 0.7, 123.456),
    ] {
        let plate = rectangular_plate(RectangularPlate {
            width_mm: w,
            length_mm: l,
            thickness_mm: t,
        })
        .unwrap();
        let c = cross_check_plate(&plate).unwrap();
        assert_eq!(c.shell, ShellStatus::Certified, "{w} {l} {t}");
        assert_eq!(c.analytic_status, QuantityStatus::Current);
        assert_eq!(c.analytic_volume_mm3, plate.volume_mm3());
        assert!(c.agrees, "{w} {l} {t}: {c:?}");
        assert!(c.analytic_enclosure_mm3.contains(plate.volume_mm3()));
        // Independent closed form of the exact box volume (f64 rounding only).
        let exact = (w * l) * t;
        assert!(near(c.mesh_volume_mm3.unwrap(), exact, 1e-14));
    }
}

#[test]
fn a_plate_blob_round_trips_byte_for_byte() {
    let plate = rectangular_plate(RectangularPlate {
        width_mm: 1000.0,
        length_mm: 500.0,
        thickness_mm: 10.0,
    })
    .unwrap();
    let parsed = TriMesh::from_blob(plate.mesh().blob()).unwrap();
    assert_eq!(parsed.to_blob().as_slice(), plate.mesh().blob().as_slice());
    assert_eq!(parsed.face_count(), 6);
    let q = measured(&parsed);
    assert!(q.volume_mm3().unwrap().contains(5_000_000.0));
    assert!(
        q.area_mm2()
            .unwrap()
            .contains(2.0 * (500_000.0 + 10_000.0 + 5_000.0))
    );
}
