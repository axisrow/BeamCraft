// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic sections that follow no national standard. Expected areas are
// closed forms derived by hand (rectangles, quarter circles, centre-line length times thickness
// for constant-thickness bends), not from the code.

//! A07.1 section outlines and straight members of any section.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_geom::Point3;
use beamcraft_kernel::profile::Point2;
use beamcraft_model::error::ErrorCode;
use core::f64::consts::PI;
use steel::kernel::section::{
    Representation, SectionMember, SectionOutline, outline, section_member,
};
use steel::model::section::{CustomSection, ParametricSection, SectionFamily, SectionShape};

fn shape(family: SectionFamily, values: &[f64]) -> SectionShape {
    SectionShape::Parametric(ParametricSection::new(family, values.to_vec()).unwrap())
}

fn shoelace(points: &[Point2]) -> f64 {
    let n = points.len();
    (0..n)
        .map(|i| {
            let (p, q) = (points[i], points[(i + 1) % n]);
            p.x * q.y - q.x * p.y
        })
        .sum::<f64>()
        / 2.0
}

fn net_area(o: &SectionOutline) -> f64 {
    shoelace(&o.outer) + o.cavities.iter().map(|c| shoelace(c.points())).sum::<f64>()
}

fn close(actual: f64, expected: f64, relative: f64) {
    assert!(
        (actual - expected).abs() <= relative * expected.abs(),
        "{actual} vs {expected}"
    );
}

/// Area bound of quarter arcs approximated with vertices on the arc and the detailed sagitta
/// bound `s = max(0.25 mm, 0.005 r)`: every chord region is thinner than `s` and no longer than its arc, so the error of
/// one quarter arc of radius `r` is below `s * (pi/2) * r`.
fn arcs(radii: &[f64]) -> f64 {
    radii
        .iter()
        .map(|r| 0.25f64.max(0.005 * r) * PI / 2.0 * r)
        .sum()
}

fn within(actual: f64, expected: f64, bound: f64) {
    assert!(
        (actual - expected).abs() <= bound,
        "{actual} vs {expected} (bound {bound})"
    );
}

/// Quarter-circle corner area removed (convex) or added (concave fillet): `(1 - pi/4) r^2`.
fn corner(r: f64) -> f64 {
    (1.0 - PI / 4.0) * r * r
}

#[test]
fn hot_rolled_areas_match_closed_forms_in_both_representations() {
    let (h, b, tw, tf, r) = (300.0, 150.0, 7.0, 10.0, 15.0);
    let sharp = 2.0 * b * tf + (h - 2.0 * tf) * tw;
    let i = shape(SectionFamily::IShape, &[h, b, tw, tf, r]);
    let simple = outline(&i, Representation::Simplified).unwrap();
    assert_eq!(simple.outer.len(), 12);
    close(net_area(&simple), sharp, 1e-12);
    let detailed = outline(&i, Representation::Detailed).unwrap();
    assert!(detailed.outer.len() > 12 && detailed.approximated);
    within(net_area(&detailed), sharp + 4.0 * corner(r), arcs(&[r; 4]));
    assert_eq!((detailed.width_mm, detailed.height_mm), (b, h));

    let ch = shape(SectionFamily::Channel, &[200.0, 80.0, 6.0, 10.0, 10.0]);
    let sharp = 2.0 * 80.0 * 10.0 + 180.0 * 6.0;
    close(
        net_area(&outline(&ch, Representation::Simplified).unwrap()),
        sharp,
        1e-12,
    );
    within(
        net_area(&outline(&ch, Representation::Detailed).unwrap()),
        sharp + 2.0 * corner(10.0),
        arcs(&[10.0; 2]),
    );

    let angle = shape(SectionFamily::Angle, &[100.0, 80.0, 8.0, 10.0]);
    let sharp = 80.0 * 8.0 + 92.0 * 8.0;
    within(
        net_area(&outline(&angle, Representation::Detailed).unwrap()),
        sharp + corner(10.0),
        arcs(&[10.0]),
    );
}

