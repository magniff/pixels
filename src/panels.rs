//! The two panels behind the Pixels menu.
//!
//! Both are drawn over everything and take the pointer with them, the way the
//! file dialogs do. They are application chrome rather than toolkit widgets:
//! what a setting *means* — which weights exist, what a good prompt is, how to
//! fetch one — is this app's business, and the toolkit contributes the panel,
//! the buttons and the text area they are built from.

use pixui::{font, Align, Key, Rect, ScrollState, Tone, Ui};

use crate::fetch::{megabytes, Download};
use crate::markdown;
use crate::settings::{self, Settings, CATALOGUE};
use crate::text::{Buffer, Cursor};
use crate::vim::{Mode, Vim};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Panel {
    About,
    Settings,
}

/// Which page of the settings is showing.
///
/// One page so far, and a list to reach it from. The list is the point: the
/// next setting that turns up has somewhere to go that is not the bottom of a
/// panel about something else.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Page {
    #[default]
    Index,
    Appearance,
    Assistant,
}

/// What the settings panel wants done about what was just pressed.
pub enum Action {
    None,
    Close,
    /// Run these weights from now on.
    Use(String),
    /// Fetch this entry of the catalogue.
    Fetch(usize),
    /// Stop fetching.
    Cancel,
    /// The prompt was edited, so the assistant needs rebuilding with it.
    Prompt,
    /// Wear this colour scheme.
    Scheme(String),
    /// Read in this face.
    Font(String),
    /// The context ceiling moved, so the assistant needs rebuilding with it.
    Context,
}

/// The windows offered, in tokens. Powers of two because that is how everything
/// downstream thinks about them, and stopping at 32K because past that the
/// key/value cache for a twenty-billion-parameter model is measured in gigabytes
/// and this is a note editor.
const WINDOWS: &[u32] = &[4096, 8192, 16384, 32768];

/// The system prompt, edited the way everything else in this app is edited.
///
/// A small instance of the same machinery the notes use: the same buffer, the
/// same vim grammar, the same scrollbar. A settings field that cannot do what
/// the rest of the app does is a settings field you have to think about.
pub struct PromptEditor {
    pub buf: Buffer,
    pub vim: Vim,
    /// First visual row on screen. Rows, not lines: the prompt is one long
    /// paragraph and every row of it is a wrap.
    pub scroll: usize,
    bar: ScrollState,
}

impl PromptEditor {
    pub fn new(text: &str) -> Self {
        Self {
            buf: Buffer::from_text(text),
            vim: Vim::new(),
            scroll: 0,
            bar: ScrollState::default(),
        }
    }

    pub fn text(&self) -> String {
        self.buf.to_text()
    }

    /// Every wrapped row of the buffer, as (line, from, to).
    fn rows(&self, cols: usize) -> Vec<(usize, usize, usize)> {
        let mut out = Vec::new();
        for line in 0..self.buf.line_count() {
            for (from, to) in markdown::wrap_ranges(self.buf.line(line), cols) {
                out.push((line, from, to));
            }
        }
        out
    }

