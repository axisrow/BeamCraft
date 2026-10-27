// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Baseline GPU metrics of one snapshot rendered offscreen: adapter identity, scene size,
//! upload cost and per-frame timings. CPU wall clock, not GPU timestamps; one run on one adapter
//! is a baseline, not a hardware survey.

use crate::camera::Camera;
use crate::render::{Gpu, Renderer, Uploaded, wait};
use crate::scene::Scene;
use crate::stats::Stats;
use crate::{Error, json_string};
use beamcraft_engine::RenderMode;
use std::time::Instant;

/// Most measured frames of one run.
pub const MAX_FRAMES: u32 = 10_000;

/// The measurements of one run.
#[derive(Debug, Clone, PartialEq)]
pub struct Baseline {
    /// Adapter name as the driver reports it.
    pub adapter: String,
    /// Adapter type (discrete, integrated, virtual, CPU, other).
    pub device_type: String,
    /// Graphics backend.
    pub backend: String,
    /// Target size in pixels.
    pub size: [u32; 2],
    /// MSAA samples used.
    pub samples: u32,
    /// Elements drawn.
    pub elements: usize,
    /// Distinct meshes uploaded.
    pub meshes: usize,
    /// Instances drawn.
    pub instances: u64,
    /// Triangles drawn per frame.
    pub triangles: u64,
    /// Vertex and instance bytes uploaded.
    pub upload_bytes: u64,
    /// Upload including device completion, milliseconds.
    pub upload_ms: f64,
    /// Measured frames (after warm-up frames).
    pub frames: u32,
    /// Encode and submit: median and p95, milliseconds.
    pub submit_ms: [f64; 2],
    /// Through device completion: median and p95, milliseconds.
    pub complete_ms: [f64; 2],
}

fn percentiles(mut samples: Vec<f64>) -> [f64; 2] {
    samples.sort_by(f64::total_cmp);
    let at = |q: f64| -> f64 {
        let n = samples.len();
        if n == 0 {
            return 0.0;
        }
        let rank = ((q * n as f64).ceil() as usize).clamp(1, n) - 1;
        samples.get(rank).copied().unwrap_or(0.0)
    };
    [at(0.5), at(0.95)]
}

/// Renders `scene` offscreen `frames` times after ten warm-up frames, orbiting slowly, with
/// `samples` (1 or 4; `None` is the adapter's default).
pub fn measure(
    scene: &Scene,
    frames: u32,
    size: [u32; 2],
    samples: Option<u32>,
    stats: &Stats,
) -> Result<Baseline, Error> {
    if frames == 0 || frames > MAX_FRAMES {
        return Err(Error::Input("frames 1..=10000"));
    }
    let gpu = pollster::block_on(Gpu::headless())?;
    let renderer =
        Renderer::with_samples(&gpu.adapter, &gpu.device, size[0], size[1], samples, stats)?;
    let uploaded = Uploaded::new(&gpu.device, &gpu.queue, scene, stats)?;
    let (target, extent) = scene.framing();
    let aspect = size[0] as f32 / size[1] as f32;
    let mut camera = Camera::new(target, extent, aspect)?;
    let mut submit = Vec::new();
    let mut complete = Vec::new();
    for frame in 0..frames.saturating_add(10) {
        camera.navigate([0.01, 0.0], [0.0; 2], 0.0)?;
        let view = camera.view(aspect)?;
        let start = Instant::now();
        let index = renderer.draw(
            &gpu.device,
            &gpu.queue,
            &uploaded,
            view,
            stats,
            RenderMode::Shaded,
        );
        let submitted = start.elapsed().as_secs_f64() * 1000.0;
        wait(&gpu.device, index)?;
        let completed = start.elapsed().as_secs_f64() * 1000.0;
        if frame >= 10 {
            submit.push(submitted);
            complete.push(completed);
        }
    }
    let info = gpu.adapter.get_info();
    let (meshes, instances) = scene.counts();
    Ok(Baseline {
        adapter: info.name,
        device_type: format!("{:?}", info.device_type),
        backend: format!("{:?}", info.backend),
        size,
        samples: renderer.samples,
        elements: scene.elements,
        meshes,
        instances,
        triangles: scene.triangles,
        upload_bytes: uploaded.bytes,
        upload_ms: uploaded.upload_ms,
        frames,
        submit_ms: percentiles(submit),
        complete_ms: percentiles(complete),
    })
}

impl Baseline {
    /// One JSON object.
    pub fn to_json(&self) -> String {
        format!(
            concat!(
                "{{\"outcome\":\"viewport_metrics\",\"adapter\":{},\"device_type\":{},",
                "\"backend\":{},\"width\":{},\"height\":{},\"samples\":{},\"elements\":{},",
                "\"meshes\":{},\"instances\":{},\"triangles\":{},\"upload_bytes\":{},",
                "\"upload_ms\":{:.3},\"frames\":{},\"submit_median_ms\":{:.4},",
                "\"submit_p95_ms\":{:.4},\"complete_median_ms\":{:.4},\"complete_p95_ms\":{:.4}}}"
            ),
            json_string(&self.adapter),
            json_string(&self.device_type),
            json_string(&self.backend),
            self.size[0],
            self.size[1],
            self.samples,
            self.elements,
            self.meshes,
            self.instances,
            self.triangles,
            self.upload_bytes,
            self.upload_ms,
            self.frames,
            self.submit_ms[0],
            self.submit_ms[1],
            self.complete_ms[0],
            self.complete_ms[1],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_use_the_nearest_rank() {
        assert_eq!(percentiles(vec![]), [0.0, 0.0]);
        assert_eq!(percentiles(vec![3.0, 1.0, 2.0]), [2.0, 3.0]);
        let many: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(percentiles(many), [50.0, 95.0]);
    }
}
