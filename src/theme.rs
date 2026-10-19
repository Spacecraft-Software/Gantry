// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>

//! Theme registry and resolution (Steelbore Standard §11, §11.6).
//!
//! Colour values are never retyped: they are read at start-up from Gantry's
//! verbatim copy of the palette skill's `steelbore.toml`, embedded at build
//! time. This module resolves which registered theme renders, then turns its
//! eleven role tokens into ratatui colours for the active [`ColorMode`].
//!
//! On a Linux virtual console (`TERM=linux`) the kernel only knows sixteen
//! colours, so Gantry reprograms those sixteen palette slots to the theme
//! (see [`vt_palette_sequence`]) and addresses colours by slot number.

use std::collections::HashMap;
use std::fmt;
use std::fmt::Write as _;

use ratatui::style::Color;

use crate::fx::FxPalette;

/// The canonical palette file, embedded verbatim (Standard §11.4).
const PALETTE_TOML: &str = include_str!("../assets/steelbore.toml");

/// The palette-independent mono theme slug (§11.1.1).
pub const MONO: &str = "steelbore-mono";

/// The project default palette (§11.4): Steelbore Modern.
pub const DEFAULT_THEME: &str = "steelbore";

/// Suffix of every accessible-mode sibling (§11.1.1).
const HIGH_CONTRAST_SUFFIX: &str = "-high-contrast";

/// A 24-bit colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// Parses `#RRGGBB`.
    fn parse(hex: &str) -> Option<Self> {
        let digits = hex.strip_prefix('#')?;
        if digits.len() != 6 || !digits.is_ascii() {
            return None;
        }
        let channel = |i: usize| u8::from_str_radix(digits.get(i..i + 2)?, 16).ok();
        Some(Self(channel(0)?, channel(2)?, channel(4)?))
    }

    /// Linear blend from `self` to `other`; `t` is clamped to `0.0..=1.0`.
    fn mix(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let lerp = |a: u8, b: u8| {
            let v = f32::from(a) + (f32::from(b) - f32::from(a)) * t;
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "v is a blend of two u8 values, so it lies in 0..=255"
            )]
            let byte = v.round() as u8;
            byte
        };
        Self(
            lerp(self.0, other.0),
            lerp(self.1, other.1),
            lerp(self.2, other.2),
        )
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02X}{:02X}{:02X}", self.0, self.1, self.2)
    }
}

/// One role token: an exact colour, or (mono theme only) a terminal-palette name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    /// An exact colour from the palette file.
    Rgb(Rgb),
    /// A 4-bit ANSI name, used only by `steelbore-mono`.
    Ansi(Color),
    /// Reverse video (mono `focus`).
    Reverse,
}

impl Token {
    fn parse(value: &str) -> Option<Self> {
        if let Some(rgb) = Rgb::parse(value) {
            return Some(Self::Rgb(rgb));
        }
        let named = match value {
            "default" => Color::Reset,
            "bright-white" => Color::White,
            "white" => Color::Gray,
            "blue" => Color::Blue,
            "green" => Color::Green,
            "red" => Color::Red,
            "yellow" => Color::Yellow,
            "reverse-video" => return Some(Self::Reverse),
            _ => return None,
        };
        Some(Self::Ansi(named))
    }
}

/// The eleven §11.1 role tokens of one theme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeDef {
    /// Registered slug, e.g. `steelbore-high-contrast`.
    pub slug: String,
    /// Canvas.
    pub background: Token,
    /// Fill for panels.
    pub surface: Token,
    /// Alternate fill.
    pub surface_alt: Token,
    /// Body text.
    pub foreground: Token,
    /// Brand accent.
    pub accent: Token,
    /// Structural lines and headings.
    pub structure: Token,
    /// Success status.
    pub success: Token,
    /// Error status.
    pub error: Token,
    /// Warning status.
    pub warning: Token,
    /// Keyboard focus.
    pub focus: Token,
    /// Borders.
    pub border: Token,
}

/// Every theme the palette file defines, plus its registration lists.
#[derive(Debug, Clone)]
pub struct Registry {
    themes: HashMap<String, ThemeDef>,
    registered: Vec<String>,
    fidelity: Vec<String>,
}

