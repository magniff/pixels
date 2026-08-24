//! A demo application for the `pixui` toolkit.
//!
//! # The split
//!
//! This crate depends on exactly one thing: `pixui`. It has no windowing
//! dependency, no rasteriser, no font, no colour maths of its own. That is not
//! a stylistic claim you have to take on faith — look at `Cargo.toml`. If any
//! toolkit concern had leaked into application code, this crate would need a
//! second dependency to express it, and it does not have one.
//!
//! What lives where:
//!
//! | In `pixui` (the library)                  | Here (the application)          |
//! |-------------------------------------------|---------------------------------|
//! | Window, event loop, present               | Screen layout                   |
//! | Rasterising, clipping, dithering           | Which widgets exist, and where   |
//! | Bitmap font and text measurement           | The words in them               |
//! | Widget look, press springs, focus ring     | What a click *means*            |
//! | Palette and theme                          | Which theme is selected         |
//!
//! The rule of thumb: the library owns everything that would be identical in a
//! different application; this file owns everything that would differ.

use pixui::{palette, Align, Rect, Theme, Tone, Ui};

/// The demo's theme: the stock warm look, with the library's own scanline pass
/// switched off so the demo can toggle it at runtime instead.
pub fn theme() -> Theme {
    let mut theme = Theme::warm();
    theme.scanline = 0.0;
    theme
}

const TABS: [&str; 4] = ["WIDGETS", "SCROLL", "PALETTE", "ABOUT"];

/// Everything the demo remembers between frames.
///
/// Note what is *not* here: no widget handles, no retained tree, no observers.
/// In immediate mode this struct is the entire UI state.
pub struct Demo {
    pub tab: usize,
    pub clicks: u32,
    pub mode: usize,
    pub scanlines: bool,
    pub autorun: bool,
    pub dither: bool,
    pub volume: f32,
    pub speed: f32,
    /// Fills on its own when `autorun` is on, to keep something moving.
    pub progress: f32,
    pub dark: bool,
    /// The scrolling tab's list. Long enough to actually need scrolling.
    pub items: Vec<bool>,
    pub playing: Option<usize>,
    /// Smoothed so the readout does not flicker every frame.
    pub fps: f32,
    pub last_action: String,
}

impl Default for Demo {
    fn default() -> Self {
        Self {
            tab: 0,
            clicks: 0,
            mode: 1,
            scanlines: true,
            autorun: true,
            dither: true,
            volume: 0.72,
            speed: 0.35,
            progress: 0.0,
            dark: false,
            items: (0..24).map(|i| i % 3 == 0).collect(),
            playing: Some(2),
            fps: 60.0,
            last_action: "READY".to_string(),
        }
    }
}

/// Called once per frame. Everything below here is ordinary application code.
pub fn frame(ui: &mut Ui, state: &mut Demo) {
    let dt = ui.input.dt;
    state.fps = pixui::smooth(state.fps, 1.0 / dt.max(1e-4), 3.0, dt);

    if state.autorun {
        state.progress = (state.progress + dt * (0.08 + state.speed * 0.5)) % 1.0;
    }

    let screen = ui.canvas.bounds();
    let (titlebar, rest) = screen.split_top(14);
    let (body, statusbar) = rest.split_bottom(12);

    draw_titlebar(ui, titlebar, state);
    draw_statusbar(ui, statusbar, state);

    ui.column(body.inset_xy(6, 5), 5, |ui| {
        ui.row_h(15, 5, |ui| {
            let tabs = ui.alloc(TABS.len() as i32 * 62);
            ui.segmented_at("tabs", tabs, &TABS, &mut state.tab);

            // Right-aligned theme switch, sharing the same row.
            let rest = ui.alloc_rest();
            let sw = Rect::new(rest.right() - 96, rest.y, 96, rest.h);
            let mut theme_idx = usize::from(state.dark);
            if ui.segmented_at("theme", sw, &["WARM", "NIGHT"], &mut theme_idx) {
                state.dark = theme_idx == 1;
                let mut t = if state.dark {
                    Theme::midnight()
                } else {
                    Theme::warm()
                };
                t.scanline = 0.0;
                ui.request_theme(t);
                state.last_action = if state.dark {
                    "THEME: NIGHT"
                } else {
                    "THEME: WARM"
                }
                .into();
            }
        });

        let panel_area = ui.alloc_rest();
        match state.tab {
            0 => tab_widgets(ui, panel_area, state),
            1 => tab_scroll(ui, panel_area, state),
            2 => tab_palette(ui, panel_area),
            _ => tab_about(ui, panel_area),
        }
    });

    if state.scanlines {
        let bounds = ui.canvas.bounds();
        ui.canvas.scanlines(bounds, 0.07);
    }
}

