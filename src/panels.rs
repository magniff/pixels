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
}

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
        let line_h = font::LINE_H;
        let advance = font::ADVANCE;

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

        // ---- keep the caret on screen -------------------------------------
        let caret = self.buf.cursor;
        let at = rows
            .iter()
            .position(|&(line, from, to)| {
                line == caret.line && caret.col >= from && caret.col <= to
            })
            .unwrap_or(0);
        if at < self.scroll {
            self.scroll = at;
        } else if at >= self.scroll + visible {
            self.scroll = at + 1 - visible;
        }
        if ui.input.wheel != 0.0 && resp.hovered {
            let step = (ui.input.wheel * 3.0).round() as i32;
            self.scroll = (self.scroll as i32 - step).max(0) as usize;
        }
        self.scroll = self
            .scroll
            .min(rows.len().saturating_sub(visible.min(rows.len())));

        // ---- the bar, the same one both note panes carry -------------------
        let mut st = self.bar;
        st.content = rows.len() as i32 * line_h;
        st.viewport = visible as i32 * line_h;
        st.target = self.scroll as f32 * line_h as f32;
        st.shown = st.target;
        ui.scroll_bar(track, "prompt-bar", &mut st);
        self.scroll = (st.target / line_h as f32).round().max(0.0) as usize;
        self.bar = st;

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
                            Rect::new(x, y - 1, (b - a) as i32 * advance, line_h),
                            th.accent.lo,
                        );
                    }
                }
                let slice: String = text.chars().skip(from).take(to - from).collect();
                font::draw_text(ui.canvas, inner.x, y, &slice, th.ink_light);

                if line == caret.line && caret.col >= from && caret.col <= to {
                    let x = inner.x + (caret.col - from) as i32 * advance;
                    if insert {
                        ui.canvas
                            .fill_rect(Rect::new(x - 1, y - 1, 2, line_h), th.positive.face);
                    } else {
                        ui.canvas.fill_rect(
                            Rect::new(x - 1, y - 1, font::GLYPH_W + 2, line_h),
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

const WIDTH: i32 = 320;
/// Room for the prompt: enough of it to read without scrolling.
const PROMPT_H: i32 = 62;

/// What the app is, and which build of it this is.
pub fn about(ui: &mut Ui) -> bool {
    let screen = ui.canvas.bounds();
    ui.canvas
        .fill_rect_blend(screen, pixui::palette::VOID, 0.55);
    let rect = screen.centered(WIDTH, 150);
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
    ui.canvas
        .fill_rect_blend(screen, pixui::palette::VOID, 0.55);

    // As tall as what goes in it, measured from the frame before: the contents
    // change with what is installed and what is being fetched, and dead space
    // under the last control reads as a layout bug. One frame at the guessed
    // height when it first opens, and the right height from then on.
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
            // One entry, and a list to hold it. What matters is that the next
            // setting has somewhere to go.
            let row = ui.alloc(15);
            let entry = ui.button_at(row, "AI ASSISTANT", Tone::Neutral);
            if entry.clicked {
                chrome.page = Page::Assistant;
            }
            ui.space(1);
            let note = ui.alloc(8);
            ui.draw_text_in(
                note,
                "WHICH MODEL, AND WHAT TO TELL IT",
                th.ink_soft,
                Align::Left,
            );
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
                ui.label_colored("MODEL", th.accent.face);

                // ---- the catalogue ------------------------------------------------
                for (i, weights) in CATALOGUE.iter().enumerate() {
                    let path = settings::models_dir().join(weights.file);
                    let here = path.exists();
                    let current = running.as_deref() == Some(weights.file);
                    let row = ui.alloc(9);
                    let (name, button) = row.split_left(row.w - 56);
                    let ink = if current { th.positive.face } else { th.ink };
                    ui.draw_text_in(name, weights.label, ink, Align::Left);
                    ui.draw_text_in(
                        name,
                        &format!("{} MB", weights.megabytes),
                        th.ink_soft,
                        Align::Right,
                    );

                    let at = Rect::new(button.x + 4, row.y - 3, 52, 13);
                    if current {
                        ui.draw_text_in(at, "IN USE", th.positive.face, Align::Center);
                    } else if here {
                        if ui.button_at(at, "USE", Tone::Accent).clicked {
                            action = Action::Use(weights.file.to_string());
                        }
                    } else if chrome.download.is_some() {
                        ui.draw_text_in(at, "-", th.ink_soft, Align::Center);
                    } else if ui.button_at(at, "GET", Tone::Neutral).clicked {
                        action = Action::Fetch(i);
                    }
                    let note = ui.alloc(8);
                    ui.draw_text_in(note, weights.note, th.ink_soft.shade(-0.1), Align::Left);
                }

                // ---- anything else already on disk ---------------------------------
                for name in extras(config) {
                    let row = ui.alloc(9);
                    let (label, button) = row.split_left(row.w - 56);
                    let current = running.as_deref() == Some(name.as_str());
                    let ink = if current { th.positive.face } else { th.ink };
                    ui.draw_text_in(label, &name.to_uppercase(), ink, Align::Left);
                    let at = Rect::new(button.x + 4, row.y - 3, 52, 13);
                    if current {
                        ui.draw_text_in(at, "IN USE", th.positive.face, Align::Center);
                    } else if ui.button_at(at, "USE", Tone::Accent).clicked {
                        action = Action::Use(name.clone());
                    }
                    ui.space(8);
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
    chrome.panel_h = used + 24 + 19 + 4;

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
    action
}

/// Weights on disk that the catalogue does not describe.
fn extras(_config: &Settings) -> Vec<String> {
    settings::installed()
        .iter()
        .filter(|p| settings::described(p).is_none())
        .filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(String::from))
        .collect()
}
