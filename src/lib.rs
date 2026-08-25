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

pub mod assist;
pub mod dialog;
pub mod finder;
pub mod diff;
pub mod fetch;
pub mod indent;
pub mod llm;
pub mod markdown;
pub mod panels;
pub mod render;
pub mod settings;
pub mod shots;
pub mod showcase;
pub mod syntax;
pub mod text;
pub mod vim;

use std::path::{Path, PathBuf};

use pixui::{Align, Color, Key, Point, Rect, Theme, Tone, Ui};

use dialog::{DialogKind, DialogResult, FileDialog};
use markdown::Tok;
use text::Buffer;
use vim::{Mode, Vim, VimEvent};

/// Room for a three-digit line number plus a little breathing space.
///
/// Measured in the face being read in, or a wider one writes its numbers into
/// the first column of the text.
pub fn gutter() -> i32 {
    3 * pixui::font::advance() + 5
}

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

/// The selection travelling between two notes.
///
/// It does not slide: the highlight lets go of the note it was on and takes
/// hold of the new one, which is what choosing something feels like. The old
/// one shrinks away quickly and the new one springs open a moment later, so
/// the two halves read as one movement rather than as a cross-fade.
pub struct Pick {
    /// The row being left, while there is still any of it to draw.
    pub from: Option<usize>,
    /// How much of the old highlight is left, 1 down to 0.
    pub leave: f32,
    /// The new one arriving, which overshoots a little on the way in.
    pub enter: pixui::Spring,
    /// The selection this last saw, so a change can be noticed.
    seen: usize,
}

impl Pick {
    fn at(current: usize) -> Self {
        // Softer and slower than the button press this borrows from: a press
        // is an answer to something you did, and this is a thing moving across
        // the screen, which wants long enough to be seen moving.
        let mut enter = pixui::Spring::new(520.0, 26.0);
        // Already arrived: the first frame is not an animation of anything.
        enter.snap(1.0);
        Self {
            from: None,
            leave: 0.0,
            enter,
            seen: current,
        }
    }

