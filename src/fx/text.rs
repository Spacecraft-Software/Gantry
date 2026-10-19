// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// Rust guideline compliant 2026-05-18

//! Reveal and highlight effects for the ASCII-art banner.

use std::fmt;
use std::str::FromStr;

use ratatui::{buffer::Buffer, layout::Rect, style::Color};

use super::{FxPalette, ParseFxError};

/// Characters revealed per tick by [`TextFxKind::Typewriter`].
const CHARS_PER_TICK: u64 = 4;

/// Ticks per beam sweep (3 s at 30 Hz).
const BEAM_PERIOD: u64 = 90;

/// Columns on each side of the beam's center column.
const BEAM_HALF: i64 = 1;

/// Banner reveal/highlight effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextFxKind {
    /// Characters appear in reading order; the newest one is accented.
    #[default]
    Typewriter,
    /// A three-column beam sweeps left to right every three seconds.
    Beams,
    /// Plain text.
    None,
}

impl TextFxKind {
    /// Every text effect, in cycling order.
    pub const ALL: [Self; 3] = [Self::Typewriter, Self::Beams, Self::None];

    /// The next effect in [`TextFxKind::ALL`] order, wrapping around.
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Typewriter => Self::Beams,
            Self::Beams => Self::None,
            Self::None => Self::Typewriter,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Typewriter => "typewriter",
            Self::Beams => "beams",
            Self::None => "none",
        }
    }
}

impl fmt::Display for TextFxKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for TextFxKind {
    type Err = ParseFxError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.name().eq_ignore_ascii_case(s.trim()))
            .ok_or_else(|| ParseFxError::new(s))
    }
}

/// Number of drawable (non-whitespace) characters in `lines`.
fn visible_chars(lines: &[&str]) -> u64 {
    let count: usize = lines
        .iter()
        .map(|l| l.chars().filter(|c| !c.is_whitespace()).count())
        .sum();
    u64::try_from(count).unwrap_or(u64::MAX)
}

/// Beam center relative to the banner's left edge at `tick`.
fn beam_center(tick: u64, block_width: u16) -> i64 {
    let travel = i64::from(block_width) + 2 * (BEAM_HALF + 1);
    let phase = i64::try_from(tick % BEAM_PERIOD).unwrap_or(0);
    let period = i64::try_from(BEAM_PERIOD).unwrap_or(1);
    phase * travel / period - (BEAM_HALF + 1)
}

/// Draw `lines` (already-trimmed banner art, each line one row) centered horizontally in `area`
/// starting at area.y, using `base` as the normal text color and `pal` for highlights.
/// `tick` counts frames since the banner was shown (30 per second).
///
/// Typewriter: reveals characters in reading order at ~4 chars/tick; the most recent char uses
/// `pal.accent`. Beams: all text visible in `base`; a vertical beam 3 columns wide sweeps
/// left→right every ~3 s and recolors the columns it covers (`pal.foreground` in the center,
/// `pal.accent` at the edges). None: draw plainly.
///
/// Never draws outside `area`. Lines wider than `area` are clipped equally on both sides so
/// the middle stays visible. Whitespace is not drawn (the background stays visible). Only
/// the foreground color of drawn cells is set.
pub fn render_banner(
    lines: &[&str],
    area: Rect,
    buf: &mut Buffer,
    base: Color,
    pal: &FxPalette,
    kind: TextFxKind,
    tick: u64,
) {
    let width = usize::from(area.width);
    let revealed = tick.saturating_mul(CHARS_PER_TICK);
    let widest = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    let block_width = u16::try_from(widest.min(width)).unwrap_or(area.width);
    let block_left = i64::from(area.x) + i64::from((area.width - block_width) / 2);
    let beam = beam_center(tick, block_width);
    let mut ordinal: u64 = 0;
    for (i, line) in lines.iter().enumerate() {
        let Some(y) = u16::try_from(i)
            .ok()
            .filter(|&i| i < area.height)
            .and_then(|i| area.y.checked_add(i))
        else {
            break;
        };
        if kind == TextFxKind::Typewriter && ordinal >= revealed {
            break;
        }
        let len = line.chars().count();
        let (skip, left) = if len <= width {
            (0, usize::from(area.x) + (width - len) / 2)
        } else {
            ((len - width) / 2, usize::from(area.x))
        };
        for (j, ch) in line.chars().enumerate() {
            if ch.is_whitespace() {
                continue;
            }
            let k = ordinal;
            ordinal += 1;
            let color = match kind {
                TextFxKind::Typewriter if k >= revealed => break,
                TextFxKind::Typewriter if k + 1 == revealed => pal.accent,
                TextFxKind::Typewriter | TextFxKind::Beams | TextFxKind::None => base,
            };
            if j < skip || j - skip >= width {
                continue;
            }
            let Ok(x) = u16::try_from(left + (j - skip)) else {
                continue;
            };
            let color = if kind == TextFxKind::Beams {
                match (i64::from(x) - block_left - beam).abs() {
                    0 => pal.foreground,
                    d if d <= BEAM_HALF => pal.accent,
                    _ => color,
                }
            } else {
                color
            };
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_char(ch).set_fg(color);
            }
        }
    }
}

