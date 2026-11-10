// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>

//! Modern terminal protocols (`spacecraft-tui-protocols`).
//!
//! Gantry speaks the protocols that help a login screen and stays silent
//! everywhere else:
//!
//! | Protocol | Status |
//! |----------|--------|
//! | Kitty keyboard (flag 1, disambiguate) | implemented, probed |
//! | OSC 9;4 progress | implemented, allow-listed terminals only |
//! | Kitty graphics, animation | not used: the banner is VT-safe text art |
//! | OSC 52 clipboard (plain and rich) | not used: nothing is copied |
//! | OSC 22 pointer shape | not used: no mouse-interactive regions |
//!
//! All five rules from the skill hold here. Nothing is emitted unless the
//! output is a TTY, no agent is running, and `TERM` names a real terminal
//! (§2.1). Support is probed once, fenced by a DA1 query, under a deadline
//! (§2.2). Every mode that was turned on is undone on normal exit, error,
//! panic and signal (§2.3). OSC 9;4 is sent only to terminals known to treat
//! it as progress, never "sent in hope" (§2.4). A flag and a variable
//! override detection (§2.5).

use std::io::{self, Write as _};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};

/// Default probe deadline: long enough for a local terminal, short enough
/// that a terminal answering nothing delays the login screen only briefly.
pub const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_millis(200);

/// How long to keep swallowing late reply bytes after the probe ends, so no
/// reply leaks into the key reader as typed text.
const DRAIN_WINDOW: Duration = Duration::from_millis(20);

/// One protocol the user can switch on or off by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// Kitty keyboard protocol.
    Keyboard,
    /// Kitty graphics protocol and the image fallback ladder.
    Graphics,
    /// Kitty graphics animation.
    Animation,
    /// OSC 52 clipboard.
    Clipboard,
    /// OSC 5522 rich clipboard.
    ClipboardRich,
    /// OSC 9;4 progress.
    Progress,
    /// OSC 22 pointer shapes.
    Pointer,
}

impl Protocol {
    /// Every protocol, in the order the skill lists them.
    pub const ALL: [Self; 7] = [
        Self::Keyboard,
        Self::Graphics,
        Self::Animation,
        Self::Clipboard,
        Self::ClipboardRich,
        Self::Progress,
        Self::Pointer,
    ];

    /// The name used by `--term-protocols`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Keyboard => "keyboard",
            Self::Graphics => "graphics",
            Self::Animation => "animation",
            Self::Clipboard => "clipboard",
            Self::ClipboardRich => "clipboard-rich",
            Self::Progress => "progress",
            Self::Pointer => "pointer",
        }
    }

    /// Why Gantry has no code for this protocol, or `None` when it does.
    #[must_use]
    pub const fn not_used_because(self) -> Option<&'static str> {
        match self {
            Self::Keyboard | Self::Progress => None,
            Self::Graphics | Self::Animation => Some("the banner is VT-safe text art, not images"),
            Self::Clipboard | Self::ClipboardRich => Some("Gantry never copies anything"),
            Self::Pointer => Some("the interface has no mouse-interactive regions"),
        }
    }

    const fn index(self) -> usize {
        match self {
            Self::Keyboard => 0,
            Self::Graphics => 1,
            Self::Animation => 2,
            Self::Clipboard => 3,
            Self::ClipboardRich => 4,
            Self::Progress => 5,
            Self::Pointer => 6,
        }
    }
}

/// The user's overrides: `Some(true)` forces a protocol on, `Some(false)`
/// forces it off, `None` leaves it to detection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Overrides {
    forced: [Option<bool>; 7],
}

