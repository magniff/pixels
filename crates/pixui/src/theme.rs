//! Colours and metrics, in one swappable struct.
//!
//! Widgets never reach into [`crate::color::palette`] directly — they read the
//! theme. Restyling the whole toolkit is therefore one struct literal, not a
//! sweep through the widget code.

use crate::color::{palette, Color};

/// The semantic role of a control, which selects its colour ramp.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tone {
    /// Cream, the default.
    #[default]
    Neutral,
    /// Warm orange. The one call to action on a screen.
    Accent,
    /// Red. Destructive.
    Danger,
    /// Green. Confirmations and healthy states.
    Positive,
    /// Teal. Informational, secondary emphasis.
    Info,
}

/// A three-stop ramp: the face, its lit top edge, its shaded bottom edge.
#[derive(Clone, Copy, Debug)]
pub struct Ramp {
    pub face: Color,
    pub hi: Color,
    pub lo: Color,
    /// Text drawn on top of `face`.
    pub ink: Color,
}

/// Pixel measurements. All integers — see [`crate::geom`].
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    /// Default height of a button, toggle or slider row.
    pub control_h: i32,
    /// Padding inside a panel.
    pub pad: i32,
    /// Gap between stacked controls.
    pub gap: i32,
    /// Corner pixels cut off chrome. 2 is the sweet spot at this scale.
    pub chamfer: i32,
    /// How far a button travels when pressed, and how tall its shadow is.
    pub press_depth: i32,
    /// Horizontal breathing room between a control's edge and its label.
    pub text_pad: i32,
    /// Height of a panel's title strip.
    pub title_h: i32,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            control_h: 15,
            pad: 5,
            gap: 4,
            chamfer: 2,
            press_depth: 2,
            text_pad: 5,
            title_h: 11,
        }
    }
}

/// The full look of a pixui application.
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    /// Behind the virtual screen, in the letterbox bars.
    pub letterbox: Color,
    /// The app background.
    pub background: Color,

    /// Filled panel body.
    pub panel: Color,
    pub panel_border: Color,
    /// Panel title strip.
    pub panel_title: Color,
    pub panel_title_ink: Color,

    /// Recessed areas: slider tracks, wells, text fields.
    pub well: Color,
    pub well_border: Color,

    pub ink: Color,
    pub ink_soft: Color,
    /// Text on dark surfaces.
    pub ink_light: Color,
    /// Cast beneath raised chrome.
    pub shadow: Color,

    pub neutral: Ramp,
    pub accent: Ramp,
    pub danger: Ramp,
    pub positive: Ramp,
    pub info: Ramp,

    pub focus_ring: Color,
    /// The body of the drawn mouse pointer.
    pub cursor_fill: Color,
    /// Its outline, which is what keeps it visible over both light panels and
    /// dark wells.
    pub cursor_outline: Color,
    /// Strength of the CRT scanline overlay, 0 to disable.
    pub scanline: f32,

    pub metrics: Metrics,
}

impl Default for Theme {
    fn default() -> Self {
        Self::warm()
    }
}

impl Theme {
    /// The stock look: dusty plum ground, cream chrome, warm orange accent.
    pub fn warm() -> Self {
        Self {
            letterbox: palette::VOID,
            background: palette::BASE,

            panel: palette::PAPER,
            panel_border: palette::INK,
            panel_title: palette::ACCENT,
            panel_title_ink: palette::INK,

            well: palette::BASE_HI,
            well_border: palette::INK,

            ink: palette::INK,
            ink_soft: palette::INK_SOFT,
            ink_light: palette::PAPER,
            shadow: palette::SHADOW,

            neutral: Ramp {
                face: palette::BUTTON,
                hi: palette::BUTTON_HI,
                lo: palette::BUTTON_LO,
                ink: palette::INK,
            },
            accent: Ramp {
                face: palette::ACCENT,
                hi: palette::ACCENT_HI,
                lo: palette::ACCENT_LO,
                ink: palette::INK,
            },
            danger: Ramp {
                face: palette::RED,
                hi: palette::RED.shade(0.30),
                lo: palette::RED.shade(-0.30),
                ink: palette::PAPER,
            },
            positive: Ramp {
                face: palette::GREEN,
                hi: palette::GREEN.shade(0.30),
                lo: palette::GREEN.shade(-0.30),
                ink: palette::INK,
            },
            info: Ramp {
                face: palette::TEAL,
                hi: palette::TEAL_HI,
                lo: palette::TEAL.shade(-0.30),
                ink: palette::INK,
            },

            focus_ring: palette::YELLOW,
            cursor_fill: palette::BUTTON_HI,
            cursor_outline: palette::INK,
            scanline: 0.05,
            metrics: Metrics::default(),
        }
    }

    /// A cooler, higher-contrast variant — proof that the theme is the only
    /// thing standing between the widgets and a different look.
    pub fn midnight() -> Self {
        let mut t = Self::warm();
        t.letterbox = Color::hex(0x0B0F14);
        t.background = Color::hex(0x161D26);
        t.panel = Color::hex(0x1F2833);
        t.panel_border = Color::hex(0x0B0F14);
        t.panel_title = Color::hex(0x3C6E8F);
        t.panel_title_ink = Color::hex(0xDDEBF5);
        t.well = Color::hex(0x121820);
        t.well_border = Color::hex(0x0B0F14);
        t.ink = Color::hex(0xDDEBF5);
        t.ink_soft = Color::hex(0x7E93A6);
        t.ink_light = Color::hex(0xDDEBF5);
        t.shadow = Color::hex(0x0B0F14);
        t.neutral = Ramp {
            face: Color::hex(0x33404F),
            hi: Color::hex(0x4A5B6D),
            lo: Color::hex(0x1F2833),
            ink: Color::hex(0xDDEBF5),
        };
        t.accent = Ramp {
            face: Color::hex(0x4FA3C7),
            hi: Color::hex(0x7FCBE8),
            lo: Color::hex(0x2E6B87),
            ink: Color::hex(0x0B0F14),
        };
        t.info = Ramp {
            face: Color::hex(0x3F8C9E),
            hi: Color::hex(0x6FBDCC),
            lo: Color::hex(0x25596B),
            ink: Color::hex(0x0B0F14),
        };
        t.positive = Ramp {
            face: Color::hex(0x5FA87C),
            hi: Color::hex(0x8ACFA1),
            lo: Color::hex(0x376B4F),
            ink: Color::hex(0x0B0F14),
        };
        t.danger = Ramp {
            face: Color::hex(0xC4564F),
            hi: Color::hex(0xE08079),
            lo: Color::hex(0x7E322D),
            ink: Color::hex(0xF5E3E1),
        };
        t.focus_ring = Color::hex(0x7FCBE8);
        t.cursor_fill = Color::hex(0xDDEBF5);
        t.cursor_outline = Color::hex(0x0B0F14);
        t
    }

    pub fn ramp(&self, tone: Tone) -> Ramp {
        match tone {
            Tone::Neutral => self.neutral,
            Tone::Accent => self.accent,
            Tone::Danger => self.danger,
            Tone::Positive => self.positive,
            Tone::Info => self.info,
        }
    }

    /// Ink that will actually be readable on `bg`.
    pub fn ink_on(&self, bg: Color) -> Color {
        if bg.luma() > 0.55 {
            self.ink
        } else {
            self.ink_light
        }
    }
}
