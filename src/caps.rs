// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>

//! Caps Lock detection.
//!
//! The Linux console has no keyboard protocol that reports lock state, and
//! an ioctl would need `unsafe`. The kernel already exposes each keyboard's
//! Caps Lock LED as a world-readable sysfs file, which works the same on a
//! VT and under a kiosk compositor, so Gantry reads that.

use std::path::Path;

const LEDS: &str = "/sys/class/leds";

/// True when any keyboard's Caps Lock LED is lit. Unknown counts as off.
#[must_use]
pub fn is_on() -> bool {
    is_on_in(Path::new(LEDS))
}

fn is_on_in(root: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    entries.filter_map(Result::ok).any(|e| {
        e.file_name().to_string_lossy().ends_with("::capslock")
            && std::fs::read_to_string(e.path().join("brightness")).is_ok_and(|b| b.trim() != "0")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_led_brightness() {
        let root = std::env::temp_dir().join(format!("gantry-leds-{}", std::process::id()));
        let led = root.join("input3::capslock");
        std::fs::create_dir_all(&led).expect("dir");
        std::fs::write(led.join("brightness"), "0\n").expect("write");
        assert!(!is_on_in(&root));
        std::fs::write(led.join("brightness"), "1\n").expect("write");
        assert!(is_on_in(&root));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn missing_sysfs_is_off() {
        assert!(!is_on_in(Path::new("/nonexistent/leds")));
    }
}
