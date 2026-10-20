// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>

//! The greeter's state machine.
//!
//! [`App`] is pure state plus transitions: it receives key events, frame
//! ticks and [`Event`]s from the auth worker, and emits [`Command`]s. It does
//! no terminal I/O, so every transition is unit-testable.

use std::sync::mpsc::Sender;
use std::time::Instant;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use zeroize::Zeroizing;

use crate::art::Banner;
use crate::auth::{Command, Event, MessageKind, Touch};
use crate::config::Config;
use crate::fingerprint::{self, Signal};
use crate::fx::{self, Background, EffectKind, TextFxKind};
use crate::keys::{Action, Keymap};
use crate::sessions::{Session, User};
use crate::state::State;
use crate::theme::{self, ColorMode, Registry, Styles, ThemeInputs, Tint};

/// Frames per second of the UI loop.
pub const FPS: u64 = 30;

/// Status severity. Always rendered with a text tag, never colour alone (§18.2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// `[INFO]`.
    Info,
    /// `[OK]`.
    Ok,
    /// `[WARN]`.
    Warn,
    /// `[ERROR]`.
    Error,
}

impl Level {
    /// Text tag for the status line.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Self::Info => "[INFO]",
            Self::Ok => "[OK]",
            Self::Warn => "[WARN]",
            Self::Error => "[ERROR]",
        }
    }
}

/// Which input has focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// User name.
    User,
    /// Answer to the PAM prompt.
    Answer,
}

/// Fingerprint widget state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scan {
    /// Failed matches so far in this attempt.
    pub misses: u8,
    /// Tick when the current state began (drives the animation).
    pub since: u64,
    /// The user pressed Back to ask for the password instead.
    pub wants_password: bool,
}

/// Where the login conversation stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// Editing the user name.
    Idle,
    /// Waiting for greetd's first reply.
    Waiting,
    /// PAM asked a question.
    Prompt {
        /// Prompt text.
        text: String,
        /// Hide the answer.
        secret: bool,
    },
    /// `pam_fprintd` is waiting for a finger.
    Scanning(Scan),
    /// An answer was sent.
    Checking,
    /// Authenticated; the session is being started.
    Starting,
}

/// A modal panel over the login box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    /// Session picker; the value is the highlighted row.
    Sessions(usize),
    /// Power menu.
    Power(usize),
    /// Settings menu.
    Settings(usize),
    /// Idle screensaver.
    Screensaver,
}

/// Power menu rows.
pub const POWER_ITEMS: [&str; 3] = ["Shut down", "Reboot", "Cancel"];
/// Settings menu rows.
pub const SETTINGS_ITEMS: [&str; 4] = ["Theme", "Background effect", "Banner effect", "Close"];

/// What the main loop must do after a transition.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Effects {
    /// Load the new theme's palette into the VT.
    pub reload_palette: bool,
    /// Leave the TUI and continue in linear accessible mode.
    pub go_linear: bool,
    /// Run this power command after leaving the TUI.
    pub power: Option<Vec<String>>,
}

/// The whole greeter state.
#[derive(Debug)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent flags, not a state machine in disguise"
)]
pub struct App {
    /// Effective configuration.
    pub cfg: Config,
    keymap: Keymap,
    registry: Registry,
    theme_inputs: ThemeInputs,
    mode: ColorMode,
    /// Active colours.
    pub styles: Styles,
    /// Startable sessions, "Shell" first.
    pub sessions: Vec<Session>,
    /// Selected session index.
    pub session: usize,
    /// Known users for the picker.
    pub users: Vec<User>,
    /// User name being typed.
    pub username: String,
    /// Answer being typed (wiped on drop and on every reset).
    pub answer: Zeroizing<String>,
    /// Focused field.
    pub focus: Focus,
    /// Login phase.
    pub phase: Phase,
    /// Open panel, if any.
    pub panel: Option<Panel>,
    /// Status line.
    pub status: Option<(Level, String)>,
    /// Consecutive denied attempts.
    pub failures: u32,
    /// Caps Lock is on.
    pub caps_lock: bool,
    /// Banner for the selected session.
    pub banner: Banner,
    /// Selected banner variant.
    pub variant: usize,
    /// Tick when the banner last changed (restarts its text effect).
    pub banner_since: u64,
    /// Background effect kind.
    pub effect_kind: EffectKind,
    /// Running background effect.
    pub effect: Option<Box<dyn Background>>,
    /// Banner text effect.
    pub text_fx: TextFxKind,
    /// Frame counter.
    pub tick: u64,
    /// `--test` mode (no real power commands).
    pub test_mode: bool,
    /// Submit buffered type-ahead as soon as the password prompt arrives.
    submit_on_prompt: bool,
    last_input: Instant,
    commands: Sender<Command>,
    /// The greeter should exit.
    pub quit: bool,
    /// `--test-fingerprint`: the simulated reader's touch channel.
    pub touches: Option<Sender<Touch>>,
}

