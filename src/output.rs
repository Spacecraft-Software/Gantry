// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>

//! The dual-mode CLI surface (`spacecraft-cli-standard`, `spacecraft-agentic-cli`).
//!
//! Gantry's non-interactive commands serve two readers: a person at a
//! terminal and an agent paying for tokens. This module holds what both
//! share: agent detection, the output-format cascade, the JSON envelope,
//! severity-tagged diagnostics on stderr, and structured errors with exit
//! codes and runnable hints.
//!
//! Standard output carries data only. Everything else goes to standard
//! error, one line per diagnostic.

use std::fmt;
use std::fmt::Write as _;
use std::io::{self, IsTerminal as _, Write as _};

use serde::Serialize;
use serde_json::{Map, Value, json};

/// The tool's name as registered.
pub const TOOL: &str = "gantry";
/// Maintainer line required in `--version` output.
pub const MAINTAINER: &str = "Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>";
/// Project page.
pub const WEBSITE: &str = "https://github.com/Spacecraft-Software/Gantry";

/// True when an AI agent or CI runner drives the process.
///
/// Presence-based: `AI_AGENT` and `AGENT` count when set to any non-empty
/// value (a live harness exports a descriptive string, not `1`). `CI` is
/// the one value-carrying variable: it counts unless empty, `0` or `false`.
#[must_use]
pub fn is_agent_env(get: &impl Fn(&str) -> Option<String>) -> bool {
    let present = |k: &str| get(k).is_some_and(|v| !v.is_empty());
    let ci = get("CI").is_some_and(|v| !matches!(v.to_lowercase().as_str(), "" | "0" | "false"));
    present("AI_AGENT") || present("AGENT") || ci
}

/// [`is_agent_env`] on the real environment.
#[must_use]
pub fn agent_from_process() -> bool {
    is_agent_env(&|k| std::env::var(k).ok())
}

/// Which kind of caller the environment suggests, for `describe`.
#[must_use]
pub fn profile_from_process() -> &'static str {
    let get = |k: &str| std::env::var(k).ok();
    let present = |k: &str| get(k).is_some_and(|v| !v.is_empty());
    if present("AI_AGENT") || present("AGENT") {
        "agent"
    } else if is_agent_env(&get) {
        "ci"
    } else {
        "human"
    }
}

/// Current time as ISO 8601 UTC with the mandatory `Z`, whole seconds.
#[must_use]
pub fn timestamp() -> String {
    jiff::Timestamp::from_second(jiff::Timestamp::now().as_second()).map_or_else(
        |_unrepresentable| "1970-01-01T00:00:00Z".to_owned(),
        |t| t.to_string(),
    )
}

// ---------------------------------------------------------------- severity

/// The four message severities, lowest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Narration, shown under `--verbose`.
    Info,
    /// A side effect completed.
    Ok,
    /// Something degraded or fell back.
    Warn,
    /// The command failed.
    Error,
}

impl Severity {
    /// The text tag. Always printed: colour is never the only signal.
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::Info => "[INFO]",
            Self::Ok => "[OK]",
            Self::Warn => "[WARN]",
            Self::Error => "[ERROR]",
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }

    /// 4-bit ANSI, the `steelbore-mono` approach: the user's own terminal
    /// palette decides the actual colour.
    const fn sgr(self) -> &'static str {
        match self {
            Self::Info => "34",
            Self::Ok => "32",
            Self::Warn => "1;35",
            Self::Error => "1;31",
        }
    }
}

/// The minimum severity written to stderr.
#[must_use]
pub const fn severity_floor(quiet: bool, verbose: bool, agent: bool) -> Severity {
    if quiet {
        Severity::Error
    } else if verbose {
        Severity::Info
    } else if agent {
        Severity::Warn
    } else {
        Severity::Ok
    }
}

// ------------------------------------------------------------------ errors

/// Stable error codes, with the exit code each maps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// Bad invocation.
    InvalidArgument,
    /// The named resource does not exist.
    NotFound,
    /// Unexpected failure.
    InternalError,
    /// Not running under greetd.
    GreetdUnavailable,
    /// An interactive command was run without a terminal.
    NotATty,
}

impl ErrorCode {
    /// Every code Gantry can emit, for `schema`.
    pub const ALL: [Self; 5] = [
        Self::InvalidArgument,
        Self::NotFound,
        Self::InternalError,
        Self::GreetdUnavailable,
        Self::NotATty,
    ];

