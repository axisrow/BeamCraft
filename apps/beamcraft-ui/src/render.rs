// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! Offscreen native wgpu renderer: one draw call per distinct mesh with one instance per element.
//! CPU serialization is explicit little endian, no unsafe casts. Derived from the V01 spike.
use crate::{
    Error,
    camera::View,
    scene::{INSTANCE_BYTES, Scene, VERTEX_BYTES},
    stats::STATS,
};
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
/// The scene on the device: per distinct mesh one vertex buffer and one instance buffer.
pub struct Uploaded {
    meshes: Vec<(wgpu::Buffer, wgpu::Buffer, u32, u32)>,
    /// Exact vertex and instance bytes sent (excludes the fixed 80-byte camera uniform).
    pub bytes: u64,
    /// Buffer allocation, writes and flush-to-completion time.
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
impl Uploaded {
    /// Uploads every mesh of the scene and its instances, then waits for completion.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, scene: &Scene) -> Result<Self, Error> {
        let start = Instant::now();
        let limit = device.limits().max_buffer_size;
        let scopes = error_scopes(device);
        let mut meshes = Vec::with_capacity(scene.meshes.len());
        let mut bytes = 0u64;
        for mesh in &scene.meshes {
            if mesh.vertices.len() != mesh.vertex_count as usize * VERTEX_BYTES
                || mesh.instances.len() != mesh.instance_count as usize * INSTANCE_BYTES
                || mesh.vertices.is_empty()
                || mesh.instances.is_empty()
            {
                return Err(Error::Input("scene buffer sizes"));
            }
            let sizes = [mesh.vertices.len() as u64, mesh.instances.len() as u64];
            if sizes.iter().any(|s| *s > limit) {
                return Err(Error::Input("adapter buffer limit"));
            }
            bytes = bytes.saturating_add(sizes[0]).saturating_add(sizes[1]);
            meshes.push((
                buffer(device, queue, &mesh.vertices),
                buffer(device, queue, &mesh.instances),
                mesh.vertex_count,
                mesh.instance_count,
            ));
        }
        STATS.submits.add(1);
        let completion = wait(device, queue.submit([]));
        finish_scopes(scopes)?;
        completion?;
        Ok(Self {
            meshes,
            bytes,
            upload_ms: start.elapsed().as_secs_f64() * 1000.0,
        })
    }
}
/// Offscreen target with depth and 4x MSAA when both formats support it.
pub struct Renderer {
    /// Resolved texture view, suitable for registering with egui.
    pub view: wgpu::TextureView,
    texture: wgpu::Texture,
    depth: wgpu::TextureView,
    msaa: Option<wgpu::TextureView>,
    instanced: wgpu::RenderPipeline,
    camera: wgpu::Buffer,
    bind: wgpu::BindGroup,
    /// Actual sample count used.
    pub samples: u32,
    /// Offscreen width in pixels.
    pub width: u32,
    /// Offscreen height in pixels.
    pub height: u32,
}
impl Renderer {
    /// Create bounded targets and pipelines. Setup is excluded from upload and frame measurements.
    pub fn new(
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> Result<Self, Error> {
        if width == 0
            || height == 0
            || width > 4096
            || height > 4096
            || width > device.limits().max_texture_dimension_2d
            || height > device.limits().max_texture_dimension_2d
        {
            return Err(Error::Input("target size 1..=4096"));
        }
        let samples = if [FORMAT, DEPTH].into_iter().all(|f| {
            adapter
                .get_texture_format_features(f)
                .flags
                .sample_count_supported(4)
        }) {
            4
        } else {
            1
        };
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
        let msaa = if samples == 4 {
            Some(
                texture_fn(FORMAT, 4, wgpu::TextureUsages::RENDER_ATTACHMENT)
                    .create_view(&Default::default()),
            )
        } else {
            None
        };
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
        let instance_attributes = wgpu::vertex_attr_array![2=>Float32x3];
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
        let pipeline = |entry, buffers: &[_]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(entry),
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
                    entry_point: Some("shade"),
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
        let instanced = pipeline("instanced", &[vertex_layout(), instance_layout]);
        finish_scopes(scopes)?;
        STATS.pipeline_builds.add(1);
        STATS.target_builds.add(1);
        Ok(Self {
            view,
            texture,
            depth,
            msaa,
            instanced,
            camera,
            bind,
            samples,
            width,
            height,
        })
    }
    /// Encodes one lit, depth-tested frame and submits it; completion is a separate measurement.
    pub fn draw(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        uploaded: &Uploaded,
        camera: View,
    ) -> wgpu::SubmissionIndex {
        STATS.viewport_draws.add(1);
        STATS.submits.add(1);
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
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.02,
                        g: 0.03,
                        b: 0.05,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
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
            for (vertices, instances, vertex_count, instance_count) in &uploaded.meshes {
                pass.set_vertex_buffer(0, vertices.slice(..));
                pass.set_vertex_buffer(1, instances.slice(..));
                pass.draw(0..*vertex_count, 0..*instance_count);
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