/// Everything [`App::new`] needs.
#[derive(Debug)]
pub struct AppInit {
    /// Configuration.
    pub cfg: Config,
    /// Key bindings.
    pub keymap: Keymap,
    /// Theme registry.
    pub registry: Registry,
    /// Theme inputs, kept so the settings menu can re-resolve.
    pub theme_inputs: ThemeInputs,
    /// Colour mode.
    pub mode: ColorMode,
    /// Sessions.
    pub sessions: Vec<Session>,
    /// Users.
    pub users: Vec<User>,
    /// Remembered state.
    pub state: State,
    /// Channel to the auth worker.
    pub commands: Sender<Command>,
    /// Test mode.
    pub test_mode: bool,
    /// Start in the screensaver.
    pub screensaver: bool,
    /// Current time.
    pub now: Instant,
    /// `--test-fingerprint`: where F9/F10 send simulated touches.
    pub touches: Option<Sender<Touch>>,
}

impl App {
    /// Builds the initial state.
    #[must_use]
    pub fn new(init: AppInit) -> Self {
        let AppInit {
            cfg,
            keymap,
            registry,
            mut theme_inputs,
            mode,
            sessions,
            users,
            state,
            commands,
            test_mode,
            screensaver,
            now,
            touches,
        } = init;
        if theme_inputs.cli.is_none() {
            theme_inputs.cli.clone_from(&state.theme);
        }
        let session = state
            .session
            .as_deref()
            .and_then(|k| sessions.iter().position(|s| s.key() == k))
            .unwrap_or(0);
        let username = if cfg.general.remember {
            state.user.clone().unwrap_or_default()
        } else {
            String::new()
        };
        let effect_kind = state
            .effect
            .as_deref()
            .and_then(|e| e.parse().ok())
            .unwrap_or(cfg.general.effect);
        let styles = styles_for(&registry, &theme_inputs, mode, tint_for(effect_kind));
        let text_fx = cfg.general.text_effect;
        let banner = banner_for(&cfg, sessions.get(session));
        let animations = cfg.general.animations;
        let mut app = Self {
            keymap,
            registry,
            theme_inputs,
            mode,
            styles,
            sessions,
            session,
            users,
            focus: Focus::User,
            username,
            answer: empty_answer(),
            phase: Phase::Idle,
            panel: screensaver.then_some(Panel::Screensaver),
            status: None,
            failures: 0,
            caps_lock: false,
            banner,
            variant: 0,
            banner_since: 0,
            effect_kind,
            effect: None,
            text_fx,
            tick: 0,
            test_mode,
            submit_on_prompt: false,
            last_input: now,
            commands,
            quit: false,
            touches,
            cfg,
        };
        if animations {
            app.effect = fx::make(app.effect_kind, 0x6A17_7259);
        }
        app
    }

    /// The selected session.
    #[must_use]
    pub fn selected_session(&self) -> Option<&Session> {
        self.sessions.get(self.session)
    }

    /// True while something on screen is moving, so the loop must run at full rate.
    #[must_use]
    pub fn is_animating(&self) -> bool {
        if !self.cfg.general.animations {
            return false;
        }
        let lines = self.banner.lines(self.variant);
        self.effect.is_some()
            || matches!(self.phase, Phase::Scanning(_))
            || !fx::text::is_settled(
                &lines,
                self.text_fx,
                self.tick.saturating_sub(self.banner_since),
            )
    }

    /// Advances one frame.
    pub fn on_tick(&mut self, now: Instant, area: ratatui::layout::Rect) {
        self.tick = self.tick.wrapping_add(1);
        if self.tick % (FPS / 2) == 0 {
            self.caps_lock = crate::caps::is_on();
        }
        let animate = self.cfg.general.animations;
        if let Some(effect) = self.effect.as_mut().filter(|_| animate) {
            effect.step(area);
        }
        let idle_ok = matches!(self.phase, Phase::Idle | Phase::Prompt { .. });
        let timeout = self.cfg.screensaver.idle_timeout;
        if self.cfg.screensaver.enabled
            && idle_ok
            && self.panel.is_none()
            && now.duration_since(self.last_input) >= timeout
        {
            self.panel = Some(Panel::Screensaver);
        }
    }

