// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! G07 reproducible timing and determinism harness for the first kernel scope.
//!
//! Run in release mode, twice, and compare the `DIGEST` lines to see byte identity across
//! processes:
//!
//! ```text
//! cargo run --release -p beamcraft-kernel --example g07_bench
//! ```
//!
//! `TIME` lines are wall-clock measurements on the machine that ran them (minimum, median and
//! maximum of the repeated calls). They are evidence of the order of magnitude on that machine
//! only, never a performance promise. `DIGEST` lines are SHA-256 digests of the sealed BCMESH01
//! blobs and carry no timing. `OUTCOME` lines record the typed result of work that is expected
//! to stop at a bound. The harness uses only the public kernel API and `std`, and exits with a
//! nonzero status when an expectation about the outcome (not about time) is not met.

use beamcraft_geom::Point3;
use beamcraft_kernel::measure::measure;
use beamcraft_kernel::ops::{
    Body, CutPlane, HoleCutter, Keep, OpLimits, plane_cut, through_hole, through_hole_with,
};
use beamcraft_kernel::profile::{Cavity, CurveSpec, Point2, extrude};
use beamcraft_kernel::topology::analyze;
use beamcraft_kernel::trimesh::TriMesh;
use beamcraft_kernel::verdict::ShellStatus;
use beamcraft_kernel::{RectangularPlate, StraightMember, rectangular_plate, straight_member};
use beamcraft_model::digest::Sha256;
use std::hint::black_box;
use std::process::ExitCode;
use std::time::{Duration, Instant};

fn digest_of(bytes: &[u8]) -> String {
    Sha256::digest(bytes).map_or_else(|_| "digest-error".to_owned(), |d| d.to_string())
}

fn fail(what: &str) -> ExitCode {
    eprintln!("EXPECTATION FAILED: {what}");
    ExitCode::from(2)
}

/// Runs `work` once to warm up, then `iterations` times, and prints min, median and max.
fn time<R>(name: &str, iterations: u32, mut work: impl FnMut() -> R) -> R {
    let mut last = black_box(work());
    let mut samples: Vec<Duration> = Vec::new();
    for _ in 0..iterations.max(1) {
        let start = Instant::now();
        last = black_box(work());
        samples.push(start.elapsed());
    }
    samples.sort();
    let pick = |q: usize| samples.get(q).copied().unwrap_or_default();
    let micros = |d: Duration| d.as_secs_f64() * 1e6;
    println!(
        "TIME {name}: n={} min={:.1}us median={:.1}us max={:.1}us",
        samples.len(),
        micros(pick(0)),
        micros(pick(samples.len() / 2)),
        micros(pick(samples.len().saturating_sub(1))),
    );
    last
}

fn p2(x: f64, y: f64) -> Point2 {
    Point2::new(x, y)
}

fn rect(w: f64, d: f64) -> Vec<Point2> {
    vec![p2(0.0, 0.0), p2(w, 0.0), p2(w, d), p2(0.0, d)]
}

fn circle(x: f64, y: f64, r: f64, segments: u32) -> Option<Cavity> {
    // The sagitta bound is loose on purpose: the fixed edge count is what is being timed.
    let spec = CurveSpec {
        max_sagitta_mm: r,
        min_segments: segments,
        max_segments: segments,
    };
    Cavity::circle(p2(x, y), r, spec).ok()
}

/// A closed welded slab of `w x d` cells (10 mm each, `th` thick) at offset `base`.
fn slab(w: u32, d: u32, th: f64, base: f64) -> (Vec<[f64; 3]>, Vec<[u32; 3]>) {
    let stride = w + 1;
    let at = |i: u32, j: u32, top: u32| (top * stride * (d + 1)) + j * stride + i;
    let mut v = Vec::new();
    for top in 0..2u32 {
        for j in 0..=d {
            for i in 0..=w {
                v.push([
                    base + f64::from(i) * 10.0,
                    base + f64::from(j) * 10.0,
                    base + if top == 0 { 0.0 } else { th },
                ]);
            }
        }
    }
    let mut t = Vec::new();
    for j in 0..d {
        for i in 0..w {
            let (a, b, c, e) = (
                at(i, j, 0),
                at(i + 1, j, 0),
                at(i + 1, j + 1, 0),
                at(i, j + 1, 0),
            );
            t.push([a, e, c]);
            t.push([a, c, b]);
            let (a, b, c, e) = (
                at(i, j, 1),
                at(i + 1, j, 1),
                at(i + 1, j + 1, 1),
                at(i, j + 1, 1),
            );
            t.push([a, b, c]);
            t.push([a, c, e]);
        }
    }
    for j in 0..d {
        let (p, q) = (at(0, j, 0), at(0, j + 1, 0));
        let (pt, qt) = (at(0, j, 1), at(0, j + 1, 1));
        t.push([p, pt, qt]);
        t.push([p, qt, q]);
        let (p, q) = (at(w, j, 0), at(w, j + 1, 0));
        let (pt, qt) = (at(w, j, 1), at(w, j + 1, 1));
        t.push([p, q, qt]);
        t.push([p, qt, pt]);
    }
    for i in 0..w {
        let (p, q) = (at(i, 0, 0), at(i + 1, 0, 0));
        let (pt, qt) = (at(i, 0, 1), at(i + 1, 0, 1));
        t.push([p, q, qt]);
        t.push([p, qt, pt]);
        let (p, q) = (at(i, d, 0), at(i + 1, d, 0));
        let (pt, qt) = (at(i, d, 1), at(i + 1, d, 1));
        t.push([p, qt, q]);
        t.push([p, pt, qt]);
    }
    (v, t)
}

