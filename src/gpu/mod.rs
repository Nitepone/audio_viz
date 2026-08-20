/// gpu/ — wgpu rendering engine.
///
/// Owns the surface, device and the two visualizer render paths:
///
///   Software — the visualizer's CPU `Framebuffer` is uploaded to an
///     sRGB texture and blitted to the swapchain.
///
///   Shader — the visualizer's WGSL fragment source is compiled once
///     (with prelude.wgsl prepended) into a pipeline that renders a
///     fullscreen triangle into a ping-pong pair of offscreen textures.
///     The previous frame is bound as an input, enabling phosphor
///     persistence / feedback effects.  The result is blitted (and, when
///     the internal resolution is capped, upscaled) to the swapchain.
///
/// The visualizer does not necessarily own the whole window: the UI layer
/// (src/ui.rs) reserves side panels, and the remaining central area is set
/// each frame via `set_viz_rect()`.  Both render paths blit into that
/// viewport only; the egui pass then draws the panels on top within the
/// same frame (see `FrameCtx` / `begin_frame` / `end_frame`).
///
/// wgpu selects the native backend per platform: Metal on macOS,
/// Vulkan/DX12 on Windows, Vulkan/GL on Linux — all backends stay enabled.

use std::sync::Arc;

use anyhow::Context;
use winit::window::Window;

use crate::visualizer::{Framebuffer, AUDIO_TEX_WIDTH};

const PRELUDE_WGSL: &str = include_str!("prelude.wgsl");
const BLIT_WGSL: &str = include_str!("blit.wgsl");
const FX_PRELUDE_WGSL: &str = include_str!("fx_prelude.wgsl");

/// Offscreen (feedback) texture format. Float precision keeps slow phosphor
/// decay smooth where 8-bit would posterise.
const FEEDBACK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Cap the internal render resolution of shader visualizers (in pixels).
/// Fragment-shader visualizers iterate over hundreds of audio samples per
/// pixel; capping keeps fullscreen retina windows fluid.  The blit pass
/// upscales with linear filtering.
const MAX_INTERNAL_PIXELS: u32 = 1_600_000;

// ── Per-frame uniforms (must match prelude.wgsl) ─────────────────────────────

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Uniforms {
    pub resolution: [f32; 2],
    pub time: f32,
    pub dt: f32,
    pub rms: [f32; 2],
    pub beat: f32,
    pub sample_rate: f32,
    pub params: [[f32; 4]; 4],
}

/// Audio data uploaded to the GPU each frame: four rows of AUDIO_TEX_WIDTH
/// floats (left, right, mono, fft).
pub struct AudioTexData {
    pub texels: Vec<f32>, // 4 * AUDIO_TEX_WIDTH
}

impl AudioTexData {
    pub fn pack(left: &[f32], right: &[f32], mono: &[f32], fft: &[f32]) -> Self {
        let w = AUDIO_TEX_WIDTH;
        let mut texels = vec![0.0f32; w * 4];
        for (row, src) in [left, right, mono, fft].iter().enumerate() {
            let n = src.len().min(w);
            texels[row * w..row * w + n].copy_from_slice(&src[..n]);
        }
        Self { texels }
    }
}

// ── Engine state ─────────────────────────────────────────────────────────────

struct FeedbackTarget {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

/// Ping-pong offscreen targets for the active shader visualizer.
/// Rebuilt lazily whenever the internal render size changes.
struct FeedbackState {
    targets: [FeedbackTarget; 2],
    /// bind_groups[i] renders INTO targets[i] with targets[1-i] as prev_frame.
    bind_groups: [wgpu::BindGroup; 2],
    idx: usize,
    size: (u32, u32),
}

struct SoftwareState {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: (u32, u32),
}

// ── Post-effect chain (see src/fx.rs and fx_prelude.wgsl) ────────────────────

/// One requested post-effect pass: WGSL fragment source (compiled against
/// fx_prelude.wgsl) plus its per-frame parameters.
pub struct FxInstance {
    pub name: &'static str,
    pub wgsl: &'static str,
    pub params: [f32; 8],
}

/// Per-pass uniforms (must match fx_prelude.wgsl).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FxUniforms {
    resolution: [f32; 2],
    time: f32,
    _pad: f32,
    params: [[f32; 4]; 2],
}