    /// Draw it, take its keys, and say whether the text changed.
    fn show(&mut self, ui: &mut Ui, rect: Rect) -> bool {
        let th = *ui.theme;
        let line_h = font::line_h();
        let advance = font::advance();

        ui.canvas.box_chamfer(rect, th.well, th.well_border, 1);
        let track = Rect::new(
            rect.right() - Ui::BAR_W - 1,
            rect.y + 1,
            Ui::BAR_W,
            rect.h - 2,
        );
        let inner = Rect::new(
            rect.x + 3,
            rect.y + 3,
            rect.w - 6 - Ui::SCROLL_GUTTER,
            rect.h - 6,
        );
        let cols = ((inner.w / advance).max(8)) as usize;
        let visible = (inner.h / line_h).max(1) as usize;

        // ---- keys ---------------------------------------------------------
        let before = self.buf.to_text();
        let id = ui.id("prompt-editor");
        let resp = ui.interact(id, rect);
        if resp.hovered {
            ui.request_cursor(pixui::Cursor::Text);
        }
        for key in ui.input.keys.clone() {
            // Escape in insert mode is vim's; in normal mode the panel wants
            // it, and takes it before this ever runs.
            self.vim.handle(&mut self.buf, key, ui.input.mods);
        }
        let changed = self.buf.to_text() != before;

        // ---- the pointer --------------------------------------------------
        let rows = self.rows(cols);
        if resp.held {
            let row = ((ui.input.mouse.y - inner.y) / line_h).max(0) as usize + self.scroll;
            if let Some(&(line, from, to)) = rows.get(row.min(rows.len().saturating_sub(1))) {
                let col = ((ui.input.mouse.x - inner.x) / advance).max(0) as usize;
                self.vim
                    .click_at(&mut self.buf, Cursor::new(line, (from + col).min(to)));
            }
        }

        // ---- the view, moved by hand --------------------------------------
        // Before the caret is chased, and the caret is carried along with it —
        // the same order the note panes use, and for the same reason. Scroll
        // first and follow second and every notch of the wheel is undone by the
        // follow on the very next frame: the view jumps back to the caret,
        // which has not moved, and the whole thing sticks to the caret and
        // shivers.
        let last_top = rows.len().saturating_sub(visible.min(rows.len()));
        let where_caret_is = |buf: &Buffer, rows: &[(usize, usize, usize)]| {
            let caret = buf.cursor;
            rows.iter()
                .position(|&(line, from, to)| {
                    line == caret.line && caret.col >= from && caret.col <= to
                })
                .unwrap_or(0)
        };

        let was = self.scroll;
        if ui.input.wheel != 0.0 && resp.hovered {
            let step = self.bar.wheel_rows(ui.input.wheel, 3.0);
            self.scroll = (self.scroll as i32 - step).clamp(0, last_top as i32) as usize;
        }

        // ---- the bar, the same one both note panes carry -------------------
        let mut st = self.bar;
        st.content = rows.len() as i32 * line_h;
        st.viewport = visible as i32 * line_h;
        st.target = self.scroll as f32 * line_h as f32;
        st.shown = st.target;
        ui.scroll_bar(track, "prompt-bar", &mut st);
        self.scroll = (st.target / line_h as f32).round().max(0.0) as usize;
        self.bar = st;
        self.scroll = self.scroll.min(last_top);

        if self.scroll != was && !rows.is_empty() {
            // The caret goes to the nearest row still on screen, so the follow
            // below has nothing left to chase.
            let at = where_caret_is(&self.buf, &rows);
            let want = at
                .clamp(self.scroll, self.scroll + visible - 1)
                .min(rows.len() - 1);
            if want != at {
                let (line, from, _) = rows[want];
                self.buf.cursor = Cursor::new(line, from);
            }
        }

        // ---- keep the caret on screen -------------------------------------
        let caret = self.buf.cursor;
        let at = where_caret_is(&self.buf, &rows);
        if at < self.scroll {
            self.scroll = at;
        } else if at >= self.scroll + visible {
            self.scroll = at + 1 - visible;
        }

        // ---- draw ----------------------------------------------------------
        let selection = self.vim.selection(&self.buf);
        let insert = self.vim.mode == Mode::Insert;
        ui.clipped(inner, |ui| {
            for (i, &(line, from, to)) in rows.iter().enumerate().skip(self.scroll).take(visible) {
                let y = inner.y + (i - self.scroll) as i32 * line_h;
                let text = self.buf.line(line);
                if let Some((lo, hi)) =
                    selection.and_then(|s| s.columns_on(line, text.chars().count()))
                {
                    let a = lo.max(from);
                    let b = hi.min(to.max(from));
                    if b > a {
                        let x = inner.x + (a - from) as i32 * advance - 1;
                        ui.canvas.fill_rect(
                            Rect::new(x, font::row_top(y), (b - a) as i32 * advance, line_h),
                            th.accent.lo,
                        );
                    }
                }
                let slice: String = text.chars().skip(from).take(to - from).collect();
                font::draw_text(ui.canvas, inner.x, y, &slice, th.ink_light);

                if line == caret.line && caret.col >= from && caret.col <= to {
                    let x = inner.x + (caret.col - from) as i32 * advance;
                    if insert {
                        ui.canvas.fill_rect(
                            Rect::new(x - 1, font::row_top(y), 2, line_h),
                            th.positive.face,
                        );
                    } else {
                        ui.canvas.fill_rect(
                            Rect::new(x - 1, font::row_top(y), font::glyph_w() + 2, line_h),
                            th.accent.face,
                        );
                        let under = text.chars().nth(caret.col).unwrap_or(' ');
                        font::draw_char(ui.canvas, x, y, under, th.accent.ink);
                    }
                }
            }
        });
        changed
    }
}

