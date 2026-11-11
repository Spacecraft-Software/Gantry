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
//! Sessions without a file use the built-in banner: the Steelbore OS logo on
//! Steelbore OS, Gantry's own art elsewhere. Art must stay
//! within the Linux console font: ASCII, CP437 box drawing and block glyphs.

use std::path::Path;
use std::sync::OnceLock;

use crate::config::Brand;

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

/// The Steelbore OS logo as half-blocks, made by `tools/logo2art.py`.
const STEELBORE_LOGO: &str = include_str!("../assets/steelbore-logo.txt");

/// "STEELBORE OS" in three rows, for screens too short for the logo.
const STEELBORE_WORDMARK: &str = include_str!("../assets/steelbore-wordmark.txt");

/// Whether an `os-release` file describes Steelbore OS (`ID=steelbore`).
#[must_use]
pub fn is_steelbore_os(os_release: &str) -> bool {
    os_release
        .lines()
        .filter_map(|l| l.trim().strip_prefix("ID="))
        .any(|v| v.trim().trim_matches(['"', '\'']) == "steelbore")
}

/// Whether this host runs Steelbore OS, read once from `/etc/os-release`
/// (`/usr/lib/os-release` as the fallback, per `os-release(5)`).
#[must_use]
pub fn host_is_steelbore() -> bool {
    static HOST: OnceLock<bool> = OnceLock::new();
    *HOST.get_or_init(|| {
        ["/etc/os-release", "/usr/lib/os-release"]
            .iter()
            .find_map(|p| std::fs::read_to_string(p).ok())
            .is_some_and(|text| is_steelbore_os(&text))
    })
}

/// Name shown in place of the logo (and read out in accessible mode).
#[must_use]
pub fn product_name(brand: Brand, host_is_steelbore: bool) -> &'static str {
    match brand {
        Brand::Steelbore => "Steelbore OS",
        Brand::Auto if host_is_steelbore => "Steelbore OS",
        Brand::Auto | Brand::Gantry => "Gantry",
    }
}

impl Banner {
    /// The Steelbore OS logo, then the name on its own.
    #[must_use]
    pub fn steelbore() -> Self {
        Self {
            description: "Steelbore OS: the Steelbore logo, a gear-like emblem above the name"
                .to_owned(),
            variants: vec![STEELBORE_LOGO.to_owned(), STEELBORE_WORDMARK.to_owned()],
        }
    }

    /// The built-in banner for `brand`; `host_is_steelbore` settles `auto`.
    #[must_use]
    pub fn for_brand(brand: Brand, host_is_steelbore: bool) -> Self {
        match brand {
            Brand::Steelbore => Self::steelbore(),
            Brand::Auto if host_is_steelbore => Self::steelbore(),
            Brand::Auto | Brand::Gantry => Self::builtin(),
        }
    }

    /// Gantry's own banners.
    #[must_use]
    pub fn builtin() -> Self {
        Self {
            description: "Gantry: a launch tower beside a rocket on the pad".to_owned(),
            variants: vec![WORDMARK.to_owned(), TOWER.to_owned()],
        }
    }

    /// Loads `<dir>/<key>.toml`, or the built-in banner when absent or invalid.
    #[cfg(test)]
    #[must_use]
    pub fn for_session(dir: Option<&Path>, key: &str) -> Self {
        Self::for_session_or(dir, key, Self::builtin())
    }

    /// Like [`Banner::for_session`], with `fallback` when the session has no art.
    #[must_use]
    pub fn for_session_or(dir: Option<&Path>, key: &str, fallback: Self) -> Self {
        let safe_key = key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
        let Some(dir) = dir.filter(|_| safe_key && !key.starts_with('.')) else {
            return fallback;
        };
        let path = dir.join(format!("{key}.toml"));
        let Ok(text) = std::fs::read_to_string(&path) else {
            return fallback;
        };
        match toml::from_str::<Self>(&text) {
            Ok(b) if !b.variants.is_empty() => b,
            Ok(_) => fallback,
            Err(e) => {
                tracing::warn!(event = "art.invalid", path = %path.display(), error = %e, "banner ignored");
                fallback
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

    /// The variant to draw: `preferred` if it fits in `max_rows`, else the
    /// tallest one that does, else `preferred` (the caller then hides the art).
    #[must_use]
    pub fn fitting(&self, preferred: usize, max_rows: usize) -> usize {
        let n = self.variants.len().max(1);
        let preferred = preferred % n;
        if self.lines(preferred).len() <= max_rows {
            return preferred;
        }
        (0..n)
            .filter(|&i| self.lines(i).len() <= max_rows)
            .max_by_key(|&i| self.lines(i).len())
            .unwrap_or(preferred)
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
    fn os_release_id_is_detected() {
        assert!(is_steelbore_os(
            "NAME=\"Steelbore OS\"\nID=steelbore\nID_LIKE=nixos\n"
        ));
        assert!(is_steelbore_os("ID='steelbore'"));
        assert!(!is_steelbore_os("ID=nixos\nID_LIKE=steelbore\n"));
        assert!(!is_steelbore_os("VARIANT_ID=steelbore\n"));
        assert!(!is_steelbore_os(""));
    }

    #[test]
    fn steelbore_art_is_vt_safe_and_narrow() {
        let b = Banner::steelbore();
        assert_eq!(b.count(), 2);
        for i in 0..b.count() {
            for line in b.lines(i) {
                assert!(line.chars().all(is_vt_safe), "{line:?}");
                assert!(line.chars().count() <= 60, "{line:?}");
            }
        }
        assert!(b.lines(0).len() > b.lines(1).len());
    }

    #[test]
    fn brand_picks_the_banner() {
        assert_eq!(Banner::for_brand(Brand::Auto, true), Banner::steelbore());
        assert_eq!(Banner::for_brand(Brand::Auto, false), Banner::builtin());
        assert_eq!(
            Banner::for_brand(Brand::Steelbore, false),
            Banner::steelbore()
        );
        assert_eq!(Banner::for_brand(Brand::Gantry, true), Banner::builtin());
        assert_eq!(product_name(Brand::Auto, true), "Steelbore OS");
        assert_eq!(product_name(Brand::Gantry, true), "Gantry");
    }

    #[test]
    fn short_screens_get_the_wordmark() {
        let b = Banner::steelbore();
        let tall = b.lines(0).len();
        assert_eq!(b.fitting(0, tall), 0);
        assert_eq!(b.fitting(0, tall - 1), 1);
        assert_eq!(b.fitting(0, 3), 1);
        // Nothing fits: keep the preferred variant; the caller hides the art.
        assert_eq!(b.fitting(0, 1), 0);
    }

    #[test]
    fn session_art_falls_back_to_the_given_banner() {
        let b = Banner::for_session_or(None, "niri", Banner::steelbore());
        assert_eq!(b, Banner::steelbore());
    }

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
