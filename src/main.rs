// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>

//! Gantry — a themed, animated greetd greeter for the Linux console.
//!
//! Gantry runs as greetd's greeter on a bare VT (or inside a kiosk
//! terminal), draws a login screen, and relays the PAM conversation to
//! greetd. It never authenticates anyone itself.
//!
//! Memory allocator: the system allocator is kept on purpose. A login
//! screen allocates almost nothing per frame, and swapping in mimalloc
//! would add a C dependency to an otherwise all-Rust binary.

mod app;
mod art;
mod auth;
mod caps;
mod config;
mod fingerprint;
mod fx;
mod keys;
mod linear;
mod sessions;
mod state;
mod theme;
mod ui;

use std::io::{self, Write as _};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use ratatui::crossterm::event::{self, Event as TermEvent, KeyCode, KeyModifiers};

use crate::app::{App, AppInit, Level};
use crate::config::Config;
use crate::theme::{ColorMode, Registry, ThemeInputs};

const LONG_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    "\nMaintainer: Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>",
    "\nProject: https://github.com/Spacecraft-Software/Gantry",
    "\nLicense: GPL-3.0-or-later"
);

/// Command line.
#[derive(Debug, Parser)]
#[command(name = "gantry", version, long_version = LONG_VERSION, about)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "one field per command-line switch"
)]
struct Cli {
    /// Run against a simulated greetd; nothing is authenticated or started.
    #[arg(long)]
    test: bool,
    /// With --test, simulate a fingerprint reader (user names containing "finger" match).
    #[arg(long, requires = "test")]
    test_fingerprint: bool,
    /// Configuration file.
    #[arg(long, global = true, default_value = config::DEFAULT_PATH)]
    config: PathBuf,
    /// Theme slug (overrides config and `SPACECRAFT_THEME`).
    #[arg(long)]
    theme: Option<String>,
    /// Force accessible linear mode.
    #[arg(long, overrides_with = "no_accessible")]
    accessible: bool,
    /// Force the full-screen interface.
    #[arg(long)]
    no_accessible: bool,
    /// Start in the screensaver.
    #[arg(long)]
    screensaver: bool,
    /// Report theme and accessibility resolution on stderr.
    #[arg(long, short)]
    verbose: bool,
    /// Write a structured log to this file (never contains passwords).
    #[arg(long)]
    log: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Sub>,
}

/// Non-interactive subcommands (Standard §18.2.3).
#[derive(Debug, Subcommand)]
enum Sub {
    /// List startable sessions.
    Sessions {
        /// Print JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// List local users that may log in.
    Users {
        /// Print JSON instead of text.
        #[arg(long)]
        json: bool,
    },
}

/// Which §18.1 source decided accessible mode.
#[derive(Debug, Clone, Copy)]
enum A11ySource {
    Flag,
    Environment,
    Config,
    Hint,
    Unset,
}

fn resolve_accessible(cli: &Cli, cfg: &Config) -> (bool, A11ySource) {
    if cli.accessible {
        return (true, A11ySource::Flag);
    }
    if cli.no_accessible {
        return (false, A11ySource::Flag);
    }
    match std::env::var("SPACECRAFT_A11Y").as_deref() {
        Ok("1") => return (true, A11ySource::Environment),
        Ok("0") => return (false, A11ySource::Environment),
        _ => {}
    }
    if let Some(on) = cfg.accessibility.enabled {
        return (on, A11ySource::Config);
    }
    // Unambiguous hints only; NO_COLOR alone selects the mono theme instead.
    let dumb = std::env::var("TERM").is_ok_and(|t| t == "dumb");
    let atk = std::env::var("GTK_MODULES").is_ok_and(|m| m.contains("gail:atk"));
    if dumb || atk {
        return (true, A11ySource::Hint);
    }
    (false, A11ySource::Unset)
}

fn init_logging(path: Option<&PathBuf>) {
    let Some(path) = path else { return };
    let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        eprintln!("[WARN] cannot open log file {}", path.display());
        return;
    };
    let _ = tracing_subscriber::fmt()
        .with_writer(std::sync::Mutex::new(file))
        .with_ansi(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("GANTRY_LOG")
                .unwrap_or_else(|_unset| "info".into()),
        )
        .try_init();
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    init_logging(cli.log.as_ref());
    let (cfg, cfg_warning) = Config::load(&cli.config);

