// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// Rust guideline compliant 2026-05-18

//! The DOOM PSX fire spread algorithm, drawn with half blocks.
//!
//! The heat grid has two rows per terminal row; each cell shows the upper
//! sample as the foreground of `▀` and the lower sample as its background.

use ratatui::{buffer::Buffer, layout::Rect, style::Color};

use super::rng::Rng;
use super::{Background, FxPalette, Glyphs, ramp, visible};

/// Lowest source heat, so tiny areas still show a few ramp steps.
const MIN_HEAT: u8 = 8;

/// Highest source heat; the original uses 36 above "off".
const MAX_HEAT: u8 = 36;

/// Rising flames fed from a white-hot bottom row.
#[derive(Debug, Clone)]
pub struct Fire {
    rng: Rng,
    cols: u16,
    rows: u16,
    /// Grid width in samples (== `cols`).
    width: usize,
    /// Grid height in samples (== 2 * `rows`).
    height: usize,
    /// Heat of the source row; flames reach roughly `2 * top` samples up.
    top: u8,
    heat: Vec<u8>,
}

impl Fire {
    /// Creates an empty fire seeded with `seed`.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            rng: Rng::new(seed),
            cols: 0,
            rows: 0,
            width: 0,
            height: 0,
            top: MIN_HEAT,
            heat: Vec::new(),
        }
    }

    fn resize(&mut self, area: Rect) {
        if area.width == self.cols && area.height == self.rows {
            return;
        }
        self.cols = area.width;
        self.rows = area.height;
        self.width = usize::from(area.width);
        self.height = usize::from(area.height) * 2;
        // Each sample row loses 0.5 heat on average, so this keeps the
        // flames to about the lower half of the area.
        self.top = u8::try_from(self.height / 4)
            .unwrap_or(MAX_HEAT)
            .clamp(MIN_HEAT, MAX_HEAT);
        self.heat.clear();
        self.heat.resize(self.width * self.height, 0);
    }

    /// Heat at sample (`col`, `row`), or 0 outside the grid.
    fn at(&self, col: usize, row: usize) -> u8 {
        if col >= self.width || row >= self.height {
            return 0;
        }
        self.heat.get(row * self.width + col).copied().unwrap_or(0)
    }

    /// Ramp step 1..=7 for a nonzero heat.
    fn level(&self, heat: u8) -> usize {
        let step = usize::from(heat.saturating_sub(1)) * 7 / usize::from(self.top.max(1));
        (1 + step).min(7)
    }

    fn color(&self, pal: &FxPalette, heat: u8) -> Color {
        ramp(pal, self.level(heat))
    }
}

impl Background for Fire {
    fn step(&mut self, area: Rect) {
        self.resize(area);
        let (w, h) = (self.width, self.height);
        if w == 0 || h == 0 {
            return;
        }
        let source = (h - 1) * w;
        if let Some(row) = self.heat.get_mut(source..source + w) {
            row.fill(self.top);
        }
        for y in 1..h {
            for x in 0..w {
                let pixel = self.at(x, y);
                let above = (y - 1) * w;
                if pixel == 0 {
                    if let Some(cell) = self.heat.get_mut(above + x) {
                        *cell = 0;
                    }
                    continue;
                }
                // `r` in 0..4 drifts the flame between one column right and
                // two left; its low bit decides whether the heat decays.
                let r = self.rng.below(4);
                let drift = usize::try_from(r).unwrap_or(0);
                let dst = (x + 2 * w + 1 - drift) % w;
                let decay = u8::from(r & 1 == 1);
                if let Some(cell) = self.heat.get_mut(above + dst) {
                    *cell = pixel.saturating_sub(decay);
                }
            }
        }
    }

    fn render(&self, area: Rect, buf: &mut Buffer, pal: &FxPalette, _glyphs: Glyphs) {
        let clip = visible(area, buf);
        for y in clip.top()..clip.bottom() {
            let row = usize::from(y - area.y);
            if row * 2 >= self.height {
                break;
            }
            for x in clip.left()..clip.right() {
                let col = usize::from(x - area.x);
                if col >= self.width {
                    break;
                }
                let upper = self.at(col, row * 2);
                let lower = self.at(col, row * 2 + 1);
                let (symbol, fg, bg) = match (upper, lower) {
                    (0, 0) => continue,
                    (0, l) => ("▄", self.color(pal, l), pal.background),
                    (u, 0) => ("▀", self.color(pal, u), pal.background),
                    (u, l) if self.level(u) == self.level(l) => {
                        let c = self.color(pal, u);
                        ("█", c, c)
                    }
                    (u, l) => ("▀", self.color(pal, u), self.color(pal, l)),
                };
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_symbol(symbol).set_fg(fg).set_bg(bg);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{buffer::Buffer, layout::Rect};

    use super::{Background, Fire};
    use crate::fx::{Glyphs, testkit};

    #[test]
    fn survives_all_sizes() {
        testkit::exercise(|| Fire::new(1));
    }

    #[test]
    fn deterministic() {
        testkit::assert_deterministic(Fire::new);
    }

    #[test]
    fn vt_safe() {
        testkit::assert_vt_safe(Fire::new(2));
    }

    #[test]
    fn bottom_half_burns() {
        let area = Rect::new(0, 0, 40, 20);
        let buf = testkit::snapshot(&mut Fire::new(4), area, 60, Glyphs::VtSafe);
        let lit = (10..20)
            .flat_map(|y| (0..40).map(move |x| (x, y)))
            .filter(|&(x, y)| buf.cell((x, y)).is_some_and(|c| c.symbol() != " "))
            .count();
        assert!(lit > 0);
    }

    #[test]
    fn zero_heat_cells_are_untouched() {
        let area = Rect::new(0, 0, 10, 10);
        let mut fire = Fire::new(9);
        // Resized but never lit: every sample is still zero heat.
        fire.resize(area);
        let mut buf = Buffer::empty(area);
        let before = buf.clone();
        fire.render(area, &mut buf, &testkit::palette(), Glyphs::VtSafe);
        assert_eq!(buf, before);
    }
}