/// Everything the chrome remembers between frames.
#[derive(Default)]
pub struct Chrome {
    pub menu_open: bool,
    /// How tall the settings panel wanted to be last time it drew.
    pub panel_h: i32,
    /// Which page that height belongs to. A height measured for another page is
    /// somebody else's, and drawing at it is the flicker this avoids.
    pub measured: Option<Page>,
    /// The settings as they were when the panel opened, so closing it can tell
    /// whether anything actually changed. Rebuilding the assistant means
    /// loading a model again, which is not a thing to do for nothing.
    pub opened_with: Option<Settings>,
    /// The prompt, while it is being edited.
    pub prompt: Option<PromptEditor>,
    /// Which settings page is open.
    pub page: Page,
    pub panel: Option<Panel>,
    pub download: Option<Download>,
    /// The last thing worth saying in the settings panel.
    pub notice: String,
}

/// Wide enough to edit a prompt in. The panel is a place of work, not a
/// notification, and the prompt inside it is a paragraph.
const WIDTH: i32 = 430;
/// A narrower panel for the one that only has something to say.
const ABOUT_W: i32 = 330;
/// Room for the prompt: most of it at once, and room to move around in.
const PROMPT_H: i32 = 100;

/// What the app is, and which build of it this is.
pub fn about(ui: &mut Ui) -> bool {
    let screen = ui.canvas.bounds();
    ui.canvas
        .fill_rect_blend(screen, pixui::palette::VOID, 0.55);
    let rect = screen.centered(ABOUT_W, 150);
    let inner = ui.panel(rect, "ABOUT");
    ui.capture_keyboard();

    let mut closed = ui.input.key_pressed(Key::Escape);
    let (body, footer) = inner.split_bottom(19);

    ui.column(body, 3, |ui| {
        ui.heading("PIXUI NOTES");
        ui.label_dim("A MARKDOWN EDITOR WITH VIM KEYS, DRAWN");
        ui.label_dim("ONE PIXEL AT A TIME. NO WIDGET TOOLKIT,");
        ui.label_dim("NO FONT ENGINE, NO GPU EXCEPT TO SHOW");
        ui.label_dim("THE FINISHED FRAME.");
        ui.space(2);
        ui.value_row("VERSION", env!("CARGO_PKG_VERSION"));
        // The trailing `+` means the tree had changes the commit does not.
        ui.value_row("BUILD", env!("GIT_REV"));
    });

    let close = Rect::new(footer.right() - 70, footer.y + 4, 70, 15);
    if ui.button_at(close, "CLOSE", Tone::Neutral).clicked {
        closed = true;
    }
    closed
}

