// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! Offscreen native wgpu renderer: one draw call per distinct mesh with one instance per element.
//! CPU serialization is explicit little endian, no unsafe casts. Derived from the V01 spike.
use crate::{
    Error,
    camera::View,
    scene::{DrawMesh, INSTANCE_BYTES, Scene, VERTEX_BYTES},
    stats::Stats,
};
use beamcraft_model::digest::ContentDigest;
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};
fn error_scopes(device: &wgpu::Device) -> [wgpu::ErrorScopeGuard; 3] {
    [
        device.push_error_scope(wgpu::ErrorFilter::OutOfMemory),
        device.push_error_scope(wgpu::ErrorFilter::Internal),
        device.push_error_scope(wgpu::ErrorFilter::Validation),
    ]
}
fn finish_scopes(scopes: [wgpu::ErrorScopeGuard; 3]) -> Result<(), Error> {
    let mut error = None;
    for scope in scopes.into_iter().rev() {
        if let Some(e) = pollster::block_on(scope.pop()) {
            error = Some(Error::Graphics(e.to_string()));
        }
    }
    match error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Graphics context and the actual adapter; selection prefers discrete, integrated, then CPU.
pub struct Gpu {
    /// Adapter used, never inferred from the operating system.
    pub adapter: wgpu::Adapter,
    /// Native graphics device.
    pub device: wgpu::Device,
    /// Its command queue.
    pub queue: wgpu::Queue,
}
impl Gpu {
    /// Enumerate native adapters; try device creation in ranked order, including software fallback.
    pub async fn headless() -> Result<Self, Error> {
        let backends = wgpu::Backends::DX12 | wgpu::Backends::VULKAN | wgpu::Backends::METAL;
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = backends;
        let instance = wgpu::Instance::new(desc);
        let mut adapters = instance.enumerate_adapters(backends).await;
        adapters.sort_by_key(|a| match a.get_info().device_type {
            wgpu::DeviceType::DiscreteGpu => 0,
            wgpu::DeviceType::IntegratedGpu => 1,
            wgpu::DeviceType::VirtualGpu => 2,
            wgpu::DeviceType::Other => 3,
            wgpu::DeviceType::Cpu => 4,
        });
        if adapters.is_empty() {
            return Err(Error::NoAdapter);
        }
        let mut errors = Vec::new();
        for adapter in adapters {
            match adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("BeamCraft viewport"),
                    ..Default::default()
                })
                .await
            {
                Ok((device, queue)) => {
                    return Ok(Self {
                        adapter,
                        device,
                        queue,
                    });
                }
                Err(e) => errors.push(e.to_string()),
            }
        }
        Err(Error::Graphics(errors.join("; ")))
    }
}
/// Wait for a specific submission, with a finite device timeout.
pub fn wait(device: &wgpu::Device, index: wgpu::SubmissionIndex) -> Result<(), Error> {
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(index),
            timeout: Some(Duration::from_secs(60)),
        })
        .map_err(|e| Error::Graphics(e.to_string()))?;
    Ok(())
}
/// One distinct mesh on the device.
struct GpuMesh {
    digest: ContentDigest,
    vertices: wgpu::Buffer,
    instances: wgpu::Buffer,
    vertex_count: u32,
    instance_count: u32,
}

/// The scene on the device: per distinct mesh one vertex buffer and one instance buffer.
pub struct Uploaded {
    meshes: Vec<GpuMesh>,
    /// Exact vertex and instance bytes this upload sent (excludes the fixed 80-byte camera
    /// uniform); vertex buffers reused from the previous scene are not sent again.
    pub bytes: u64,
    /// Buffer allocation and writes, and for [`Uploaded::new`] the flush to completion.
    pub upload_ms: f64,
}
fn buffer(device: &wgpu::Device, queue: &wgpu::Queue, bytes: &[u8]) -> wgpu::Buffer {
    let b = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("BeamCraft vertices/instances"),
        size: bytes.len() as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&b, 0, bytes);
    b
}
/// Checks one mesh's buffer sizes; `vertices` is false for a mesh whose vertex buffer is reused.
fn check(mesh: &DrawMesh, vertices: bool, limit: u64) -> Result<(), Error> {
    let instances_ok = mesh.instances.len() == mesh.instance_count as usize * INSTANCE_BYTES
        && !mesh.instances.is_empty();
    let vertices_ok = !vertices
        || (mesh.vertices.len() == mesh.vertex_count as usize * VERTEX_BYTES
            && !mesh.vertices.is_empty());
    if !instances_ok || !vertices_ok {
        return Err(Error::Input("scene buffer sizes"));
    }
    if [mesh.vertices.len(), mesh.instances.len()]
        .iter()
        .any(|s| *s as u64 > limit)
    {
        return Err(Error::Input("adapter buffer limit"));
    }
    Ok(())
}