struct FxPass {
    name: &'static str,
    /// Renders into an intermediate FEEDBACK_FORMAT target (non-final passes).
    pipeline_inter: wgpu::RenderPipeline,
    /// Renders into the swapchain viz rect (final pass of the chain).
    pipeline_final: wgpu::RenderPipeline,
    ubuf: wgpu::Buffer,
    params: [f32; 8],
}

/// Ping-pong targets between chained passes; only allocated for chains of
/// two or more effects.
struct FxIntermediates {
    views: [wgpu::TextureView; 2],
    _textures: [wgpu::Texture; 2],
    size: (u32, u32),
}

/// One in-flight frame: acquired swapchain texture plus the command encoder
/// shared by the visualizer passes and the UI overlay pass.
pub struct FrameCtx {
    frame: wgpu::SurfaceTexture,
    pub view: wgpu::TextureView,
    pub encoder: wgpu::CommandEncoder,
}

pub struct GpuContext {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,

    sampler: wgpu::Sampler,
    uniform_buf: wgpu::Buffer,
    audio_tex: wgpu::Texture,
    audio_view: wgpu::TextureView,

    shader_bgl: wgpu::BindGroupLayout,
    blit_bgl: wgpu::BindGroupLayout,
    blit_pipeline: wgpu::RenderPipeline,

    shader_pipeline: Option<wgpu::RenderPipeline>,
    feedback: Option<FeedbackState>,
    software: Option<SoftwareState>,

    fx_bgl: wgpu::BindGroupLayout,
    fx_passes: Vec<FxPass>,
    fx_inter: Option<FxIntermediates>,
    fx_time: f32,

    /// Area of the surface the visualizer renders into (x, y, w, h in
    /// physical pixels).  The UI layer updates this every frame.
    viz_rect: (u32, u32, u32, u32),
}

impl GpuContext {
    pub fn new(window: Arc<Window>) -> anyhow::Result<Self> {
        pollster::block_on(Self::new_async(window))
    }

