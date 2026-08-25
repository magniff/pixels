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
    /// Body text inside a well — the editor, a text field. In a dark scheme
    /// that is a light ink; in a light one it is a dark ink, because the well
    /// is light too.
    pub ink_light: Color,
    /// Ink that can be read on a dark, saturated face: a button in the accent
    /// colour, a title strip. Distinct from `ink_light`, which follows the
    /// well rather than the face — in a light scheme those are opposites, and
    /// a theme that uses one for the other prints dark text on a dark button.
    pub ink_inverse: Color,
    /// Cast beneath raised chrome.
    pub shadow: Color,

    pub neutral: Ramp,
    pub accent: Ramp,
    pub danger: Ramp,
    pub positive: Ramp,
    pub info: Ramp,

    pub focus_ring: Color,
    /// What a search hit and a run of bold prose are lit with.
    pub highlight: Color,
    /// The colours code is drawn in. Kept here rather than in the application
    /// because a colour scheme is mostly a syntax palette: recolouring the
    /// chrome and leaving the code alone would be half a scheme.
    pub syntax: Syntax,
    /// The body of the drawn mouse pointer.
    pub cursor_fill: Color,
    /// Its outline, which is what keeps it visible over both light panels and
    /// dark wells.
    pub cursor_outline: Color,
    /// Strength of the CRT scanline overlay, 0 to disable.
    pub scanline: f32,

    pub metrics: Metrics,
}

/// The colours of code.
///
/// Seven roles, which is what this app's highlighter distinguishes: any scheme
/// worth the name has an opinion about all of them.
#[derive(Clone, Copy, Debug)]
pub struct Syntax {
    pub keyword: Color,
    pub type_name: Color,
    pub function: Color,
    pub string: Color,
    pub number: Color,
    pub comment: Color,
    pub punctuation: Color,
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
            ink_inverse: palette::PAPER,
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
            highlight: palette::YELLOW,
            syntax: Syntax {
                keyword: palette::ACCENT,
                type_name: palette::TEAL,
                function: palette::TEAL_HI,
                string: palette::GREEN,
                number: palette::YELLOW,
                // Not `ink_soft`, which is meant for text on the cream chrome
                // and sits at 1.5:1 against the editor's well — comments in a
                // fenced block were all but invisible until a contrast test
                // said so out loud.
                comment: palette::INK_SOFT.lerp(palette::PAPER, 0.42),
                punctuation: palette::PAPER.shade(-0.30),
            },
            cursor_fill: Color::hex(0xFFFFFF),
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
        t.ink_inverse = Color::hex(0xDDEBF5);
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
        t.highlight = Color::hex(0xE8C06A);
        t.syntax = Syntax {
            keyword: Color::hex(0x7FCBE8),
            type_name: Color::hex(0x6FBDCC),
            function: Color::hex(0x8ACFA1),
            string: Color::hex(0x8FBF8F),
            number: Color::hex(0xE8C06A),
            comment: Color::hex(0x7E93A6),
            punctuation: Color::hex(0x9FB2C4),
        };
        t.cursor_fill = Color::hex(0xFFFFFF);
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
        if bg.contrast(self.ink) >= bg.contrast(self.ink_inverse) {
            self.ink
        } else {
            self.ink_inverse
        }
    }
}

// ------------------------------------------------------------------ schemes