// ---------------------------------------------------------------------- chrome

fn draw_titlebar(ui: &mut Ui, rect: Rect, state: &Demo) {
    let fps = format!("{:>3} FPS", state.fps.round() as i32);
    ui.title_bar(rect, "PIXUI CONTROL DECK", Some(&fps));
}

fn draw_statusbar(ui: &mut Ui, rect: Rect, state: &Demo) {
    let th = *ui.theme;
    ui.canvas.fill_rect(rect, th.background.shade(-0.35));
    ui.canvas.hline(rect.x, rect.y, rect.w, th.panel_border);

    let inner = rect.inset_xy(6, 0);
    ui.draw_text_in(inner, &state.last_action, th.accent.face, Align::Left);
    ui.draw_text_in(
        inner,
        "TAB CYCLE  ENTER PRESS  ESC DROP",
        th.ink_soft,
        Align::Right,
    );
}

// ------------------------------------------------------------------- tab: main

fn tab_widgets(ui: &mut Ui, area: Rect, state: &mut Demo) {
    let inner = ui.panel(area, "WIDGET GALLERY");

    let (top, bottom) = inner.split_top(inner.h - 26);
    let col_w = (top.w - 8) / 2;

    ui.row(top, 8, |ui| {
        // ---- left column -------------------------------------------------
        ui.column_w(col_w, 4, |ui| {
            ui.heading("BUTTONS");
            ui.row_h(15, 4, |ui| {
                let w = (col_w - 8) / 3;
                let cell = ui.alloc(w);
                if ui.button_at(cell, "SAVE", Tone::Accent).clicked {
                    state.last_action = "SAVED".into();
                }
                let cell = ui.alloc(w);
                if ui.button_at(cell, "COPY", Tone::Neutral).clicked {
                    state.last_action = "COPIED".into();
                }
                let cell = ui.alloc_rest();
                if ui.button_at(cell, "WIPE", Tone::Danger).clicked {
                    state.clicks = 0;
                    state.last_action = "WIPED".into();
                }
            });

            if ui.button("INCREMENT").clicked {
                state.clicks += 1;
                state.last_action = format!("COUNT {}", state.clicks);
            }
            ui.value_row("CLICKS", &state.clicks.to_string());
            ui.separator();

            ui.heading("MODE");
            if ui.segmented("mode", &["SOFT", "HARD", "OFF"], &mut state.mode) {
                state.last_action = format!("MODE {}", ["SOFT", "HARD", "OFF"][state.mode]);
            }
        });

        // ---- right column ------------------------------------------------
        let right = ui.alloc_rest();
        ui.column(right, 4, |ui| {
            ui.heading("SWITCHES");
            if ui.toggle("SCANLINES", &mut state.scanlines).changed {
                state.last_action = if state.scanlines { "CRT ON" } else { "CRT OFF" }.into();
            }
            if ui.toggle("AUTORUN", &mut state.autorun).changed {
                state.last_action = if state.autorun { "RUNNING" } else { "PAUSED" }.into();
            }
            ui.checkbox("DITHERING", &mut state.dither);
            ui.separator();

            ui.heading("LEVELS");
            let vol = format!("{:>3}%", (state.volume * 100.0).round() as i32);
            if ui
                .slider_labeled("VOLUME", &mut state.volume, 0.0, 1.0, &vol)
                .changed
            {
                state.last_action = format!("VOLUME {vol}");
            }
        });
    });

    // ---- full-width readout ---------------------------------------------
    ui.column(bottom, 3, |ui| {
        let pct = format!("{:>3}%", (state.progress * 100.0).round() as i32);
        ui.value_row("BUILD", &pct);
        let tone = if state.progress > 0.85 {
            Tone::Positive
        } else {
            Tone::Info
        };
        ui.progress(state.progress, tone);
    });
}

// ----------------------------------------------------------------- tab: scroll