    if let Some(sub) = &cli.command {
        return run_subcommand(sub, &cfg);
    }

    let (accessible, a11y_source) = resolve_accessible(&cli, &cfg);
    let registry = Registry::embedded()?;
    let inputs = ThemeInputs::from_system(cli.theme.clone(), cfg.theme.name.clone(), accessible);
    let mode = ColorMode::detect();
    if cli.verbose {
        let r = theme::resolve(&registry, &inputs);
        eprintln!(
            "[INFO] theme {} (base {} from {}, overlay {}), colour mode {mode:?}",
            r.slug, r.base, r.base_source, r.overlay
        );
        eprintln!("[INFO] accessible mode {accessible} (decided by {a11y_source:?})");
    }

    let (keymap, key_warnings) = keys::Keymap::new(&cfg.keys);
    let sessions = sessions::discover(&cfg.general.default_session, &cfg.session_dirs);
    let users = sessions::users();
    let state = if cfg.general.remember && !cli.test {
        state::State::load(&cfg.general.cache_file)
    } else {
        state::State::default()
    };

    let backend: Box<dyn auth::Backend> = if cli.test {
        Box::new(auth::Mock::new(
            cli.test_fingerprint,
            Duration::from_millis(1500),
        ))
    } else {
        Box::new(auth::Greetd::connect()?)
    };
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (ev_tx, ev_rx) = mpsc::channel();
    let worker = auth::spawn(backend, cmd_rx, ev_tx)?;

    let mut go_linear = accessible;
    if !go_linear {
        let mut app = App::new(AppInit {
            cfg: cfg.clone(),
            keymap,
            registry,
            theme_inputs: inputs,
            mode,
            sessions: sessions.clone(),
            users,
            state: state.clone(),
            commands: cmd_tx.clone(),
            test_mode: cli.test,
            screensaver: cli.screensaver,
            now: Instant::now(),
        });
        if let Some(w) = cfg_warning.iter().chain(&key_warnings).next() {
            app.status = Some((Level::Warn, w.clone()));
        }
        go_linear = run_tui(&mut app, &ev_rx)?;
    }
    if go_linear {
        let default_user = state.user.as_deref().filter(|_| cfg.general.remember);
        linear::run(&cfg, &sessions, default_user, &cmd_tx, &ev_rx, cli.test)?;
    }
    drop(cmd_tx);
    let _ = worker.join();
    Ok(())
}

fn run_subcommand(sub: &Sub, cfg: &Config) -> anyhow::Result<()> {
    let mut out = io::stdout().lock();
    match sub {
        Sub::Sessions { json } => {
            let list = sessions::discover(&cfg.general.default_session, &cfg.session_dirs);
            if *json {
                writeln!(out, "{}", serde_json::to_string_pretty(&list)?)?;
            } else {
                for s in list {
                    writeln!(
                        out,
                        "name: {}\nkind: {:?}\nexec: {}\n",
                        s.name,
                        s.kind,
                        s.exec.join(" ")
                    )?;
                }
            }
        }
        Sub::Users { json } => {
            let list = sessions::users();
            if *json {
                writeln!(out, "{}", serde_json::to_string_pretty(&list)?)?;
            } else {
                for u in list {
                    writeln!(out, "name: {}\nuid: {}\n", u.name, u.uid)?;
                }
            }
        }
    }
    Ok(())
}