    /// The `error.code` string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidArgument => "INVALID_ARGUMENT",
            Self::NotFound => "NOT_FOUND",
            Self::InternalError => "INTERNAL_ERROR",
            Self::GreetdUnavailable => "GREETD_UNAVAILABLE",
            Self::NotATty => "NOT_A_TTY",
        }
    }

    /// The process exit code. 2 and 3 follow the canonical map; 6 and 7 are
    /// Gantry's own and are documented in `schema`.
    #[must_use]
    pub const fn exit_code(self) -> u8 {
        match self {
            Self::InternalError => 1,
            Self::InvalidArgument => 2,
            Self::NotFound => 3,
            Self::GreetdUnavailable => 6,
            Self::NotATty => 7,
        }
    }

    /// What the exit code means, for `schema`.
    #[must_use]
    pub const fn meaning(self) -> &'static str {
        match self {
            Self::InternalError => "unexpected failure",
            Self::InvalidArgument => "usage error; fix the invocation, do not retry",
            Self::NotFound => "the named command, session or user does not exist",
            Self::GreetdUnavailable => "GREETD_SOCK is unset or refuses the connection",
            Self::NotATty => "the greeter needs a terminal on standard input",
        }
    }
}

/// A failed command: what happened, and the exact command to try next.
#[derive(Debug, Clone)]
pub struct AppError {
    /// Stable code.
    pub code: ErrorCode,
    /// One sentence, lowercase first word, no trailing period.
    pub message: String,
    /// A runnable command that resolves or investigates the error.
    pub hint: Option<String>,
}

impl AppError {
    /// Builds an error with a hint.
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            hint: Some(hint.into()),
        }
    }

    /// A usage error.
    #[must_use]
    pub fn usage(message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidArgument, message, hint)
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for AppError {}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        Self {
            code: ErrorCode::InternalError,
            message: lowercase_first(&format!("{e:#}")),
            hint: None,
        }
    }
}

impl From<io::Error> for AppError {
    fn from(e: io::Error) -> Self {
        Self {
            code: ErrorCode::InternalError,
            message: lowercase_first(&e.to_string()),
            hint: None,
        }
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        Self {
            code: ErrorCode::InternalError,
            message: lowercase_first(&e.to_string()),
            hint: None,
        }
    }
}

