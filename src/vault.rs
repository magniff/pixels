//! The vault: notes on disk, notes in memory, and keeping the two the same.
//!
//! Everything that opens, reads, writes, renames or removes a file is here,
//! along with the one rule that decides between a buffer and a file when they
//! disagree - whoever wrote last wins, and somebody who saved a file meant to.

use std::path::{Path, PathBuf};

use crate::text::Buffer;
use crate::vim::Vim;
use crate::*;

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

/// Whether one line of markdown links to a note.
///
/// The two spellings a vault uses: an ordinary markdown link whose target is a
/// file, and a wiki link in double brackets. Matched on the name rather than by
/// parsing the line, because a link is the only thing that ever ends in `.md)`
/// or sits inside `[[ ]]`, and a backlink list that missed half of them would
/// be worse than none.
pub(crate) fn points_at(line: &str, stem: &str, project: &str, near: bool) -> bool {
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
pub(crate) fn bare(named: &str) -> String {
    chat::own_name(named)
}

/// When a file was last written, as the filesystem has it.
pub(crate) fn stamp(path: &Path) -> Option<std::time::SystemTime> {
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
pub(crate) fn markdown_in(dir: &Path, project: &str) -> Vec<Note> {
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
    pub(crate) fn project_of(&self, path: &Path) -> String {
        path.parent()
            .filter(|p| *p != self.notes_dir)
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
    }

    pub(crate) fn note(&self) -> &Note {
        &self.notes[self.current.min(self.notes.len() - 1)]
    }

    pub(crate) fn note_mut(&mut self) -> &mut Note {
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
    pub(crate) fn settle(&mut self) {
        for note in &mut self.notes {
            let Some(path) = note.path.clone() else {
                continue;
            };
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
            let Some(path) = note.path.clone() else {
                continue;
            };
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

    pub(crate) fn save_to(&mut self, path: &Path) {
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

    pub(crate) fn open_path(&mut self, path: &Path) {
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
    pub(crate) fn order(note: &Note) -> (String, bool, String) {
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
    pub(crate) fn project_dir(&self, project: &str) -> PathBuf {
        if project.is_empty() {
            self.notes_dir.clone()
        } else {
            self.notes_dir.join(project)
        }
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
}

pub(crate) fn slug(title: &str) -> String {
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
pub(crate) fn install_reference(dir: &Path) {
    let path = dir.join("markdown-showcase.md");
    if !path.exists() {
        let _ = std::fs::write(&path, showcase::SHOWCASE);
    }
}

pub(crate) fn seed_if_empty(dir: &Path) {
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
pub(crate) fn main() {
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