impl Overrides {
    /// Applies a list such as `-graphics,+progress` on top of what is
    /// already set. A bare name means `+name`; `all` covers every protocol.
    ///
    /// # Errors
    /// The list names a protocol that does not exist.
    pub fn apply(&mut self, list: &str) -> Result<(), String> {
        for item in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let (on, name) = match item.as_bytes().first() {
                Some(b'+') => (true, &item[1..]),
                Some(b'-') => (false, &item[1..]),
                _ => (true, item),
            };
            if name == "all" {
                self.forced = [Some(on); 7];
                continue;
            }
            let protocol = Protocol::ALL
                .into_iter()
                .find(|p| p.name() == name)
                .ok_or_else(|| format!("unknown terminal protocol `{name}`"))?;
            self.forced[protocol.index()] = Some(on);
        }
        Ok(())
    }

    /// Builds the overrides from the variable, then the flag on top, so the
    /// flag wins any conflict (§2.5).
    ///
    /// # Errors
    /// Either list names a protocol that does not exist.
    pub fn resolve(variable: Option<&str>, flag: Option<&str>) -> Result<Self, String> {
        let mut out = Self::default();
        for list in [variable, flag].into_iter().flatten() {
            out.apply(list)?;
        }
        Ok(out)
    }

    /// The override for one protocol.
    #[must_use]
    pub const fn get(self, protocol: Protocol) -> Option<bool> {
        self.forced[protocol.index()]
    }
}

/// What the process knows about its terminal, read once.
#[derive(Debug, Clone, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "one flag per environment signal"
)]
pub struct TermEnv {
    /// Standard output is a terminal.
    pub stdout_tty: bool,
    /// Standard input is a terminal.
    pub stdin_tty: bool,
    /// An AI agent or CI runner is driving the process.
    pub agent: bool,
    /// `TERM`.
    pub term: Option<String>,
    /// `TERM_PROGRAM`.
    pub term_program: Option<String>,
    /// `TERM_PROGRAM_VERSION`.
    pub term_program_version: Option<String>,
    /// `WT_SESSION` is set.
    pub windows_terminal: bool,
    /// `ConEmuPID` is set or `ConEmuANSI=ON`.
    pub conemu: bool,
    /// Inside tmux, zellij or GNU screen.
    pub multiplexer: bool,
}

impl TermEnv {
    /// Reads the real process environment.
    #[must_use]
    pub fn from_process() -> Self {
        use std::io::IsTerminal as _;
        let mut env = Self::from_lookup(|k| std::env::var(k).ok());
        env.stdout_tty = io::stdout().is_terminal();
        env.stdin_tty = io::stdin().is_terminal();
        env
    }

    /// Builds the environment from a lookup, for tests. TTY flags stay false.
    #[must_use]
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Self {
        let set = |k: &str| get(k).is_some_and(|v| !v.is_empty());
        Self {
            stdout_tty: false,
            stdin_tty: false,
            agent: crate::output::is_agent_env(&get),
            term: get("TERM"),
            term_program: get("TERM_PROGRAM"),
            term_program_version: get("TERM_PROGRAM_VERSION"),
            windows_terminal: set("WT_SESSION"),
            conemu: set("ConEmuPID") || get("ConEmuANSI").is_some_and(|v| v == "ON"),
            multiplexer: set("TMUX") || set("ZELLIJ") || set("STY"),
        }
    }

    /// The §2.1 gate. `Some(reason)` means no sequence and no probe may be
    /// sent, whatever the user forced.
    #[must_use]
    pub fn gate(&self) -> Option<&'static str> {
        if !self.stdout_tty || !self.stdin_tty {
            return Some("not a terminal");
        }
        if self.agent {
            return Some("agent or CI mode");
        }
        match self.term.as_deref() {
            None | Some("" | "dumb") => Some("TERM is unset or dumb"),
            // The Linux console parses a few of these sequences as something
            // else. Gantry's primary home gets none of them.
            Some("linux") => Some("Linux console"),
            Some(_) => None,
        }
    }

    /// True when this terminal is known to treat `OSC 9;4` as progress
    /// rather than as a desktop notification (§2.4). Unknown means no.
    #[must_use]
    pub fn progress_allowed(&self) -> bool {
        if self.multiplexer {
            return false;
        }
        if self.windows_terminal || self.conemu {
            return true;
        }
        let version = self.term_program_version.as_deref().unwrap_or_default();
        let program = self
            .term_program
            .as_deref()
            .unwrap_or_default()
            .to_lowercase();
        if ["ghostty", "vscode", "rio", "contour"]
            .iter()
            .any(|n| program.contains(n))
        {
            return true;
        }
        if program.contains("iterm") {
            return version_at_least(version, &[3, 6]);
        }
        if program == "wezterm" {
            // The 20240203 stable build shows OSC 9 as a notification.
            return leading_number(version) >= 20_250_210;
        }
        // kitty is deliberately absent: its first release with OSC 9;4 is not
        // recorded here, so it needs `--term-protocols=+progress`.
        self.term.as_deref().is_some_and(|t| t.starts_with("foot"))
    }
}

