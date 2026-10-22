// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Independent check of the hand-written F07 expectations: volume, axis length, mass and
//! bounds are recomputed from the fixture inputs by closed-form vector algebra (Rodrigues'
//! rotation and frame algebra, no production math helpers) and compared with
//! `tests/fixtures/expectations/f07.json`. The generator computes no expected values, so the
//! comparison legs are independent by construction; the committed digests, outcomes and
//! directory exactness are verified by the `verify` command tests in `cli.rs`.

// Test-only file: helper code outside the assertions may panic on setup failures.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::fs;
use std::path::{Path, PathBuf};

use beamcraft_fixtures::json::{self, Json, ParseLimits};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn committed_expectations() -> PathBuf {
    repo_root()
        .join("tests")
        .join("fixtures")
        .join("expectations")
        .join("f07.json")
}

fn load(path: &Path) -> Json {
    let limits = ParseLimits {
        max_bytes: 1 << 20,
        max_depth: 16,
        max_values: 1 << 16,
        max_string_bytes: 1 << 16,
        max_number_chars: 32,
        max_members: 256,
    };
    json::parse(&fs::read(path).unwrap(), &limits).unwrap()
}

fn member_of<'a>(value: &'a Json, name: &str) -> &'a Json {
    value.get(name).unwrap()
}

fn f(value: &Json, name: &str) -> f64 {
    member_of(value, name).as_f64().unwrap()
}

/// An optional expectation: JSON `null` must stay `None` on the recomputed side too.
fn optional(value: &Json, name: &str) -> Option<f64> {
    match member_of(value, name) {
        Json::Null => None,
        other => other.as_f64(),
    }
}

fn num_at(value: &Json, i: usize) -> f64 {
    value.as_array().unwrap()[i].as_f64().unwrap()
}

fn vec3(value: &Json, name: &str) -> [f64; 3] {
    let v = member_of(value, name);
    [num_at(v, 0), num_at(v, 1), num_at(v, 2)]
}

// ---- Vector algebra -------------------------------------------------------------------------

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

/// Rodrigues' rotation of `p` about `axis` by `degrees`, right-hand rule.
fn rotate(p: [f64; 3], axis: [f64; 3], degrees: f64) -> [f64; 3] {
    let n = norm(axis);
    let k = [axis[0] / n, axis[1] / n, axis[2] / n];
    let (sin, cos) = degrees.to_radians().sin_cos();
    let cross_kp = cross(k, p);
    let dot_kp = dot(k, p);
    std::array::from_fn(|i| p[i] * cos + cross_kp[i] * sin + k[i] * dot_kp * (1.0 - cos))
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] + b[i])
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}

fn scale(a: [f64; 3], k: f64) -> [f64; 3] {
    std::array::from_fn(|i| a[i] * k)
}

// ---- Closed-form body quantities ---------------------------------------------------------------

/// (volume_mm3, mass_kg, bounds corners, axis_length_mm) from one fixture object.
fn recompute(obj: &Json) -> (f64, Option<f64>, Vec<[f64; 3]>, Option<f64>) {
    let density = obj.get("density").and_then(|d| d.as_f64());
    match member_of(obj, "kind").as_str().unwrap() {
        "plate" => {
            let size = member_of(obj, "size");
            let (sx, sy, sz) = (num_at(size, 0), num_at(size, 1), num_at(size, 2));
            let (axis, angle) = (
                vec3(obj.get("placement").unwrap(), "axis"),
                f(member_of(obj, "placement"), "angle"),
            );
            let origin = vec3(member_of(obj, "placement"), "origin");
            let corners: Vec<[f64; 3]> = {
                let mut out = Vec::new();
                for x in [0.0, sx] {
                    for y in [0.0, sy] {
                        for z in [0.0, sz] {
                            out.push(add(rotate([x, y, z], axis, angle), origin));
                        }
                    }
                }
                out
            };
            (
                sx * sy * sz,
                density.map(|d| d * sx * sy * sz * 1e-9),
                corners,
                None,
            )
        }
        _ => {
            let start = vec3(obj, "start");
            let end = vec3(obj, "end");
            let profile = member_of(obj, "profile");
            let (width, height) = (f(profile, "width"), f(profile, "height"));
            let axis = sub(end, start);
            let length = norm(axis);
            let x = scale(axis, 1.0 / length);
            let up = vec3(obj, "up");
            // Height direction: the declared up made orthogonal to the axis; width completes
            // a right-handed (x, y, z) frame.
            let height_dir = sub(up, scale(x, dot(up, x)));
            let z = scale(height_dir, 1.0 / norm(height_dir));
            let y = cross(z, x);
            let mut corners = Vec::new();
            for base in [start, end] {
                for sy in [-1.0, 1.0] {
                    for sz in [-1.0, 1.0] {
                        corners.push(add(
                            add(base, scale(y, sy * width / 2.0)),
                            scale(z, sz * height / 2.0),
                        ));
                    }
                }
            }
            (
                width * height * length,
                density.map(|d| d * width * height * length * 1e-9),
                corners,
                Some(length),
            )
        }
    }
}