/// Loads or resets the VT palette; a no-op outside the Linux console.
fn write_palette(app: &App) -> io::Result<()> {
    let mut out = io::stdout().lock();
    match &app.styles.vt_slots {
        Some(slots) => write!(out, "{}", theme::vt_palette_sequence(slots))?,
        None if app.styles.mode == ColorMode::LinuxVt => {
            write!(out, "{}", theme::VT_PALETTE_RESET)?;
        }
        None => {}
    }
    out.flush()
}

fn reset_palette(mode: ColorMode) {
    if mode == ColorMode::LinuxVt {
        let mut out = io::stdout().lock();
        let _ = write!(out, "{}", theme::VT_PALETTE_RESET);
        let _ = out.flush();
    }
}

/// Restores the terminal and the kernel palette however the UI exits:
/// normal return, an `?` error, or (through the panic hook) a panic.
struct TuiGuard {
    mode: ColorMode,
}

impl TuiGuard {
    fn enter(mode: ColorMode) -> io::Result<(Self, ratatui::DefaultTerminal)> {
        let terminal = ratatui::try_init()?;
        // ratatui's hook restores raw mode; chain one that also resets the palette.
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            reset_palette(mode);
            previous(info);
        }));
        Ok((Self { mode }, terminal))
    }
}

impl Drop for TuiGuard {
    fn drop(&mut self) {
        reset_palette(self.mode);
        ratatui::restore();
    }
}

/// Runs the full-screen UI. Returns true when the user asked for linear mode.
fn run_tui(app: &mut App, events: &mpsc::Receiver<auth::Event>) -> anyhow::Result<bool> {
    let (_guard, mut terminal) = TuiGuard::enter(app.styles.mode)?;
    write_palette(app)?;
    let frame = Duration::from_millis(1000 / app::FPS);
    let slow = Duration::from_millis(500);
    let mut next = Instant::now();
    loop {
        let now = Instant::now();
        if now >= next {
            let size = terminal.size()?;
            app.on_tick(
                now,
                ratatui::layout::Rect::new(0, 0, size.width, size.height),
            );
            drain(app, events);
            if app.quit {
                return Ok(false);
            }
            terminal.draw(|f| ui::draw(f, app))?;
            // Full rate only while something moves; a still screen wakes twice a second.
            next = now + if app.is_animating() { frame } else { slow };
        }
        if event::poll(next.saturating_duration_since(Instant::now()))? {
            match event::read()? {
                TermEvent::Key(k)
                    if app.test_mode
                        && k.code == KeyCode::Char('c')
                        && k.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    return Ok(false);
                }
                TermEvent::Key(k) => {
                    // Apply any prompt that arrived since the last frame, so the
                    // first keys of a fast-typed password land in the right field.
                    drain(app, events);
                    let fx = app.on_key(k, Instant::now());
                    if fx.reload_palette {
                        write_palette(app)?;
                    }
                    if fx.go_linear {
                        return Ok(true);
                    }
                    if let Some(cmd) = fx.power {
                        run_power(app, &cmd);
                    }
                    next = Instant::now();
                }
                _ => next = Instant::now(),
            }
        }
    }
}

fn drain(app: &mut App, events: &mpsc::Receiver<auth::Event>) {
    while let Ok(ev) = events.try_recv() {
        app.on_auth(ev);
    }
}

/// Runs a power command without leaving raw mode, so keys typed meanwhile
/// cannot signal Gantry or echo onto the console.
fn run_power(app: &mut App, cmd: &[String]) {
    let Some((bin, args)) = cmd.split_first() else {
        return;
    };
    let status = std::process::Command::new(bin)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    let line = cmd.join(" ");
    app.status = Some(match status {
        Ok(s) if s.success() => (Level::Info, format!("Ran {line}")),
        Ok(s) => (Level::Error, format!("{line} exited with {s}")),
        Err(e) => (Level::Error, format!("{line}: {e}")),
    });
}
