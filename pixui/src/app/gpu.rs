//! GPU presenter: upload the canvas as a texture, let the GPU magnify it.
//!
//! The CPU backend upscales before handing pixels to the platform, so at a 6x
//! scale it moves 36x more data per frame than the canvas contains. This one
//! uploads the canvas *unscaled* — 384x240 is 360 KiB — and does the
//! nearest-neighbour magnification in the fragment shader, which is the one
//! thing a GPU is unambiguously better at than a CPU.
//!
//! Three details keep the result pixel-exact rather than merely fast:
//!
//! - **A non-sRGB surface format** is chosen when one is offered. The palette
//!   is authored in sRGB already; letting the hardware "helpfully" convert it
//!   would wash every colour out.
//! - **Nearest filtering plus an integer viewport.** The quad is rasterised
//!   into exactly `canvas * scale` pixels, so every texel maps to a whole
//!   number of fragments and no interpolation can occur.
//! - **BGRA byte order**, matching the canvas's `0x00RRGGBB` words on a
//!   little-endian machine, so the upload is a straight copy.
//!
//! [`PresentPipeline`] deliberately knows nothing about windows or surfaces, so
//! the same code that runs on screen can be pointed at an offscreen target and
//! checked pixel-for-pixel against the CPU blit. See `tests/gpu.rs`.

use std::error::Error;
use std::sync::Arc;

use winit::window::Window;

use super::Presenter;
use crate::canvas::Canvas;
use crate::color::Color;

/// The format the canvas is uploaded as: a byte-for-byte match for the
/// canvas's own `0x00RRGGBB` words on little-endian hardware.
pub const CANVAS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;

/// A fullscreen triangle, sampled with nearest filtering. That is the whole
/// pipeline — there is no geometry, no transform, and no vertex buffer.
const SHADER: &str = r#"
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VsOut {
    // One oversized triangle covers the viewport with no index buffer.
    let x = f32(i32(idx) / 2) * 4.0 - 1.0;
    let y = f32(i32(idx) & 1) * 4.0 - 1.0;
    var out: VsOut;
    out.pos = vec4<f32>(x, y, 0.0, 1.0);
    // Flip Y: texture rows run top-down, clip space runs bottom-up.
    out.uv = vec2<f32>((x + 1.0) * 0.5, 1.0 - (y + 1.0) * 0.5);
    return out;
}

@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(textureSample(tex, samp, in.uv).rgb, 1.0);
}
"#;

/// The window-independent half of the GPU presenter.
///
/// Everything here works against any render target, which is what makes the
/// backend testable without opening a window.
pub struct PresentPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl PresentPipeline {
    /// Build the pipeline for a target of the given format.
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pixui-present"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pixui-present"),
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

        // Nearest in every direction. This one line is what separates crisp
        // pixel art from a blurry mess.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("pixui-nearest"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pixui-present"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pixui-present"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(target_format.into())],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Self {
            pipeline,
            bind_layout,
            sampler,
        }
    }

    /// Allocate a texture to hold a canvas of this size.
    pub fn create_canvas_texture(
        &self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pixui-canvas"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: CANVAS_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }

    /// Bind a canvas texture for drawing.
    pub fn bind(&self, device: &wgpu::Device, texture: &wgpu::Texture) -> wgpu::BindGroup {
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pixui-canvas"),
            layout: &self.bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    /// Record the magnified draw into `pass`, confined to `viewport`
    /// (`x`, `y`, `width`, `height` in target pixels).
    pub fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        bind_group: &wgpu::BindGroup,
        viewport: (u32, u32, u32, u32),
    ) {
        let (x, y, w, h) = viewport;
        if w == 0 || h == 0 {
            return;
        }
        pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// Copy `canvas` into `staging` with rows padded to the copy alignment, then
/// upload it to `texture`.
pub fn upload_canvas(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    canvas: &Canvas,
    staging: &mut Vec<u8>,
) {
    let w = canvas.width() as usize;
    let h = canvas.height() as usize;
    let row_bytes = w * 4;
    let padded = row_bytes.div_ceil(256) * 256;
    staging.resize(padded * h, 0);

    let src = canvas.pixels();
    for y in 0..h {
        let line = &src[y * w..y * w + w];
        let dst = &mut staging[y * padded..y * padded + row_bytes];
        for (x, &p) in line.iter().enumerate() {
            let o = x * 4;
            dst[o] = p as u8;
            dst[o + 1] = (p >> 8) as u8;
            dst[o + 2] = (p >> 16) as u8;
            dst[o + 3] = 0xFF;
        }
    }

    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        staging,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(padded as u32),
            rows_per_image: Some(h as u32),
        },
        wgpu::Extent3d {
            width: w as u32,
            height: h as u32,
            depth_or_array_layers: 1,
        },
    );
}

/// Turn a pixui colour into a wgpu clear value. The target is a plain `Unorm`
/// format, so the components go across unchanged.
pub fn clear_color(color: Color) -> wgpu::Color {
    wgpu::Color {
        r: color.r() as f64 / 255.0,
        g: color.g() as f64 / 255.0,
        b: color.b() as f64 / 255.0,
        a: 1.0,
    }
}