fn bounds(corners: &[[f64; 3]]) -> ([f64; 3], [f64; 3]) {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for c in corners {
        for i in 0..3 {
            lo[i] = lo[i].min(c[i]);
            hi[i] = hi[i].max(c[i]);
        }
    }
    (lo, hi)
}

/// Compensated summation.
fn total(values: impl Iterator<Item = f64>) -> f64 {
    let (mut sum, mut corr) = (0.0, 0.0);
    for v in values {
        let adjusted = v - corr;
        let next = sum + adjusted;
        corr = (next - sum) - adjusted;
        sum = next;
    }
    sum
}

// ---- The comparison ----------------------------------------------------------------------------

/// Recomputed quantities of one accepted expectation object.
type Recomputed<'a> = (&'a str, f64, Option<f64>, Vec<[f64; 3]>);

/// Recomputes every accepted case against its expectations; one message per mismatch.
fn check_expectations(expectations: &Json, fixtures: &Path) -> Vec<String> {
    let mut failures = Vec::new();
    let mut fail = |format_args: std::fmt::Arguments| failures.push(format!("{format_args}"));
    let tol = member_of(expectations, "tolerances");
    let (volume_rel, mass_rel, abs_mm) = (
        f(tol, "volume_rel"),
        f(tol, "mass_rel"),
        f(tol, "length_and_bounds_abs_mm"),
    );
    let manifest = load(&fixtures.join("manifest.json"));
    let entries = member_of(&manifest, "fixtures").as_array().unwrap();
    let mut quantities = 0usize;
    for case in member_of(expectations, "cases").as_array().unwrap() {
        if member_of(case, "outcome").as_str().unwrap() != "accept" {
            continue; // Rejections are covered by the outcome tests in cli.rs.
        }
        let id = member_of(case, "id").as_str().unwrap();
        let path = entries
            .iter()
            .find(|e| member_of(e, "id").as_str() == Some(id))
            .map(|e| fixtures.join(member_of(e, "path").as_str().unwrap()))
            .unwrap_or_else(|| panic!("no manifest entry for {id}"));
        let fixture = load(&path);
        let objects = member_of(&fixture, "objects").as_array().unwrap();
        let find = |key: &str| {
            objects
                .iter()
                .find(|o| member_of(o, "key").as_str() == Some(key))
                .unwrap_or_else(|| panic!("{id}: no fixture object {key}"))
        };
        let mut results: Vec<Recomputed> = Vec::new();
        for exp in member_of(case, "objects").as_array().unwrap() {
            let key = member_of(exp, "key").as_str().unwrap();
            let (volume, mass, corners, axis_length) = recompute(find(key));
            results.push((key, volume, mass, corners.clone()));
            let where_ = format!("{id}/{key}");
            quantities += 1;
            if (volume - f(exp, "volume_mm3")).abs() > volume_rel * f(exp, "volume_mm3") {
                fail(format_args!(
                    "{where_}: volume {volume} != {}",
                    f(exp, "volume_mm3")
                ));
            }
            quantities += 1;
            let mass_ok = match (mass, optional(exp, "mass_kg")) {
                (None, None) => true,
                (Some(got), Some(want)) => (got - want).abs() <= mass_rel * want.abs().max(1e-300),
                _ => false,
            };
            if !mass_ok {
                fail(format_args!(
                    "{where_}: mass {mass:?} != {:?} (unknown must stay null)",
                    optional(exp, "mass_kg")
                ));
            }
            let (lo, hi) = bounds(&corners);
            for (half, corner) in [("min", lo), ("max", hi)] {
                let want = vec3(member_of(exp, "bounds_mm"), half);
                for (i, axis) in ["x", "y", "z"].iter().enumerate() {
                    quantities += 1;
                    if (corner[i] - want[i]).abs() > abs_mm {
                        fail(format_args!(
                            "{where_}: bounds.{half}.{axis} {} != {}",
                            corner[i], want[i]
                        ));
                    }
                }
            }
            match (
                axis_length,
                exp.get("axis_length_mm").map(|a| a.as_f64().unwrap()),
            ) {
                (None, None) => (),
                (Some(got), Some(want)) if (got - want).abs() <= abs_mm => {
                    quantities += 1;
                }
                (got, want) => {
                    quantities += 1;
                    fail(format_args!(
                        "{where_}: axis length {got:?} != {want:?} (members only)"
                    ))
                }
            }
        }
        let want_total = member_of(case, "total");
        let where_ = format!("{id}/total");
        quantities += 1;
        let volume_sum = total(results.iter().map(|(_, v, _, _)| *v));
        if (volume_sum - f(want_total, "volume_mm3")).abs()
            > volume_rel * f(want_total, "volume_mm3")
        {
            fail(format_args!(
                "{where_}: volume {volume_sum} != {}",
                f(want_total, "volume_mm3")
            ));
        }
        quantities += 1;
        let any_unknown = results.iter().any(|(_, _, m, _)| m.is_none());
        let mass_sum = (!any_unknown).then(|| total(results.iter().filter_map(|(_, _, m, _)| *m)));
        let total_want = optional(want_total, "mass_kg");
        let total_ok = match (mass_sum, total_want) {
            (None, None) => true,
            (Some(got), Some(want)) => (got - want).abs() <= mass_rel * want.abs().max(1e-300),
            _ => false,
        };
        if !total_ok {
            fail(format_args!(
                "{where_}: mass {mass_sum:?} != {total_want:?} (unknown must stay null)"
            ));
        }
        let union: Vec<[f64; 3]> = results
            .iter()
            .flat_map(|(_, _, _, c)| c.iter().copied())
            .collect();
        let (lo, hi) = bounds(&union);
        for (half, corner) in [("min", lo), ("max", hi)] {
            let want = vec3(member_of(want_total, "bounds_mm"), half);
            for (i, axis) in ["x", "y", "z"].iter().enumerate() {
                quantities += 1;
                if (corner[i] - want[i]).abs() > abs_mm {
                    fail(format_args!(
                        "{where_}: bounds.{half}.{axis} {} != {}",
                        corner[i], want[i]
                    ));
                }
            }
        }
        if let Some(sep) = case.get("separation") {
            let between = member_of(sep, "between").as_array().unwrap();
            let locate = |name: &Json| {
                results
                    .iter()
                    .find(|(k, ..)| *k == name.as_str().unwrap())
                    .map(|(_, _, _, corners)| bounds(corners))
                    .unwrap()
            };
            let (a, b) = (locate(&between[0]), locate(&between[1]));
            let i = match member_of(sep, "axis").as_str().unwrap() {
                "x" => 0,
                "y" => 1,
                _ => 2,
            };
            let gap = b.0[i] - a.1[i];
            quantities += 1;
            if (gap - f(sep, "gap_mm")).abs() > abs_mm {
                fail(format_args!(
                    "{id}: separation gap {gap} != {}",
                    f(sep, "gap_mm")
                ));
            }
        }
    }
    // The retired Python oracle checked 172 quantities on this inventory.
    assert_eq!(quantities, 172, "the quantity inventory changed");
    failures
}

