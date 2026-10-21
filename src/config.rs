// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>

//! `/etc/gantry/config.toml`.
//!
//! Every key has a default, so a missing, partial or malformed file still
//! yields a working greeter (Standard §3.1, graceful degradation). Unknown
//! keys are rejected so a typo is reported rather than silently ignored.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::fx::{EffectKind, TextFxKind};

/// Default location written by the NixOS module.
pub const DEFAULT_PATH: &str = "/etc/gantry/config.toml";

/// Top-level configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// General behavior.
    pub general: General,
    /// Theme selection (§11.6.3 stage-1 source 1).
    pub theme: ThemeSection,
    /// Accessible mode (§18.1 source 3).
    pub accessibility: Accessibility,
    /// Idle screensaver.
    pub screensaver: Screensaver,
    /// Fingerprint prompts from `pam_fprintd`.
    pub fingerprint: Fingerprint,
    /// Power menu commands.
    pub power: Power,
    /// Key bindings.
    pub keys: crate::keys::KeyConfig,
    /// Extra session search directories, searched before the defaults.
    pub session_dirs: Vec<PathBuf>,
    /// Per-session banner art directory.
    pub art_dir: Option<PathBuf>,
}

/// `[general]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "one field per independent config switch"
)]
pub struct General {
    /// Line shown above the login box.
    pub greeting: String,
    /// Command offered first as "Shell" (e.g. a Nushell login).
    pub default_session: Vec<String>,
    /// Remember the last user and session in the cache file.
    pub remember: bool,
    /// Background effect.
    #[serde(with = "effect_kind")]
    pub effect: EffectKind,
    /// Banner text effect.
    #[serde(with = "text_fx_kind")]
    pub text_effect: TextFxKind,
    /// Master switch for all animation.
    pub animations: bool,
    /// Cache file path.
    pub cache_file: PathBuf,
    /// Prefix for X11 sessions (greetd does not start an X server itself).
    pub xsession_wrapper: Vec<String>,
    /// On the Linux console, load a copy of the console font with half-width
    /// katakana while Gantry runs (for the Matrix rain). Off by default.
    pub console_font: bool,
    /// kbd's `setfont`, used to save, patch-load and restore the console font.
    pub setfont: PathBuf,
    /// Show the local date and time, with its UTC offset, on the login screen.
    pub clock: bool,
    /// `strftime` format of that clock (jiff's dialect).
    pub clock_format: String,
}

/// Default login-screen clock: local time with its UTC offset.
pub const DEFAULT_CLOCK_FORMAT: &str = "%a %d %b %Y  %H:%M  UTC%:z";

impl Default for General {
    fn default() -> Self {
        Self {
            xsession_wrapper: vec!["startx".to_owned(), "/usr/bin/env".to_owned()],
            greeting: "Steelbore OS Bravais".to_owned(),
            default_session: vec!["nu".to_owned(), "--login".to_owned()],
            remember: true,
            effect: EffectKind::Starfield,
            text_effect: TextFxKind::Typewriter,
            animations: true,
            cache_file: PathBuf::from("/var/cache/gantry/state.toml"),
            console_font: false,
            setfont: PathBuf::from("setfont"),
            clock: true,
            clock_format: DEFAULT_CLOCK_FORMAT.to_owned(),
        }
    }
}

/// `[theme]`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeSection {
    /// A registered theme slug.
    pub name: Option<String>,
}

/// `[accessibility]`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Accessibility {
    /// Turn accessible mode on from config.
    pub enabled: Option<bool>,
}

/// `[screensaver]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Screensaver {
    /// Enable the idle screensaver.
    pub enabled: bool,
    /// Idle time before it starts, as an ISO 8601 duration (`PT5M`).
    #[serde(with = "iso_duration")]
    pub idle_timeout: Duration,
}

impl Default for Screensaver {
    fn default() -> Self {
        Self {
            enabled: true,
            idle_timeout: Duration::from_secs(5 * 60),
        }
    }
}

/// `[fingerprint]`. Patterns are case-insensitive substrings of PAM messages.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Fingerprint {
    /// Recognize fingerprint prompts and show the scanner.
    pub enabled: bool,
    /// Buffer typed keys during a scan and submit them at the password prompt.
    pub type_ahead: bool,
    /// Messages meaning "scan now".
    pub scanning: Vec<String>,
    /// Messages meaning "no match, try again".
    pub retry: Vec<String>,
    /// Messages meaning "scan window over".
    pub timeout: Vec<String>,
    /// Messages meaning "sensor failed".
    pub failure: Vec<String>,
}

