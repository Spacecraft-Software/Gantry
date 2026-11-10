// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>

//! Gantry — a themed, animated greetd greeter for the Linux console.
//!
//! Gantry runs as greetd's greeter on a bare VT (or inside a kiosk
//! terminal), draws a login screen, and relays the PAM conversation to
//! greetd. It never authenticates anyone itself.
//!
//! Alongside the greeter it ships a small dual-mode CLI (`session list`,
//! `user list`, `schema`, `describe`) that follows the Spacecraft CLI
//! Standard: JSON for agents, tagged text for people.
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
mod introspect;
mod keys;
mod linear;
mod output;
mod sessions;
mod state;
mod termproto;
mod theme;
mod ui;

use std::ffi::OsString;
use std::io::{self, IsTerminal as _, Write as _};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use clap::error::ErrorKind;
use clap::{Parser, Subcommand};
use ratatui::crossterm::event::{self, Event as TermEvent, KeyCode, KeyEventKind, KeyModifiers};
use serde_json::{Value, json};

use crate::app::{App, AppInit, Level, Phase};
use crate::config::Config;
use crate::output::{AppError, ErrorCode, Format, Listing, Reporter, Severity};
use crate::termproto::{Active, Overrides, TermEnv};
use crate::theme::{ColorMode, Registry, ThemeInputs};

const LONG_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    "\nMaintained by Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>",
    "\nProject: https://github.com/Spacecraft-Software/Gantry",
    "\nLicense: GPL-3.0-or-later"
);

/// Command line.
#[derive(Debug, Parser)]
#[command(
    name = "gantry",
    disable_version_flag = true,
    about,
    after_help = concat!(
        "Examples:\n",
        "  gantry --test                     try the greeter against a simulated greetd\n",
        "  gantry session list --json        sessions as a JSON envelope\n",
        "  gantry describe                   capability manifest for agents\n\n",
        "Gantry — https://github.com/Spacecraft-Software/Gantry\n",
        "Maintained by Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>"
    )
)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "one field per command-line switch"
)]
struct Cli {
    /// Run against a simulated greetd; nothing is authenticated or started.
    #[arg(long)]
    test: bool,
    /// With --test, simulate a fingerprint reader: F9 is a matching finger, F10 a wrong one.
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
    /// Force the full-screen interface (ignored when an agent is driving).
    #[arg(long)]
    no_accessible: bool,
    /// Start in the screensaver.
    #[arg(long)]
    screensaver: bool,
    /// Terminal protocol overrides, e.g. `-graphics,+progress`; names:
    /// keyboard, graphics, animation, clipboard, clipboard-rich, progress,
    /// pointer, all. `SPACECRAFT_TERM_PROTOCOLS` takes the same list; this flag wins.
    #[arg(long, global = true, value_name = "LIST", allow_hyphen_values = true)]
    term_protocols: Option<String>,
    /// Write a structured log to this file (never contains passwords).
    #[arg(long)]
    log: Option<PathBuf>,

    /// Print data as JSON (alias for `--format json`).
    #[arg(long, global = true)]
    json: bool,
    /// Output format: json, jsonl or csv.
    #[arg(long, global = true, value_name = "FORMAT")]
    format: Option<String>,
    /// Only these fields, comma-separated.
    #[arg(long, global = true, value_delimiter = ',', value_name = "FIELDS")]
    fields: Vec<String>,
    /// Report everything on stderr, including narration.
    #[arg(long, short, global = true, conflicts_with = "quiet")]
    verbose: bool,
    /// Report errors only.
    #[arg(long, short, global = true)]
    quiet: bool,
    /// Never colour diagnostics (same as `--color never`).
    #[arg(long, global = true)]
    no_color: bool,
    /// When to colour diagnostics.
    #[arg(long, global = true, value_name = "WHEN", value_parser = ["never", "always", "auto"])]
    color: Option<String>,
    /// Accepted for consistency; Gantry prints no relative times.
    #[arg(long, global = true)]
    absolute_time: bool,
    /// NUL-delimit the first field of each record, for `xargs -0`.
    #[arg(long, short = '0', global = true)]
    print0: bool,
    /// Accepted for consistency; no Gantry command writes or deletes.
    #[arg(long, global = true)]
    dry_run: bool,
    /// Accepted for consistency; no Gantry command asks for confirmation.
    #[arg(long, global = true)]
    yes: bool,
    /// Accepted for consistency; no Gantry command needs forcing.
    #[arg(long, global = true)]
    force: bool,
    /// Print version, maintainer and project URL.
    #[arg(long, short = 'V')]
    version: bool,
    #[command(subcommand)]
    command: Option<Sub>,
}

