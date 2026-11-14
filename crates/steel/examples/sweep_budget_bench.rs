// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Certification cost of sweep-shaped meshes at and beyond the sweep budget (#629, item 1).
//!
//! The 10 m / R 12 m arc of an I 300×150 r15 section needs about 4 896 vertices and 9 788
//! triangles, which the sweep budget (4 096 / 8 192) refuses. Before raising that budget, the
//! certification stage must be shown to decide such meshes. This harness measures, with the
//! public API and the default budgets only:
//!
//! 1. the real 8.4 m arc (the largest filleted I arc that builds today, 4 064 vertices,
//!    8 124 triangles) swept and sealed end to end, then analyzed and measured from its blob;
//! 2. straight I-prism mirrors at the sizes a longer arc would need: 128 rings (today's
//!    vertex cap edge, 4 096 / 8 188), 153 rings (the refused 10 m arc size, 4 896 / 9 788)
//!    and 256 rings (the proposed 8 192 / 16 384 budget edge, 8 192 / 16 380).
//!
//! The mirrors keep the sweep's certification-cost profile: one ring per path facet, wall
//! quads split into exactly coplanar triangle pairs, cap fans from the same ear-clipped
//! outline. Run it as (see `docs/design/g07-kernel-first-scope-acceptance.md` §7 for the
//! profile's history):
//!
//! ```text
//! cargo run --profile dist -p steel --example sweep_budget_bench
//! ```
//!
//! `TIME` lines are wall clock on the machine that ran them (min, median, max of the
//! repeated calls) — evidence of the order of magnitude there, never a promise. `OUTCOME`
//! lines record the typed certification verdict at the default budgets. The harness exits
//! nonzero when a structural expectation (not a time) is not met.

use beamcraft_geom::Point3;
use beamcraft_kernel::profile::{Point2, Profile};
use beamcraft_kernel::trimesh::TriMesh;
use beamcraft_kernel::{measure::measure, topology::analyze};
use beamcraft_model::digest::Sha256;
use std::hint::black_box;
use std::process::ExitCode;
use std::time::{Duration, Instant};
use steel::kernel::section::{Representation, outline};
use steel::kernel::sweep::{Sweep, SweepPath, sweep};
use steel::model::section::{ParametricSection, SectionFamily, SectionShape};

/// The I 300×150 r15 section of the #629 example, as the kernel admits it.
fn i_section() -> Option<SectionShape> {
    let parameters =
        ParametricSection::new(SectionFamily::IShape, vec![300.0, 150.0, 7.0, 10.0, 15.0]);
    parameters.ok().map(SectionShape::Parametric)
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
    let millis = |d: Duration| d.as_secs_f64() * 1e3;
    println!(
        "TIME {name}: n={} min={:.1}ms median={:.1}ms max={:.1}ms",
        samples.len(),
        millis(pick(0)),
        millis(pick(samples.len() / 2)),
        millis(pick(samples.len().saturating_sub(1))),
    );
    last
}

fn digest_of(bytes: &[u8]) -> String {
    Sha256::digest(bytes).map_or_else(|_| "digest-error".to_owned(), |d| d.to_string())
}

/// Builds a straight prism over `outer` with `rings` rings spaced `ring_mm` apart along +X,
/// mirroring the sweep mesh: every ring is the section polygon, walls are quads split into
/// coplanar pairs, the caps reuse the profile's own ear-clipping triangulation.
fn prism(
    outer: &[Point2],
    caps: &[[u32; 3]],
    rings: u32,
    ring_mm: f64,
) -> (Vec<[f64; 3]>, Vec<[u32; 3]>) {
    let points = outer.len();
    let stride = |ring: u32, point: usize| ring * points as u32 + point as u32;
    let mut vertices = Vec::with_capacity(rings as usize * points);
    for ring in 0..rings {
        for point in outer {
            vertices.push([f64::from(ring) * ring_mm, point.y, point.x]);
        }
    }
    let mut triangles = Vec::with_capacity((rings as usize - 1) * points * 2 + caps.len() * 2);
    for ring in 0..rings - 1 {
        for point in 0..points {
            let next = (point + 1) % points;
            let (a, a2, b, b2) = (
                stride(ring, point),
                stride(ring, next),
                stride(ring + 1, point),
                stride(ring + 1, next),
            );
            triangles.push([a, b2, a2]);
            triangles.push([a, b, b2]);
        }
    }
    let last = rings - 1;
    for cap in caps {
        let [a, b, c] = *cap;
        triangles.push([a, b, c]);
        let base = last * points as u32;
        triangles.push([base + a, base + c, base + b]);
    }
    (vertices, triangles)
}

