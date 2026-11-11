// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! One offscreen viewport screenshot of a scene as a PNG file: the same renderer the window
//! draws with, headless, with the camera fitted over the whole scene and its project grids
//! like the `Fit model` action. The grid lines and labels are painted over the render by the
//! window's own grid painter through a headless egui pass (issue 543). The pixels come back
//! through the renderer's own readback and are encoded by the `image` crate; the file is
//! created new, never overwritten.

use crate::Error;
use crate::camera::Camera;
use crate::camera::View;
use crate::cut_plane::ClipGpu;
use crate::grid::GridScene;
use crate::render::{Gpu, LIGHT_BACKGROUND, Renderer, Uploaded, wait};
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
    /// Project grid lines painted over the render.
    pub grid_lines: usize,
    /// Pixels that differ from the background colour.
    pub lit_pixels: u64,
}

/// The camera pose over the fitted framing.
pub struct CameraPose {
    /// Orbit direction in radians (`yaw`, `pitch` above the horizon); `None` keeps the
    /// fitted default direction exactly as the `Fit model` action shows it.
    pub angles: Option<(f32, f32)>,
    /// The projection: `Some(true)` orthographic, `Some(false)` perspective; `None` keeps
    /// the fitted default, which is the orthographic start of `Camera::new`.
    pub ortho: Option<bool>,
    /// Logarithmic zoom; positive moves the camera closer, the orthographic window
    /// follows the distance.
    pub zoom: f32,
    /// Screen-plane pan of the fitted target in millimetres.
    pub pan: [f32; 2],
}

/// The clear colour behind the render (issue 537): the renderer's own dark theme, or a
/// light paper tone so a frame matches a drawing-style reference picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Background {
    /// The renderer's dark clear colour, today's default.
    #[default]
    Dark,
    /// A light clear colour near the drawing paper value `rgb(245, 246, 248)`.
    Light,
}

/// Renders `scene` once at `size`, fits the camera over the scene and the `grids`, paints the
/// grid lines and labels over it, paints the resolved D06 `annotations` the way the window's
/// viewport does, and writes the PNG to `out`, which must not exist. `samples` is 1 or 4;
/// `None` is the adapter's default. `background` picks the clear colour the renderer starts
/// its frame from and the theme the overlays are painted in.
#[allow(clippy::too_many_arguments)]
pub fn capture(
    scene: &Scene,
    grids: &GridScene,
    out: &Path,
    size: [u32; 2],
    samples: Option<u32>,
    pose: &CameraPose,
    background: Background,
    annotations: &[crate::app::annotation_panel::Row],
    stats: &Stats,
) -> Result<Screenshot, Error> {
    let gpu = pollster::block_on(Gpu::headless())?;
    let mut renderer =
        Renderer::with_samples(&gpu.adapter, &gpu.device, size[0], size[1], samples, stats)?;
    if background == Background::Light {
        renderer.background = LIGHT_BACKGROUND;
    }
    let uploaded = Uploaded::new(&gpu.device, &gpu.queue, scene, stats)?;
    let (target, extent) = grids.framing(&scene.summary());
    let aspect = size[0] as f32 / size[1] as f32;
    let mut camera = Camera::new(target, extent, aspect)?;
    // Camera::new starts at yaw -0.8, pitch 0.85; navigate takes deltas and clamps the
    // pitch to its own bounds. Orbit first, then zoom, then pan, exactly like the window's
    // orbit navigation, and the projection switch last, so the orthographic window follows
    // the final distance.
    let (yaw, pitch) = pose.angles.unwrap_or((-0.8, 0.85));
    camera.navigate([yaw - -0.8, pitch - 0.85], pose.pan, pose.zoom)?;
    // The switch is applied even for the perspective request: the camera starts orthographic,
    // so only an explicit switch ever reaches it (issue 562 item 13).
    if let Some(ortho) = pose.ortho {
        camera.set_projection(ortho);
    }
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
    if !grids.lines.is_empty() || !annotations.is_empty() {
        paint_overlays(
            &gpu,
            &renderer,
            grids,
            scene,
            &view,
            size,
            background,
            annotations,
        )?;
    }
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
        grid_lines: grids.lines.len(),
        lit_pixels,
    })
}