fn lowercase_first(s: &str) -> String {
    let s = s.trim().trim_end_matches('.');
    let mut chars = s.chars();
    match chars.next() {
        // Keep acronyms (GREETD_SOCK, PAM) intact.
        Some(c) if c.is_lowercase() || chars.clone().next().is_some_and(char::is_uppercase) => {
            s.to_owned()
        }
        Some(c) => c.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Removes control characters so an echoed argument cannot inject escapes.
#[must_use]
pub fn sanitize(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

// ---------------------------------------------------------------- reporter

/// Writes diagnostics to stderr in the right dialect.
#[derive(Debug, Clone)]
pub struct Reporter {
    /// One-line JSON instead of tagged text.
    pub machine: bool,
    /// Lowest severity written.
    pub floor: Severity,
    /// 4-bit ANSI on the tag.
    pub color: bool,
    /// The invocation, as parsed, echoed in machine diagnostics.
    pub command: String,
}

impl Reporter {
    /// The reporter used before the command line is understood.
    #[must_use]
    pub fn bootstrap(machine: bool, command: String) -> Self {
        Self {
            machine,
            floor: Severity::Ok,
            color: false,
            command,
        }
    }

    /// Emits a non-error diagnostic if it clears the floor.
    pub fn emit(&self, severity: Severity, code: &str, message: &str, hint: Option<&str>) {
        if severity < self.floor {
            return;
        }
        let line = if self.machine {
            let mut d = Map::new();
            d.insert("severity".into(), json!(severity.name()));
            d.insert("code".into(), json!(code));
            d.insert("message".into(), json!(message));
            if let Some(h) = hint {
                d.insert("hint".into(), json!(h));
            }
            d.insert("timestamp".into(), json!(timestamp()));
            d.insert("command".into(), json!(self.command));
            json!({ "diagnostic": d }).to_string()
        } else {
            self.human(severity, message, hint)
        };
        write_stderr(&line);
    }

    /// Emits an error. Never suppressed by the floor.
    pub fn error(&self, err: &AppError) {
        let line = if self.machine {
            let hint = err
                .hint
                .clone()
                .unwrap_or_else(|| format!("{} --verbose", self.command));
            json!({ "error": {
                "code": err.code.as_str(),
                "exit_code": err.code.exit_code(),
                "message": err.message,
                "hint": hint,
                "timestamp": timestamp(),
                "command": self.command,
            }})
            .to_string()
        } else {
            self.human(Severity::Error, &err.message, err.hint.as_deref())
        };
        write_stderr(&line);
    }

    fn human(&self, severity: Severity, message: &str, hint: Option<&str>) -> String {
        let tag = if self.color {
            format!("\x1b[{}m{}\x1b[0m", severity.sgr(), severity.tag())
        } else {
            severity.tag().to_owned()
        };
        let mut out = format!("{tag} {message}");
        if let Some(h) = hint {
            out.push_str("\n  hint: ");
            out.push_str(h);
        }
        out
    }
}

fn write_stderr(line: &str) {
    let mut err = io::stderr().lock();
    let _ = writeln!(err, "{line}");
    let _ = err.flush();
}

/// `--color` / `--no-color` / environment, in the standard's precedence:
/// explicit flag, then agent (off), `NO_COLOR` (off), `FORCE_COLOR` (on),
/// `TERM=dumb` (off), then whether stderr is a terminal.
#[must_use]
pub fn color_enabled(
    when: Option<&str>,
    no_color: bool,
    agent: bool,
    get: &impl Fn(&str) -> Option<String>,
    stderr_tty: bool,
) -> bool {
    if no_color || when == Some("never") {
        return false;
    }
    if when == Some("always") {
        return true;
    }
    if agent || get("NO_COLOR").is_some_and(|v| !v.is_empty()) {
        return false;
    }
    if get("FORCE_COLOR").is_some_and(|v| !matches!(v.as_str(), "" | "0")) {
        return true;
    }
    if get("TERM").as_deref() == Some("dumb") {
        return false;
    }
    stderr_tty
}

/// [`color_enabled`] on the real environment.
#[must_use]
pub fn color_from_process(when: Option<&str>, no_color: bool, agent: bool) -> bool {
    color_enabled(
        when,
        no_color,
        agent,
        &|k| std::env::var(k).ok(),
        io::stderr().is_terminal(),
    )
}

// ------------------------------------------------------------------ format

/// How data is written to stdout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// `key: value` lines, readable with POSIX tools.
    Text,
    /// One envelope document.
    Json,
    /// One record per line.
    Jsonl,
    /// RFC 4180.
    Csv,
}

/// Resolves the format by the standard's cascade: explicit flag, agent
/// variable, TTY, non-TTY.
///
/// # Errors
/// The requested format is unknown or not offered by Gantry.
pub fn resolve_format(
    json_flag: bool,
    format: Option<&str>,
    agent: bool,
    stdout_tty: bool,
) -> Result<Format, AppError> {
    if json_flag {
        return Ok(Format::Json);
    }
    match format {
        Some("json") => Ok(Format::Json),
        Some("jsonl") => Ok(Format::Jsonl),
        Some("csv") => Ok(Format::Csv),
        Some(f @ ("yaml" | "explore")) => Err(AppError::usage(
            format!("format `{f}` is not offered by gantry"),
            "gantry session list --format json",
        )),
        Some(other) => Err(AppError::usage(
            format!("unknown format `{}`", sanitize(other)),
            "gantry session list --format json",
        )),
        None if agent || !stdout_tty => Ok(Format::Json),
        None => Ok(Format::Text),
    }
}

/// Drops null members, so no token is spent on `"field": null`.
fn strip_nulls(record: &mut Value) {
    if let Value::Object(map) = record {
        map.retain(|_, v| !v.is_null());
    }
}

/// Keeps only `fields`, in the order given.
///
/// # Errors
/// A field is not one of `allowed`.
pub fn select_fields(
    records: &mut [Value],
    fields: &[String],
    allowed: &[&str],
    command: &str,
) -> Result<(), AppError> {
    if let Some(bad) = fields.iter().find(|f| !allowed.contains(&f.as_str())) {
        return Err(AppError::usage(
            format!("unknown field `{}`", sanitize(bad)),
            format!("gantry schema {command}"),
        ));
    }
    for rec in records {
        if let Value::Object(map) = rec {
            let mut kept = Map::new();
            for f in fields {
                if let Some(v) = map.get(f) {
                    kept.insert(f.clone(), v.clone());
                }
            }
            *map = kept;
        }
    }
    Ok(())
}

fn scalar(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Array(items) => items.iter().map(scalar).collect::<Vec<_>>().join(" "),
        other => other.to_string(),
    }
}

fn csv_cell(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

/// What to print for a list command.
#[derive(Debug, Clone)]
pub struct Listing<'a> {
    /// The invocation, for the envelope.
    pub command: &'a str,
    /// Column order for CSV; also the order for an unfiltered record.
    pub columns: &'a [&'a str],
    /// Pretty-print JSON (a terminal is reading).
    pub pretty: bool,
    /// NUL-delimit the first column instead of using `format`.
    pub print0: bool,
}

