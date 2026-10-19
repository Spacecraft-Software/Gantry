// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>

//! Remappable key bindings (Standard §10).
//!
//! Each action has a CUA chord and a Vim-style chord by default. A
//! `[keys]` entry replaces that action's whole list. Chords are written as
//! `Ctrl+j`, `Alt+s`, `Shift+Tab`, `F2`, `PageUp`, `Enter`, `Esc`.

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::Deserialize;

/// Something a key can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    /// Move focus to the next field.
    NextField,
    /// Move focus to the previous field.
    PrevField,
    /// Submit the focused field or menu choice.
    Submit,
    /// Close a menu, clear an error, or note "use password" during a scan.
    Back,
    /// Open the session picker.
    SessionMenu,
    /// Open the settings menu (theme, effect).
    SettingsMenu,
    /// Open the power menu.
    PowerMenu,
    /// Previous banner variant.
    ArtPrev,
    /// Next banner variant.
    ArtNext,
    /// Leave the full-screen UI for linear accessible mode.
    AccessibleMode,
    /// Move up in a menu.
    Up,
    /// Move down in a menu.
    Down,
}

impl Action {
    /// Every action, in config-key order.
    pub const ALL: [Self; 12] = [
        Self::NextField,
        Self::PrevField,
        Self::Submit,
        Self::Back,
        Self::SessionMenu,
        Self::SettingsMenu,
        Self::PowerMenu,
        Self::ArtPrev,
        Self::ArtNext,
        Self::AccessibleMode,
        Self::Up,
        Self::Down,
    ];

    /// Built-in chords: CUA first, Vim-style second.
    fn defaults(self) -> &'static [&'static str] {
        match self {
            Self::NextField => &["Tab", "Ctrl+j"],
            Self::PrevField => &["Shift+Tab", "BackTab", "Ctrl+k"],
            Self::Submit => &["Enter"],
            Self::Back => &["Esc"],
            Self::SessionMenu => &["F2", "Alt+s"],
            Self::SettingsMenu => &["F1", "Alt+t"],
            Self::PowerMenu => &["F12", "Alt+p"],
            Self::ArtPrev => &["PageUp", "Alt+h"],
            Self::ArtNext => &["PageDown", "Alt+l"],
            Self::AccessibleMode => &["F5", "Alt+a"],
            Self::Up => &["Up", "Ctrl+p"],
            Self::Down => &["Down", "Ctrl+n"],
        }
    }

    fn config_key(self) -> &'static str {
        match self {
            Self::NextField => "next_field",
            Self::PrevField => "prev_field",
            Self::Submit => "submit",
            Self::Back => "back",
            Self::SessionMenu => "session_menu",
            Self::SettingsMenu => "settings_menu",
            Self::PowerMenu => "power_menu",
            Self::ArtPrev => "art_prev",
            Self::ArtNext => "art_next",
            Self::AccessibleMode => "accessible_mode",
            Self::Up => "up",
            Self::Down => "down",
        }
    }
}

/// `[keys]`: action name → list of chords.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(transparent)]
pub struct KeyConfig(pub HashMap<String, Vec<String>>);

/// A parsed chord.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Chord {
    code: KeyCode,
    mods: KeyModifiers,
}

impl Chord {
    /// Normalizes a terminal key event into a chord.
    #[must_use]
    pub fn from_event(ev: &KeyEvent) -> Self {
        let mut mods =
            ev.modifiers & (KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT);
        let code = match ev.code {
            // Shift is implied by the character itself.
            KeyCode::Char(c) => {
                mods.remove(KeyModifiers::SHIFT);
                KeyCode::Char(c.to_ascii_lowercase())
            }
            KeyCode::BackTab => {
                mods.insert(KeyModifiers::SHIFT);
                KeyCode::Tab
            }
            other => other,
        };
        Self { code, mods }
    }
}

/// Error for an unparseable chord.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChordError(String);

impl fmt::Display for ChordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown key chord {:?}", self.0)
    }
}

impl std::error::Error for ChordError {}