/// Non-interactive subcommands (Standard §18.2.3), noun then verb.
#[derive(Debug, Subcommand)]
enum Sub {
    /// Sessions the greeter can start.
    #[command(after_help = concat!(
        "Examples:\n  gantry session list\n  gantry session list --json --fields name,kind\n\n",
        "Gantry — https://github.com/Spacecraft-Software/Gantry\n",
        "Maintained by Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>"
    ))]
    Session {
        #[command(subcommand)]
        verb: Verb,
    },
    /// Local users that may log in.
    #[command(after_help = concat!(
        "Examples:\n  gantry user list\n  gantry user list --json --fields name,uid\n\n",
        "Gantry — https://github.com/Spacecraft-Software/Gantry\n",
        "Maintained by Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>"
    ))]
    User {
        #[command(subcommand)]
        verb: Verb,
    },
    /// Print JSON Schema for every command, or one noun or verb.
    #[command(after_help = concat!(
        "Examples:\n  gantry schema\n  gantry schema session list --format anthropic\n\n",
        "Gantry — https://github.com/Spacecraft-Software/Gantry\n",
        "Maintained by Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>"
    ))]
    Schema {
        /// Optional noun and verb.
        words: Vec<String>,
    },
    /// Print the capability manifest and how this run resolved its environment.
    #[command(after_help = concat!(
        "Examples:\n  gantry describe\n  gantry describe --term-protocols=-all\n\n",
        "Gantry — https://github.com/Spacecraft-Software/Gantry\n",
        "Maintained by Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>"
    ))]
    Describe,
    /// Legacy spelling of `session list`.
    #[command(hide = true)]
    Sessions,
    /// Legacy spelling of `user list`.
    #[command(hide = true)]
    Users,
}

/// The only verb Gantry's nouns have.
#[derive(Debug, Clone, Copy, Subcommand)]
enum Verb {
    /// Enumerate records.
    List,
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
    // Both modules must be named: `gail` or `atk` alone is not the AT-SPI bridge.
    let atk = std::env::var("GTK_MODULES").is_ok_and(|m| {
        let parts: Vec<&str> = m.split(':').collect();
        parts.contains(&"gail") && parts.contains(&"atk")
    });
    if dumb || atk {
        return (true, A11ySource::Hint);
    }
    (false, A11ySource::Unset)
}

fn init_logging(path: Option<&PathBuf>) -> Result<(), String> {
    let Some(path) = path else { return Ok(()) };
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| {
            format!(
                "cannot open log file `{}`: {e}",
                output::sanitize(&path.display().to_string())
            )
        })?;
    let _ = tracing_subscriber::fmt()
        .with_writer(std::sync::Mutex::new(file))
        .with_ansi(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("GANTRY_LOG")
                .unwrap_or_else(|_unset| "info".into()),
        )
        .try_init();
    Ok(())
}

/// Facts read off the raw arguments before clap runs, so even a usage error
/// can be reported in the right dialect.
struct Peek {
    command: String,
    machine: bool,
}

impl Peek {
    fn new(args: &[OsString]) -> Self {
        let words: Vec<String> = args
            .iter()
            .skip(1)
            .map(|a| output::sanitize(&a.to_string_lossy()))
            .collect();
        let machine = words
            .iter()
            .any(|w| w == "--json" || w == "--format" || w.starts_with("--format="));
        let command = std::iter::once(output::TOOL.to_owned())
            .chain(words)
            .collect::<Vec<_>>()
            .join(" ");
        Self { command, machine }
    }
}

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().collect();
    let peek = Peek::new(&args);
    let agent = output::agent_from_process();
    let cli = match Cli::try_parse_from(&args) {
        Ok(cli) => cli,
        Err(e) if matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) => {
            e.exit()
        }
        Err(e) => {
            let text = e.to_string();
            let first = text.lines().next().unwrap_or_default();
            let message = output::sanitize(first.strip_prefix("error: ").unwrap_or(first));
            let err = AppError::usage(message, "gantry --help");
            let reporter = Reporter::bootstrap(peek.machine || agent, peek.command);
            reporter.error(&err);
            return ExitCode::from(err.code.exit_code());
        }
    };

    let stdout_tty = io::stdout().is_terminal();
    let machine =
        cli.json || cli.format.is_some() || agent || (cli.command.is_some() && !stdout_tty);
    let reporter = Reporter {
        machine,
        floor: output::severity_floor(cli.quiet, cli.verbose, agent),
        color: output::color_from_process(cli.color.as_deref(), cli.no_color, agent),
        command: peek.command,
    };
    install_panic_report(&reporter);

    match run(&cli, &reporter, agent, stdout_tty) {
        Ok(Exit::Done) => ExitCode::SUCCESS,
        Ok(Exit::Signal(n)) => ExitCode::from(u8::try_from(128 + n).unwrap_or(1)),
        Err(err) => {
            reporter.error(&err);
            ExitCode::from(err.code.exit_code())
        }
    }
}

