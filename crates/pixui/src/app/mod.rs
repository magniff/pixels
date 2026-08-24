//! The windowing backend: winit for events, a swappable [`Presenter`] for pixels.
//!
//! This is the *only* part of the workspace that names a platform crate.
//! Everything above it — widgets, layout, theming, the application itself —
//! deals in [`crate::input::Input`] and [`crate::canvas::Canvas`].
//!
//! Two things happen here that matter to how the result looks:
//!
//! 1. **Integer scaling.** The virtual canvas is blown up by a whole-number
//!    factor. A fractional scale is what makes pixel art look like a bad JPEG:
//!    some source pixels get two output pixels and their neighbours get three.
//!    Refusing to do that is not a limitation, it is the entire point.
//!    [`Scaling`] decides what a resize therefore *means* — magnify the canvas
//!    ([`Scaling::Fixed`]) or grow it ([`Scaling::Adaptive`]).
//! 2. **Physical pixels throughout.** Hit testing and scaling work in physical
//!    pixels, so a HiDPI display simply yields a larger integer scale. There is
//!    no DPI factor anywhere in the widget code.
//!
//! ## Backends
//!
//! Getting a finished canvas onto the screen is the one job that genuinely
//! differs between platforms and eras, so it sits behind [`Presenter`]:
//!
//! - [`soft::SoftPresenter`] (feature `soft`, default) upscales on the CPU and
//!   hands the whole window-sized buffer to softbuffer.
//! - [`gpu::GpuPresenter`] (feature `gpu`) uploads the *unscaled* canvas as a
//!   texture and lets the GPU do the nearest-neighbour magnification.
//!
//! The second moves dramatically less data — at a 6x scale the CPU path pushes
//! 36x more bytes per frame — which turns out to be where essentially all of
//! the frame time was going. See the crate README for numbers.

#[cfg(feature = "gpu")]
pub mod gpu;
#[cfg(feature = "soft")]
pub mod soft;

use std::error::Error;
use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key as WKey, NamedKey};
use winit::window::{CursorIcon, Window, WindowId};

use crate::canvas::Canvas;
use crate::color::Color;
use crate::geom::Point;
use crate::input::{Cursor, Input, Key, Mods};
use crate::theme::Theme;
use crate::ui::{Ui, UiState};

/// Puts a finished [`Canvas`] on the screen.
///
/// Implementing this is all it takes to port pixui somewhere new — a different
/// graphics API, a framebuffer, a test harness that writes PNGs. Nothing above
/// this trait knows how pixels reach a display.
pub trait Presenter: Sized {
    /// Short name, shown in the `PIXUI_PROFILE` output.
    const NAME: &'static str;

    fn new(window: Arc<Window>, vsync: bool) -> Result<Self, Box<dyn Error>>;

    /// The window changed size, in physical pixels.
    fn resize(&mut self, width: u32, height: u32);

    /// Draw `canvas` magnified by the whole number `scale`, its top-left at
    /// `offset` physical pixels, with `letterbox` filling everything around it.
    fn present(&mut self, canvas: &Canvas, scale: i32, offset: (i32, i32), letterbox: Color);

    /// Whether [`Presenter::present`] blocks until the display is ready for a
    /// new frame.
    ///
    /// When it does, the swapchain is the authority on pacing and the frame
    /// timer only needs to be a backstop — running the timer at exactly the
    /// refresh rate would let jitter make us miss a vblank and judder.
    fn paces_frames(&self) -> bool {
        false
    }
}

/// How the virtual canvas relates to the window.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Scaling {
    /// The canvas is a fixed size, magnified by the largest whole number that
    /// fits, and the remainder is letterboxed.
    ///
    /// Right for anything with a composed, fixed layout — a game, a title
    /// screen, a instrument panel — where resizing should make the pixels
    /// bigger, not the screen roomier.
    #[default]
    Fixed,
    /// The magnification is fixed and the canvas grows with the window.
    ///
    /// Right for anything with content in it — an editor, a browser, a list —
    /// where a bigger window should mean *more room* at the same pixel size.
    /// [`Config::width`] and [`Config::height`] become the minimum canvas, and
    /// the window is given a matching minimum size so layout never has less
    /// space than it was designed for.
    Adaptive,
}

/// What a window size implies: how much to magnify, how big the canvas is, and
/// where it sits inside the window.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Geometry {
    pub scale: i32,
    pub canvas: (i32, i32),
    pub offset: (i32, i32),
}