/// A closed bipyramid with `n` equatorial vertices: long thin triangles whose boxes overlap.
fn bipyramid(n: u32, radius: f64) -> (Vec<[f64; 3]>, Vec<[u32; 3]>) {
    let mut v = Vec::new();
    for k in 0..n {
        // Octant-exact points are not needed: any finite ring gives a closed shell.
        let angle = core::f64::consts::TAU * f64::from(k) / f64::from(n);
        v.push([radius * angle.cos(), radius * angle.sin(), 0.0]);
    }
    let (top, bottom) = (n, n + 1);
    v.push([0.0, 0.0, radius]);
    v.push([0.0, 0.0, -radius]);
    let mut t = Vec::new();
    for k in 0..n {
        let next = (k + 1) % n;
        t.push([k, next, top]);
        t.push([next, k, bottom]);
    }
    (v, t)
}

fn describe(status: ShellStatus) -> String {
    format!("{status:?}")
}

fn main() -> ExitCode {
    println!(
        "MACHINE os={} arch={}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    println!(
        "MACHINE threads={} debug_assertions={}",
        std::thread::available_parallelism().map_or(0, usize::from),
        cfg!(debug_assertions)
    );

    // 1. Plate and member builders.
    let plate_input = RectangularPlate {
        width_mm: 1000.0,
        length_mm: 500.0,
        thickness_mm: 10.0,
    };
    let Ok(plate) = rectangular_plate(plate_input) else {
        return fail("plate builds");
    };
    let _ = time("plate 1000x500x10 build", 20_000, || {
        rectangular_plate(plate_input)
    });
    println!("DIGEST plate {}", plate.mesh().mesh_digest());
    let member_input = StraightMember {
        start: Point3::new(0.0, 0.0, 0.0),
        end: Point3::new(3000.0, 4000.0, 0.0),
        width_mm: 100.0,
        height_mm: 300.0,
        roll_rad: None,
        placement: beamcraft_model::command::SectionPlacement::default(),
    };
    let Ok(member) = straight_member(member_input) else {
        return fail("member builds");
    };
    let _ = time("member 3-4-5 diagonal build", 20_000, || {
        straight_member(member_input)
    });
    println!("DIGEST member {}", member.mesh().mesh_digest());

    // 2. Profile extrusion with cavities. The extrusion time covers admission, triangulation,
    // build and the self-checks: an `Extrusion` exists only after every check passed, so it is
    // the cost of certifying the body, not only of building it.
    let outer = rect(1000.0, 500.0);
    for (name, count, segments, iterations) in [
        ("extrude 1 round cavity (64 edges)", 1_u32, 64_u32, 500_u32),
        ("extrude 16 round cavities (32 edges)", 16, 32, 100),
        ("extrude 128 round cavities (8 edges)", 128, 8, 10),
        ("extrude 64 round cavities (1020 edges)", 64, 1020, 3),
    ] {
        let mut cavities = Vec::new();
        for k in 0..count {
            let (col, row) = (k % 16, k / 16);
            let (x, y) = (40.0 + 60.0 * f64::from(col), 40.0 + 60.0 * f64::from(row));
            let Some(c) = circle(x, y, 10.0, segments) else {
                return fail("cavity builds");
            };
            cavities.push(c);
        }
        let Ok(result) = extrude(&outer, &cavities, 10.0) else {
            return fail("extrusion with cavities builds");
        };
        let _ = time(name, iterations, || extrude(&outer, &cavities, 10.0));
        println!("DIGEST {name} {}", digest_of(result.mesh().blob()));
    }

    // 3. G06 certification of a body at the raised F06 caps: the same sealed 64-hole plate the
    // caps exist for, analyzed the way the operations certify a candidate. The default budgets
    // stop the scan with a typed `Uncertain`; that is the kernel certification capacity, a
    // separate implementation limit (`GEO_KERNEL_CAPACITY`), reported and never hidden.
    {
        let mut cavities = Vec::new();
        for k in 0..64_u32 {
            let (col, row) = (k % 16, k / 16);
            let Some(c) = circle(
                40.0 + 60.0 * f64::from(col),
                40.0 + 60.0 * f64::from(row),
                10.0,
                1020,
            ) else {
                return fail("cap-body cavity builds");
            };
            cavities.push(c);
        }
        let Ok(result) = extrude(&outer, &cavities, 10.0) else {
            return fail("cap-body extrusion builds");
        };
        let blob = result.mesh().blob().to_vec();
        println!("SIZE cap-body blob bytes={}", blob.len());
        let Ok(mesh) = TriMesh::from_blob(&blob) else {
            return fail("cap-body blob parses");
        };
        let analysis = time("analyze cap body (64 round holes, 1020 edges)", 2, || {
            analyze(&mesh)
        });
        let Ok(a) = analysis else {
            return fail("cap-body analysis returns a verdict");
        };
        println!("OUTCOME cap body status {}", describe(a.status()));
    }

    // 4. Plane cut.
    let Ok(body) = Body::from_plate(&plate) else {
        return fail("body from plate");
    };
    let oblique = CutPlane::new([300.0, 200.0, 5.0], [1.0, 0.4, 0.3]);
    let Ok(cut) = plane_cut(&body, &oblique, Keep::Positive, 0) else {
        return fail("oblique cut");
    };
    let _ = time(
        "plane cut of a plate (oblique, hexagonal section)",
        500,
        || plane_cut(&body, &oblique, Keep::Positive, 0),
    );
    println!("DIGEST plane-cut {}", cut.mesh_digest());

    // 5. Through hole(s).
    let Some(round) = circle(500.0, 250.0, 20.0, 64) else {
        return fail("hole cavity");
    };
    let cutter = HoleCutter::new(round, -1.0, 11.0, 0);
    let Ok(hole) = through_hole(&body, &cutter) else {
        return fail("through hole");
    };
    let _ = time("through hole (64-gon) in a plate", 500, || {
        through_hole(&body, &cutter)
    });
    println!("DIGEST through-hole {}", hole.mesh_digest());

    // 6. Sequences of holes on the result body: every call re-certifies the whole body, so the
    // cost grows with the body, and the default budgets end the sequence with a typed failure.
    for segments in [16_u32, 32, 64] {
        let mut current: Body = match through_hole(&body, &cutter) {
            Ok(first) => first.into_body(),
            Err(_) => return fail("first hole of the sequence"),
        };
        let mut accepted = 1_u32;
        let mut ordinal: u16 = 1;
        let started = Instant::now();
        loop {
            let (col, row) = (u32::from(ordinal) % 14, u32::from(ordinal) / 14);
            let Some(cavity) = circle(
                40.0 + 60.0 * f64::from(col),
                40.0 + 60.0 * f64::from(row),
                15.0,
                segments,
            ) else {
                return fail("sequence cavity");
            };
            let next = HoleCutter::new(cavity, -1.0, 11.0, ordinal);
            let start = Instant::now();
            match through_hole(&current, &next) {
                Ok(result) => {
                    accepted += 1;
                    println!(
                        "TIME hole sequence ({segments} edges) call {ordinal}: {:.1}ms triangles_out={}",
                        start.elapsed().as_secs_f64() * 1e3,
                        result.stats().triangles_out
                    );
                    current = result.into_body();
                }
                Err(failure) => {
                    println!(
                        "TIME hole sequence ({segments} edges) refused call {ordinal}: {:.1}ms",
                        start.elapsed().as_secs_f64() * 1e3
                    );
                    println!(
                        "OUTCOME hole sequence ({segments} edges) refused call {ordinal}: {} ({:?})",
                        failure.code.as_str(),
                        failure.defect
                    );
                    // The refusal is the certification budget, not the geometry: the same call
                    // with the G06 budgets raised sixteen-fold is sealed.
                    let mut relaxed = OpLimits::DEFAULT;
                    relaxed.analysis.max_candidate_pairs *= 16;
                    relaxed.analysis.max_exact_predicates *= 16;
                    relaxed.max_exact_predicates *= 16;
                    let start = Instant::now();
                    let again = through_hole_with(&current, &next, &relaxed, |_| false);
                    println!(
                        "TIME hole sequence ({segments} edges) call {ordinal} with 16x budgets: {:.1}ms",
                        start.elapsed().as_secs_f64() * 1e3
                    );
                    println!(
                        "OUTCOME hole sequence ({segments} edges) call {ordinal} with 16x budgets: {}",
                        again
                            .as_ref()
                            .map_or_else(|f| f.code.as_str(), |_| "sealed")
                    );
                    break;
                }
            }
            ordinal += 1;
            if ordinal > 40 {
                println!(
                    "OUTCOME hole sequence ({segments} edges) reached 40 calls without a bound"
                );
                break;
            }
        }
        println!("OUTCOME hole sequence ({segments} edges) holes accepted: {accepted}");
        println!(
            "TIME hole sequence ({segments} edges) total: {:.1}s",
            started.elapsed().as_secs_f64()
        );
    }

    // 7. Analysis and measurement of welded slabs of growing size (all closed and outward).
    // The size at which certification stops being decided within its default budgets is a
    // property of the budgets, not of the mesh: it is reported, never hidden.
    let mut largest_certified: Option<(u32, TriMesh)> = None;
    for cells in [4_u32, 8, 12, 16, 20, 24, 32, 44] {
        let (sv, st) = slab(cells, cells, 10.0, 0.0);
        let Ok(mesh) = TriMesh::from_buffers(&sv, &st) else {
            return fail("slab buffers");
        };
        let label = format!("analyze slab {cells}x{cells} ({} triangles)", st.len());
        let analysis = time(&label, 3, || analyze(&mesh));
        let Ok(analysis) = analysis else {
            return fail("slab analysis returns a verdict");
        };
        println!(
            "OUTCOME slab {cells}x{cells} triangles={} vertices={} status {}",
            st.len(),
            sv.len(),
            describe(analysis.status())
        );
        if analysis.is_certified() {
            largest_certified = Some((cells, mesh));
        } else if cells <= 8 {
            return fail("a small welded slab is certified");
        }
    }
    let Some((cells, mesh)) = largest_certified else {
        return fail("some slab is certified");
    };
    let Ok(analysis) = analyze(&mesh) else {
        return fail("analysis of the largest certified slab");
    };
    let measurement = time(&format!("measure slab {cells}x{cells}"), 3, || {
        measure(&mesh, &analysis)
    });
    println!("OUTCOME slab measurement ok={}", measurement.is_ok());
    let blob = mesh.to_blob();
    println!("SIZE slab blob bytes={}", blob.len());
    let _ = time("parse slab blob (structural gate)", 200, || {
        TriMesh::from_blob(&blob)
    });
    println!("DIGEST slab-blob {}", digest_of(&blob));

    // Structural gate alone at the F06 caps: 262,144 vertices, 524,288 triangles,
    // 13,631,520 bytes (the slab here sits just inside them).
    let (fv, ft) = slab(360, 360, 10.0, 0.0);
    let Ok(cap_mesh) = TriMesh::from_buffers(&fv, &ft) else {
        return fail("cap-size buffers");
    };
    let cap_blob = cap_mesh.to_blob();
    println!("SIZE cap-size blob bytes={}", cap_blob.len());
    let _ = time("parse near-cap blob (structural gate only)", 200, || {
        TriMesh::from_blob(&cap_blob)
    });

    let (fv, ft) = slab(16, 16, 10.0, 300_000_000.0);
    let Ok(far) = TriMesh::from_buffers(&fv, &ft) else {
        return fail("far slab buffers");
    };
    let far_analysis = time("analyze a 16x16 slab at 3e8 mm from the origin", 3, || {
        analyze(&far)
    });
    if let Ok(a) = &far_analysis {
        println!("OUTCOME far slab 16x16 status {}", describe(a.status()));
    }

    // 8. Adversarial work: overlapping boxes up to the pair budget.
    for (n, iterations) in [(200_u32, 3_u32), (1000, 3), (4000, 3)] {
        let (bv, bt) = bipyramid(n, 1000.0);
        let Ok(bm) = TriMesh::from_buffers(&bv, &bt) else {
            return fail("bipyramid buffers");
        };
        let label = format!("analyze bipyramid n={n} ({} triangles)", bt.len());
        let outcome = time(&label, iterations, || analyze(&bm));
        match outcome {
            Ok(a) => println!("OUTCOME bipyramid n={n} status {}", describe(a.status())),
            Err(e) => println!("OUTCOME bipyramid n={n} error {}", e.code.as_str()),
        }
    }

    ExitCode::SUCCESS
}
