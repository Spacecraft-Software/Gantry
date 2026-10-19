// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// Rust guideline compliant 2026-05-18

//! Falling digital-rain columns.
//!
//! Each column carries one drop. The head is drawn in `foreground`, the next
//! two cells in `success`, and the rest of the trail fades from `ramp[5]`
//! down to `ramp[1]`. Glyphs are ASCII plus `·` in both glyph modes.

use ratatui::{buffer::Buffer, layout::Rect, style::Color};

use super::rng::Rng;
use super::{Background, FxPalette, Glyphs, ramp, visible};

/// ASCII rain glyphs. Indices in `GLYPHS.len()..GLYPH_SPAN` draw as `·`.
const GLYPHS: &[u8] =
    b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz@#$%&*+=-<>?!/\\|:;";

/// Number of distinct glyph indices; the excess over `GLYPHS` maps to `·`.
const GLYPH_SPAN: u32 = 84;

/// Shortest trail, in cells (head plus two `success` cells plus fade).
const MIN_LEN: i32 = 4;

/// One falling drop.
#[derive(Debug, Clone, Copy)]
struct Drop {
    /// Row of the head; negative while waiting above the top edge.
    head: i32,
    /// Trail length including the head.
    len: i32,
    /// Ticks per one-row move (1 = fastest).
    period: u8,
    /// Ticks since the last move.
    wait: u8,
}

/// Matrix-style digital rain.
#[derive(Debug, Clone)]
pub struct Matrix {
    rng: Rng,
    cols: u16,
    rows: u16,
    /// Glyph index per cell, row-major.
    glyphs: Vec<u8>,
    /// One drop per column.
    drops: Vec<Drop>,
}

/// Random `i32` in `0..n`.
fn rand_i32(rng: &mut Rng, n: u32) -> i32 {
    i32::try_from(rng.below(n)).unwrap_or(0)
}

fn random_glyph(rng: &mut Rng) -> u8 {
    u8::try_from(rng.below(GLYPH_SPAN)).unwrap_or(0)
}

fn glyph_char(index: u8) -> char {
    GLYPHS
        .get(usize::from(index))
        .map_or('·', |&b| char::from(b))
}

/// A new drop above the top edge; `initial` staggers drops over a taller span.
fn spawn(rng: &mut Rng, rows: u16, initial: bool) -> Drop {
    let rows = u32::from(rows);
    let delay_span = if initial { rows * 2 + 1 } else { rows / 2 + 1 };
    Drop {
        head: -rand_i32(rng, delay_span),
        len: MIN_LEN + rand_i32(rng, rows / 2 + 4),
        period: 1 + u8::try_from(rng.below(3)).unwrap_or(0),
        wait: 0,
    }
}

impl Matrix {
    /// Creates an empty rain field seeded with `seed`.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            rng: Rng::new(seed),
            cols: 0,
            rows: 0,
            glyphs: Vec::new(),
            drops: Vec::new(),
        }
    }

    fn resize(&mut self, area: Rect) {
        if area.width == self.cols && area.height == self.rows {
            return;
        }
        self.cols = area.width;
        self.rows = area.height;
        let cells = usize::from(self.cols) * usize::from(self.rows);
        let rng = &mut self.rng;
        self.glyphs = (0..cells).map(|_| random_glyph(rng)).collect();
        let rows = self.rows;
        self.drops = (0..self.cols).map(|_| spawn(rng, rows, true)).collect();
    }

    /// Trail color at `dist` cells behind the head of a drop of length `len`.
    fn trail_color(pal: &FxPalette, dist: i32, len: i32) -> Color {
        match dist {
            0 => pal.foreground,
            1 | 2 => pal.success,
            _ => {
                let span = (len - 3).max(1);
                let step = 5 - (dist - 3) * 5 / span;
                ramp(pal, usize::try_from(step.clamp(1, 5)).unwrap_or(1))
            }
        }
    }
}

impl Background for Matrix {
    fn step(&mut self, area: Rect) {
        self.resize(area);
        let rows = self.rows;
        let bottom = i32::from(rows);
        for drop in &mut self.drops {
            drop.wait = drop.wait.saturating_add(1);
            if drop.wait >= drop.period {
                drop.wait = 0;
                drop.head += 1;
            }
            if drop.head - drop.len >= bottom {
                *drop = spawn(&mut self.rng, rows, false);
            }
        }
        // Flicker a few glyphs each tick.
        let cells = u32::try_from(self.glyphs.len()).unwrap_or(u32::MAX);
        if cells > 0 {
            for _ in 0..=(cells / 40) {
                let i = usize::try_from(self.rng.below(cells)).unwrap_or(0);
                let g = random_glyph(&mut self.rng);
                if let Some(slot) = self.glyphs.get_mut(i) {
                    *slot = g;
                }
            }
        }
    }

    fn render(&self, area: Rect, buf: &mut Buffer, pal: &FxPalette, _glyphs: Glyphs) {
        let clip = visible(area, buf);
        let cols = usize::from(self.cols);
        for x in clip.left()..clip.right() {
            let col = x - area.x;
            let Some(drop) = self.drops.get(usize::from(col)) else {
                break;
            };
            for y in clip.top()..clip.bottom() {
                let row = y - area.y;
                if row >= self.rows {
                    break;
                }
                let dist = drop.head - i32::from(row);
                if !(0..drop.len).contains(&dist) {
                    continue;
                }
                let index = usize::from(row) * cols + usize::from(col);
                let Some(&g) = self.glyphs.get(index) else {
                    continue;
                };
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(glyph_char(g))
                        .set_fg(Self::trail_color(pal, dist, drop.len))
                        .set_bg(pal.background);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;

    use super::{GLYPH_SPAN, GLYPHS, Matrix};
    use crate::fx::{Glyphs, testkit};

    #[test]
    fn survives_all_sizes() {
        testkit::exercise(|| Matrix::new(1));
    }

    #[test]
    fn deterministic() {
        testkit::assert_deterministic(Matrix::new);
    }

    #[test]
    fn vt_safe() {
        testkit::assert_vt_safe(Matrix::new(2));
    }

    #[test]
    fn glyph_table_leaves_room_for_dots() {
        assert!(GLYPHS.is_ascii());
        assert!(GLYPHS.len() < usize::try_from(GLYPH_SPAN).unwrap_or(0));
    }

    #[test]
    fn rain_appears_and_uses_head_color() {
        let area = Rect::new(0, 0, 40, 20);
        let pal = testkit::palette();
        let buf = testkit::snapshot(&mut Matrix::new(5), area, 80, Glyphs::VtSafe);
        let drawn = buf.content.iter().filter(|c| c.symbol() != " ").count();
        assert!(drawn > 0);
        assert!(buf.content.iter().any(|c| c.fg == pal.foreground));
    }
}
