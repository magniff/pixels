//! The system clipboard, borrowed from the platform's own tool.
//!
//! An application that yanks a line and cannot paste it into a browser has not
//! got a clipboard, it has got a register — and nobody who copies text thinks
//! of it that way. So this exists to make the boundary invisible: what the
//! editor copies is on the system clipboard, and what the system is holding is
//! what the editor pastes.
//!
//! It is done by talking to the small program every desktop already ships for
//! exactly this — `pbcopy` and `pbpaste`, `wl-copy`, `xclip`, `clip` — rather
//! than by linking the platform's clipboard API. The reason is compile weight,
//! not principle: the obvious crate for it pulls in a second copy of the whole
//! AppKit binding that the window backend already carries, to move a few
//! kilobytes of text on a keystroke a user pressed deliberately. Spawning
//! `pbcopy` costs a few milliseconds once per copy and nothing at all the rest
//! of the time.
//!
//! What is copied is also kept here, which does two things. It is the answer
//! when there is no helper to run — an application still yanks and pastes
//! within itself, which is where it was before this module existed — and it is
//! what a headless run uses, so a test suite never reaches out and overwrites
//! the clipboard of whoever is running it.

use std::cell::RefCell;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether to talk to the system at all. Off until an application says
/// otherwise, so tests and headless renders stay inside the process.
static SYSTEM: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// What this thread last copied.
    ///
    /// Per thread rather than per process, which matters in exactly one place:
    /// a test binary runs a test per thread, and a shared fallback clipboard
    /// would have them copying over each other's. An application draws on one
    /// thread and has one of these, which is the clipboard.
    static MIRROR: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Start using the system clipboard.
///
/// Called by the window backend on the way up. An application that draws
/// without a window — rendering screenshots, running tests — never calls it,
/// and keeps a clipboard of its own instead.
pub fn connect() {
    SYSTEM.store(true, Ordering::Relaxed);
}

/// Put `text` on the clipboard.
pub fn copy(text: &str) {
    MIRROR.with(|held| text.clone_into(&mut held.borrow_mut()));
    if !SYSTEM.load(Ordering::Relaxed) {
        return;
    }
    for (writer, _) in tools() {
        if write_with(writer, text) {
            return;
        }
    }
}

/// What the clipboard is holding, if anything.
pub fn paste() -> Option<String> {
    if SYSTEM.load(Ordering::Relaxed) {
        for (_, reader) in tools() {
            // An empty answer is not an answer: it is what an empty clipboard
            // and a helper that failed halfway both look like, and in either
            // case what this process last copied is the better guess.
            if let Some(text) = read_with(reader).filter(|t| !t.is_empty()) {
                return Some(text);
            }
        }
    }
    MIRROR.with(|held| Some(held.borrow().clone()).filter(|t| !t.is_empty()))
}

/// The (write, read) pairs to try, in order.
///
/// More than one on Linux because which of them exists says which display
/// server is running, and asking the wrong one is how you find out.
fn tools() -> &'static [(&'static [&'static str], &'static [&'static str])] {
    #[cfg(target_os = "macos")]
    {
        &[(&["pbcopy"], &["pbpaste"])]
    }
    #[cfg(target_os = "windows")]
    {
        &[(
            &["clip"],
            &["powershell", "-NoProfile", "-Command", "Get-Clipboard"],
        )]
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        &[
            (&["wl-copy"], &["wl-paste", "--no-newline"]),
            (
                &["xclip", "-selection", "clipboard"],
                &["xclip", "-selection", "clipboard", "-o"],
            ),
            (
                &["xsel", "--clipboard", "--input"],
                &["xsel", "--clipboard", "--output"],
            ),
        ]
    }
}

/// Feed `text` to a helper's standard input. False if it is not there.
fn write_with(argv: &[&str], text: &str) -> bool {
    let Ok(mut child) = Command::new(argv[0])
        .args(&argv[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    // Dropped before the wait, so the helper sees the end of its input and
    // exits instead of the two of us waiting on each other.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
    }
    child.wait().is_ok_and(|status| status.success())
}

/// Read a helper's standard output. None if it is not there, or said nothing.
fn read_with(argv: &[&str]) -> Option<String> {
    let out = Command::new(argv[0])
        .args(&argv[1..])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    // Line endings are normalised on the way in: everything above this reads
    // and writes text a line at a time, and a stray carriage return would be
    // pasted into a document as a character nothing can see.
    Some(
        String::from_utf8_lossy(&out.stdout)
            .replace("\r\n", "\n")
            .replace('\r', "\n"),
    )
}