/// Resolve the geometry for a window, as a pure function.
///
/// Split out from the event loop so the resize behaviour can be tested without
/// opening a window — this is the arithmetic that decides whether dragging an
/// edge magnifies the UI or gives it more room.
pub fn resolve_geometry(
    scaling: Scaling,
    canvas: (i32, i32),
    logical_scale: i32,
    dpr: f64,
    window: (i32, i32),
) -> Geometry {
    let (win_w, win_h) = (window.0.max(1), window.1.max(1));

    let (scale, canvas) = match scaling {
        Scaling::Fixed => {
            // The largest whole number that fits; the remainder is letterbox.
            let sx = win_w / canvas.0.max(1);
            let sy = win_h / canvas.1.max(1);
            (sx.min(sy).max(1), canvas)
        }
        Scaling::Adaptive => {
            // The scale is pinned to logical points, so one virtual pixel stays
            // the same physical size whatever the display density, and a bigger
            // window buys canvas rather than chunkier pixels.
            let scale = ((logical_scale as f64 * dpr).round() as i32).max(1);
            // Floor, never round up: the canvas must never be wider than the
            // window, or the presenter would have to squash it to fit.
            (scale, ((win_w / scale).max(16), (win_h / scale).max(16)))
        }
    };

    let vw = canvas.0 * scale;
    let vh = canvas.1 * scale;
    let offset = match scaling {
        // A fixed canvas floats in the middle of its letterbox.
        Scaling::Fixed => (((win_w - vw) / 2).max(0), ((win_h - vh) / 2).max(0)),
        // An adaptive canvas is pinned to the top-left instead. Centring would
        // split the sub-scale remainder between both edges, so every few pixels
        // of a drag would shunt the whole UI sideways by one — far more
        // noticeable than the remainder itself.
        Scaling::Adaptive => (0, 0),
    };

    Geometry {
        scale,
        canvas,
        offset,
    }
}

/// How to open the window and what the virtual screen looks like.
pub struct Config {
    pub title: String,
    /// Virtual canvas width, in the chunky pixels the user sees. Under
    /// [`Scaling::Adaptive`] this is the *minimum* width.
    pub width: i32,
    /// Virtual canvas height, or its starting height under [`Scaling::Adaptive`].
    pub height: i32,
    /// Smallest canvas the layout is expected to cope with.
    ///
    /// Under [`Scaling::Adaptive`] this sets the window's minimum size, and so
    /// decides how much room there is to resize *within*. Defaulting it to the
    /// starting size means the window opens already at its minimum and can only
    /// grow — usually not what you want.
    pub min_width: i32,
    pub min_height: i32,
    /// How many logical points one virtual pixel occupies.
    ///
    /// Under [`Scaling::Fixed`] this only picks the initial window size; the
    /// real magnification is then whatever whole number fits. Under
    /// [`Scaling::Adaptive`] it is fixed for the life of the window.
    pub initial_scale: i32,
    /// Whether the canvas grows with the window.
    pub scaling: Scaling,
    pub resizable: bool,
    pub theme: Theme,
    /// Frames per second to aim for, or `None` to follow the display.
    ///
    /// Following the display is almost always what you want: it is 120 on a
    /// ProMotion panel and 60 on a normal one, and hardcoding either is wrong
    /// somewhere. Set it explicitly only to deliberately cap the frame rate.
    pub target_fps: Option<u32>,
    /// Sync presentation to the display refresh. `PIXUI_VSYNC=0` overrides this
    /// to off, which is how you get a meaningful number out of the profiler —
    /// with vsync on, "present" is mostly the wait for the next vblank.
    pub vsync: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            title: "pixui".to_string(),
            width: 384,
            height: 240,
            min_width: 384,
            min_height: 240,
            initial_scale: 3,
            scaling: Scaling::Fixed,
            resizable: true,
            theme: Theme::warm(),
            target_fps: None,
            vsync: true,
        }
    }
}

impl Config {
    pub fn new(title: impl Into<String>, width: i32, height: i32) -> Self {
        Self {
            title: title.into(),
            width,
            height,
            min_width: width,
            min_height: height,
            ..Default::default()
        }
    }

    /// Set the smallest canvas the layout can cope with, which under
    /// [`Scaling::Adaptive`] is what the window may be shrunk to.
    ///
    /// Leaving it at the starting size means the window opens already at its
    /// own minimum and can only ever grow, which is rarely what you want.
    pub fn with_min_canvas(mut self, width: i32, height: i32) -> Self {
        self.min_width = width.max(16);
        self.min_height = height.max(16);
        self
    }

