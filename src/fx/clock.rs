// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// Rust guideline compliant 2026-05-18

//! A three-row half-block font for the big clock.
//!
//! Each terminal row holds two pixel rows (`▀` upper, `▄` lower, `█` both),
//! so glyphs are 3 columns by 6 pixels; digits use the top five pixels.

/// The three rows of a blank digit-width (3-column) cell.
const BLANK: [&str; 3] = ["   ", "   ", "   "];

/// Rows for one character.
fn glyph(ch: char) -> [&'static str; 3] {
    match ch {
        '0' => ["█▀█", "█ █", "▀▀▀"],
        '1' => ["▄█ ", " █ ", "▀▀▀"],
        '2' => ["▀▀█", "█▀▀", "▀▀▀"],
        '3' => ["▀▀█", " ▀█", "▀▀▀"],
        '4' => ["█ █", "▀▀█", "  ▀"],
        '5' => ["█▀▀", "▀▀█", "▀▀▀"],
        '6' => ["█▀▀", "█▀█", "▀▀▀"],
        '7' => ["▀▀█", "  █", "  ▀"],
        '8' => ["█▀█", "█▀█", "▀▀▀"],
        '9' => ["█▀█", "▀▀█", "▀▀▀"],
        ':' => ["▄", "▄", " "],
        '-' => ["   ", "▀▀▀", "   "],
        '+' => [" ▄ ", "▀█▀", "   "],
        _ => BLANK,
    }
}

/// Render `text` (digits 0-9, ':', ' ', '-', '+') as large block glyphs, 3 rows tall,
/// using only `VtSafe` glyphs (█ ▀ ▄ and space). Each digit is 3 columns wide plus 1 column gap;
/// ':' is 1 column wide. Unknown chars render as blank of digit width. Return the rows.
///
/// The gap is placed between glyphs only (no trailing gap), and all three rows always have
/// the same width in characters.
#[must_use]
pub fn big_text(text: &str) -> [String; 3] {
    let mut rows = [String::new(), String::new(), String::new()];
    for (i, ch) in text.chars().enumerate() {
        for (row, part) in rows.iter_mut().zip(glyph(ch)) {
            if i > 0 {
                row.push(' ');
            }
            row.push_str(part);
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::{big_text, glyph};

    /// Width of a digit (and of blanks and unknown characters).
    const DIGIT_WIDTH: usize = 3;

    fn width(s: &str) -> usize {
        s.chars().count()
    }

    #[test]
    fn digits_are_uniform_and_distinct() {
        let digits: Vec<[&str; 3]> = ('0'..='9').map(glyph).collect();
        for d in &digits {
            assert!(d.iter().all(|row| width(row) == DIGIT_WIDTH), "{d:?}");
        }
        for (i, a) in digits.iter().enumerate() {
            for b in &digits[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn only_block_glyphs() {
        let rows = big_text("0123456789:-+ ?");
        for row in &rows {
            assert!(
                row.chars().all(|c| matches!(c, '█' | '▀' | '▄' | ' ')),
                "{row}"
            );
        }
    }

    #[test]
    fn layout_widths() {
        assert_eq!(big_text(""), [String::new(), String::new(), String::new()]);
        let clock = big_text("12:34");
        // Four digits, one colon, four gaps.
        let expected = 4 * DIGIT_WIDTH + 1 + 4;
        assert!(clock.iter().all(|r| width(r) == expected), "{clock:?}");
        let odd = big_text("x 1");
        assert!(odd.iter().all(|r| width(r) == 3 * DIGIT_WIDTH + 2));
        assert_eq!(width(&big_text("?")[0]), DIGIT_WIDTH);
    }

    #[test]
    fn renders_known_digit() {
        assert_eq!(big_text("0"), ["█▀█", "█ █", "▀▀▀"].map(String::from));
    }
}