/// Paints the overlays over the resolved render with the window's own painters: the grid
/// guides through [`GridScene::paint`] and the resolved D06 annotations through the
/// annotation panel's painter — one headless egui frame at one pixel per point, tessellated
/// and drawn by `egui_wgpu` into the renderer's own target with the render kept
/// (`LoadOp::Load`).
#[allow(clippy::too_many_arguments)]
fn paint_overlays(
    gpu: &Gpu,
    renderer: &Renderer,
    grids: &GridScene,
    scene: &Scene,
    view: &View,
    size: [u32; 2],
    background: Background,
    annotations: &[crate::app::annotation_panel::Row],
) -> Result<(), Error> {
    use eframe::egui_wgpu;
    let ctx = egui::Context::default();
    // Inter is the window's face; without it egui's built-in default face still draws the
    // labels, so a missing asset degrades the font instead of failing the screenshot.
    let _ = crate::app::inter(&ctx);
    crate::theme::install(&ctx);
    ctx.set_theme(match background {
        Background::Dark => egui::Theme::Dark,
        Background::Light => egui::Theme::Light,
    });
    let rect =
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size[0] as f32, size[1] as f32));
    let input = egui::RawInput {
        screen_rect: Some(rect),
        ..Default::default()
    };
    // The first frame lays the fonts out; the second paints with them. Texture deltas of both
    // frames are applied in order, since the font atlas is created by the first.
    let mut deltas = Vec::new();
    let mut output = None;
    for _ in 0..2 {
        let mut frame = ctx.run_ui(input.clone(), |ui| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ui, |ui| {
                    // No saved view here: the CLI screenshot poses the camera itself, so no
                    // depth slab filters the guides.
                    grids.paint(ui, rect, view, scene.origin_mm, None, None);
                    if !annotations.is_empty() {
                        crate::app::annotation_panel::paint(
                            ui,
                            rect,
                            view,
                            scene.origin_mm,
                            annotations,
                            &|_| true,
                        );
                    }
                });
        });
        deltas.extend(
            frame
                .textures_delta
                .set
                .iter()
                .map(|(id, steps)| (*id, steps.clone())),
        );
        // The deltas are applied below; the frame's own record is cleared as egui requires.
        frame.textures_delta.clear();
        output = Some(frame);
    }
    let output = output.ok_or(Error::Allocation)?;
    let primitives = ctx.tessellate(output.shapes, 1.0);
    let descriptor = egui_wgpu::ScreenDescriptor {
        size_in_pixels: size,
        pixels_per_point: 1.0,
    };
    let mut painter = egui_wgpu::Renderer::new(
        &gpu.device,
        wgpu::TextureFormat::Rgba8Unorm,
        egui_wgpu::RendererOptions::default(),
    );
    for (id, steps) in &deltas {
        for delta in steps {
            painter.update_texture(&gpu.device, &gpu.queue, *id, delta);
        }
    }
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("screenshot grids"),
        });
    let mut buffers = painter.update_buffers(
        &gpu.device,
        &gpu.queue,
        &mut encoder,
        &primitives,
        &descriptor,
    );
    {
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("screenshot grids"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &renderer.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            })
            .forget_lifetime();
        painter.render(&mut pass, &primitives, &descriptor);
    }
    buffers.push(encoder.finish());
    let index = gpu.queue.submit(buffers);
    wait(&gpu.device, index)
}

impl Screenshot {
    /// One JSON object.
    pub fn to_json(&self) -> String {
        format!(
            concat!(
                "{{\"outcome\":\"screenshot_saved\",\"bytes\":{},\"width\":{},\"height\":{},",
                "\"samples\":{},\"elements\":{},\"grid_lines\":{},\"lit_pixels\":{}}}"
            ),
            self.bytes,
            self.width,
            self.height,
            self.samples,
            self.elements,
            self.grid_lines,
            self.lit_pixels,
        )
    }
}
