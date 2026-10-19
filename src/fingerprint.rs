// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>

//! Recognizes `pam_fprintd` messages so the UI can show a scanner.
//!
//! Gantry never touches the sensor: PAM inside greetd does the matching and
//! alone grants a login. This module only classifies the text PAM relays.

use crate::config::Fingerprint;

/// What a PAM message means for the scanner widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// Put a finger on the reader now.
    Scanning,
    /// That scan did not match; another try is coming.
    Retry,
    /// The scan window closed; a password prompt follows.
    TimedOut,
    /// The sensor or daemon failed; a password prompt follows.
    Failure,
}

/// Classifies `text` with the configured patterns (case-insensitive substrings).
#[must_use]
pub fn classify(cfg: &Fingerprint, text: &str) -> Option<Signal> {
    if !cfg.enabled {
        return None;
    }
    let lower = text.to_lowercase();
    let hit = |pats: &[String]| {
        pats.iter()
            .any(|p| !p.is_empty() && lower.contains(&p.to_lowercase()))
    };
    // Order matters: failure/timeout strings are checked before the broad "place your".
    if hit(&cfg.failure) {
        Some(Signal::Failure)
    } else if hit(&cfg.timeout) {
        Some(Signal::TimedOut)
    } else if hit(&cfg.retry) {
        Some(Signal::Retry)
    } else if hit(&cfg.scanning) {
        Some(Signal::Scanning)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pam_fprintd_strings_classify() {
        let cfg = Fingerprint::default();
        let cases = [
            (
                "Place your right index finger on the fingerprint reader",
                Some(Signal::Scanning),
            ),
            (
                "Swipe your left thumb across Synaptics Sensor",
                Some(Signal::Scanning),
            ),
            ("Failed to match fingerprint", Some(Signal::Retry)),
            ("Verification timed out", Some(Signal::TimedOut)),
            ("An unknown error occurred", Some(Signal::Failure)),
            ("Password:", None),
        ];
        for (text, want) in cases {
            assert_eq!(classify(&cfg, text), want, "{text}");
        }
    }

    #[test]
    fn disabled_classifies_nothing() {
        let cfg = Fingerprint {
            enabled: false,
            ..Fingerprint::default()
        };
        assert_eq!(classify(&cfg, "Place your finger"), None);
    }
}
