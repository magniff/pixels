//! The two panels behind the Pixels menu.
//!
//! Both are drawn over everything and take the pointer with them, the way the
//! file dialogs do. They are application chrome rather than toolkit widgets:
//! what a setting *means* — which weights exist, what a good prompt is, how to
//! fetch one — is this app's business, and the toolkit contributes the panel,
//! the buttons and the text area they are built from.

use pixui::{Align, Key, Rect, Tone, Ui};

use crate::fetch::{megabytes, Download};
use crate::settings::{self, Settings, CATALOGUE};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Panel {
    About,
    Settings,
}

/// What the settings panel wants done about what was just pressed.
pub enum Action {
    None,
    Close,
    /// Run these weights from now on.
    Use(String),
    /// Fetch this entry of the catalogue.
    Fetch(usize),
    /// Stop fetching.
    Cancel,
    /// The prompt was edited, so the assistant needs rebuilding with it.
    Prompt,
}

/// Everything the chrome remembers between frames.
#[derive(Default)]
pub struct Chrome {
    pub menu_open: bool,
    /// How tall the settings panel wanted to be last time it drew.
    pub panel_h: i32,
    /// The settings as they were when the panel opened, so closing it can tell
    /// whether anything actually changed. Rebuilding the assistant means
    /// loading a model again, which is not a thing to do for nothing.
    pub opened_with: Option<Settings>,
    pub panel: Option<Panel>,
    pub download: Option<Download>,
    /// The last thing worth saying in the settings panel.
    pub notice: String,
}

const WIDTH: i32 = 320;
/// Room for the prompt: enough of it to read without scrolling.
const PROMPT_H: i32 = 62;

/// What the app is, and which build of it this is.
pub fn about(ui: &mut Ui) -> bool {
    let screen = ui.canvas.bounds();
    ui.canvas
        .fill_rect_blend(screen, pixui::palette::VOID, 0.55);
    let rect = screen.centered(WIDTH, 150);
    let inner = ui.panel(rect, "ABOUT");
    ui.capture_keyboard();

    let mut closed = ui.input.key_pressed(Key::Escape);
    let (body, footer) = inner.split_bottom(19);

    ui.column(body, 3, |ui| {
        ui.heading("PIXUI NOTES");
        ui.label_dim("A MARKDOWN EDITOR WITH VIM KEYS, DRAWN");
        ui.label_dim("ONE PIXEL AT A TIME. NO WIDGET TOOLKIT,");
        ui.label_dim("NO FONT ENGINE, NO GPU EXCEPT TO SHOW");
        ui.label_dim("THE FINISHED FRAME.");
        ui.space(2);
        ui.value_row("VERSION", env!("CARGO_PKG_VERSION"));
        // The trailing `+` means the tree had changes the commit does not.
        ui.value_row("BUILD", env!("GIT_REV"));
    });

    let close = Rect::new(footer.right() - 70, footer.y + 4, 70, 15);
    if ui.button_at(close, "CLOSE", Tone::Neutral).clicked {
        closed = true;
    }
    closed
}