impl Registry {
    /// Loads the embedded palette file.
    ///
    /// # Errors
    /// Fails only if the embedded file is malformed, which the test suite rules out.
    pub fn embedded() -> anyhow::Result<Self> {
        Self::parse(PALETTE_TOML)
    }

    fn parse(text: &str) -> anyhow::Result<Self> {
        let root: toml::Table = toml::from_str(text)?;
        let meta = root
            .get("meta")
            .and_then(toml::Value::as_table)
            .ok_or_else(|| anyhow::anyhow!("palette file has no [meta]"))?;
        let list = |key: &str| -> Vec<String> {
            meta.get(key)
                .and_then(toml::Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default()
        };
        let registered = list("registered-set");
        let fidelity = list("fidelity-palettes");

        let mut themes = HashMap::new();
        if let Some(table) = root.get("themes").and_then(toml::Value::as_table) {
            for (slug, value) in table {
                if let Some(def) = value.as_table().and_then(|t| theme_from_table(slug, t)) {
                    themes.insert(slug.clone(), def);
                }
            }
        }
        anyhow::ensure!(
            themes.contains_key(DEFAULT_THEME),
            "palette file lacks the default theme"
        );
        Ok(Self {
            themes,
            registered,
            fidelity,
        })
    }

    /// Looks up a theme by slug.
    #[must_use]
    pub fn get(&self, slug: &str) -> Option<&ThemeDef> {
        self.themes.get(slug)
    }

    /// The §11.6.1 registered slugs, in file order.
    #[must_use]
    pub fn registered(&self) -> &[String] {
        &self.registered
    }

    fn is_usable(&self, slug: &str, source: BaseSource) -> bool {
        let known = self.themes.contains_key(slug);
        let registered = self.registered.iter().any(|s| s == slug);
        let fidelity = self.fidelity.iter().any(|s| s == slug);
        known && (registered || (fidelity && source == BaseSource::InApp))
    }
}

fn theme_from_table(slug: &str, t: &toml::Table) -> Option<ThemeDef> {
    let token = |key: &str| {
        t.get(key)
            .and_then(toml::Value::as_str)
            .and_then(Token::parse)
    };
    let background = token("background")?;
    let foreground = token("foreground")?;
    // Mono and similar themes may omit surfaces; they fall back to the canvas.
    let surface = token("surface").unwrap_or(background);
    let surface_alt = token("surface-alt").unwrap_or(surface);
    let structure = token("structure")?;
    Some(ThemeDef {
        slug: slug.to_owned(),
        background,
        surface,
        surface_alt,
        foreground,
        accent: token("accent")?,
        structure,
        success: token("success")?,
        error: token("error")?,
        warning: token("warning")?,
        focus: token("focus")?,
        border: token("border").unwrap_or(structure),
    })
}

/// Which stage-1 source picked the base palette (§11.6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseSource {
    /// `--theme` or `[theme] name`.
    InApp,
    /// `SPACECRAFT_THEME`.
    Environment,
    /// `/etc/steelbore/theme.toml`.
    SystemDeclaration,
    /// Nothing else applied.
    ProjectDefault,
}

impl fmt::Display for BaseSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InApp => "in-app selection",
            Self::Environment => "SPACECRAFT_THEME",
            Self::SystemDeclaration => "system declaration",
            Self::ProjectDefault => "project default",
        })
    }
}

/// Which stage-2 signal chose the rendered sibling (§11.6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    /// The user named a variant slug directly.
    Pinned,
    /// `NO_COLOR` selected mono.
    NoColor,
    /// §18.1 accessible mode selected the high-contrast sibling.
    Accessible,
    /// The system declaration asked for high contrast.
    PlatformHighContrast,
    /// No overlay.
    None,
}

impl fmt::Display for Overlay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Pinned => "pinned variant",
            Self::NoColor => "NO_COLOR",
            Self::Accessible => "accessible mode",
            Self::PlatformHighContrast => "platform high contrast",
            Self::None => "none",
        })
    }
}