impl FromStr for Chord {
    type Err = ChordError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ChordError(s.to_owned());
        let mut mods = KeyModifiers::NONE;
        let mut parts: Vec<&str> = s.split('+').collect();
        let key = parts.pop().filter(|k| !k.is_empty()).ok_or_else(err)?;
        for m in parts {
            match m.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => mods.insert(KeyModifiers::CONTROL),
                "alt" | "meta" => mods.insert(KeyModifiers::ALT),
                "shift" => mods.insert(KeyModifiers::SHIFT),
                _ => return Err(err()),
            }
        }
        let lower = key.to_ascii_lowercase();
        let code = match lower.as_str() {
            "tab" => KeyCode::Tab,
            "backtab" => {
                mods.insert(KeyModifiers::SHIFT);
                KeyCode::Tab
            }
            "enter" | "return" => KeyCode::Enter,
            "esc" | "escape" => KeyCode::Esc,
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "pageup" | "pgup" => KeyCode::PageUp,
            "pagedown" | "pgdn" => KeyCode::PageDown,
            "home" => KeyCode::Home,
            "end" => KeyCode::End,
            "space" => KeyCode::Char(' '),
            f if f.starts_with('f') && f.len() > 1 => {
                let n: u8 = f.get(1..).and_then(|d| d.parse().ok()).ok_or_else(err)?;
                if !(1..=24).contains(&n) {
                    return Err(err());
                }
                KeyCode::F(n)
            }
            single => {
                let mut chars = single.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => KeyCode::Char(c),
                    _ => return Err(err()),
                }
            }
        };
        if let KeyCode::Char(_) = code {
            mods.remove(KeyModifiers::SHIFT);
        }
        Ok(Self { code, mods })
    }
}

/// The resolved chord → action table.
#[derive(Debug, Clone)]
pub struct Keymap {
    table: HashMap<Chord, Action>,
}

impl Keymap {
    /// Builds the map from defaults plus overrides. Returns warnings for bad
    /// entries; those entries keep their defaults.
    #[must_use]
    pub fn new(cfg: &KeyConfig) -> (Self, Vec<String>) {
        let mut warnings = Vec::new();
        for key in cfg.0.keys() {
            if !Action::ALL.iter().any(|a| a.config_key() == key) {
                warnings.push(format!("unknown key action {key:?}"));
            }
        }
        let mut table = HashMap::new();
        for action in Action::ALL {
            let chords: Vec<Chord> = match cfg.0.get(action.config_key()) {
                Some(list) => {
                    let parsed: Result<Vec<Chord>, _> = list.iter().map(|s| s.parse()).collect();
                    match parsed {
                        Ok(v) => v,
                        Err(e) => {
                            warnings.push(e.to_string());
                            parse_defaults(action)
                        }
                    }
                }
                None => parse_defaults(action),
            };
            for chord in chords {
                table.insert(chord, action);
            }
        }
        (Self { table }, warnings)
    }

    /// Looks up the action for a key event.
    #[must_use]
    pub fn action(&self, ev: &KeyEvent) -> Option<Action> {
        self.table.get(&Chord::from_event(ev)).copied()
    }
}

fn parse_defaults(action: Action) -> Vec<Chord> {
    action
        .defaults()
        .iter()
        .filter_map(|s| s.parse().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn every_default_chord_parses() {
        for action in Action::ALL {
            for chord in action.defaults() {
                assert!(chord.parse::<Chord>().is_ok(), "{chord} parses");
            }
        }
    }

    #[test]
    fn cua_and_vim_defaults_both_work() {
        let (map, warnings) = Keymap::new(&KeyConfig::default());
        assert!(warnings.is_empty());
        assert_eq!(
            map.action(&ev(KeyCode::F(2), KeyModifiers::NONE)),
            Some(Action::SessionMenu)
        );
        assert_eq!(
            map.action(&ev(KeyCode::Char('s'), KeyModifiers::ALT)),
            Some(Action::SessionMenu)
        );
        assert_eq!(
            map.action(&ev(KeyCode::BackTab, KeyModifiers::SHIFT)),
            Some(Action::PrevField)
        );
        assert_eq!(
            map.action(&ev(KeyCode::Char('x'), KeyModifiers::NONE)),
            None
        );
    }

    #[test]
    fn override_replaces_the_list() {
        let mut cfg = KeyConfig::default();
        cfg.0.insert("power_menu".into(), vec!["Ctrl+q".into()]);
        let (map, warnings) = Keymap::new(&cfg);
        assert!(warnings.is_empty());
        assert_eq!(
            map.action(&ev(KeyCode::Char('q'), KeyModifiers::CONTROL)),
            Some(Action::PowerMenu)
        );
        assert_eq!(map.action(&ev(KeyCode::F(12), KeyModifiers::NONE)), None);
    }

    #[test]
    fn bad_override_warns_and_keeps_default() {
        let mut cfg = KeyConfig::default();
        cfg.0.insert("power_menu".into(), vec!["Hyper+x".into()]);
        cfg.0.insert("launch_rocket".into(), vec!["F9".into()]);
        let (map, warnings) = Keymap::new(&cfg);
        assert_eq!(warnings.len(), 2);
        assert_eq!(
            map.action(&ev(KeyCode::F(12), KeyModifiers::NONE)),
            Some(Action::PowerMenu)
        );
    }
}
