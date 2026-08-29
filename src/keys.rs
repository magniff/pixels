//! What the keys do, in whichever pane has them.

use pixui::{Key, Ui};

use crate::dialog::{DialogKind, FileDialog};
use crate::vault::slug;
use crate::view::open_externally;
use crate::vim::{Mode, VimEvent};
use crate::*;

impl Notes {
    /// Run a `:` command.
    pub(crate) fn run_command(&mut self, cmd: &str, ui: &mut Ui) {
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

    pub(crate) fn enter_list(&mut self) {
        let Some(&first) = self.shown().first() else {
            return;
        };
        self.current = first;
        self.scroll = 0;
        self.focus_pane(Pane::Notes);
    }

    /// Move the selection `delta` rows down the visible list, wrapping.
    pub(crate) fn step_note(&mut self, delta: i32) {
        let shown = self.shown();
        if shown.is_empty() {
            return;
        }
        let at = shown.iter().position(|&i| i == self.current).unwrap_or(0) as i32;
        let n = shown.len() as i32;
        self.current = shown[(at + delta).rem_euclid(n) as usize];
        self.scroll = 0;
    }

    pub(crate) fn follow_link(&mut self, href: &str) {
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

    pub(crate) fn close_current(&mut self) {
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

    /// Open a note, from something that found it by name.
    ///
    /// The keyboard goes to the editor rather than to the sidebar: somebody who
    /// asked for a note by typing its name is on their way into it, not on
    /// their way to a list of it.
    pub(crate) fn go_to_note(&mut self, i: usize) {
        if i >= self.notes.len() {
            return;
        }
        self.current = i;
        self.scroll = 0;
        let title = self.notes[i].title();
        self.focus_pane(Pane::Editor);
        self.status = title;
    }

    pub(crate) fn open_save_dialog(&mut self) {
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

/// Shortcuts that work wherever the keyboard is, the search field included.
///
/// Command specifically, not the primary modifier: off macOS the toolkit maps
/// `cmd` onto Control, and Control is already vim's — `Ctrl-r` is `Ctrl-r`
/// everywhere, and `Ctrl-n` walks the sidebar.
pub(crate) fn handle_shortcuts(ui: &mut Ui, app: &mut Notes) {
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
pub(crate) fn handle_search_keys(ui: &mut Ui, app: &mut Notes) {
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
pub(crate) fn handle_notes_keys(ui: &mut Ui, app: &mut Notes) {
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
pub(crate) fn handle_preview_keys(ui: &mut Ui, app: &mut Notes) {
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

pub(crate) fn handle_keys(ui: &mut Ui, app: &mut Notes) {
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
