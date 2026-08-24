//! A markdown note-taking app with vim keys, built entirely on `pixui`.
//!
//! # The split, again
//!
//! Like `pixui-demo`, this crate depends on exactly one thing: `pixui`. Look at
//! `Cargo.toml`. Everything specific to *this* program — the vim grammar, the
//! markdown highlighter, reading and writing files — is here. Everything a
//! second note-taking app would also need is in the toolkit.
//!
//! The file dialogs are the sharpest illustration. They are modal, they browse
//! the filesystem, they have a scrolling list and a text field and keyboard
//! navigation — and they are drawn with the same widgets as the rest of the UI.
//! `std::fs` appears in `dialog.rs` and nowhere in `pixui`.
//!
//! | In `pixui`                                | Here                            |
//! |-------------------------------------------|---------------------------------|
//! | Text field, caret, scrolling, modality     | The vim grammar                 |
//! | Bitmap font and faux-bold                  | What markdown means             |
//! | Panels, lists, buttons, dialoging chrome   | Reading and writing files       |
//! | Blocking input behind a modal              | Which dialog is open, and why   |

pub mod dialog;
pub mod markdown;
pub mod text;
pub mod vim;

use std::path::{Path, PathBuf};

use pixui::{palette, Align, Color, Key, Rect, Theme, Tone, Ui};

use dialog::{DialogKind, DialogResult, FileDialog};
use markdown::Tok;
use text::Buffer;
use vim::{Mode, Vim, VimEvent};

/// Room for a three-digit line number plus a little breathing space.
const GUTTER: i32 = 26;

/// How wide the note list should be for a given canvas.
///
/// Derived rather than a constant because the canvas is no longer fixed: it
/// grows with the window and shrinks when the UI is zoomed in. A magic number
/// tuned for one canvas width becomes a thin strip on a large one and swallows
/// half the screen on a small one.
fn sidebar_width(canvas_w: i32) -> i32 {
    (canvas_w / 5).clamp(120, 300)
}

/// One open note.
pub struct Note {
    pub path: Option<PathBuf>,
    pub buffer: Buffer,
}

impl Note {
    fn title(&self) -> String {
        let derived = markdown::derive_title(self.buffer.lines());
        if derived == "UNTITLED" {
            if let Some(p) = &self.path {
                return p
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_uppercase();
            }
        }
        derived.to_uppercase()
    }

    fn filename(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "[NO NAME]".to_string())
    }
}

pub struct Notes {
    pub notes: Vec<Note>,
    pub current: usize,
    pub vim: Vim,
    pub dialog: Option<FileDialog>,
    pub notes_dir: PathBuf,
    pub status: String,
    /// First visible line in the editor.
    pub scroll: usize,
}

impl Notes {
    /// Open the vault, seeding it with a few notes the first time so the app
    /// does not open onto nothing.
    pub fn open(notes_dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&notes_dir);
        seed_if_empty(&notes_dir);