    /// Handles a report from the auth worker.
    pub fn on_auth(&mut self, ev: Event) {
        match ev {
            Event::Message { kind, text } => self.on_pam_message(kind, &text),
            Event::Prompt { text, secret } => {
                let buffered = std::mem::take(&mut self.submit_on_prompt);
                if matches!(self.phase, Phase::Scanning(_)) {
                    self.set_status(
                        Level::Info,
                        "Fingerprint not recognized — type your password",
                    );
                }
                if !secret {
                    // Type-ahead was meant for a password; never show it or send it
                    // as the answer to a visible question.
                    self.reset_answer();
                }
                self.phase = Phase::Prompt { text, secret };
                self.focus = Focus::Answer;
                if buffered && secret && !self.answer.is_empty() {
                    self.submit_answer();
                }
            }
            Event::Authenticated => {
                self.set_status(Level::Ok, "Authenticated");
                self.start_session();
            }
            Event::Started => self.quit = true,
            Event::Denied(reason) => {
                self.failures = self.failures.saturating_add(1);
                tracing::info!(
                    event = "auth.denied",
                    failures = self.failures,
                    "login denied"
                );
                let msg = if reason.is_empty() {
                    "Login failed".to_owned()
                } else {
                    format!("Login failed: {reason}")
                };
                self.set_status(Level::Error, &msg);
                self.reset_answer();
                if self.phase == Phase::Checking {
                    // A refused answer: start a fresh conversation for the same user.
                    self.begin();
                } else {
                    // Refused before any question (e.g. a locked account): retrying
                    // would loop, so hand the user name back.
                    self.phase = Phase::Idle;
                    self.focus = Focus::User;
                }
            }
            Event::Failed(reason) => {
                self.set_status(Level::Error, &format!("greetd: {reason}"));
                self.reset_answer();
                self.phase = Phase::Idle;
                self.focus = Focus::User;
            }
        }
    }

    fn on_pam_message(&mut self, kind: MessageKind, text: &str) {
        match fingerprint::classify(&self.cfg.fingerprint, text) {
            Some(Signal::Scanning) => {
                let prev = match &self.phase {
                    Phase::Scanning(s) => *s,
                    _ => Scan {
                        misses: 0,
                        since: self.tick,
                        wants_password: false,
                    },
                };
                self.phase = Phase::Scanning(Scan {
                    since: self.tick,
                    ..prev
                });
                self.focus = Focus::Answer;
                self.set_status(Level::Info, text);
            }
            Some(Signal::Retry) => {
                if let Phase::Scanning(s) = &mut self.phase {
                    s.misses = s.misses.saturating_add(1);
                    s.since = self.tick;
                }
                self.set_status(Level::Warn, text);
            }
            Some(Signal::TimedOut | Signal::Failure) => {
                // pam_fprintd may retry after this, so don't promise a password yet.
                self.set_status(Level::Info, text);
            }
            None => {
                let level = if kind == MessageKind::Error {
                    Level::Error
                } else {
                    Level::Info
                };
                self.set_status(level, text);
            }
        }
    }

    /// Handles a key press. Returns what the main loop must do.
    pub fn on_key(&mut self, key: KeyEvent, now: Instant) -> Effects {
        let mut out = Effects::default();
        if key.kind == KeyEventKind::Release {
            return out;
        }
        self.last_input = now;
        if self.panel == Some(Panel::Screensaver) {
            // The waking key is consumed so it never lands in a field.
            self.panel = None;
            return out;
        }
        if self.simulated_touch(&key) {
            return out;
        }
        let action = self.keymap.action(&key);
        if let Some(panel) = self.panel {
            self.on_panel_key(panel, action, &mut out);
            return out;
        }
        match action {
            Some(Action::SessionMenu) if self.can_change_session() => {
                self.panel = Some(Panel::Sessions(self.session));
            }
            Some(Action::PowerMenu) => self.panel = Some(Panel::Power(0)),
            Some(Action::SettingsMenu) => self.panel = Some(Panel::Settings(0)),
            Some(Action::AccessibleMode) => self.leave_for_linear(&mut out),
            Some(Action::ArtPrev) => self.cycle_variant(false),
            Some(Action::ArtNext) => self.cycle_variant(true),
            Some(Action::Submit) => self.on_submit(),
            Some(Action::Back) => self.on_back(),
            Some(Action::NextField | Action::PrevField) => self.toggle_focus(),
            Some(Action::Up) if self.focus == Focus::User => self.cycle_user(false),
            Some(Action::Down) if self.focus == Focus::User => self.cycle_user(true),
            _ => self.on_text_key(key),
        }
        out
    }