/// True once the effect has nothing left to animate (Typewriter fully revealed, including the
/// accent on the last character fading; Beams never; None always).
#[must_use]
pub fn is_settled(lines: &[&str], kind: TextFxKind, tick: u64) -> bool {
    match kind {
        TextFxKind::Typewriter => {
            let total = visible_chars(lines);
            total == 0 || tick.saturating_mul(CHARS_PER_TICK) > total
        }
        TextFxKind::Beams => false,
        TextFxKind::None => true,
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{buffer::Buffer, layout::Rect, style::Color};

    use super::{TextFxKind, is_settled, render_banner};
    use crate::fx::testkit;

    const ART: [&str; 3] = ["╔═╗ GANTRY", "║ ║  login", "╚═╝"];

    fn draw(kind: TextFxKind, tick: u64, area: Rect) -> Buffer {
        let mut buf = Buffer::empty(area);
        render_banner(
            &ART,
            area,
            &mut buf,
            Color::White,
            &testkit::palette(),
            kind,
            tick,
        );
        buf
    }

    fn drawn(buf: &Buffer) -> usize {
        buf.content.iter().filter(|c| c.symbol() != " ").count()
    }

    #[test]
    fn parse_display_next() {
        for kind in TextFxKind::ALL {
            assert_eq!(
                kind.to_string().to_uppercase().parse::<TextFxKind>(),
                Ok(kind)
            );
        }
        let _ = "sparkle".parse::<TextFxKind>().unwrap_err();
        assert_eq!(TextFxKind::None.next(), TextFxKind::Typewriter);
        assert_eq!(TextFxKind::default(), TextFxKind::Typewriter);
    }

    #[test]
    fn typewriter_reveals_progressively() {
        let area = Rect::new(0, 0, 40, 5);
        let total = ART
            .iter()
            .map(|l| l.chars().filter(|c| *c != ' ').count())
            .sum::<usize>();
        assert_eq!(drawn(&draw(TextFxKind::Typewriter, 0, area)), 0);
        let first = draw(TextFxKind::Typewriter, 1, area);
        assert_eq!(drawn(&first), 4);
        assert_eq!(
            first
                .content
                .iter()
                .filter(|c| c.fg == Color::Indexed(11))
                .count(),
            1
        );
        let done = draw(TextFxKind::Typewriter, 1000, area);
        assert_eq!(drawn(&done), total);
        assert!(
            done.content
                .iter()
                .all(|c| c.symbol() == " " || c.fg == Color::White)
        );
        assert_eq!(drawn(&draw(TextFxKind::None, 0, area)), total);
    }

    #[test]
    fn settled_rules() {
        assert!(!is_settled(&ART, TextFxKind::Typewriter, 0));
        assert!(is_settled(&ART, TextFxKind::Typewriter, 1000));
        assert!(is_settled(&[], TextFxKind::Typewriter, 0));
        assert!(!is_settled(&ART, TextFxKind::Beams, 1_000_000));
        assert!(is_settled(&ART, TextFxKind::None, 0));
        // Settled exactly when further ticks stop changing the output.
        let area = Rect::new(0, 0, 40, 5);
        for tick in 0..20 {
            let same = draw(TextFxKind::Typewriter, tick, area)
                == draw(TextFxKind::Typewriter, tick + 1, area);
            assert_eq!(
                is_settled(&ART, TextFxKind::Typewriter, tick),
                same,
                "tick {tick}"
            );
        }
    }

    #[test]
    fn beams_sweep_across() {
        let area = Rect::new(0, 0, 40, 5);
        let lit: Vec<bool> = (0..90)
            .map(|t| {
                draw(TextFxKind::Beams, t, area)
                    .content
                    .iter()
                    .any(|c| c.fg == Color::Indexed(15))
            })
            .collect();
        assert!(lit.iter().any(|&b| b));
        assert!(lit.iter().any(|&b| !b));
        assert_eq!(
            draw(TextFxKind::Beams, 5, area),
            draw(TextFxKind::Beams, 95, area)
        );
    }

    #[test]
    fn clips_without_panicking() {
        for area in [
            Rect::new(0, 0, 5, 1),
            Rect::new(0, 0, 0, 0),
            Rect::new(0, 0, 1, 1),
            Rect::new(3, 2, 5, 1),
            Rect::new(u16::MAX - 2, u16::MAX - 1, 2, 1),
        ] {
            for kind in TextFxKind::ALL {
                for tick in [0, 1, 3, 1000] {
                    let buf = draw(kind, tick, area);
                    assert!(drawn(&buf) <= usize::from(area.width) * usize::from(area.height));
                }
            }
        }
        // Drawing into a buffer smaller than the area must not escape it.
        let mut small = Buffer::empty(Rect::new(0, 0, 3, 1));
        let pal = testkit::palette();
        render_banner(
            &ART,
            Rect::new(0, 0, 40, 5),
            &mut small,
            Color::White,
            &pal,
            TextFxKind::None,
            0,
        );
    }
}