/// The colours a published scheme actually names, before they are mapped onto
/// the roles this toolkit draws with.
///
/// Every value here is the scheme's own, copied from its documentation. What is
/// *derived* is only the lighter and darker edge of each ramp: a raised control
/// needs three tones and most schemes name one, so the other two are a shade
/// either side of it. Nothing is invented, and nothing is guessed at.
pub struct Scheme {
    /// As the scheme's authors spell it.
    pub name: &'static str,
    pub dark: bool,
    /// The page everything sits on.
    pub bg: u32,
    /// Chrome drawn over it: panels, the drawer, dialogs.
    pub surface: u32,
    /// Raised controls on that chrome.
    pub raised: u32,
    /// Recessed fields: the editor, text boxes, scroll tracks.
    pub well: u32,
    /// Borders, shadows and the letterbox.
    pub edge: u32,
    /// Body text on `surface`.
    pub fg: u32,
    /// Body text on `well`, which in a light scheme is the same ink and in a
    /// dark one is usually brighter.
    pub fg_well: u32,
    /// The lightest tone the scheme has: what a label on a saturated accent
    /// button is printed in. Body text is not bright enough for that — most
    /// schemes' body text is a mid grey by design, and a mid grey on a mid blue
    /// is not a label, it is a smudge.
    pub bright: u32,
    /// Muted text on the chrome: hints, the second line of a list row.
    pub muted: u32,
    /// Comments in code, which sit on the well rather than on the chrome. Most
    /// schemes name one colour for both; the ones that put their panels and
    /// their editor at different depths need two, or the muted text vanishes
    /// into the panel it is written on.
    pub comment: u32,
    /// The one colour the scheme is known by.
    pub accent: u32,
    pub red: u32,
    pub green: u32,
    pub yellow: u32,
    pub blue: u32,
    pub cyan: u32,
    pub purple: u32,
}

impl Scheme {
    /// Map the scheme onto every role the widgets draw with.
    pub fn theme(&self) -> Theme {
        let c = Color::hex;
        let fg = c(self.fg);
        let fg_well = c(self.fg_well);
        let edge = c(self.edge);
        // Ink that can be read on a given face: the ramps are the scheme's own
        // colours, and how bright they are is the scheme's business, so which
        // ink goes on them is decided per colour rather than per theme.
        // The ink that reads on a dark face. In a dark scheme that is the body
        // ink; in a light one the body ink is dark too, so it has to be the
        // page — a light scheme still needs something to print on its accent.
        let inverse = c(self.bright);
        // And the ink that reads on a light face. In a dark scheme the darkest
        // thing to hand is the border; in a light one the border is a pale grey
        // and the body ink is what a label on a pale button wants.
        let on_light = if self.dark { edge } else { fg };
        // Whichever of the two can actually be read on it. A threshold on
        // brightness gets a mid-tone green wrong in one direction and a
        // mid-tone blue wrong in the other.
        let ink_on = move |face: Color| {
            if face.contrast(on_light) >= face.contrast(inverse) {
                on_light
            } else {
                inverse
            }
        };
        let ramp = move |v: u32| {
            let face = c(v);
            Ramp {
                face,
                hi: face.shade(0.26),
                lo: face.shade(-0.26),
                ink: ink_on(face),
            }
        };

        Theme {
            letterbox: edge.shade(if self.dark { -0.35 } else { -0.10 }),
            background: c(self.bg),

            panel: c(self.surface),
            panel_border: edge,
            panel_title: c(self.accent),
            panel_title_ink: ink_on(c(self.accent)),

            well: c(self.well),
            well_border: edge,

            ink: fg,
            ink_soft: c(self.muted),
            ink_light: fg_well,
            ink_inverse: inverse,
            shadow: edge,

            neutral: Ramp {
                face: c(self.raised),
                hi: c(self.raised).shade(0.20),
                lo: c(self.raised).shade(-0.20),
                ink: ink_on(c(self.raised)),
            },
            accent: ramp(self.accent),
            danger: ramp(self.red),
            positive: ramp(self.green),
            info: ramp(self.cyan),

            focus_ring: c(self.yellow),
            highlight: c(self.yellow),
            syntax: Syntax {
                // The roles every scheme documents, in the words they use:
                // keywords are the primary syntax colour, types the secondary,
                // strings green, numbers the scheme's purple or yellow.
                keyword: c(self.blue),
                type_name: c(self.cyan),
                function: c(self.accent),
                string: c(self.green),
                number: c(self.purple),
                comment: c(self.comment),
                punctuation: fg_well.lerp(c(self.comment), 0.5),
            },
            cursor_fill: if self.dark {
                c(self.fg_well)
            } else {
                Color::hex(0xFFFFFF)
            },
            cursor_outline: edge,
            // A little on the dark schemes, less on the light ones, where the
            // same strength reads as grey stripes rather than as glass.
            scanline: if self.dark { 0.05 } else { 0.02 },
            metrics: Metrics::default(),
        }
    }
}