    pub fn with_scale(mut self, scale: i32) -> Self {
        self.initial_scale = scale.max(1);
        self
    }

    /// Grow the canvas with the window instead of letterboxing it.
    pub fn adaptive(mut self) -> Self {
        self.scaling = Scaling::Adaptive;
        self
    }

    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    /// Cap the frame rate instead of following the display.
    pub fn with_fps(mut self, fps: u32) -> Self {
        self.target_fps = Some(fps.max(1));
        self
    }

    pub fn with_vsync(mut self, vsync: bool) -> Self {
        self.vsync = vsync;
        self
    }

    fn resolved_vsync(&self) -> bool {
        match std::env::var("PIXUI_VSYNC").as_deref() {
            Ok("0") | Ok("off") | Ok("false") => false,
            Ok("1") | Ok("on") | Ok("true") => true,
            _ => self.vsync,
        }
    }
}

/// Run the application on a specific backend.
///
/// `ui_fn` is called once per frame with the frame context and your state; this
/// does not return until the window closes.
pub fn run_with<P, S, F>(config: Config, state: S, ui_fn: F) -> Result<(), Box<dyn Error>>
where
    P: Presenter + 'static,
    S: 'static,
    F: FnMut(&mut Ui, &mut S) + 'static,
{
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App::<S, F, P>::new(config, state, ui_fn);
    event_loop.run_app(&mut app)?;
    Ok(())
}

/// Run the application on the default backend.
///
/// With both backends compiled in, `PIXUI_BACKEND=soft` selects the CPU one;
/// otherwise the GPU presenter is used, since it is cheaper by a wide margin.
pub fn run<S, F>(config: Config, state: S, ui_fn: F) -> Result<(), Box<dyn Error>>
where
    S: 'static,
    F: FnMut(&mut Ui, &mut S) + 'static,
{
    #[cfg(all(feature = "gpu", feature = "soft"))]
    {
        if std::env::var("PIXUI_BACKEND").as_deref() == Ok("soft") {
            return run_with::<soft::SoftPresenter, S, F>(config, state, ui_fn);
        }
        run_with::<gpu::GpuPresenter, S, F>(config, state, ui_fn)
    }
    #[cfg(all(feature = "gpu", not(feature = "soft")))]
    {
        run_with::<gpu::GpuPresenter, S, F>(config, state, ui_fn)
    }
    #[cfg(all(feature = "soft", not(feature = "gpu")))]
    {
        run_with::<soft::SoftPresenter, S, F>(config, state, ui_fn)
    }
    #[cfg(not(any(feature = "soft", feature = "gpu")))]
    {
        let _ = (config, state, ui_fn);
        Err("pixui was built with no backend; enable the `soft` or `gpu` feature".into())
    }
}

struct App<S, F, P> {
    config: Config,
    state: S,
    ui_fn: F,

    window: Option<Arc<Window>>,
    presenter: Option<P>,

    canvas: Canvas,
    input: Input,
    ui_state: UiState,

    /// Whole-number upscale currently in use.
    scale: i32,
    /// Top-left of the scaled canvas within the window, in physical pixels.
    offset: (i32, i32),
    mouse_phys: PhysicalPosition<f64>,

    /// The monitor the window is currently on, and its refresh rate.
    monitor: Option<winit::monitor::MonitorHandle>,
    display_fps: u32,

    /// Set by `PIXUI_PROFILE=1`. Prints a per-second breakdown of where the
    /// frame went, which is the only honest way to answer "is this fast
    /// enough" — the answer is usually not in the code you wrote.
    profile: bool,
    prof_ui: Duration,
    prof_present: Duration,
    prof_frames: u32,
    prof_since: Instant,

    start: Instant,
    last_frame: Instant,
    next_frame: Instant,
    frame_budget: Duration,
    applied_cursor: Cursor,
    quit_requested: bool,
}