/// In machine mode a panic must still produce the structured error on
/// stderr and a non-zero exit. The payload is never echoed: it could hold
/// anything the program was handling.
fn install_panic_report(reporter: &Reporter) {
    if !reporter.machine {
        return;
    }
    let reporter = reporter.clone();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let place = info
            .location()
            .map_or_else(String::new, |l| format!(" at {}:{}", l.file(), l.line()));
        reporter.error(&AppError {
            code: ErrorCode::InternalError,
            message: format!("unexpected panic{place}"),
            hint: None,
        });
        previous(info);
        std::process::exit(i32::from(ErrorCode::InternalError.exit_code()));
    }));
}

/// How a successful run ended.
enum Exit {
    Done,
    Signal(i32),
}

fn run(cli: &Cli, reporter: &Reporter, agent: bool, stdout_tty: bool) -> Result<Exit, AppError> {
    // Accepted for consistency with every Spacecraft CLI; none applies here.
    let _ = (cli.absolute_time, cli.dry_run, cli.yes, cli.force);

    let term = TermEnv::from_process();
    let overrides = Overrides::resolve(
        std::env::var("SPACECRAFT_TERM_PROTOCOLS").ok().as_deref(),
        cli.term_protocols.as_deref(),
    )
    .map_err(|m| AppError::usage(m, "gantry describe"))?;

    if cli.version {
        return print_version(cli).map(|()| Exit::Done);
    }
    if let Err(message) = init_logging(cli.log.as_ref()) {
        reporter.emit(Severity::Warn, "CONFIG_WARNING", &message, None);
    }
    let (cfg, cfg_warning) = Config::load(&cli.config);
    let cfg_warning: Vec<String> = cfg_warning.into_iter().collect();

    if let Some(sub) = &cli.command {
        for w in &cfg_warning {
            reporter.emit(Severity::Warn, "CONFIG_WARNING", w, None);
        }
        let format = output::resolve_format(cli.json, cli.format.as_deref(), agent, stdout_tty)?;
        let report = introspect::RunReport {
            profile: output::profile_from_process(),
            output_format: match format {
                Format::Text => "text",
                Format::Json => "json",
                Format::Jsonl => "jsonl",
                Format::Csv => "csv",
            },
            color: reporter.color,
            tui_allowed: !agent,
            overrides,
            term,
        };
        run_subcommand(sub, cli, &cfg, format, stdout_tty, &report)?;
        return Ok(Exit::Done);
    }

    run_greeter(cli, reporter, agent, &cfg, &cfg_warning, &term, overrides)
}

fn print_version(cli: &Cli) -> Result<(), AppError> {
    let mut out = io::stdout().lock();
    if cli.json || cli.format.as_deref() == Some("json") {
        let doc = json!({
            "tool": output::TOOL,
            "version": env!("CARGO_PKG_VERSION"),
            "maintainer": output::MAINTAINER,
            "website": output::WEBSITE,
            "license": "GPL-3.0-or-later",
        });
        writeln!(out, "{doc}")?;
    } else {
        writeln!(out, "gantry {LONG_VERSION}")?;
    }
    Ok(())
}