/// Solarized, by Ethan Schoonover: sixteen tones chosen for equal perceived
/// contrast, so the dark and the light are the same scheme seen from either
/// side. <https://ethanschoonover.com/solarized/>
pub const SOLARIZED_DARK: Scheme = Scheme {
    name: "SOLARIZED DARK",
    dark: true,
    bg: 0x002b36,      // base03
    surface: 0x073642, // base02
    raised: 0x586e75,  // base01
    well: 0x002b36,    // base03
    edge: 0x001f27,    // below base03, for the one job the palette has no tone for
    fg: 0x93a1a1,      // base1
    fg_well: 0x839496, // base0
    bright: 0xfdf6e3,  // base3
    // base00 for the chrome and base01 for comments: Solarized means base01
    // against base03, and this app's panels are base02.
    muted: 0x657b83,   // base00
    comment: 0x586e75, // base01
    accent: 0x268bd2,  // blue
    red: 0xdc322f,
    green: 0x859900,
    yellow: 0xb58900,
    blue: 0x268bd2,
    cyan: 0x2aa198,
    purple: 0x6c71c4, // violet
};

pub const SOLARIZED_LIGHT: Scheme = Scheme {
    name: "SOLARIZED LIGHT",
    dark: false,
    bg: 0xfdf6e3,      // base3
    surface: 0xeee8d5, // base2
    raised: 0xfdf6e3,  // base3
    // The editor is the page, which in light mode is base3: Solarized keeps
    // base2 for the chrome around it.
    well: 0xfdf6e3,    // base3
    edge: 0x93a1a1,    // base1
    fg: 0x657b83,      // base00
    fg_well: 0x586e75, // base01
    bright: 0xfdf6e3,  // base3
    muted: 0x93a1a1,   // base1
    comment: 0x93a1a1, // base1
    accent: 0x268bd2,
    red: 0xdc322f,
    green: 0x859900,
    yellow: 0xb58900,
    blue: 0x268bd2,
    cyan: 0x2aa198,
    purple: 0x6c71c4,
};

/// Gruvbox, by Pavel Pertsev: retro groove, warm and low contrast.
/// <https://github.com/morhetz/gruvbox>
pub const GRUVBOX_DARK: Scheme = Scheme {
    name: "GRUVBOX DARK",
    dark: true,
    bg: 0x282828,      // dark0
    surface: 0x3c3836, // dark1
    raised: 0x504945,  // dark2
    well: 0x1d2021,    // dark0_hard
    edge: 0x1d2021,    // dark0_hard
    fg: 0xebdbb2,      // light1
    fg_well: 0xebdbb2, // light1
    bright: 0xfbf1c7,  // light0
    muted: 0x928374,   // gray
    comment: 0x928374, // gray
    accent: 0xfe8019,  // bright_orange
    red: 0xfb4934,     // bright_red
    green: 0xb8bb26,   // bright_green
    yellow: 0xfabd2f,  // bright_yellow
    blue: 0x83a598,    // bright_blue
    cyan: 0x8ec07c,    // bright_aqua
    purple: 0xd3869b,  // bright_purple
};

pub const GRUVBOX_LIGHT: Scheme = Scheme {
    name: "GRUVBOX LIGHT",
    dark: false,
    bg: 0xfbf1c7,      // light0
    surface: 0xebdbb2, // light1
    raised: 0xf2e5bc,  // light0_soft
    well: 0xf9f5d7,    // light0_hard
    edge: 0xa89984,    // light4
    fg: 0x3c3836,      // dark1
    fg_well: 0x3c3836, // dark1
    bright: 0xfbf1c7,  // light0
    muted: 0x7c6f64,   // dark4
    comment: 0x7c6f64, // dark4
    accent: 0xaf3a03,  // faded_orange
    red: 0x9d0006,     // faded_red
    green: 0x79740e,   // faded_green
    yellow: 0xb57614,  // faded_yellow
    blue: 0x076678,    // faded_blue
    cyan: 0x427b58,    // faded_aqua
    purple: 0x8f3f71,  // faded_purple
};