impl Uploaded {
    /// Uploads every mesh of the scene and its instances, then waits for completion: the
    /// measured upload of `--metrics` and the render tests. The window uses [`Uploaded::upload`].
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &Scene,
        stats: &Stats,
    ) -> Result<Self, Error> {
        let start = Instant::now();
        let scopes = error_scopes(device);
        let uploaded = Self::write(device, queue, scene, None);
        stats.submits.add(1);
        let completion = wait(device, queue.submit([]));
        finish_scopes(scopes)?;
        completion?;
        let mut uploaded = uploaded?;
        uploaded.upload_ms = start.elapsed().as_secs_f64() * 1000.0;
        Ok(uploaded)
    }

    /// The window's upload: writes the scene's buffers without waiting for the device (the queue
    /// orders them before the next draw). A vertex buffer of `previous` with the same mesh digest
    /// is reused instead of being sent again (equal digests are equal blobs, so equal display
    /// vertices); instance buffers are always new, because they depend on the scene origin.
    /// Buffers of `previous` that the new scene does not use are released with it.
    pub fn upload(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &Scene,
        previous: Option<&Self>,
    ) -> Result<Self, Error> {
        let start = Instant::now();
        let scopes = error_scopes(device);
        let uploaded = Self::write(device, queue, scene, previous);
        finish_scopes(scopes)?;
        let mut uploaded = uploaded?;
        uploaded.upload_ms = start.elapsed().as_secs_f64() * 1000.0;
        Ok(uploaded)
    }

    fn write(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &Scene,
        previous: Option<&Self>,
    ) -> Result<Self, Error> {
        let limit = device.limits().max_buffer_size;
        let on_device: BTreeMap<ContentDigest, &GpuMesh> = previous
            .map(|p| p.meshes.iter().map(|g| (g.digest, g)).collect())
            .unwrap_or_default();
        let mut meshes = Vec::with_capacity(scene.meshes.len());
        let mut bytes = 0u64;
        for mesh in &scene.meshes {
            let reused = on_device
                .get(&mesh.digest)
                .filter(|g| g.vertex_count == mesh.vertex_count);
            if mesh.resident && reused.is_none() {
                return Err(Error::Input("a resident mesh is not on the device"));
            }
            check(mesh, reused.is_none(), limit)?;
            let vertices = match reused {
                Some(gpu) => gpu.vertices.clone(),
                None => {
                    bytes = bytes.saturating_add(mesh.vertices.len() as u64);
                    buffer(device, queue, &mesh.vertices)
                }
            };
            bytes = bytes.saturating_add(mesh.instances.len() as u64);
            meshes.push(GpuMesh {
                digest: mesh.digest,
                vertices,
                instances: buffer(device, queue, &mesh.instances),
                vertex_count: mesh.vertex_count,
                instance_count: mesh.instance_count,
            });
        }
        Ok(Self {
            meshes,
            bytes,
            upload_ms: 0.0,
        })
    }

    /// The digests of the meshes on the device, for [`Scene::from_snapshot_reusing`].
    pub fn resident(&self) -> BTreeSet<ContentDigest> {
        self.meshes.iter().map(|m| m.digest).collect()
    }

    /// Vertex buffers this scene shares with `other` (the same buffer, not a copy).
    pub fn shared_vertex_buffers(&self, other: &Self) -> usize {
        self.meshes
            .iter()
            .filter(|m| other.meshes.iter().any(|o| o.vertices == m.vertices))
            .count()
    }
}
/// The sample count a viewport uses by default: 4x MSAA on a hardware adapter that supports it,
/// 1x on a software (CPU) adapter, where every sample is rasterized by the processor: four times
/// the colour and depth work and a target of 36 instead of 8 bytes per pixel.
pub fn samples_for(device_type: wgpu::DeviceType, supports_4x: bool) -> u32 {
    if supports_4x && device_type != wgpu::DeviceType::Cpu {
        4
    } else {
        1
    }
}

