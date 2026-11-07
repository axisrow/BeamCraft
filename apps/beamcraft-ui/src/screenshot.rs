// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! One offscreen viewport screenshot of a scene as a PNG file: the same renderer the window
//! draws with, headless, with the camera fitted over the whole scene like `--metrics` and
//! like the `Fit model` action. The pixels come back through the renderer's own readback
//! and are encoded by the `image` crate; the file is created new, never overwritten.

use crate::Error;
use crate::camera::Camera;
use crate::cut_plane::ClipGpu;
use crate::render::{Gpu, Renderer, Uploaded, wait};
use crate::scene::Scene;
use crate::stats::Stats;
use beamcraft_engine::RenderMode;
use std::io::BufWriter;
use std::io::Write;
use std::path::Path;

/// The result of one capture.
#[derive(Debug, Clone, PartialEq)]
pub struct Screenshot {
    /// PNG bytes written.
    pub bytes: u64,
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// MSAA samples the renderer used.
    pub samples: u32,
    /// Elements drawn.
    pub elements: usize,
    /// Pixels that differ from the background colour.
    pub lit_pixels: u64,
}

/// Renders `scene` once at `size`, fits the camera over the whole scene, and writes the PNG
/// to `out`, which must not exist. `samples` is 1 or 4; `None` is the adapter's default.
pub fn capture(
    scene: &Scene,
    out: &Path,
    size: [u32; 2],
    samples: Option<u32>,
    stats: &Stats,
) -> Result<Screenshot, Error> {
    let gpu = pollster::block_on(Gpu::headless())?;
    let renderer =
        Renderer::with_samples(&gpu.adapter, &gpu.device, size[0], size[1], samples, stats)?;
    let uploaded = Uploaded::new(&gpu.device, &gpu.queue, scene, stats)?;
    let (target, extent) = scene.framing();
    let aspect = size[0] as f32 / size[1] as f32;
    let camera = Camera::new(target, extent, aspect)?;
    let view = camera.view(aspect)?;
    let index = renderer.draw(
        &gpu.device,
        &gpu.queue,
        &uploaded,
        view,
        stats,
        RenderMode::Shaded,
        &ClipGpu::none(),
    );
    wait(&gpu.device, index)?;
    let (data, row) = renderer.readback(&gpu.device, &gpu.queue)?;
    let height = usize::try_from(size[1]).map_err(|_| Error::Allocation)?;
    let wide = usize::try_from(size[0])
        .map_err(|_| Error::Allocation)?
        .checked_mul(4)
        .ok_or(Error::Allocation)?;
    let mut tight = Vec::with_capacity(wide.checked_mul(height).ok_or(Error::Allocation)?);
    for y in 0..height {
        let start = y * row as usize;
        let end = start + wide;
        let row_bytes = data.get(start..end).ok_or(Error::Allocation)?;
        tight.extend_from_slice(row_bytes);
    }
    // The clear colour reaches the file quantized; a pixel within one step of it on every
    // channel is background (the pixel tests use the same bands).
    let background = renderer.background;
    let near = |channel: f64, pixel: u8| {
        let expected = (channel * 255.0).round() as i16;
        (i16::from(pixel) - expected).abs() <= 1
    };
    let is_background = |p: &[u8; 4]| {
        near(background.r, p[0])
            && near(background.g, p[1])
            && near(background.b, p[2])
            && p[3] == 255
    };
    let lit_pixels = tight
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| !is_background(p))
        .count() as u64;
    let image = image::RgbaImage::from_raw(size[0], size[1], tight).ok_or(Error::Allocation)?;
    let file = std::fs::File::create_new(out)
        .map_err(|_| Error::Input("the output file exists or cannot be created"))?;
    let mut writer = BufWriter::new(file);
    image
        .write_to(&mut writer, image::ImageFormat::Png)
        .map_err(|e| Error::Graphics(format!("the PNG could not be encoded: {e}")))?;
    writer.flush().map_err(Error::Io)?;
    let file = writer.into_inner().map_err(|e| Error::Io(e.into_error()))?;
    file.sync_all().map_err(Error::Io)?;
    let bytes = file.metadata().map_err(Error::Io)?.len();
    Ok(Screenshot {
        bytes,
        width: size[0],
        height: size[1],
        samples: renderer.samples,
        elements: scene.elements,
        lit_pixels,
    })
}

impl Screenshot {
    /// One JSON object.
    pub fn to_json(&self) -> String {
        format!(
            concat!(
                "{{\"outcome\":\"screenshot_saved\",\"bytes\":{},\"width\":{},\"height\":{},",
                "\"samples\":{},\"elements\":{},\"lit_pixels\":{}}}"
            ),
            self.bytes, self.width, self.height, self.samples, self.elements, self.lit_pixels,
        )
    }
}