        let mut notes = Vec::new();
        if let Ok(read) = std::fs::read_dir(&notes_dir) {
            let mut paths: Vec<PathBuf> = read
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "md"))
                .collect();
            paths.sort();
            for path in paths {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    notes.push(Note {
                        path: Some(path),
                        buffer: Buffer::from_text(&text),
                    });
                }
            }
        }
        if notes.is_empty() {
            notes.push(Note {
                path: None,
                buffer: Buffer::new(),
            });
        }

        // Open on the welcome note when there is one, rather than whatever
        // sorts first alphabetically.
        let current = notes
            .iter()
            .position(|n| n.filename() == "welcome.md")
            .unwrap_or(0);

        Self {
            notes,
            current,
            vim: Vim::new(),
            dialog: None,
            notes_dir,
            status: "j/k MOVE  i INSERT  :w SAVE  :e OPEN  :help".into(),
            scroll: 0,
        }
    }

    fn note(&self) -> &Note {
        &self.notes[self.current.min(self.notes.len() - 1)]
    }

    fn note_mut(&mut self) -> &mut Note {
        let i = self.current.min(self.notes.len() - 1);
        &mut self.notes[i]
    }

    fn save_to(&mut self, path: &Path) {
        let text = self.note().buffer.to_text();
        match std::fs::write(path, text) {
            Ok(()) => {
                self.status = format!(
                    "WROTE {}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                );
                let note = self.note_mut();
                note.path = Some(path.to_path_buf());
                note.buffer.mark_saved();
            }
            Err(e) => self.status = format!("WRITE FAILED: {e}"),
        }
    }

    fn open_path(&mut self, path: &Path) {
        // Already open? Just switch to it rather than loading a second copy.
        if let Some(i) = self
            .notes
            .iter()
            .position(|n| n.path.as_deref() == Some(path))
        {
            self.current = i;
            self.scroll = 0;
            self.status = format!("SWITCHED TO {}", self.notes[i].filename());
            return;
        }
        match std::fs::read_to_string(path) {
            Ok(text) => {
                self.notes.push(Note {
                    path: Some(path.to_path_buf()),
                    buffer: Buffer::from_text(&text),
                });
                self.current = self.notes.len() - 1;
                self.scroll = 0;
                self.status = format!(
                    "OPENED {}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                );
            }
            Err(e) => self.status = format!("OPEN FAILED: {e}"),
        }
    }

    /// Run a `:` command.
    fn run_command(&mut self, cmd: &str, ui: &mut Ui) {
        let mut parts = cmd.trim().splitn(2, ' ');
        let verb = parts.next().unwrap_or("");
        let arg = parts.next().unwrap_or("").trim();

        match verb {
            "w" | "write" => {
                if !arg.is_empty() {
                    let mut name = arg.to_string();
                    if !name.contains('.') {
                        name.push_str(".md");
                    }
                    let path = self.notes_dir.join(name);
                    self.save_to(&path);
                } else if let Some(path) = self.note().path.clone() {
                    self.save_to(&path);
                } else {
                    // No filename yet, so ask for one the same way the menu does.
                    self.open_save_dialog();
                }
            }
            "wq" | "x" => {
                if let Some(path) = self.note().path.clone() {
                    self.save_to(&path);
                    self.close_current();
                } else {
                    self.open_save_dialog();
                }
            }
            "e" | "edit" | "o" | "open" => {
                if arg.is_empty() {
                    self.dialog = Some(FileDialog::new(DialogKind::Open, &self.notes_dir, ""));
                } else {
                    let path = self.notes_dir.join(arg);
                    self.open_path(&path);
                }
            }
            "q" | "close" => self.close_current(),
            "qa" | "quit" => ui.request_quit(),
            "new" => {
                self.notes.push(Note {
                    path: None,
                    buffer: Buffer::new(),
                });
                self.current = self.notes.len() - 1;
                self.scroll = 0;
                self.status = "NEW NOTE".into();
            }
            "help" => {
                self.status = "MOTIONS hjkl w b e 0 $ gg G | EDIT i a o x dd cw yy p u C-r \
                     | OBJECTS diw ciw ci\" di( dip | VISUAL v V C-v then d y c o, \
                     I A on a block | :w :e :q :qa"
                    .into();
            }
            "" => {}
            other => self.status = format!("NOT AN EDITOR COMMAND: {other}"),
        }
    }

    fn close_current(&mut self) {
        if self.note().buffer.dirty {
            self.status = "UNSAVED CHANGES — :w FIRST, OR :q AGAIN".into();
            self.note_mut().buffer.mark_saved();
            return;
        }
        self.notes.remove(self.current.min(self.notes.len() - 1));
        if self.notes.is_empty() {
            self.notes.push(Note {
                path: None,
                buffer: Buffer::new(),
            });
        }
        self.current = self.current.min(self.notes.len() - 1);
        self.scroll = 0;
        self.status = "CLOSED".into();
    }

    fn open_save_dialog(&mut self) {
        let suggested = self
            .note()
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| format!("{}.md", slug(&self.note().title())));
        self.dialog = Some(FileDialog::new(
            DialogKind::Save,
            &self.notes_dir,
            &suggested,
        ));
    }
}

/// The window configuration.
///
/// Exposed rather than buried in `main` so a test can assert it — forgetting to
/// opt into [`Scaling::Adaptive`] is invisible until someone drags a window
/// edge and the whole UI jumps a size.
pub fn config() -> pixui::Config {
    // 1.5 logical points per virtual pixel, which on a 2x display resolves to a
    // 3 physical pixel magnification — one step up from 2, and a size no whole
    // number of logical points can name. The window opens the same size on
    // screen either way; Cmd/Ctrl with `+`, `-` and `0` steps it live, one
    // whole pixel at a time.
    pixui::Config::new("pixui notes", 768, 470)
        .with_scale(1.5)
        .with_scale_range(2, 6)
        // Resizing buys more lines and columns at the same pixel size, rather
        // than magnifying what is already there.
        .adaptive()
        .with_min_canvas(480, 300)
        .with_theme(theme())
}