impl<S, F, P> App<S, F, P>
where
    F: FnMut(&mut Ui, &mut S),
    P: Presenter,
{
    fn new(config: Config, state: S, ui_fn: F) -> Self {
        let canvas = Canvas::new(config.width, config.height);
        let now = Instant::now();
        // Replaced with the real display rate as soon as there is a window.
        let frame_budget = Duration::from_secs_f64(1.0 / 60.0);
        Self {
            config,
            state,
            ui_fn,
            window: None,
            presenter: None,
            canvas,
            input: Input {
                dt: 1.0 / 60.0,
                mouse_in_window: false,
                ..Default::default()
            },
            ui_state: UiState::new(),
            scale: 1,
            offset: (0, 0),
            mouse_phys: PhysicalPosition::new(0.0, 0.0),
            monitor: None,
            display_fps: 60,
            profile: std::env::var("PIXUI_PROFILE").is_ok(),
            prof_ui: Duration::ZERO,
            prof_present: Duration::ZERO,
            prof_frames: 0,
            prof_since: now,
            start: now,
            last_frame: now,
            next_frame: now,
            frame_budget,
            applied_cursor: Cursor::Default,
            quit_requested: false,
        }
    }

    /// Re-read which monitor the window is on and re-pace to it.
    fn detect_display(&mut self) {
        self.monitor = self.window.as_ref().and_then(|w| w.current_monitor());
        self.recompute_frame_budget();
    }

    /// Work out how often to draw, from the display and the config.
    fn recompute_frame_budget(&mut self) {
        // A monitor that reports something implausible (or nothing at all) is
        // more likely to be a driver quirk than a real 5Hz panel, so fall back.
        self.display_fps = self
            .monitor
            .as_ref()
            .and_then(|m| m.refresh_rate_millihertz())
            .map(|mhz| (mhz as f64 / 1000.0).round() as u32)
            .filter(|fps| *fps >= 24)
            .unwrap_or(60);

        let target = self.config.target_fps.unwrap_or(self.display_fps).max(1);
        // If the presenter blocks on the display, let it do the pacing and keep
        // the timer as a loose backstop against spinning when the window is
        // occluded and `present` returns immediately.
        let paced = self.presenter.as_ref().is_some_and(|p| p.paces_frames());
        let ticks = if paced {
            target.saturating_mul(2)
        } else {
            target
        };
        self.frame_budget = Duration::from_secs_f64(1.0 / ticks as f64);
    }

    /// Work out the magnification and, under [`Scaling::Adaptive`], the canvas
    /// size to go with it. See [`resolve_geometry`] for the arithmetic.
    fn recompute_scale(&mut self, win_w: i32, win_h: i32) {
        let dpr = self
            .window
            .as_ref()
            .map(|w| w.scale_factor())
            .unwrap_or(1.0);
        let geom = resolve_geometry(
            self.config.scaling,
            (self.canvas.width(), self.canvas.height()),
            self.config.initial_scale,
            dpr,
            (win_w, win_h),
        );
        self.scale = geom.scale;
        self.offset = geom.offset;
        self.canvas.resize(geom.canvas.0, geom.canvas.1);
    }

    /// Undo the scale and letterbox so widgets see virtual coordinates.
    fn update_mouse(&mut self) {
        let x = (self.mouse_phys.x as i32 - self.offset.0).div_euclid(self.scale);
        let y = (self.mouse_phys.y as i32 - self.offset.1).div_euclid(self.scale);
        self.input.mouse = Point::new(x, y);
    }

    fn draw_frame(&mut self) {
        let now = Instant::now();
        self.input.dt = (now - self.last_frame).as_secs_f32().clamp(0.0, 0.1);
        self.input.time = (now - self.start).as_secs_f32();
        self.last_frame = now;

        let t_ui = Instant::now();
        self.canvas.clear(self.config.theme.background);

        let out = {
            let mut ui = Ui::begin(
                &mut self.canvas,
                &self.input,
                &self.config.theme,
                &mut self.ui_state,
            );
            (self.ui_fn)(&mut ui, &mut self.state);
            ui.finish()
        };

        if let Some(theme) = out.theme {
            self.config.theme = theme;
        }
        self.quit_requested |= out.quit;

        let scanline = self.config.theme.scanline;
        if scanline > 0.0 {
            let bounds = self.canvas.bounds();
            self.canvas.scanlines(bounds, scanline);
        }

        if out.cursor != self.applied_cursor {
            self.applied_cursor = out.cursor;
            if let Some(w) = &self.window {
                w.set_cursor(match out.cursor {
                    Cursor::Default => CursorIcon::Default,
                    Cursor::Pointer => CursorIcon::Pointer,
                    Cursor::Grab => CursorIcon::Grab,
                    Cursor::Text => CursorIcon::Text,
                });
            }
        }
        let ui_elapsed = t_ui.elapsed();

        let t_present = Instant::now();
        if let Some(p) = self.presenter.as_mut() {
            p.present(
                &self.canvas,
                self.scale,
                self.offset,
                self.config.theme.letterbox,
            );
        }
        let present_elapsed = t_present.elapsed();

        self.input.begin_frame();

        if self.profile {
            self.prof_ui += ui_elapsed;
            self.prof_present += present_elapsed;
            self.prof_frames += 1;
            if self.prof_since.elapsed() >= Duration::from_secs(1) {
                let n = self.prof_frames.max(1) as f64;
                let ms = |d: Duration| d.as_secs_f64() * 1000.0 / n;
                let size = self.window.as_ref().map(|w| w.inner_size());
                println!(
                    "pixui[{}]: {:>3}/{} fps | ui+raster {:.3} ms | present {:.3} ms | {}x{} @ {}x",
                    P::NAME,
                    self.prof_frames,
                    self.display_fps,
                    ms(self.prof_ui),
                    ms(self.prof_present),
                    size.map(|s| s.width).unwrap_or(0),
                    size.map(|s| s.height).unwrap_or(0),
                    self.scale,
                );
                self.prof_ui = Duration::ZERO;
                self.prof_present = Duration::ZERO;
                self.prof_frames = 0;
                self.prof_since = Instant::now();
            }
        }
    }
}

