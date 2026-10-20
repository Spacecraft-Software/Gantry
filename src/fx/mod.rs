// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// Rust guideline compliant 2026-05-18

//! Animated backgrounds, banner text effects, and the big clock font.
//!
//! Everything here is deterministic: effects advance only through
//! [`Background::step`] (the app calls it at 30 Hz), read no clock, and draw
//! only with the colors of an [`FxPalette`].

use std::fmt;
use std::str::FromStr;

use ratatui::{buffer::Buffer, layout::Rect, style::Color};

pub mod clock;
mod fire;
mod matrix;
mod plasma;
pub mod rng;
mod starfield;
pub mod text;

pub use text::TextFxKind;

/// Colors an effect may use, already resolved by the theme layer (RGB in truecolor
/// mode, indexed slots on a Linux VT). `ramp[0]` is the darkest (== background),
/// `ramp[7]` the brightest. Effects must use ONLY these colors (never construct `Color::Rgb`).
#[derive(Debug, Clone)]
pub struct FxPalette {
    /// Eight-step brightness ramp from background (0) to brightest (7).
    pub ramp: [Color; 8],
    /// Screen background color.
    pub background: Color,
    /// Normal text color.
    pub foreground: Color,
    /// Highlight color.
    pub accent: Color,
}

/// Which glyph repertoire is safe. `VtSafe` = only ASCII, and these: █ ▀ ▄ ▌ ▐ ░ ▒ ▓ ·
/// (present in the Linux console's default font). `Unicode` may add others such as ⣿ ✦ ┃ but
/// must degrade correctly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyphs {
    /// ASCII plus the Linux console's block and shade glyphs.
    VtSafe,
    /// Any glyph a modern terminal font is expected to carry.
    Unicode,
    /// `VtSafe` plus half-width katakana, which Gantry loaded into the
    /// console font (see `vtfont`). Only the Matrix rain uses them.
    VtKana,
}

/// An animated full-area background.
pub trait Background: Send + fmt::Debug {
    /// Advance one tick. `area` is the current drawing area; resize internal state when it changes.
    fn step(&mut self, area: Rect);
    /// Draw into `buf` within `area` (set symbol, fg, and bg of cells). Must not panic if `area`
    /// differs from the one last passed to `step` (just clip/skip).
    fn render(&self, area: Rect, buf: &mut Buffer, pal: &FxPalette, glyphs: Glyphs);
}

/// Boxed effects (as returned by [`make`]) are effects too.
impl<B: Background + ?Sized> Background for Box<B> {
    fn step(&mut self, area: Rect) {
        (**self).step(area);
    }

    fn render(&self, area: Rect, buf: &mut Buffer, pal: &FxPalette, glyphs: Glyphs) {
        (**self).render(area, buf, pal, glyphs);
    }
}

/// Error returned when an effect name is not recognized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseFxError {
    input: String,
}

impl ParseFxError {
    fn new(input: &str) -> Self {
        Self {
            input: input.to_owned(),
        }
    }
}

impl fmt::Display for ParseFxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown effect name {:?}", self.input)
    }
}

impl std::error::Error for ParseFxError {}

/// Background effect selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EffectKind {
    /// DOOM-style rising fire.
    Fire,
    /// Falling digital rain.
    Matrix,
    /// Stars flying toward the viewer.
    #[default]
    Starfield,
    /// Classic sine plasma.
    Plasma,
    /// No background effect.
    None,
}

impl EffectKind {
    /// Every effect, in cycling order.
    pub const ALL: [Self; 5] = [
        Self::Fire,
        Self::Matrix,
        Self::Starfield,
        Self::Plasma,
        Self::None,
    ];

    /// The next effect in [`EffectKind::ALL`] order, wrapping around.
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Fire => Self::Matrix,
            Self::Matrix => Self::Starfield,
            Self::Starfield => Self::Plasma,
            Self::Plasma => Self::None,
            Self::None => Self::Fire,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Fire => "fire",
            Self::Matrix => "matrix",
            Self::Starfield => "starfield",
            Self::Plasma => "plasma",
            Self::None => "none",
        }
    }
}

impl fmt::Display for EffectKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for EffectKind {
    type Err = ParseFxError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.name().eq_ignore_ascii_case(s.trim()))
            .ok_or_else(|| ParseFxError::new(s))
    }
}

/// Build an effect, or None for `EffectKind::None`.
#[must_use]
pub fn make(kind: EffectKind, seed: u64) -> Option<Box<dyn Background>> {
    match kind {
        EffectKind::Fire => Some(Box::new(fire::Fire::new(seed))),
        EffectKind::Matrix => Some(Box::new(matrix::Matrix::new(seed))),
        EffectKind::Starfield => Some(Box::new(starfield::Starfield::new(seed))),
        EffectKind::Plasma => Some(Box::new(plasma::Plasma::new(seed))),
        EffectKind::None => None,
    }
}

/// Ramp color `i`, clamped to the brightest step.
fn ramp(pal: &FxPalette, i: usize) -> Color {
    pal.ramp[i.min(pal.ramp.len() - 1)]
}

/// The part of `area` that lies inside `buf`.
fn visible(area: Rect, buf: &Buffer) -> Rect {
    area.intersection(buf.area)
}