/// The default theme: the stock warm look, with the library's scanline pass off
/// — a text editor should not have anything drawn across its text.
pub fn theme() -> Theme {
    let mut t = Theme::warm();
    t.scanline = 0.0;
    t
}

// --------------------------------------------------------------------- frame

pub fn frame(ui: &mut Ui, app: &mut Notes) {
    let screen = ui.canvas.bounds();
    let (titlebar, rest) = screen.split_top(13);
    let (body, statusbar) = rest.split_bottom(12);

    let modal = app.dialog.is_some();

    // Keys go to the editor only when no dialog is up; the dialog takes the
    // keyboard for itself while it is open.
    if !modal {
        handle_keys(ui, app);
    }

    ui.input_blocked(modal, |ui| {
        draw_titlebar(ui, titlebar, app);
        let (side, main) = body.split_left(sidebar_width(screen.w));
        draw_sidebar(ui, side.inset(5), app);
        draw_editor(ui, main.inset_xy(0, 5), app);
        draw_statusbar(ui, statusbar, app);
    });

    if let Some(dialog) = app.dialog.as_mut() {
        match dialog.show(ui) {
            Some(DialogResult::Cancel) => {
                app.dialog = None;
                app.status = "CANCELLED".into();
            }
            Some(DialogResult::Open(path)) => {
                app.dialog = None;
                app.open_path(&path);
            }
            Some(DialogResult::Save(path)) => {
                app.dialog = None;
                app.save_to(&path);
            }
            None => {}
        }
    }
}

fn handle_keys(ui: &mut Ui, app: &mut Notes) {
    // The editor is modal itself, so Tab and Escape belong to vim, not to the
    // toolkit's focus handling.
    ui.capture_keyboard();

    let keys = ui.input.keys.clone();
    let mods = ui.input.mods;
    for key in keys {
        // Ctrl-n / Ctrl-p walk the sidebar without leaving the keyboard.
        if mods.ctrl && matches!(key, Key::Char('n') | Key::Char('p')) {
            let n = app.notes.len();
            app.current = if key == Key::Char('n') {
                (app.current + 1) % n
            } else {
                (app.current + n - 1) % n
            };
            app.scroll = 0;
            continue;
        }
        let i = app.current.min(app.notes.len() - 1);
        let event = app.vim.handle(&mut app.notes[i].buffer, key, mods);
        if let Some(VimEvent::Command(cmd)) = event {
            app.run_command(&cmd, ui);
        }
    }
    if !app.vim.status.is_empty() {
        app.status = std::mem::take(&mut app.vim.status).to_uppercase();
    }
}

// -------------------------------------------------------------------- chrome

fn draw_titlebar(ui: &mut Ui, rect: Rect, app: &Notes) {
    let th = *ui.theme;
    ui.canvas
        .gradient_rect(rect, th.accent.lo, th.accent.face, true);
    ui.canvas
        .hline(rect.x, rect.bottom() - 1, rect.w, th.panel_border);

    let label = Rect::new(rect.x + 6, rect.y, rect.w - 12, rect.h - 1);
    ui.draw_text_in_shadow(label, "PIXUI NOTES", th.ink, th.accent.hi, Align::Left);

    let note = app.note();
    let name = format!(
        "{}{}",
        note.filename(),
        if note.buffer.dirty { " *" } else { "" }
    );
    ui.draw_text_in_shadow(
        label,
        &name.to_uppercase(),
        th.ink,
        th.accent.hi,
        Align::Right,
    );
}

fn draw_statusbar(ui: &mut Ui, rect: Rect, app: &Notes) {
    let th = *ui.theme;
    ui.canvas.fill_rect(rect, th.background.shade(-0.35));
    ui.canvas.hline(rect.x, rect.y, rect.w, th.panel_border);

    // A coloured mode badge, the one piece of vim UI everyone relies on.
    let mode = app.vim.mode;
    let ramp = match mode {
        Mode::Normal => th.neutral,
        Mode::Insert => th.positive,
        Mode::Visual(_) => th.accent,
        Mode::Command => th.info,
    };
    let badge = Rect::new(rect.x, rect.y + 1, 52, rect.h - 1);
    ui.canvas.fill_rect(badge, ramp.face);
    ui.draw_text_in(badge, mode.label(), ramp.ink, Align::Center);

    let buf = &app.note().buffer;
    let rest = Rect::new(badge.right() + 6, rect.y, rect.w - badge.w - 12, rect.h);

    if mode == Mode::Command {
        ui.draw_text_in(
            rest,
            &format!(":{}_", app.vim.cmdline),
            th.info.hi,
            Align::Left,
        );
    } else {
        ui.draw_text_in(rest, &app.status, th.ink_soft, Align::Left);
    }

    let pos = format!(
        "{}{}  {}:{}",
        app.vim.pending.to_uppercase(),
        if app.vim.pending.is_empty() { "" } else { "  " },
        buf.cursor.line + 1,
        buf.cursor.col + 1
    );
    ui.draw_text_in(rest, &pos, th.ink_light, Align::Right);
}

