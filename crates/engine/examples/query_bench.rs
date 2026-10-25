// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic scenes, MPL-2.0.

//! Timing harness for the view snapshot paths the viewport and the CLI use: building a snapshot
//! from kernel results, writing and re-reading its bundle (what `beamcraft export-view` sends and
//! the window rebuilds), and the ray, point and snap queries over it.
//!
//! ```text
//! cargo run --release -p beamcraft-engine --example query_bench -- [--iterations K] [SCENARIO...]
//! cargo run --release -p beamcraft-engine --example query_bench -- write-bundle SCENARIO FILE
//! cargo run --release -p beamcraft-engine --example query_bench -- from-bundle FILE [REPEAT]
//! ```
//!
//! A scenario is `copies:N` (N elements sharing one box plate mesh), `holed-copies:N` (N
//! elements sharing one plate with a 64-gon through hole), `synthetic:N` (the engine's varied
//! synthetic plates) or `synthetic-simplified:N` (the same with stand-in simplified meshes).
//! Each timed scenario prints one JSON line with the minimum, median and maximum of K runs in
//! milliseconds. `write-bundle` and `from-bundle` split one rebuild into its own process so an
//! external tool (DHAT, callgrind) can count exactly what `ViewSnapshot::from_bundle` does.
//! Times are wall clock on the machine that ran them: evidence for that machine only. The harness
//! uses only the public engine API and `std`.

use beamcraft_engine::query::feature::FeatureKind;
use beamcraft_engine::query::pick::{
    PickQuery, PointPick, RayPick, SnapKind, SnapQuery, pick, snap,
};
use beamcraft_engine::scene::SceneMesh;
use beamcraft_engine::solid::SolidMesh;
use beamcraft_engine::synthetic::plate_scene;
use beamcraft_engine::{ElementId, ElementKind, Limits, SceneElement, ViewPin, ViewSnapshot};
use beamcraft_geom::Translation3;
use beamcraft_kernel::ops::{Body, HoleCutter, through_hole};
use beamcraft_kernel::profile::{Cavity, CurveSpec, Point2};
use beamcraft_kernel::{RectangularPlate, rectangular_plate};
use beamcraft_model::digest::Sha256;
use std::hint::black_box;
use std::process::ExitCode;
use std::time::Instant;

type Run<T> = Result<T, String>;

fn fail<E: std::fmt::Debug>(what: &str) -> impl Fn(E) -> String + '_ {
    move |e| format!("{what}: {e:?}")
}

/// One shared mesh: a box plate, or a plate with a 64-gon through hole.
fn shared_mesh(holed: bool) -> Run<SceneMesh> {
    let plate = rectangular_plate(RectangularPlate {
        width_mm: 300.0,
        length_mm: 200.0,
        thickness_mm: 10.0,
    })
    .map_err(fail("plate"))?;
    if !holed {
        return Ok(SceneMesh::from(&plate));
    }
    let body = Body::from_plate(&plate).map_err(fail("body"))?;
    let spec = CurveSpec {
        max_sagitta_mm: 20.0,
        min_segments: 64,
        max_segments: 64,
    };
    let round = Cavity::circle(Point2::new(150.0, 100.0), 20.0, spec).map_err(fail("hole"))?;
    let hole = through_hole(&body, &HoleCutter::new(round, -1.0, 11.0, 0)).map_err(fail("cut"))?;
    Ok(SolidMesh::from_operation(&hole).scene_mesh())
}

fn id(ordinal: usize) -> Run<ElementId> {
    ElementId::parse(&format!("01990000-0000-7000-8000-{ordinal:012x}")).map_err(fail("id"))
}

/// `count` elements on a 100-column grid (pitch 400 mm) that all use `mesh`.
fn copies(count: usize, mesh: &SceneMesh) -> Run<Vec<SceneElement>> {
    (0..count)
        .map(|n| {
            let x = (n % 100) as f64 * 400.0;
            let y = (n / 100) as f64 * 400.0;
            let placement = Translation3::new(x, y, 0.0).map_err(fail("placement"))?;
            Ok(SceneElement::new(
                id(n)?,
                ElementKind::Plate,
                placement,
                mesh.clone(),
            ))
        })
        .collect()
}

fn scenario(text: &str) -> Run<Vec<SceneElement>> {
    let (name, count) = text
        .split_once(':')
        .ok_or_else(|| format!("scenario {text}: NAME:COUNT"))?;
    let count: u32 = count.parse().map_err(fail("count"))?;
    if count == 0 || count > 65_536 {
        return Err("count is 1..=65536".into());
    }
    let count_usize = usize::try_from(count).map_err(fail("count"))?;
    match name {
        "copies" => copies(count_usize, &shared_mesh(false)?),
        "holed-copies" => copies(count_usize, &shared_mesh(true)?),
        "synthetic" => plate_scene(count, false).map_err(fail("synthetic")),
        "synthetic-simplified" => plate_scene(count, true).map_err(fail("synthetic")),
        _ => Err(format!("unknown scenario {name}")),
    }
}

fn pin() -> Run<ViewPin> {
    Ok(ViewPin::new(
        Sha256::digest(b"query-bench").map_err(fail("digest"))?,
        1,
    ))
}