fn leading_number(text: &str) -> u64 {
    text.chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .unwrap_or(0)
}

fn version_at_least(text: &str, want: &[u64]) -> bool {
    let have: Vec<u64> = text.split('.').map(leading_number).collect();
    have.as_slice() >= want
}

/// What this run decided, for `--verbose` and `describe`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// In use.
    On,
    /// Not in use, with the reason.
    Off(&'static str),
}

impl State {
    /// True for [`State::On`].
    #[must_use]
    pub const fn is_on(self) -> bool {
        matches!(self, Self::On)
    }
}

/// The protocols in force for the full-screen interface.
#[derive(Debug, Clone, Copy)]
pub struct Active {
    /// Kitty keyboard flags were pushed.
    pub keyboard: State,
    /// OSC 9;4 may be sent.
    pub progress: State,
}

impl Active {
    /// Everything off, for linear mode and agents.
    #[must_use]
    pub const fn none(reason: &'static str) -> Self {
        Self {
            keyboard: State::Off(reason),
            progress: State::Off(reason),
        }
    }

    /// One line for the `--verbose` report.
    #[must_use]
    pub fn summary(&self) -> String {
        let show = |name: &str, s: State| match s {
            State::On => format!("{name}=on"),
            State::Off(why) => format!("{name}=off ({why})"),
        };
        format!(
            "terminal protocols: {}, {}",
            show("keyboard", self.keyboard),
            show("progress", self.progress)
        )
    }
}

/// Decides progress without touching the terminal.
#[must_use]
pub fn decide_progress(env: &TermEnv, overrides: Overrides) -> State {
    if let Some(reason) = env.gate() {
        return State::Off(reason);
    }
    match overrides.get(Protocol::Progress) {
        Some(false) => State::Off("switched off by the user"),
        Some(true) => State::On,
        None if env.progress_allowed() => State::On,
        None => State::Off("terminal not on the OSC 9;4 allow-list"),
    }
}

/// Replies found in the bytes the terminal sent back to the probe.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Replies {
    /// `CSI ? <flags> u` arrived.
    pub kitty_keyboard: bool,
    /// `CSI ? <params> c` arrived: the fence.
    pub da1: bool,
}