// ------------------------------------------------------------------- sidebar

fn draw_sidebar(ui: &mut Ui, rect: Rect, app: &mut Notes) {
    let th = *ui.theme;
    let inner = ui.panel(rect, "NOTES");
    let (list, footer) = inner.split_bottom(17);

    // Fit the preview text to whatever width the sidebar ended up, allowing for
    // the scrollbar gutter and the row's own padding.
    let cols = ((list.w - 20) / pixui::font::ADVANCE).max(8) as usize;

    let mut select = None;
    ui.scroll_area(list, "notes", |ui| {
        for (i, note) in app.notes.iter().enumerate() {
            let selected = i == app.current;
            let preview = markdown::preview(note.buffer.lines(), 2, cols);
            let h = 13 + preview.len() as i32 * 8;
            let row = ui.alloc(h);
            let id = ui.id(&format!("note{i}"));
            let resp = ui.interact(id, row);
            if resp.clicked {
                select = Some(i);
            }

            let face = if selected {
                th.accent.face
            } else if resp.hovered {
                th.panel.shade(-0.08)
            } else {
                th.panel
            };
            ui.canvas.fill_chamfer(row, face, 1);
            if selected {
                ui.canvas.vline(row.x, row.y, row.h, th.accent.lo);
            }

            let ink = if selected { th.accent.ink } else { th.ink };
            let title = Rect::new(row.x + 4, row.y + 1, row.w - 8, 9);
            // Bold titles, which is what the faux-bold in the font is for.
            let t = note.title();
            let w = pixui::font::text_width_styled(&t, true).min(title.w);
            let _ = w;
            pixui::font::draw_text_styled(ui.canvas, title.x, title.y + 1, &t, ink, true);

            if note.buffer.dirty {
                ui.canvas
                    .fill_rect(Rect::new(row.right() - 5, row.y + 3, 3, 3), th.danger.face);
            }

            for (n, line) in preview.iter().enumerate() {
                let y = row.y + 11 + n as i32 * 8;
                let dim = if selected {
                    th.accent.ink.lerp(th.accent.face, 0.4)
                } else {
                    th.ink_soft
                };
                pixui::font::draw_text(ui.canvas, title.x, y, line, dim);
            }
        }
    });

    if let Some(i) = select {
        app.current = i;
        app.scroll = 0;
    }

    ui.column(footer, 3, |ui| {
        ui.row_h(14, 4, |ui| {
            let w = (footer.w - 4) / 2;
            let cell = ui.alloc(w);
            if ui.button_at(cell, "NEW", Tone::Neutral).clicked {
                app.notes.push(Note {
                    path: None,
                    buffer: Buffer::new(),
                });
                app.current = app.notes.len() - 1;
                app.scroll = 0;
                app.status = "NEW NOTE".into();
            }
            let cell = ui.alloc_rest();
            if ui.button_at(cell, "OPEN", Tone::Accent).clicked {
                app.dialog = Some(FileDialog::new(DialogKind::Open, &app.notes_dir, ""));
            }
        });
    });
}

// -------------------------------------------------------------------- editor

