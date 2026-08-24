//! Colour, stored in the `0x00RRGGBB` layout that softbuffer wants, so
//! presenting a frame is a straight memcpy with no per-pixel conversion.

/// A packed opaque colour: `0x00RRGGBB`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Color(pub u32);

impl Color {
    pub const fn hex(v: u32) -> Self {
        Color(v & 0x00FF_FFFF)
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color(((r as u32) << 16) | ((g as u32) << 8) | b as u32)
    }

    pub const fn r(self) -> u8 {
        (self.0 >> 16) as u8
    }

    pub const fn g(self) -> u8 {
        (self.0 >> 8) as u8
    }

    pub const fn b(self) -> u8 {
        self.0 as u8
    }

    /// Blend towards `other`. `t` is clamped to `0..=1`.
    pub fn lerp(self, other: Color, t: f32) -> Color {
        let t = t.clamp(0.0, 1.0);
        let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Color::rgb(
            mix(self.r(), other.r()),
            mix(self.g(), other.g()),
            mix(self.b(), other.b()),
        )
    }

    /// Positive `amount` lightens towards white, negative darkens towards black.
    /// Retro palettes look muddy under a pure multiply, so this is a lerp.
    pub fn shade(self, amount: f32) -> Color {
        if amount >= 0.0 {
            self.lerp(Color::hex(0xFFFFFF), amount)
        } else {
            self.lerp(Color::hex(0x000000), -amount)
        }
    }

    /// Rough perceptual luminance, `0.0..=1.0`. Used to pick readable ink.
    pub fn luma(self) -> f32 {
        (0.299 * self.r() as f32 + 0.587 * self.g() as f32 + 0.114 * self.b() as f32) / 255.0
    }
}

/// The stock palette: warm, slightly dusty, deliberately small.
///
/// Sixteen colours is not a technical limit — it is a discipline. A tight ramp
/// is what makes hand-drawn pixel art read as a coherent set rather than a pile
/// of gradients, and the same holds for a UI built out of it.
pub mod palette {
    use super::Color;

    /// Near-black warm plum. Outlines and body text.
    pub const INK: Color = Color::hex(0x2A1F2D);
    /// Softer ink for secondary text on light surfaces.
    pub const INK_SOFT: Color = Color::hex(0x6B5560);
    /// Warm shadow tone, used under raised chrome.
    pub const SHADOW: Color = Color::hex(0x4A3540);

    /// Deep background behind everything.
    pub const VOID: Color = Color::hex(0x241A21);
    /// App background.
    pub const BASE: Color = Color::hex(0x3A2B34);
    /// Slightly raised background, for wells and inset tracks.
    pub const BASE_HI: Color = Color::hex(0x4C3A44);

    /// Cream panel surface.
    pub const PAPER: Color = Color::hex(0xF2E2C4);
    /// Dimmer cream, for alternating rows and disabled fills.
    pub const PAPER_DIM: Color = Color::hex(0xDCC7A2);

    /// Default control face.
    pub const BUTTON: Color = Color::hex(0xE8D0A8);
    /// Top-edge highlight on a raised control.
    pub const BUTTON_HI: Color = Color::hex(0xFFF3DA);
    /// Bottom-edge shading on a raised control.
    pub const BUTTON_LO: Color = Color::hex(0xC2A67C);

    /// Primary accent: warm orange.
    pub const ACCENT: Color = Color::hex(0xE8834A);
    pub const ACCENT_HI: Color = Color::hex(0xFFAB6E);
    pub const ACCENT_LO: Color = Color::hex(0xB35A31);

    /// Cool counterweight so the warm tones have something to sit against.
    pub const TEAL: Color = Color::hex(0x5FA8A0);
    pub const TEAL_HI: Color = Color::hex(0x8AD3C7);

    pub const GREEN: Color = Color::hex(0x8FBF5F);
    pub const YELLOW: Color = Color::hex(0xF0C04A);
    pub const RED: Color = Color::hex(0xD1544C);

    /// Every colour above, in ramp order. Handy for palette swatches.
    pub const ALL: &[(&str, Color)] = &[
        ("VOID", VOID),
        ("INK", INK),
        ("SHADOW", SHADOW),
        ("BASE", BASE),
        ("BASE_HI", BASE_HI),
        ("INK_SOFT", INK_SOFT),
        ("BUTTON_LO", BUTTON_LO),
        ("PAPER_DIM", PAPER_DIM),
        ("BUTTON", BUTTON),
        ("PAPER", PAPER),
        ("BUTTON_HI", BUTTON_HI),
        ("ACCENT_LO", ACCENT_LO),
        ("ACCENT", ACCENT),
        ("ACCENT_HI", ACCENT_HI),
        ("TEAL", TEAL),
        ("TEAL_HI", TEAL_HI),
        ("GREEN", GREEN),
        ("YELLOW", YELLOW),
        ("RED", RED),
    ];
}
