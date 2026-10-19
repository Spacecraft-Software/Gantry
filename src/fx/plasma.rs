// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// Rust guideline compliant 2026-05-18

//! Classic sine plasma shaded with `░▒▓█`.
//!
//! Each cell's value is quantized to 28 levels: the ramp step pair
//! (`ramp[band]` as background, `ramp[band + 1]` as foreground) and one of
//! four shade glyphs that blends between them.

use std::f32::consts::TAU;

use ratatui::{buffer::Buffer, layout::Rect};

use super::{Background, FxPalette, Glyphs, ramp, visible};

/// Ticks per full animation cycle (24 s at 30 Hz). All phase terms use
/// integer multiples of the base angle, so the wrap is seamless.
const PERIOD: u16 = 720;

/// Shade glyphs from least to most foreground coverage.
const SHADES: [&str; 4] = ["░", "▒", "▓", "█"];

/// Seven ramp bands times four shades.
const LEVELS: u8 = 28;

/// Sine plasma.
#[derive(Debug, Clone)]
pub struct Plasma {
    tick: u16,
    cols: u16,
    rows: u16,
    /// Quantized level per cell, row-major; reused across frames.
    levels: Vec<u8>,
}

/// Quantizes a plasma sum in `[-4, 4]` to `0..LEVELS`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is clamped to 0..LEVELS before the cast"
)]
fn quantize(sum: f32) -> u8 {
    let max = f32::from(LEVELS - 1);
    let scaled = (sum + 4.0) / 8.0 * f32::from(LEVELS);
    if scaled.is_nan() {
        return 0;
    }
    scaled.clamp(0.0, max) as u8
}

impl Plasma {
    /// Creates a plasma whose phase starts at an offset derived from `seed`.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        let tick = u16::try_from(seed % u64::from(PERIOD)).unwrap_or(0);
        Self {
            tick,
            cols: 0,
            rows: 0,
            levels: Vec::new(),
        }
    }

    fn resize(&mut self, area: Rect) {
        if area.width == self.cols && area.height == self.rows {
            return;
        }
        self.cols = area.width;
        self.rows = area.height;
        self.levels.clear();
        self.levels
            .resize(usize::from(self.cols) * usize::from(self.rows), 0);
    }
}

impl Background for Plasma {
    fn step(&mut self, area: Rect) {
        self.resize(area);
        self.tick = (self.tick + 1) % PERIOD;
        let a = TAU * f32::from(self.tick) / f32::from(PERIOD);
        let w = f32::from(self.cols);
        // Cells are about twice as tall as wide; double y to keep blobs round.
        let h = f32::from(self.rows) * 2.0;
        let (cx, cy) = (w * (0.5 + 0.3 * a.sin()), h * (0.5 + 0.3 * (2.0 * a).cos()));
        let mut slots = self.levels.iter_mut();
        for row in 0..self.rows {
            let fy = f32::from(row) * 2.0;
            let wave_y = (fy * 0.11 - 2.0 * a).sin();
            for col in 0..self.cols {
                let fx = f32::from(col);
                let (dx, dy) = (fx - cx, fy - cy);
                let sum = (fx * 0.16 + a).sin()
                    + wave_y
                    + ((fx + fy) * 0.08 + 3.0 * a).sin()
                    + ((dx * dx + dy * dy).sqrt() * 0.18 - 2.0 * a).sin();
                if let Some(slot) = slots.next() {
                    *slot = quantize(sum);
                }
            }
        }
    }

    fn render(&self, area: Rect, buf: &mut Buffer, pal: &FxPalette, _glyphs: Glyphs) {
        let clip = visible(area, buf);
        let cols = usize::from(self.cols);
        for y in clip.top()..clip.bottom() {
            let row = y - area.y;
            if row >= self.rows {
                break;
            }
            for x in clip.left()..clip.right() {
                let col = x - area.x;
                if col >= self.cols {
                    break;
                }
                let Some(&level) = self.levels.get(usize::from(row) * cols + usize::from(col))
                else {
                    continue;
                };
                let band = usize::from(level / 4);
                let shade = SHADES[usize::from(level % 4)];
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_symbol(shade)
                        .set_fg(ramp(pal, band + 1))
                        .set_bg(ramp(pal, band));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;

    use super::{LEVELS, Plasma, quantize};
    use crate::fx::{Background, testkit};

    #[test]
    fn survives_all_sizes() {
        testkit::exercise(|| Plasma::new(1));
    }

    #[test]
    fn deterministic() {
        testkit::assert_deterministic(Plasma::new);
    }

    #[test]
    fn vt_safe() {
        testkit::assert_vt_safe(Plasma::new(2));
    }

    #[test]
    fn quantize_stays_in_range() {
        for sum in [-10.0, -4.0, 0.0, 3.999, 4.0, 10.0, f32::NAN, f32::INFINITY] {
            assert!(quantize(sum) < LEVELS, "{sum}");
        }
        assert_eq!(quantize(-4.0), 0);
        assert_eq!(quantize(4.0), LEVELS - 1);
    }

    #[test]
    fn buffer_is_reused_across_frames() {
        let area = Rect::new(0, 0, 80, 24);
        let mut fx = Plasma::new(0);
        fx.step(area);
        let ptr = fx.levels.as_ptr();
        let cap = fx.levels.capacity();
        for _ in 0..100 {
            fx.step(area);
        }
        assert_eq!(fx.levels.as_ptr(), ptr);
        assert_eq!(fx.levels.capacity(), cap);
        assert!(fx.levels.iter().any(|&l| l != fx.levels[0]));
    }
}