impl Default for Fingerprint {
    fn default() -> Self {
        let v = |items: &[&str]| items.iter().map(|s| (*s).to_owned()).collect();
        // The English strings pam_fprintd sends (fprintd pam/fingerprint-strings.h).
        Self {
            enabled: true,
            type_ahead: true,
            scanning: v(&["place your", "swipe your"]),
            retry: v(&["failed to match"]),
            timeout: v(&["verification timed out"]),
            failure: v(&["unknown error", "protocol error"]),
        }
    }
}

/// `[power]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Power {
    /// Shut-down command.
    pub shutdown: Vec<String>,
    /// Reboot command.
    pub reboot: Vec<String>,
}

impl Default for Power {
    fn default() -> Self {
        let v = |items: &[&str]| items.iter().map(|s| (*s).to_owned()).collect();
        Self {
            shutdown: v(&["systemctl", "poweroff"]),
            reboot: v(&["systemctl", "reboot"]),
        }
    }
}

impl Config {
    /// Loads `path`. A missing file gives the defaults; a malformed file
    /// gives the defaults plus a warning string for the caller to show.
    #[must_use]
    pub fn load(path: &Path) -> (Self, Option<String>) {
        match std::fs::read_to_string(path) {
            Ok(text) => match toml::from_str(&text) {
                Ok(cfg) => (cfg, None),
                Err(e) => (
                    Self::default(),
                    Some(format!("config {} ignored: {e}", path.display())),
                ),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Self::default(), None),
            Err(e) => (
                Self::default(),
                Some(format!("config {} unreadable: {e}", path.display())),
            ),
        }
    }
}

/// Parses an ISO 8601 duration such as `PT5M` or `PT90S`.
///
/// # Errors
/// Returns a message when the text is not an ISO 8601 duration or is negative.
pub fn parse_iso_duration(text: &str) -> Result<Duration, String> {
    let span: jiff::SignedDuration = text.parse().map_err(|e| format!("{text}: {e}"))?;
    Duration::try_from(span).map_err(|e| format!("{text}: {e}"))
}

mod iso_duration {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer};

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let text = String::deserialize(d)?;
        super::parse_iso_duration(&text).map_err(serde::de::Error::custom)
    }
}

mod effect_kind {
    use serde::{Deserialize, Deserializer};

    use crate::fx::EffectKind;

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<EffectKind, D::Error> {
        let text = String::deserialize(d)?;
        text.parse()
            .map_err(|_ignored: _| serde::de::Error::custom(format!("unknown effect {text:?}")))
    }
}

mod text_fx_kind {
    use serde::{Deserialize, Deserializer};

    use crate::fx::TextFxKind;

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<TextFxKind, D::Error> {
        let text = String::deserialize(d)?;
        text.parse().map_err(|_ignored: _| {
            serde::de::Error::custom(format!("unknown text effect {text:?}"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_gives_defaults() {
        let cfg: Config = toml::from_str("").expect("empty config parses");
        assert!(cfg.general.remember);
        assert_eq!(cfg.screensaver.idle_timeout, Duration::from_secs(300));
    }

    #[test]
    fn partial_file_keeps_other_defaults() {
        let cfg: Config = toml::from_str(
            "[general]\neffect = \"fire\"\n[screensaver]\nidle_timeout = \"PT90S\"\n",
        )
        .expect("partial config parses");
        assert_eq!(cfg.general.effect, EffectKind::Fire);
        assert_eq!(cfg.screensaver.idle_timeout, Duration::from_secs(90));
        assert_eq!(cfg.general.greeting, "Steelbore OS Bravais");
    }

    #[test]
    fn clock_is_on_with_the_utc_offset() {
        let cfg = Config::default();
        assert!(cfg.general.clock);
        assert!(cfg.general.clock_format.contains("UTC%:z"));
        let off: Config = toml::from_str("[general]\nclock = false\n").expect("parses");
        assert!(!off.general.clock);
    }

    #[test]
    fn console_font_is_opt_in() {
        assert!(!Config::default().general.console_font);
        let cfg: Config = toml::from_str(
            "[general]\nconsole_font = true\nsetfont = \"/run/current-system/sw/bin/setfont\"\n",
        )
        .expect("parses");
        assert!(cfg.general.console_font);
        assert_eq!(
            cfg.general.setfont,
            PathBuf::from("/run/current-system/sw/bin/setfont")
        );
    }

    #[test]
    fn typo_is_rejected() {
        let err = toml::from_str::<Config>("[general]\ngreting = \"x\"\n").unwrap_err();
        assert!(err.to_string().contains("greting"));
    }

    #[test]
    fn negative_duration_is_rejected() {
        let err = parse_iso_duration("-PT5M").unwrap_err();
        assert!(err.contains("-PT5M"));
    }

    #[test]
    fn missing_file_is_silent() {
        let (cfg, warning) = Config::load(Path::new("/nonexistent/gantry.toml"));
        assert!(warning.is_none());
        assert!(cfg.fingerprint.enabled);
    }
}