/// The size-dependent half of the renderer: the resolved texture, the depth target and the
/// multisampled colour target.
struct Targets {
    view: wgpu::TextureView,
    texture: wgpu::Texture,
    depth: wgpu::TextureView,
    msaa: Option<wgpu::TextureView>,
}

fn targets(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    samples: u32,
    stats: &Stats,
) -> Result<Targets, Error> {
    if width == 0
        || height == 0
        || width > 4096
        || height > 4096
        || width > device.limits().max_texture_dimension_2d
        || height > device.limits().max_texture_dimension_2d
    {
        return Err(Error::Input("target size 1..=4096"));
    }
    let scopes = error_scopes(device);
    let texture_fn = |format, sample_count, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("BeamCraft target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    let texture = texture_fn(
        FORMAT,
        1,
        wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::TEXTURE_BINDING,
    );
    let view = texture.create_view(&Default::default());
    let depth = texture_fn(DEPTH, samples, wgpu::TextureUsages::RENDER_ATTACHMENT)
        .create_view(&Default::default());
    let msaa = (samples > 1).then(|| {
        texture_fn(FORMAT, samples, wgpu::TextureUsages::RENDER_ATTACHMENT)
            .create_view(&Default::default())
    });
    finish_scopes(scopes)?;
    stats.target_builds.add(1);
    Ok(Targets {
        view,
        texture,
        depth,
        msaa,
    })
}

/// The device-, format- and sample-dependent half: the shader, the pipeline and the camera
/// uniform with its bind group. A resize keeps it.
fn pipeline(
    device: &wgpu::Device,
    samples: u32,
    stats: &Stats,
) -> Result<
    (
        wgpu::RenderPipeline,
        wgpu::RenderPipeline,
        wgpu::Buffer,
        wgpu::BindGroup,
    ),
    Error,
> {
    let scopes = error_scopes(device);
    let camera = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("BeamCraft camera"),
        size: 80,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("BeamCraft camera layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("BeamCraft camera bind"),
        layout: &layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: camera.as_entire_binding(),
        }],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("BeamCraft pipeline layout"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("BeamCraft viewport shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("viewport.wgsl").into()),
    });
    let attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3];
    let instance_attributes = wgpu::vertex_attr_array![2=>Float32x3,3=>Float32x4];
    let vertex_layout = || {
        Some(wgpu::VertexBufferLayout {
            array_stride: 24,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attributes,
        })
    };
    let instance_layout = Some(wgpu::VertexBufferLayout {
        array_stride: INSTANCE_BYTES as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &instance_attributes,
    });
    let pipeline = |label, fragment, buffers: &[_]| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("instanced"),
                compilation_options: Default::default(),
                buffers,
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: samples,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(fragment),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    };
    let instanced = pipeline(
        "BeamCraft instanced",
        "shade",
        &[vertex_layout(), instance_layout.clone()],
    );
    let ghost = pipeline(
        "BeamCraft ghost",
        "shade_ghost",
        &[vertex_layout(), instance_layout],
    );
    finish_scopes(scopes)?;
    stats.pipeline_builds.add(2);
    Ok((instanced, ghost, camera, bind))
}

/// The ghost preview's device buffers: one mesh, one instance.
struct GhostDraw {
    vertices: wgpu::Buffer,
    instances: wgpu::Buffer,
    vertex_count: u32,
}

/// Offscreen target with depth and, by default, 4x MSAA on a hardware adapter that supports it
/// ([`samples_for`]).
pub struct Renderer {
    /// Resolved texture view, suitable for registering with egui.
    pub view: wgpu::TextureView,
    texture: wgpu::Texture,
    depth: wgpu::TextureView,
    msaa: Option<wgpu::TextureView>,
    instanced: wgpu::RenderPipeline,
    /// The ghost preview pipeline: the same vertices, a distinct fragment colour.
    ghost: wgpu::RenderPipeline,
    camera: wgpu::Buffer,
    bind: wgpu::BindGroup,
    /// The ghost preview's device buffers, when a placement is armed.
    ghost_buffers: Option<GhostDraw>,
    /// Actual sample count used.
    pub samples: u32,
    /// Offscreen width in pixels.
    pub width: u32,
    /// Offscreen height in pixels.
    pub height: u32,
    /// The colour behind the model; the window sets it from the theme.
    pub background: wgpu::Color,
}
impl Renderer {
    /// Create bounded targets and pipelines with the adapter's default sample count
    /// ([`samples_for`]). Setup is excluded from upload and frame measurements.
    pub fn new(
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        stats: &Stats,
    ) -> Result<Self, Error> {
        Self::with_samples(adapter, device, width, height, None, stats)
    }

