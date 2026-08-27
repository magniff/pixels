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
pub mod calc;
pub mod chat;
pub mod clock;
pub mod dialog;
pub mod e2e;
pub mod diff;
pub mod digest;
pub mod fetch;
pub mod finder;
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
pub mod tools;
pub mod vim;
pub mod web;

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
    /// When the file was last written or read by this program.
    ///
    /// What tells a change made somewhere else from one made here. Nothing
    /// else notices: the vault is read once at startup, so a note edited in
    /// another window stayed as it was in the editor, in the sidebar, and in
    /// what the assistant was shown - it answered "the bike is red" about a
    /// file that had said green for ten minutes.
    pub seen: Option<std::time::SystemTime>,
    /// The project it belongs to: the folder it sits in, under the vault.
    /// Empty for a note lying loose at the top of the vault, which is where
    /// one that has never been saved starts out.
    pub project: String,
}

impl Note {
    /// A note that has never been on disk.
    pub fn blank(project: String) -> Self {
        Self {
            path: None,
            buffer: Buffer::new(),
            seen: None,
            project,
        }
    }

    /// Where it sits in the vault: `project/file.md`, or just the file when it
    /// is loose. Unique across the vault, which its filename alone is not -
    /// two projects may each have a `todo.md`.
    pub fn slug(&self) -> String {
        if self.project.is_empty() {
            return self.filename();
        }
        format!("{}/{}", self.project, self.filename())
    }

    /// The note's display name: its first heading, else its file stem.
    pub fn title(&self) -> String {
        let derived = markdown::derive_title(self.buffer.lines(), 24);
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

/// What a menu opened on, and how far it has got.
///
/// The confirmations are the same thing one step on rather than a second kind
/// of menu: throwing a note away is one click and a second one that means it,
/// and both are entries in a list that appeared where the pointer is.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Context {
    Note(usize),
    SureAboutNote(usize),
    Project(String),
    SureAboutProject(String),
}

/// What renaming is renaming, while a name is being typed for it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Renaming {
    Note(usize),
    Project(String),
}

/// Whether one line of markdown links to a note.
///
/// The two spellings a vault uses: an ordinary markdown link whose target is a
/// file, and a wiki link in double brackets. Matched on the name rather than by
/// parsing the line, because a link is the only thing that ever ends in `.md)`
/// or sits inside `[[ ]]`, and a backlink list that missed half of them would
/// be worse than none.
fn points_at(line: &str, stem: &str, project: &str, near: bool) -> bool {
    let lower = line.to_lowercase();
    let mut targets = vec![format!("{stem}.md"), format!("[[{stem}]]")];
    if !project.is_empty() {
        targets.push(format!("{}/{stem}.md", project.to_lowercase()));
    }
    for want in targets {
        let Some(at) = lower.find(&want) else {
            continue;
        };
        // A bare name only counts from a note beside it; from another project
        // it would be a link to that project's own file of the same name.
        if want.contains('/') || want.starts_with("[[") || near {
            // Not part of a longer name: `water.md` is not `rainwater.md`.
            let before = lower[..at].chars().next_back();
            if before.is_none_or(|c| !c.is_alphanumeric() && c != '-' && c != '_') {
                return true;
            }
        }
    }
    false
}

/// How long a pause has to be before a note is written down, in seconds.
///
/// Long enough that it is a pause rather than a gap between two words, short
/// enough that what you lose to a power cut is a sentence.
const SETTLED: f32 = 1.5;

/// How often to look for a note changed by something other than this program.
///
/// Every frame would be a stat for every note sixty times a second, for a
/// thing that happens once an hour. Twice a second is far inside what anybody
/// notices and nothing at all beside one keystroke's work.
const LOOK: f32 = 0.5;

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
    /// The conversation that is open, if one is.
    pub chat: Option<chat::Chat>,
    /// Open while somebody is choosing which conversation to carry on.
    pub picker: Option<chat::Picker>,
    /// Seconds since the vault was last looked at for changes made elsewhere.
    pub looked: f32,
    /// Seconds since anything was typed, for the save that happens by itself.
    /// Counted from the last change rather than on a timer, so a save lands in
    /// a pause and never in the middle of a sentence.
    pub still: f32,
    /// Projects whose notes are folded away in the sidebar.
    pub folded: std::collections::HashSet<String>,
    /// The menu the other mouse button opened, and what it is about.
    pub context: Option<(pixui::Point, Context)>,
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
    pub renaming: Option<(Renaming, String)>,
    /// Set on the frame a rename begins, to move focus into its field.
    focus_rename: bool,
    /// Sidebar width, once the user has dragged it.
    ///
    /// `None` means "follow the canvas", which is the right default and stays
    /// right as the window is resized or the UI zoomed. A dragged divider is a
    /// deliberate choice, so from then on it wins.
    pub sidebar_w: Option<i32>,
}