/// Runs `work` once to warm up, then `iterations` times; returns min, median and max in ms.
fn time<R>(iterations: u32, mut work: impl FnMut() -> Run<R>) -> Run<[f64; 3]> {
    black_box(work()?);
    let mut samples = Vec::new();
    for _ in 0..iterations.max(1) {
        let start = Instant::now();
        black_box(work()?);
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(f64::total_cmp);
    let at = |i: usize| samples.get(i).copied().unwrap_or(0.0);
    Ok([
        at(0),
        at(samples.len() / 2),
        at(samples.len().saturating_sub(1)),
    ])
}

fn triple(t: [f64; 3]) -> String {
    format!("[{:.4},{:.4},{:.4}]", t[0], t[1], t[2])
}

fn bench(text: &str, iterations: u32) -> Run<String> {
    let elements = scenario(text)?;
    let limits = Limits::default();
    let pin = pin()?;
    let build = time(iterations, || {
        ViewSnapshot::build(pin, elements.clone(), &limits).map_err(fail("build"))
    })?;
    let snapshot = ViewSnapshot::build(pin, elements, &limits).map_err(fail("build"))?;
    let to_bundle = time(iterations, || snapshot.to_bundle().map_err(fail("bundle")))?;
    let bundle = snapshot.to_bundle().map_err(fail("bundle"))?;
    let from_bundle = time(iterations, || {
        ViewSnapshot::from_bundle(&bundle, &limits).map_err(fail("from_bundle"))
    })?;
    // A ray straight down onto the middle of the grid, and a point and a snap at a plate corner.
    let bounds = snapshot.engineering_bounds().ok_or("empty snapshot")?;
    let centre = [
        (bounds.min[0] + bounds.max[0]) / 2.0 + 1.0,
        (bounds.min[1] + bounds.max[1]) / 2.0 + 1.0,
        bounds.max[2] + 100.0,
    ];
    let targets = vec![FeatureKind::Face, FeatureKind::Edge, FeatureKind::Vertex];
    let ray = PickQuery::Ray(RayPick::new(centre, [0.0, 0.0, -1.0], targets.clone()));
    let mut point = PointPick::new([0.0, 0.0, 0.0], targets);
    point.radius_mm = 5.0;
    let point = PickQuery::Point(point);
    let snap_query = SnapQuery {
        point_mm: [0.0, 0.0, 0.0],
        radius_mm: 5.0,
        kinds: vec![SnapKind::Vertex, SnapKind::AxisEnd, SnapKind::Axis],
        max_hits: 16,
    };
    let ray_ms = time(iterations, || pick(&snapshot, &ray).map_err(fail("ray")))?;
    let point_ms = time(iterations, || {
        pick(&snapshot, &point).map_err(fail("point"))
    })?;
    let snap_ms = time(iterations, || {
        snap(&snapshot, &snap_query).map_err(fail("snap"))
    })?;
    Ok(format!(
        concat!(
            "{{\"scenario\":\"{}\",\"elements\":{},\"meshes\":{},\"bundle_bytes\":{},",
            "\"iterations\":{},\"build_ms\":{},\"to_bundle_ms\":{},\"from_bundle_ms\":{},",
            "\"pick_ray_ms\":{},\"pick_point_ms\":{},\"snap_ms\":{}}}"
        ),
        text,
        snapshot.len(),
        snapshot.meshes().len(),
        bundle.len(),
        iterations,
        triple(build),
        triple(to_bundle),
        triple(from_bundle),
        triple(ray_ms),
        triple(point_ms),
        triple(snap_ms),
    ))
}

fn run(args: &[String]) -> Run<()> {
    match args {
        [mode, name, file] if mode == "write-bundle" => {
            let snapshot = ViewSnapshot::build(pin()?, scenario(name)?, &Limits::default())
                .map_err(fail("build"))?;
            let bundle = snapshot.to_bundle().map_err(fail("bundle"))?;
            std::fs::write(file, &bundle).map_err(fail("write"))?;
            println!(
                "{{\"written\":\"{name}\",\"elements\":{},\"meshes\":{},\"bytes\":{}}}",
                snapshot.len(),
                snapshot.meshes().len(),
                bundle.len()
            );
            Ok(())
        }
        [mode, file, rest @ ..] if mode == "from-bundle" => {
            let repeat: u32 = match rest {
                [] => 1,
                [n] => n.parse().map_err(fail("repeat"))?,
                _ => return Err("from-bundle FILE [REPEAT]".into()),
            };
            let bytes = std::fs::read(file).map_err(fail("read"))?;
            for _ in 0..repeat.max(1) {
                let snapshot = ViewSnapshot::from_bundle(&bytes, &Limits::default())
                    .map_err(fail("rebuild"))?;
                black_box(snapshot);
            }
            Ok(())
        }
        _ => {
            let mut iterations = 20;
            let mut scenarios = Vec::new();
            let mut rest = args.iter();
            while let Some(arg) = rest.next() {
                if arg == "--iterations" {
                    iterations = rest
                        .next()
                        .ok_or("--iterations K")?
                        .parse()
                        .map_err(fail("iterations"))?;
                } else {
                    scenarios.push(arg.clone());
                }
            }
            if scenarios.is_empty() {
                scenarios = [
                    "copies:10000",
                    "copies:65536",
                    "holed-copies:10000",
                    "synthetic:10000",
                    "synthetic-simplified:10000",
                ]
                .map(String::from)
                .to_vec();
            }
            for name in &scenarios {
                println!("{}", bench(name, iterations)?);
            }
            Ok(())
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).take(64).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("query_bench: {e}");
            ExitCode::from(2)
        }
    }
}
