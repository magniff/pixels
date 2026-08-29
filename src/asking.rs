//! What the application does with the assistant: the question it sends, the
//! change it applies, and what it tells the conversation afterwards.

use crate::text::Buffer;
use crate::vault::bare;
use crate::view::selected_range;
use crate::vim::Mode;
use crate::{assist, chat, digest, llm, settings, text, tools, Note, Notes};

impl Notes {
    /// Write the settings down and start an assistant that matches them.
    pub(crate) fn rebuild_assistant(&mut self) {
        self.helper = llm::Assistant::spawn(assistant(&self.settings));
        self.status = if self.settings.assist {
            format!("ASSISTANT: {}", self.helper.name())
        } else {
            "ASSISTANT OFF".into()
        };
    }

    /// Ask a download how it is getting on, once a frame.
    pub(crate) fn watch_download(&mut self) {
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

    /// Note that the assistant was asked for, if there is anything to ask about.
    ///
    /// Only a flag: where the block opens depends on where the selection lands
    /// on screen, and that is not known until the editor has drawn.
    pub(crate) fn want_assist(&mut self) {
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
    pub(crate) fn open_chat(&mut self) {
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
    /// The same as `took_up`, reachable from a test.
    ///
    /// Applying a change and telling the conversation about it are one action
    /// as far as anything outside is concerned, and a test that could do the
    /// first without the second would be testing a state the application never
    /// reaches.
    pub fn took_up_for_test(&mut self, change: &chat::Change, talk: &mut chat::Chat) {
        self.took_up(change, talk);
    }

    /// Tell the conversation what the files say now that it has been applied.
    ///
    /// Only the files this change touched, and only what they say here: the
    /// point is that a change the user accepted is not a change to be reported
    /// back as news. Everything else is still measured the usual way, so an
    /// edit made somewhere else still arrives at the end of the next question.
    pub(crate) fn took_up(&mut self, change: &chat::Change, talk: &mut chat::Chat) {
        let named = change
            .file
            .clone()
            .unwrap_or_else(|| self.note().filename());
        let named = bare(&named);
        // In this project. A note is known by its name within its folder,
        // and the same name can be in two: the vault has an `ideas.md` at the
        // top, and a model that made `trip/ideas.md` was told its new note
        // said what the old one did - a checklist about preview panes - and
        // then, at the next question, that the note had changed on disk.
        let here = self.note().project.clone();
        let says = |app: &Self, want: &str| {
            app.notes
                .iter()
                .find(|n| n.project == here && n.filename() == want)
                .map(|n| n.buffer.to_text())
        };
        // Files that are simply gone, which the model does know about.
        if let chat::What::Merge { from, .. } = &change.what {
            for one in from {
                let one = bare(one);
                if one != named {
                    talk.wrote(&one, None);
                }
            }
        }
        let now = says(self, &named);
        if matches!(change.what, chat::What::Delete) {
            talk.wrote(&named, None);
            return;
        }
        let Some(now) = now else {
            return;
        };
        // Told what the file says now, every time - as a diff for a file at
        // the front, and whole, numbered, for one the model made itself.
        //
        // An edit is line numbers, and if the numbers were wrong the file is
        // not what the model meant and it is the only one who cannot tell.
        // That happened: asked to change a line that was fifth, it wrote
        // `lines="3-3"`, the third line was somebody else's, and it went on
        // answering about a file that had stopped existing when the change it
        // asked for was made. A write it knows - the block is the file - but
        // the block has no numbers in the margin, and the next edit is made
        // against numbers it counted itself: a summary with the total on the
        // third line had its fourth line changed. So the block does not go
        // back, and this does. What it is not told, it cannot notice.
        let kind = match &change.what {
            chat::What::Write { .. } | chat::What::Lay { .. } => "write",
            chat::What::Merge { .. } => "merge",
            _ => "edit",
        };
        match talk.knows(&named).map(str::to_string) {
            Some(before) => talk.did(kind, &named, &before, &now),
            None => talk.made(kind, &named, &now),
        }
        talk.wrote(&named, Some(&now));
    }

    pub fn apply_change(&mut self, change: &chat::Change) {
        let here = self.note().project.clone();
        // Only this project. The panel does not offer a change aimed at
        // another, and this is the same rule at the other door, so nothing
        // that reaches here by another road can get past it either.
        if let Some(project) = change.misplaced(&self.folder()) {
            self.status =
                format!("THAT NOTE IS IN {project} - OPEN IT TO CHANGE IT").to_uppercase();
            return;
        }
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
            // A file laid down where one is already: refused, as the panel
            // refuses it. See `What::Lay`.
            chat::What::Lay { .. } if found.is_some() => {
                self.status = "THAT FILE IS ALREADY THERE - SAY WHICH LINES".into();
            }
            chat::What::Write { text } | chat::What::Lay { text } => {
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
            chat::What::Insert { after, text } => {
                let Some(i) = found else {
                    self.status = "THAT FILE IS NOT THERE".into();
                    return;
                };
                let buf = &mut self.notes[i].buffer;
                // One past the end is the end: see `Change::replacing`.
                if *after > buf.line_count() + 1 {
                    self.status = "THAT LINE IS NOT THERE".into();
                    return;
                }
                let after = (*after).min(buf.line_count());
                let fresh: Vec<String> = text.split('\n').map(str::to_string).collect();
                buf.checkpoint();
                buf.insert_lines(after, &fresh);
                buf.cursor = text::Cursor::new(after, 0);
                buf.clamp_cursor(false);
                self.current = i;
                self.status = "APPLIED".into();
            }
            chat::What::Edit { from, to, text } => {
                // A named file that is not there is refused, as in
                // `Change::replacing`; only an edit with no name at all means
                // the note in front of you, and that one `named` already is.
                let Some(i) = found else {
                    self.status = "THAT FILE IS NOT THERE".into();
                    return;
                };
                let buf = &mut self.notes[i].buffer;
                // One past the end is an append: see `Change::replacing`.
                let Some(first) = from.checked_sub(1).filter(|f| *f <= buf.line_count()) else {
                    self.status = "THOSE LINES ARE NOT THERE".into();
                    return;
                };
                if to < from {
                    self.status = "THOSE LINES ARE NOT THERE".into();
                    return;
                }
                buf.checkpoint();
                if first < buf.line_count() {
                    let last = to.min(&buf.line_count()).saturating_sub(1);
                    buf.delete_lines(first, last);
                }
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

    /// The project a conversation is about, as it is right now.
    pub(crate) fn folder(&self) -> chat::Folder<'_> {
        let here = self.note().project.clone();
        chat::Folder {
            project: here.clone(),
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
    pub(crate) fn begin_chat(&self, mut talk: chat::Chat) -> chat::Chat {
        talk.overhead = digest::vault(&self.notes).len() / 4;
        talk
    }

    /// The conversation, told what it is about.
    pub(crate) fn chat_ask(&mut self, talk: &mut chat::Chat) -> llm::Ask {
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
        // Kept with the question rather than folded into it on the way: see
        // `chat::told`. What the model is told once it must go on being told.
        if let Some(since) = &since {
            talk.tell(since);
        }
        llm::Ask {
            // What it said, without the bodies of the changes it proposed: a
            // block is a copy of a file, and the file itself is above, once
            // and current. See `chat::without_bodies`.
            turns: chat::as_sent(&talk.turns, &files)
                .into_iter()
                .zip(talk.turns.iter())
                .map(|(text, t)| llm::Turn { mine: t.mine, text })
                .collect(),
            vault,
            file: self.note().slug(),
            within: Some(within),
            since: None,
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
    pub(crate) fn open_assist(&mut self) {
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
    pub(crate) fn surround(&self, ask: &mut llm::Ask) {
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
    pub(crate) fn apply_suggestion(&mut self, open: &assist::Assist, text: &str) {
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
}

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