/// Replaces one member of one expectation object and writes the result to a scratch file.
fn with_edit(edit: impl FnOnce(&Json) -> Json) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!(
        "f07-expectations-{}-{}.json",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    fs::write(
        &path,
        json::to_pretty_string(&edit(&load(&committed_expectations()))),
    )
    .unwrap();
    path
}

fn replace_member(value: &Json, name: &str, replacement: Json) -> Json {
    Json::Object(
        value
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| {
                if k == name {
                    (name.to_owned(), replacement.clone())
                } else {
                    (k.clone(), v.clone())
                }
            })
            .collect(),
    )
}

#[test]
fn the_committed_expectations_match_the_recomputation() {
    let fixtures = repo_root().join("tests").join("fixtures").join("f07");
    assert!(
        check_expectations(&load(&committed_expectations()), &fixtures).is_empty(),
        "the committed expectations disagree with the closed forms"
    );
}

#[test]
fn a_wrong_volume_is_rejected() {
    let path = with_edit(|data| {
        let cases: Vec<Json> = member_of(data, "cases")
            .as_array()
            .unwrap()
            .iter()
            .map(|case| {
                if member_of(case, "id").as_str() != Some("f07.member.along-x") {
                    return case.clone();
                }
                let objects: Vec<Json> = member_of(case, "objects")
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|o| {
                        if member_of(o, "key").as_str() != Some("member-1") {
                            o.clone()
                        } else {
                            replace_member(o, "volume_mm3", Json::Number(131_040_001.0))
                        }
                    })
                    .collect();
                replace_member(case, "objects", Json::Array(objects))
            })
            .collect();
        replace_member(data, "cases", Json::Array(cases))
    });
    let fixtures = repo_root().join("tests").join("fixtures").join("f07");
    let failures = check_expectations(&load(&path), &fixtures);
    assert!(
        failures
            .iter()
            .any(|m| m.contains("f07.member.along-x/member-1: volume")),
        "{failures:?}"
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn an_invented_mass_is_rejected() {
    let path = with_edit(|data| {
        let cases: Vec<Json> = member_of(data, "cases")
            .as_array()
            .unwrap()
            .iter()
            .map(|case| {
                if member_of(case, "id").as_str() != Some("f07.plate.rotated-z-30") {
                    return case.clone();
                }
                let objects: Vec<Json> = member_of(case, "objects")
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|o| {
                        if member_of(o, "key").as_str() != Some("plate-1") {
                            o.clone()
                        } else {
                            replace_member(o, "mass_kg", Json::Number(0.0))
                        }
                    })
                    .collect();
                replace_member(case, "objects", Json::Array(objects))
            })
            .collect();
        replace_member(data, "cases", Json::Array(cases))
    });
    let fixtures = repo_root().join("tests").join("fixtures").join("f07");
    let failures = check_expectations(&load(&path), &fixtures);
    assert!(
        failures
            .iter()
            .any(|m| m.contains("unknown must stay null")),
        "{failures:?}"
    );
    fs::remove_file(path).unwrap();
}
