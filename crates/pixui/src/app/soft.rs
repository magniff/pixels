//! CPU presenter: upscale into a window-sized buffer, hand it to softbuffer.
//!
//! Straightforward, dependency-light, and works anywhere winit does. Its cost
//! is that it moves a *lot* of memory: at a 6x scale the buffer handed over
//! each frame is 36x larger than the canvas that produced it, and the platform
//! then copies it again on its way to the screen. See [`super::gpu`] for the
//! alternative.

use std::error::Error;
use std::num::NonZeroU32;
use std::sync::Arc;

use winit::window::Window;

use super::{blit, Presenter};
use crate::canvas::Canvas;
use crate::color::Color;

pub struct SoftPresenter {
    window: Arc<Window>,
    /// Held because the surface borrows from it for its whole life.
    _context: softbuffer::Context<Arc<Window>>,
    surface: softbuffer::Surface<Arc<Window>, Arc<Window>>,
    /// One scaled output row, reused every frame to keep the blit allocation-free.
    scratch: Vec<u32>,
}

impl Presenter for SoftPresenter {
    const NAME: &'static str = "soft";

    fn new(window: Arc<Window>, _vsync: bool) -> Result<Self, Box<dyn Error>> {
        // softbuffer presents whenever it is told to; there is no swapchain to
        // pace against, so the vsync request has nowhere to go.
        let context = softbuffer::Context::new(window.clone())?;
        let surface = softbuffer::Surface::new(&context, window.clone())?;
        Ok(Self {
            window,
            _context: context,
            surface,
            scratch: Vec::new(),
        })
    }

    fn resize(&mut self, _width: u32, _height: u32) {
        // Handled per-frame in `present`, which has to check anyway.
    }

    fn present(&mut self, canvas: &Canvas, scale: i32, offset: (i32, i32), letterbox: Color) {
        let size = self.window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else {
            return;
        };
        if self.surface.resize(w, h).is_err() {
            return;
        }
        let Ok(mut buffer) = self.surface.buffer_mut() else {
            return;
        };

        blit(
            canvas,
            &mut self.scratch,
            &mut buffer,
            size.width as usize,
            size.height as usize,
            scale as usize,
            offset,
            letterbox,
        );

        let _ = buffer.present();
    }
}
