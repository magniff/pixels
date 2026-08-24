//! Proves the GPU presenter is pixel-identical to the CPU one.
//!
//! Without this, "the shader magnifies the canvas correctly" is an assertion
//! nobody has checked. The three things most likely to be silently wrong are
//! exactly the three that a screenshot would catch and a compiler would not:
//!
//! - the **Y flip** between texture rows and clip space,
//! - the **BGRA byte order** of the upload,
//! - an **sRGB transfer curve** applied where it should not be.
//!
//! So this renders through the real [`PresentPipeline`] into an offscreen
//! target, reads it back, and compares every pixel against the CPU [`blit`].
//!
//! Skips itself with a printed note when no GPU adapter is available, so it
//! stays honest in a headless CI container instead of failing spuriously.

#![cfg(feature = "gpu")]

use pixui::app::gpu::{clear_color, upload_canvas, PresentPipeline, CANVAS_FORMAT};
use pixui::{palette, Canvas, Color, Rect};

const SCALE: u32 = 3;
const OFFSET: (u32, u32) = (2, 2);

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

/// `None` if this machine has no usable adapter.
fn gpu() -> Option<Gpu> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .ok()?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("pixui-test"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
    }))
    .ok()?;
    Some(Gpu { device, queue })
}

/// A canvas whose colours would visibly break under any of the failure modes
/// above: mid grey shifts hard under an sRGB curve, and the pure primaries in
/// distinct corners catch a channel swap or a flip.
fn test_canvas() -> Canvas {
    let mut c = Canvas::new(5, 4);
    c.clear(Color::hex(0x808080));
    c.set_px(0, 0, Color::hex(0xFF0000)); // top-left red
    c.set_px(4, 0, Color::hex(0x00FF00)); // top-right green
    c.set_px(0, 3, Color::hex(0x0000FF)); // bottom-left blue
    c.set_px(4, 3, Color::hex(0xFFFFFF)); // bottom-right white
    c.fill_rect(Rect::new(1, 1, 2, 2), palette::ACCENT);
    c
}

/// Render `canvas` through the GPU pipeline and read the target back as
/// `0x00RRGGBB` words, matching the canvas's own layout.
fn render_on_gpu(
    gpu: &Gpu,
    canvas: &Canvas,
    target_w: u32,
    target_h: u32,
    letterbox: Color,
) -> Vec<u32> {
    let Gpu { device, queue } = gpu;
    let present = PresentPipeline::new(device, CANVAS_FORMAT);

    let canvas_tex =
        present.create_canvas_texture(device, canvas.width() as u32, canvas.height() as u32);
    let mut staging = Vec::new();
    upload_canvas(queue, &canvas_tex, canvas, &mut staging);
    let bind_group = present.bind(device, &canvas_tex);

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("target"),
        size: wgpu::Extent3d {
            width: target_w,
            height: target_h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: CANVAS_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());

    // Readback rows must be 256-byte aligned.
    let row_bytes = (target_w * 4) as usize;
    let padded = row_bytes.div_ceil(256) * 256;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (padded * target_h as usize) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("present"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear_color(letterbox)),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        present.draw(
            &mut pass,
            &bind_group,
            (
                OFFSET.0,
                OFFSET.1,
                canvas.width() as u32 * SCALE,
                canvas.height() as u32 * SCALE,
            ),
        );
    }
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded as u32),
                rows_per_image: Some(target_h),
            },
        },
        wgpu::Extent3d {
            width: target_w,
            height: target_h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(std::iter::once(encoder.finish()));

    readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("device poll failed");

    let mapped = readback
        .slice(..)
        .get_mapped_range()
        .expect("buffer did not map");
    let mut out = vec![0u32; (target_w * target_h) as usize];
    for y in 0..target_h as usize {
        for x in 0..target_w as usize {
            let o = y * padded + x * 4;
            // BGRA bytes back into a 0x00RRGGBB word.
            out[y * target_w as usize + x] =
                (mapped[o + 2] as u32) << 16 | (mapped[o + 1] as u32) << 8 | mapped[o] as u32;
        }
    }
    drop(mapped);
    readback.unmap();
    out
}

