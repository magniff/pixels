//! A markdown note editor with vim keys, drawn one pixel at a time.
//!
//! The whole application lives here. `pixui`, the toolkit it is drawn with, is
//! the only dependency — look at `Cargo.toml`. The line between them is worth
//! knowing when reading this:
//!
//! | In `pixui`                                | Here                            |
//! |-------------------------------------------|---------------------------------|
//! | Text fields, scrolling, splitters, modality | The vim grammar                 |
//! | The bitmap font and the widgets drawn with it | What markdown means           |
//! | Panels, lists, buttons, dialog chrome      | Reading and writing files       |
//!
//! The file dialogs are the sharpest illustration. They are modal, they browse
//! the filesystem, they have a scrolling list and a text field and keyboard
//! navigation — and they are drawn with the same widgets as the rest of the UI.
//! `std::fs` appears in `dialog.rs` and nowhere in `pixui`.

pub mod dialog;
pub mod markdown;
pub mod render;
pub mod shots;
pub mod syntax;
pub mod text;
pub mod vim;

use std::path::{Path, PathBuf};

use pixui::{palette, Align, Color, Key, Point, Rect, Theme, Tone, Ui};

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
    /// The note's display name: its first heading, else its file stem.
    pub fn title(&self) -> String {
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

    /// The file this note lives in, or a placeholder if it has never been saved.
    pub fn filename(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "[NO NAME]".to_string())
    }
}

/// Which of the three panes the keyboard is currently aimed at.
///
/// The editor is modal on its own, so this cannot be the toolkit's focus
/// ring: vim wants every bare key, and a focus ring is driven by Tab. What
/// the toolkit does own is the search field, which takes the keyboard the
/// ordinary way — so `Search` is the one state where this defers to it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pane {
    /// Vim has the keys.
    Editor,
    /// The note list: j/k walk it, Enter or Escape goes back to the editor.
    Notes,
    /// The sidebar's filter field.
    Search,
}