    async fn new_async(window: Arc<Window>) -> anyhow::Result<Self> {
        let size = window.inner_size();

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let surface = instance.create_surface(window)?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .context("no compatible GPU adapter found")?;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("audio-viz device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::default(),
                },
                None,
            )
            .await?;

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let audio_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("audio data"),
            size: wgpu::Extent3d {
                width: AUDIO_TEX_WIDTH as u32,
                height: 4,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let audio_view = audio_tex.create_view(&wgpu::TextureViewDescriptor::default());

        // Bind group layout for shader visualizers (see prelude.wgsl).
        let shader_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shader viz bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        // Blit pipeline: offscreen / software texture → swapchain.
        let blit_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blit bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let blit_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blit"),
            source: wgpu::ShaderSource::Wgsl(BLIT_WGSL.into()),
        });
        let blit_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("blit layout"),
            bind_group_layouts: &[&blit_bgl],
            push_constant_ranges: &[],
        });
        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blit pipeline"),
            layout: Some(&blit_layout),
            vertex: wgpu::VertexState {
                module: &blit_module,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &blit_module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        // Post-effect bind group layout: previous stage + sampler + uniforms.
        let fx_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fx bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let viz_rect = (0, 0, config.width, config.height);

        Ok(Self {
            surface,
            device,
            queue,
            config,
            sampler,
            uniform_buf,
            audio_tex,
            audio_view,
            shader_bgl,
            blit_bgl,
            blit_pipeline,
            shader_pipeline: None,
            feedback: None,
            software: None,
            fx_bgl,
            fx_passes: Vec::new(),
            fx_inter: None,
            fx_time: 0.0,
            viz_rect,
        })
    }

    // ── Accessors used by the UI layer ───────────────────────────────────────

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }
    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.config.format
    }
    pub fn surface_size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Set the surface area the visualizer renders into (physical pixels).
    /// Called every frame by the app after the UI has claimed its panels.
    pub fn set_viz_rect(&mut self, x: u32, y: u32, w: u32, h: u32) {
        let x = x.min(self.config.width);
        let y = y.min(self.config.height);
        let w = w.min(self.config.width - x).max(1);
        let h = h.min(self.config.height - y).max(1);
        self.viz_rect = (x, y, w, h);
    }

    /// Internal render resolution for visualizers: the viz rect, area-capped
    /// preserving aspect.  Applies to both the shader path (feedback targets)
    /// and the software path (CPU framebuffer) — the present blit upscales the
    /// capped source to fill the viz rect, so full-retina panes stay fluid.
    pub fn render_resolution(&self) -> (u32, u32) {
        let (_, _, w, h) = self.viz_rect;
        let area = w as u64 * h as u64;
        if area <= MAX_INTERNAL_PIXELS as u64 {
            return (w, h);
        }
        let scale = (MAX_INTERNAL_PIXELS as f64 / area as f64).sqrt();
        (((w as f64 * scale) as u32).max(1), ((h as f64 * scale) as u32).max(1))
    }

    // ── Shader path setup ────────────────────────────────────────────────────

    /// Compile a shader visualizer's fragment source.  Feedback targets are
    /// created lazily at render time (they track the viz rect size).
    pub fn set_shader(&mut self, fragment_wgsl: &str) {
        let full_src = format!("{PRELUDE_WGSL}\n{fragment_wgsl}");
        let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shader viz"),
            source: wgpu::ShaderSource::Wgsl(full_src.into()),
        });

        let layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shader viz layout"),
            bind_group_layouts: &[&self.shader_bgl],
            push_constant_ranges: &[],
        });

        let pipeline = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shader viz pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: FEEDBACK_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        self.shader_pipeline = Some(pipeline);
        self.feedback = None;
        self.software = None;
    }

    /// (Re)build feedback targets if their size no longer matches.
    fn ensure_feedback(&mut self) {
        let size = self.render_resolution();
        if self.feedback.as_ref().map(|f| f.size) == Some(size) {
            return;
        }

        let make_target = |label: &str| {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FEEDBACK_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            FeedbackTarget { _texture: texture, view }
        };
        let targets = [make_target("feedback 0"), make_target("feedback 1")];

        let make_bind_group = |prev: &FeedbackTarget| {
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("shader viz bind group"),
                layout: &self.shader_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.uniform_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&self.audio_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&prev.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            })
        };
        // bind_groups[i] renders into targets[i], reading targets[1-i].
        let bind_groups = [make_bind_group(&targets[1]), make_bind_group(&targets[0])];

        self.feedback = Some(FeedbackState { targets, bind_groups, idx: 0, size });
    }

    // ── Frame lifecycle ──────────────────────────────────────────────────────

    pub fn begin_frame(&mut self) -> anyhow::Result<FrameCtx> {
        let frame = self.acquire_frame()?;
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        Ok(FrameCtx { frame, view, encoder })
    }

    /// Submit the frame.  `pre_cmds` (e.g. egui buffer uploads) are submitted
    /// before the frame's own encoder.
    pub fn end_frame(&mut self, fctx: FrameCtx, pre_cmds: Vec<wgpu::CommandBuffer>) {
        let FrameCtx { frame, view: _view, encoder } = fctx;
        self.queue.submit(pre_cmds.into_iter().chain(std::iter::once(encoder.finish())));
        frame.present();
    }

    /// Render one frame of the active shader visualizer into the viz rect.
    pub fn render_shader(&mut self, fctx: &mut FrameCtx, uniforms: &Uniforms, audio: &AudioTexData) {
        if self.shader_pipeline.is_none() {
            return;
        }
        self.ensure_feedback();

        self.queue.write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(uniforms));
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.audio_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&audio.texels),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some((AUDIO_TEX_WIDTH * 4) as u32),
                rows_per_image: Some(4),
            },
            wgpu::Extent3d { width: AUDIO_TEX_WIDTH as u32, height: 4, depth_or_array_layers: 1 },
        );

        let pipeline = self.shader_pipeline.as_ref().unwrap();
        let feedback = self.feedback.as_ref().unwrap();
        let idx = feedback.idx;

        // Pass 1: visualizer → feedback target
        {
            let mut pass = fctx.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viz pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &feedback.targets[idx].view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &feedback.bind_groups[idx], &[]);
            pass.draw(0..3, 0..1);
        }

        // Pass 2: feedback target → post-effect chain (or plain blit) →
        // swapchain viz rect (upscales if capped)
        let src_view = feedback.targets[idx].view.clone();
        let src_size = feedback.size;
        self.present(&mut fctx.encoder, &src_view, src_size, &fctx.view);

        if let Some(f) = self.feedback.as_mut() {
            f.idx = 1 - f.idx;
        }
    }

    /// Render one frame of a software visualizer from its CPU framebuffer.
    pub fn render_software(&mut self, fctx: &mut FrameCtx, fb: &Framebuffer) {
        if fb.width == 0 || fb.height == 0 {
            return;
        }

        // (Re)create the upload texture when the framebuffer size changes.
        let needs_new = self
            .software
            .as_ref()
            .map(|s| s.size != (fb.width, fb.height))
            .unwrap_or(true);
        if needs_new {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("software framebuffer"),
                size: wgpu::Extent3d {
                    width: fb.width,
                    height: fb.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.software = Some(SoftwareState { texture, view, size: (fb.width, fb.height) });
            self.shader_pipeline = None;
            self.feedback = None;
        }

        let sw = self.software.as_ref().unwrap();
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &sw.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &fb.data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(fb.width * 4),
                rows_per_image: Some(fb.height),
            },
            wgpu::Extent3d { width: fb.width, height: fb.height, depth_or_array_layers: 1 },
        );

        let sw = self.software.as_ref().unwrap();
        let src_view = sw.view.clone();
        let src_size = sw.size;
        self.present(&mut fctx.encoder, &src_view, src_size, &fctx.view);
    }

    // ── Post-effect chain ────────────────────────────────────────────────────

    /// Set the post-effect chain applied at present time (src/fx.rs builds
    /// it from the active visualizer's settings each frame).  Pipelines are
    /// rebuilt only when the chain composition changes; parameters are
    /// refreshed on every call.
    pub fn set_post_chain(&mut self, chain: &[FxInstance], time: f32) {
        self.fx_time = time;
        let same = self.fx_passes.len() == chain.len()
            && self.fx_passes.iter().zip(chain).all(|(p, c)| p.name == c.name);
        if !same {
            let passes: Vec<FxPass> = chain.iter().map(|c| self.build_fx_pass(c)).collect();
            self.fx_passes = passes;
        }
        for (pass, inst) in self.fx_passes.iter_mut().zip(chain) {
            pass.params = inst.params;
        }
    }

    fn build_fx_pass(&self, inst: &FxInstance) -> FxPass {
        let full_src = format!("{FX_PRELUDE_WGSL}\n{}", inst.wgsl);
        let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(inst.name),
            source: wgpu::ShaderSource::Wgsl(full_src.into()),
        });
        let layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fx layout"),
            bind_group_layouts: &[&self.fx_bgl],
            push_constant_ranges: &[],
        });
        let make_pipeline = |format: wgpu::TextureFormat| {
            self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(inst.name),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            })
        };
        FxPass {
            name: inst.name,
            pipeline_inter: make_pipeline(FEEDBACK_FORMAT),
            pipeline_final: make_pipeline(self.config.format),
            ubuf: self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("fx uniforms"),
                size: std::mem::size_of::<FxUniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            params: inst.params,
        }
    }

    /// (Re)build the between-pass ping-pong targets; only chains of two or
    /// more effects need them.
    fn ensure_fx_intermediates(&mut self, size: (u32, u32)) {
        if self.fx_passes.len() < 2 {
            return;
        }
        if self.fx_inter.as_ref().map(|i| i.size) == Some(size) {
            return;
        }
        let make = |label: &str| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FEEDBACK_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let textures = [make("fx inter 0"), make("fx inter 1")];
        let views = [
            textures[0].create_view(&wgpu::TextureViewDescriptor::default()),
            textures[1].create_view(&wgpu::TextureViewDescriptor::default()),
        ];
        self.fx_inter = Some(FxIntermediates { views, _textures: textures, size });
    }

    fn fx_bind_group(&self, view: &wgpu::TextureView, ubuf: &wgpu::Buffer) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx bind group"),
            layout: &self.fx_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry { binding: 2, resource: ubuf.as_entire_binding() },
            ],
        })
    }

    /// Final stage shared by both render paths: run the post-effect chain
    /// (or a plain blit when it is empty) ending in the swapchain viz rect.
    fn present(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        src_view: &wgpu::TextureView,
        src_size: (u32, u32),
        target: &wgpu::TextureView,
    ) {
        if self.fx_passes.is_empty() {
            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("blit bind group"),
                layout: &self.blit_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(src_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            self.run_blit(encoder, &bind_group, target);
            return;
        }

        self.ensure_fx_intermediates(src_size);
        let n = self.fx_passes.len();
        let mut cur = src_view.clone();

        for (i, pass) in self.fx_passes.iter().enumerate() {
            let last = i == n - 1;
            let out_size = if last { (self.viz_rect.2, self.viz_rect.3) } else { src_size };
            let uniforms = FxUniforms {
                resolution: [out_size.0 as f32, out_size.1 as f32],
                time: self.fx_time,
                _pad: 0.0,
                params: [
                    [pass.params[0], pass.params[1], pass.params[2], pass.params[3]],
                    [pass.params[4], pass.params[5], pass.params[6], pass.params[7]],
                ],
            };
            self.queue.write_buffer(&pass.ubuf, 0, bytemuck::bytes_of(&uniforms));
            let bind_group = self.fx_bind_group(&cur, &pass.ubuf);

            let dst =
                if last { target.clone() } else { self.fx_inter.as_ref().unwrap().views[i % 2].clone() };
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("fx pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &dst,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            if last {
                let (x, y, w, h) = self.viz_rect;
                rpass.set_pipeline(&pass.pipeline_final);
                rpass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
            } else {
                rpass.set_pipeline(&pass.pipeline_inter);
            }
            rpass.set_bind_group(0, &bind_group, &[]);
            rpass.draw(0..3, 0..1);
            drop(rpass);

            cur = dst;
        }
    }

    /// Blit `bind_group`'s texture into the viz rect of `target`, clearing
    /// the rest of the target to black.
    fn run_blit(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        bind_group: &wgpu::BindGroup,
        target: &wgpu::TextureView,
    ) {
        let (x, y, w, h) = self.viz_rect;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("blit pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.blit_pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
        pass.draw(0..3, 0..1);
    }

    fn acquire_frame(&mut self) -> anyhow::Result<wgpu::SurfaceTexture> {
        match self.surface.get_current_texture() {
            Ok(f) => Ok(f),
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.surface.configure(&self.device, &self.config);
                Ok(self.surface.get_current_texture()?)
            }
            Err(e) => Err(e.into()),
        }
    }
}