#[test]
fn hollow_sections_have_one_clockwise_cavity() {
    let (h, b, t, r) = (200.0, 100.0, 6.0, 12.0);
    let rhs = outline(
        &shape(SectionFamily::Rhs, &[h, b, t, r]),
        Representation::Detailed,
    )
    .unwrap();
    assert_eq!(rhs.cavities.len(), 1);
    assert!(shoelace(rhs.cavities[0].points()) < 0.0);
    let expected =
        (b * h - 4.0 * corner(r)) - ((b - 2.0 * t) * (h - 2.0 * t) - 4.0 * corner(r - t));
    within(
        net_area(&rhs),
        expected,
        arcs(&[r, r, r, r, r - t, r - t, r - t, r - t]),
    );
    let sharp = outline(
        &shape(SectionFamily::Rhs, &[h, b, t, r]),
        Representation::Simplified,
    )
    .unwrap();
    close(
        net_area(&sharp),
        b * h - (b - 2.0 * t) * (h - 2.0 * t),
        1e-12,
    );

    let (d, t) = (168.0, 5.0);
    let chs = outline(
        &shape(SectionFamily::Chs, &[d, t]),
        Representation::Detailed,
    )
    .unwrap();
    within(
        net_area(&chs),
        PI / 4.0 * (d * d - (d - 2.0 * t).powi(2)),
        4.0 * arcs(&[d / 2.0, d / 2.0 - t]),
    );
    let coarse = outline(
        &shape(SectionFamily::Chs, &[d, t]),
        Representation::Simplified,
    )
    .unwrap();
    assert_eq!(coarse.outer.len(), 16);
    assert!(chs.outer.len() > coarse.outer.len());
}

#[test]
fn cold_formed_area_is_thickness_times_centre_line() {
    // Concentric bends of inner radius r and outer r + t: area = t * centre-line length, with the
    // centre line of each bend a quarter circle of radius r + t/2.
    let (h, b, c, t, r) = (150.0, 50.0, 15.0, 1.5, 2.0);
    let big = r + t;
    let arc = PI / 2.0 * (r + t / 2.0);
    let lipped = outline(
        &shape(SectionFamily::LippedC, &[h, b, c, t, r]),
        Representation::Detailed,
    )
    .unwrap();
    let centre = (h - 2.0 * big) + 2.0 * (b - 2.0 * big) + 2.0 * (c - big) + 4.0 * arc;
    within(net_area(&lipped), t * centre, arcs(&[big, r]) * 4.0);

    let plain = outline(
        &shape(SectionFamily::PlainC, &[h, b, t, r]),
        Representation::Detailed,
    )
    .unwrap();
    let centre = (h - 2.0 * big) + 2.0 * (b - big) + 2.0 * arc;
    within(net_area(&plain), t * centre, arcs(&[big, r]) * 2.0);

    let (h, b, c, t, r) = (200.0, 60.0, 18.0, 2.0, 3.0);
    let (big, arc) = (r + t, PI / 2.0 * (r + t / 2.0));
    let z = outline(
        &shape(SectionFamily::LippedZ, &[h, b, c, t, r]),
        Representation::Detailed,
    )
    .unwrap();
    let centre = (h - 2.0 * big) + 2.0 * (b - 2.0 * big) + 2.0 * (c - big) + 4.0 * arc;
    within(net_area(&z), t * centre, arcs(&[big, r]) * 4.0);
    assert_eq!(z.width_mm, 2.0 * b - t);

    let (h, b, c, t, r) = (40.0, 30.0, 20.0, 0.8, 1.0);
    let (big, arc) = (r + t, PI / 2.0 * (r + t / 2.0));
    let omega = outline(
        &shape(SectionFamily::Omega, &[h, b, c, t, r]),
        Representation::Detailed,
    )
    .unwrap();
    let centre = 2.0 * (c - big) + 2.0 * (h - 2.0 * big) + (b - 2.0 * big) + 4.0 * arc;
    within(net_area(&omega), t * centre, arcs(&[big, r]) * 4.0);

    // Sharp corners: the simplified outline is the plain polygon of the same section.
    let simple = outline(
        &shape(SectionFamily::LippedC, &[150.0, 50.0, 15.0, 1.5, 2.0]),
        Representation::Simplified,
    )
    .unwrap();
    assert_eq!(simple.outer.len(), 12);
    let t = 1.5;
    let sharp = 150.0 * t + 2.0 * (50.0 - t) * t + 2.0 * (15.0 - t) * t;
    close(net_area(&simple), sharp, 1e-12);
}