#[expect(
    clippy::too_many_lines,
    reason = "linear setup of one run; splitting would only pass state around"
)]
fn run_greeter(
    cli: &Cli,
    reporter: &Reporter,
    agent: bool,
    cfg: &Config,
    cfg_warning: &[String],
    term: &TermEnv,
    overrides: Overrides,
) -> Result<Exit, AppError> {
    let (accessible, a11y_source) = resolve_accessible(cli, cfg);
    let registry = Registry::embedded()?;
    let inputs = ThemeInputs::from_system(cli.theme.clone(), cfg.theme.name.clone(), accessible);
    let mode = ColorMode::detect();
    if cli.verbose {
        let r = theme::resolve(&registry, &inputs);
        reporter.emit(
            Severity::Info,
            "THEME_RESOLVED",
            &format!(
                "theme `{}` (base `{}` from {}, overlay {}), colour mode {mode:?}",
                r.slug, r.base, r.base_source, r.overlay
            ),
            None,
        );
        reporter.emit(
            Severity::Info,
            "ACCESSIBLE_MODE",
            &format!("accessible mode {accessible} (decided by {a11y_source:?})"),
            None,
        );
    }

    // An agent must never be trapped in a redraw loop (CLI Standard §5).
    let mut go_linear = accessible;
    if agent && !accessible {
        reporter.emit(
            Severity::Warn,
            "TUI_FALLBACK",
            "full-screen interface unavailable while an agent is driving; using accessible linear mode",
            Some("gantry session list --json"),
        );
        go_linear = true;
    }
    if go_linear && !io::stdin().is_terminal() {
        return Err(AppError::new(
            ErrorCode::NotATty,
            "the greeter needs a terminal on standard input",
            "gantry session list --json",
        ));
    }

    let (keymap, key_warnings) = keys::Keymap::new(&cfg.keys);
    let sessions = sessions::discover(&cfg.general.default_session, &cfg.session_dirs);
    let users = sessions::users();
    let state = if cfg.general.remember && !cli.test {
        state::State::load(&cfg.general.cache_file)
    } else {
        state::State::default()
    };

    // `--test-fingerprint`: a simulated reader with pam_fprintd's real timing
    // (the module's 10 s window); F9/F10 stand in for touching it.
    let (touch_tx, touch_rx) = if cli.test_fingerprint {
        let (tx, rx) = mpsc::channel();
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };
    let backend: Box<dyn auth::Backend> = if cli.test {
        Box::new(auth::Mock::new(
            cli.test_fingerprint,
            Duration::from_secs(10),
            touch_rx,
        ))
    } else {
        Box::new(auth::Greetd::connect().map_err(|e| {
            AppError::new(
                ErrorCode::GreetdUnavailable,
                output::sanitize(&format!("{e:#}")),
                "gantry --test",
            )
        })?)
    };
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (ev_tx, ev_rx) = mpsc::channel();
    let worker = auth::spawn(backend, cmd_rx, ev_tx)?;

    let mut exit = Exit::Done;
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
            touches: touch_tx,
        });
        if let Some(w) = cfg_warning.iter().chain(&key_warnings).next() {
            app.status = Some((Level::Warn, w.clone()));
        }
        let ui = TuiSetup {
            term,
            overrides,
            probe_timeout: cfg.terminal.probe_timeout,
            reporter,
        };
        match run_tui(&mut app, &ev_rx, &ui)? {
            TuiEnd::Quit => {}
            TuiEnd::Linear => go_linear = true,
            TuiEnd::Signal(n) => exit = Exit::Signal(n),
        }
    }
    if go_linear {
        for w in cfg_warning.iter().chain(&key_warnings) {
            reporter.emit(Severity::Warn, "CONFIG_WARNING", w, None);
        }
        let default_user = state.user.as_deref().filter(|_| cfg.general.remember);
        linear::run(cfg, &sessions, default_user, &cmd_tx, &ev_rx, cli.test)?;
    }
    drop(cmd_tx);
    let _ = worker.join();
    Ok(exit)
}

fn run_subcommand(
    sub: &Sub,
    cli: &Cli,
    cfg: &Config,
    format: Format,
    stdout_tty: bool,
    report: &introspect::RunReport,
) -> Result<(), AppError> {
    let text = match sub {
        Sub::Session { verb: Verb::List } | Sub::Sessions => {
            let list = sessions::discover(&cfg.general.default_session, &cfg.session_dirs);
            let records = list
                .iter()
                .map(serde_json::to_value)
                .collect::<Result<Vec<_>, _>>()?;
            list_output(
                records,
                &introspect::SESSION_FIELDS,
                "session list",
                cli,
                format,
                stdout_tty,
            )?
        }
        Sub::User { verb: Verb::List } | Sub::Users => {
            let list = sessions::users();
            let records = list
                .iter()
                .map(serde_json::to_value)
                .collect::<Result<Vec<_>, _>>()?;
            list_output(
                records,
                &introspect::USER_FIELDS,
                "user list",
                cli,
                format,
                stdout_tty,
            )?
        }
        Sub::Schema { words } => {
            let doc = introspect::schema(words, cli.format.as_deref())?;
            json_document(&doc, stdout_tty)?
        }
        Sub::Describe => json_document(&introspect::describe(report), stdout_tty)?,
    };
    let mut out = io::stdout().lock();
    match out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
        // A closed pipe (`| head`) is the reader's choice, not a failure.
        Err(e) if e.kind() != io::ErrorKind::BrokenPipe => Err(e.into()),
        _ => Ok(()),
    }
}

fn json_document(doc: &Value, pretty: bool) -> Result<String, AppError> {
    let mut s = if pretty {
        serde_json::to_string_pretty(doc)?
    } else {
        serde_json::to_string(doc)?
    };
    s.push('\n');
    Ok(s)
}