    /// Accessible mode starts its own conversation, so the current one must
    /// be closed first. While greetd is busy (waiting, scanning, checking,
    /// starting) a cancel would block or race, so the switch waits.
    fn leave_for_linear(&mut self, out: &mut Effects) {
        match self.phase {
            Phase::Idle => out.go_linear = true,
            Phase::Prompt { .. } => {
                self.send(Command::Cancel);
                self.reset_answer();
                self.phase = Phase::Idle;
                out.go_linear = true;
            }
            _ => self.set_status(
                Level::Info,
                "Accessible mode opens once this login step finishes",
            ),
        }
    }

    /// `--test-fingerprint` only: F9 is an enrolled finger, F10 a wrong one.
    fn simulated_touch(&mut self, key: &KeyEvent) -> bool {
        let touch = match key.code {
            KeyCode::F(9) => Touch::Match,
            KeyCode::F(10) => Touch::Wrong,
            _ => return false,
        };
        let Some(tx) = self
            .touches
            .as_ref()
            .filter(|_| matches!(self.phase, Phase::Scanning(_)))
        else {
            return false;
        };
        let sent = tx.send(touch).is_ok();
        if sent {
            self.set_status(Level::Info, "Test: finger on the reader");
        }
        sent
    }

    fn can_change_session(&self) -> bool {
        matches!(self.phase, Phase::Idle | Phase::Prompt { .. })
    }

