// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The committed G11 set is exactly what the generator writes, and its references are
//! self-consistent and checked against hand-derived values.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_fixtures::g11::{self, REPOSITORY_DIR};
use beamcraft_fixtures::rational::Rational;
use std::path::{Path, PathBuf};

fn committed() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(REPOSITORY_DIR)
}

fn expectation_rows() -> Vec<Vec<String>> {
    let (_, bytes) = g11::generate()
        .unwrap()
        .into_iter()
        .find(|(name, _)| *name == "expectations.tsv")
        .unwrap();
    String::from_utf8(bytes)
        .unwrap()
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(|l| l.split('\t').map(str::to_owned).collect())
        .collect()
}

fn reference(rows: &[Vec<String>], id: &str, column: usize) -> Rational {
    let row = rows.iter().find(|r| r[0] == id).unwrap();
    Rational::parse_fraction(row[column].split_once(' ').unwrap().0).unwrap()
}

fn f(x: f64) -> Rational {
    Rational::from_f64(x).unwrap()
}

#[test]
fn the_committed_set_is_the_generated_set() {
    let problems = g11::verify(&committed()).unwrap();
    assert!(problems.is_empty(), "{problems:?}");
}

#[test]
fn a_changed_reference_fails_verification() {
    let dir = std::env::temp_dir().join(format!("g11-mutation-{}", std::process::id()));
    g11::write(&dir).unwrap();
    let path = dir.join("expectations.tsv");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, text.replacen("ok\texact", "ok\tbounded", 1)).unwrap();
    let problems = g11::verify(&dir).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(
        problems,
        ["expectations.tsv: differs from the generator".to_owned()]
    );
}

#[test]
fn every_decimal_is_the_rounding_of_its_fraction() {
    let rows = expectation_rows();
    assert_eq!(rows.len(), g11::cases().len());
    let mut cells = 0;
    for row in &rows {
        for column in &row[4..] {
            if column == "-" {
                continue;
            }
            for cell in column.split(',') {
                let (fraction, decimal) = cell.split_once(' ').unwrap();
                let value = Rational::parse_fraction(fraction).unwrap();
                assert_eq!(value.to_scientific(21), decimal, "{}", row[0]);
                let parsed: f64 = decimal.parse().unwrap();
                assert!(parsed.is_finite(), "{}", row[0]);
                cells += 1;
            }
        }
    }
    assert!(cells > 200, "{cells} reference cells");
}

#[test]
fn hand_derived_references_agree() {
    let rows = expectation_rows();
    // A box of binary64 sides is the exact product of their rational values.
    assert_eq!(
        reference(&rows, "g11.mesh.plate_decimal_box", 4),
        f(0.1).mul(&f(0.3)).mul(&f(0.7))
    );
    // Its mass at 7850 kg/m3 is V * 7850 / 1e9.
    assert_eq!(
        reference(&rows, "g11.mesh.plate_decimal_box", 5),
        f(0.1)
            .mul(&f(0.3))
            .mul(&f(0.7))
            .mul(&f(7850.0))
            .div(&Rational::integer(1_000_000_000))
            .unwrap()
    );
    // The flat axis (3000, 4000, 0) is 5000 long; a 3-4-12 axis is 1300 long.
    assert_eq!(
        reference(&rows, "g11.mesh.member_rectangle", 7),
        Rational::integer(5000)
    );
    assert_eq!(
        reference(&rows, "g11.mesh.member_angle_sharp", 7),
        Rational::integer(1300)
    );
    // A sharp RHS is 2t(h + b - 2t), whatever the rearrangement.
    let (h, b, t) = (f(200.2), f(100.1), f(6.3));
    assert_eq!(
        reference(&rows, "g11.mesh.member_rhs_sharp", 6),
        h.mul(&b).sub(&h.sub(&t).sub(&t).mul(&b.sub(&t).sub(&t)))
    );
    // A translated rectangle's centroid is its centre moved by exactly the translation; a half
    // turn about Z negates x and y first.
    let centroid = |id: &str| -> Vec<Rational> {
        let row = rows.iter().find(|r| r[0] == id).unwrap();
        row[10]
            .split(',')
            .map(|c| Rational::parse_fraction(c.split_once(' ').unwrap().0).unwrap())
            .collect()
    };
    let half = |x: f64| f(x).div(&Rational::integer(2)).unwrap();
    assert_eq!(
        centroid("g11.far.plate_translated"),
        [
            half(300.3).add(&f(999_998_000.0)),
            half(700.7).add(&f(-999_999_000.0)),
            half(10.1).add(&f(999_999_000.0)),
        ]
    );
    assert_eq!(
        centroid("g11.far.plate_half_turn"),
        [
            half(300.3).neg().add(&f(999_999_000.0)),
            half(700.7).neg().add(&f(999_999_000.0)),
            half(10.1).add(&f(-999_999_990.0)),
        ]
    );
    // The subnormal class: refused below the normal range, current above it.
    let outcome = |id: &str| rows.iter().find(|r| r[0] == id).unwrap()[1].clone();
    for id in [
        "g11.subnormal.min_plate_tiny_density",
        "g11.subnormal.unit_plate_1e310",
        "g11.subnormal.largest_plate_smallest_density",
        "g11.subnormal.member_tiny_mass",
    ] {
        assert_eq!(outcome(id), "UNSUPPORTED_GEOMETRY", "{id}");
    }
    for id in [
        "g11.subnormal.min_normal_density",
        "g11.subnormal.normal_small_mass",
        "g11.subnormal.cap_density_min_plate",
        "g11.subnormal.zero_density",
    ] {
        assert_eq!(outcome(id), "ok", "{id}");
    }
}