#[test]
fn every_outline_is_centred_on_its_bounding_box() {
    for (family, values) in [
        (SectionFamily::Channel, &[200.0, 80.0, 6.0, 10.0, 10.0][..]),
        (SectionFamily::Angle, &[100.0, 80.0, 8.0, 10.0]),
        (SectionFamily::LippedZ, &[200.0, 60.0, 18.0, 2.0, 3.0]),
        (SectionFamily::Omega, &[40.0, 30.0, 20.0, 0.8, 1.0]),
    ] {
        let o = outline(&shape(family, values), Representation::Detailed).unwrap();
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for p in &o.outer {
            lo = [lo[0].min(p.x), lo[1].min(p.y)];
            hi = [hi[0].max(p.x), hi[1].max(p.y)];
        }
        close(hi[0] - lo[0], o.width_mm, 1e-12);
        close(hi[1] - lo[1], o.height_mm, 1e-12);
        assert!(
            (hi[0] + lo[0]).abs() < 1e-9 && (hi[1] + lo[1]).abs() < 1e-9,
            "{family:?}"
        );
    }
}

fn beam(shape: SectionShape, representation: Representation) -> SectionMember {
    SectionMember {
        start: Point3::new(0.0, 0.0, 0.0),
        end: Point3::new(6000.0, 0.0, 0.0),
        shape,
        representation,
        roll_rad: None,
        placement: steel::model::command::SectionPlacement::default(),
    }
}

#[test]
fn a_horizontal_i_beam_is_a_certified_shell_in_the_member_frame() {
    let (h, b, tw, tf, r) = (300.0, 150.0, 7.0, 10.0, 15.0);
    let i = shape(SectionFamily::IShape, &[h, b, tw, tf, r]);
    let detailed = section_member(&beam(i.clone(), Representation::Detailed)).unwrap();
    let area = 2.0 * b * tf + (h - 2.0 * tf) * tw + 4.0 * corner(r);
    within(detailed.net_area_mm2(), area, arcs(&[r; 4]));
    close(
        detailed.volume_mm3(),
        detailed.net_area_mm2() * 6000.0,
        1e-12,
    );
    // Divergence-theorem volume of the sealed mesh, independent of the builder's product.
    let mesh = detailed.body().mesh();
    let v = mesh.vertices();
    let volume: f64 = mesh
        .triangles()
        .iter()
        .map(|&[a, b, c]| {
            let (a, b, c) = (v[a as usize], v[b as usize], v[c as usize]);
            (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0]))
                / 6.0
        })
        .sum();
    close(volume, detailed.volume_mm3(), 1e-9);
    // F06 section 4 default frame of a horizontal axis: width along world Y, height along Z.
    let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
    for p in v {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    close(hi[0] - lo[0], 6000.0, 1e-12);
    close(hi[1] - lo[1], b, 1e-9);
    close(hi[2] - lo[2], h, 1e-9);
    assert!(detailed.approximated());

    let simple = section_member(&beam(i.clone(), Representation::Simplified)).unwrap();
    assert!(simple.body().mesh().vertices().len() < v.len());
    assert_ne!(simple.mesh_digest(), detailed.mesh_digest());
    // Same input, same bytes.
    let again = section_member(&beam(i, Representation::Detailed)).unwrap();
    assert_eq!(again.blob(), detailed.blob());
}