/// Nearest-neighbour integer upscale of the whole canvas into `dst`, which is
/// `win_w` x `win_h` pixels of `0x00RRGGBB`.
///
/// Public because a CPU backend needs it: presenting a pixui canvas without a
/// GPU is exactly this call followed by whatever your platform does with a
/// buffer.
///
/// Each source row is expanded once into `scratch` and then memcpy'd `scale`
/// times. Only the letterbox bars are cleared, not the whole buffer: at a 6x
/// scale on a HiDPI display the destination is several megapixels, and every
/// one of them is about to be overwritten anyway.
#[allow(clippy::too_many_arguments)]
pub fn blit(
    canvas: &Canvas,
    scratch: &mut Vec<u32>,
    dst: &mut [u32],
    win_w: usize,
    win_h: usize,
    scale: usize,
    offset: (i32, i32),
    letterbox: Color,
) {
    let scale = scale.max(1);
    let vw = canvas.width() as usize;
    let vh = canvas.height() as usize;
    let (ox, oy) = (offset.0.max(0) as usize, offset.1.max(0) as usize);
    if ox >= win_w || oy >= win_h {
        dst.fill(letterbox.0);
        return;
    }

    let row_px = (vw * scale).min(win_w - ox);
    if row_px == 0 {
        dst.fill(letterbox.0);
        return;
    }

    // Bars above and below the scaled canvas.
    let n = dst.len();
    let band_end = (oy + vh * scale).min(win_h);
    dst[..(oy * win_w).min(n)].fill(letterbox.0);
    dst[(band_end * win_w).min(n)..].fill(letterbox.0);
    // Bars either side of it, row by row.
    if ox > 0 || ox + row_px < win_w {
        for y in oy..band_end {
            let row = &mut dst[y * win_w..(y + 1) * win_w];
            row[..ox].fill(letterbox.0);
            row[ox + row_px..].fill(letterbox.0);
        }
    }

    scratch.resize(vw * scale, 0);

    let src = canvas.pixels();
    for vy in 0..vh {
        let dy0 = oy + vy * scale;
        if dy0 >= win_h {
            break;
        }
        let line = &src[vy * vw..vy * vw + vw];
        for (i, &p) in line.iter().enumerate() {
            scratch[i * scale..(i + 1) * scale].fill(p);
        }
        for k in 0..scale.min(win_h - dy0) {
            let start = (dy0 + k) * win_w + ox;
            dst[start..start + row_px].copy_from_slice(&scratch[..row_px]);
        }
    }
}

/// Translate a winit key into pixui's small key set.
fn map_key(key: &WKey) -> Option<Key> {
    Some(match key {
        WKey::Named(NamedKey::Tab) => Key::Tab,
        WKey::Named(NamedKey::Enter) => Key::Enter,
        WKey::Named(NamedKey::Space) => Key::Space,
        WKey::Named(NamedKey::Escape) => Key::Escape,
        WKey::Named(NamedKey::Backspace) => Key::Backspace,
        WKey::Named(NamedKey::Delete) => Key::Delete,
        WKey::Named(NamedKey::ArrowLeft) => Key::Left,
        WKey::Named(NamedKey::ArrowRight) => Key::Right,
        WKey::Named(NamedKey::ArrowUp) => Key::Up,
        WKey::Named(NamedKey::ArrowDown) => Key::Down,
        WKey::Named(NamedKey::Home) => Key::Home,
        WKey::Named(NamedKey::End) => Key::End,
        WKey::Character(s) => Key::Char(s.chars().next()?),
        _ => return None,
    })
}