#[test]
fn gpu_present_matches_the_cpu_blit_exactly() {
    let Some(gpu) = gpu() else {
        eprintln!("skipping: no GPU adapter available on this machine");
        return;
    };

    let canvas = test_canvas();
    let letterbox = palette::VOID;
    let target_w = canvas.width() as u32 * SCALE + OFFSET.0 * 2;
    let target_h = canvas.height() as u32 * SCALE + OFFSET.1 * 2;

    let gpu_pixels = render_on_gpu(&gpu, &canvas, target_w, target_h, letterbox);

    let mut cpu_pixels = vec![0u32; (target_w * target_h) as usize];
    let mut scratch = Vec::new();
    pixui::app::blit(
        &canvas,
        &mut scratch,
        &mut cpu_pixels,
        target_w as usize,
        target_h as usize,
        SCALE as usize,
        (OFFSET.0 as i32, OFFSET.1 as i32),
        letterbox,
    );

    let mut mismatches = Vec::new();
    for y in 0..target_h as usize {
        for x in 0..target_w as usize {
            let i = y * target_w as usize + x;
            if gpu_pixels[i] != cpu_pixels[i] {
                mismatches.push((x, y, cpu_pixels[i], gpu_pixels[i]));
            }
        }
    }

    if !mismatches.is_empty() {
        let shown: Vec<String> = mismatches
            .iter()
            .take(8)
            .map(|(x, y, cpu, gpu)| format!("({x},{y}) cpu #{cpu:06X} != gpu #{gpu:06X}"))
            .collect();
        panic!(
            "{} of {} pixels differ between the CPU and GPU present paths:\n  {}",
            mismatches.len(),
            target_w * target_h,
            shown.join("\n  ")
        );
    }
}

#[test]
fn gpu_present_does_not_apply_an_srgb_curve() {
    let Some(gpu) = gpu() else {
        eprintln!("skipping: no GPU adapter available on this machine");
        return;
    };

    // Mid grey is the sharpest probe there is: an unwanted sRGB decode drags
    // 0x80 down to roughly 0x37, and an unwanted encode pushes it to ~0xBC.
    let mut canvas = Canvas::new(2, 2);
    canvas.clear(Color::hex(0x808080));

    let (w, h) = (2 * SCALE + OFFSET.0 * 2, 2 * SCALE + OFFSET.1 * 2);
    let pixels = render_on_gpu(&gpu, &canvas, w, h, palette::VOID);

    let centre = pixels[((OFFSET.1 + 1) * w + OFFSET.0 + 1) as usize];
    assert_eq!(
        centre, 0x808080,
        "mid grey came back as #{centre:06X}; a colour-space conversion is being applied somewhere"
    );
}

#[test]
fn gpu_letterbox_uses_the_theme_colour() {
    let Some(gpu) = gpu() else {
        eprintln!("skipping: no GPU adapter available on this machine");
        return;
    };

    let canvas = test_canvas();
    let letterbox = Color::hex(0x123456);
    let w = canvas.width() as u32 * SCALE + OFFSET.0 * 2;
    let h = canvas.height() as u32 * SCALE + OFFSET.1 * 2;
    let pixels = render_on_gpu(&gpu, &canvas, w, h, letterbox);

    assert_eq!(pixels[0], 0x123456, "top-left corner should be letterbox");
    assert_eq!(
        pixels[(w * h - 1) as usize],
        0x123456,
        "bottom-right corner should be letterbox"
    );
    assert_ne!(
        pixels[((OFFSET.1 + 1) * w + OFFSET.0 + 1) as usize],
        0x123456,
        "the canvas area should not be letterbox"
    );
}
