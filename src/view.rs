//! The frame: what is drawn where, every time the window is drawn.

use pixui::{Align, Color, Point, Rect, Theme, Tone, Ui};

use crate::dialog::DialogResult;
use crate::keys::{
    handle_keys, handle_notes_keys, handle_preview_keys, handle_search_keys, handle_shortcuts,
};
use crate::markdown::Tok;
use crate::text::Buffer;
use crate::vault::slug;
use crate::vim::Mode;
use crate::*;

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
pub(crate) fn sidebar_width(canvas_w: i32) -> i32 {
    (canvas_w / 5).clamp(120, 300)
}

impl Notes {
    /// Aim the keyboard at a pane, and let this frame's drawing know to claim
    /// focus for it.
    pub(crate) fn focus_pane(&mut self, pane: Pane) {
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
    pub(crate) fn shown(&self) -> Vec<usize> {
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
    pub(crate) fn apply_settings(&mut self, ui: &mut Ui, action: panels::Action) {
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

    /// Say whether anything here will look different next frame.
    ///
    /// The toolkit notices its own springs and blends; these are the app's own,
    /// and a frame nobody asks for is a frame that is not drawn.
    pub(crate) fn ask_for_frames(&self, ui: &mut Ui) {
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
    pub(crate) fn step_pick(&mut self, dt: f32) {
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
                    talk.failed = Some(if app.helper.gone() {
                        "the assistant is not running any more - reopen the app".into()
                    } else {
                        "still busy with the last one".into()
                    });
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
                app.took_up(&change, &mut talk);
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

/// The strip along the top: the application's menu on the left, and which note
/// is open on the right.
///
/// Returns where a dropdown from the menu would hang, which the caller needs
/// after everything else has drawn — a menu that opens under the pane it covers
/// is not a menu.
pub(crate) fn draw_titlebar(ui: &mut Ui, rect: Rect, app: &mut Notes) -> Point {
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

pub(crate) fn draw_statusbar(ui: &mut Ui, rect: Rect, app: &Notes) {
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
pub(crate) fn is_scheme(s: &str) -> bool {
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
pub(crate) fn open_externally(href: &str) -> std::io::Result<()> {
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

pub(crate) fn draw_sidebar(ui: &mut Ui, rect: Rect, app: &mut Notes, arrived: bool) {
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
pub(crate) fn context_menu(ui: &mut Ui, app: &mut Notes) {
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
pub(crate) fn draw_view(ui: &mut Ui, rect: Rect, app: &mut Notes, which: usize) -> Rect {
    if which == 0 {
        draw_editor(ui, rect, app)
    } else {
        draw_preview(ui, rect, app)
    }
}

pub(crate) fn draw_preview(ui: &mut Ui, rect: Rect, app: &mut Notes) -> Rect {
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

pub(crate) fn position_at(
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
pub(crate) fn draw_editor(ui: &mut Ui, rect: Rect, app: &mut Notes) -> Rect {
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
pub(crate) fn selected_range(
    sel: vim::Selection,
    buf: &Buffer,
) -> Option<(text::Cursor, text::Cursor)> {
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

pub(crate) fn token_color(th: &Theme, tok: Tok) -> Color {
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
