//! Save and open dialogs, drawn with pixui widgets.
//!
//! These are deliberately *not* native file pickers. The point of the exercise
//! is that a modal file browser — a scrolling list, a text field, keyboard
//! navigation, a dimmed backdrop that swallows clicks — is buildable out of the
//! same widget set as everything else.
//!
//! The toolkit contributes the widgets and the input gating; picking a
//! directory apart is application work, so `std::fs` lives here and nowhere in
//! `pixui`.

use std::path::{Path, PathBuf};

use pixui::{palette, Align, Key, Rect, Tone, Ui};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DialogKind {
    Open,
    Save,
}

/// What the dialog decided, once the user is done with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialogResult {
    Open(PathBuf),
    Save(PathBuf),
    Cancel,
}

#[derive(Clone, Debug)]
struct Entry {
    label: String,
    path: PathBuf,
    is_dir: bool,
}

pub struct FileDialog {
    pub kind: DialogKind,
    dir: PathBuf,
    entries: Vec<Entry>,
    selected: usize,
    /// The filename being typed, in save mode.
    pub filename: String,
    error: Option<String>,
    /// True on the frame the dialog is created.
    ///
    /// The keystroke that *opened* the dialog is still in this frame's input
    /// when the dialog first draws. Without this, the Enter that submits `:e`
    /// immediately confirms the dialog it just opened.
    fresh: bool,
}

impl FileDialog {
    pub fn new(kind: DialogKind, dir: &Path, filename: &str) -> Self {
        let mut d = Self {
            kind,
            dir: dir.to_path_buf(),
            entries: Vec::new(),
            selected: 0,
            filename: filename.to_string(),
            error: None,
            fresh: true,
        };
        d.refresh();
        d
    }