fn list_output(
    mut records: Vec<Value>,
    allowed: &[&str],
    noun_verb: &str,
    cli: &Cli,
    format: Format,
    stdout_tty: bool,
) -> Result<String, AppError> {
    let columns: Vec<&str> = if cli.fields.is_empty() {
        allowed.to_vec()
    } else {
        output::select_fields(&mut records, &cli.fields, allowed, noun_verb)?;
        cli.fields.iter().map(String::as_str).collect()
    };
    let command = format!("gantry {noun_verb}");
    output::render_list(
        format,
        records,
        &Listing {
            command: &command,
            columns: &columns,
            pretty: stdout_tty && format == Format::Json,
            print0: cli.print0,
        },
    )
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

/// Restores the terminal however the UI exits: normal return, an `?`
/// error, or (through the panic hook) a panic. Order is the reverse of
/// setup: terminal protocols, then the kernel palette, then raw mode and
/// the alternate screen.
struct TuiGuard {
    mode: ColorMode,
}

impl TuiGuard {
    fn enter(mode: ColorMode, active: &Active) -> io::Result<(Self, ratatui::DefaultTerminal)> {
        let terminal = ratatui::try_init()?;
        // The keyboard stack is per screen: push after entering the alternate one.
        if active.keyboard.is_on() {
            termproto::push_keyboard();
        }
        // ratatui's hook restores raw mode; chain one that undoes the rest first.
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            termproto::restore();
            reset_palette(mode);
            previous(info);
        }));
        Ok((Self { mode }, terminal))
    }
}

impl Drop for TuiGuard {
    fn drop(&mut self) {
        termproto::restore();
        reset_palette(self.mode);
        ratatui::restore();
    }
}

/// What the full-screen interface needs to negotiate with the terminal.
struct TuiSetup<'a> {
    term: &'a TermEnv,
    overrides: Overrides,
    probe_timeout: Duration,
    reporter: &'a Reporter,
}

/// How the full-screen interface ended.
enum TuiEnd {
    Quit,
    Linear,
    Signal(i32),
}

/// Runs the full-screen UI.
fn run_tui(
    app: &mut App,
    events: &mpsc::Receiver<auth::Event>,
    setup: &TuiSetup<'_>,
) -> Result<TuiEnd, AppError> {
    // SIGTERM and SIGHUP must unwind through the guard; in raw mode Ctrl-C is
    // a key, so SIGINT only arrives from `kill`.
    let signalled = Arc::new(AtomicUsize::new(0));
    for sig in [
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGHUP,
        signal_hook::consts::SIGINT,
    ] {
        let value = usize::try_from(sig).unwrap_or(1);
        signal_hook::flag::register_usize(sig, Arc::clone(&signalled), value)?;
    }

    let active = termproto::negotiate(setup.term, setup.overrides, setup.probe_timeout);
    setup
        .reporter
        .emit(Severity::Info, "TERM_PROTOCOLS", &active.summary(), None);
    let (_guard, mut terminal) = TuiGuard::enter(app.styles.mode, &active)?;
    write_palette(app)?;
    let frame = Duration::from_millis(1000 / app::FPS);
    let slow = Duration::from_millis(500);
    let mut next = Instant::now();
    loop {
        let signal = signalled.load(Ordering::Relaxed);
        if signal != 0 {
            return Ok(TuiEnd::Signal(i32::try_from(signal).unwrap_or(15)));
        }
        let now = Instant::now();
        if now >= next {
            let size = terminal.size()?;
            app.on_tick(
                now,
                ratatui::layout::Rect::new(0, 0, size.width, size.height),
            );
            drain(app, events);
            if app.quit {
                return Ok(TuiEnd::Quit);
            }
            if active.progress.is_on() {
                // Taskbar progress is decorative; the screen already says "Checking".
                termproto::set_busy(matches!(
                    app.phase,
                    Phase::Waiting | Phase::Checking | Phase::Starting
                ));
            }
            terminal.draw(|f| ui::draw(f, app))?;
            // Full rate only while something moves; a still screen wakes twice a second.
            next = now + if app.is_animating() { frame } else { slow };
        }
        if event::poll(next.saturating_duration_since(Instant::now()))? {
            match event::read()? {
                TermEvent::Key(k) if k.kind == KeyEventKind::Release => {}
                TermEvent::Key(k)
                    if app.test_mode
                        && k.code == KeyCode::Char('c')
                        && k.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    return Ok(TuiEnd::Quit);
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
                        return Ok(TuiEnd::Linear);
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