/// Renders `records` in `format`.
///
/// # Errors
/// Serialization failure.
pub fn render_list(
    format: Format,
    mut records: Vec<Value>,
    spec: &Listing<'_>,
) -> Result<String, AppError> {
    records.iter_mut().for_each(strip_nulls);
    let columns: Vec<String> = spec.columns.iter().map(|c| (*c).to_owned()).collect();
    if spec.print0 {
        let key = columns.first().map_or("name", String::as_str);
        let mut out = String::new();
        for r in &records {
            out.push_str(&scalar(r.get(key).unwrap_or(&Value::Null)));
            out.push('\0');
        }
        return Ok(out);
    }
    Ok(match format {
        Format::Json => {
            let doc = json!({
                "metadata": {
                    "tool": TOOL,
                    "version": env!("CARGO_PKG_VERSION"),
                    "command": spec.command,
                    "timestamp": timestamp(),
                    "maintainer": MAINTAINER,
                    "website": WEBSITE,
                },
                "data": records,
            });
            let mut s = if spec.pretty {
                serde_json::to_string_pretty(&doc)?
            } else {
                serde_json::to_string(&doc)?
            };
            s.push('\n');
            s
        }
        Format::Jsonl => {
            let mut s = String::new();
            for r in &records {
                s.push_str(&serde_json::to_string(r)?);
                s.push('\n');
            }
            s
        }
        Format::Csv => {
            let mut s = columns
                .iter()
                .map(|c| csv_cell(c))
                .collect::<Vec<_>>()
                .join(",");
            s.push('\n');
            for r in &records {
                let row: Vec<String> = columns
                    .iter()
                    .map(|c| csv_cell(&scalar(r.get(c).unwrap_or(&Value::Null))))
                    .collect();
                s.push_str(&row.join(","));
                s.push('\n');
            }
            s
        }
        Format::Text => {
            let mut s = String::new();
            for r in &records {
                for c in &columns {
                    if let Some(v) = r.get(c) {
                        let _ = writeln!(s, "{c}: {}", scalar(v));
                    }
                }
                s.push('\n');
            }
            s
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| {
            vars.iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    #[test]
    fn agent_detection_is_presence_based() {
        assert!(is_agent_env(&lookup(&[(
            "AI_AGENT",
            "claude-code_2-1-218_agent"
        )])));
        assert!(is_agent_env(&lookup(&[("AGENT", "x")])));
        assert!(is_agent_env(&lookup(&[("CI", "true")])));
        assert!(!is_agent_env(&lookup(&[("AI_AGENT", "")])));
        assert!(!is_agent_env(&lookup(&[("CI", "false")])));
        assert!(!is_agent_env(&lookup(&[("CI", "0")])));
        assert!(!is_agent_env(&lookup(&[("CLAUDECODE", "1")])));
        assert!(!is_agent_env(&lookup(&[])));
    }

    #[test]
    fn severity_floor_follows_flags_then_agent() {
        assert_eq!(severity_floor(false, false, false), Severity::Ok);
        assert_eq!(severity_floor(false, false, true), Severity::Warn);
        assert_eq!(severity_floor(true, false, true), Severity::Error);
        assert_eq!(severity_floor(false, true, true), Severity::Info);
    }

    #[test]
    fn format_cascade() {
        assert_eq!(
            resolve_format(false, None, false, true).unwrap(),
            Format::Text
        );
        assert_eq!(
            resolve_format(false, None, false, false).unwrap(),
            Format::Json
        );
        assert_eq!(
            resolve_format(false, None, true, true).unwrap(),
            Format::Json
        );
        assert_eq!(
            resolve_format(false, Some("csv"), true, true).unwrap(),
            Format::Csv
        );
        assert_eq!(
            resolve_format(true, Some("csv"), false, true).unwrap(),
            Format::Json
        );
        let e = resolve_format(false, Some("yaml"), false, true).unwrap_err();
        assert_eq!(e.code.exit_code(), 2);
        assert!(e.hint.is_some());
    }

    #[test]
    fn colour_precedence() {
        let none = lookup(&[]);
        assert!(!color_enabled(None, false, false, &none, false));
        assert!(color_enabled(None, false, false, &none, true));
        assert!(!color_enabled(None, false, true, &none, true));
        assert!(color_enabled(Some("always"), false, true, &none, false));
        assert!(!color_enabled(Some("always"), true, false, &none, true));
        assert!(!color_enabled(
            None,
            false,
            false,
            &lookup(&[("NO_COLOR", "1")]),
            true
        ));
        assert!(color_enabled(
            None,
            false,
            false,
            &lookup(&[("FORCE_COLOR", "1")]),
            false
        ));
        assert!(!color_enabled(
            None,
            false,
            false,
            &lookup(&[("TERM", "dumb")]),
            true
        ));
    }

    fn sample() -> Vec<Value> {
        vec![
            json!({"name": "Shell", "exec": ["nu", "--login"], "kind": "tty", "source": null}),
            json!({"name": "a,\"b\"", "exec": ["x"], "kind": "x11", "source": null}),
        ]
    }

    fn spec(print0: bool) -> Listing<'static> {
        Listing {
            command: "gantry session list",
            columns: &["name", "exec", "kind", "source"],
            pretty: false,
            print0,
        }
    }

    #[test]
    fn json_envelope_is_compact_and_null_free() {
        let out = render_list(Format::Json, sample(), &spec(false)).unwrap();
        assert_eq!(out.matches('\n').count(), 1);
        let doc: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(doc["metadata"]["tool"], "gantry");
        assert!(
            doc["metadata"]["timestamp"]
                .as_str()
                .unwrap()
                .ends_with('Z')
        );
        assert!(doc["data"][0].get("source").is_none());
    }

    #[test]
    fn jsonl_is_one_record_per_line() {
        let out = render_list(Format::Jsonl, sample(), &spec(false)).unwrap();
        assert_eq!(out.lines().count(), 2);
        assert!(
            out.lines()
                .all(|l| serde_json::from_str::<Value>(l).is_ok())
        );
    }

    #[test]
    fn csv_quotes_and_joins_arrays() {
        let out = render_list(Format::Csv, sample(), &spec(false)).unwrap();
        let mut lines = out.lines();
        assert_eq!(lines.next(), Some("name,exec,kind,source"));
        assert_eq!(lines.next(), Some("Shell,nu --login,tty,"));
        assert_eq!(lines.next(), Some("\"a,\"\"b\"\"\",x,x11,"));
    }

    #[test]
    fn text_is_key_value_lines() {
        let out = render_list(Format::Text, sample(), &spec(false)).unwrap();
        assert!(out.starts_with("name: Shell\nexec: nu --login\nkind: tty\n\n"));
    }

    #[test]
    fn print0_emits_first_column() {
        let out = render_list(Format::Text, sample(), &spec(true)).unwrap();
        assert_eq!(out, "Shell\0a,\"b\"\0");
    }

    #[test]
    fn fields_filter_and_reject_unknown() {
        let mut recs = sample();
        select_fields(
            &mut recs,
            &["kind".into(), "name".into()],
            &["name", "kind"],
            "session list",
        )
        .unwrap();
        assert_eq!(recs[0], json!({"kind": "tty", "name": "Shell"}));
        let e = select_fields(&mut recs, &["nope".into()], &["name"], "session list").unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidArgument);
        assert_eq!(e.hint.as_deref(), Some("gantry schema session list"));
    }

    #[test]
    fn error_codes_are_unique_and_outside_reserved_bands() {
        let codes: Vec<u8> = ErrorCode::ALL.iter().map(|c| c.exit_code()).collect();
        let mut sorted = codes.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), codes.len());
        assert!(codes.iter().all(|c| !(126..=255).contains(c)));
    }

    #[test]
    fn message_style() {
        assert_eq!(lowercase_first("Cannot open file."), "cannot open file");
        assert_eq!(
            lowercase_first("GREETD_SOCK is not set"),
            "GREETD_SOCK is not set"
        );
    }

    #[test]
    fn sanitize_strips_escapes() {
        assert_eq!(sanitize("a\x1b[31mb\n"), "a[31mb");
    }
}
