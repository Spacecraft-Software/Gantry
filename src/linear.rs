// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>

//! Linear accessible mode (Standard §18.2.2, §18.2.3).
//!
//! Append-only and line-based: no alternate screen, no redraws, no art and
//! no animation. Every prompt states its question, choices and default in
//! plain text, so Speakup or BRLTTY on the console read it in order.
//!
//! Raw mode is used only to read keys without echo; all output is plain
//! lines ending in CR LF.

use std::io::{self, Write as _};
use std::sync::mpsc::{Receiver, Sender};

use ratatui::crossterm::event::{self, Event as TermEvent, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::crossterm::terminal;
use zeroize::Zeroizing;

use crate::auth::{Command, Event, MessageKind};
use crate::config::Config;
use crate::fingerprint::{self, Signal};
use crate::sessions::Session;

/// Restores cooked mode when dropped, even on an early return.
struct RawGuard;

impl RawGuard {
    fn new() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}

fn say(line: &str) -> io::Result<()> {
    let mut out = io::stdout().lock();
    write!(out, "{line}\r\n")?;
    out.flush()
}

/// Reads one line. `echo = false` prints nothing as keys arrive.
/// Returns `None` on Esc (or Ctrl+C in test mode).
fn read_line(prompt: &str, echo: bool, test_mode: bool) -> io::Result<Option<Zeroizing<String>>> {
    let mut out = io::stdout().lock();
    write!(out, "{prompt}")?;
    out.flush()?;
    // Preallocated so a growing password is never copied and freed.
    let mut buf = Zeroizing::new(String::with_capacity(crate::app::FIELD_MAX + 4));
    loop {
        let TermEvent::Key(k) = event::read()? else {
            continue;
        };
        if k.kind == KeyEventKind::Release {
            continue;
        }
        match k.code {
            KeyCode::Enter => {
                write!(out, "\r\n")?;
                out.flush()?;
                return Ok(Some(buf));
            }
            KeyCode::Esc => {
                write!(out, "\r\n")?;
                return Ok(None);
            }
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) && test_mode => {
                return Ok(None);
            }
            KeyCode::Backspace => {
                if buf.pop().is_some() && echo {
                    write!(out, "\x08 \x08")?;
                }
            }
            KeyCode::Char(c)
                if !k.modifiers.contains(KeyModifiers::CONTROL)
                    && buf.len() + c.len_utf8() <= crate::app::FIELD_MAX =>
            {
                buf.push(c);
                if echo {
                    write!(out, "{c}")?;
                }
            }
            _ => {}
        }
        out.flush()?;
    }
}

/// Runs the linear greeter until a session starts.
///
/// # Errors
/// Terminal I/O failure.
pub fn run(
    cfg: &Config,
    sessions: &[Session],
    default_user: Option<&str>,
    commands: &Sender<Command>,
    events: &Receiver<Event>,
    test_mode: bool,
) -> anyhow::Result<()> {
    let _raw = RawGuard::new()?;
    say("")?;
    say(&format!(
        "{} — Gantry login, accessible mode.",
        cfg.general.greeting
    ))?;
    say("Commands at the user prompt: :sessions to choose a session, :shutdown, :reboot.")?;
    if test_mode {
        say("[INFO] Test mode: no real login happens.")?;
    }
    let mut session = 0usize;
    loop {
        let prompt = match default_user {
            Some(u) => format!("User name (default {u}): "),
            None => "User name: ".to_owned(),
        };
        let Some(line) = read_line(&prompt, true, test_mode)? else {
            if test_mode {
                return Ok(());
            }
            continue;
        };
        let typed = line.trim();
        match typed {
            ":sessions" => {
                session = choose_session(sessions, session, test_mode)?;
                continue;
            }
            ":shutdown" | ":reboot" => {
                let cmd = if typed == ":shutdown" {
                    &cfg.power.shutdown
                } else {
                    &cfg.power.reboot
                };
                if test_mode {
                    say(&format!("[INFO] Test mode: would run {}", cmd.join(" ")))?;
                } else if let Some((bin, args)) = cmd.split_first() {
                    let status = std::process::Command::new(bin).args(args).status();
                    if !status.is_ok_and(|s| s.success()) {
                        say("[ERROR] The power command failed.")?;
                    }
                }
                continue;
            }
            _ => {}
        }
        let user = if typed.is_empty() {
            default_user.unwrap_or_default().to_owned()
        } else {
            typed.to_owned()
        };
        if user.is_empty() {
            say("[WARN] Type a user name first.")?;
            continue;
        }
        if converse(
            cfg,
            sessions.get(session),
            &user,
            commands,
            events,
            test_mode,
        )? {
            say("[OK] Starting your session.")?;
            return Ok(());
        }
    }
}

fn choose_session(sessions: &[Session], current: usize, test_mode: bool) -> anyhow::Result<usize> {
    say(&format!("Sessions available: {}.", sessions.len()))?;
    for (i, s) in sessions.iter().enumerate() {
        say(&format!("  {}: {}", i + 1, s.name))?;
    }
    let prompt = format!("Session number (default {}): ", current + 1);
    let Some(line) = read_line(&prompt, true, test_mode)? else {
        return Ok(current);
    };
    let picked = line
        .trim()
        .parse::<usize>()
        .ok()
        .filter(|n| (1..=sessions.len()).contains(n))
        .map(|n| n - 1);
    let chosen = picked.unwrap_or(current);
    if let Some(s) = sessions.get(chosen) {
        say(&format!("Session: {}", s.name))?;
    }
    Ok(chosen)
}

/// One PAM conversation. Returns true when the session started.
fn converse(
    cfg: &Config,
    session: Option<&Session>,
    user: &str,
    commands: &Sender<Command>,
    events: &Receiver<Event>,
    test_mode: bool,
) -> anyhow::Result<bool> {
    commands.send(Command::Begin {
        username: user.to_owned(),
    })?;
    for ev in events {
        match ev {
            Event::Message { kind, text } => {
                let tag = match (fingerprint::classify(&cfg.fingerprint, &text), kind) {
                    (Some(Signal::Scanning), _) => "[INFO] Fingerprint:",
                    (Some(Signal::Retry), _) => "[WARN] Fingerprint:",
                    (_, MessageKind::Error) => "[ERROR]",
                    _ => "[INFO]",
                };
                say(&format!("{tag} {text}"))?;
            }
            Event::Prompt { text, secret } => {
                let hint = if secret { " (typing is not shown)" } else { "" };
                let Some(answer) = read_line(&format!("{text}{hint} "), !secret, test_mode)? else {
                    commands.send(Command::Cancel)?;
                    say("[INFO] Cancelled.")?;
                    return Ok(false);
                };
                commands.send(Command::Answer(answer))?;
            }
            Event::Authenticated => {
                let Some(s) = session else {
                    say("[ERROR] No session is configured.")?;
                    commands.send(Command::Cancel)?;
                    return Ok(false);
                };
                let (cmd, env) = s.launch(&cfg.general.xsession_wrapper);
                commands.send(Command::Start { cmd, env })?;
            }
            Event::Started => return Ok(true),
            Event::Denied(reason) => {
                say(&format!("[ERROR] Login failed. {reason}"))?;
                return Ok(false);
            }
            Event::Failed(reason) => {
                say(&format!("[ERROR] greetd: {reason}"))?;
                return Ok(false);
            }
        }
    }
    Ok(false)
}