    fn on_text_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let typing_answer = self.focus == Focus::Answer
            && (matches!(self.phase, Phase::Prompt { .. })
                || (matches!(self.phase, Phase::Scanning(_)) && self.cfg.fingerprint.type_ahead));
        let field: Option<&mut String> = match self.focus {
            Focus::User if self.phase == Phase::Idle => Some(&mut self.username),
            Focus::Answer if typing_answer => Some(&mut self.answer),
            _ => None,
        };
        let Some(field) = field else { return };
        match key.code {
            KeyCode::Char('u') if ctrl => field.clear(),
            KeyCode::Char('w') if ctrl => {
                let keep = field.trim_end().rfind(' ').map_or(0, |i| i + 1);
                field.truncate(keep);
            }
            KeyCode::Char(c)
                if !ctrl && !c.is_control() && field.len() + c.len_utf8() <= FIELD_MAX =>
            {
                field.push(c);
            }
            KeyCode::Backspace => {
                field.pop();
            }
            _ => {}
        }
    }

    fn toggle_focus(&mut self) {
        self.focus = match (self.focus, &self.phase) {
            (Focus::User, Phase::Prompt { .. } | Phase::Scanning(_)) => Focus::Answer,
            _ => Focus::User,
        };
    }

    fn cycle_user(&mut self, forward: bool) {
        if self.users.is_empty() || self.phase != Phase::Idle {
            return;
        }
        let n = self.users.len();
        let cur = self.users.iter().position(|u| u.name == self.username);
        let next = match (cur, forward) {
            (None, true) => 0,
            (None, false) => n - 1,
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
        };
        if let Some(u) = self.users.get(next) {
            self.username.clone_from(&u.name);
        }
    }

    fn cycle_variant(&mut self, forward: bool) {
        let n = self.banner.count().max(1);
        self.variant = if forward {
            (self.variant + 1) % n
        } else {
            (self.variant + n - 1) % n
        };
        self.banner_since = self.tick;
    }

    fn on_submit(&mut self) {
        match (&self.phase, self.focus) {
            (Phase::Idle, _) => {
                if self.username.trim().is_empty() {
                    self.set_status(Level::Warn, "Type a user name first");
                } else {
                    self.begin();
                }
            }
            (Phase::Prompt { .. }, Focus::Answer) => self.submit_answer(),
            (Phase::Prompt { .. }, Focus::User) => self.focus = Focus::Answer,
            (Phase::Scanning(_), _) if self.cfg.fingerprint.type_ahead => {
                self.submit_on_prompt = true;
                self.set_status(
                    Level::Info,
                    "Password queued; it is sent when the scan window closes",
                );
            }
            _ => {}
        }
    }

    fn on_back(&mut self) {
        match &mut self.phase {
            Phase::Prompt { .. } => {
                self.send(Command::Cancel);
                self.reset_answer();
                self.phase = Phase::Idle;
                self.focus = Focus::User;
                self.status = None;
            }
            Phase::Scanning(scan) => {
                // greetd cannot cancel while PAM waits on the sensor (see the PRD),
                // so remember the wish and let the scan window run out.
                scan.wants_password = true;
                self.focus = Focus::Answer;
                self.set_status(
                    Level::Info,
                    "Password prompt follows when the scan window closes",
                );
            }
            _ => self.status = None,
        }
    }

    fn begin(&mut self) {
        let username = self.username.trim().to_owned();
        self.send(Command::Begin { username });
        self.phase = Phase::Waiting;
    }

    fn submit_answer(&mut self) {
        let answer = std::mem::replace(&mut self.answer, empty_answer());
        self.send(Command::Answer(answer));
        self.phase = Phase::Checking;
    }

    fn start_session(&mut self) {
        let Some(session) = self.selected_session() else {
            self.set_status(Level::Error, "No session is configured");
            self.send(Command::Cancel);
            self.phase = Phase::Idle;
            self.focus = Focus::User;
            return;
        };
        let (cmd, env) = session.launch(&self.cfg.general.xsession_wrapper);
        let key = session.key();
        if self.cfg.general.remember && !self.test_mode {
            let mut st = State::load(&self.cfg.general.cache_file);
            st.user = Some(self.username.trim().to_owned());
            st.session = Some(key);
            st.save(&self.cfg.general.cache_file);
        }
        self.send(Command::Start { cmd, env });
        self.phase = Phase::Starting;
    }

    fn reset_answer(&mut self) {
        self.answer = empty_answer();
        self.submit_on_prompt = false;
    }

    fn send(&mut self, cmd: Command) {
        if self.commands.send(cmd).is_err() {
            self.set_status(
                Level::Error,
                "Authentication worker stopped; restart the greeter",
            );
        }
    }

    fn set_status(&mut self, level: Level, text: &str) {
        self.status = Some((level, text.to_owned()));
    }

    fn on_panel_key(&mut self, panel: Panel, action: Option<Action>, out: &mut Effects) {
        let rows = match panel {
            Panel::Sessions(_) => self.sessions.len(),
            Panel::Power(_) => POWER_ITEMS.len(),
            Panel::Settings(_) => SETTINGS_ITEMS.len(),
            Panel::Screensaver => 0,
        }
        .max(1);
        let sel = match panel {
            Panel::Sessions(i) | Panel::Power(i) | Panel::Settings(i) => i,
            Panel::Screensaver => 0,
        };
        let moved = |i: usize, fwd: bool| {
            if fwd {
                (i + 1) % rows
            } else {
                (i + rows - 1) % rows
            }
        };
        let with = |i: usize| match panel {
            Panel::Sessions(_) => Panel::Sessions(i),
            Panel::Power(_) => Panel::Power(i),
            Panel::Settings(_) => Panel::Settings(i),
            Panel::Screensaver => Panel::Screensaver,
        };
        match action {
            Some(Action::Up | Action::PrevField) => self.panel = Some(with(moved(sel, false))),
            Some(Action::Down | Action::NextField) => self.panel = Some(with(moved(sel, true))),
            Some(Action::Back) => self.panel = None,
            Some(Action::Submit) => match panel {
                Panel::Sessions(i) => {
                    self.session = i.min(self.sessions.len().saturating_sub(1));
                    self.banner = banner_for(&self.cfg, self.selected_session());
                    self.variant = 0;
                    self.banner_since = self.tick;
                    self.panel = None;
                }
                Panel::Power(i) => {
                    self.panel = None;
                    let cmd = match i {
                        0 => Some(self.cfg.power.shutdown.clone()),
                        1 => Some(self.cfg.power.reboot.clone()),
                        _ => None,
                    };
                    if let Some(cmd) = cmd.filter(|c| !c.is_empty()) {
                        if self.test_mode {
                            self.set_status(
                                Level::Info,
                                &format!("Test mode: would run {}", cmd.join(" ")),
                            );
                        } else {
                            out.power = Some(cmd);
                        }
                    }
                }
                Panel::Settings(i) => self.on_setting(i, out),
                Panel::Screensaver => {}
            },
            _ => {}
        }
    }

    fn on_setting(&mut self, row: usize, out: &mut Effects) {
        match row {
            0 => {
                let reg = self.registry.registered();
                let cur = reg.iter().position(|s| *s == self.styles.slug);
                let next = cur.map_or(0, |i| (i + 1) % reg.len().max(1));
                if let Some(slug) = reg.get(next) {
                    self.theme_inputs.cli = Some(slug.clone());
                    // A whole-surface, atomic switch (§11.6.2): every role is replaced at once.
                    self.styles = styles_for(
                        &self.registry,
                        &self.theme_inputs,
                        self.mode,
                        tint_for(self.effect_kind),
                    );
                    out.reload_palette = true;
                    self.remember(|st, app| st.theme = Some(app.styles.slug.clone()));
                }
            }
            1 => {
                self.effect_kind = self.effect_kind.next();
                self.effect = fx::make(self.effect_kind, self.tick ^ 0x6A17_7259);
                // Each effect may tint the ramp; reload it (and the VT slots) at once.
                self.styles = styles_for(
                    &self.registry,
                    &self.theme_inputs,
                    self.mode,
                    tint_for(self.effect_kind),
                );
                out.reload_palette = true;
                self.remember(|st, app| st.effect = Some(app.effect_kind.to_string()));
            }
            2 => {
                self.text_fx = self.text_fx.next();
                self.banner_since = self.tick;
            }
            _ => self.panel = None,
        }
    }

    fn remember(&self, f: impl FnOnce(&mut State, &Self)) {
        if self.test_mode {
            return;
        }
        let mut st = State::load(&self.cfg.general.cache_file);
        f(&mut st, self);
        st.save(&self.cfg.general.cache_file);
    }

    /// Idle time before the screensaver.
    #[cfg(test)]
    #[must_use]
    pub fn idle_timeout(&self) -> std::time::Duration {
        self.cfg.screensaver.idle_timeout
    }
}