impl<S, F, P> ApplicationHandler for App<S, F, P>
where
    F: FnMut(&mut Ui, &mut S),
    P: Presenter,
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        // Size the window in *logical* units so it comes up the intended
        // physical size on screen; the actual integer scale is then derived
        // from the physical size, which is what makes HiDPI free.
        let scale = self.config.initial_scale.max(1);
        let logical = LogicalSize::new(
            (self.config.width * scale) as f64,
            (self.config.height * scale) as f64,
        );
        // Under Adaptive the canvas is the window divided by the scale, so the
        // minimum window size is what guarantees the minimum canvas size.
        let min = match self.config.scaling {
            Scaling::Fixed => LogicalSize::new(self.config.width as f64, self.config.height as f64),
            Scaling::Adaptive => LogicalSize::new(
                (self.config.min_width * scale) as f64,
                (self.config.min_height * scale) as f64,
            ),
        };

        let attrs = Window::default_attributes()
            .with_title(self.config.title.clone())
            .with_inner_size(logical)
            .with_min_inner_size(min)
            .with_resizable(self.config.resizable);

        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("pixui: could not create window: {e}");
                event_loop.exit();
                return;
            }
        };

        let presenter = match P::new(window.clone(), self.config.resolved_vsync()) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("pixui: could not start the {} backend: {e}", P::NAME);
                event_loop.exit();
                return;
            }
        };

        let size = window.inner_size();
        self.window = Some(window);
        self.presenter = Some(presenter);
        self.recompute_scale(size.width as i32, size.height as i32);
        self.detect_display();
        self.next_frame = Instant::now();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::Resized(size) => {
                self.recompute_scale(size.width as i32, size.height as i32);
                if let Some(p) = self.presenter.as_mut() {
                    p.resize(size.width, size.height);
                }
                self.update_mouse();
            }

            WindowEvent::CursorMoved { position, .. } => {
                self.mouse_phys = position;
                self.input.mouse_in_window = true;
                self.update_mouse();
            }

            // Dragging between a 120Hz laptop panel and a 60Hz external display
            // should change the pace. `Moved` fires constantly during a drag, so
            // only do the work when the monitor actually changed.
            // Moving to a display with a different density changes how many
            // physical pixels a virtual one should occupy.
            WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(size) = self.window.as_ref().map(|w| w.inner_size()) {
                    self.recompute_scale(size.width as i32, size.height as i32);
                    if let Some(p) = self.presenter.as_mut() {
                        p.resize(size.width, size.height);
                    }
                }
            }

            WindowEvent::Moved(_) => {
                let now = self.window.as_ref().and_then(|w| w.current_monitor());
                if now != self.monitor {
                    self.monitor = now;
                    self.recompute_frame_budget();
                }
            }

            WindowEvent::CursorEntered { .. } => self.input.mouse_in_window = true,
            WindowEvent::CursorLeft { .. } => self.input.mouse_in_window = false,

            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                match state {
                    ElementState::Pressed => {
                        self.input.mouse_down = true;
                        self.input.mouse_pressed = true;
                    }
                    ElementState::Released => {
                        self.input.mouse_down = false;
                        self.input.mouse_released = true;
                    }
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }

            WindowEvent::MouseWheel { delta, .. } => {
                self.input.wheel += match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 20.0,
                };
            }

            WindowEvent::ModifiersChanged(mods) => {
                let s = mods.state();
                self.input.mods = Mods {
                    shift: s.shift_key(),
                    cmd: if cfg!(target_os = "macos") {
                        s.super_key()
                    } else {
                        s.control_key()
                    },
                    ctrl: s.control_key(),
                    alt: s.alt_key(),
                };
            }

            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed {
                    if let Some(k) = map_key(&event.logical_key) {
                        self.input.keys.push(k);
                    }
                }
            }

            WindowEvent::RedrawRequested => {
                self.draw_frame();
                if self.quit_requested {
                    event_loop.exit();
                }
            }

            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Springs and hover blends need a frame every tick regardless of input,
        // so this redraws on a clock rather than only on events — but sleeps
        // between frames instead of spinning.
        let now = Instant::now();
        if now >= self.next_frame {
            self.next_frame = now + self.frame_budget;
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
    }
}