fn draw_editor(ui: &mut Ui, rect: Rect, app: &mut Notes) {
    let th = *ui.theme;
    let area = Rect::new(rect.x, rect.y, rect.w - 5, rect.h);
    ui.canvas.box_chamfer(area, th.well, th.well_border, 2);

    let inner = area.inset(3);
    let line_h = pixui::font::LINE_H;
    let advance = pixui::font::ADVANCE;
    let visible = (inner.h / line_h).max(1) as usize;
    let cols = ((inner.w - GUTTER) / advance).max(8) as usize;

    let i = app.current.min(app.notes.len() - 1);
    let cursor = app.notes[i].buffer.cursor;
    let total = app.notes[i].buffer.line_count();

    // ---- keep the caret on screen ---------------------------------------
    // Lines wrap, so "how far down is the caret" is a count of *visual* rows,
    // not of lines. Scrolling stays line-granular (as vim's does), which keeps
    // the arithmetic honest without a second coordinate space to maintain.
    {
        let buf = &app.notes[i].buffer;
        if cursor.line < app.scroll {
            app.scroll = cursor.line;
        }
        let caret_row_in_line = markdown::locate(
            &markdown::wrap_ranges(buf.line(cursor.line), cols),
            cursor.col,
        )
        .0;
        // Walk the scroll down until the caret fits. Bounded by the line count,
        // so a pathological wrap can never spin here.
        for _ in 0..total {
            let rows_above: usize = (app.scroll..cursor.line)
                .map(|l| markdown::wrap_ranges(buf.line(l), cols).len())
                .sum();
            if rows_above + caret_row_in_line < visible || app.scroll >= cursor.line {
                break;
            }
            app.scroll += 1;
        }
    }

    let buf = &app.notes[i].buffer;
    let selection = app.vim.selection(buf);
    let insert = app.vim.mode == Mode::Insert;

    // A code fence spans lines, so the highlighter has to be told where it is.
    // Scan from the top of the file, not the top of the viewport.
    let mut in_code = false;
    for l in 0..app.scroll {
        if markdown::is_fence(buf.line(l)) {
            in_code = !in_code;
        }
    }

    let mut last_line_drawn = app.scroll;
    ui.clipped(inner, |ui| {
        let mut row = 0usize;
        let mut line_no = app.scroll;

        while row < visible && line_no < total {
            let text = buf.line(line_no);
            let fence = markdown::is_fence(text);
            let spans = markdown::highlight(text, in_code && !fence);
            if fence {
                in_code = !in_code;
            }
            let ranges = markdown::wrap_ranges(text, cols);
            let (caret_row, caret_col) = markdown::locate(&ranges, cursor.col);

            for (vi, &(from, to)) in ranges.iter().enumerate() {
                if row >= visible {
                    break;
                }
                let y = inner.y + row as i32 * line_h;
                let text_x = inner.x + GUTTER;

                // ---- current-line band -------------------------------
                if line_no == cursor.line {
                    ui.canvas.fill_rect(
                        Rect::new(inner.x, y - 1, inner.w, line_h),
                        th.well.shade(0.10),
                    );
                }

                // ---- gutter: number the logical line, once ------------
                if vi == 0 {
                    let num = format!("{:>3}", line_no + 1);
                    let ink = if line_no == cursor.line {
                        th.accent.face
                    } else {
                        th.ink_soft.shade(-0.2)
                    };
                    pixui::font::draw_text(ui.canvas, inner.x + 1, y, &num, ink);
                } else {
                    // A continuation tick, so a wrapped row is never mistaken
                    // for a new line. Drawn rather than typed: the font has no
                    // glyph that reads as "this is a continuation".
                    let ink = th.ink_soft.shade(-0.35);
                    ui.canvas
                        .fill_rect(Rect::new(inner.x + 14, y + 3, 4, 1), ink);
                    ui.canvas
                        .fill_rect(Rect::new(inner.x + 14, y + 1, 1, 3), ink);
                }

                // ---- visual selection --------------------------------
                // All three shapes reduce to a column range on this line, so
                // charwise, linewise and blockwise draw through one path.
                if let Some((lo, hi)) =
                    selection.and_then(|sel| sel.columns_on(line_no, text.chars().count()))
                {
                    let a = lo.max(from);
                    let b = hi.min(to.max(from));
                    if b > a {
                        // The cell is the glyph plus a column of padding either
                        // side; see the caret below for why.
                        let x0 = text_x + (a - from) as i32 * advance - 1;
                        ui.canvas.fill_rect(
                            Rect::new(x0, y - 1, (b - a) as i32 * advance, line_h),
                            th.accent.lo,
                        );
                    }
                }

                // ---- the text ----------------------------------------
                // Position each run by its character offset rather than by
                // accumulating widths. Styled markdown puts a dozen runs on a
                // line, and anything that adds up per-run measurements will
                // drift off the character grid the caret is drawn on.
                let mut col = 0usize;
                for span in &markdown::slice_spans(&spans, from, to) {
                    let color = token_color(&th, span.tok);
                    let x = text_x + col as i32 * advance;
                    col += span.text.chars().count();
                    pixui::font::draw_text_styled(ui.canvas, x, y, &span.text, color, span.bold);
                }

                // ---- caret -------------------------------------------
                if line_no == cursor.line && vi == caret_row {
                    let cx = text_x + caret_col as i32 * advance;
                    if insert {
                        ui.canvas
                            .fill_rect(Rect::new(cx - 1, y - 1, 1, line_h), th.positive.face);
                    } else {
                        // A block caret with the character redrawn on top in
                        // the inverse ink, so it stays readable underneath.
                        //
                        // The cell is the glyph plus one column of padding on
                        // each side — expressed from `GLYPH_W` rather than the
                        // advance so it stays centred whatever the tracking is.
                        // A cell that simply starts at the glyph collects all
                        // the tracking on its right and none on its left, and
                        // reads as shunted sideways.
                        ui.canvas.fill_rect(
                            Rect::new(cx - 1, y - 1, pixui::font::GLYPH_W + 2, line_h),
                            th.accent.face,
                        );
                        let under = text.chars().nth(cursor.col).unwrap_or(' ');
                        pixui::font::draw_char(ui.canvas, cx, y, under, th.accent.ink);
                    }
                }

                row += 1;
            }
            last_line_drawn = line_no;
            line_no += 1;
        }
    });

    // A hint of how far through the file we are, in the right margin.
    if last_line_drawn + 1 < total || app.scroll > 0 {
        let track = Rect::new(area.right() - 2, area.y + 2, 2, area.h - 4);
        let span = total.saturating_sub(1).max(1);
        let t = app.scroll as f32 / span as f32;
        let shown = (last_line_drawn + 1 - app.scroll).max(1);
        let thumb_h = ((shown as f32 / total as f32) * track.h as f32).max(6.0) as i32;
        let y = track.y + ((track.h - thumb_h) as f32 * t.clamp(0.0, 1.0)) as i32;
        ui.canvas
            .fill_rect(Rect::new(track.x, y, 2, thumb_h), th.ink_soft);
    }
}