/// Analyzes and measures one mesh, printing the verdict line. Returns `false` on a
/// structural failure (the buffers do not parse or the verdict is missing).
fn certify(name: &str, mesh: &TriMesh) -> bool {
    let analysis = time(&format!("analyze {name}"), 3, || analyze(mesh));
    let Ok(analysis) = analysis else {
        eprintln!("EXPECTATION FAILED: {name} analysis returns a verdict");
        return false;
    };
    println!(
        "OUTCOME {name}: vertices={} triangles={} status {:?}",
        mesh.vertices().len(),
        mesh.triangles().len(),
        analysis.status()
    );
    if analysis.is_certified() {
        let measured = time(&format!("measure {name}"), 3, || measure(mesh, &analysis));
        println!("OUTCOME measure {name} ok={}", measured.is_ok());
    }
    true
}

fn main() -> ExitCode {
    println!(
        "MACHINE os={} arch={} threads={} debug_assertions={}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::thread::available_parallelism().map_or(0, usize::from),
        cfg!(debug_assertions)
    );
    let shape = match i_section() {
        Some(shape) => shape,
        None => return fail("the I section admits"),
    };

    // 1. The real filleted I arc: the 8.4 m chord at R 12 m is the longest one that fits the
    // sweep budget at 0.05 mm today (127 rings x 32 outline points). Sweeping covers the
    // whole seal path (path faceting, walk, mesh, G09 certification); the blob then goes
    // through the standalone certification and measurement the way a reader sees it.
    let Ok(arc) = sweep(&Sweep {
        path: SweepPath::Arc {
            start: Point3::new(0.0, 0.0, 0.0),
            on: Point3::new(4200.0, 758.7803, 0.0),
            end: Point3::new(8400.0, 0.0, 0.0),
        },
        shape: shape.clone(),
        representation: Representation::Detailed,
        chord_deviation_mm: 0.05,
    }) else {
        return fail("the 8.4 m arc sweeps within the budget");
    };
    let blob = arc.blob();
    println!(
        "SIZE arc-8.4m blob bytes={} digest {}",
        blob.len(),
        digest_of(blob)
    );
    let Ok(mesh) = TriMesh::from_blob(blob) else {
        return fail("the 8.4 m arc blob parses");
    };
    if !certify("arc-8.4m", &mesh) {
        return ExitCode::from(2);
    }

    // 2. Straight I-prism mirrors at the sizes a longer arc would need. The outline and the
    // cap triangulation come from the kernel itself, so the mirror has the sweep's own
    // point count and cap fans.
    let Ok(out) = outline(&shape, Representation::Detailed) else {
        return fail("the I outline resolves");
    };
    let Ok(profile) = Profile::admit(&out.outer, &out.cavities) else {
        return fail("the I profile admits");
    };
    let Ok(caps) = profile.triangulate() else {
        return fail("the I profile triangulates");
    };
    let points = out.outer.len();
    println!("SIZE outline points={points} cap triangles={}", caps.len());
    for rings in [128_u32, 153, 256] {
        let (vertices, triangles) = prism(&out.outer, &caps, rings, 100.0);
        let label = format!("prism-{rings}r");
        println!(
            "SIZE {label} vertices={} triangles={}",
            vertices.len(),
            triangles.len()
        );
        let Ok(mesh) = TriMesh::from_buffers(&vertices, &triangles) else {
            return fail(&format!("{label} buffers parse"));
        };
        if !certify(&label, &mesh) {
            return ExitCode::from(2);
        }
    }

    ExitCode::SUCCESS
}