    /// How much of row `i`'s highlight to draw, where 1.0 is all of it and
    /// more than that is the overshoot on the way in.
    pub fn grow(&self, i: usize, current: usize) -> f32 {
        if i == current {
            self.enter.pos.max(0.0)
        } else if self.from == Some(i) {
            self.leave
        } else {
            0.0
        }
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
    /// Open while somebody is looking for a line of the note they are in.
    pub finder: Option<finder::Finder>,
    pub notes_dir: PathBuf,
    /// Which pane the keyboard is aimed at.
    pub pane: Pane,
    /// Set on the frame a shortcut moves the keyboard, so the pane taking it
    /// can claim focus once rather than holding it against every click.
    pub pane_grab: bool,
    /// The pane the arrival cue has already been shown for. Focus also moves
    /// by clicking, which no shortcut tells us about, so the cue watches this
    /// change rather than trusting `pane_grab` alone.
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
    /// Set by the key that asks for the assistant, and spent by the drawing,
    /// which is where the selection's position on screen is known.
    pub assist_wanted: bool,
    /// How far the text was lifted last frame to keep the assistant's block on
    /// screen. Kept so a click lands on the line it looked like it was on.
    pub assist_lift: i32,
    /// The selection moving from one note to another.
    pub pick: Pick,
    /// The assistant's panel, while one is open.
    pub assist: Option<assist::Assist>,
    /// The model on its thread, whichever one this build has.
    pub helper: llm::Assistant,
    /// What the assistant is and what it is told, kept between runs.
    pub settings: settings::Settings,
    /// The menu, the panels behind it, and anything being downloaded.
    pub chrome: panels::Chrome,
    /// A source line the preview should bring into view on the next draw,
    /// which is when the document's heights are known.
    pub preview_reveal: Option<usize>,
    /// Whether a `g` is waiting for its second half in the preview.
    pub preview_g: bool,
    /// The source view's scrollbar. The scrolling itself lives in `scroll`,
    /// counted in lines; this is only what the bar needs to remember between
    /// frames, which is where a drag took hold of the thumb.
    pub editor_scroll: pixui::ScrollState,
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
        install_reference(&notes_dir);

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

        let config = settings::Settings::load();

        Self {
            notes,
            current,
            vim: Vim::new(),
            dialog: None,
            finder: None,
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
            editor_scroll: pixui::ScrollState::default(),
            preview_g: false,
            preview_reveal: None,
            pick: Pick::at(current),
            assist: None,
            assist_wanted: false,
            assist_lift: 0,
            helper: llm::Assistant::spawn(assistant(&config)),
            settings: config,
            chrome: panels::Chrome::default(),
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
                     double-click a note to rename, click a link in the preview | SEARCH BOX \
                     down TO THE FIRST MATCH, esc CLEARS THEN LEAVES | PANES cmd-e EDITOR cmd-n NOTES cmd-s \
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

    /// Step out of the search box into the list it is filtering, landing on
    /// the first match. Stays put when there is nothing to land on, since
    /// moving to an empty list only takes the keyboard somewhere useless.
    /// Act on whatever the settings panel decided.
    ///
    /// A change of weights or of prompt means a new assistant, which means
    /// loading the model again — so it is done when the panel closes rather
    /// than on every keystroke in the prompt.
    fn apply_settings(&mut self, ui: &mut Ui, action: panels::Action) {
        match action {
            panels::Action::Font(name) => {
                self.settings.font = name;
                ui.request_theme(theme_for(&self.settings));
            }
            panels::Action::Scheme(name) => {
                self.settings.scheme = name;
                // Worn immediately: a colour scheme you have to close a panel
                // to see is a colour scheme you cannot choose between. Written
                // down when the panel closes, along with everything else.
                ui.request_theme(theme_for(&self.settings));
            }
            panels::Action::None | panels::Action::Prompt => {}
            panels::Action::Context => self.rebuild_assistant(),
            panels::Action::Use(file) => {
                self.settings.model = Some(file);
                self.rebuild_assistant();
                let _ = self.settings.save();
            }
            panels::Action::Fetch(i) => {
                let weights = &settings::CATALOGUE[i];
                self.chrome.notice.clear();
                match fetch::Download::start(weights, &settings::models_dir()) {
                    Ok(down) => self.chrome.download = Some(down),
                    Err(why) => self.chrome.notice = why.to_uppercase(),
                }
            }
            panels::Action::Cancel => {
                if let Some(mut down) = self.chrome.download.take() {
                    down.cancel();
                    self.chrome.notice = "STOPPED - IT WILL RESUME WHERE IT LEFT OFF".into();
                }
            }
            panels::Action::Close => {
                self.chrome.panel = None;
                self.chrome.prompt = None;
                let was = self.chrome.opened_with.take();
                if was.as_ref() != Some(&self.settings) {
                    if let Err(why) = self.settings.save() {
                        self.status = format!("COULD NOT SAVE SETTINGS: {why}").to_uppercase();
                    }
                }
                // Only what the assistant is made of is worth rebuilding it
                // for. A colour scheme is not, and rebuilding means loading a
                // model again.
                let assistant_changed = was.is_some_and(|before| {
                    before.assist != self.settings.assist
                        || before.model != self.settings.model
                        || before.prompt != self.settings.prompt
                });
                if assistant_changed {
                    self.rebuild_assistant();
                }
            }
        }
    }

    /// Write the settings down and start an assistant that matches them.
    fn rebuild_assistant(&mut self) {
        self.helper = llm::Assistant::spawn(assistant(&self.settings));
        self.status = if self.settings.assist {
            format!("ASSISTANT: {}", self.helper.name())
        } else {
            "ASSISTANT OFF".into()
        };
    }

    /// Ask a download how it is getting on, once a frame.
    fn watch_download(&mut self) {
        let Some(down) = self.chrome.download.as_mut() else {
            return;
        };
        match down.poll() {
            None => {}
            Some(Ok(path)) => {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
                    .to_string();
                self.chrome.download = None;
                self.chrome.notice.clear();
                // Fetched to be used, so it is used.
                self.settings.model = Some(name);
                self.rebuild_assistant();
            }
            Some(Err(why)) => {
                self.chrome.download = None;
                self.chrome.notice = why.to_uppercase();
            }
        }
    }

    /// Say whether anything here will look different next frame.
    ///
    /// The toolkit notices its own springs and blends; these are the app's own,
    /// and a frame nobody asks for is a frame that is not drawn.
    fn ask_for_frames(&self, ui: &mut Ui) {
        let moving = self.tab_anim > 0.0
            || self.pick.leave > 0.0
            || self.pick.enter.vel.abs() > 0.002
            || (self.notes_focus > 0.01 && self.notes_focus < 0.99)
            // A caret that pulses is a change every frame, but only while
            // there is a caret pulsing.
            || (self.vim.mode == Mode::Insert && self.pane == Pane::Editor)
            // And anything waiting on something that is not the keyboard: a
            // model thinking, a download arriving. Neither sends an event.
            || self.helper.busy()
            || self.chrome.download.is_some();
        if moving {
            ui.request_repaint();
        }
    }

    /// Move the selection animation on by a frame.
    fn step_pick(&mut self, dt: f32) {
        if self.pick.seen != self.current {
            self.pick.from = Some(self.pick.seen);
            self.pick.leave = 1.0;
            self.pick.enter.snap(0.0);
            self.pick.seen = self.current;
        }
        self.pick.leave = pixui::smooth(self.pick.leave, 0.0, 22.0, dt);
        // The new highlight waits for the old one to be mostly gone. Started
        // together they read as a dissolve, and choosing something is not a
        // dissolve.
        let target = f32::from(u8::from(self.pick.leave < 0.25));
        self.pick.enter.step(target, dt);
        if self.pick.leave < 0.02 {
            self.pick.leave = 0.0;
            self.pick.from = None;
        }
    }

    /// Note that the assistant was asked for, if there is anything to ask about.
    ///
    /// Only a flag: where the block opens depends on where the selection lands
    /// on screen, and that is not known until the editor has drawn.
    fn want_assist(&mut self) {
        if self.settings.assist && self.vim.visual_kind().is_some() {
            self.assist_wanted = true;
        }
    }

    /// Open the assistant on whatever is selected.
    ///
    /// The range is taken now and kept: by the time an answer comes back the
    /// selection may be anywhere, and the answer is about what was asked. Where
    /// the block opens follows from the range, so there is nothing else to
    /// remember.
    fn open_assist(&mut self) {
        let i = self.current.min(self.notes.len() - 1);
        let buf = &self.notes[i].buffer;
        let Some(sel) = self.vim.selection(buf) else {
            return;
        };
        let Some((from, to)) = selected_range(sel, buf) else {
            return;
        };
        let source = buf.text_between(from, to);
        if source.trim().is_empty() {
            return;
        }
        let open = assist::Assist::new(from, to, source);
        self.status = open.headline();
        self.assist = Some(open);
    }

    /// Put a suggestion in place of the range it was asked about.
    ///
    /// Typed in rather than spliced, because the answer may be several lines
    /// where the question was one, and the buffer's own editing operations are
    /// the only things that know how a line becomes two.
    fn apply_suggestion(&mut self, open: &assist::Assist, text: &str) {
        let i = self.current.min(self.notes.len() - 1);
        let buf = &mut self.notes[i].buffer;
        buf.checkpoint();
        buf.delete_between(open.from, open.to);
        for c in text.chars() {
            if c == '\n' {
                buf.insert_newline();
            } else {
                buf.insert_char(c);
            }
        }
        buf.clamp_cursor(false);
        // The selection it was about is gone, so the mode that was showing it
        // goes too.
        self.vim.mode = Mode::Normal;
    }

    fn enter_list(&mut self) {
        let Some(&first) = self.shown().first() else {
            return;
        };
        self.current = first;
        self.scroll = 0;
        self.focus_pane(Pane::Notes);
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

    /// Open a note, from something that found it by name.
    ///
    /// The keyboard goes to the editor rather than to the sidebar: somebody who
    /// asked for a note by typing its name is on their way into it, not on
    /// their way to a list of it.
    fn go_to_note(&mut self, i: usize) {
        if i >= self.notes.len() {
            return;
        }
        self.current = i;
        self.scroll = 0;
        let title = self.notes[i].title();
        self.focus_pane(Pane::Editor);
        self.status = title;
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
    theme_for(&settings::Settings::load())
}

/// The theme these settings ask for.
///
/// The scanline is turned off whichever scheme it is: it belongs to the
/// toolkit's demo, and a note editor is a thing to read from for an hour.
pub fn theme_for(config: &settings::Settings) -> Theme {
    // The face first: a theme's metrics are sized from the line height, so the
    // font has to be in place before the theme is built from it.
    if let Some(i) = pixui::font::face_named(&config.font) {
        pixui::font::use_face(i);
    }
    let mut t = pixui::scheme_named(&config.scheme).unwrap_or_else(Theme::warm);
    t.scanline = 0.0;
    t
}

// --------------------------------------------------------------------- frame

/// The assistant this build has: the chosen local model when one is compiled in
/// and its weights are on disk, and the rehearsal stub otherwise.
///
/// Chosen here rather than at the call site so that the interface never has to
/// know which one it is talking to.
pub fn assistant(config: &settings::Settings) -> Box<dyn llm::Backend> {
    #[cfg(feature = "llm")]
    if let Some(path) = config.model_path() {
        return Box::new(llm::local::Local::new(
            path,
            config.prompt.clone(),
            config.context,
        ));
    }
    let _ = config;
    Box::new(llm::Rehearsal)
}

pub fn frame(ui: &mut Ui, app: &mut Notes) {
    let screen = ui.canvas.bounds();
    // Every band is a line of text with room around it, so they follow the
    // face rather than the numbers that happened to suit a five-by-seven one.
    let line = pixui::font::line_h();
    let (titlebar, rest) = screen.split_top(line + 4);
    let (body, statusbar) = rest.split_bottom(line + 3);

    // An answer, if the worker has one. Collected before anything draws, so a
    // suggestion appears on the frame it arrives rather than the one after.
    // How the one in flight is getting on, so the block can say more than that
    // it is busy.
    if app.helper.busy() {
        let p = app.helper.progress();
        if let Some(open) = app.assist.as_mut() {
            if open.waiting() && open.progress != p {
                open.progress = p;
                app.status = open.headline();
            }
        }
    }
    if let Some(reply) = app.helper.poll() {
        let mut said = None;
        if let Some(open) = app.assist.as_mut() {
            open.answered(reply);
            said = Some(open.headline());
        }
        if let Some(said) = said {
            app.status = said;
        }
    }

    // Two different kinds of "something else has the keys".
    //
    // The dialog is a layer over the whole screen: nothing underneath it should
    // take a click either, so the whole page is drawn with input blocked. The
    // assistant's block is *in* the text — the pointer has to keep working all
    // around it, and its own layer settles who gets a click that lands on it.
    // Both, though, take the keyboard away from vim entirely, because a model
    // rewriting the note behind you while you type at it is not a feature.
    let modal = app.dialog.is_some() || app.chrome.panel.is_some() || app.finder.is_some();
    let typing_elsewhere = modal || app.assist.is_some();
    app.caret_phase += ui.input.dt;
    app.step_pick(ui.input.dt);
    app.ask_for_frames(ui);

    // A dialog, or the assistant's block, takes the keyboard for itself while
    // it is open; nothing below it sees a key.
    if !typing_elsewhere {
        // The pane shortcuts run first and everywhere, including from inside
        // the search field, so there is always a way back out to the editor.
        handle_shortcuts(ui, app);

        // Focus can also be lost by clicking elsewhere, which no shortcut
        // told us about. The field holding the keyboard is the truth.
        if app.pane == Pane::Search && !app.pane_grab && !ui.text_input_active() {
            app.pane = Pane::Editor;
        }
        match app.pane {
            // The field has the keyboard, but the filter and the list it
            // filters are one gesture: Down walks out of the box into the
            // results, and Escape throws the search away.
            Pane::Search => handle_search_keys(ui, app),
            _ if ui.text_input_active() => {}
            Pane::Notes => handle_notes_keys(ui, app),
            _ if app.editor_tab == 1 => handle_preview_keys(ui, app),
            _ => handle_keys(ui, app),
        }

        // After everything that can move the keyboard, so a pane taken during
        // dispatch still gets the field released for it.
        if app.pane_grab && app.pane != Pane::Search {
            ui.clear_focus();
        }
    }

    let arrived = app.pane != app.pane_seen || app.pane_grab;
    app.pane_seen = app.pane;
    app.notes_focus = pixui::smooth(
        app.notes_focus,
        f32::from(u8::from(app.pane == Pane::Notes)),
        9.0,
        ui.input.dt,
    );

    let mut menu_at = Point::new(0, 0);
    ui.input_blocked(modal, |ui| {
        menu_at = draw_titlebar(ui, titlebar, app);
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
        let (tabs, content) = pane.split_top(line + 11);
        let strip = Rect::new(tabs.x, tabs.y, 190, line + 7);
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
        ui.focus_flare("pane:main", pane_inner, ui.theme.background, editor_arrived);

        draw_statusbar(ui, statusbar, app);
    });

    if app.finder.is_some() {
        // Taken out while it draws, because what it decides is about the notes
        // it is drawn over, and it cannot hold a borrow of them.
        let mut finder = app.finder.take().expect("checked above");
        let library: Vec<finder::Candidate> = app
            .notes
            .iter()
            .map(|note| finder::Candidate {
                title: note.title(),
                file: note.filename(),
            })
            .collect();
        match finder.show(ui, &library) {
            finder::Found::None => app.finder = Some(finder),
            finder::Found::Close => app.status = "EDITOR".into(),
            finder::Found::Open(i) => app.go_to_note(i),
        }
    }

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

    // ---- the menu, over whatever it covers -----------------------------
    if app.chrome.menu_open {
        let entries = [
            pixui::Segment::with_icon(pixui::icon::SLIDERS, "SETTINGS"),
            pixui::Segment::with_icon(pixui::icon::INFO, "ABOUT"),
        ];
        let pick = ui.menu_items(menu_at, &entries);
        match pick.chosen {
            Some(0) => {
                app.chrome.menu_open = false;
                app.chrome.opened_with = Some(app.settings.clone());
                app.chrome.prompt = None;
                app.chrome.page = panels::Page::Index;
                app.chrome.panel = Some(panels::Panel::Settings);
                // Opened fresh: whatever height is on file was measured for a
                // panel that is no longer on screen.
                app.chrome.measured = None;
            }
            Some(1) => {
                app.chrome.menu_open = false;
                app.chrome.panel = Some(panels::Panel::About);
            }
            _ if pick.dismissed => app.chrome.menu_open = false,
            _ => {}
        }
    }

    // ---- and the panels behind it ---------------------------------------
    match app.chrome.panel {
        Some(panels::Panel::About) => {
            if panels::about(ui) {
                app.chrome.panel = None;
            }
        }
        Some(panels::Panel::Settings) => {
            let mut config = std::mem::take(&mut app.settings);
            let action = panels::settings(ui, &mut config, &mut app.chrome);
            app.settings = config;
            app.apply_settings(ui, action);
        }
        None => {}
    }
    app.watch_download();

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
            // The one key for the assistant: it calls it on a selection, and
            // keeps what it suggests. Off macOS the toolkit maps `cmd` onto
            // Control, which is where this lives on those keyboards anyway.
            Key::Enter => app.want_assist(),
            // The note list answers "which note?"; this answers "where in it?".
            Key::Char('p') => app.finder = Some(finder::Finder::new()),
            Key::Char('e') => app.focus_pane(Pane::Editor),
            Key::Char('n') => app.focus_pane(Pane::Notes),
            Key::Char('s') => app.focus_pane(Pane::Search),
            _ => {}
        }
    }
}

/// The search box with the keyboard. The field itself takes the typing; this
/// is only the two keys that are about the list rather than about the text.
fn handle_search_keys(ui: &mut Ui, app: &mut Notes) {
    // Escape means this, not the toolkit's "drop focus" — otherwise the box
    // would empty and be abandoned in the same keystroke, and there would be
    // no way to clear a search you are still working on.
    ui.capture_keyboard();
    // Down steps into the results; Enter says the same thing with the key the
    // hand is already on after typing. Both do nothing when nothing matched —
    // there is no first result to step onto, and dropping the keyboard into an
    // empty list would strand it.
    if ui.input.key_pressed(Key::Down) || ui.input.key_pressed(Key::Enter) {
        app.enter_list();
    } else if ui.input.key_pressed(Key::Escape) {
        // Clear first, leave second. A search you are still reading is worth
        // one Escape to undo and another to walk away from.
        if app.filter.is_empty() {
            app.focus_pane(Pane::Editor);
        } else {
            app.filter.clear();
            app.status = "SEARCH CLEARED".into();
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

/// The preview with the keyboard: the half of the grammar that moves a page.
///
/// Every other vim key is about a caret, and there is no caret here — pressing
/// them would edit a note while showing you a rendering that says nothing
/// about it. So the reading view takes the motions that scroll and drops the
/// rest, keeping the two that are about the file rather than about the view:
/// the command line, and walking the vault.
fn handle_preview_keys(ui: &mut Ui, app: &mut Notes) {
    ui.capture_keyboard();
    let line = pixui::font::line_h() as f32;
    // A page is what is actually on screen, measured by the scroll area on the
    // frame before — the same number it pages by when its own track is clicked.
    let page = app.preview_scroll.viewport as f32;
    let mods = ui.input.mods;
    for key in ui.input.keys.clone() {
        // Already spent on a shortcut.
        if mods.cmd && !mods.ctrl {
            continue;
        }
        // Two things belong to the note rather than to the view of it, and both
        // are handed to vim whole: the command line, and the search. A search
        // lands on a source line, and the preview scrolls to the block that
        // line was parsed into.
        let searching = matches!(app.vim.mode, Mode::Search { .. });
        let steps = matches!(key, Key::Char('n') | Key::Char('N'));
        let opens = matches!(key, Key::Char(':') | Key::Char('/') | Key::Char('?'));
        if searching || steps || opens || app.vim.mode == Mode::Command {
            let i = app.current.min(app.notes.len() - 1);
            let event = app.vim.handle(&mut app.notes[i].buffer, key, mods);
            if let Some(VimEvent::Command(cmd)) = event {
                app.run_command(&cmd, ui);
            }
            // A finished search has put the cursor on its hit. Not while the
            // pattern is still being typed, and not for the command line,
            // which moves nothing and should not move the page either.
            let landed = matches!(key, Key::Enter) && searching;
            if landed || steps {
                app.preview_reveal = Some(app.notes[i].buffer.cursor.line);
            }
            continue;
        }
        if mods.ctrl && matches!(key, Key::Char('n') | Key::Char('p')) {
            app.step_note(if key == Key::Char('n') { 1 } else { -1 });
            continue;
        }
        // `gg` is the one motion here that needs a keystroke of memory, and it
        // is forgotten by anything that is not the second `g`.
        let doubled = std::mem::take(&mut app.preview_g);
        let by = match key {
            Key::Char('j') | Key::Down | Key::Enter => line,
            Key::Char('k') | Key::Up => -line,
            Key::Char('e') if mods.ctrl => line,
            Key::Char('y') if mods.ctrl => -line,
            Key::Char('d') if mods.ctrl => page / 2.0,
            Key::Char('u') if mods.ctrl => -page / 2.0,
            Key::Char('f') if mods.ctrl => page,
            Key::Char('b') if mods.ctrl => -page,
            Key::Space => page,
            Key::Char('g') if !doubled => {
                app.preview_g = true;
                continue;
            }
            Key::Char('g') | Key::Home => {
                app.preview_scroll.target = 0.0;
                continue;
            }
            Key::Char('G') | Key::End => {
                app.preview_scroll.target = app.preview_scroll.max_offset();
                continue;
            }
            _ => continue,
        };
        app.preview_scroll.target += by;
    }
    if !app.vim.status.is_empty() {
        app.status = std::mem::take(&mut app.vim.status).to_uppercase();
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
        // The same call, spelled with the other modifier: the block accepts a
        // suggestion on either, and the key that opens it should not be fussier
        // than the key that finishes with it.
        if mods.ctrl && key == Key::Enter {
            app.want_assist();
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

/// The strip along the top: the application's menu on the left, and which note
/// is open on the right.
///
/// Returns where a dropdown from the menu would hang, which the caller needs
/// after everything else has drawn — a menu that opens under the pane it covers
/// is not a menu.
fn draw_titlebar(ui: &mut Ui, rect: Rect, app: &mut Notes) -> Point {
    let note = app.note();
    let badge = format!(
        "{}{}",
        note.filename(),
        if note.buffer.dirty { " *" } else { "" }
    );
    // The strip with no title of its own: the menu stands where the name was.
    ui.title_bar(rect, "", Some(&badge.to_uppercase()));

    // Room for the name and the caret that says it opens onto something.
    let w = pixui::font::advance_width("PIXELS") + 22;
    let at = Rect::new(rect.x + 12, rect.y + 1, w, rect.h - 3);
    if ui.menu_title(at, "PIXELS", app.chrome.menu_open).clicked {
        app.chrome.menu_open = !app.chrome.menu_open;
    }
    Point::new(at.x, rect.bottom())
}

fn draw_statusbar(ui: &mut Ui, rect: Rect, app: &Notes) {
    let th = *ui.theme;
    // A well, like the editor and the fields: a band derived from the
    // background is a band that goes grey in a light scheme and black in a
    // dark one, and the ink on it has to guess which.
    ui.canvas.fill_rect(rect, th.well);
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
    let cols = ((list.w - 20) / pixui::font::advance()).max(8) as usize;

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
            // A title, then a line of preview each: sized from the face, or a
            // taller one writes the second row over the first.
            let line = pixui::font::line_h();
            let h = line + 4 + preview.len() as i32 * line;
            let row = ui.alloc(h);
            let id = ui.id(&format!("note{i}"));
            let resp = ui.interact(id, row);
            if resp.clicked {
                select = Some(i);
            }
            if resp.double_clicked {
                begin_rename = Some(i);
            }

            let face = if resp.hovered && !selected {
                th.panel.shade(-0.08)
            } else {
                th.panel
            };
            ui.canvas.fill_chamfer(row, face, 1);

            // The highlight lets go of one row and takes hold of another. The
            // movement is in the colour; the size only breathes with it, a few
            // pixels in and a pixel back out past full on the spring. A patch
            // that grew from nothing would be a different effect and a louder
            // one, and this is a list you walk with `j`.
            let grow = app.pick.grow(i, app.current);
            let held = grow.clamp(0.0, 1.0);
            let patch = if grow > 0.01 {
                let shrink = ((1.0 - held) * 3.0).round() as i32;
                let bulge = ((grow - 1.0).max(0.0) * 6.0).round() as i32;
                Some(row.inset(shrink - bulge))
            } else {
                None
            };
            if let Some(patch) = patch {
                ui.canvas
                    .fill_chamfer(patch, th.panel.lerp(th.accent.face, held), 1);
                ui.canvas
                    .vline(row.x, patch.y, patch.h, th.panel.lerp(th.accent.lo, held));
                // When the list itself has the keyboard, say so on the row the
                // keys will move — otherwise j and k appear to do nothing.
                if selected && app.notes_focus > 0.03 {
                    // Fades in with the pane rather than snapping on, so
                    // arriving here reads as one movement and not two events.
                    let ring = th.accent.face.lerp(th.accent.ink, app.notes_focus);
                    // Still dashes: this one stays for as long as the list has
                    // the keyboard, and marching it would keep the whole window
                    // redrawing for all of that time.
                    ui.canvas.stroke_rect_dashed(patch, ring, 2, 2, 0);
                }
            }

            let title_at = Rect::new(row.x + 4, row.y + 1, row.w - 8, line);

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
                // The ink travels with the fill under it. The patch covers the
                // words the whole way, so this is a blend rather than the two
                // clipped passes a patch growing from nothing would need.
                let ink = th.ink.lerp(th.accent.ink, held);
                let dim = th
                    .ink_soft
                    .lerp(th.accent.ink.lerp(th.accent.face, 0.4), held);
                pixui::font::draw_text_styled(
                    ui.canvas,
                    title_at.x,
                    title_at.y + 1,
                    &title,
                    ink,
                    true,
                );
                for (n, text) in preview.iter().enumerate() {
                    let y = row.y + line + 2 + n as i32 * line;
                    pixui::font::draw_text(ui.canvas, title_at.x, y, text, dim);
                }
                if dirty {
                    ui.canvas
                        .fill_rect(Rect::new(row.right() - 5, row.y + 3, 3, 3), th.danger.face);
                }
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

    // Only the list needs one. A text field already lights its own border
    // when it takes the keyboard, so a ring around the search box would be a
    // second ring outside the first — which is what a doubled outline looks
    // like, not what an arrival looks like.
    if app.pane == Pane::Notes {
        // Around the whole drawer, not just the rows: the pane the keyboard
        // is aimed at is the notes widget, and ringing a part of it points at
        // something that is not a thing you can move to. Drawn last so nothing
        // paints over it, and over the background the panel sits on.
        ui.focus_flare("pane:notes", rect, th.background, arrived);
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

    // The same frame inset the source view uses, so the two views put their
    // gutter, their text and their scrollbar in exactly the same places and
    // switching between them does not shift the page.
    let inner = area.inset(3);
    // Parsed every frame. A note is a few kilobytes and the parse is a linear
    // scan, so caching it would buy nothing and could go stale.
    let blocks = markdown::parse_located(app.note().buffer.lines());
    // The scroll area keeps a gutter for its bar; the document gets the rest.
    let width = inner.w - Ui::SCROLL_GUTTER;
    // The app holds the scroll position, so it survives the tab being hidden.
    let mut scroll = app.preview_scroll;
    let req = render::Request {
        width,
        // The same pattern the source view lights up, so a search made in
        // either view is answered in both.
        search: app.vim.search_pattern().map(str::to_owned),
        reveal: app.preview_reveal.take(),
    };
    let mut drawn = render::Drawn::default();
    ui.scroll_area_with(inner, "preview", &mut scroll, |ui| {
        drawn = render::draw_document(ui, &blocks, req);
    });
    if let Some(y) = drawn.reveal {
        // A couple of rows of lead-in, so the hit lands inside the page rather
        // than jammed against its top edge.
        scroll.target = (y - pixui::font::line_h() * 2).max(0) as f32;
    }
    app.preview_scroll = scroll;
    if let Some(href) = drawn.clicked {
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
/// The furthest down the view may go: the line at which the last row of the
/// note sits on the last row of the pane.
///
/// Counted in the rows the lines actually wrap into, and walked from the end.
/// A note whose last paragraph wraps into six rows has six rows of tail and
/// one line of it — take the line count for the row count, as a bar measured
/// in lines does, and the end of the note is somewhere the view is not allowed
/// to reach. Which is exactly how it looks: the wheel stops, and there is
/// still text below it.
pub fn last_top(buf: &Buffer, cols: usize, visible: usize) -> usize {
    let total = buf.line_count();
    let mut rows = 0usize;
    let mut top = total;
    while top > 0 {
        let r = markdown::wrap_ranges(buf.line(top - 1), cols).len().max(1);
        if rows + r > visible {
            break;
        }
        rows += r;
        top -= 1;
    }
    top.min(total.saturating_sub(1))
}

fn position_at(
    buf: &Buffer,
    scroll: usize,
    cols: usize,
    origin: pixui::Point,
    p: pixui::Point,
) -> text::Cursor {
    let target_row = ((p.y - origin.y) / pixui::font::line_h()).max(0) as usize;
    let mut row = 0usize;
    let mut line = scroll;
    while line < buf.line_count() {
        let ranges = markdown::wrap_ranges(buf.line(line), cols);
        if row + ranges.len() > target_row {
            let (from, to) = ranges[target_row - row];
            // Floor rather than round: clicking a character should land *on*
            // it, since the caret in normal mode is a block over a character
            // rather than a bar between two.
            let rel = ((p.x - origin.x) / pixui::font::advance()).max(0) as usize;
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

    // The text keeps clear of the scrollbar's gutter, the same one a scroll
    // area reserves, so the two views of a note are the same shape.
    let framed = area.inset(3);
    let inner = Rect::new(framed.x, framed.y, framed.w - Ui::SCROLL_GUTTER, framed.h);
    let line_h = pixui::font::line_h();
    let advance = pixui::font::advance();
    let visible = (inner.h / line_h).max(1) as usize;
    let cols = ((inner.w - gutter()) / advance).max(8) as usize;

    let i = app.current.min(app.notes.len() - 1);
    let total = app.notes[i].buffer.line_count();

    // ---- pointer --------------------------------------------------------
    // Done before the caret-follow below, so a click sets the caret and the
    // scrolling then keeps it visible, rather than the two fighting.
    let origin = pixui::Point::new(inner.x + gutter(), inner.y - app.assist_lift);
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

    // ---- the view, moved by hand ----------------------------------------
    // The wheel and the bar move the *view*; everything else here moves the
    // caret and lets the view follow. So both are applied first, and the caret
    // is then pulled into whatever is now on screen — which is what an editor
    // does when you scroll away from the line you were typing on.
    let max_scroll = last_top(&app.notes[i].buffer, cols, visible);

    let was = app.scroll;
    if resp.hovered && ui.input.wheel != 0.0 {
        let step = app.editor_scroll.wheel_rows(ui.input.wheel, 3.0);
        app.scroll = (app.scroll as i32 - step).clamp(0, max_scroll as i32) as usize;
    }
    {
        // The bar is told about logical lines, not the visual rows they wrap
        // into: the editor scrolls by whole lines, as vim does, and a bar
        // measured in something the scrolling cannot express would stop
        // exactly where it could not go. Its content is stated as the distance
        // it may travel plus a pane, which is the same limit the wheel has.
        let mut st = app.editor_scroll;
        st.content = (max_scroll + visible) as i32 * line_h;
        st.viewport = visible as i32 * line_h;
        st.target = app.scroll as f32 * line_h as f32;
        st.shown = st.target;
        let track = Rect::new(framed.right() - Ui::BAR_W, framed.y, Ui::BAR_W, framed.h);
        ui.scroll_bar(track, "editor-bar", &mut st);
        app.scroll = (st.target / line_h as f32).round().max(0.0) as usize;
        app.editor_scroll = st;
    }
    app.scroll = app.scroll.min(max_scroll);
    if app.scroll != was {
        // Carry the caret with the view rather than letting the follow below
        // snap the view straight back to it.
        let buf = &mut app.notes[i].buffer;
        let line = buf
            .cursor
            .line
            .clamp(app.scroll, (app.scroll + visible - 1).min(total - 1));
        buf.cursor.line = line;
        buf.cursor.col = buf.cursor.col.min(buf.line_len(line));
    }

    let cursor = app.notes[i].buffer.cursor;
    // Set by the drawing loop below, and used after it: the mark is a control,
    // so it must not be clipped away with the text it hangs off.
    let mut mark_at = None;
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

    // ---- the assistant, opened between the lines -------------------------
    // Taken out of the application while the drawing borrows it, and put back
    // with whatever it decided afterwards.
    let model = app.helper.name().to_string();
    let mut open = app.assist.take();
    let block_w = inner.w - gutter();
    let block_h = open.as_ref().map_or(0, |a| a.height(block_w));
    let block_rows = ((block_h + line_h - 1) / line_h) as usize;
    let anchor = open.as_ref().map(|a| a.anchor());
    let mut outcome = assist::Outcome::None;
    let mut opened = false;

    // ---- room for it -----------------------------------------------------
    // The block opens under the line the selection ended on, and a selection
    // that ends at the foot of the pane leaves it nowhere to go — select a
    // whole note and the conversation opens below the last row on screen,
    // which is to say off it. So the text is lifted by however much the block
    // overhangs: the block sits against the bottom edge, the line it belongs
    // to stays directly above it, and the rows at the top scroll away, which
    // are the ones furthest from what is being talked about.
    //
    // Never lifted so far that the block's own top passes the top of the pane:
    // past that there is nothing left to give, and the block scrolls inside
    // itself instead.
    let lift = match anchor {
        Some(line) if block_h > 0 => {
            let mut rows = 0usize;
            for l in app.scroll..=line.min(total.saturating_sub(1)) {
                rows += markdown::wrap_ranges(buf.line(l), cols).len().max(1);
            }
            let block_y = inner.y + rows as i32 * line_h;
            (block_y + block_h - inner.bottom())
                .max(0)
                .min(block_y - inner.y)
        }
        _ => 0,
    };
    app.assist_lift = lift;
    // The rows the lift brings up from below.
    let extra = (lift + line_h - 1) / line_h;

    ui.clipped(inner, |ui| {
        let mut row = 0usize;
        let mut line_no = app.scroll;

        while row < visible + extra as usize && line_no < total {
            let text = buf.line(line_no);
            let spans = match code.get(line_no).and_then(Option::as_ref) {
                Some(highlighted) => highlighted.clone(),
                None => markdown::highlight(text, false),
            };
            let ranges = markdown::wrap_ranges(text, cols);
            let (caret_row, caret_col) = markdown::locate(&ranges, cursor.col);

            for (vi, &(from, to)) in ranges.iter().enumerate() {
                if row >= visible + extra as usize {
                    break;
                }
                let y = inner.y + row as i32 * line_h - lift;
                let text_x = inner.x + gutter();

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
                                th.well.lerp(th.highlight, 0.30),
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
                    // Where the assistant's mark goes: after the last row of
                    // the selection, which is the last one this ever runs for.
                    // Not for a block selection: that is a rectangle of
                    // columns, not a run of prose, and there is nothing to hand
                    // an editor — so it is not offered one.
                    let prose = !matches!(selection, Some(vim::Selection::Block { .. }));
                    if b >= a && prose {
                        // Out at the margin rather than against the end of the
                        // selection: a charwise selection usually ends in the
                        // middle of a line, and a control sitting there covers
                        // the very words it is offering to change.
                        mark_at = Some(pixui::Point::new(inner.right() - assist::MARK - 2, y));
                    }
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
                        // The caret closes toward its middle and opens back
                        // out, rather than switching on and off: the two ends
                        // travelling to meet each other is a blink you can
                        // watch happen, where a bar that simply vanishes at
                        // this size reads as a dropped frame.
                        //
                        // The curve holds it open for most of the cycle and
                        // spends only the turn of the wave shut, so the caret
                        // is at full height whenever you glance at it.
                        let cycle = app.caret_phase * 1.6;
                        let wave = (cycle * std::f32::consts::TAU).cos() * 0.5 + 0.5;
                        let h = (line_h as f32 * wave.powf(0.45)).round() as i32;
                        if h > 0 {
                            let bar = Rect::new(cx - 1, y - 1 + (line_h - h) / 2, 2, h);
                            ui.canvas.fill_rect(bar, th.positive.face);
                            // Lit ends, so what reads is the two edges closing
                            // in rather than the bar merely getting shorter.
                            ui.canvas.hline(bar.x, bar.y, 2, th.positive.hi);
                            ui.canvas.hline(bar.x, bar.bottom() - 1, 2, th.positive.hi);
                        }
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
                            Rect::new(cx - 1, y - 1, pixui::font::glyph_w() + 2, line_h),
                            th.accent.face,
                        );
                        let under = text.chars().nth(cursor.col).unwrap_or(' ');
                        pixui::font::draw_char(ui.canvas, cx, y, under, th.accent.ink);
                    }
                }

                row += 1;
            }
            // The block opens under the last line of what was selected, and
            // the lines below it move down to make room, the way an editor
            // makes room for anything else it has to say.
            if Some(line_no) == anchor && !opened {
                if let Some(a) = open.as_mut() {
                    let y = inner.y + row as i32 * line_h - lift;
                    let at = Rect::new(inner.x + gutter(), y, block_w, block_h);
                    outcome = ui.layer(at, |ui| a.show(ui, at, &model));
                    row += block_rows;
                    opened = true;
                }
            }
            line_no += 1;
        }

        // The line it belongs to is above the fold, so it goes at the top: a
        // conversation you cannot reach is worse than one out of place.
        if let (Some(a), false) = (open.as_mut(), opened) {
            let at = Rect::new(inner.x + gutter(), inner.y, block_w, block_h);
            outcome = ui.layer(at, |ui| a.show(ui, at, &model));
        }
    });

    app.assist = open;
    match outcome {
        assist::Outcome::None => {}
        assist::Outcome::Ask(ask) => {
            if !app.helper.ask(ask) {
                if let Some(a) = app.assist.as_mut() {
                    a.phase = assist::Phase::Failed("still busy with the last one".into());
                }
            }
        }
        assist::Outcome::Apply(text) => {
            if let Some(a) = app.assist.take() {
                app.apply_suggestion(&a, &text);
                app.status = "APPLIED".into();
            }
        }
        assist::Outcome::Close => {
            app.assist = None;
            app.status = "DISMISSED".into();
        }
    }

    // ---- the assistant's mark -------------------------------------------
    // Only where there is something to talk about, and only where the end of
    // the selection is actually on screen — a control you cannot see is a
    // control that takes clicks meant for something else.
    // A floating control over a text view: painted after the text so it is not
    // drawn over, and in a layer so the editor's own hit test — which covers
    // the whole pane — does not take the click first and move the caret with it.
    let offer = app.settings.assist && app.assist.is_none() && app.vim.mode != Mode::Insert;
    if let Some(at) = mark_at.filter(|p: &pixui::Point| offer && inner.contains(*p)) {
        let rect = Rect::new(at.x, at.y - 1, assist::MARK, assist::MARK);
        let pressed = ui.layer(rect, |ui| {
            ui.icon_button_at(rect, "assist-mark", pixui::icon::SPARK, pixui::Tone::Accent)
                .clicked
        });
        if pressed || std::mem::take(&mut app.assist_wanted) {
            app.open_assist();
        }
    }
    app.assist_wanted = false;

    area.inset(1)
}

/// The character range a selection covers, as one span of text.
///
/// A block selection has none: it is a rectangle of columns, and there is no
/// single run of prose to hand anybody. The mark simply does not appear for it.
fn selected_range(sel: vim::Selection, buf: &Buffer) -> Option<(text::Cursor, text::Cursor)> {
    match sel {
        vim::Selection::Chars { from, to } => Some((from, text::Cursor::new(to.line, to.col + 1))),
        vim::Selection::Lines { from, to } => {
            let last = to.min(buf.line_count().saturating_sub(1));
            Some((
                text::Cursor::new(from, 0),
                text::Cursor::new(last, buf.line_len(last)),
            ))
        }
        vim::Selection::Block { .. } => None,
    }
}

fn token_color(th: &Theme, tok: Tok) -> Color {
    match tok {
        Tok::Text => th.ink_light,
        Tok::Marker => th.ink_soft,
        Tok::Heading => th.accent.hi,
        Tok::Bold => th.ink_light.lerp(th.highlight, 0.7),
        Tok::Italic => th.info.hi,
        Tok::Code => th.positive.face,
        Tok::Link => th.info.face,
        Tok::Quote => th.ink_soft.lerp(th.ink_light, 0.4),
        Tok::Strike => th.ink_soft,
        Tok::Image => th.info.hi,
        Tok::CodePlain => th.ink_light,
        Tok::CodeKeyword => th.syntax.keyword,
        Tok::CodeType => th.syntax.type_name,
        Tok::CodeFunction => th.syntax.function,
        Tok::CodeString => th.syntax.string,
        Tok::CodeNumber => th.syntax.number,
        Tok::CodeComment => th.syntax.comment,
        Tok::CodePunct => th.syntax.punctuation,
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
/// Put the reference note in a vault that has not got one.
///
/// Separate from seeding, which only ever runs on an empty vault: this note
/// is the app's own, it grows every time the parser does, and a vault with
/// notes in it should still be able to show what the renderer can do. Never
/// overwritten — once it is in a vault it belongs to whoever is reading it.
fn install_reference(dir: &Path) {
    let path = dir.join("markdown-showcase.md");
    if !path.exists() {
        let _ = std::fs::write(&path, showcase::SHOWCASE);
    }
}

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
    let showcase = crate::showcase::SHOWCASE;

    let _ = std::fs::write(dir.join("welcome.md"), welcome);
    let _ = std::fs::write(dir.join("markdown-showcase.md"), showcase);
    let _ = std::fs::write(dir.join("vim-keys.md"), vim);
    let _ = std::fs::write(dir.join("ideas.md"), ideas);
}
