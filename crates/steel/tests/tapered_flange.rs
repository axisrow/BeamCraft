// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! The tapered-flange families (`tapered_i`, `tapered_c`) against the published standards.
//!
//! Every row below was re-entered by hand from the official standard texts (no table copied
//! from any commercial source):
//!
//! * GB/T 706-2016, tables A.1 (45 I-beams) and A.2 (41 channels) of the official English
//!   edition (ChineseStandard.net translation of the standard, public registry entry
//!   openstd.samr.gov.cn); slope 1:6 for I-beams and 1:10 for channels per the standard's own
//!   section figures, `t` the middle-leg flange thickness, `r` the root fillet, `r1` the leg-end
//!   arc, both marked "for hole design, not for delivery condition".
//! * GOST 8239-89 (17 beams) and GOST 8240-89 table 1, the slope series (18 channels); both
//!   print a slope RANGE for the inner flange faces (6-12 % and 4-10 %) as a roll-pass
//!   reference, not a delivery dimension, and GOST 8239 names `t` the mean flange thickness.
//!
//! The acceptance is owner decision D3: expressing a published row through the family admits it
//! and the declared catalogue area never raises the D3 warning against the exact area of the
//! section (a relative gap above 1 percent). The sweep keeps a tighter drift bound (0.5 percent)
//! so a real formula or transcription error cannot hide inside the D3 margin.

use steel::engine::member::declared_section_area_warning;
use steel::kernel::section::{Representation, SectionMember, outline, section_member};
use steel::model::command::SectionPlacement;
use steel::model::definition::{SectionProfileSpec, SectionProperties};
use steel::model::section::{ParametricSection, SectionFamily, SectionShape};

/// One published row: designation, then h, b, s (web), t (mid-leg), r (root), r1 (toe) in mm
/// and the declared cross-sectional area in cm2, exactly as printed.
type Row<'a> = (&'a str, [f64; 7]);

/// Slope of the GB/T 706-2016 I-beam inner flange faces: 1:6.
const GB_I_SLOPE: f64 = 1.0 / 6.0;
/// Slope of the GB/T 706-2016 channel inner flange faces: 1:10.
const GB_C_SLOPE: f64 = 0.1;

/// Tighter-than-D3 drift bound of the GB/T 706 rows, whose slope the standard prints exactly.
const GB_DRIFT_BOUND: f64 = 0.005;
/// Drift bound of the GOST rows: the standards print a slope RANGE (a roll-pass reference, not
/// a delivery dimension), so the slope the published areas were computed at is unknown inside
/// the range and the mid-range sweep keeps a wider, still sub-D3 bound.
const GOST_DRIFT_BOUND: f64 = 0.008;

fn tapered(family: SectionFamily, row: &[f64; 7], slope: f64) -> ParametricSection {
    ParametricSection::new(
        family,
        vec![row[0], row[1], row[2], row[3], row[4], row[5], slope],
    )
    .unwrap()
}

fn declared_warning(
    family: SectionFamily,
    row: &[f64; 7],
    slope: f64,
    area_cm2: f64,
) -> Option<steel::model::definition::DeclaredAreaWarning> {
    let spec = SectionProfileSpec {
        shape: SectionShape::Parametric(tapered(family, row, slope)),
        section: SectionProperties {
            designation: None,
            standard: None,
            area_mm2: Some(area_cm2 * 100.0),
            mass_per_meter_kg: None,
            surface_per_meter_m2: None,
            display_name: None,
            area_convention: steel::model::section::AreaConvention::ExactFilleted,
        },
    };
    declared_section_area_warning(&spec)
}