fn token_color(th: &Theme, tok: Tok) -> Color {
    match tok {
        Tok::Text => th.ink_light,
        Tok::Marker => th.ink_soft,
        Tok::Heading => th.accent.hi,
        Tok::Bold => th.ink_light.lerp(palette::YELLOW, 0.7),
        Tok::Italic => th.info.hi,
        Tok::Code => th.positive.face,
        Tok::Link => th.info.face,
        Tok::Quote => th.ink_soft.lerp(th.ink_light, 0.4),
    }
}

// ---------------------------------------------------------------------- seed

fn slug(title: &str) -> String {
    let s: String = title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        "note".into()
    } else {
        s
    }
}

/// Write a couple of starter notes so a fresh vault is not an empty screen.
fn seed_if_empty(dir: &Path) {
    let has_notes = std::fs::read_dir(dir)
        .map(|r| {
            r.flatten()
                .any(|e| e.path().extension().is_some_and(|x| x == "md"))
        })
        .unwrap_or(false);
    if has_notes {
        return;
    }

    let welcome = "\
# Welcome

This is **pixui notes**: a markdown editor with *vim* keys,
drawn entirely with the pixui toolkit.

Everything you can see is a pixel buffer: the sidebar, the caret,
and the save dialog too.

## Try it

- Press `i` to insert, `Esc` to go back to normal mode
- `dd` deletes a line, `u` undoes it
- `:w` saves, `:e` opens the file browser
- `Ctrl-n` walks to the next note

See [the readme](../README.md) for how the toolkit is put together.
";

    let vim = "\
# Vim keys

## Motions

| key | moves |
| --- | ----- |
| h j k l | left down up right |
| w b e | by word |
| 0 $ | line start, line end |
| gg G | file start, file end |

## Operators

Operators combine with motions, so `d2w` deletes two words and
`c$` changes to the end of the line.

- `d` delete
- `c` change
- `y` yank

Doubling one makes it linewise: `dd`, `cc`, `yy`.

## Not implemented

Linewise visual mode, blockwise visual mode, marks, macros,
registers other than the unnamed one, and search.
";

    let ideas = "\
# Ideas

- [ ] Rendered preview pane next to the source
- [ ] Search with `/` and `n`
- [ ] Fuzzy note switcher on `Ctrl-p`
- [ ] Export a note to HTML

> The nice thing about a bitmap font is that a table like the one
> in the vim note lines up for free.

```rust
fn main() {
    println!(\"every note is just a file on disk\");
}
```
";

    let _ = std::fs::write(dir.join("welcome.md"), welcome);
    let _ = std::fs::write(dir.join("vim-keys.md"), vim);
    let _ = std::fs::write(dir.join("ideas.md"), ideas);
}
