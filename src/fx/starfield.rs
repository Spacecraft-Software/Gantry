// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// Rust guideline compliant 2026-05-18

//! A 3-D starfield flying toward the viewer.
//!
//! Stars live in a box `x, y ∈ [-1, 1]`, `z ∈ (0, FAR]` and are projected
//! with `screen = center + center * (x / z)`, so far stars cluster near the
//! middle of the area and stream outward as they approach.

use ratatui::{buffer::Buffer, layout::Rect, style::Color};

use super::rng::Rng;
use super::{Background, FxPalette, Glyphs, ramp};

/// Depth at which stars are born.
const FAR: f32 = 4.0;

/// Depth at which stars are recycled.
const NEAR: f32 = 0.05;

/// One star per this many cells.
const CELLS_PER_STAR: usize = 20;

/// Upper bound on the star count, so huge areas stay cheap.
const MAX_STARS: usize = 1000;

#[derive(Debug, Clone, Copy)]
struct Star {
    x: f32,
    y: f32,
    z: f32,
    /// Depth lost per tick.
    speed: f32,
}

/// Perspective starfield.
#[derive(Debug, Clone)]
pub struct Starfield {
    rng: Rng,
    cols: u16,
    rows: u16,
    /// Sorted far to near after every step, so near stars draw on top.
    stars: Vec<Star>,
}

fn spawn(rng: &mut Rng, z: f32) -> Star {
    Star {
        x: rng.unit() * 2.0 - 1.0,
        y: rng.unit() * 2.0 - 1.0,
        z,
        speed: 0.015 + rng.unit() * 0.02,
    }
}

/// Converts a projected coordinate to a cell offset in `0..limit`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "v is checked to be finite and in 0..limit, which fits u16"
)]
fn to_cell(v: f32, limit: u16) -> Option<u16> {
    (v.is_finite() && v >= 0.0 && v < f32::from(limit)).then_some(v as u16)
}

/// Glyph and color for a star at depth fraction `f` (1 = farthest).
fn look(f: f32, pal: &FxPalette, glyphs: Glyphs) -> (char, Color) {
    match f {
        f if f > 0.75 => ('·', ramp(pal, 2)),
        f if f > 0.55 => ('·', ramp(pal, 3)),
        f if f > 0.40 => ('·', ramp(pal, 4)),
        f if f > 0.28 => ('+', ramp(pal, 5)),
        f if f > 0.16 => ('+', ramp(pal, 6)),
        f if f > 0.08 => ('*', ramp(pal, 7)),
        _ => match glyphs {
            Glyphs::Unicode => ('✦', pal.foreground),
            Glyphs::VtSafe | Glyphs::VtKana => ('*', pal.foreground),
        },
    }
}

impl Starfield {
    /// Creates an empty starfield seeded with `seed`.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            rng: Rng::new(seed),
            cols: 0,
            rows: 0,
            stars: Vec::new(),
        }
    }

    fn resize(&mut self, area: Rect) {
        if area.width == self.cols && area.height == self.rows {
            return;
        }
        self.cols = area.width;
        self.rows = area.height;
        let cells = usize::from(self.cols) * usize::from(self.rows);
        let count = (cells / CELLS_PER_STAR).clamp(usize::from(cells > 0), MAX_STARS);
        let rng = &mut self.rng;
        self.stars = (0..count)
            .map(|_| {
                let z = NEAR + rng.unit() * (FAR - NEAR);
                spawn(rng, z)
            })
            .collect();
    }
}

impl Background for Starfield {
    fn step(&mut self, area: Rect) {
        self.resize(area);
        for star in &mut self.stars {
            star.z -= star.speed;
            let gone =
                star.z < NEAR || (star.x / star.z).abs() > 1.0 || (star.y / star.z).abs() > 1.0;
            if gone {
                *star = spawn(&mut self.rng, FAR);
            }
        }
        self.stars.sort_by(|a, b| b.z.total_cmp(&a.z));
    }

    fn render(&self, area: Rect, buf: &mut Buffer, pal: &FxPalette, glyphs: Glyphs) {
        let cx = f32::from(area.width) / 2.0;
        let cy = f32::from(area.height) / 2.0;
        for star in &self.stars {
            let px = cx + cx * (star.x / star.z);
            let py = cy + cy * (star.y / star.z);
            let (Some(col), Some(row)) = (to_cell(px, area.width), to_cell(py, area.height)) else {
                continue;
            };
            let (Some(x), Some(y)) = (area.x.checked_add(col), area.y.checked_add(row)) else {
                continue;
            };
            let (symbol, fg) = look(star.z / FAR, pal, glyphs);
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_char(symbol).set_fg(fg).set_bg(pal.background);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;

    use super::{FAR, Starfield, to_cell};
    use crate::fx::{Glyphs, testkit};

    #[test]
    fn survives_all_sizes() {
        testkit::exercise(|| Starfield::new(1));
    }

    #[test]
    fn deterministic() {
        testkit::assert_deterministic(Starfield::new);
    }

    #[test]
    fn vt_safe() {
        testkit::assert_vt_safe(Starfield::new(2));
    }

    #[test]
    fn to_cell_rejects_out_of_range() {
        assert_eq!(to_cell(-0.1, 10), None);
        assert_eq!(to_cell(10.0, 10), None);
        assert_eq!(to_cell(f32::NAN, 10), None);
        assert_eq!(to_cell(f32::INFINITY, 10), None);
        assert_eq!(to_cell(9.9, 10), Some(9));
        assert_eq!(to_cell(0.0, 0), None);
    }

    #[test]
    fn stars_are_drawn_and_sorted() {
        let mut fx = Starfield::new(3);
        let buf = testkit::snapshot(&mut fx, Rect::new(0, 0, 80, 24), 30, Glyphs::Unicode);
        assert!(buf.content.iter().any(|c| c.symbol() != " "));
        assert!(fx.stars.windows(2).all(|w| w[0].z >= w[1].z));
        assert!(fx.stars.iter().all(|s| s.z > 0.0 && s.z <= FAR));
    }
}