/// Which weights to run, what to tell them, and how to get more.
pub fn settings(ui: &mut Ui, config: &mut Settings, chrome: &mut Chrome) -> Action {
    let th = *ui.theme;
    let screen = ui.canvas.bounds();
    ui.canvas
        .fill_rect_blend(screen, pixui::palette::VOID, 0.55);

    // As tall as what goes in it, measured from the frame before: the contents
    // change with what is installed and what is being fetched, and dead space
    // under the last control reads as a layout bug. One frame at the guessed
    // height when it first opens, and the right height from then on.
    let height = chrome.panel_h.clamp(120, screen.h - 20);
    let rect = screen.centered(WIDTH, height);
    let inner = ui.panel(rect, "SETTINGS");
    ui.capture_keyboard();

    let mut action = if ui.input.key_pressed(Key::Escape) {
        Action::Close
    } else {
        Action::None
    };

    // What would actually run: the chosen weights, or — when nothing has been
    // chosen — whatever is installed, which is what the assistant falls back to.
    let running = config
        .model_path()
        .and_then(|p| p.file_name().and_then(|n| n.to_str()).map(String::from));

    let (body, footer) = inner.split_bottom(19);
    let (_, used) = ui.column_measured(body, 2, |ui| {
        ui.label_colored("MODEL", th.accent.face);

        // ---- the catalogue ------------------------------------------------
        for (i, weights) in CATALOGUE.iter().enumerate() {
            let path = settings::models_dir().join(weights.file);
            let here = path.exists();
            let current = running.as_deref() == Some(weights.file);
            let row = ui.alloc(9);
            let (name, button) = row.split_left(row.w - 56);
            let ink = if current { th.positive.face } else { th.ink };
            ui.draw_text_in(name, weights.label, ink, Align::Left);
            ui.draw_text_in(
                name,
                &format!("{} MB", weights.megabytes),
                th.ink_soft,
                Align::Right,
            );

            let at = Rect::new(button.x + 4, row.y - 3, 52, 13);
            if current {
                ui.draw_text_in(at, "IN USE", th.positive.face, Align::Center);
            } else if here {
                if ui.button_at(at, "USE", Tone::Accent).clicked {
                    action = Action::Use(weights.file.to_string());
                }
            } else if chrome.download.is_some() {
                ui.draw_text_in(at, "-", th.ink_soft, Align::Center);
            } else if ui.button_at(at, "GET", Tone::Neutral).clicked {
                action = Action::Fetch(i);
            }
            let note = ui.alloc(8);
            ui.draw_text_in(note, weights.note, th.ink_soft.shade(-0.1), Align::Left);
        }

        // ---- anything else already on disk ---------------------------------
        for name in extras(config) {
            let row = ui.alloc(9);
            let (label, button) = row.split_left(row.w - 56);
            let current = running.as_deref() == Some(name.as_str());
            let ink = if current { th.positive.face } else { th.ink };
            ui.draw_text_in(label, &name.to_uppercase(), ink, Align::Left);
            let at = Rect::new(button.x + 4, row.y - 3, 52, 13);
            if current {
                ui.draw_text_in(at, "IN USE", th.positive.face, Align::Center);
            } else if ui.button_at(at, "USE", Tone::Accent).clicked {
                action = Action::Use(name.clone());
            }
            ui.space(8);
        }

        // ---- whatever is happening -----------------------------------------
        if let Some(down) = &chrome.download {
            let row = ui.alloc(9);
            let (label, button) = row.split_left(row.w - 56);
            ui.draw_text_in(
                label,
                &format!("{} {}", down.label, megabytes(down.bytes())),
                th.info.hi,
                Align::Left,
            );
            let at = Rect::new(button.x + 4, row.y - 3, 52, 13);
            if ui.button_at(at, "STOP", Tone::Danger).clicked {
                action = Action::Cancel;
            }
            let bar = ui.alloc(7);
            ui.progress_at(
                Rect::new(bar.x, bar.y, bar.w, 7),
                down.fraction(),
                Tone::Info,
            );
        } else if !chrome.notice.is_empty() {
            let row = ui.alloc(8);
            ui.draw_text_in(row, &chrome.notice, th.danger.face, Align::Left);
        }

        ui.space(2);
        ui.label_colored("SYSTEM PROMPT", th.accent.face);
        let area = ui.alloc(PROMPT_H);
        if ui.text_area_at(area, "prompt", &mut config.prompt).changed {
            action = Action::Prompt;
        }
    });

    // The chrome a panel spends on itself: border, title strip, the line under
    // it, and the padding inside.
    chrome.panel_h = used + 24 + 19 + 4;

    let restore = Rect::new(footer.x, footer.y + 4, 90, 15);
    if ui.button_at(restore, "DEFAULT", Tone::Neutral).clicked {
        config.prompt = settings::DEFAULT_PROMPT.to_string();
        action = Action::Prompt;
    }
    let close = Rect::new(footer.right() - 70, footer.y + 4, 70, 15);
    if ui.button_at(close, "CLOSE", Tone::Accent).clicked {
        action = Action::Close;
    }
    action
}

/// Weights on disk that the catalogue does not describe.
fn extras(_config: &Settings) -> Vec<String> {
    settings::installed()
        .iter()
        .filter(|p| settings::described(p).is_none())
        .filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(String::from))
        .collect()
}