pub struct GpuPresenter {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    present: PresentPipeline,
    texture: Option<wgpu::Texture>,
    bind_group: Option<wgpu::BindGroup>,
    tex_size: (u32, u32),
    /// Largest surface dimension this device will accept.
    max_dim: u32,
    /// Whether the swapchain is pacing us.
    vsync: bool,
    /// Row-padded copy of the canvas, reused every frame.
    staging: Vec<u8>,
}

impl GpuPresenter {
    /// (Re)create the canvas texture and its bind group when the size changes.
    fn ensure_texture(&mut self, width: u32, height: u32) {
        if self.texture.is_some() && self.tex_size == (width, height) {
            return;
        }
        let texture = self
            .present
            .create_canvas_texture(&self.device, width, height);
        self.bind_group = Some(self.present.bind(&self.device, &texture));
        self.texture = Some(texture);
        self.tex_size = (width, height);
    }
}

impl Presenter for GpuPresenter {
    const NAME: &'static str = "gpu";

    fn new(window: Arc<Window>, vsync: bool) -> Result<Self, Box<dyn Error>> {
        // The display handle is what lets the GL backend find a Wayland or X11
        // connection; the other backends ignore it.
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
            Box::new(window.clone()),
        ));
        let surface = instance.create_surface(window.clone())?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            // Nothing here is demanding; on a laptop the integrated GPU is the
            // right answer and keeps the fans off.
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))?;

        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("pixui"),
                required_features: wgpu::Features::empty(),
                // Downlevel defaults so this still runs on very old GL hardware —
                // but with the adapter's real texture-size limits folded back in.
                // The stock downlevel cap is 2048 and a Retina window is wider than
                // that, so asking for the defaults alone makes `configure` fail on
                // exactly the machines this backend is meant to help.
                required_limits:
                    wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
            }))?;

        let max_dim = device.limits().max_texture_dimension_2d;
        let size = window.inner_size();
        // Start from wgpu's own defaults so new fields keep sane values, then
        // override only the two things that actually matter here.
        let mut config = surface
            .get_default_config(
                &adapter,
                size.width.clamp(1, max_dim),
                size.height.clamp(1, max_dim),
            )
            .ok_or("the surface is not supported by this adapter")?;

        // Prefer a non-sRGB format: the palette is already authored in sRGB, and
        // an sRGB target would apply the transfer curve a second time, washing
        // every colour out.
        let caps = surface.get_capabilities(&adapter);
        if let Some(linear) = caps.formats.iter().copied().find(|f| !f.is_srgb()) {
            config.format = linear;
        }
        config.present_mode = if vsync {
            wgpu::PresentMode::AutoVsync
        } else {
            wgpu::PresentMode::AutoNoVsync
        };
        surface.configure(&device, &config);

        let present = PresentPipeline::new(&device, config.format);

        Ok(Self {
            surface,
            device,
            queue,
            config,
            present,
            texture: None,
            bind_group: None,
            tex_size: (0, 0),
            max_dim,
            vsync,
            staging: Vec::new(),
        })
    }

    fn paces_frames(&self) -> bool {
        self.vsync
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        // Configuring beyond the device limit is a hard validation error, so
        // clamp rather than crash on an implausibly large window.
        self.config.width = width.min(self.max_dim);
        self.config.height = height.min(self.max_dim);
        self.surface.configure(&self.device, &self.config);
    }

    fn present(&mut self, canvas: &Canvas, scale: i32, offset: (i32, i32), letterbox: Color) {
        self.ensure_texture(canvas.width() as u32, canvas.height() as u32);
        if let Some(texture) = self.texture.as_ref() {
            upload_canvas(&self.queue, texture, canvas, &mut self.staging);
        }

        use wgpu::CurrentSurfaceTexture as Acquired;
        let frame = match self.surface.get_current_texture() {
            Acquired::Success(f) => f,
            // Suboptimal still hands back a usable texture; reconfiguring on
            // the next resize is soon enough.
            Acquired::Suboptimal(f) => f,
            Acquired::Outdated | Acquired::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            // Nothing to draw into this frame; skipping is the correct response.
            Acquired::Timeout | Acquired::Occluded => return,
            Acquired::Validation => {
                eprintln!("pixui: the surface rejected this frame (validation error)");
                return;
            }
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pixui"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pixui-present"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        // The clear covers the whole attachment; the viewport
                        // then confines the quad, so this *is* the letterbox
                        // with no extra geometry.
                        load: wgpu::LoadOp::Clear(clear_color(letterbox)),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            // Clamp to the surface: an out-of-bounds viewport is a validation
            // error, and a window can be resized smaller than the canvas.
            let ox = offset.0.max(0) as u32;
            let oy = offset.1.max(0) as u32;
            let vw = (canvas.width() * scale).max(0) as u32;
            let vh = (canvas.height() * scale).max(0) as u32;
            let vw = vw.min(self.config.width.saturating_sub(ox));
            let vh = vh.min(self.config.height.saturating_sub(oy));

            if let Some(bind_group) = self.bind_group.as_ref() {
                self.present.draw(&mut pass, bind_group, (ox, oy, vw, vh));
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        self.queue.present(frame);
    }
}
