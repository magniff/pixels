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

pub mod diff;

pub mod digest;

pub mod e2e;

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

mod asking;
mod keys;
mod vault;
mod view;

pub use asking::assistant;
pub use vault::{notes_dir, projects, read_vault, Note};
pub use view::{config, external_scheme, frame, gutter, last_top, note_matches, theme, theme_for};

use std::path::PathBuf;

use dialog::FileDialog;
use vim::Vim;

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

/// How long a pause has to be before a note is written down, in seconds.
///
/// Long enough that it is a pause rather than a gap between two words, short
/// enough that what you lose to a power cut is a sentence.
pub(crate) const SETTLED: f32 = 1.5;

/// How often to look for a note changed by something other than this program.
///
/// Every frame would be a stat for every note sixty times a second, for a
/// thing that happens once an hour. Twice a second is far inside what anybody
/// notices and nothing at all beside one keystroke's work.
pub(crate) const LOOK: f32 = 0.5;

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
/// The status this process means to leave with.
///
/// Read by the handler that takes the process down before ggml's own teardown
/// can - see `llm::local::leave_before_ggml_does`. That handler leaves through
/// `_exit`, which takes a code of its own and knows nothing about the one
/// `std::process::exit` was called with. So the suite that drives the real
/// model said "3 of 12 scenes failed" and then exited 0, every time, and a
/// shell script built on its status could never have noticed.
pub static EXIT_CODE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// Leave with this status, by whichever door the process leaves through.
pub fn leave(code: i32) -> ! {
    EXIT_CODE.store(code, std::sync::atomic::Ordering::SeqCst);
    std::process::exit(code)
}
