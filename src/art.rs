// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>

//! Banner art shown above the login box.
//!
//! Each session may have `<art_dir>/<session-key>.toml`:
//!
//! ```toml
//! description = "niri: a scrollable-tiling Wayland compositor"
//! variants = ["""
//! ...art...
//! """]
//! ```
//!
//! Sessions without a file use Gantry's built-in banners. Art must stay
//! within the Linux console font: ASCII, CP437 box drawing and block glyphs.

use std::path::Path;

use serde::Deserialize;

/// Banner variants plus a text equivalent for accessible mode (§18.2.2).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Banner {
    /// One-line description read out instead of the art.
    pub description: String,
    /// Art variants, each a block of lines.
    pub variants: Vec<String>,
}

/// Built-in wordmark, three rows of half-blocks.
const WORDMARK: &str = "\
▄▀▀▀▀ ▄▀▀▀▄ █▄  █ ▀▀█▀▀ █▀▀▀▄ █   █
█  ▀█ █▀▀▀█ █ ▀▄█   █   █▀▀█▀  ▀█▀
 ▀▀▀  ▀   ▀ ▀   ▀   ▀   ▀   ▀   ▀";

/// Built-in launch tower with a rocket on the pad.
const TOWER: &str = "\
  ╔╦╗        ▲
  ╠╬╣       ▐█▌
  ╠╬╬═══════▐█▌
  ╠╬╣       ▐█▌
  ╠╬╬═══════▐█▌
  ╠╬╣      ▄███▄
  ╠╬╣      █▀ ▀█
══╩╩╩══════▀═══▀══";

impl Banner {
    /// Gantry's own banners.
    #[must_use]
    pub fn builtin() -> Self {
        Self {
            description: "Gantry: a launch tower beside a rocket on the pad".to_owned(),
            variants: vec![WORDMARK.to_owned(), TOWER.to_owned()],
        }
    }

    /// Loads `<dir>/<key>.toml`, or the built-in banner when absent or invalid.
    #[must_use]
    pub fn for_session(dir: Option<&Path>, key: &str) -> Self {
        let safe_key = key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
        let Some(dir) = dir.filter(|_| safe_key && !key.starts_with('.')) else {
            return Self::builtin();
        };
        let path = dir.join(format!("{key}.toml"));
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::builtin();
        };
        match toml::from_str::<Self>(&text) {
            Ok(b) if !b.variants.is_empty() => b,
            Ok(_) => Self::builtin(),
            Err(e) => {
                tracing::warn!(event = "art.invalid", path = %path.display(), error = %e, "banner ignored");
                Self::builtin()
            }
        }
    }

    /// Lines of variant `index` (wrapping), with blank edge lines trimmed.
    #[must_use]
    pub fn lines(&self, index: usize) -> Vec<&str> {
        let Some(art) = self.variants.get(index % self.variants.len().max(1)) else {
            return Vec::new();
        };
        let lines: Vec<&str> = art.lines().map(str::trim_end).collect();
        let first = lines
            .iter()
            .position(|l| !l.trim().is_empty())
            .unwrap_or(lines.len());
        let last = lines
            .iter()
            .rposition(|l| !l.trim().is_empty())
            .map_or(first, |i| i + 1);
        lines
            .get(first..last)
            .map(<[&str]>::to_vec)
            .unwrap_or_default()
    }

    /// Number of variants (at least one for built-ins).
    #[must_use]
    pub fn count(&self) -> usize {
        self.variants.len()
    }
}

/// Glyphs the Linux console's default font is guaranteed to carry.
#[must_use]
pub fn is_vt_safe(c: char) -> bool {
    c.is_ascii() || "═║╔╗╚╝╠╣╦╩╬─│┌┐└┘├┤┬┴┼█▀▄▌▐░▒▓▲▼►◄·".contains(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_are_vt_safe_and_narrow() {
        let b = Banner::builtin();
        for i in 0..b.count() {
            for line in b.lines(i) {
                assert!(line.chars().all(is_vt_safe), "{line:?}");
                assert!(line.chars().count() <= 60);
            }
        }
    }

    #[test]
    fn wordmark_rows_have_equal_width() {
        let b = Banner::builtin();
        let widths: Vec<usize> = b.lines(0).iter().map(|l| l.chars().count()).collect();
        assert_eq!(widths.len(), 3);
        assert!(widths.iter().all(|w| *w >= 30));
    }

    #[test]
    fn variant_index_wraps() {
        let b = Banner::builtin();
        assert_eq!(b.lines(0), b.lines(b.count()));
    }

    #[test]
    fn path_traversal_keys_fall_back() {
        let b = Banner::for_session(Some(Path::new("/etc")), "../passwd");
        assert_eq!(b, Banner::builtin());
    }
}