/// One swept row: admitted, the exact area within `drift_bound` of the published value and the
/// D3 warning silent.
fn sweep(family: SectionFamily, rows: &[Row<'_>], slope: f64, drift_bound: f64) -> usize {
    let mut checked = 0;
    for (designation, row) in rows {
        let section = tapered(family, row, slope);
        section
            .validate()
            .unwrap_or_else(|e| panic!("{designation}: {e:?}"));
        let exact = section.exact_area_mm2();
        let declared = row[6] * 100.0;
        let gap = (declared - exact).abs() / declared;
        assert!(
            gap <= drift_bound,
            "{designation}: exact {exact} vs published {declared} (gap {:.3} %)",
            gap * 100.0
        );
        assert!(
            declared_warning(family, row, slope, row[6]).is_none(),
            "{designation}: the declared area warned"
        );
        checked += 1;
    }
    checked
}

/// GB/T 706-2016 table A.1, the 45 I-beam designations. Rows of one a/b/c group share the
/// group's t, r and r1 (merged cells in the standard).
const GB_I_BEAMS: [Row<'static>; 45] = [
    ("10", [100.0, 68.0, 4.5, 7.6, 6.5, 3.3, 14.33]),
    ("12", [120.0, 74.0, 5.0, 8.4, 7.0, 3.5, 17.80]),
    ("12.6", [126.0, 74.0, 5.0, 8.4, 7.0, 3.5, 18.10]),
    ("14", [140.0, 80.0, 5.5, 9.1, 7.5, 3.8, 21.50]),
    ("16", [160.0, 88.0, 6.0, 9.9, 8.0, 4.0, 26.11]),
    ("18", [180.0, 94.0, 6.5, 10.7, 8.5, 4.3, 30.74]),
    ("20a", [200.0, 100.0, 7.0, 11.4, 9.0, 4.5, 35.55]),
    ("20b", [200.0, 102.0, 9.0, 11.4, 9.0, 4.5, 39.55]),
    ("22a", [220.0, 110.0, 7.5, 12.3, 9.5, 4.8, 42.10]),
    ("22b", [220.0, 112.0, 9.5, 12.3, 9.5, 4.8, 46.50]),
    ("24a", [240.0, 116.0, 8.0, 13.0, 10.0, 5.0, 47.71]),
    ("24b", [240.0, 118.0, 10.0, 13.0, 10.0, 5.0, 52.51]),
    ("25a", [250.0, 116.0, 8.0, 13.0, 10.0, 5.0, 48.51]),
    ("25b", [250.0, 118.0, 10.0, 13.0, 10.0, 5.0, 53.51]),
    ("27a", [270.0, 122.0, 8.5, 13.7, 10.5, 5.3, 54.52]),
    ("27b", [270.0, 124.0, 10.5, 13.7, 10.5, 5.3, 59.92]),
    ("28a", [280.0, 122.0, 8.5, 13.7, 10.5, 5.3, 55.37]),
    ("28b", [280.0, 124.0, 10.5, 13.7, 10.5, 5.3, 60.97]),
    ("30a", [300.0, 126.0, 9.0, 14.4, 11.0, 5.5, 61.22]),
    ("30b", [300.0, 128.0, 11.0, 14.4, 11.0, 5.5, 67.22]),
    ("30c", [300.0, 130.0, 13.0, 14.4, 11.0, 5.5, 73.22]),
    ("32a", [320.0, 130.0, 9.5, 15.0, 11.5, 5.8, 67.12]),
    ("32b", [320.0, 132.0, 11.5, 15.0, 11.5, 5.8, 73.52]),
    ("32c", [320.0, 134.0, 13.5, 15.0, 11.5, 5.8, 79.92]),
    ("36a", [360.0, 136.0, 10.0, 15.8, 12.0, 6.0, 76.44]),
    ("36b", [360.0, 138.0, 12.0, 15.8, 12.0, 6.0, 83.64]),
    ("36c", [360.0, 140.0, 14.0, 15.8, 12.0, 6.0, 90.84]),
    ("40a", [400.0, 142.0, 10.5, 16.5, 12.5, 6.3, 86.07]),
    ("40b", [400.0, 144.0, 12.5, 16.5, 12.5, 6.3, 94.07]),
    ("40c", [400.0, 146.0, 14.5, 16.5, 12.5, 6.3, 102.1]),
    ("45a", [450.0, 150.0, 11.5, 18.0, 13.5, 6.8, 102.4]),
    ("45b", [450.0, 152.0, 13.5, 18.0, 13.5, 6.8, 111.4]),
    ("45c", [450.0, 154.0, 15.5, 18.0, 13.5, 6.8, 120.4]),
    ("50a", [500.0, 158.0, 12.0, 20.0, 14.0, 7.0, 119.2]),
    ("50b", [500.0, 160.0, 14.0, 20.0, 14.0, 7.0, 129.2]),
    ("50c", [500.0, 162.0, 16.0, 20.0, 14.0, 7.0, 139.2]),
    ("55a", [550.0, 166.0, 12.5, 21.0, 14.5, 7.3, 134.1]),
    ("55b", [550.0, 168.0, 14.5, 21.0, 14.5, 7.3, 145.1]),
    ("55c", [550.0, 170.0, 16.5, 21.0, 14.5, 7.3, 156.1]),
    ("56a", [560.0, 166.0, 12.5, 21.0, 14.5, 7.3, 135.4]),
    ("56b", [560.0, 168.0, 14.5, 21.0, 14.5, 7.3, 146.6]),
    ("56c", [560.0, 170.0, 16.5, 21.0, 14.5, 7.3, 157.8]),
    ("63a", [630.0, 176.0, 13.0, 22.0, 15.0, 7.5, 154.6]),
    ("63b", [630.0, 178.0, 15.0, 22.0, 15.0, 7.5, 167.2]),
    ("63c", [630.0, 180.0, 17.0, 22.0, 15.0, 7.5, 179.8]),
];

/// GB/T 706-2016 table A.2, the 41 channel designations.
const GB_CHANNELS: [Row<'static>; 41] = [
    ("5", [50.0, 37.0, 4.5, 7.0, 7.0, 3.5, 6.925]),
    ("6.3", [63.0, 40.0, 4.8, 7.5, 7.5, 3.8, 8.446]),
    ("6.5", [65.0, 40.0, 4.3, 7.5, 7.5, 3.8, 8.292]),
    ("8", [80.0, 43.0, 5.0, 8.0, 8.0, 4.0, 10.24]),
    ("10", [100.0, 48.0, 5.3, 8.5, 8.5, 4.2, 12.74]),
    ("12", [120.0, 53.0, 5.5, 9.0, 9.0, 4.5, 15.36]),
    ("12.6", [126.0, 53.0, 5.5, 9.0, 9.0, 4.5, 15.69]),
    ("14a", [140.0, 58.0, 6.0, 9.5, 9.5, 4.8, 18.51]),
    ("14b", [140.0, 60.0, 8.0, 9.5, 9.5, 4.8, 21.31]),
    ("16a", [160.0, 63.0, 6.5, 10.0, 10.0, 5.0, 21.95]),
    ("16b", [160.0, 65.0, 8.5, 10.0, 10.0, 5.0, 25.15]),
    ("18a", [180.0, 68.0, 7.0, 10.5, 10.5, 5.2, 25.69]),
    ("18b", [180.0, 70.0, 9.0, 10.5, 10.5, 5.2, 29.29]),
    ("20a", [200.0, 73.0, 7.0, 11.0, 11.0, 5.5, 28.83]),
    ("20b", [200.0, 75.0, 9.0, 11.0, 11.0, 5.5, 32.83]),
    ("22a", [220.0, 77.0, 7.0, 11.5, 11.5, 5.8, 31.83]),
    ("22b", [220.0, 79.0, 9.0, 11.5, 11.5, 5.8, 36.23]),
    ("24a", [240.0, 78.0, 7.0, 12.0, 12.0, 6.0, 34.21]),
    ("24b", [240.0, 80.0, 9.0, 12.0, 12.0, 6.0, 39.01]),
    ("24c", [240.0, 82.0, 11.0, 12.0, 12.0, 6.0, 43.81]),
    ("25a", [250.0, 78.0, 7.0, 12.0, 12.0, 6.0, 34.91]),
    ("25b", [250.0, 80.0, 9.0, 12.0, 12.0, 6.0, 39.91]),
    ("25c", [250.0, 82.0, 11.0, 12.0, 12.0, 6.0, 44.91]),
    ("27a", [270.0, 82.0, 7.5, 12.5, 12.5, 6.2, 39.27]),
    ("27b", [270.0, 84.0, 9.5, 12.5, 12.5, 6.2, 44.67]),
    ("27c", [270.0, 86.0, 11.5, 12.5, 12.5, 6.2, 50.07]),
    ("28a", [280.0, 82.0, 7.5, 12.5, 12.5, 6.2, 40.02]),
    ("28b", [280.0, 84.0, 9.5, 12.5, 12.5, 6.2, 45.62]),
    ("28c", [280.0, 86.0, 11.5, 12.5, 12.5, 6.2, 51.22]),
    ("30a", [300.0, 85.0, 7.5, 13.5, 13.5, 6.8, 43.89]),
    ("30b", [300.0, 87.0, 9.5, 13.5, 13.5, 6.8, 49.89]),
    ("30c", [300.0, 89.0, 11.5, 13.5, 13.5, 6.8, 55.89]),
    ("32a", [320.0, 88.0, 8.0, 14.0, 14.0, 7.0, 48.50]),
    ("32b", [320.0, 90.0, 10.0, 14.0, 14.0, 7.0, 54.90]),
    ("32c", [320.0, 92.0, 12.0, 14.0, 14.0, 7.0, 61.30]),
    ("36a", [360.0, 96.0, 9.0, 16.0, 16.0, 8.0, 60.89]),
    ("36b", [360.0, 98.0, 11.0, 16.0, 16.0, 8.0, 68.09]),
    ("36c", [360.0, 100.0, 13.0, 16.0, 16.0, 8.0, 75.29]),
    ("40a", [400.0, 100.0, 10.5, 18.0, 18.0, 9.0, 75.04]),
    ("40b", [400.0, 102.0, 12.5, 18.0, 18.0, 9.0, 83.04]),
    ("40c", [400.0, 104.0, 14.5, 18.0, 18.0, 9.0, 91.04]),
];

#[test]
fn the_gbt706_i_beam_table_sweeps_without_the_declared_area_warning() {
    assert_eq!(
        sweep(
            SectionFamily::TaperedI,
            &GB_I_BEAMS,
            GB_I_SLOPE,
            GB_DRIFT_BOUND
        ),
        45
    );
}

#[test]
fn the_gbt706_channel_table_sweeps_without_the_declared_area_warning() {
    assert_eq!(
        sweep(
            SectionFamily::TaperedC,
            &GB_CHANNELS,
            GB_C_SLOPE,
            GB_DRIFT_BOUND
        ),
        41
    );
}

/// GOST 8239-89, the 17 beams of the assortment, as printed.
const GOST_BEAMS: [Row<'static>; 17] = [
    ("10", [100.0, 55.0, 4.5, 7.2, 7.0, 2.5, 12.0]),
    ("12", [120.0, 64.0, 4.8, 7.3, 7.5, 3.0, 14.7]),
    ("14", [140.0, 73.0, 4.9, 7.5, 8.0, 3.0, 17.4]),
    ("16", [160.0, 81.0, 5.0, 7.8, 8.5, 3.5, 20.2]),
    ("18", [180.0, 90.0, 5.1, 8.1, 9.0, 3.5, 23.4]),
    ("20", [200.0, 100.0, 5.2, 8.4, 9.5, 4.0, 26.8]),
    ("22", [220.0, 110.0, 5.4, 8.7, 10.0, 4.0, 30.6]),
    ("24", [240.0, 115.0, 5.6, 9.5, 10.5, 4.0, 34.8]),
    ("27", [270.0, 125.0, 6.0, 9.8, 11.0, 4.5, 40.2]),
    ("30", [300.0, 135.0, 6.5, 10.2, 12.0, 5.0, 46.5]),
    ("33", [330.0, 140.0, 7.0, 11.2, 13.0, 5.0, 53.8]),
    ("36", [360.0, 145.0, 7.5, 12.3, 14.0, 6.0, 61.9]),
    ("40", [400.0, 155.0, 8.3, 13.0, 15.0, 6.0, 72.6]),
    ("45", [450.0, 160.0, 9.0, 14.2, 16.0, 7.0, 84.7]),
    ("50", [500.0, 170.0, 10.0, 15.2, 17.0, 7.0, 100.0]),
    ("55", [550.0, 180.0, 11.0, 16.5, 18.0, 7.0, 118.0]),
    ("60", [600.0, 190.0, 12.0, 17.8, 20.0, 8.0, 138.0]),
];

/// GOST 8240-89 table 1, the slope-series channels, as printed.
const GOST_CHANNELS: [Row<'static>; 18] = [
    ("5", [50.0, 32.0, 4.4, 7.0, 6.0, 2.5, 6.16]),
    ("6.5", [65.0, 36.0, 4.4, 7.2, 6.0, 2.5, 7.51]),
    ("8", [80.0, 40.0, 4.5, 7.4, 6.5, 2.5, 8.98]),
    ("10", [100.0, 46.0, 4.5, 7.6, 7.0, 3.0, 10.90]),
    ("12", [120.0, 52.0, 4.8, 7.8, 7.5, 3.0, 13.30]),
    ("14", [140.0, 58.0, 4.9, 8.1, 8.0, 3.0, 15.60]),
    ("16", [160.0, 64.0, 5.0, 8.4, 8.5, 3.5, 18.10]),
    ("16a", [160.0, 68.0, 5.0, 9.0, 8.5, 3.5, 19.50]),
    ("18", [180.0, 70.0, 5.1, 8.7, 9.0, 3.5, 20.70]),
    ("18a", [180.0, 74.0, 5.1, 9.3, 9.0, 3.5, 22.20]),
    ("20", [200.0, 76.0, 5.2, 9.0, 9.5, 4.0, 23.40]),
    ("22", [220.0, 82.0, 5.4, 9.5, 10.0, 4.0, 26.70]),
    ("24", [240.0, 90.0, 5.6, 10.0, 10.5, 4.0, 30.60]),
    ("27", [270.0, 95.0, 6.0, 10.5, 11.0, 4.5, 35.20]),
    ("30", [300.0, 100.0, 6.5, 11.0, 12.0, 5.0, 40.50]),
    ("33", [330.0, 105.0, 7.0, 11.7, 13.0, 5.0, 46.50]),
    ("36", [360.0, 110.0, 7.5, 12.6, 14.0, 6.0, 53.40]),
    ("40", [400.0, 115.0, 8.0, 13.5, 15.0, 6.0, 61.50]),
];

#[test]
fn the_gost_tables_sweep_without_the_warning_across_the_printed_slope_ranges() {
    // Both standards name the slope a roll-pass reference in a range (8239: 6-12 %, 8240:
    // 4-10 %), not a delivery dimension. Every row stays warning-free at the middle of the
    // range and at both extremes, so any slope the standard allows expresses the row.
    for (family, rows, mid, low, high) in [
        (SectionFamily::TaperedI, &GOST_BEAMS[..], 0.10, 0.06, 0.12),
        (
            SectionFamily::TaperedC,
            &GOST_CHANNELS[..],
            0.07,
            0.04,
            0.10,
        ),
    ] {
        assert_eq!(
            sweep(family, rows, mid, GOST_DRIFT_BOUND),
            rows.len(),
            "{family:?}"
        );
        for slope in [low, high] {
            for (designation, row) in rows {
                assert!(
                    declared_warning(family, row, slope, row[6]).is_none(),
                    "{family:?} {designation} at slope {slope}: the declared area warned"
                );
            }
        }
    }
}

#[test]
fn the_tapered_outline_carries_the_slope_symmetry_and_toe_radius() {
    use beamcraft_kernel::profile::Point2;

    let area = |o: &steel::kernel::section::SectionOutline| -> f64 {
        let doubled: f64 = o
            .outer
            .iter()
            .zip(o.outer.iter().cycle().skip(1))
            .map(|(p, q)| p.x * q.y - q.x * p.y)
            .sum();
        doubled / 2.0
    };
    let bounds = |o: &steel::kernel::section::SectionOutline| {
        o.outer
            .iter()
            .fold(([f64::MAX; 2], [f64::MIN; 2]), |(lo, hi), p| {
                (
                    [lo[0].min(p.x), lo[1].min(p.y)],
                    [hi[0].max(p.x), hi[1].max(p.y)],
                )
            })
    };
    for (family, values, slope) in [
        (
            SectionFamily::TaperedI,
            [100.0, 68.0, 4.5, 7.6, 6.5, 3.3, GB_I_SLOPE],
            GB_I_SLOPE,
        ),
        (
            SectionFamily::TaperedC,
            [50.0, 37.0, 4.5, 7.0, 7.0, 3.5, GB_C_SLOPE],
            GB_C_SLOPE,
        ),
    ] {
        let section = ParametricSection::new(family, values.to_vec()).unwrap();
        let o = outline(&SectionShape::Parametric(section), Representation::Detailed).unwrap();
        // The bounding box is the published h by b, centred on the origin.
        let (lo, hi) = bounds(&o);
        assert!((hi[0] - lo[0] - values[1]).abs() < 1e-9, "{family:?}");
        assert!((hi[1] - lo[1] - values[0]).abs() < 1e-9, "{family:?}");
        assert!(hi[0] + lo[0] < 1e-9 && hi[1] + lo[1] < 1e-9, "{family:?}");
        // The outline holds the parameter's slope: the longest oblique edge of the polygon is
        // an inner flange face inclined at atan(k) (its tangent is k within the chord
        // discretisation).
        let (mut length, mut tangent) = (0.0f64, 0.0f64);
        for (p, q) in o.outer.iter().zip(o.outer.iter().cycle().skip(1)) {
            let (dx, dy) = (q.x - p.x, q.y - p.y);
            if (dx * dx + dy * dy).sqrt() > length && dx.abs() > 1e-9 && dy.abs() > 1e-9 {
                length = (dx * dx + dy * dy).sqrt();
                tangent = (dy / dx).abs();
            }
        }
        assert!(
            (tangent - slope).abs() < 0.01,
            "{family:?}: flange face tangent {tangent} vs slope {slope}"
        );
        // Doubly symmetric (I) or mirror-symmetric about the mid-height (C): for every vertex
        // the mirrored one exists within the arc discretisation.
        let mirrored = |p: &Point2| {
            Point2::new(
                if family == SectionFamily::TaperedI {
                    -p.x
                } else {
                    p.x
                },
                -p.y,
            )
        };
        for p in &o.outer {
            let m = mirrored(p);
            assert!(
                o.outer
                    .iter()
                    .any(|q| (q.x - m.x).abs() < 1e-6 && (q.y - m.y).abs() < 1e-6),
                "{family:?}: {p:?} has no mirrored vertex"
            );
        }
        // The radii are present: the chord polygon adds the root-fillet material over the sharp
        // trapezoid outline and overshoots the true arcs by no more than the discretisation.
        let exact = ParametricSection::new(family, values.to_vec())
            .unwrap()
            .exact_area_mm2();
        let sharp = {
            let section = ParametricSection::new(family, values.to_vec())
                .unwrap()
                .without_radius();
            outline(&SectionShape::Parametric(section), Representation::Detailed).unwrap()
        };
        let polygon = area(&o);
        assert!(
            polygon > area(&sharp),
            "{family:?}: polygon {polygon} sharp {}",
            area(&sharp)
        );
        assert!(
            polygon > exact && polygon - exact < 0.01 * exact,
            "{family:?}: polygon {polygon} exact {exact}"
        );
        // A member of the section is a sealed certified body in both representations, flagged
        // approximated in both: the detailed one approximates the fillet arcs and the
        // simplified one drops them (the not-exact contract of Simplified).
        for representation in [Representation::Detailed, Representation::Simplified] {
            let member = SectionMember {
                start: beamcraft_geom::Point3::ORIGIN,
                end: beamcraft_geom::Point3::new(0.0, 0.0, 1000.0),
                shape: SectionShape::Parametric(
                    ParametricSection::new(family, values.to_vec()).unwrap(),
                ),
                representation,
                roll_rad: None,
                placement: SectionPlacement::default(),
            };
            let built = section_member(&member).unwrap_or_else(|e| panic!("{family:?}: {e:?}"));
            assert!(built.volume_mm3() > 0.0);
            assert!(built.approximated(), "{family:?} {representation:?}");
        }
        // A sharp tapered section stays exact when detailed and flags its dropped fillets when
        // simplified, exactly like the parallel-flange families.
        let mut sharp = values;
        sharp[4] = 0.0;
        sharp[5] = 0.0;
        for (representation, expected) in [
            (Representation::Detailed, false),
            (Representation::Simplified, true),
        ] {
            let o = outline(
                &SectionShape::Parametric(ParametricSection::new(family, sharp.to_vec()).unwrap()),
                representation,
            )
            .unwrap();
            assert_eq!(o.approximated, expected, "{family:?} {representation:?}");
        }
    }
}

#[test]
fn degenerate_tapered_parameters_are_refused_before_any_point() {
    use beamcraft_model::error::ErrorCode;
    // A slope that consumes the toe thickness, a toe radius that does not fit it, and a root
    // radius that does not fit the flange are `INVALID_ARGUMENT` naming the parameter; the
    // kernel outline reports the same refusal as a shape failure.
    let cases: [(SectionFamily, [f64; 7], ErrorCode); 3] = [
        (
            SectionFamily::TaperedI,
            [100.0, 68.0, 4.5, 7.6, 6.5, 3.3, 3.0],
            ErrorCode::InvalidArgument,
        ),
        (
            SectionFamily::TaperedI,
            [100.0, 68.0, 4.5, 7.6, 6.5, 5.0, GB_I_SLOPE],
            ErrorCode::InvalidArgument,
        ),
        (
            SectionFamily::TaperedC,
            [50.0, 37.0, 4.5, 7.0, 33.0, 3.5, 0.1],
            ErrorCode::InvalidArgument,
        ),
    ];
    for (family, values, code) in cases {
        let section = ParametricSection::new(family, values.to_vec()).unwrap();
        let error = section.validate().unwrap_err();
        assert_eq!(error.code, code, "{family:?} {values:?}");
        assert!(
            matches!(
                outline(&SectionShape::Parametric(section), Representation::Detailed),
                Err(steel::kernel::section::SectionFailure::Shape(_))
            ),
            "{family:?}: the outline built from refused values"
        );
    }
}

#[test]
fn a_zero_slope_tapered_section_is_the_parallel_flange_family() {
    // With the slope and the toe radius at zero the tapered family is the parallel-flange one:
    // the same exact area, and with sharp corners the identical polygon. With radii the two
    // polygons discretise the same arcs at their own chord counts, so both stay within half a
    // percent of the exact area and within one percent of each other.
    let polygon = |section: &ParametricSection| -> f64 {
        let o = outline(
            &SectionShape::Parametric(section.clone()),
            Representation::Detailed,
        )
        .unwrap();
        o.outer
            .iter()
            .zip(o.outer.iter().cycle().skip(1))
            .map(|(p, q)| p.x * q.y - q.x * p.y)
            .sum::<f64>()
            / 2.0
    };
    for (tapered_family, values, parallel_family, parallel_values) in [
        (
            SectionFamily::TaperedI,
            vec![298.0, 149.0, 5.5, 8.0, 13.0, 0.0, 0.0],
            SectionFamily::IShape,
            vec![298.0, 149.0, 5.5, 8.0, 13.0],
        ),
        (
            SectionFamily::TaperedC,
            vec![200.0, 80.0, 6.0, 10.0, 10.0, 0.0, 0.0],
            SectionFamily::Channel,
            vec![200.0, 80.0, 6.0, 10.0, 10.0],
        ),
        (
            SectionFamily::TaperedI,
            vec![300.0, 150.0, 7.0, 10.0, 0.0, 0.0, 0.0],
            SectionFamily::IShape,
            vec![300.0, 150.0, 7.0, 10.0, 0.0],
        ),
        (
            SectionFamily::TaperedC,
            vec![200.0, 80.0, 6.0, 10.0, 0.0, 0.0, 0.0],
            SectionFamily::Channel,
            vec![200.0, 80.0, 6.0, 10.0, 0.0],
        ),
    ] {
        let tapered = ParametricSection::new(tapered_family, values.clone()).unwrap();
        let parallel = ParametricSection::new(parallel_family, parallel_values).unwrap();
        let a = tapered.exact_area_mm2();
        let b = parallel.exact_area_mm2();
        assert!((a - b).abs() <= 1e-9 * b, "{tapered_family:?}: {a} vs {b}");
        let (pa, pb) = (polygon(&tapered), polygon(&parallel));
        if values[4] == 0.0 {
            assert!(
                (pa - pb).abs() <= 1e-9 * pb,
                "{tapered_family:?}: {pa} vs {pb}"
            );
        } else {
            assert!(
                (pa - b).abs() <= 0.005 * b && (pb - b).abs() <= 0.005 * b,
                "{tapered_family:?}: {pa} vs {pb} around {b}"
            );
        }
    }
}