    /// [`Renderer::new`] with an explicit sample count, 1 or 4; `None` is the adapter's default.
    /// 4 on an adapter whose formats do not support it is refused.
    pub fn with_samples(
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        samples: Option<u32>,
        stats: &Stats,
    ) -> Result<Self, Error> {
        let supports_4x = [FORMAT, DEPTH].into_iter().all(|f| {
            adapter
                .get_texture_format_features(f)
                .flags
                .sample_count_supported(4)
        });
        let samples = match samples {
            None => samples_for(adapter.get_info().device_type, supports_4x),
            Some(1) => 1,
            Some(4) if supports_4x => 4,
            Some(4) => return Err(Error::Input("this adapter does not support 4x MSAA")),
            Some(_) => return Err(Error::Input("samples are 1 or 4")),
        };
        let Targets {
            view,
            texture,
            depth,
            msaa,
        } = targets(device, width, height, samples, stats)?;
        let (instanced, ghost, camera, bind) = pipeline(device, samples, stats)?;
        Ok(Self {
            view,
            texture,
            depth,
            msaa,
            instanced,
            ghost,
            camera,
            bind,
            ghost_buffers: None,
            samples,
            width,
            height,
            background: wgpu::Color {
                r: 0.02,
                g: 0.03,
                b: 0.05,
                a: 1.0,
            },
        })
    }