/// Shared helpers for the effect tests.
#[cfg(test)]
mod testkit {
    use ratatui::{buffer::Buffer, layout::Rect, style::Color};

    use super::{Background, FxPalette, Glyphs};

    /// Non-ASCII glyphs allowed in `Glyphs::VtSafe` mode.
    const VT_EXTRA: [char; 9] = ['█', '▀', '▄', '▌', '▐', '░', '▒', '▓', '·'];

    /// Area sizes every effect must survive.
    const SIZES: [(u16, u16); 5] = [(0, 0), (1, 1), (7, 3), (80, 24), (300, 100)];

    pub(crate) fn palette() -> FxPalette {
        FxPalette {
            ramp: [0, 1, 2, 3, 4, 5, 6, 7].map(Color::Indexed),
            background: Color::Indexed(0),
            foreground: Color::Indexed(15),
            accent: Color::Indexed(11),
        }
    }

    pub(crate) fn is_vt_safe(symbol: &str) -> bool {
        symbol
            .chars()
            .all(|c| c.is_ascii() || VT_EXTRA.contains(&c))
    }

    /// Steps `fx` `steps` times on `area`, then renders it into a fresh buffer.
    pub(crate) fn snapshot<B: Background>(
        fx: &mut B,
        area: Rect,
        steps: usize,
        glyphs: Glyphs,
    ) -> Buffer {
        for _ in 0..steps {
            fx.step(area);
        }
        let mut buf = Buffer::empty(area);
        fx.render(area, &mut buf, &palette(), glyphs);
        buf
    }

    /// Runs every size, both glyph sets, and mismatched step/render areas.
    pub(crate) fn exercise<B: Background>(make: impl Fn() -> B) {
        let pal = palette();
        for (w, h) in SIZES {
            let area = Rect::new(0, 0, w, h);
            for glyphs in [Glyphs::VtSafe, Glyphs::Unicode] {
                let mut fx = make();
                let _ = snapshot(&mut fx, area, 100, glyphs);
            }
        }
        let mut fx = make();
        for _ in 0..20 {
            fx.step(Rect::new(0, 0, 80, 24));
        }
        for (w, h) in SIZES {
            let area = Rect::new(0, 0, w, h);
            let mut buf = Buffer::empty(area);
            fx.render(area, &mut buf, &pal, Glyphs::Unicode);
        }
        // Area offset from, and larger than, the buffer.
        let mut small = Buffer::empty(Rect::new(0, 0, 10, 5));
        fx.render(Rect::new(3, 2, 200, 60), &mut small, &pal, Glyphs::VtSafe);
        // Area entirely outside the buffer.
        fx.render(Rect::new(50, 50, 5, 5), &mut small, &pal, Glyphs::VtSafe);
        // Shrink and grow between steps.
        for (w, h) in [(300, 100), (1, 1), (0, 0), (40, 12)] {
            fx.step(Rect::new(0, 0, w, h));
            let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
            fx.render(Rect::new(0, 0, 80, 24), &mut buf, &pal, Glyphs::VtSafe);
        }
    }

    /// Asserts two instances built with the same seed render identically.
    pub(crate) fn assert_deterministic<B: Background>(make: impl Fn(u64) -> B) {
        let area = Rect::new(0, 0, 60, 20);
        for seed in [0, 1, 0xDEAD_BEEF] {
            let a = snapshot(&mut make(seed), area, 50, Glyphs::Unicode);
            let b = snapshot(&mut make(seed), area, 50, Glyphs::Unicode);
            assert_eq!(a, b, "seed {seed}");
        }
    }

    /// Asserts a `VtSafe` render on 80x24 uses only VT-safe glyphs.
    pub(crate) fn assert_vt_safe<B: Background>(mut fx: B) {
        let buf = snapshot(&mut fx, Rect::new(0, 0, 80, 24), 120, Glyphs::VtSafe);
        for cell in &buf.content {
            assert!(is_vt_safe(cell.symbol()), "{:?}", cell.symbol());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{EffectKind, make, testkit};

    #[test]
    fn parse_and_display_round_trip() {
        for kind in EffectKind::ALL {
            let text = kind.to_string();
            assert_eq!(text, text.to_lowercase());
            assert_eq!(text.parse::<EffectKind>(), Ok(kind));
            assert_eq!(text.to_uppercase().parse::<EffectKind>(), Ok(kind));
        }
        let err = "lava".parse::<EffectKind>().err();
        assert_eq!(
            err.map(|e| e.to_string()),
            Some("unknown effect name \"lava\"".to_owned())
        );
    }

    #[test]
    fn next_cycles_in_all_order() {
        for (i, kind) in EffectKind::ALL.iter().enumerate() {
            let expected = EffectKind::ALL[(i + 1) % EffectKind::ALL.len()];
            assert_eq!(kind.next(), expected);
        }
        assert_eq!(EffectKind::default(), EffectKind::Starfield);
    }

    #[test]
    fn make_builds_every_effect() {
        for kind in EffectKind::ALL {
            let Some(fx) = make(kind, 3) else {
                assert_eq!(kind, EffectKind::None);
                continue;
            };
            assert_ne!(kind, EffectKind::None);
            let build = |seed| make(kind, seed).expect("effect kind is not None");
            testkit::exercise(|| build(3));
            testkit::assert_deterministic(build);
            testkit::assert_vt_safe(fx);
        }
    }
}