/// Inputs to theme resolution, gathered by the caller.
#[derive(Debug, Clone, Default)]
pub struct ThemeInputs {
    /// `--theme`.
    pub cli: Option<String>,
    /// `[theme] name` from Gantry's config.
    pub config: Option<String>,
    /// `SPACECRAFT_THEME`.
    pub env: Option<String>,
    /// `active` from `/etc/steelbore/theme.toml`.
    pub system_active: Option<String>,
    /// `high-contrast` from `/etc/steelbore/theme.toml`.
    pub system_high_contrast: bool,
    /// `NO_COLOR` is set and non-empty.
    pub no_color: bool,
    /// §18.1 accessible mode is on.
    pub accessible: bool,
}

impl ThemeInputs {
    /// Reads the environment and the system declaration file.
    #[must_use]
    pub fn from_system(cli: Option<String>, config: Option<String>, accessible: bool) -> Self {
        let env = std::env::var("SPACECRAFT_THEME")
            .ok()
            .filter(|s| !s.is_empty());
        let no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
        let (system_active, system_high_contrast) = read_system_declaration();
        Self {
            cli,
            config,
            env,
            system_active,
            system_high_contrast,
            no_color,
            accessible,
        }
    }
}

fn read_system_declaration() -> (Option<String>, bool) {
    let Ok(text) = std::fs::read_to_string("/etc/steelbore/theme.toml") else {
        return (None, false);
    };
    let Ok(root) = toml::from_str::<toml::Table>(&text) else {
        tracing::warn!(
            event = "theme.system_declaration_invalid",
            "ignoring malformed theme declaration"
        );
        return (None, false);
    };
    let theme = root.get("theme").and_then(toml::Value::as_table);
    let active = theme
        .and_then(|t| t.get("active"))
        .and_then(toml::Value::as_str)
        .map(str::to_owned);
    let high_contrast = theme
        .and_then(|t| t.get("high-contrast"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(false);
    (active, high_contrast)
}

/// Outcome of resolution, reported under `--verbose`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// The base palette chosen at stage 1.
    pub base: String,
    /// The stage-1 source that decided it.
    pub base_source: BaseSource,
    /// The slug that renders after stage 2.
    pub slug: String,
    /// The stage-2 signal that applied.
    pub overlay: Overlay,
}

/// Strips a variant suffix, giving the base palette slug.
fn base_of(slug: &str) -> &str {
    if slug == MONO {
        return DEFAULT_THEME;
    }
    slug.strip_suffix(HIGH_CONTRAST_SUFFIX).unwrap_or(slug)
}

fn is_variant(slug: &str) -> bool {
    slug == MONO || slug.ends_with(HIGH_CONTRAST_SUFFIX)
}

/// Two-stage resolution (§11.6.3). An unusable slug is skipped, never fatal.
#[must_use]
pub fn resolve(registry: &Registry, inputs: &ThemeInputs) -> Resolved {
    let candidates = [
        (inputs.cli.as_deref(), BaseSource::InApp),
        (inputs.config.as_deref(), BaseSource::InApp),
        (inputs.env.as_deref(), BaseSource::Environment),
        (
            inputs.system_active.as_deref(),
            BaseSource::SystemDeclaration,
        ),
    ];
    let picked = candidates.into_iter().find_map(|(slug, source)| {
        let slug = slug?.trim();
        if registry.is_usable(slug, source) {
            Some((slug.to_owned(), source))
        } else {
            tracing::warn!(event = "theme.slug_skipped", slug, source = %source, "unusable theme slug skipped");
            None
        }
    });
    let (chosen, base_source) =
        picked.unwrap_or_else(|| (DEFAULT_THEME.to_owned(), BaseSource::ProjectDefault));

    let pinned =
        is_variant(&chosen) && matches!(base_source, BaseSource::InApp | BaseSource::Environment);
    let base = base_of(&chosen).to_owned();
    let high_contrast = format!("{base}{HIGH_CONTRAST_SUFFIX}");
    let has_high_contrast = registry.get(&high_contrast).is_some();

    let (slug, overlay) = if pinned {
        (chosen, Overlay::Pinned)
    } else if inputs.no_color {
        (MONO.to_owned(), Overlay::NoColor)
    } else if inputs.accessible && has_high_contrast {
        (high_contrast, Overlay::Accessible)
    } else if inputs.system_high_contrast && has_high_contrast {
        (high_contrast, Overlay::PlatformHighContrast)
    } else {
        (base.clone(), Overlay::None)
    };
    Resolved {
        base,
        base_source,
        slug,
        overlay,
    }
}

/// How colours reach the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    /// 24-bit SGR (Alacritty, foot, kitty under kiosk mode).
    TrueColor,
    /// Linux virtual console: sixteen reprogrammed slots, eight usable as backgrounds.
    LinuxVt,
}