/// Scans `bytes` for the two replies the probe asks for. Anything else,
/// including partial sequences, is ignored.
#[must_use]
pub fn parse_replies(bytes: &[u8]) -> Replies {
    let mut out = Replies::default();
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i..].starts_with(b"\x1b[?") {
            let mut j = i + 3;
            while j < bytes.len() && (bytes[j].is_ascii_digit() || bytes[j] == b';') {
                j += 1;
            }
            match bytes.get(j) {
                Some(b'u') => out.kitty_keyboard = true,
                Some(b'c') => out.da1 = true,
                _ => {}
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// Waits up to `wait` for standard input to become readable.
fn readable(wait: Duration) -> io::Result<bool> {
    let stdin = rustix::stdio::stdin();
    let mut fds = [PollFd::new(&stdin, PollFlags::IN)];
    let timeout = Timespec {
        tv_sec: i64::try_from(wait.as_secs()).unwrap_or(0),
        tv_nsec: i64::from(wait.subsec_nanos()),
    };
    Ok(poll(&mut fds, Some(&timeout))? > 0)
}

fn read_some(buf: &mut [u8]) -> io::Result<usize> {
    Ok(rustix::io::read(rustix::stdio::stdin(), buf)?)
}

/// Asks the terminal whether it speaks the kitty keyboard protocol.
///
/// Raw mode must already be on. Writes `CSI ? u` then the DA1 fence, reads
/// until the fence or `deadline`, then drains late bytes. Any failure means
/// "no".
fn probe_keyboard(deadline: Duration) -> bool {
    let attempt = || -> io::Result<bool> {
        let mut out = io::stdout().lock();
        out.write_all(b"\x1b[?u\x1b[c")?;
        out.flush()?;
        drop(out);
        let end = Instant::now() + deadline;
        let mut seen = Vec::new();
        let mut buf = [0u8; 256];
        while let Some(left) = end.checked_duration_since(Instant::now()) {
            if !readable(left)? {
                break;
            }
            let n = read_some(&mut buf)?;
            if n == 0 {
                break;
            }
            seen.extend_from_slice(&buf[..n]);
            if parse_replies(&seen).da1 {
                break;
            }
        }
        // Swallow stragglers so they never reach the key reader.
        while readable(DRAIN_WINDOW)? {
            if read_some(&mut buf)? == 0 {
                break;
            }
        }
        Ok(parse_replies(&seen).kitty_keyboard)
    };
    attempt().unwrap_or(false)
}

/// Works out which protocols are in force, probing the keyboard once.
///
/// Call before entering the alternate screen: the probe runs briefly in raw
/// mode on the main screen. Never blocks longer than `deadline` plus the
/// drain window.
#[must_use]
pub fn negotiate(env: &TermEnv, overrides: Overrides, deadline: Duration) -> Active {
    let progress = decide_progress(env, overrides);
    if let Some(reason) = env.gate() {
        return Active::none(reason);
    }
    let keyboard = match overrides.get(Protocol::Keyboard) {
        Some(false) => State::Off("switched off by the user"),
        Some(true) => State::On,
        None => {
            let raw = ratatui::crossterm::terminal::enable_raw_mode().is_ok();
            let found = raw && probe_keyboard(deadline);
            if raw {
                let _ = ratatui::crossterm::terminal::disable_raw_mode();
            }
            if found {
                State::On
            } else {
                State::Off("no reply to the kitty keyboard query")
            }
        }
    };
    Active { keyboard, progress }
}

static KEYBOARD_PUSHED: AtomicBool = AtomicBool::new(false);
static PROGRESS_SHOWN: AtomicBool = AtomicBool::new(false);

/// Pushes kitty keyboard flag 1 (disambiguate escape codes). Call after the
/// alternate screen is entered; the main and alternate screens keep separate
/// stacks. Flag 1 only: Gantry needs no release events.
pub fn push_keyboard() {
    let mut out = io::stdout().lock();
    if out
        .write_all(b"\x1b[>1u")
        .and_then(|()| out.flush())
        .is_ok()
    {
        KEYBOARD_PUSHED.store(true, Ordering::SeqCst);
    }
}

/// Shows or clears the indeterminate taskbar progress (`OSC 9;4;3` / `;0`).
/// Written to stderr, because standard output carries the frame. Only
/// transitions are sent.
pub fn set_busy(busy: bool) {
    if PROGRESS_SHOWN.swap(busy, Ordering::SeqCst) == busy {
        return;
    }
    let seq: &[u8] = if busy {
        b"\x1b]9;4;3;0\x1b\\"
    } else {
        b"\x1b]9;4;0;0\x1b\\"
    };
    let mut err = io::stderr().lock();
    let _ = err.write_all(seq).and_then(|()| err.flush());
}

/// Undoes every protocol mode that was turned on, in reverse order of setup
/// (§2.3). Safe to call from `Drop`, the panic hook and the signal path:
/// each mode is cleared once, so a second call does nothing.
pub fn restore() {
    if KEYBOARD_PUSHED.swap(false, Ordering::SeqCst) {
        let mut out = io::stdout().lock();
        let _ = out.write_all(b"\x1b[<u").and_then(|()| out.flush());
    }
    if PROGRESS_SHOWN.swap(false, Ordering::SeqCst) {
        let mut err = io::stderr().lock();
        let _ = err
            .write_all(b"\x1b]9;4;0;0\x1b\\")
            .and_then(|()| err.flush());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(vars: &[(&str, &str)]) -> TermEnv {
        let mut e = TermEnv::from_lookup(|k| {
            vars.iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| (*v).to_owned())
        });
        e.stdout_tty = true;
        e.stdin_tty = true;
        e
    }

    #[test]
    fn overrides_layer_and_flag_wins() {
        let o = Overrides::resolve(Some("+progress,-keyboard"), Some("-progress,keyboard"))
            .expect("valid");
        assert_eq!(o.get(Protocol::Progress), Some(false));
        assert_eq!(o.get(Protocol::Keyboard), Some(true));
        assert_eq!(o.get(Protocol::Pointer), None);
    }

    #[test]
    fn minus_all_turns_everything_off() {
        let o = Overrides::resolve(None, Some("-all")).expect("valid");
        assert!(Protocol::ALL.iter().all(|p| o.get(*p) == Some(false)));
    }

    #[test]
    fn unknown_protocol_is_rejected() {
        let err = Overrides::resolve(None, Some("-sparkles")).unwrap_err();
        assert!(err.contains("sparkles"));
    }

    #[test]
    fn gate_blocks_non_terminals_agents_and_dumb() {
        let ok = env(&[("TERM", "xterm-256color")]);
        assert_eq!(ok.gate(), None);
        let mut piped = ok.clone();
        piped.stdout_tty = false;
        assert!(piped.gate().is_some());
        assert!(
            env(&[("TERM", "xterm"), ("AI_AGENT", "claude-code_2-1")])
                .gate()
                .is_some()
        );
        assert!(env(&[("TERM", "xterm"), ("CI", "true")]).gate().is_some());
        assert!(env(&[("TERM", "dumb")]).gate().is_some());
        assert!(env(&[("TERM", "linux")]).gate().is_some());
        assert!(env(&[]).gate().is_some());
    }

    #[test]
    fn progress_allow_list() {
        assert!(env(&[("TERM", "xterm"), ("WT_SESSION", "x")]).progress_allowed());
        assert!(env(&[("TERM", "xterm"), ("TERM_PROGRAM", "ghostty")]).progress_allowed());
        assert!(env(&[("TERM", "foot")]).progress_allowed());
        assert!(!env(&[("TERM", "xterm-256color")]).progress_allowed());
        assert!(!env(&[("TERM", "alacritty")]).progress_allowed());
        assert!(!env(&[("TERM", "xterm-kitty")]).progress_allowed());
    }

    #[test]
    fn wezterm_stable_is_refused_and_nightly_allowed() {
        let stable = env(&[
            ("TERM_PROGRAM", "WezTerm"),
            ("TERM_PROGRAM_VERSION", "20240203-110809-5046fc22"),
        ]);
        let nightly = env(&[
            ("TERM_PROGRAM", "WezTerm"),
            ("TERM_PROGRAM_VERSION", "20250301-000000-abc"),
        ]);
        assert!(!stable.progress_allowed());
        assert!(nightly.progress_allowed());
    }

    #[test]
    fn iterm_needs_3_6() {
        assert!(
            !env(&[
                ("TERM_PROGRAM", "iTerm.app"),
                ("TERM_PROGRAM_VERSION", "3.5.11")
            ])
            .progress_allowed()
        );
        assert!(
            env(&[
                ("TERM_PROGRAM", "iTerm.app"),
                ("TERM_PROGRAM_VERSION", "3.6.0")
            ])
            .progress_allowed()
        );
        assert!(!env(&[("TERM_PROGRAM", "iTerm.app")]).progress_allowed());
    }

    #[test]
    fn multiplexer_skips_progress() {
        assert!(
            !env(&[("TERM", "foot"), ("TMUX", "/tmp/tmux-1000/default,1,0")]).progress_allowed()
        );
    }

    #[test]
    fn user_can_force_progress_but_not_through_the_gate() {
        let on = Overrides::resolve(None, Some("+progress")).expect("valid");
        assert!(decide_progress(&env(&[("TERM", "xterm")]), on).is_on());
        assert!(!decide_progress(&env(&[("TERM", "xterm"), ("AI_AGENT", "x")]), on).is_on());
        let off = Overrides::resolve(None, Some("-progress")).expect("valid");
        assert!(!decide_progress(&env(&[("TERM", "foot")]), off).is_on());
    }

    #[test]
    fn parses_probe_replies() {
        assert_eq!(
            parse_replies(b"\x1b[?0u\x1b[?62;4c"),
            Replies {
                kitty_keyboard: true,
                da1: true
            }
        );
        assert_eq!(
            parse_replies(b"\x1b[?62;4c"),
            Replies {
                kitty_keyboard: false,
                da1: true
            }
        );
        assert_eq!(parse_replies(b"\x1b[?62;"), Replies::default());
        assert_eq!(parse_replies(b"abc\x1b[A"), Replies::default());
    }
}