/// Longest field accepted, in bytes.
pub const FIELD_MAX: usize = 512;

/// An answer buffer that never reallocates (and so never frees a copy of a
/// partial password): it holds `FIELD_MAX` bytes plus one UTF-8 character.
fn empty_answer() -> Zeroizing<String> {
    Zeroizing::new(String::with_capacity(FIELD_MAX + 4))
}

fn styles_for(registry: &Registry, inputs: &ThemeInputs, mode: ColorMode, tint: Tint) -> Styles {
    let resolved = theme::resolve(registry, inputs);
    tracing::debug!(
        event = "theme.resolved",
        slug = %resolved.slug,
        base = %resolved.base,
        source = %resolved.base_source,
        overlay = %resolved.overlay,
        "theme resolved"
    );
    match registry
        .get(&resolved.slug)
        .or_else(|| registry.get(theme::DEFAULT_THEME))
    {
        Some(def) => Styles::tinted(def, mode, tint),
        // The registry constructor guarantees the default exists; mono is the safe floor.
        None => Styles::tinted(&fallback_mono(), mode, tint),
    }
}

/// The ramp tint an effect wants: the Matrix rain is green, the rest use the accent.
fn tint_for(kind: EffectKind) -> Tint {
    match kind {
        EffectKind::Matrix => Tint::Success,
        _ => Tint::Accent,
    }
}

fn fallback_mono() -> theme::ThemeDef {
    use ratatui::style::Color;
    use theme::Token;
    let t = Token::Ansi(Color::Reset);
    theme::ThemeDef {
        slug: theme::MONO.to_owned(),
        background: t,
        surface: t,
        surface_alt: t,
        foreground: t,
        accent: t,
        structure: t,
        success: t,
        error: t,
        warning: t,
        focus: Token::Reverse,
        border: t,
    }
}