impl ColorMode {
    /// Detects the mode from `TERM` and `COLORTERM`.
    #[must_use]
    pub fn detect() -> Self {
        match std::env::var("TERM").as_deref() {
            Ok("linux") => Self::LinuxVt,
            _ => Self::TrueColor,
        }
    }
}

/// Fixed VT slot for each role. Slots 0–7 are the only ones the Linux console
/// can paint as a background, so every role that may fill a cell lives there.
mod slot {
    pub const BACKGROUND: u8 = 0;
    pub const ERROR: u8 = 1;
    pub const SUCCESS: u8 = 2;
    pub const WARNING: u8 = 3;
    pub const STRUCTURE: u8 = 4;
    pub const ACCENT: u8 = 5;
    pub const SURFACE: u8 = 6;
    pub const FOREGROUND: u8 = 7;
    pub const SURFACE_ALT: u8 = 8;
    /// Ramp steps 1..=7 occupy slots 9..=15 (step 0 is the background).
    pub const RAMP_START: u8 = 9;
}

/// Resolved ratatui colours for every role, ready to draw with.
#[derive(Debug, Clone)]
pub struct Styles {
    /// Slug actually rendering.
    pub slug: String,
    /// Active colour mode.
    pub mode: ColorMode,
    /// Canvas.
    pub background: Color,
    /// Panel fill.
    pub surface: Color,
    /// Body text.
    pub foreground: Color,
    /// Brand accent.
    pub accent: Color,
    /// Structure and headings.
    pub structure: Color,
    /// Success.
    pub success: Color,
    /// Error.
    pub error: Color,
    /// Warning.
    pub warning: Color,
    /// Focus colour; `None` means reverse video (mono).
    pub focus: Option<Color>,
    /// Borders.
    pub border: Color,
    /// Colours handed to the animation layer.
    pub fx: FxPalette,
    /// Sixteen RGB values to load into a Linux VT, when in that mode with an RGB theme.
    pub vt_slots: Option<[Rgb; 16]>,
}

impl Styles {
    /// Builds styles for `def` in `mode`.
    #[must_use]
    pub fn new(def: &ThemeDef, mode: ColorMode) -> Self {
        let rgb = |t: Token| match t {
            Token::Rgb(c) => Some(c),
            Token::Ansi(_) | Token::Reverse => None,
        };
        let roles = (rgb(def.background), rgb(def.accent), rgb(def.foreground));
        // A theme is "exact" when its core roles carry RGB values (all but mono).
        let ramp_rgb = match roles {
            (Some(bg), Some(accent), Some(fg)) => Some(ramp(bg, accent, fg)),
            _ => None,
        };

        let pick = |t: Token, vt_slot: u8| -> Color {
            match (t, mode) {
                (Token::Rgb(_), ColorMode::LinuxVt) => Color::Indexed(vt_slot),
                (Token::Rgb(Rgb(r, g, b)), ColorMode::TrueColor) => Color::Rgb(r, g, b),
                (Token::Ansi(c), _) => c,
                (Token::Reverse, _) => Color::Reset,
            }
        };

        let background = pick(def.background, slot::BACKGROUND);
        let accent = pick(def.accent, slot::ACCENT);
        let foreground = pick(def.foreground, slot::FOREGROUND);
        let structure = pick(def.structure, slot::STRUCTURE);
        let success = pick(def.success, slot::SUCCESS);
        let error = pick(def.error, slot::ERROR);
        let warning = pick(def.warning, slot::WARNING);

        let ramp_colors: [Color; 8] = match (ramp_rgb, mode) {
            (Some(r), ColorMode::TrueColor) => r.map(|Rgb(a, b, c)| Color::Rgb(a, b, c)),
            (Some(_), ColorMode::LinuxVt) => std::array::from_fn(|i| {
                if i == 0 {
                    Color::Indexed(slot::BACKGROUND)
                } else {
                    // i is 1..=7, so the slot is 9..=15.
                    Color::Indexed(slot::RAMP_START + u8::try_from(i - 1).unwrap_or(0))
                }
            }),
            (None, _) => [
                Color::Reset,
                Color::DarkGray,
                Color::DarkGray,
                Color::Gray,
                Color::Gray,
                Color::White,
                Color::White,
                Color::White,
            ],
        };

        let vt_slots = match (mode, ramp_rgb) {
            (ColorMode::LinuxVt, Some(r)) => vt_slot_table(def, &r),
            _ => None,
        };

        let focus = match def.focus {
            Token::Reverse => None,
            t => Some(pick(t, slot::SUCCESS)),
        };
        Self {
            slug: def.slug.clone(),
            mode,
            background,
            surface: pick(def.surface, slot::SURFACE),
            foreground,
            accent,
            structure,
            success,
            error,
            warning,
            focus,
            border: pick(def.border, slot::STRUCTURE),
            fx: FxPalette {
                ramp: ramp_colors,
                background,
                foreground,
                accent,
                success,
            },
            vt_slots,
        }
    }
}

