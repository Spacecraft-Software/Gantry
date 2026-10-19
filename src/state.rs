// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>

//! Small remembered state in `/var/cache/gantry/state.toml`.
//!
//! Holds no secrets: the last user name, session key, and the theme and
//! effect picked from the settings menu. Writes go to a temporary file that
//! is renamed into place, so a power cut never leaves a torn file. Any read
//! or write failure is logged and otherwise ignored.

use std::io::Write as _;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// What Gantry remembers between boots.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    /// Last user who logged in.
    pub user: Option<String>,
    /// Key of the last session started.
    pub session: Option<String>,
    /// Theme picked in the settings menu.
    pub theme: Option<String>,
    /// Background effect picked in the settings menu.
    pub effect: Option<String>,
}

impl State {
    /// Reads `path`, giving the empty state on any problem.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default()
    }

    /// Writes `path` atomically.
    pub fn save(&self, path: &Path) {
        if let Err(e) = self.try_save(path) {
            tracing::warn!(event = "state.save_failed", path = %path.display(), error = %e, "could not remember state");
        }
    }

    fn try_save(&self, path: &Path) -> anyhow::Result<()> {
        let text = toml::to_string(self)?;
        let dir = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("state path has no parent"))?;
        std::fs::create_dir_all(dir)?;
        let tmp = path.with_extension("toml.tmp");
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let dir = std::env::temp_dir().join(format!("gantry-state-{}", std::process::id()));
        let path = dir.join("state.toml");
        let s = State {
            user: Some("mj".into()),
            session: Some("niri".into()),
            ..State::default()
        };
        s.save(&path);
        assert_eq!(State::load(&path), s);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn garbage_gives_empty_state() {
        let dir = std::env::temp_dir().join(format!("gantry-state-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("state.toml");
        std::fs::write(&path, "user = [[[").expect("write");
        assert_eq!(State::load(&path), State::default());
        let _ = std::fs::remove_dir_all(dir);
    }
}