    /// Re-read the current directory: sub-directories first, then `.md` files.
    fn refresh(&mut self) {
        self.entries.clear();
        self.selected = 0;

        if self.dir.parent().is_some() {
            self.entries.push(Entry {
                label: "../".to_string(),
                path: self.dir.parent().unwrap_or(&self.dir).to_path_buf(),
                is_dir: true,
            });
        }

        let read = match std::fs::read_dir(&self.dir) {
            Ok(r) => r,
            Err(e) => {
                self.error = Some(format!("CANNOT READ DIR: {e}"));
                return;
            }
        };

        let mut dirs = Vec::new();
        let mut files = Vec::new();
        for entry in read.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            // Hidden files are noise in a notes folder.
            if name.starts_with('.') {
                continue;
            }
            let is_dir = path.is_dir();
            if is_dir {
                dirs.push(Entry {
                    label: format!("{name}/"),
                    path,
                    is_dir,
                });
            } else if path.extension().is_some_and(|e| e == "md" || e == "txt") {
                files.push(Entry {
                    label: name,
                    path,
                    is_dir,
                });
            }
        }
        dirs.sort_by_key(|e| e.label.to_lowercase());
        files.sort_by_key(|e| e.label.to_lowercase());
        self.entries.extend(dirs);
        self.entries.extend(files);
        self.error = None;
    }

    fn enter(&mut self, index: usize) -> Option<DialogResult> {
        let entry = self.entries.get(index)?.clone();
        if entry.is_dir {
            self.dir = entry.path;
            self.refresh();
            return None;
        }
        match self.kind {
            DialogKind::Open => Some(DialogResult::Open(entry.path)),
            DialogKind::Save => {
                // Selecting an existing file in save mode fills the name in
                // rather than overwriting on one click.
                self.filename = entry.label;
                None
            }
        }
    }

    fn confirm(&mut self) -> Option<DialogResult> {
        match self.kind {
            DialogKind::Open => {
                let index = self.selected;
                self.enter(index)
            }
            DialogKind::Save => {
                let mut name = self.filename.trim().to_string();
                if name.is_empty() {
                    self.error = Some("NAME REQUIRED".into());
                    return None;
                }
                if !name.contains('.') {
                    name.push_str(".md");
                }
                Some(DialogResult::Save(self.dir.join(name)))
            }
        }
    }

    /// Draw the dialog and report a decision. `None` means it is still open.
    ///
    /// Call this *after* the rest of the UI, with the rest wrapped in
    /// [`Ui::input_blocked`], so clicks cannot reach what is behind it.
    pub fn show(&mut self, ui: &mut Ui) -> Option<DialogResult> {
        let screen = ui.canvas.bounds();

        // Dim the whole screen so the dialog reads as a separate layer.
        ui.canvas.fill_rect_blend(screen, palette::VOID, 0.55);

        let rect = screen.centered(300, 210);
        let title = match self.kind {
            DialogKind::Open => "OPEN NOTE",
            DialogKind::Save => "SAVE NOTE AS",
        };
        let inner = ui.panel(rect, title);

        // The dialog owns the keyboard while it is up.
        ui.capture_keyboard();
        let mut result = None;

        // ---- keyboard: vim-ish navigation --------------------------------
        // Skip the frame we were opened on, so the key that opened us does not
        // also act on us.
        if self.fresh {
            self.fresh = false;
        } else {
            let count = self.entries.len();
            let typing = self.kind == DialogKind::Save;
            for key in &ui.input.keys.clone() {
                match key {
                    Key::Escape => result = Some(DialogResult::Cancel),
                    Key::Enter => result = self.confirm(),
                    Key::Down if count > 0 => self.selected = (self.selected + 1) % count,
                    Key::Up if count > 0 => self.selected = (self.selected + count - 1) % count,
                    // j/k navigate, but not while a filename is being typed —
                    // there they are just letters.
                    Key::Char('j') if count > 0 && !typing => {
                        self.selected = (self.selected + 1) % count;
                    }
                    Key::Char('k') if count > 0 && !typing => {
                        self.selected = (self.selected + count - 1) % count;
                    }
                    _ => {}
                }
            }
        }
        if result.is_some() {
            return result;
        }

        // Reserve the fixed-height footer, then let the list have the rest —
        // otherwise the dialog grows a dead band above the buttons.
        let footer_h = if self.kind == DialogKind::Save {
            15 + 4 + 15
        } else {
            15
        };
        let (top, footer) = inner.split_bottom(footer_h + 4);

        ui.column(top, 4, |ui| {
            // ---- current directory, or the last error --------------------
            match &self.error {
                Some(err) => {
                    let msg = err.clone();
                    ui.label_colored(&msg, palette::RED);
                }
                None => {
                    let path = self.dir.display().to_string();
                    let shown = shorten_path(&path, 46);
                    ui.label_dim(&shown);
                }
            }

            // ---- file list -----------------------------------------------
            let list = ui.alloc_rest();
            let th = *ui.theme;
            ui.canvas.box_chamfer(list, th.well, th.well_border, 1);

            let mut clicked: Option<usize> = None;
            ui.scroll_area(list.inset(2), "files", |ui| {
                for (i, entry) in self.entries.iter().enumerate() {
                    let row = ui.alloc(10);
                    let selected = i == self.selected;
                    let id = ui.id(&format!("row{i}"));
                    let resp = ui.interact(id, row);

                    if selected {
                        ui.canvas.fill_rect(row, th.accent.face);
                    } else if resp.hovered {
                        ui.canvas.fill_rect(row, th.well.shade(0.15));
                    }

                    let ink = if selected {
                        th.accent.ink
                    } else if entry.is_dir {
                        th.info.face
                    } else {
                        th.ink_light
                    };
                    let text = Rect::new(row.x + 3, row.y, row.w - 6, row.h);
                    ui.draw_text_in(text, &entry.label, ink, Align::Left);

                    if resp.clicked {
                        clicked = Some(i);
                    }
                }
                if self.entries.is_empty() {
                    ui.label_dim("  (NO NOTES HERE)");
                }
            });

            if let Some(i) = clicked {
                // A click selects; a click on the already-selected row opens.
                if self.selected == i {
                    result = self.enter(i);
                } else {
                    self.selected = i;
                }
            }
        });

        ui.column(footer, 4, |ui| {
            // ---- filename ------------------------------------------------
            if self.kind == DialogKind::Save {
                let row = ui.alloc(15);
                let (label, field) = row.split_left(56);
                ui.draw_text_in(label, "NAME", ui.theme.ink, Align::Left);
                let mut name = std::mem::take(&mut self.filename);
                ui.text_field_at(field, "filename", &mut name);
                self.filename = name;
            }

            // ---- buttons -------------------------------------------------
            let row = ui.alloc(15);
            ui.row(row, 5, |ui| {
                let w = (row.w - 10) / 3;
                let cell = ui.alloc(w);
                if ui.button_at(cell, "UP", Tone::Neutral).clicked {
                    if let Some(parent) = self.dir.parent() {
                        self.dir = parent.to_path_buf();
                        self.refresh();
                    }
                }
                let cell = ui.alloc(w);
                if ui.button_at(cell, "CANCEL", Tone::Neutral).clicked {
                    result = Some(DialogResult::Cancel);
                }
                let cell = ui.alloc_rest();
                let label = if self.kind == DialogKind::Open {
                    "OPEN"
                } else {
                    "SAVE"
                };
                if ui.button_at(cell, label, Tone::Accent).clicked {
                    result = self.confirm();
                }
            });
        });

        result
    }
}

/// Keep the tail of a path, which is the informative end.
fn shorten_path(path: &str, max: usize) -> String {
    let n = path.chars().count();
    if n <= max {
        return path.to_uppercase();
    }
    let tail: String = path.chars().skip(n - max + 3).collect();
    format!("...{tail}").to_uppercase()
}