fn banner_for(cfg: &Config, session: Option<&Session>) -> Banner {
    match session {
        Some(s) => Banner::for_session(cfg.art_dir.as_deref(), &s.key()),
        None => Banner::builtin(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{self, Receiver};
    use std::time::Duration;

    use super::*;
    use crate::keys::KeyConfig;

    fn app() -> (App, Receiver<Command>) {
        let (tx, rx) = mpsc::channel();
        let cfg = Config::default();
        let (keymap, _) = Keymap::new(&KeyConfig::default());
        let app = App::new(AppInit {
            sessions: crate::sessions::Session::shell(&cfg.general.default_session)
                .into_iter()
                .collect(),
            cfg,
            keymap,
            registry: Registry::embedded().expect("palette"),
            theme_inputs: ThemeInputs::default(),
            mode: ColorMode::TrueColor,
            users: vec![],
            state: State::default(),
            commands: tx,
            test_mode: true,
            screensaver: false,
            now: Instant::now(),
            touches: None,
        });
        (app, rx)
    }

    fn key(app: &mut App, code: KeyCode) {
        let _ = app.on_key(KeyEvent::new(code, KeyModifiers::NONE), Instant::now());
    }

    fn type_text(app: &mut App, s: &str) {
        for c in s.chars() {
            key(app, KeyCode::Char(c));
        }
    }

    #[test]
    fn enter_on_user_begins_a_session() {
        let (mut app, rx) = app();
        type_text(&mut app, "mj");
        key(&mut app, KeyCode::Enter);
        assert!(matches!(rx.try_recv(), Ok(Command::Begin { username }) if username == "mj"));
        assert_eq!(app.phase, Phase::Waiting);
    }

    #[test]
    fn password_prompt_flow() {
        let (mut app, rx) = app();
        type_text(&mut app, "mj");
        key(&mut app, KeyCode::Enter);
        let _ = rx.try_recv();
        app.on_auth(Event::Prompt {
            text: "Password:".into(),
            secret: true,
        });
        assert_eq!(app.focus, Focus::Answer);
        type_text(&mut app, "pw");
        key(&mut app, KeyCode::Enter);
        assert!(matches!(rx.try_recv(), Ok(Command::Answer(a)) if a.as_str() == "pw"));
        assert!(app.answer.is_empty(), "answer buffer is emptied once sent");
        app.on_auth(Event::Authenticated);
        assert!(
            matches!(rx.try_recv(), Ok(Command::Start { cmd, .. }) if cmd == ["nu", "--login"])
        );
        app.on_auth(Event::Started);
        assert!(app.quit);
    }

    #[test]
    fn denial_restarts_the_conversation() {
        let (mut app, rx) = app();
        type_text(&mut app, "mj");
        key(&mut app, KeyCode::Enter);
        let _ = rx.try_recv();
        app.on_auth(Event::Prompt {
            text: "Password:".into(),
            secret: true,
        });
        type_text(&mut app, "wrong");
        key(&mut app, KeyCode::Enter);
        let _ = rx.try_recv();
        app.on_auth(Event::Denied(String::new()));
        assert_eq!(app.failures, 1);
        assert!(matches!(app.status, Some((Level::Error, _))));
        assert!(matches!(rx.try_recv(), Ok(Command::Begin { .. })));
    }

    #[test]
    fn type_ahead_during_scan_submits_at_the_prompt() {
        let (mut app, rx) = app();
        type_text(&mut app, "mj");
        key(&mut app, KeyCode::Enter);
        let _ = rx.try_recv();
        app.on_auth(Event::Message {
            kind: MessageKind::Info,
            text: "Place your right index finger on the fingerprint reader".into(),
        });
        assert!(matches!(app.phase, Phase::Scanning(_)));
        type_text(&mut app, "pw");
        key(&mut app, KeyCode::Enter);
        app.on_auth(Event::Message {
            kind: MessageKind::Error,
            text: "Failed to match fingerprint".into(),
        });
        assert!(matches!(app.phase, Phase::Scanning(Scan { misses: 1, .. })));
        app.on_auth(Event::Prompt {
            text: "Password:".into(),
            secret: true,
        });
        assert!(matches!(rx.try_recv(), Ok(Command::Answer(a)) if a.as_str() == "pw"));
        assert_eq!(app.phase, Phase::Checking);
    }

    #[test]
    fn esc_during_scan_only_records_the_wish() {
        let (mut app, rx) = app();
        app.phase = Phase::Scanning(Scan {
            misses: 0,
            since: 0,
            wants_password: false,
        });
        key(&mut app, KeyCode::Esc);
        assert!(matches!(
            app.phase,
            Phase::Scanning(Scan {
                wants_password: true,
                ..
            })
        ));
        assert!(
            rx.try_recv().is_err(),
            "no cancel is sent while PAM holds greetd"
        );
    }

    #[test]
    fn screensaver_starts_when_idle_and_wakes_without_typing() {
        let (mut app, _rx) = app();
        let start = Instant::now();
        app.last_input = start;
        let later = start + app.idle_timeout() + Duration::from_secs(1);
        app.on_tick(later, ratatui::layout::Rect::new(0, 0, 80, 24));
        assert_eq!(app.panel, Some(Panel::Screensaver));
        let _ = app.on_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE), later);
        assert_eq!(app.panel, None);
        assert!(app.username.is_empty(), "the waking key is not typed");
    }

    #[test]
    fn power_menu_in_test_mode_runs_nothing() {
        let (mut app, _rx) = app();
        key(&mut app, KeyCode::F(12));
        assert_eq!(app.panel, Some(Panel::Power(0)));
        let out = app.on_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            Instant::now(),
        );
        assert!(out.power.is_none());
        assert!(matches!(&app.status, Some((Level::Info, s)) if s.contains("systemctl poweroff")));
    }

    #[test]
    fn visible_prompt_discards_type_ahead() {
        let (mut app, rx) = app();
        app.phase = Phase::Scanning(Scan {
            misses: 0,
            since: 0,
            wants_password: false,
        });
        app.focus = Focus::Answer;
        type_text(&mut app, "secret");
        key(&mut app, KeyCode::Enter);
        app.on_auth(Event::Prompt {
            text: "Verification code:".into(),
            secret: false,
        });
        assert!(app.answer.is_empty());
        assert!(rx.try_recv().is_err(), "nothing was submitted");
    }

    #[test]
    fn denial_before_any_question_returns_to_idle() {
        let (mut app, rx) = app();
        type_text(&mut app, "locked");
        key(&mut app, KeyCode::Enter);
        let _ = rx.try_recv();
        app.on_auth(Event::Denied("account locked".into()));
        assert_eq!(app.phase, Phase::Idle);
        assert!(rx.try_recv().is_err(), "no retry loop");
    }

    #[test]
    fn accessible_switch_waits_for_a_safe_point() {
        let (mut app, rx) = app();
        app.phase = Phase::Scanning(Scan {
            misses: 0,
            since: 0,
            wants_password: false,
        });
        let out = app.on_key(
            KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE),
            Instant::now(),
        );
        assert!(!out.go_linear);
        app.phase = Phase::Prompt {
            text: "Password:".into(),
            secret: true,
        };
        let out = app.on_key(
            KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE),
            Instant::now(),
        );
        assert!(out.go_linear);
        assert!(matches!(rx.try_recv(), Ok(Command::Cancel)));
    }

    #[test]
    fn no_session_cancels_after_authentication() {
        let (mut app, rx) = app();
        app.sessions.clear();
        app.phase = Phase::Checking;
        app.on_auth(Event::Authenticated);
        assert!(matches!(rx.try_recv(), Ok(Command::Cancel)));
        assert_eq!(app.phase, Phase::Idle);
    }

    #[test]
    fn answer_buffer_never_reallocates() {
        let (mut app, _rx) = app();
        app.phase = Phase::Prompt {
            text: "Password:".into(),
            secret: true,
        };
        app.focus = Focus::Answer;
        let cap = app.answer.capacity();
        type_text(&mut app, &"é".repeat(400));
        assert_eq!(app.answer.capacity(), cap);
        assert!(app.answer.len() <= FIELD_MAX);
    }

    #[test]
    fn test_keys_touch_the_simulated_reader_only_while_scanning() {
        let (mut app, _rx) = app();
        let (tx, trx) = mpsc::channel();
        app.touches = Some(tx);
        key(&mut app, KeyCode::F(9));
        assert!(trx.try_recv().is_err(), "no touch outside a scan");
        app.phase = Phase::Scanning(Scan {
            misses: 0,
            since: 0,
            wants_password: false,
        });
        key(&mut app, KeyCode::F(10));
        key(&mut app, KeyCode::F(9));
        assert_eq!(trx.try_recv().ok(), Some(Touch::Wrong));
        assert_eq!(trx.try_recv().ok(), Some(Touch::Match));
    }

    #[test]
    fn switching_to_matrix_turns_the_ramp_green() {
        let (mut app, _rx) = app();
        let before = app.styles.fx.ramp;
        key(&mut app, KeyCode::F(1));
        key(&mut app, KeyCode::Down);
        let mut reloaded = false;
        for _ in 0..crate::fx::EffectKind::ALL.len() {
            if app.effect_kind == EffectKind::Matrix {
                break;
            }
            reloaded |= app
                .on_key(
                    KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                    Instant::now(),
                )
                .reload_palette;
        }
        assert_eq!(app.effect_kind, EffectKind::Matrix);
        assert!(reloaded);
        assert_eq!(
            app.styles.fx.ramp[4], app.styles.success,
            "ramp runs through the success token"
        );
        assert_ne!(app.styles.fx.ramp, before);
    }

    #[test]
    fn theme_cycling_reloads_the_palette() {
        let (mut app, _rx) = app();
        let before = app.styles.slug.clone();
        key(&mut app, KeyCode::F(1));
        let out = app.on_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            Instant::now(),
        );
        assert!(out.reload_palette);
        assert_ne!(app.styles.slug, before);
    }
}