    /// Gives the targets a new size, keeping the shader, the pipeline and the camera uniform
    /// (they depend on the device, the formats and the sample count only). Returns whether
    /// anything changed; on an error the renderer keeps its previous targets.
    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        stats: &Stats,
    ) -> Result<bool, Error> {
        if width == self.width && height == self.height {
            return Ok(false);
        }
        let Targets {
            view,
            texture,
            depth,
            msaa,
        } = targets(device, width, height, self.samples, stats)?;
        self.view = view;
        self.texture = texture;
        self.depth = depth;
        self.msaa = msaa;
        self.width = width;
        self.height = height;
        Ok(true)
    }

    /// Replaces the ghost preview's buffers; `None` drops them.
    pub fn set_ghost(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        ghost: Option<&crate::scene::GhostMesh>,
    ) {
        self.ghost_buffers = ghost.map(|g| GhostDraw {
            vertices: buffer(device, queue, &g.vertices),
            instances: buffer(device, queue, &g.instance),
            vertex_count: g.vertex_count,
        });
    }

    /// Encodes one lit, depth-tested frame and submits it; completion is a separate measurement.
    pub fn draw(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        uploaded: &Uploaded,
        camera: View,
        stats: &Stats,
    ) -> wgpu::SubmissionIndex {
        stats.viewport_draws.add(1);
        stats.submits.add(1);
        queue.write_buffer(&self.camera, 0, &camera.bytes());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("BeamCraft frame"),
        });
        {
            let color = wgpu::RenderPassColorAttachment {
                view: self.msaa.as_ref().unwrap_or(&self.view),
                depth_slice: None,
                resolve_target: self.msaa.as_ref().map(|_| &self.view),
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(self.background),
                    // A multisampled target is resolved into `view` at the end of the pass;
                    // its own samples are never read again.
                    store: if self.msaa.is_some() {
                        wgpu::StoreOp::Discard
                    } else {
                        wgpu::StoreOp::Store
                    },
                },
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("BeamCraft model"),
                color_attachments: &[Some(color)],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&self.instanced);
            pass.set_bind_group(0, &self.bind, &[]);
            for mesh in &uploaded.meshes {
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                pass.set_vertex_buffer(1, mesh.instances.slice(..));
                pass.draw(0..mesh.vertex_count, 0..mesh.instance_count);
            }
            if let Some(g) = &self.ghost_buffers {
                pass.set_pipeline(&self.ghost);
                pass.set_bind_group(0, &self.bind, &[]);
                pass.set_vertex_buffer(0, g.vertices.slice(..));
                pass.set_vertex_buffer(1, g.instances.slice(..));
                pass.draw(0..g.vertex_count, 0..1);
            }
        }
        queue.submit([encoder.finish()])
    }
    /// Reads RGBA pixels (row padding retained) for headless validation; never used per frame.
    pub fn readback(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<(Vec<u8>, u32), Error> {
        let row = (self.width * 4).div_ceil(256) * 256;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("BeamCraft readback"),
            size: u64::from(row) * u64::from(self.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        let index = queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        buffer.map_async(wgpu::MapMode::Read, .., move |r| {
            let _ = tx.send(r);
        });
        wait(device, index)?;
        rx.recv_timeout(Duration::from_secs(10))
            .map_err(|e| Error::Graphics(e.to_string()))?
            .map_err(|e| Error::Graphics(e.to_string()))?;
        let mapped = buffer
            .get_mapped_range(..)
            .map_err(|e| Error::Graphics(e.to_string()))?;
        let data = mapped.to_vec();
        drop(mapped);
        buffer.unmap();
        Ok((data, row))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardware_adapters_default_to_4x_and_software_ones_to_1x() {
        use wgpu::DeviceType::{Cpu, DiscreteGpu, IntegratedGpu, Other, VirtualGpu};
        for gpu in [DiscreteGpu, IntegratedGpu, VirtualGpu, Other] {
            assert_eq!(samples_for(gpu, true), 4);
            assert_eq!(samples_for(gpu, false), 1);
        }
        assert_eq!(samples_for(Cpu, true), 1);
        assert_eq!(samples_for(Cpu, false), 1);
    }

    #[test]
    fn a_resize_rebuilds_the_targets_and_keeps_the_pipeline() {
        let gpu = match pollster::block_on(Gpu::headless()) {
            Ok(gpu) => gpu,
            Err(Error::NoAdapter) => {
                eprintln!("SKIP resize: no native wgpu adapter");
                return;
            }
            Err(e) => panic!("adapter or device failed: {e}"),
        };
        let supports_4x = [FORMAT, DEPTH].into_iter().all(|f| {
            gpu.adapter
                .get_texture_format_features(f)
                .flags
                .sample_count_supported(4)
        });
        let samples = if supports_4x { 4 } else { 1 };
        let stats = Stats::default();
        let mut renderer =
            Renderer::with_samples(&gpu.adapter, &gpu.device, 64, 48, Some(samples), &stats)
                .unwrap();
        assert_eq!(renderer.samples, samples);
        assert_eq!(renderer.msaa.is_some(), samples == 4);
        let pipeline = renderer.instanced.clone();
        let bind = renderer.bind.clone();
        let texture = renderer.texture.clone();
        assert!(
            !renderer.resize(&gpu.device, 64, 48, &stats).unwrap(),
            "same size"
        );
        assert!(renderer.texture == texture);
        assert!(renderer.resize(&gpu.device, 96, 80, &stats).unwrap());
        assert_eq!((renderer.width, renderer.height), (96, 80));
        assert!(renderer.texture != texture, "new targets");
        assert!(
            renderer.instanced == pipeline && renderer.bind == bind,
            "same pipeline"
        );
        // The resized target draws and reads back at its new size.
        let pin = beamcraft_engine::ViewPin::new(
            beamcraft_model::digest::Sha256::digest(b"revision").unwrap(),
            1,
        );
        let snapshot = beamcraft_engine::ViewSnapshot::build(
            pin,
            beamcraft_engine::synthetic::plate_scene(1, false).unwrap(),
            &beamcraft_engine::Limits::default(),
        )
        .unwrap();
        let scene =
            Scene::from_snapshot(&snapshot, beamcraft_engine::Representation::Full).unwrap();
        let uploaded = Uploaded::new(&gpu.device, &gpu.queue, &scene, &stats).unwrap();
        let (target, extent) = scene.framing();
        let view = crate::camera::Camera::new(target, extent, 1.2)
            .unwrap()
            .view(1.2)
            .unwrap();
        let index = renderer.draw(&gpu.device, &gpu.queue, &uploaded, view, &stats);
        wait(&gpu.device, index).unwrap();
        let (pixels, row) = renderer.readback(&gpu.device, &gpu.queue).unwrap();
        assert_eq!(pixels.len(), row as usize * 80);
        // A bad size keeps the previous targets; a bad sample count is refused.
        assert!(renderer.resize(&gpu.device, 0, 80, &stats).is_err());
        assert_eq!((renderer.width, renderer.height), (96, 80));
        assert!(Renderer::with_samples(&gpu.adapter, &gpu.device, 8, 8, Some(2), &stats).is_err());
    }
}