fn tab_scroll(ui: &mut Ui, area: Rect, state: &mut Demo) {
    let inner = ui.panel(area, "SCROLLING");
    let (list, footer) = inner.split_bottom(11);
    let count = state.items.len();

    let (_, scroll) = ui.scroll_area(list, "tracks", |ui| {
        for i in 0..count {
            if i % 8 == 0 {
                ui.heading(&format!("SIDE {}", (b'A' + (i / 8) as u8) as char));
            }
            // Each row gets its own identity scope. Without it the PLAY/STOP
            // buttons would be identified by their labels, and the ids of every
            // row after the playing one would shift the moment it changed —
            // taking their animation state with them.
            ui.scope(&format!("row{i}"), |ui| {
                ui.row_h(15, 4, |ui| {
                    let cb = ui.alloc(150);
                    ui.checkbox_at(cb, &format!("TRACK {:02}", i + 1), &mut state.items[i]);

                    let rest = ui.alloc_rest();
                    let btn = Rect::new(rest.right() - 52, rest.y, 52, rest.h);
                    let playing = state.playing == Some(i);
                    let tone = if playing {
                        Tone::Positive
                    } else {
                        Tone::Neutral
                    };
                    let label = if playing { "STOP" } else { "PLAY" };
                    if ui.button_at(btn, label, tone).clicked {
                        state.playing = if playing { None } else { Some(i) };
                        state.last_action = format!("TRACK {:02}", i + 1);
                    }
                });
            });
        }
    });

    let pct = if scroll.max_offset() > 0.0 {
        (scroll.shown / scroll.max_offset() * 100.0).round() as i32
    } else {
        100
    };
    ui.column(footer, 2, |ui| {
        ui.value_row(
            &format!("{count} TRACKS  WHEEL OR DRAG"),
            &format!("{pct:>3}%"),
        );
    });
}

// ---------------------------------------------------------------- tab: palette

fn tab_palette(ui: &mut Ui, area: Rect) {
    let inner = ui.panel(area, "PALETTE");
    let (grid, strips) = inner.split_top(inner.h - 52);

    let cols = 5;
    let cell_w = grid.w / cols;
    let cell_h = 26;

    for (i, (name, color)) in palette::ALL.iter().enumerate() {
        let cx = grid.x + (i as i32 % cols) * cell_w;
        let cy = grid.y + (i as i32 / cols) * cell_h;
        if cy + cell_h > grid.bottom() {
            break;
        }
        let cell = Rect::new(cx, cy, cell_w - 3, cell_h - 3);
        let (swatch, label) = cell.split_top(13);
        ui.canvas
            .box_chamfer(swatch, *color, ui.theme.panel_border, 1);
        ui.draw_text_in(label, name, ui.theme.ink_soft, Align::Left);
    }

    // Two strips showing why a small palette is not actually a limit: ordered
    // dithering buys you the in-between colours without adding any.
    ui.column(strips, 2, |ui| {
        let th = *ui.theme;

        ui.value_row("ORDERED DITHER", "ACCENT -> TEAL");
        let bar = ui.alloc(11);
        ui.canvas
            .gradient_rect(bar, th.accent.face, th.info.face, false);
        ui.canvas.stroke_rect(bar, th.panel_border);

        ui.value_row("SHADE RAMP", "-60% .. +60%");
        let bar = ui.alloc(11);
        let steps = 13;
        let step_w = bar.w / steps;
        for i in 0..steps {
            let t = -0.6 + 1.2 * i as f32 / (steps - 1) as f32;
            let x = bar.x + i * step_w;
            let w = if i == steps - 1 {
                bar.right() - x
            } else {
                step_w
            };
            ui.canvas
                .fill_rect(Rect::new(x, bar.y, w, bar.h), th.accent.face.shade(t));
        }
        ui.canvas.stroke_rect(bar, th.panel_border);
    });
}

// ------------------------------------------------------------------ tab: about

fn tab_about(ui: &mut Ui, area: Rect) {
    let inner = ui.panel(area, "ABOUT");
    ui.column(inner, 3, |ui| {
        ui.heading("TWO CRATES, ONE RULE");
        ui.label_dim("PIXUI OWNS WHAT EVERY APP WOULD SHARE.");
        ui.label_dim("THIS DEMO OWNS WHAT MAKES IT THIS APP.");
        ui.space(3);

        ui.value_row("LIBRARY", "CRATES/PIXUI");
        ui.value_row("DEMO DEPS", "PIXUI ONLY");
        ui.value_row("RENDERER", "CPU, 384X240");
        ui.value_row("SCALING", "INTEGER + LETTERBOX");
        ui.value_row("FONT", "5X7 BITMAP, ASCII");
        ui.space(3);

        ui.heading("THE STACK");
        ui.value_row("APP", "THIS FILE");
        ui.value_row("WIDGETS + THEME", "PIXUI::WIDGETS");
        ui.value_row("RASTERISER", "PIXUI::CANVAS");
        ui.value_row("BACKEND", "PIXUI::APP");
        ui.space(3);

        ui.heading("KNOWN LIMITS");
        ui.label_colored("NO IME. NO SHAPING. NO A11Y TREE.", palette::RED);
    });
}