/// Where the vault is: `./notes`, or wherever `PIXUI_NOTES_DIR` says.
pub fn notes_dir() -> PathBuf {
    std::env::var_os("PIXUI_NOTES_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("notes"))
}

/// Every note in a directory, in filename order.
///
/// Separate from opening the vault because a vault can be read without being
/// entered: `--ask` wants the notes to tell the model about, and has no
/// business seeding a directory or installing a reference note to do it.
/// A file name with any folder in front of it taken off.
///
/// The instructions ask for the name on its own, and models mostly give it -
/// but not always: asked to make a note while a project was open, one answered
/// `<write file="new-one/bike.md">` with the project's own name on the front.
/// That was joined to the project's folder a second time, so the note was
/// bound for `new-one/new-one/bike.md`, a folder that does not exist. The write
/// failed, silently, and the conversation went on answering out of a note that
/// had never reached the disk - which is exactly as wrong as it sounds, and
/// looked from the outside like a file that would not change.
fn bare(named: &str) -> String {
    named
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(named)
        .trim()
        .to_string()
}

/// When a file was last written, as the filesystem has it.
fn stamp(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

pub fn read_vault(dir: &Path) -> Vec<Note> {
    let mut notes = markdown_in(dir, "");
    let mut projects: Vec<String> = Vec::new();
    if let Ok(read) = std::fs::read_dir(dir) {
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            // A folder whose name begins with a dot is the program's own -
            // conversations live in one - and is not a project.
            if entry.path().is_dir() && !name.starts_with('.') {
                projects.push(name);
            }
        }
    }
    projects.sort();
    for project in projects {
        notes.extend(markdown_in(&dir.join(&project), &project));
    }
    notes
}

/// The `.md` files directly inside one directory, in filename order.
fn markdown_in(dir: &Path, project: &str) -> Vec<Note> {
    let mut notes = Vec::new();
    let Ok(read) = std::fs::read_dir(dir) else {
        return notes;
    };
    let mut paths: Vec<PathBuf> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect();
    paths.sort();
    for path in paths {
        if let Ok(text) = std::fs::read_to_string(&path) {
            let seen = stamp(&path);
            notes.push(Note {
                path: Some(path),
                buffer: Buffer::from_text(&text),
                seen,
                project: project.to_string(),
            });
        }
    }
    notes
}

/// Every project in the vault, in the order the sidebar shows them.
pub fn projects(notes: &[Note]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for note in notes {
        if !out.contains(&note.project) {
            out.push(note.project.clone());
        }
    }
    out
}

impl Notes {
    /// Open the vault, seeding it with a few notes the first time so the app
    /// does not open onto nothing.
    pub fn open(notes_dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&notes_dir);
        seed_if_empty(&notes_dir);
        install_reference(&notes_dir);

        let mut notes = read_vault(&notes_dir);
        if notes.is_empty() {
            notes.push(Note::blank(String::new()));
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
            chat: None,
            picker: None,
            looked: 0.0,
            still: 0.0,
            folded: std::collections::HashSet::new(),
            context: None,
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

    /// Which project a path on disk belongs to: the folder under the vault it
    /// sits in, or nothing when it lies loose at the top.
    fn project_of(&self, path: &Path) -> String {
        path.parent()
            .filter(|p| *p != self.notes_dir)
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
    }

    fn note(&self) -> &Note {
        &self.notes[self.current.min(self.notes.len() - 1)]
    }

    fn note_mut(&mut self) -> &mut Note {
        let i = self.current.min(self.notes.len() - 1);
        &mut self.notes[i]
    }

    /// Write down anything that has changed and has somewhere to go.
    ///
    /// A note editor that loses a note is not a note editor, and `:w` is a
    /// thing people forget - which is the whole argument. Quietly: no status
    /// line, because a save that happened on its own is not news, and a message
    /// every few seconds would be.
    ///
    /// Only notes with a name. One that has never been saved has nowhere to go
    /// and choosing a name for somebody is not this function's business.
    pub fn keep_up(&mut self) -> usize {
        let mut written = 0;
        let mut failed: Vec<String> = Vec::new();
        for note in &mut self.notes {
            if !note.buffer.dirty {
                continue;
            }
            let Some(path) = note.path.clone() else {
                continue;
            };
            // The folder first, for a note in a project that has only just
            // been named. And a write that fails is said out loud: one that
            // failed quietly left the editor showing a note that was not
            // anywhere, and nothing to say so.
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match std::fs::write(&path, note.buffer.to_text()) {
                Ok(()) => {
                    note.buffer.mark_saved();
                    // What this program wrote is not a change made behind its
                    // back, so the mark moves with the file.
                    note.seen = stamp(&path);
                    written += 1;
                }
                Err(e) => failed.push(format!("{}: {e}", path.display())),
            }
        }
        if let Some(why) = failed.first() {
            self.status = format!("COULD NOT WRITE {why}").to_uppercase();
        }
        written
    }

    /// Take up any change made to the vault by something other than this.
    ///
    /// The vault is read once at startup, so until this existed a note edited
    /// in another window was invisible: the editor showed the old text, and so
    /// did the assistant, which answered "the bike is red" about a file that
    /// had said green since before the question was asked.
    ///
    /// A note whose file has been written since this program last touched it
    /// is read again - unless it has unsaved changes here, in which case what
    /// somebody typed and has not saved is worth more than tidiness and is
    /// left exactly where it is. Files that have appeared are picked up, and
    /// files that have gone are let go of, on the same terms.
    ///
    /// Returns how many notes moved, so a caller can say so.
    /// Write out what is unsaved here and nobody else has touched.
    ///
    /// Only worth doing before a question, and it exists to tell two states
    /// apart that look identical from the outside. A note is "unsaved" from
    /// the moment a change is accepted until the save that follows a pause -
    /// but nothing has diverged, it simply has not been written yet. A note
    /// that is unsaved *and* whose file has moved underneath is a real
    /// disagreement, and the typing wins that one.
    ///
    /// Without this, the first was mistaken for the second: a change accepted
    /// and then a file edited outside, in the seconds before the save, and the
    /// edit was passed over as though there were work to protect. Which is
    /// exactly the moment somebody is most likely to do it - they have just
    /// watched the file change and gone to look at it.
    fn settle(&mut self) {
        for note in &mut self.notes {
            let Some(path) = note.path.clone() else { continue };
            if !note.buffer.dirty || stamp(&path) != note.seen {
                continue;
            }
            if std::fs::write(&path, note.buffer.to_text()).is_ok() {
                note.buffer.mark_saved();
                note.seen = stamp(&path);
            }
        }
    }

    /// Everything that has to be true before a question is asked about the
    /// vault: what is ours is written down, and what is theirs is read in.
    pub fn before_asking(&mut self) -> usize {
        self.settle();
        self.take_up_changes()
    }

    pub fn take_up_changes(&mut self) -> usize {
        let mut moved = 0;
        for note in &mut self.notes {
            let Some(path) = note.path.clone() else { continue };
            let now = stamp(&path);
            if now == note.seen {
                continue;
            }
            match std::fs::read_to_string(&path) {
                Ok(text) if text != note.buffer.to_text() => {
                    // Whoever wrote last wins, and somebody who saved a file
                    // meant to. That includes over a buffer here that has not
                    // been saved yet: it is the same person either way, and
                    // the one thing they did on purpose was save the file.
                    //
                    // Taken as an edit rather than by replacing the buffer, so
                    // `u` puts back whatever was in it. Nothing is lost by
                    // this, only moved one keystroke away.
                    let buf = &mut note.buffer;
                    buf.checkpoint();
                    let old = buf.line_count();
                    buf.insert_lines(
                        old,
                        &text.split('\n').map(str::to_string).collect::<Vec<_>>(),
                    );
                    buf.delete_lines(0, old - 1);
                    buf.clamp_cursor(false);
                    buf.mark_saved();
                    note.seen = now;
                    moved += 1;
                }
                // Same contents after all - a touch, or our own write seen
                // twice. Move the mark so it is not looked at again.
                Ok(_) => note.seen = now,
                // Gone, or unreadable. Dropped below rather than here, so the
                // list is not shuffled while it is being walked.
                Err(_) => {}
            }
        }
        // Anything that has been taken away, and anything that has appeared.
        let here: Vec<PathBuf> = read_vault(&self.notes_dir)
            .into_iter()
            .filter_map(|n| n.path)
            .collect();
        let before = self.notes.len();
        // A note that has never been written is not a note that has been
        // deleted - it is one this program has only just made, waiting for the
        // save that follows a pause. Dropping those threw away every file the
        // assistant created, in the seconds between accepting it and its
        // reaching the disk.
        self.notes.retain(|n| match &n.path {
            Some(p) => here.contains(p) || n.seen.is_none(),
            None => true,
        });
        moved += before - self.notes.len();
        if self.current >= self.notes.len() {
            self.current = self.notes.len().saturating_sub(1);
        }
        for path in here {
            if self.notes.iter().any(|n| n.path.as_ref() == Some(&path)) {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(&path) {
                let note = Note {
                    project: self.project_of(&path),
                    seen: stamp(&path),
                    path: Some(path),
                    buffer: Buffer::from_text(&text),
                };
                self.insert_note(note);
                moved += 1;
            }
        }
        // A vault with nothing left in it still needs somewhere to type.
        if self.notes.is_empty() {
            self.notes.push(Note::blank(String::new()));
            self.current = 0;
        }
        moved
    }

    fn save_to(&mut self, path: &Path) {
        let text = self.note().buffer.to_text();
        match std::fs::write(path, text) {
            Ok(()) => {
                self.status = format!(
                    "WROTE {}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                );
                let stamped = stamp(path);
                let note = self.note_mut();
                note.path = Some(path.to_path_buf());
                note.buffer.mark_saved();
                note.seen = stamped;
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
                let note = Note {
                    project: self.project_of(path),
                    path: Some(path.to_path_buf()),
                    buffer: Buffer::from_text(&text),
                    seen: stamp(path),
                };
                self.current = self.insert_note(note);
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
                } else if let Some(i) = self.find_note(arg) {
                    // A name already in the vault goes to that note wherever it
                    // is filed. Without this, `:e rendering.md` looks for a
                    // file at the top of a vault that keeps everything in
                    // projects, and makes an empty one.
                    self.go_to_note(i);
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
                // Into whichever project is being read, which is nearly always
                // the one the new note belongs beside.
                let here = self.note().project.clone();
                self.current = self.insert_note(Note::blank(here));
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
        // While the model has the GPU, the window keeps off it. Reading a
        // question is thousands of tokens of matrix arithmetic on the same
        // card the window is drawn on, and a frame that queues behind one of
        // those waits for the whole of it: the window fell to 50-87 frames a
        // second with 200ms gaps between some of them. Presenting on the CPU
        // costs more of the CPU and none of the wait.
        if self.helper.busy() {
            ui.share_gpu();
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
        if !self.settings.assist {
            return;
        }
        // The same key means two things, told apart by whether anything is
        // selected. A selection is a passage you want changed; no selection is
        // a question you want answered, and there is nothing to change.
        if self.vim.visual_kind().is_some() {
            self.assist_wanted = true;
        } else {
            self.open_chat();
        }
    }

    /// Open a conversation about the note being read.
    ///
    /// Onto the list of them when there are any, and straight into a new one
    /// when there are not: a list with one row saying "new" is a step that
    /// exists only to be walked past.
    fn open_chat(&mut self) {
        let here = self.note().project.clone();
        if chat::filed(&self.notes_dir, &here).is_empty() {
            let fresh = self.begin_chat(chat::Chat::new(here, self.note().filename()));
            self.chat = Some(fresh);
            self.status = "CHAT - ASK SOMETHING".into();
        } else {
            self.picker = Some(chat::Picker::new());
            self.status = "CHATS IN THIS PROJECT".into();
        }
    }

    /// Put a change the conversation proposed into the note.
    ///
    /// Through the buffer's own editing rather than by rewriting the lines
    /// underneath it, so `u` takes it back like anything else and the caret
    /// ends up somewhere sensible.
    pub fn apply_change(&mut self, change: &chat::Change) {
        let here = self.note().project.clone();
        let named = change
            .file
            .as_deref()
            .map(bare)
            .unwrap_or_else(|| self.note().filename());
        let found = self
            .notes
            .iter()
            .position(|n| n.project == here && n.filename() == named);

        match &change.what {
            chat::What::Write { text } => {
                // Unsaved either way, like anything else the editor makes:
                // `:w` puts it on disk, `u` takes it back, and until one of
                // those happens nothing has really happened.
                match found {
                    Some(i) => {
                        let buf = &mut self.notes[i].buffer;
                        buf.checkpoint();
                        // The new text goes in under the old and the old comes
                        // out from over it, rather than the other way round: a
                        // buffer keeps a line even when everything is deleted,
                        // and emptying it first leaves that line behind.
                        let old = buf.line_count();
                        buf.insert_lines(
                            old,
                            &text.split('\n').map(str::to_string).collect::<Vec<_>>(),
                        );
                        buf.delete_lines(0, old - 1);
                        buf.cursor = text::Cursor::new(0, 0);
                        buf.clamp_cursor(false);
                        self.current = i;
                        self.status = format!("REWROTE {named}").to_uppercase();
                    }
                    None => {
                        let mut note = Note::blank(here.clone());
                        note.buffer = Buffer::from_text(text);
                        note.path = Some(self.project_dir(&here).join(&named));
                        note.buffer.dirty = true;
                        self.current = self.insert_note(note);
                        self.status = format!("CREATED {named}").to_uppercase();
                    }
                }
                self.scroll = 0;
            }
            chat::What::Merge { from, text } => {
                // The parts, before anything moves: once the target has been
                // written the sources may already be gone, and a merge that
                // half happened is the thing this verb exists to prevent.
                let parts: Vec<String> = from
                    .iter()
                    .filter_map(|name| {
                        self.notes
                            .iter()
                            .find(|n| n.project == here && n.filename() == *name)
                            .map(|n| n.buffer.to_text())
                    })
                    .collect();
                if parts.len() != from.len() {
                    self.status = "SOME OF THOSE FILES ARE NOT THERE".into();
                    return;
                }
                let body = if text.is_empty() {
                    parts.join("\n\n")
                } else {
                    text.clone()
                };
                self.apply_change(&chat::Change {
                    file: change.file.clone(),
                    what: chat::What::Write { text: body },
                    state: None,
                });
                for name in from {
                    // Not the one being merged into, when it is one of them.
                    if *name == named {
                        continue;
                    }
                    self.apply_change(&chat::Change {
                        file: Some(name.clone()),
                        what: chat::What::Delete,
                        state: None,
                    });
                }
                self.status = format!("MERGED INTO {named}").to_uppercase();
                if let Some(i) = self.find_note(&named) {
                    self.current = i;
                }
            }
            chat::What::Delete => {
                let Some(i) = found else {
                    self.status = "THAT FILE IS NOT THERE".into();
                    return;
                };
                // Off the disk as well as out of the list. There is no undo for
                // this one, which is what the asking was for.
                if let Some(path) = self.notes[i].path.clone() {
                    let _ = std::fs::remove_file(path);
                }
                self.notes.remove(i);
                if self.notes.is_empty() {
                    self.notes.push(Note::blank(here));
                }
                self.current = self.current.min(self.notes.len() - 1);
                self.scroll = 0;
                self.status = format!("DELETED {named}").to_uppercase();
            }
            chat::What::Edit { from, to, text } => {
                let Some(i) = found else {
                    self.status = "THAT FILE IS NOT THERE".into();
                    return;
                };
                let buf = &mut self.notes[i].buffer;
                let Some(first) = from.checked_sub(1).filter(|f| *f < buf.line_count()) else {
                    self.status = "THOSE LINES ARE NOT THERE".into();
                    return;
                };
                let last = to.min(&buf.line_count()).saturating_sub(1);
                if last < first {
                    self.status = "THOSE LINES ARE NOT THERE".into();
                    return;
                }
                buf.checkpoint();
                buf.delete_lines(first, last);
                let fresh: Vec<String> = if text.is_empty() {
                    Vec::new()
                } else {
                    text.split('\n').map(str::to_string).collect()
                };
                buf.insert_lines(first, &fresh);
                buf.cursor = text::Cursor::new(first.min(buf.line_count().saturating_sub(1)), 0);
                buf.clamp_cursor(false);
                // Onto the note that changed, so what was accepted is what you
                // are looking at.
                self.current = i;
                self.status = "APPLIED".into();
            }
        }
    }

    /// A note already open, by where it sits or by what it is called.
    ///
    /// The slug first, because it is the unambiguous one, and the filename
    /// after it - preferring the current project, since a bare `todo.md` typed
    /// while reading one almost always means that project's.
    pub fn find_note(&self, name: &str) -> Option<usize> {
        if let Some(i) = self.notes.iter().position(|n| n.slug() == name) {
            return Some(i);
        }
        let here = self.note().project.clone();
        self.notes
            .iter()
            .position(|n| n.project == here && n.filename() == name)
            .or_else(|| self.notes.iter().position(|n| n.filename() == name))
    }

    /// Take a note off the disk and out of the drawer.
    pub fn delete_note(&mut self, index: usize) {
        if index >= self.notes.len() {
            return;
        }
        let named = self.notes[index].filename();
        if let Some(path) = self.notes[index].path.clone() {
            if let Err(e) = std::fs::remove_file(path) {
                self.status = format!("COULD NOT DELETE: {e}").to_uppercase();
                return;
            }
        }
        self.notes.remove(index);
        if self.notes.is_empty() {
            self.notes.push(Note::blank(String::new()));
        }
        if self.current >= index {
            self.current = self.current.saturating_sub(1).min(self.notes.len() - 1);
        }
        self.scroll = 0;
        self.status = format!("DELETED {named}").to_uppercase();
    }

    /// Rename a project: the folder, the notes in it, and its conversations.
    pub fn rename_project(&mut self, old: &str, name: &str) {
        let name = slug(name.trim());
        if name.is_empty() {
            self.status = "NAME REQUIRED".into();
            return;
        }
        if name == old {
            return;
        }
        let to = self.notes_dir.join(&name);
        if to.exists() {
            self.status = format!("{name} ALREADY EXISTS").to_uppercase();
            return;
        }
        if let Err(e) = std::fs::rename(self.notes_dir.join(old), &to) {
            self.status = format!("RENAME FAILED: {e}").to_uppercase();
            return;
        }
        // The conversations go with it: they are filed under the project, and
        // a project that moved without them would look like one that had never
        // been talked about.
        let chats = chat::folder(&self.notes_dir, old);
        if chats.exists() {
            let _ = std::fs::create_dir_all(self.notes_dir.join(".chats"));
            let _ = std::fs::rename(chats, chat::folder(&self.notes_dir, &name));
        }
        for note in &mut self.notes {
            if note.project == old {
                note.project = name.clone();
                if let Some(file) = note.path.as_ref().and_then(|p| p.file_name()) {
                    note.path = Some(to.join(file));
                }
            }
        }
        if self.folded.remove(old) {
            self.folded.insert(name.clone());
        }
        self.notes.sort_by_key(Self::order);
        self.status = format!("RENAMED TO {name}").to_uppercase();
    }

    /// Take a project, everything in it, and everything said about it.
    pub fn delete_project(&mut self, project: &str) {
        if let Err(e) = std::fs::remove_dir_all(self.notes_dir.join(project)) {
            self.status = format!("COULD NOT DELETE: {e}").to_uppercase();
            return;
        }
        let _ = std::fs::remove_dir_all(chat::folder(&self.notes_dir, project));
        self.notes.retain(|n| n.project != project);
        if self.notes.is_empty() {
            self.notes.push(Note::blank(String::new()));
        }
        self.current = self.current.min(self.notes.len() - 1);
        self.folded.remove(project);
        self.scroll = 0;
        self.status = format!("DELETED PROJECT {project}").to_uppercase();
    }

    /// Start a project: a folder, and the first note in it to make it visible.
    ///
    /// A project with nothing in it has no heading to draw - the headings come
    /// from the notes - so an empty one would be a folder you had made and
    /// could not see.
    pub fn new_project(&mut self) -> String {
        let mut name = "new-project".to_string();
        for n in 2.. {
            if !self.notes_dir.join(&name).exists() {
                break;
            }
            name = format!("new-project-{n}");
        }
        if std::fs::create_dir_all(self.notes_dir.join(&name)).is_err() {
            self.status = "COULD NOT MAKE THE FOLDER".into();
            return String::new();
        }
        self.current = self.insert_note(Note::blank(name.clone()));
        self.scroll = 0;
        self.filter.clear();
        self.status = "NEW PROJECT - NAME IT".into();
        name
    }

    /// Where a note sorts: the order the vault reads it in.
    fn order(note: &Note) -> (String, bool, String) {
        (note.project.clone(), note.path.is_none(), note.filename())
    }

    /// Put a note where it belongs, and say where that was.
    ///
    /// Where it belongs is the order the vault is read in: loose notes first,
    /// then projects, then filenames. Appending instead would put a new note
    /// after every project rather than in its own, and the sidebar - which
    /// begins a heading wherever the project changes - would draw that project
    /// a second time.
    pub fn insert_note(&mut self, note: Note) -> usize {
        // Named notes in filename order, and one that has never been saved
        // after them: it has no name yet, only a placeholder, and sorting on
        // that would file it under whatever punctuation the placeholder starts
        // with. At the end of its project is where "just made" belongs.
        let at = self
            .notes
            .iter()
            .position(|other| Self::order(other) > Self::order(&note))
            .unwrap_or(self.notes.len());
        self.notes.insert(at, note);
        at
    }

    /// Where a project's files live on disk.
    fn project_dir(&self, project: &str) -> PathBuf {
        if project.is_empty() {
            self.notes_dir.clone()
        } else {
            self.notes_dir.join(project)
        }
    }

    /// The project a conversation is about, as it is right now.
    fn folder(&self) -> chat::Folder<'_> {
        let here = self.note().project.clone();
        chat::Folder {
            here: self.note().filename(),
            files: self
                .notes
                .iter()
                .filter(|n| n.project == here)
                .map(|n| (n.filename(), n.buffer.lines()))
                .collect(),
        }
    }

    /// Hand a conversation the one number it cannot work out for itself.
    ///
    /// The vault digest is built here anyway on every question; doing it once
    /// more when a chat opens is cheaper than a chat rebuilding it every frame
    /// to put a number in its title bar.
    fn begin_chat(&self, mut talk: chat::Chat) -> chat::Chat {
        talk.overhead = digest::vault(&self.notes).len() / 4;
        talk
    }

    /// The conversation, told what it is about.
    fn chat_ask(&mut self, talk: &mut chat::Chat) -> llm::Ask {
        // Before anything is described to the model, make sure what is being
        // described is what is on disk. A question asked about a file somebody
        // edited in another window should be answered about that file - and
        // what is merely waiting to be saved here should not be mistaken for
        // work in danger, which is what `settle` is for.
        self.before_asking();
        let here = self.note().project.clone();
        let files: Vec<(String, String)> = self
            .notes
            .iter()
            .filter(|n| n.project == here)
            .map(|n| (n.filename(), n.buffer.to_text()))
            .collect();
        let (vault, within, since) = talk.context(&digest::vault(&self.notes), &files);
        llm::Ask {
            // What it said, without the bodies of the changes it proposed: a
            // block is a copy of a file, and the file itself is above, once
            // and current. See `chat::without_bodies`.
            turns: talk
                .turns
                .iter()
                .map(|t| llm::Turn {
                    mine: t.mine,
                    text: if t.mine {
                        t.text.clone()
                    } else {
                        chat::without_bodies(&t.text)
                    },
                })
                .collect(),
            vault,
            file: self.note().slug(),
            within: Some(within),
            since,
            // Only when it has been turned on, and only for a conversation.
            // A tool the model was never offered is one it cannot reach for
            // and cannot mention.
            // Working a sum out happens here and is always on offer; looking
            // something up goes to somebody else's server and is not.
            tools: tools::available(self.settings.web),
            // Off is not the same as absent, and the model should be able to
            // say which it is: a refusal that does not mention the switch
            // looks exactly like a feature that does not work.
            web_off: !self.settings.web,
            ..llm::Ask::default()
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

    /// Tell the question what it is part of: the note, and the vault.
    ///
    /// Done here rather than where the question was raised because this is
    /// where the notes are. The block knows a passage and a request; the model
    /// gets those plus everything around them.
    fn surround(&self, ask: &mut llm::Ask) {
        ask.vault = digest::vault(&self.notes);
        let Some(open) = self.assist.as_ref() else {
            return;
        };
        let i = self.current.min(self.notes.len() - 1);
        ask.file = self.notes[i].filename();
        ask.within = digest::around(&self.notes[i].buffer, open.from, open.to);
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
    /// The notes that point at this one.
    ///
    /// The thing that makes a vault a vault rather than a folder: a note is
    /// worth as much for what refers to it as for what is in it, and nothing
    /// else in the app answers "where did I mention this".
    ///
    /// Read out of the notes themselves rather than kept in an index. A vault
    /// is a few hundred notes held in memory already, and an index is a second
    /// copy of the truth that has to be told every time the first one changes.
    pub fn linked_from(&self, to: usize) -> Vec<usize> {
        let Some(target) = self.notes.get(to) else {
            return Vec::new();
        };
        let name = target.filename();
        let stem = name.strip_suffix(".md").unwrap_or(&name).to_lowercase();
        let mut out = Vec::new();
        for (i, note) in self.notes.iter().enumerate() {
            if i == to {
                continue;
            }
            // A note in another project may only be reached by a path, so a
            // bare name is a link to a neighbour and nothing else.
            let near = note.project == target.project;
            if note
                .buffer
                .lines()
                .iter()
                .any(|line| points_at(line, &stem, &target.project, near))
            {
                out.push(i);
            }
        }
        out
    }

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
            self.notes.push(Note::blank(String::new()));
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
        // Into the note's own project, not the top of the vault: renaming a
        // note should not move it out of the folder it belongs to.
        let dest = self
            .project_dir(&self.notes[index].project.clone())
            .join(&name);
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
        return Box::new(llm::local::Local::new(path, config.prompt.clone()));
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
        if let Some(talk) = app.chat.as_mut().filter(|c| c.waiting) {
            talk.progress = p;
        }
        let words = app.helper.partial().to_string();
        if let Some(talk) = app.chat.as_mut().filter(|c| c.waiting) {
            talk.partial = words;
        }
    }
    if let Some(reply) = app.helper.poll() {
        // Whoever is waiting gets it, and only one thing ever is: the worker
        // takes one question at a time and refuses a second while it is busy.
        let mut said = None;
        if let Some(talk) = app.chat.as_mut().filter(|c| c.waiting) {
            talk.partial.clear();
            talk.answered(reply, &app.notes_dir);
            said = Some("CHAT".to_string());
        } else if let Some(open) = app.assist.as_mut() {
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
    let modal = app.dialog.is_some()
        || app.chrome.panel.is_some()
        || app.finder.is_some()
        || app.chat.is_some()
        || app.picker.is_some();
    let typing_elsewhere = modal || app.assist.is_some();
    app.caret_phase += ui.input.dt;

    // ---- what somebody else changed --------------------------------------
    // Looked for a few times a second rather than every frame: it is a stat
    // for each note, which is cheap but not free, and a file being edited
    // elsewhere is not something that needs answering within 16ms.
    app.looked += ui.input.dt;
    if app.looked > LOOK {
        app.looked = 0.0;
        if app.take_up_changes() > 0 {
            app.status = "A NOTE CHANGED ON DISK".into();
        }
    }

    // ---- the save nobody asked for ---------------------------------------
    // A pause in the typing is when a note goes to disk. Anything at all that
    // arrived this frame resets the clock, so the write lands between
    // sentences rather than inside one, and `:w` becomes the thing you do when
    // you want to be sure rather than the thing between you and losing work.
    let busy = !ui.input.keys.is_empty() || ui.input.mouse_pressed || ui.input.wheel != 0.0;
    app.still = if busy { 0.0 } else { app.still + ui.input.dt };
    if app.notes.iter().any(|n| n.buffer.dirty) {
        if app.still > SETTLED {
            app.still = 0.0;
            app.keep_up();
        } else {
            // Frames are drawn when there is a reason to draw one, so a clock
            // that is running is a reason. Without this the app stops
            // redrawing the moment you stop typing, which is exactly when the
            // save was due.
            ui.request_repaint();
        }
    }
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

    // ---- conversations ---------------------------------------------------
    // The list first: choosing from it is what opens the other one, so the
    // frame it is chosen on is the frame the conversation appears.
    if let Some(mut picker) = app.picker.take() {
        // The project's conversations, and the file the chat will look at:
        // whichever one you were reading when you asked for it.
        let here = app.note().project.clone();
        let focus = app.note().filename();
        let chats = chat::filed(&app.notes_dir, &here);
        match picker.show(ui, &here, &chats) {
            chat::Picked::None => app.picker = Some(picker),
            chat::Picked::Close => app.status = "EDITOR".into(),
            chat::Picked::Fresh => {
                app.chat = Some(app.begin_chat(chat::Chat::new(here, focus)));
                app.status = "CHAT - ASK SOMETHING".into();
            }
            chat::Picked::Delete(path) => {
                // Off the disk, and out of memory if it is the one that is
                // open: a conversation you deleted while reading it should not
                // still be there, and should not save itself on the way out.
                let _ = std::fs::remove_file(&path);
                if app.chat.as_ref().and_then(|c| c.path.clone()) == Some(path) {
                    app.chat = None;
                }
                app.status = "CHAT DELETED".into();
                app.picker = Some(picker);
            }
            chat::Picked::Open(path) => {
                app.chat = Some(app.begin_chat(chat::Chat::open(&path, here, focus)));
                app.status = "CHAT".into();
            }
        }
    }
    if let Some(mut talk) = app.chat.take() {
        let folder = app.folder();
        match talk.show(ui, &folder) {
            chat::Outcome::None => app.chat = Some(talk),
            chat::Outcome::Ask => {
                let ask = app.chat_ask(&mut talk);
                if !app.helper.ask(ask) {
                    talk.waiting = false;
                    talk.failed = Some("still busy with the last one".into());
                }
                app.chat = Some(talk);
            }
            chat::Outcome::Web => {
                app.settings.web = !app.settings.web;
                let _ = app.settings.save();
                talk.notice = Some(if app.settings.web {
                    "looking things up is on - the weather, wikipedia, releases, and any page"
                        .into()
                } else {
                    "looking things up is off - nothing leaves this machine".into()
                });
                app.chat = Some(talk);
            }
            chat::Outcome::Stop => {
                app.helper.stop();
                app.status = "STOPPING".into();
                app.chat = Some(talk);
            }
            chat::Outcome::Save => {
                let _ = talk.save(&app.notes_dir);
                app.chat = Some(talk);
            }
            chat::Outcome::Apply(change) => {
                app.apply_change(&change);
                // The chat has already written the decision into its own
                // transcript; this is what puts that on disk.
                let _ = talk.save(&app.notes_dir);
                app.chat = Some(talk);
            }
            chat::Outcome::Close => {
                // Written down on the way out as well as after every answer: a
                // question typed and abandoned is still what you were thinking.
                let _ = talk.save(&app.notes_dir);
                app.status = "EDITOR".into();
            }
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
    // After the panes, not inside the one it was opened over: a layer only
    // settles who gets the *pointer*, and a menu drawn while the drawer was
    // being drawn is painted over by the editor beside it a moment later.
    context_menu(ui, app);

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
            // The clipboard keys, for the hand that reaches for these instead
            // of for `y` and `p`. Only where there is a note under them: a
            // text field handles its own three, and the note list has nothing
            // to copy.
            Key::Char('c') | Key::Char('x') | Key::Char('v')
                if app.pane == Pane::Editor && app.editor_tab == 0 && !ui.text_input_active() =>
            {
                let i = app.current.min(app.notes.len() - 1);
                match key {
                    Key::Char('c') => app.vim.copy_out(&mut app.notes[i].buffer),
                    Key::Char('x') => app.vim.cut_out(&mut app.notes[i].buffer),
                    _ => app.vim.paste_in(&mut app.notes[i].buffer),
                }
                if !app.vim.status.is_empty() {
                    app.status = std::mem::take(&mut app.vim.status).to_uppercase();
                }
            }
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
    // The vault is a forest, so the list is drawn as one: a heading per
    // project and its notes indented under it. A project folded away keeps its
    // heading - a tree you cannot see the shape of is a list again - and the
    // note being edited is never hidden by a fold.
    let mut folded: Vec<String> = Vec::new();
    for project in projects(&app.notes) {
        let holds_current = app
            .notes
            .get(app.current)
            .is_some_and(|n| n.project == project);
        if app.folded.contains(&project) && !holds_current {
            folded.push(project);
        }
    }

    let mut select = None;
    let mut toggle = None;
    let mut opened = None;
    let mut begin_rename = None;
    let mut commit_rename = None;
    let mut cancel_rename = false;

    ui.scroll_area(list, "notes", |ui| {
        // Tighter than the theme's, which is meant for controls. These are
        // names in a tree, and the air between two buttons is, between two
        // filenames, the air that stops them reading as one list.
        ui.set_spacing(1);
        if shown.is_empty() {
            ui.label_dim("  NO MATCHES");
            return;
        }
        let mut last = None;
        for &i in &shown {
            // A heading wherever the project changes, which is where the shown
            // notes are already grouped: they come out of the vault in project
            // order, so this needs no sorting of its own.
            let project = app.notes[i].project.clone();
            if last.as_ref() != Some(&project) {
                if !project.is_empty() {
                    // The air between projects, put back by hand now that the
                    // rows have none: it is the gap that says where one tree
                    // ends and the next begins, and it is the one that was
                    // right already.
                    if last.is_some() {
                        ui.space(6);
                    }
                    let head = ui.alloc(pixui::font::line_h() + 2);
                    let id = ui.id(&format!("proj{project}"));
                    let resp = ui.interact(id, head);
                    if resp.clicked {
                        toggle = Some(project.clone());
                    }
                    if resp.right_clicked {
                        opened = Some((ui.input.mouse, Context::Project(project.clone())));
                    }
                    let naming =
                        matches!(&app.renaming, Some((Renaming::Project(p), _)) if *p == project);
                    if naming {
                        // The heading becomes a field in place, the way a note
                        // row does, so a rename reads as editing *this* one.
                        let field = Rect::new(head.x + 11, head.y + 1, head.w - 15, 11);
                        let mut name = match app.renaming.take() {
                            Some((_, n)) => n,
                            None => String::new(),
                        };
                        let grab = app.focus_rename;
                        app.focus_rename = false;
                        ui.text_field_grab_at(field, "rename-project", &mut name, "", grab);
                        if ui.input.key_pressed(pixui::Key::Enter) {
                            commit_rename = Some((Renaming::Project(project.clone()), name));
                        } else if ui.input.key_pressed(pixui::Key::Escape) {
                            cancel_rename = true;
                        } else {
                            app.renaming = Some((Renaming::Project(project.clone()), name));
                        }
                    }
                    // Everything the heading normally says is drawn only when
                    // it is not being renamed: a caret and a count over a field
                    // being typed into is two things in one place.
                    if !naming {
                        let shut = folded.contains(&project);
                        let ink = if resp.hovered {
                            th.accent.hi
                        } else {
                            th.accent.face
                        };
                        // A caret that turns, which is the one part of a tree
                        // that says it is a tree rather than an indented list.
                        let mark = Rect::new(head.x + 3, head.y + 2, 7, 7);
                        pixui::icon::draw_centered(
                            ui.canvas,
                            mark,
                            if shut {
                                pixui::icon::CHEVRON
                            } else {
                                pixui::icon::CARET_DOWN
                            },
                            ink,
                        );
                        let at =
                            Rect::new(head.x + 13, head.y + 2, head.w - 17, pixui::font::line_h());
                        pixui::font::draw_text_styled(
                            ui.canvas,
                            at.x,
                            at.y,
                            &project.to_uppercase(),
                            ink,
                            true,
                        );
                        let count = shown
                            .iter()
                            .filter(|&&j| app.notes[j].project == project)
                            .count();
                        ui.draw_text_in(at, &count.to_string(), th.ink_soft, Align::Right);
                    }
                }
                last = Some(project.clone());
            }
            if folded.contains(&project) {
                continue;
            }
            // Copy what the row needs before drawing, so nothing holds a
            // borrow of the note list while the rename field mutates state.
            // The file's own name, not the heading inside it and not a line
            // of what it says. A tree is a list of things you can point at,
            // and the thing you point at is the file: it is what the rename
            // renames, what the model is told to edit, and what is on disk.
            // Cut to the room there is, with the tilde the previews and titles
            // use, and short of the corner where the unsaved dot goes.
            let indent = if project.is_empty() { 0 } else { 9 };
            let room = ((list.w - 26 - indent) / pixui::font::advance()).max(6) as usize;
            let title = markdown::truncate(&app.notes[i].filename(), room);
            let dirty = app.notes[i].buffer.dirty;
            let renaming = matches!(&app.renaming, Some((Renaming::Note(idx), _)) if *idx == i);

            let selected = i == app.current;
            let line = pixui::font::line_h();
            let h = line + 2;
            let row = ui.alloc(h);
            // Notes in a project are stepped in under its heading, and a loose
            // one is not: the indent is what says which of the two it is.
            let row = Rect::new(row.x + indent, row.y, row.w - indent, row.h);
            let id = ui.id(&format!("note{i}"));
            let resp = ui.interact(id, row);
            if resp.clicked {
                select = Some(i);
            }
            if resp.double_clicked {
                begin_rename = Some(i);
            }
            if resp.right_clicked {
                opened = Some((ui.input.mouse, Context::Note(i)));
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
                    commit_rename = Some((Renaming::Note(i), name));
                } else if ui.input.key_pressed(pixui::Key::Escape) {
                    cancel_rename = true;
                } else {
                    app.renaming = Some((Renaming::Note(i), name));
                }
            } else {
                // The ink travels with the fill under it. The patch covers the
                // words the whole way, so this is a blend rather than the two
                // clipped passes a patch growing from nothing would need.
                let ink = th.ink.lerp(th.accent.ink, held);
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
        app.renaming = Some((Renaming::Note(i), seed));
        app.focus_rename = true;
    }
    if let Some((what, name)) = commit_rename {
        match what {
            Renaming::Note(i) => app.rename_note(i, &name),
            Renaming::Project(old) => app.rename_project(&old, &name),
        }
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
    if let Some(project) = toggle {
        if !app.folded.remove(&project) {
            app.folded.insert(project);
        }
    }
    if let Some(where_and_what) = opened {
        app.context = Some(where_and_what);
    }

    // One button, because there is one thing down here that is not about a
    // note that already exists. Everything you can do to a note or a project
    // is on the note or the project, under the other mouse button.
    ui.column(footer, 3, |ui| {
        let cell = ui.alloc(14);
        if ui
            .icon_button_at(cell, "new-project", pixui::icon::FOLDER_PLUS, Tone::Accent)
            .clicked
        {
            let made = app.new_project();
            if !made.is_empty() {
                app.renaming = Some((Renaming::Project(made), String::new()));
                app.focus_rename = true;
            }
        }
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

/// The menu the other mouse button opened, and what it decided.
///
/// Drawn after the list rather than inside it, so it is not clipped by the
/// scroll area it was opened over, and so it sits above every row rather than
/// inside one of them.
fn context_menu(ui: &mut Ui, app: &mut Notes) {
    use pixui::icon;
    let Some((at, what)) = app.context.clone() else {
        return;
    };
    // Owned before the list borrows it: a confirmation should name the thing
    // it is about, and the name outlives the menu that mentions it.
    let naming = match &what {
        Context::SureAboutNote(i) => format!("DELETE {}", app.notes[*i].filename().to_uppercase()),
        Context::SureAboutProject(p) => {
            let held = app.notes.iter().filter(|n| n.project == *p).count();
            format!("DELETE {} AND {held} NOTES", p.to_uppercase())
        }
        _ => String::new(),
    };
    let items: Vec<pixui::Segment> = match &what {
        Context::Note(_) => vec![
            pixui::Segment::with_icon(icon::PENCIL, "RENAME"),
            pixui::Segment::with_icon(icon::BIN, "DELETE"),
        ],
        Context::Project(_) => vec![
            pixui::Segment::with_icon(icon::PAGE, "NEW NOTE"),
            pixui::Segment::with_icon(icon::PENCIL, "RENAME"),
            pixui::Segment::with_icon(icon::BIN, "DELETE"),
        ],
        // The second step, which is the same menu one entry on. A note goes
        // for good and a project takes everything in it, so neither is a thing
        // to do on one click that landed slightly wrong.
        Context::SureAboutNote(_) | Context::SureAboutProject(_) => vec![
            pixui::Segment::with_icon(icon::BIN, &naming),
            pixui::Segment::new("KEEP IT"),
        ],
    };
    let pick = ui.menu_items(at, &items);
    if pick.dismissed {
        app.context = None;
        return;
    }
    let Some(chosen) = pick.chosen else {
        return;
    };
    app.context = None;
    match (&what, chosen) {
        (Context::Note(i), 0) => {
            let seed = app.notes[*i]
                .path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| format!("{}.md", slug(&app.notes[*i].title())));
            app.renaming = Some((Renaming::Note(*i), seed));
            app.focus_rename = true;
        }
        (Context::Note(i), _) => app.context = Some((at, Context::SureAboutNote(*i))),
        (Context::Project(p), 0) => {
            app.current = app.insert_note(Note::blank(p.clone()));
            app.scroll = 0;
            app.status = "NEW NOTE".into();
        }
        (Context::Project(p), 1) => {
            app.renaming = Some((Renaming::Project(p.clone()), p.clone()));
            app.focus_rename = true;
        }
        (Context::Project(p), _) => app.context = Some((at, Context::SureAboutProject(p.clone()))),
        (Context::SureAboutNote(i), 0) => app.delete_note(*i),
        (Context::SureAboutProject(p), 0) => app.delete_project(p),
        // "Keep it", which is the whole reason there are two steps.
        _ => {}
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
        numbered: true,
        width,
        // The same pattern the source view lights up, so a search made in
        // either view is answered in both.
        search: app.vim.search_pattern().map(str::to_owned),
        reveal: app.preview_reveal.take(),
    };
    // Worked out before the closure, which cannot hold a borrow of the notes
    // while it draws into them.
    let back: Vec<(usize, String)> = app
        .linked_from(app.current.min(app.notes.len() - 1))
        .into_iter()
        .map(|i| (i, app.notes[i].filename()))
        .collect();
    let mut drawn = render::Drawn::default();
    let mut go_to = None;
    ui.scroll_area_with(inner, "preview", &mut scroll, |ui| {
        drawn = render::draw_document(ui, &blocks, req);
        if back.is_empty() {
            return;
        }
        // What points here, under what is here. A note is worth as much for
        // what refers to it as for what is in it, and this is the only place
        // the app answers "where did I mention this".
        let th = *ui.theme;
        let line = pixui::font::line_h();
        ui.space(line);
        let rule = ui.alloc(line);
        ui.canvas
            .hline(rule.x, rule.y + line / 2, rule.w, th.well_border);
        let head = ui.alloc(line);
        pixui::font::draw_text_styled(
            ui.canvas,
            head.x + crate::gutter(),
            head.y,
            &match back.len() {
                1 => "LINKED FROM 1 NOTE".to_string(),
                n => format!("LINKED FROM {n} NOTES"),
            },
            th.ink_soft,
            true,
        );
        for (i, name) in &back {
            let row = ui.alloc(line + 2);
            let at = Rect::new(
                row.x + crate::gutter(),
                row.y + 1,
                row.w - crate::gutter(),
                line,
            );
            let id = ui.id(&format!("back{i}"));
            let resp = ui.interact(id, at);
            if resp.hovered {
                ui.request_cursor(pixui::Cursor::Pointer);
            }
            if resp.clicked {
                go_to = Some(*i);
            }
            let ink = if resp.hovered {
                th.info.hi
            } else {
                th.info.face
            };
            pixui::font::draw_text(ui.canvas, at.x, at.y, name, ink);
            if resp.hovered {
                ui.canvas.hline(
                    at.x,
                    at.y + pixui::font::glyph_h() + 1,
                    pixui::font::text_width(name),
                    ink,
                );
            }
        }
        ui.space(line);
    });
    if let Some(i) = go_to {
        app.go_to_note(i);
    }
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
                        Rect::new(inner.x, pixui::font::row_top(y), inner.w, line_h),
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
                                Rect::new(
                                    x0,
                                    pixui::font::row_top(y),
                                    (b - a) as i32 * advance,
                                    line_h,
                                ),
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
                            Rect::new(
                                x0,
                                pixui::font::row_top(y),
                                (b - a) as i32 * advance,
                                line_h,
                            ),
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
                            let bar =
                                Rect::new(cx - 1, pixui::font::row_top(y) + (line_h - h) / 2, 2, h);
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
                            Rect::new(
                                cx - 1,
                                pixui::font::row_top(y),
                                pixui::font::glyph_w() + 2,
                                line_h,
                            ),
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
        assist::Outcome::Ask(mut ask) => {
            // Where the passage's surroundings are attached. The block that
            // raised the question does not know about the vault, and the model
            // does not know about either until here.
            app.surround(&mut ask);
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
    // A vault with projects in it is not an empty vault, even though there is
    // nothing lying loose at the top of it.
    if !read_vault(dir).is_empty() {
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