/// Nord, by Arctic Ice Studio: an arctic, north-bluish palette in four groups.
/// <https://www.nordtheme.com/docs/colors-and-palettes>
pub const NORD: Scheme = Scheme {
    name: "NORD",
    dark: true,
    bg: 0x2e3440,      // nord0
    surface: 0x3b4252, // nord1
    raised: 0x434c5e,  // nord2
    well: 0x272c36,    // below nord0, for the recess the palette has no tone for
    edge: 0x21252e,
    fg: 0xd8dee9,      // nord4
    fg_well: 0xe5e9f0, // nord5
    bright: 0xeceff4,  // nord6
    // Nord publishes two brightened comment tones, one for dark designs and
    // one for light: nord3 proper is unreadable against nord1, so the chrome
    // takes the lighter of the two and code takes the darker.
    muted: 0x8790a3,
    comment: 0x616e88,
    accent: 0x88c0d0, // nord8, the primary accent
    red: 0xbf616a,    // nord11
    green: 0xa3be8c,  // nord14
    yellow: 0xebcb8b, // nord13
    blue: 0x81a1c1,   // nord9, keywords
    cyan: 0x8fbcbb,   // nord7, types
    purple: 0xb48ead, // nord15, numbers
};

/// Dracula, by Zeno Rocha: dark, saturated, and specified down to the ANSI
/// table. <https://spec.draculatheme.com/>
pub const DRACULA: Scheme = Scheme {
    name: "DRACULA",
    dark: true,
    // Dracula's "background" is where its code lives, so that is the well and
    // the panels; the page behind them is its ANSI black, and the selection
    // colour raises a control off it. Putting panels on the selection colour
    // instead leaves the comment tone at 1.9:1 against them.
    bg: 0x21222c,      // ansi black
    surface: 0x282a36, // background
    raised: 0x44475a,  // current line / selection
    well: 0x282a36,    // background
    edge: 0x21222c,    // ansi black
    fg: 0xf8f8f2,      // foreground
    fg_well: 0xf8f8f2, // foreground
    bright: 0xf8f8f2,  // foreground
    muted: 0x6272a4,   // comment
    comment: 0x6272a4, // comment
    accent: 0xbd93f9,  // purple
    red: 0xff5555,
    green: 0x50fa7b,
    yellow: 0xf1fa8c,
    blue: 0xbd93f9, // purple, which is what Dracula highlights keywords with
    cyan: 0x8be9fd,
    purple: 0xff79c6, // pink, which is what it uses for numbers and operators
};

/// Catppuccin Latte, the light member of a four-flavour pastel family.
/// <https://github.com/catppuccin/catppuccin>
pub const LATTE: Scheme = Scheme {
    name: "CATPPUCCIN LATTE",
    dark: false,
    bg: 0xeff1f5,      // base
    surface: 0xe6e9ef, // mantle
    raised: 0xdce0e8,  // crust
    well: 0xdce0e8,    // crust
    edge: 0xacb0be,    // surface2
    fg: 0x4c4f69,      // text
    fg_well: 0x4c4f69, // text
    bright: 0xeff1f5,  // base
    muted: 0x8c8fa1,   // overlay1
    comment: 0x8c8fa1, // overlay1
    accent: 0x1e66f5,  // blue
    red: 0xd20f39,
    green: 0x40a02b,
    yellow: 0xdf8e1d,
    blue: 0x1e66f5,
    cyan: 0x179299, // teal
    purple: 0x8839ef,
};

/// A scheme's name, and how to build it.
pub type Named = (&'static str, fn() -> Theme);

/// Every scheme the toolkit ships, in the order an application should offer
/// them. The two hand-drawn ones come first because they are this toolkit's
/// own; the rest are other people's, reproduced from their documentation.
pub const SCHEMES: &[Named] = &[
    ("WARM", Theme::warm),
    ("MIDNIGHT", Theme::midnight),
    ("SOLARIZED DARK", || SOLARIZED_DARK.theme()),
    ("SOLARIZED LIGHT", || SOLARIZED_LIGHT.theme()),
    ("GRUVBOX DARK", || GRUVBOX_DARK.theme()),
    ("GRUVBOX LIGHT", || GRUVBOX_LIGHT.theme()),
    ("NORD", || NORD.theme()),
    ("DRACULA", || DRACULA.theme()),
    ("CATPPUCCIN LATTE", || LATTE.theme()),
];

/// The scheme with this name, if there is one.
pub fn scheme_named(name: &str) -> Option<Theme> {
    SCHEMES
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(name))
        .map(|(_, build)| build())
}