#[test]
fn every_family_builds_a_member_in_both_representations() {
    for (family, values) in [
        (SectionFamily::Channel, &[200.0, 80.0, 6.0, 10.0, 10.0][..]),
        (SectionFamily::Angle, &[100.0, 80.0, 8.0, 10.0]),
        (SectionFamily::Rhs, &[200.0, 100.0, 6.0, 12.0]),
        (SectionFamily::Chs, &[168.0, 5.0]),
        (SectionFamily::LippedC, &[150.0, 50.0, 15.0, 1.5, 2.0]),
        (SectionFamily::PlainC, &[150.0, 50.0, 1.5, 2.0]),
        (SectionFamily::LippedZ, &[200.0, 60.0, 18.0, 2.0, 3.0]),
        (SectionFamily::Omega, &[40.0, 30.0, 20.0, 0.8, 1.0]),
    ] {
        for representation in [Representation::Detailed, Representation::Simplified] {
            let result = section_member(&beam(shape(family, values), representation));
            let result = result.unwrap_or_else(|e| panic!("{family:?} {representation:?}: {e:?}"));
            assert!(result.volume_mm3() > 0.0);
        }
    }
}

#[test]
fn custom_outlines_are_kept_and_refused_by_the_kernel() {
    let square = vec![[-50.0, -50.0], [50.0, -50.0], [50.0, 50.0], [-50.0, 50.0]];
    let custom = SectionShape::Custom(CustomSection {
        outer: square.clone(),
        cavities: vec![vec![
            [-10.0, -10.0],
            [-10.0, 10.0],
            [10.0, 10.0],
            [10.0, -10.0],
        ]],
    });
    let built = section_member(&beam(custom, Representation::Detailed)).unwrap();
    close(built.net_area_mm2(), 100.0 * 100.0 - 20.0 * 20.0, 1e-12);
    assert!(!built.approximated());

    let clockwise = SectionShape::Custom(CustomSection {
        outer: square.into_iter().rev().collect(),
        cavities: vec![],
    });
    let refused = section_member(&beam(clockwise, Representation::Detailed)).unwrap_err();
    assert_eq!(refused.f04_code(), ErrorCode::InvalidArgument);
}

#[test]
fn refusals_carry_the_f04_code_of_their_stage() {
    let bad = shape(SectionFamily::IShape, &[300.0, 150.0, 7.0, 200.0, 15.0]);
    assert_eq!(
        section_member(&beam(bad, Representation::Detailed))
            .unwrap_err()
            .f04_code(),
        ErrorCode::InvalidArgument
    );
    let nan = shape(SectionFamily::Chs, &[f64::NAN, 5.0]);
    assert_eq!(
        outline(&nan, Representation::Detailed)
            .unwrap_err()
            .f04_code(),
        ErrorCode::NonFiniteNumber
    );
    let mut short = beam(
        shape(SectionFamily::Chs, &[168.0, 5.0]),
        Representation::Detailed,
    );
    short.end = Point3::new(0.0, 0.0, 0.0);
    assert_eq!(
        section_member(&short).unwrap_err().f04_code(),
        ErrorCode::InvalidArgument
    );
}

#[test]
fn an_off_origin_custom_outline_is_centred_like_the_families() {
    // Square (0,0)-(100,100) with a cavity (20,20)-(40,40): the box centre (50,50) moves to the
    // origin, the cavity with it; the area is unchanged.
    let custom = SectionShape::Custom(CustomSection {
        outer: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]],
        cavities: vec![vec![[20.0, 20.0], [20.0, 40.0], [40.0, 40.0], [40.0, 20.0]]],
    });
    let o = outline(&custom, Representation::Detailed).unwrap();
    assert_eq!(o.outer[0], Point2::new(-50.0, -50.0));
    assert_eq!(o.cavities[0].points()[0], Point2::new(-30.0, -30.0));
    assert_eq!(net_area(&o), 100.0 * 100.0 - 20.0 * 20.0);
    let built = section_member(&beam(custom, Representation::Detailed)).unwrap();
    let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
    for p in built.body().mesh().vertices() {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    // Horizontal member along +X: the section spans Y and Z symmetrically about the axis.
    assert_eq!((lo[1], hi[1], lo[2], hi[2]), (-50.0, 50.0, -50.0, 50.0));
}