/// Eight decorative steps from canvas through accent to foreground. These are
/// blends of the theme's own tokens, used only to fill animation cells, never
/// for text, so no contrast pairing depends on them.
fn ramp(bg: Rgb, accent: Rgb, fg: Rgb) -> [Rgb; 8] {
    std::array::from_fn(|i| {
        #[expect(clippy::cast_precision_loss, reason = "i is 0..8")]
        let step = i as f32;
        if i <= 4 {
            bg.mix(accent, step / 4.0)
        } else {
            accent.mix(fg, (step - 4.0) / 3.0)
        }
    })
}

/// Maps roles onto the sixteen VT slots. Roles that alias another role
/// (`focus` = `success`, `border` = `structure`) share its slot; a theme whose
/// aliases differ falls back to its own `success`/`structure` colour, which
/// every registered theme satisfies today.
fn vt_slot_table(def: &ThemeDef, ramp: &[Rgb; 8]) -> Option<[Rgb; 16]> {
    let rgb = |t: Token| match t {
        Token::Rgb(c) => Some(c),
        Token::Ansi(_) | Token::Reverse => None,
    };
    let mut table = [Rgb(0, 0, 0); 16];
    table[usize::from(slot::BACKGROUND)] = rgb(def.background)?;
    table[usize::from(slot::ERROR)] = rgb(def.error)?;
    table[usize::from(slot::SUCCESS)] = rgb(def.success)?;
    table[usize::from(slot::WARNING)] = rgb(def.warning)?;
    table[usize::from(slot::STRUCTURE)] = rgb(def.structure)?;
    table[usize::from(slot::ACCENT)] = rgb(def.accent)?;
    table[usize::from(slot::SURFACE)] = rgb(def.surface)?;
    table[usize::from(slot::FOREGROUND)] = rgb(def.foreground)?;
    table[usize::from(slot::SURFACE_ALT)] = rgb(def.surface_alt)?;
    for (i, step) in ramp.iter().enumerate().skip(1) {
        table[usize::from(slot::RAMP_START) + i - 1] = *step;
    }
    Some(table)
}

/// Linux-console escape that loads `slots` into palette entries 0–15
/// (`ESC ] P n rrggbb`, see `console_codes(4)`).
#[must_use]
pub fn vt_palette_sequence(slots: &[Rgb; 16]) -> String {
    let mut out = String::with_capacity(16 * 10);
    for (i, Rgb(r, g, b)) in slots.iter().enumerate() {
        let _ = write!(out, "\x1b]P{i:x}{r:02x}{g:02x}{b:02x}");
    }
    out
}