pub struct Notes {
    pub notes: Vec<Note>,
    pub current: usize,
    pub vim: Vim,
    pub dialog: Option<FileDialog>,
    pub notes_dir: PathBuf,
    /// Which pane the keyboard is aimed at.
    pub pane: Pane,
    /// Set on the frame a shortcut moves the keyboard, so the pane taking it
    /// can claim focus once rather than holding it against every click.
    pub pane_grab: bool,
    /// The pane the arrival cue has already been shown for. Focus also moves
    /// by clicking, which no shortcut tells us about, so the flare watches the
    /// value change rather than trusting `pane_grab` alone.
    pub pane_seen: Pane,
    /// How settled the note list's keyboard ring is, 0 to 1.
    pub notes_focus: f32,
    pub status: String,
    /// First visible line in the editor.
    pub scroll: usize,
    /// Live filter for the note list, typed into the sidebar's search box.
    pub filter: String,
    /// Where a pointer drag in the editor started, while it is in progress.
    pub drag_anchor: Option<text::Cursor>,
    /// Which editor tab is showing: 0 the source, 1 the rendering.
    pub editor_tab: usize,
    /// Where the preview was left, so switching tabs comes back to it rather
    /// than to the top.
    pub preview_scroll: pixui::ScrollState,
    /// The view being left behind, which lags `editor_tab` until the
    /// dissolve finishes. Equal to it when nothing is in flight.
    pub tab_shown: usize,
    /// Transition progress, counting 1 down to 0. Zero means settled, and
    /// doubles as the share of the outgoing view still on screen.
    pub tab_anim: f32,
    /// Scratch holding the outgoing view while the incoming one is drawn
    /// over it. Kept across frames so a transition allocates nothing.
    pub fade: pixui::Canvas,
    /// Seconds since the insert caret was last reset, which typing does. The
    /// pulse restarts solid on every keystroke, so the caret is never dim at
    /// the moment you are looking for it.
    pub caret_phase: f32,
    /// A note being renamed in place: which one, and the name so far.
    pub renaming: Option<(usize, String)>,
    /// Set on the frame a rename begins, to move focus into its field.
    focus_rename: bool,
    /// Sidebar width, once the user has dragged it.
    ///
    /// `None` means "follow the canvas", which is the right default and stays
    /// right as the window is resized or the UI zoomed. A dragged divider is a
    /// deliberate choice, so from then on it wins.
    pub sidebar_w: Option<i32>,
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
            pane: Pane::Editor,
            pane_grab: false,
            pane_seen: Pane::Editor,
            notes_focus: 0.0,
            status: "j/k MOVE  i INSERT  /  SEARCH  :w SAVE  :e OPEN  :help".into(),
            scroll: 0,
            filter: String::new(),
            drag_anchor: None,
            editor_tab: 0,
            preview_scroll: pixui::ScrollState::default(),
            tab_shown: 0,
            tab_anim: 0.0,
            fade: pixui::Canvas::new(1, 1),
            caret_phase: 0.0,
            renaming: None,
            focus_rename: false,
            sidebar_w: None,
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
            self.preview_scroll = pixui::ScrollState::default();
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
            "preview" | "p" => {
                self.editor_tab = 1;
                self.status = "PREVIEW".into();
            }
            "source" | "s" | "edit!" => {
                self.editor_tab = 0;
                self.status = "SOURCE".into();
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
                self.status = "MOTIONS hjkl w b e 0 $ gg G f t ; , | EDIT i a o x dd cw \
                     yy p u C-r | OBJECTS diw ciw ci\" di( dip | VISUAL v V C-v then \
                     d y c o | FIND / ? n N * | MOUSE click to place, drag to select, \
                     double-click a note to rename, click a link in the preview | PANES cmd-e EDITOR cmd-n NOTES cmd-s \
                     SEARCH | VIEWS cmd-1 SOURCE cmd-2 PREVIEW \
                     | :w :e :q :qa"
                    .into();
            }
            "" => {}
            other => self.status = format!("NOT AN EDITOR COMMAND: {other}"),
        }
    }

    /// Aim the keyboard at a pane, and let this frame's drawing know to claim
    /// focus for it.
    fn focus_pane(&mut self, pane: Pane) {
        self.pane = pane;
        self.pane_grab = true;
        self.status = match pane {
            Pane::Editor => "EDITOR",
            Pane::Notes => "NOTES - j/k TO WALK, ENTER TO EDIT",
            Pane::Search => "SEARCH NOTES",
        }
        .into();
    }

    /// Indices of the notes the sidebar is showing, in the order it shows
    /// them. Keyboard walking follows the filter — stepping onto a note that
    /// is not on screen would look like the selection vanishing.
    fn shown(&self) -> Vec<usize> {
        let needle = self.filter.trim().to_lowercase();
        (0..self.notes.len())
            .filter(|&i| note_matches(&self.notes[i], &needle))
            .collect()
    }

    /// Move the selection `delta` rows down the visible list, wrapping.
    fn step_note(&mut self, delta: i32) {
        let shown = self.shown();
        if shown.is_empty() {
            return;
        }
        let at = shown.iter().position(|&i| i == self.current).unwrap_or(0) as i32;
        let n = shown.len() as i32;
        self.current = shown[(at + delta).rem_euclid(n) as usize];
        self.scroll = 0;
    }

    /// Follow a link clicked in the preview.
    ///
    /// Three kinds, and the app decides which is which: another note in the
    /// vault, a fragment inside this one, or somewhere outside. Only the last
    /// leaves the program, and it leaves by asking the desktop to open it
    /// rather than by knowing anything about browsers.
    fn follow_link(&mut self, href: &str) {
        let href = href.trim();
        if href.is_empty() {
            return;
        }
        if href.starts_with('#') {
            self.status = format!("ANCHOR {href} - NOT LINKED YET").to_uppercase();
            return;
        }
        if let Some(scheme) = external_scheme(href) {
            match open_externally(href) {
                Ok(()) => self.status = format!("OPENED {scheme} LINK").to_uppercase(),
                Err(e) => self.status = format!("COULD NOT OPEN: {e}").to_uppercase(),
            }
            return;
        }

        // A relative target names a note. Match one already open before
        // touching the disk, so clicking through does not reload a note that
        // has unsaved edits in it.
        let target = href.split(['#', '?']).next().unwrap_or(href);
        let with_ext = if target.ends_with(".md") {
            target.to_string()
        } else {
            format!("{target}.md")
        };
        if let Some(i) = self
            .notes
            .iter()
            .position(|n| n.filename().eq_ignore_ascii_case(&with_ext))
        {
            self.current = i;
            self.scroll = 0;
            self.status = format!("OPENED {}", self.notes[i].filename()).to_uppercase();
            return;
        }
        let path = self.notes_dir.join(&with_ext);
        if path.exists() {
            self.open_path(&path);
        } else {
            self.status = format!("NO SUCH NOTE: {with_ext}").to_uppercase();
        }
    }

    fn close_current(&mut self) {
        if self.note().buffer.dirty {
            self.status = "UNSAVED CHANGES - :w FIRST, OR :q AGAIN".into();
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

    /// Rename a note, moving the file with it.
    ///
    /// A note that has never been saved has no file to move, so it simply
    /// adopts the name and gets written; that is what the user meant.
    pub fn rename_note(&mut self, index: usize, name: &str) {
        let mut name = name.trim().to_string();
        if name.is_empty() {
            self.status = "NAME REQUIRED".into();
            return;
        }
        if !name.contains('.') {
            name.push_str(".md");
        }
        let dest = self.notes_dir.join(&name);
        let current = self.notes[index].path.clone();
        if current.as_deref() == Some(dest.as_path()) {
            return;
        }
        // Refuse to rename onto something that is already there rather than
        // silently replacing a note the user cannot get back.
        if dest.exists() {
            self.status = format!("{name} ALREADY EXISTS").to_uppercase();
            return;
        }
        match current {
            Some(old) => match std::fs::rename(&old, &dest) {
                Ok(()) => {
                    self.notes[index].path = Some(dest);
                    self.status = format!("RENAMED TO {name}").to_uppercase();
                }
                Err(e) => self.status = format!("RENAME FAILED: {e}").to_uppercase(),
            },
            None => {
                let text = self.notes[index].buffer.to_text();
                match std::fs::write(&dest, text) {
                    Ok(()) => {
                        self.notes[index].path = Some(dest);
                        self.notes[index].buffer.mark_saved();
                        self.status = format!("SAVED AS {name}").to_uppercase();
                    }
                    Err(e) => self.status = format!("WRITE FAILED: {e}").to_uppercase(),
                }
            }
        }
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
    app.caret_phase += ui.input.dt;
    // Both the arrival cue and the steady ring are driven from this, so they
    // cannot disagree about where the keyboard is.
    let arrived = app.pane != app.pane_seen || app.pane_grab;
    app.pane_seen = app.pane;
    app.notes_focus = pixui::smooth(
        app.notes_focus,
        f32::from(u8::from(app.pane == Pane::Notes)),
        18.0,
        ui.input.dt,
    );

    // A dialog takes the keyboard for itself while it is open; nothing below
    // it sees a key.
    if !modal {
        // The pane shortcuts run first and everywhere, including from inside
        // the search field, so there is always a way back out to the editor.
        handle_shortcuts(ui, app);

        // Focus can also be lost by clicking elsewhere, which no shortcut
        // told us about. The field holding the keyboard is the truth.
        if app.pane == Pane::Search && !app.pane_grab && !ui.text_input_active() {
            app.pane = Pane::Editor;
        }
        if app.pane_grab && app.pane != Pane::Search {
            ui.clear_focus();
        }

        match app.pane {
            _ if ui.text_input_active() => {}
            Pane::Notes => handle_notes_keys(ui, app),
            _ => handle_keys(ui, app),
        }
    }

    ui.input_blocked(modal, |ui| {
        draw_titlebar(ui, titlebar, app);
        // The divider is a toolkit widget; the app only owns the number.
        let derived = sidebar_width(screen.w);
        let mut width = app.sidebar_w.unwrap_or(derived);
        let before = width;
        let (side, main) =
            ui.split_left(body, "sidebar", &mut width, (120, (screen.w / 2).max(160)));
        if width != before {
            app.sidebar_w = Some(width);
        }
        draw_sidebar(ui, side.inset(5), app, arrived);

        // The two views of the same note: its source, and what it means.
        let pane = main.inset_xy(0, 5);
        let (tabs, content) = pane.split_top(20);
        let strip = Rect::new(tabs.x, tabs.y, 190, 16);
        let views = [
            pixui::Segment::with_icon(pixui::icon::CODE, "SOURCE"),
            pixui::Segment::with_icon(pixui::icon::PAGE, "PREVIEW"),
        ];
        ui.segments_at("view", strip, &views, &mut app.editor_tab);

        // ---- the transition ------------------------------------------
        // A dissolve rather than a fade: both views are drawn, and each pixel
        // takes one or the other according to an ordered dither whose
        // threshold slides across the transition. Blending would need colours
        // between the two, which sixteen tones do not have — so the old view
        // erodes into the new one in a spreading checker instead.
        const TAB_FADE: f32 = 0.22;
        if app.editor_tab != app.tab_shown && app.tab_anim <= 0.0 {
            app.tab_anim = 1.0;
        }

        let editor_arrived = arrived && app.pane == Pane::Editor;
        let pane_inner;
        if app.tab_anim > 0.0 {
            app.tab_anim = (app.tab_anim - ui.input.dt / TAB_FADE).max(0.0);
            // The outgoing view keeps animating so it does not freeze mid
            // dissolve, but takes no input: the pointer has already left it.
            let old = ui.input_blocked(true, |ui| draw_view(ui, content, app, app.tab_shown));
            app.fade.resize(old.w, old.h);
            app.fade.blit_from(ui.canvas, old, Point::new(0, 0));
            pane_inner = draw_view(ui, content, app, app.editor_tab);
            ui.canvas
                .dither_over(pane_inner, &app.fade, Point::new(0, 0), app.tab_anim);
            if app.tab_anim <= 0.0 {
                app.tab_shown = app.editor_tab;
            }
        } else {
            pane_inner = draw_view(ui, content, app, app.tab_shown);
        }
        ui.focus_flare("pane:main", pane_inner, editor_arrived);

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

    // The pulse lasts exactly one frame: everything that wanted it has drawn.
    app.pane_grab = false;
}

/// Shortcuts that work wherever the keyboard is, the search field included.
///
/// Command specifically, not the primary modifier: off macOS the toolkit maps
/// `cmd` onto Control, and Control is already vim's — `Ctrl-r` is `Ctrl-r`
/// everywhere, and `Ctrl-n` walks the sidebar.
fn handle_shortcuts(ui: &mut Ui, app: &mut Notes) {
    let mods = ui.input.mods;
    if !mods.cmd || mods.ctrl {
        return;
    }
    for key in ui.input.keys.clone() {
        match key {
            Key::Char('1') => {
                app.editor_tab = 0;
                app.status = "SOURCE".into();
            }
            Key::Char('2') => {
                app.editor_tab = 1;
                app.status = "PREVIEW".into();
            }
            Key::Char('e') => app.focus_pane(Pane::Editor),
            Key::Char('n') => app.focus_pane(Pane::Notes),
            Key::Char('s') => app.focus_pane(Pane::Search),
            _ => {}
        }
    }
}

/// The note list with the keyboard: walk it, open one, or hand the keys back.
fn handle_notes_keys(ui: &mut Ui, app: &mut Notes) {
    ui.capture_keyboard();
    for key in ui.input.keys.clone() {
        match key {
            Key::Char('j') | Key::Down => app.step_note(1),
            Key::Char('k') | Key::Up => app.step_note(-1),
            Key::Enter | Key::Escape | Key::Char('i') => app.focus_pane(Pane::Editor),
            Key::Char('/') => app.focus_pane(Pane::Search),
            _ => {}
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
        // Already spent on a shortcut.
        if mods.cmd && !mods.ctrl {
            continue;
        }
        // Ctrl-n / Ctrl-p walk the sidebar without leaving the editor.
        if mods.ctrl && matches!(key, Key::Char('n') | Key::Char('p')) {
            app.step_note(if key == Key::Char('n') { 1 } else { -1 });
            continue;
        }
        // Typing restarts the pulse at full, the way a caret you are driving
        // should never be the dim half of a blink.
        app.caret_phase = 0.0;
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
    let note = app.note();
    let badge = format!(
        "{}{}",
        note.filename(),
        if note.buffer.dirty { " *" } else { "" }
    );
    ui.title_bar(rect, "PIXUI NOTES", Some(&badge.to_uppercase()));
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
        Mode::Command | Mode::Search { .. } => th.info,
    };
    let badge = Rect::new(rect.x, rect.y + 1, 52, rect.h - 1);
    ui.canvas.fill_rect(badge, ramp.face);
    ui.draw_text_in(badge, mode.label(), ramp.ink, Align::Center);

    let buf = &app.note().buffer;
    let rest = Rect::new(badge.right() + 6, rect.y, rect.w - badge.w - 12, rect.h);

    if let Some(prefix) = app.vim.prompt_prefix() {
        ui.draw_text_in(
            rest,
            &format!("{prefix}{}_", app.vim.cmdline),
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

/// Whether a note matches the sidebar filter.
///
/// The scheme of an absolute link, if it has one.
///
/// Matched by shape rather than against a list: anything with a scheme is for
/// the desktop to route, and guessing which schemes exist is how a link to
/// something perfectly ordinary ends up treated as a filename.
pub fn external_scheme(href: &str) -> Option<&str> {
    if let Some((scheme, _)) = href.split_once("://") {
        return is_scheme(scheme).then_some(scheme);
    }
    // The schemeless-authority forms, which have no `//` to split on.
    let (scheme, _) = href.split_once(':')?;
    is_scheme(scheme).then_some(scheme)
}

/// Whether a string could be a URI scheme: a letter, then letters, digits,
/// `+`, `-` or `.`. A Windows drive letter is one character, and is not.
fn is_scheme(s: &str) -> bool {
    s.len() > 1
        && s.starts_with(|c: char| c.is_ascii_alphabetic())
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// Hand a URL to the desktop.
///
/// The one place the program leaves its own window. Which command does it is
/// the only per-platform thing here, and the URL goes as an argument rather
/// than through a shell, so nothing in it can be read as a command.
fn open_externally(href: &str) -> std::io::Result<()> {
    let (program, args): (&str, &[&str]) = if cfg!(target_os = "macos") {
        ("open", &[])
    } else if cfg!(target_os = "windows") {
        ("cmd", &["/C", "start", ""])
    } else {
        ("xdg-open", &[])
    };
    std::process::Command::new(program)
        .args(args)
        .arg(href)
        .spawn()
        .map(|_| ())
}

/// Title, filename and body all count: searching a note vault for a word you
/// half-remember is the common case, and it is rarely in the title.
pub fn note_matches(note: &Note, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    if note.title().to_lowercase().contains(needle)
        || note.filename().to_lowercase().contains(needle)
    {
        return true;
    }
    note.buffer
        .lines()
        .iter()
        .any(|line| line.to_lowercase().contains(needle))
}

fn draw_sidebar(ui: &mut Ui, rect: Rect, app: &mut Notes, arrived: bool) {
    let th = *ui.theme;
    let inner = ui.panel(rect, "NOTES");
    let (search, rest) = inner.split_top(17);
    let (list, footer) = rest.split_bottom(17);

    // ---- filter box --------------------------------------------------
    // The list below simply reads the current string every frame, so it
    // updates as you type with nothing to wire up.
    let mut filter = std::mem::take(&mut app.filter);
    let field = Rect::new(search.x, search.y, search.w, 15);
    let grab = app.pane_grab && app.pane == Pane::Search;
    ui.search_field_grab_at(field, "filter", &mut filter, "SEARCH NOTES", grab);
    app.filter = filter;

    let needle = app.filter.trim().to_lowercase();
    let shown: Vec<usize> = (0..app.notes.len())
        .filter(|&i| note_matches(&app.notes[i], &needle))
        .collect();

    // Fit the preview text to whatever width the sidebar ended up, allowing for
    // the scrollbar gutter and the row's own padding.
    let cols = ((list.w - 20) / pixui::font::ADVANCE).max(8) as usize;

    let mut select = None;
    let mut begin_rename = None;
    let mut commit_rename = None;
    let mut cancel_rename = false;

    ui.scroll_area(list, "notes", |ui| {
        if shown.is_empty() {
            ui.label_dim("  NO MATCHES");
            return;
        }
        for &i in &shown {
            // Copy what the row needs before drawing, so nothing holds a
            // borrow of the note list while the rename field mutates state.
            let title = app.notes[i].title();
            let dirty = app.notes[i].buffer.dirty;
            let preview = markdown::preview(app.notes[i].buffer.lines(), 2, cols);
            let renaming = matches!(app.renaming, Some((idx, _)) if idx == i);

            let selected = i == app.current;
            let h = 13 + preview.len() as i32 * 8;
            let row = ui.alloc(h);
            let id = ui.id(&format!("note{i}"));
            let resp = ui.interact(id, row);
            if resp.clicked {
                select = Some(i);
            }
            if resp.double_clicked {
                begin_rename = Some(i);
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
                // When the list itself has the keyboard, say so on the row the
                // keys will move — otherwise j and k appear to do nothing.
                if app.notes_focus > 0.03 {
                    // Fades in with the pane rather than snapping on, so
                    // arriving here reads as one movement and not two events.
                    let ring = th.accent.face.lerp(th.accent.ink, app.notes_focus);
                    ui.canvas
                        .stroke_rect_dashed(row, ring, 2, 2, (ui.input.time * 14.0) as i32);
                }
            }

            let ink = if selected { th.accent.ink } else { th.ink };
            let title_at = Rect::new(row.x + 4, row.y + 1, row.w - 8, 9);

            if renaming {
                // The title is replaced in place by a field, so the row keeps
                // its shape and the rename reads as editing *this* note.
                let field = Rect::new(row.x + 2, row.y + 1, row.w - 4, 11);
                let mut name = match app.renaming.take() {
                    Some((_, n)) => n,
                    None => String::new(),
                };
                let grab = app.focus_rename;
                app.focus_rename = false;
                ui.text_field_grab_at(field, "rename", &mut name, "", grab);
                if ui.input.key_pressed(pixui::Key::Enter) {
                    commit_rename = Some((i, name));
                } else if ui.input.key_pressed(pixui::Key::Escape) {
                    cancel_rename = true;
                } else {
                    app.renaming = Some((i, name));
                }
            } else {
                pixui::font::draw_text_styled(
                    ui.canvas,
                    title_at.x,
                    title_at.y + 1,
                    &title,
                    ink,
                    true,
                );
                if dirty {
                    ui.canvas
                        .fill_rect(Rect::new(row.right() - 5, row.y + 3, 3, 3), th.danger.face);
                }
            }

            for (n, line) in preview.iter().enumerate() {
                let y = row.y + 11 + n as i32 * 8;
                let dim = if selected {
                    th.accent.ink.lerp(th.accent.face, 0.4)
                } else {
                    th.ink_soft
                };
                pixui::font::draw_text(ui.canvas, title_at.x, y, line, dim);
            }
        }
    });

    if let Some(i) = begin_rename {
        // Seed with the current file name, or the derived title for a note
        // that has never been saved.
        let seed = app.notes[i]
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| format!("{}.md", slug(&app.notes[i].title())));
        app.renaming = Some((i, seed));
        app.focus_rename = true;
    }
    if let Some((i, name)) = commit_rename {
        app.rename_note(i, &name);
        app.renaming = None;
    }
    if cancel_rename {
        app.renaming = None;
        app.status = "RENAME CANCELLED".into();
    }

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
                app.filter.clear();
                app.status = "NEW NOTE".into();
            }
            let cell = ui.alloc_rest();
            if ui.button_at(cell, "OPEN", Tone::Accent).clicked {
                app.dialog = Some(FileDialog::new(DialogKind::Open, &app.notes_dir, ""));
            }
        });
    });

    // The cue lands on the thing that took the keys, not on the drawer that
    // happens to contain it. Drawn last so nothing paints over the ring.
    match app.pane {
        Pane::Search => ui.focus_flare("pane:search", field, arrived),
        Pane::Notes => ui.focus_flare("pane:list", list, arrived),
        Pane::Editor => {}
    }
}

// ------------------------------------------------------------------- preview

/// Returns the area inside the pane's frame, which is what a transition
/// sweeps over — the frame itself should stay put.
/// Draw whichever view `which` names, and return the area it filled — the
/// region a transition dissolves over.
fn draw_view(ui: &mut Ui, rect: Rect, app: &mut Notes, which: usize) -> Rect {
    if which == 0 {
        draw_editor(ui, rect, app)
    } else {
        draw_preview(ui, rect, app)
    }
}

fn draw_preview(ui: &mut Ui, rect: Rect, app: &mut Notes) -> Rect {
    let th = *ui.theme;
    let area = Rect::new(rect.x, rect.y, rect.w - 5, rect.h);
    ui.canvas.box_chamfer(area, th.well, th.well_border, 2);

    let inner = area.inset(6);
    // Parsed every frame. A note is a few kilobytes and the parse is a linear
    // scan, so caching it would buy nothing and could go stale.
    let blocks = markdown::parse(app.note().buffer.lines());
    // The scroll area keeps a gutter for its bar; the document gets the rest.
    let width = inner.w - 12;
    // The app holds the scroll position, so it survives the tab being hidden.
    let mut scroll = app.preview_scroll;
    let mut clicked = None;
    ui.scroll_area_with(inner, "preview", &mut scroll, |ui| {
        clicked = render::draw_document(ui, &blocks, width);
    });
    app.preview_scroll = scroll;
    if let Some(href) = clicked {
        app.follow_link(&href);
    }
    area.inset(1)
}

// -------------------------------------------------------------------- editor

/// Map a point in the editor to a position in the buffer.
///
/// The inverse of the drawing loop: walk logical lines from the scroll
/// position, counting the visual rows each one wraps into, until the row under
/// the pointer is reached. Wrapping is derived from the raw text, so this and
/// the renderer cannot disagree about where a character is.
fn position_at(
    buf: &Buffer,
    scroll: usize,
    cols: usize,
    origin: pixui::Point,
    p: pixui::Point,
) -> text::Cursor {
    let target_row = ((p.y - origin.y) / pixui::font::LINE_H).max(0) as usize;
    let mut row = 0usize;
    let mut line = scroll;
    while line < buf.line_count() {
        let ranges = markdown::wrap_ranges(buf.line(line), cols);
        if row + ranges.len() > target_row {
            let (from, to) = ranges[target_row - row];
            // Floor rather than round: clicking a character should land *on*
            // it, since the caret in normal mode is a block over a character
            // rather than a bar between two.
            let rel = ((p.x - origin.x) / pixui::font::ADVANCE).max(0) as usize;
            return text::Cursor::new(line, (from + rel).min(to));
        }
        row += ranges.len();
        line += 1;
    }
    // Below the last line: the end of the buffer.
    let last = buf.line_count().saturating_sub(1);
    text::Cursor::new(last, buf.line_len(last))
}

/// Returns the area inside the pane's frame; see [`draw_preview`].
fn draw_editor(ui: &mut Ui, rect: Rect, app: &mut Notes) -> Rect {
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

    // ---- pointer --------------------------------------------------------
    // Done before the caret-follow below, so a click sets the caret and the
    // scrolling then keeps it visible, rather than the two fighting.
    let origin = pixui::Point::new(inner.x + GUTTER, inner.y);
    let editor_id = ui.id("editor");
    let resp = ui.interact(editor_id, inner);
    if resp.hovered {
        ui.request_cursor(pixui::Cursor::Text);
    }
    if resp.held {
        let i = app.current.min(app.notes.len() - 1);
        let at = position_at(
            &app.notes[i].buffer,
            app.scroll,
            cols,
            origin,
            ui.input.mouse,
        );
        if ui.input.mouse_pressed {
            app.drag_anchor = Some(at);
            app.vim.click_at(&mut app.notes[i].buffer, at);
        } else if let Some(anchor) = app.drag_anchor {
            app.vim.drag_to(&mut app.notes[i].buffer, anchor, at);
            // Dragging past an edge scrolls, or a selection could never reach
            // past what happens to be on screen.
            if ui.input.mouse.y < inner.y {
                app.scroll = app.scroll.saturating_sub(1);
            } else if ui.input.mouse.y > inner.bottom() {
                app.scroll += 1;
            }
        }
    }
    if ui.input.mouse_released {
        app.drag_anchor = None;
    }

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
    let search = app.vim.search_pattern().map(str::to_owned);
    let search = search.as_deref();
    let insert = app.vim.mode == Mode::Insert;

    // Fenced code gets the real syntax highlighter rather than one flat colour.
    // Computed for the whole buffer, not the viewport, because a grammar is
    // stateful — where a string ends depends on where it began, which may be
    // above the fold. It is memoised on the text, so this costs a hash per
    // block once the block stops changing.
    let code = syntax::code_regions(buf.lines());

    let mut last_line_drawn = app.scroll;
    ui.clipped(inner, |ui| {
        let mut row = 0usize;
        let mut line_no = app.scroll;

        while row < visible && line_no < total {
            let text = buf.line(line_no);
            let spans = match code.get(line_no).and_then(Option::as_ref) {
                Some(highlighted) => highlighted.clone(),
                None => markdown::highlight(text, false),
            };
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

                // ---- search hits -------------------------------------
                // Drawn beneath the selection, so a hit that is also selected
                // still reads as selected rather than as two highlights
                // arguing with each other.
                if let Some(pattern) = search {
                    for (ms, me) in vim::matches_in(text, pattern) {
                        let a = ms.max(from);
                        let b = me.min(to.max(from));
                        if b > a {
                            let x0 = text_x + (a - from) as i32 * advance - 1;
                            ui.canvas.fill_rect(
                                Rect::new(x0, y - 1, (b - a) as i32 * advance, line_h),
                                th.well.lerp(palette::YELLOW, 0.30),
                            );
                        }
                    }
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
                        // The caret pulses rather than blinking on and off.
                        // Dither density is the only half-brightness sixteen
                        // colours have, so it dissolves and re-forms instead of
                        // fading — and never drops below a quarter coverage,
                        // which keeps it findable at every point in the cycle.
                        let cycle = app.caret_phase * 1.9;
                        let wave = (cycle * std::f32::consts::TAU).cos() * 0.5 + 0.5;
                        let bar = Rect::new(cx - 1, y - 1, 2, line_h);
                        ui.canvas
                            .dither_fill(bar, th.positive.face, 0.28 + 0.72 * wave);
                        // The ends stay solid whatever the dither is doing, so
                        // the caret keeps a definite top and bottom.
                        ui.canvas.hline(bar.x, bar.y, 2, th.positive.hi);
                        ui.canvas.hline(bar.x, bar.bottom() - 1, 2, th.positive.hi);
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

    area.inset(1)
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
        Tok::Strike => th.ink_soft,
        Tok::Image => th.info.hi,
        Tok::CodePlain => th.ink_light,
        Tok::CodeKeyword => palette::ACCENT,
        Tok::CodeType => palette::TEAL,
        Tok::CodeFunction => palette::TEAL_HI,
        Tok::CodeString => palette::GREEN,
        Tok::CodeNumber => palette::YELLOW,
        Tok::CodeComment => th.ink_soft,
        Tok::CodePunct => th.ink_light.shade(-0.30),
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

    // A note that exercises every piece of markdown the renderer understands,
    // and names the ones it does not, so both views can be judged side by side.
    let showcase = "\
# Markdown showcase

Everything below is written in the source tab and drawn in the preview tab.
Switch between them with the tabs above, or with `:source` and `:preview`.

## Headings

Six levels are parsed. The first two take a rule under them; the rest are
told apart by weight and colour alone.

### Third level
#### Fourth level
##### Fifth level
###### Sixth level

## Inline

Text can be **bold**, *italic*, `monospaced`, ~~struck out~~, or a
[link to somewhere](https://example.com). Emphasis can sit **inside a
longer sentence** without upsetting the wrapping, and `code spans` get a
slab behind them so they read as code rather than as differently coloured
prose.

An image cannot be drawn, so its alt text stands in for it:
![a picture of a cat](cat.png)

## Paragraphs

A paragraph is not a line. These three source lines
are one paragraph, and they reflow to whatever
width the pane happens to be.

A blank line starts a new one.

## Lists

- A bullet at the top level
- Another one, with **emphasis** and `code` inside it
  - A nested bullet, one indent deeper
  - And its sibling
- Back out again

1. Ordered items keep their numbers
2. Even when they are not sequential
7. As here

- [ ] An unchecked task
- [x] A finished one
- [ ] Tasks and bullets can share a list

## Quotes

> A block quote gets a bar down its left side.
> Consecutive lines belong to the same quote.

## Code

Fenced code keeps its own slab, and is never re-wrapped: a line break
inserted into code is a lie about what the code says.

```rust
fn main() {
    let note = \"every note is just a file on disk\";
    println!(\"{note}\");
}
```

Markdown inside a fence is left alone:

```
# not a heading
- not a list
**not bold**
```

## Tables

Columns size to their content, and alignment comes from the separator row.

| left | centred | right |
| :--- | :-----: | ----: |
| one | two | three |
| a much longer cell | x | 42 |
| short | y | 7 |

## Rules

Three or more dashes make a horizontal rule:

---

## Not supported

These are left as plain text rather than pretending:

- Setext headings, the kind underlined with `===`
- Reference-style links and footnotes
- Nested block quotes
- Inline HTML
- Hard line breaks from two trailing spaces
- Any script the 5x7 ASCII font cannot draw
";

    let _ = std::fs::write(dir.join("welcome.md"), welcome);
    let _ = std::fs::write(dir.join("markdown-showcase.md"), showcase);
    let _ = std::fs::write(dir.join("vim-keys.md"), vim);
    let _ = std::fs::write(dir.join("ideas.md"), ideas);
}