/// Which weights to run, what to tell them, and how to get more.
pub fn settings(ui: &mut Ui, config: &mut Settings, chrome: &mut Chrome) -> Action {
    let th = *ui.theme;
    let screen = ui.canvas.bounds();

    // A panel is as tall as what goes in it, and what goes in it is only known
    // once it has been laid out — which is a frame too late to draw it at the
    // right size. The old answer was to draw it at the last height and let the
    // next frame correct it, and the correction is visible: the panel arrives,
    // shifts, and settles.
    //
    // So the frame that has no height to trust is laid out and not painted.
    // Everything runs — the layout, the measuring, the request for another
    // frame — into a clip with nothing inside it, and the panel appears one
    // frame later at the size it actually wants. Hit testing goes through the
    // same clip, so there is nothing to click on a panel that is not there yet.
    let drawing = chrome.page;
    let sized = chrome.measured == Some(drawing);
    if !sized {
        ui.canvas.push_clip(Rect::new(0, 0, 0, 0));
        ui.request_repaint();
    }

    ui.canvas
        .fill_rect_blend(screen, pixui::palette::VOID, 0.55);
    let height = chrome.panel_h.clamp(72, screen.h - 20);
    let rect = screen.centered(WIDTH, height);
    let inner = ui.panel(rect, "SETTINGS");
    ui.capture_keyboard();

    // Escape is vim's while vim is in a mode that means something by it, and
    // the panel's otherwise. One key, two owners, and the inner one goes first
    // — the same bargain the editor strikes with the toolkit.
    let editing = chrome
        .prompt
        .as_ref()
        .is_some_and(|p| p.vim.mode != Mode::Normal);
    let mut action = Action::None;
    if ui.input.key_pressed(Key::Escape) && !editing {
        // Out of the page first, out of the panel second: one Escape should
        // undo one step, not every step.
        if chrome.page == Page::Index {
            action = Action::Close;
        } else {
            chrome.page = Page::Index;
        }
    }

    // What would actually run: the chosen weights, or — when nothing has been
    // chosen — whatever is installed, which is what the assistant falls back to.
    let running = config
        .model_path()
        .and_then(|p| p.file_name().and_then(|n| n.to_str()).map(String::from));

    let (body, footer) = inner.split_bottom(19);
    let (_, used) = ui.column_measured(body, 2, |ui| match chrome.page {
        Page::Index => {
            for (page, label, note) in [
                (Page::Appearance, "APPEARANCE", "THE COLOUR SCHEME"),
                (
                    Page::Assistant,
                    "AI ASSISTANT",
                    "WHICH MODEL, AND WHAT TO TELL IT",
                ),
            ] {
                let row = ui.alloc(15);
                if ui.button_at(row, label, Tone::Neutral).clicked {
                    chrome.page = page;
                }
                ui.space(1);
                let hint = ui.alloc(8);
                ui.draw_text_in(hint, note, th.ink_soft, Align::Left);
                ui.space(2);
            }
        }

        Page::Appearance => {
            // j and k walk the list, and wearing is what walking *is*: the
            // point of a scheme list is to see the scheme, and a preview you
            // have to ask for twice is not a preview.
            let step = if ui.input.key_pressed(Key::Char('j')) || ui.input.key_pressed(Key::Down) {
                1
            } else if ui.input.key_pressed(Key::Char('k')) || ui.input.key_pressed(Key::Up) {
                -1
            } else {
                0
            };
            if step != 0 {
                let count = pixui::SCHEMES.len() as i32;
                let at = pixui::SCHEMES
                    .iter()
                    .position(|(name, _)| config.scheme.eq_ignore_ascii_case(name))
                    .unwrap_or(0) as i32;
                let next = (at + step).rem_euclid(count) as usize;
                action = Action::Scheme(pixui::SCHEMES[next].0.to_string());
            }

            let head = ui.alloc(15);
            let (back, title) = head.split_left(46);
            if ui.button_at(back, "BACK", Tone::Neutral).clicked {
                chrome.page = Page::Index;
            }
            ui.draw_text_in(title, "  APPEARANCE", th.ink, Align::Left);
            ui.draw_text_in(title, "J/K TO TRY THEM ", th.ink_soft, Align::Right);

            // Every scheme the toolkit ships, each with a strip of its own
            // colours: a name tells you nothing about a palette, and five
            // swatches tell you most of it without having to put it on.
            let head = ui.alloc(9);
            ui.draw_text_in(head, "SCHEME", th.accent.face, Align::Left);
            for (name, build) in pixui::SCHEMES {
                let row = ui.alloc(13);
                let current = config.scheme.eq_ignore_ascii_case(name);
                let scheme = build();
                if current {
                    ui.canvas.fill_rect(row, th.accent.lo);
                }
                // The name takes what is left after the swatches and the
                // button, both of which are a fixed size.
                let (label, rest) = row.split_left(row.w - 124);
                let ink = if current { th.accent.ink } else { th.ink };
                ui.draw_text_in(label.translate(4, 0), name, ink, Align::Left);

                // The swatches, in the order they matter: the page, the accent,
                // and the three the app spends most of its colour on.
                let swatches = [
                    scheme.background,
                    scheme.accent.face,
                    scheme.danger.face,
                    scheme.positive.face,
                    scheme.info.face,
                ];
                let mut x = rest.x;
                for (i, colour) in swatches.iter().enumerate() {
                    let at = Rect::new(x, rest.y + 2, 12, 9);
                    ui.canvas.fill_rect(at, *colour);
                    // One outline around the strip rather than five, so the
                    // swatches read as a palette and not as five buttons.
                    if i == 0 {
                        ui.canvas.vline(at.x - 1, at.y - 1, at.h + 2, th.ink_soft);
                    }
                    ui.canvas.hline(at.x, at.y - 1, at.w, th.ink_soft);
                    ui.canvas.hline(at.x, at.bottom(), at.w, th.ink_soft);
                    x += 12;
                }
                ui.canvas.vline(x, rest.y + 1, 11, th.ink_soft);

                let button = Rect::new(rest.right() - 52, rest.y, 52, 13);
                if current {
                    ui.draw_text_in(button, "IN USE", th.positive.face, Align::Center);
                } else if ui.button_at(button, "WEAR", Tone::Accent).clicked {
                    action = Action::Scheme(name.to_string());
                }
            }

            // ---- and the face ------------------------------------------
            ui.space(3);
            let head = ui.alloc(9);
            ui.draw_text_in(head, "FONT", th.accent.face, Align::Left);
            for (i, face) in pixui::font::FACES.iter().enumerate() {
                let row = ui.alloc(pixui::font::line_h() + 4);
                let current = config.font.eq_ignore_ascii_case(face.name);
                if current {
                    ui.canvas.fill_rect(row, th.accent.lo);
                }
                let ink = if current { th.accent.ink } else { th.ink };
                let (label, rest) = row.split_left(row.w - 124);
                ui.draw_text_in(label.translate(4, 0), face.name, ink, Align::Left);
                // What it costs in room, which is the thing you are choosing
                // between as much as the shapes are.
                let size = format!("{}x{}", face.glyph_w, face.glyph_h);
                ui.draw_text_in(rest, &size, th.ink_soft, Align::Left);

                let button = Rect::new(rest.right() - 52, rest.y, 52, 13);
                if current {
                    ui.draw_text_in(button, "IN USE", th.positive.face, Align::Center);
                } else if ui.button_at(button, "READ IN", Tone::Accent).clicked {
                    action = Action::Font(face.name.to_string());
                }
                let _ = i;
            }
        }
        Page::Assistant => {
            let head = ui.alloc(15);
            let (back, title) = head.split_left(46);
            if ui.button_at(back, "BACK", Tone::Neutral).clicked {
                chrome.page = Page::Index;
            }
            ui.draw_text_in(title, "  AI ASSISTANT", th.ink, Align::Left);

            let row = ui.alloc(15);
            ui.toggle_at(row, "AI ASSISTANCE", &mut config.assist);

            // Everything below belongs to the switch above it. Off, it is
            // drawn and then veiled: what is there stays legible, and none of
            // it answers the pointer.
            let locked = !config.assist;
            let from = ui.remaining().y;
            ui.input_blocked(locked, |ui| {
                // ---- the models, as a table --------------------------
                // Columns, rules and a header: three facts about each of a
                // handful of things is a table, and a table read as a table is
                // read at a glance.
                let head = ui.alloc(9);
                let cols = columns(head.w);
                cell(ui, head, &cols, 0, "MODEL", th.accent.face, Align::Left);
                cell(ui, head, &cols, 1, "GOOD FOR", th.accent.face, Align::Left);
                cell(ui, head, &cols, 2, "SIZE", th.accent.face, Align::Right);
                ui.canvas
                    .hline(head.x, head.bottom() - 1, head.w, th.well_border);

                for (i, weights) in CATALOGUE.iter().enumerate() {
                    let path = settings::models_dir().join(weights.file);
                    // Present is not the same as whole. A file that stopped
                    // halfway loads as nothing at all, and llama.cpp says so in
                    // its own vocabulary — better to notice here, where the
                    // catalogue knows how big the thing was supposed to be.
                    let short = crate::fetch::short_file(&path, weights.megabytes);
                    let here = path.exists() && short.is_none();
                    let current = running.as_deref() == Some(weights.file);
                    let row = ui.alloc(13);
                    // A quiet band on every other row, which is what stops the
                    // eye slipping between one row's name and another's size.
                    if i % 2 == 1 {
                        ui.canvas.fill_rect(row, th.panel.shade(-0.06));
                    }
                    let ink = if current { th.positive.face } else { th.ink };
                    cell(ui, row, &cols, 0, weights.label, ink, Align::Left);
                    cell(ui, row, &cols, 1, weights.note, th.ink_soft, Align::Left);
                    let (size, size_ink) = match short {
                        Some(have) => (
                            format!("{} / {} MB", have / 1_000_000, weights.megabytes),
                            th.danger.face,
                        ),
                        None => (format!("{} MB", weights.megabytes), th.ink_soft),
                    };
                    cell(ui, row, &cols, 2, &size, size_ink, Align::Right);

                    let at = Rect::new(row.x + cols[3] + 2, row.y, row.w - cols[3] - 2, 13);
                    if current {
                        ui.draw_text_in(at, "IN USE", th.positive.face, Align::Center);
                    } else if here {
                        if ui.button_at(at, "USE", Tone::Accent).clicked {
                            action = Action::Use(weights.file.to_string());
                        }
                    } else if chrome.download.is_some() {
                        ui.draw_text_in(at, "-", th.ink_soft, Align::Center);
                    } else {
                        // The same button either way: fetching one of these
                        // resumes from whatever is already on disk, so there is
                        // nothing for a second verb to mean.
                        let label = if short.is_some() { "RESUME" } else { "GET" };
                        if ui.button_at(at, label, Tone::Neutral).clicked {
                            action = Action::Fetch(i);
                        }
                    }
                    // The rules between the columns, drawn last so nothing
                    // paints over them.
                    for x in &cols[1..3] {
                        ui.canvas.vline(row.x + x - 3, row.y, row.h, th.well_border);
                    }
                }

                // ---- anything else already on disk -------------------------
                for name in extras(config) {
                    let row = ui.alloc(13);
                    let current = running.as_deref() == Some(name.as_str());
                    let ink = if current { th.positive.face } else { th.ink };
                    cell(ui, row, &cols, 0, &name.to_uppercase(), ink, Align::Left);
                    cell(
                        ui,
                        row,
                        &cols,
                        1,
                        "ALREADY ON DISK",
                        th.ink_soft,
                        Align::Left,
                    );
                    let at = Rect::new(row.x + cols[3] + 2, row.y, row.w - cols[3] - 2, 13);
                    if current {
                        ui.draw_text_in(at, "IN USE", th.positive.face, Align::Center);
                    } else if ui.button_at(at, "USE", Tone::Accent).clicked {
                        action = Action::Use(name.clone());
                    }
                    for x in &cols[1..3] {
                        ui.canvas.vline(row.x + x - 3, row.y, row.h, th.well_border);
                    }
                }

                // ---- whatever is happening -----------------------------------------
                if let Some(down) = &chrome.download {
                    let row = ui.alloc(9);
                    let (label, button) = row.split_left(row.w - 56);
                    ui.draw_text_in(
                        label,
                        &format!("{} {}", down.label, megabytes(down.bytes())),
                        th.info.hi,
                        Align::Left,
                    );
                    let at = Rect::new(button.x + 4, row.y - 3, 52, 13);
                    if ui.button_at(at, "STOP", Tone::Danger).clicked {
                        action = Action::Cancel;
                    }
                    let bar = ui.alloc(7);
                    ui.progress_at(
                        Rect::new(bar.x, bar.y, bar.w, 7),
                        down.fraction(),
                        Tone::Info,
                    );
                } else if !chrome.notice.is_empty() {
                    let row = ui.alloc(8);
                    ui.draw_text_in(row, &chrome.notice, th.danger.face, Align::Left);
                }

                // ---- how much room the model is given ----------------------
                // A ceiling rather than a size: a request opens the smallest
                // window that fits it, and this is how far it may go. The cache
                // is allocated in the same memory the weights are in, so the
                // largest setting is a promise about memory, not about quality.
                ui.space(3);
                let row = ui.alloc(13);
                let (label, choices) = row.split_left(row.w - WINDOWS.len() as i32 * 44);
                ui.draw_text_in(label, "CONTEXT WINDOW", th.accent.face, Align::Left);
                for (i, window) in WINDOWS.iter().enumerate() {
                    let at = Rect::new(choices.x + i as i32 * 44, row.y, 42, 13);
                    let worn = config.context == *window;
                    let tone = if worn { Tone::Accent } else { Tone::Neutral };
                    if ui
                        .button_at(at, &format!("{}K", window / 1024), tone)
                        .clicked
                        && !worn
                    {
                        config.context = *window;
                        action = Action::Context;
                    }
                }

                ui.space(2);
                let head = ui.alloc(8);
                ui.draw_text_in(head, "SYSTEM PROMPT", th.accent.face, Align::Left);
                // A vim editor with no mode showing is a vim editor you have to guess
                // at, so the heading doubles as the badge.
                let editor = chrome
                    .prompt
                    .get_or_insert_with(|| PromptEditor::new(&config.prompt));
                let (mode, tint) = match editor.vim.mode {
                    Mode::Insert => ("INSERT", th.positive.face),
                    Mode::Visual(_) => ("VISUAL", th.accent.face),
                    Mode::Normal => ("NORMAL", th.ink_soft),
                    _ => ("COMMAND", th.info.hi),
                };
                ui.draw_text_in(head, mode, tint, Align::Right);

                let area = ui.alloc(PROMPT_H);
                if editor.show(ui, area) {
                    config.prompt = editor.text();
                    action = Action::Prompt;
                }
            });
            if locked {
                let veil = Rect::from_min_max(body.x, from, body.right(), ui.remaining().y);
                ui.canvas.fill_rect_blend(veil, th.panel, 0.62);
            }
        }
    });

    // The chrome a panel spends on itself: border, title strip, the line under
    // it, and the padding inside.
    let want = used + 24 + 19 + 4;
    chrome.measured = Some(drawing);
    if want != chrome.panel_h {
        chrome.panel_h = want;
        // The panel is drawn at the height measured on the frame before, so the
        // frame that changes that height is a frame drawn at the wrong one. It
        // used to be followed by a right one because there was always a next
        // frame; now that frames are drawn only when something asks for one,
        // the wrong frame is what stays on screen until the pointer happens to
        // move. Which looks exactly like what it is: the panel opens, flickers,
        // and settles a moment later.
        ui.request_repaint();
    }

    // Only where there is a prompt to restore.
    let restore = Rect::new(footer.x, footer.y + 4, 90, 15);
    if chrome.page == Page::Assistant
        && config.assist
        && ui.button_at(restore, "DEFAULT", Tone::Neutral).clicked
    {
        config.prompt = settings::DEFAULT_PROMPT.to_string();
        // The editor is holding the old text; it has to be told, or the next
        // keystroke puts the old prompt straight back.
        chrome.prompt = Some(PromptEditor::new(&config.prompt));
        action = Action::Prompt;
    }
    let close = Rect::new(footer.right() - 70, footer.y + 4, 70, 15);
    if ui.button_at(close, "CLOSE", Tone::Accent).clicked {
        action = Action::Close;
    }
    if !sized {
        ui.canvas.pop_clip();
    }
    action
}

/// Where each column of the model table starts, and where the buttons do.
///
/// From the right: the button column is a fixed size because a button is, the
/// size column fits the widest number anyone will see, and the name and what
/// it is good for share what is left.
fn columns(width: i32) -> [i32; 4] {
    let button = 58;
    let size = 66;
    let name = 84;
    [0, name, width - button - size, width - button]
}

/// One cell of that table, clipped to its column so a long value cannot run
/// into the next one.
fn cell(
    ui: &mut Ui,
    row: Rect,
    cols: &[i32; 4],
    i: usize,
    text: &str,
    ink: pixui::Color,
    at: Align,
) {
    let from = row.x + cols[i];
    let to = row.x + cols[i + 1];
    let inner = Rect::from_min_max(from, row.y, to - 4, row.bottom());
    ui.clipped(inner, |ui| ui.draw_text_in(inner, text, ink, at));
}

/// Weights on disk that the catalogue does not describe.
fn extras(_config: &Settings) -> Vec<String> {
    settings::installed()
        .iter()
        .filter(|p| settings::described(p).is_none())
        .filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(String::from))
        .collect()
}