/// Linux-console escape that restores the kernel's default palette.
pub const VT_PALETTE_RESET: &str = "\x1b]R";

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> Registry {
        Registry::embedded().expect("embedded palette parses")
    }

    #[test]
    fn embedded_palette_registers_twenty_one_themes() {
        let reg = registry();
        assert_eq!(reg.registered().len(), 21);
        for slug in reg.registered() {
            assert!(
                reg.get(slug).is_some(),
                "registered theme {slug} has a definition"
            );
        }
    }

    #[test]
    fn default_theme_values_are_read_from_the_file() {
        let reg = registry();
        let def = reg.get(DEFAULT_THEME).expect("default exists");
        assert_eq!(def.background, Token::Rgb(Rgb(0x00, 0x00, 0x27)));
        assert_eq!(def.accent, Token::Rgb(Rgb(0xFF, 0x5E, 0x00)));
    }

    #[test]
    fn silence_gives_the_project_default() {
        let r = resolve(&registry(), &ThemeInputs::default());
        assert_eq!(r.slug, DEFAULT_THEME);
        assert_eq!(r.base_source, BaseSource::ProjectDefault);
        assert_eq!(r.overlay, Overlay::None);
    }

    #[test]
    fn unknown_slug_falls_through() {
        let inputs = ThemeInputs {
            cli: Some("no-such-theme".into()),
            env: Some("tokyonight".into()),
            ..ThemeInputs::default()
        };
        let r = resolve(&registry(), &inputs);
        assert_eq!(r.slug, "tokyonight");
        assert_eq!(r.base_source, BaseSource::Environment);
    }

    #[test]
    fn fidelity_palette_only_at_source_one() {
        let reg = registry();
        let system = ThemeInputs {
            system_active: Some("solarized-dark".into()),
            ..ThemeInputs::default()
        };
        assert_eq!(resolve(&reg, &system).slug, DEFAULT_THEME);
        let cli = ThemeInputs {
            cli: Some("solarized-dark".into()),
            ..ThemeInputs::default()
        };
        assert_eq!(resolve(&reg, &cli).slug, "solarized-dark");
    }

    #[test]
    fn overlays_follow_precedence() {
        let reg = registry();
        let accessible = ThemeInputs {
            accessible: true,
            ..ThemeInputs::default()
        };
        let r = resolve(&reg, &accessible);
        assert_eq!(r.slug, "steelbore-high-contrast");
        assert_eq!(r.overlay, Overlay::Accessible);

        let both = ThemeInputs {
            accessible: true,
            no_color: true,
            ..ThemeInputs::default()
        };
        assert_eq!(resolve(&reg, &both).slug, MONO);

        let pinned = ThemeInputs {
            cli: Some("steelbore-blue-high-contrast".into()),
            no_color: true,
            ..ThemeInputs::default()
        };
        let r = resolve(&reg, &pinned);
        assert_eq!(r.slug, "steelbore-blue-high-contrast");
        assert_eq!(r.base, "steelbore-blue");
        assert_eq!(r.overlay, Overlay::Pinned);
    }

    #[test]
    fn vt_mode_uses_slots_and_loads_the_palette() {
        let reg = registry();
        let styles = Styles::new(reg.get(DEFAULT_THEME).expect("default"), ColorMode::LinuxVt);
        assert_eq!(styles.background, Color::Indexed(0));
        let slots = styles.vt_slots.expect("rgb theme yields vt slots");
        assert_eq!(slots[0], Rgb(0x00, 0x00, 0x27));
        let seq = vt_palette_sequence(&slots);
        assert!(seq.starts_with("\x1b]P0000027"));
        assert_eq!(seq.matches("\x1b]P").count(), 16);
    }

    #[test]
    fn mono_has_no_vt_slots_and_reverse_focus() {
        let reg = registry();
        let styles = Styles::new(reg.get(MONO).expect("mono"), ColorMode::LinuxVt);
        assert!(styles.vt_slots.is_none());
        assert!(styles.focus.is_none());
    }

    #[test]
    fn ramp_runs_from_canvas_to_foreground() {
        let r = ramp(Rgb(0, 0, 0), Rgb(200, 100, 0), Rgb(255, 255, 255));
        assert_eq!(r[0], Rgb(0, 0, 0));
        assert_eq!(r[4], Rgb(200, 100, 0));
        assert_eq!(r[7], Rgb(255, 255, 255));
    }
}
