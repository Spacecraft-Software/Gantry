// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>

//! `gantry schema` and `gantry describe`: the CLI describes itself, so an
//! agent never needs a man page.
//!
//! `schema` emits JSON Schema (Draft 2020-12) per command, shaped so the
//! `parameters` object drops straight into `Anthropic`, `OpenAI`, `Gemini` or `MCP`
//! tool definitions. `describe` is the compact manifest an agent fetches
//! first. Both are read-only and safe to run anywhere.

use serde_json::{Value, json};

use crate::output::{AppError, ErrorCode, MAINTAINER, TOOL, WEBSITE};
use crate::termproto::{self, Overrides, Protocol, TermEnv};

/// Fields of a `session list` record, in output order.
pub const SESSION_FIELDS: [&str; 5] = ["name", "exec", "kind", "desktop_names", "source"];
/// Fields of a `user list` record, in output order.
pub const USER_FIELDS: [&str; 3] = ["name", "full_name", "uid"];

const SCHEMA_DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";

/// Non-error diagnostics Gantry can emit, documented for `schema`.
const DIAGNOSTIC_CODES: [(&str, &str); 5] = [
    (
        "TUI_FALLBACK",
        "warn: an agent is driving, so the accessible linear mode replaces the full-screen interface",
    ),
    (
        "CONFIG_WARNING",
        "warn: the configuration file or a key binding was ignored",
    ),
    (
        "THEME_RESOLVED",
        "info: which theme was chosen and why (`--verbose`)",
    ),
    (
        "ACCESSIBLE_MODE",
        "info: whether accessible mode is on and which source decided (`--verbose`)",
    ),
    (
        "TERM_PROTOCOLS",
        "info: which terminal protocols are in use (`--verbose`)",
    ),
];

/// One entry of the command tree.
struct Spec {
    /// Words after the tool name; empty for the greeter itself.
    words: &'static [&'static str],
    description: &'static str,
    parameters: Value,
    data_item: Value,
    examples: Vec<(&'static str, &'static str)>,
    supports_json: bool,
}

impl Spec {
    fn full(&self) -> String {
        std::iter::once(TOOL)
            .chain(self.words.iter().copied())
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn function_name(&self) -> String {
        if self.words.is_empty() {
            TOOL.to_owned()
        } else {
            self.words.join("_")
        }
    }
}

fn list_parameters(fields: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": {
            "format": {
                "type": "string",
                "enum": ["json", "jsonl", "csv"],
                "description": "Output format. Default: json when piped or under an agent, text on a terminal."
            },
            "fields": {
                "type": "array",
                "items": { "type": "string", "enum": fields },
                "description": "Only these fields, in this order (comma-separated on the command line)."
            },
            "print0": {
                "type": "boolean",
                "default": false,
                "description": "Print the first field of each record NUL-delimited, for xargs -0."
            }
        }
    })
}

#[expect(
    clippy::too_many_lines,
    reason = "the command tree is one table of data"
)]
fn catalog() -> Vec<Spec> {
    let session_item = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string", "description": "Display name." },
            "exec": { "type": "array", "items": { "type": "string" }, "description": "Command and arguments." },
            "kind": { "type": "string", "enum": ["wayland", "x11", "tty"], "description": "Session kind." },
            "desktop_names": { "type": "string", "description": "DesktopNames= value; omitted when unset." },
            "source": { "type": "string", "description": ".desktop file the session came from; omitted when none." }
        },
        "required": ["name", "exec", "kind"]
    });
    let user_item = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string", "description": "Login name." },
            "full_name": { "type": "string", "description": "GECOS full name; omitted when unset." },
            "uid": { "type": "integer", "minimum": 0, "description": "Numeric user ID." }
        },
        "required": ["name", "uid"]
    });
    vec![
        Spec {
            words: &[],
            description: "Run the greetd greeter: a themed full-screen login, or accessible linear mode.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "test": { "type": "boolean", "default": false, "description": "Use a simulated greetd; nothing is authenticated or started." },
                    "test_fingerprint": { "type": "boolean", "default": false, "description": "With test, simulate a fingerprint reader." },
                    "accessible": { "type": "boolean", "default": false, "description": "Force accessible linear mode." },
                    "no_accessible": { "type": "boolean", "default": false, "description": "Force the full-screen interface (ignored under an agent)." },
                    "screensaver": { "type": "boolean", "default": false, "description": "Start in the screensaver." },
                    "theme": { "type": "string", "description": "Theme slug; overrides config and SPACECRAFT_THEME." },
                    "config": { "type": "string", "format": "uri-reference", "default": crate::config::DEFAULT_PATH, "description": "Configuration file." },
                    "log": { "type": "string", "format": "uri-reference", "description": "Structured log file; never contains passwords." },
                    "term_protocols": {
                        "type": "string",
                        "pattern": "^[+-]?[a-z-]+(,[+-]?[a-z-]+)*$",
                        "description": "Terminal protocol overrides such as -graphics,+progress; SPACECRAFT_TERM_PROTOCOLS has the same syntax and the flag wins."
                    }
                }
            }),
            data_item: json!({ "type": "null", "description": "The greeter is interactive and returns no data." }),
            examples: vec![
                (
                    "gantry --test",
                    "Try the full-screen greeter against a simulated greetd",
                ),
                (
                    "gantry --test --accessible",
                    "Try accessible linear mode; an agent gets this automatically",
                ),
            ],
            supports_json: false,
        },
        Spec {
            words: &["session", "list"],
            description: "List the sessions the greeter can start.",
            parameters: list_parameters(&SESSION_FIELDS),
            data_item: session_item,
            examples: vec![
                ("gantry session list", "List sessions as text on a terminal"),
                (
                    "gantry session list --json --fields name,kind",
                    "Names and kinds as a JSON envelope",
                ),
            ],
            supports_json: true,
        },
        Spec {
            words: &["user", "list"],
            description: "List the local users that may log in.",
            parameters: list_parameters(&USER_FIELDS),
            data_item: user_item,
            examples: vec![
                ("gantry user list", "List users as text on a terminal"),
                (
                    "gantry user list --json --fields name,uid",
                    "Names and UIDs as a JSON envelope",
                ),
            ],
            supports_json: true,
        },
        Spec {
            words: &["schema"],
            description: "Print JSON Schema for every command, or one noun or verb.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "array", "items": { "type": "string" }, "description": "Optional noun and verb, e.g. session list." },
                    "format": { "type": "string", "enum": ["json", "anthropic", "openai", "gemini", "mcp"], "default": "json", "description": "json is the raw schema; the others wrap it as tool definitions." }
                }
            }),
            data_item: json!({ "type": "object", "description": "A JSON Schema document." }),
            examples: vec![
                ("gantry schema", "Every command"),
                (
                    "gantry schema session list --format anthropic",
                    "One command as an Anthropic tool definition",
                ),
            ],
            supports_json: true,
        },
        Spec {
            words: &["describe"],
            description: "Print the capability manifest and how this run resolved its environment.",
            parameters: json!({ "type": "object", "properties": {} }),
            data_item: json!({ "type": "object", "description": "The manifest." }),
            examples: vec![
                ("gantry describe", "The manifest"),
                (
                    "gantry describe --term-protocols=-all",
                    "The manifest with every terminal protocol forced off",
                ),
            ],
            supports_json: true,
        },
    ]
}

fn exit_codes() -> Value {
    let mut map = serde_json::Map::new();
    map.insert("0".into(), json!("SUCCESS"));
    let mut codes = ErrorCode::ALL.to_vec();
    codes.sort_by_key(|c| c.exit_code());
    for c in codes {
        map.insert(
            c.exit_code().to_string(),
            json!(format!("{} — {}", c.as_str(), c.meaning())),
        );
    }
    Value::Object(map)
}

fn command_schema(s: &Spec) -> Value {
    json!({
        "$schema": SCHEMA_DIALECT,
        "command": s.full(),
        "description": s.description,
        "parameters": s.parameters,
        "output_schema": {
            "type": "object",
            "properties": {
                "metadata": {
                    "type": "object",
                    "properties": {
                        "tool": { "type": "string" },
                        "version": { "type": "string" },
                        "command": { "type": "string" },
                        "timestamp": { "type": "string", "description": "ISO 8601 UTC with Z suffix." },
                        "maintainer": { "type": "string" },
                        "website": { "type": "string" }
                    }
                },
                "data": { "type": "array", "items": s.data_item }
            }
        },
        "exit_codes": exit_codes(),
        "examples": s.examples.iter().map(|(c, d)| json!({ "command": c, "description": d })).collect::<Vec<_>>(),
        "supports_json": s.supports_json,
        "supports_dry_run": false,
        "idempotent": true,
        "destructive": false,
    })
}

fn wrap(s: &Spec, provider: &str) -> Value {
    let name = s.function_name();
    match provider {
        "anthropic" => {
            json!({ "name": name, "description": s.description, "input_schema": s.parameters })
        }
        "openai" => {
            json!({ "type": "function", "function": { "name": name, "description": s.description, "parameters": s.parameters } })
        }
        "gemini" => {
            json!({ "name": name, "description": s.description, "parameters": s.parameters })
        }
        _ => json!({ "name": name, "description": s.description, "inputSchema": s.parameters }),
    }
}

/// Builds the output of `gantry schema [noun [verb]] [--format ...]`.
///
/// # Errors
/// The words name no command, or the format is not one `schema` offers.
pub fn schema(words: &[String], format: Option<&str>) -> Result<Value, AppError> {
    let provider = match format {
        None | Some("json") => None,
        Some(p @ ("anthropic" | "openai" | "gemini" | "mcp")) => Some(p),
        Some(other) => {
            return Err(AppError::usage(
                format!("unknown schema format `{}`", crate::output::sanitize(other)),
                "gantry schema --format anthropic",
            ));
        }
    };
    let all = catalog();
    let picked: Vec<&Spec> = all
        .iter()
        .filter(|s| !s.words.is_empty() || words.is_empty())
        .filter(|s| s.words.len() >= words.len() && s.words.iter().zip(words).all(|(a, b)| a == b))
        .collect();
    if picked.is_empty() {
        return Err(AppError::new(
            ErrorCode::NotFound,
            format!("no command `{}`", crate::output::sanitize(&words.join(" "))),
            "gantry describe",
        ));
    }
    if let Some(p) = provider {
        let tools: Vec<Value> = picked.iter().map(|s| wrap(s, p)).collect();
        return Ok(if p == "gemini" {
            json!([{ "function_declarations": tools }])
        } else {
            Value::Array(tools)
        });
    }
    if let [single] = picked.as_slice() {
        if !words.is_empty() && single.words.len() == words.len() {
            return Ok(command_schema(single));
        }
    }
    Ok(json!({
        "$schema": SCHEMA_DIALECT,
        "tool": TOOL,
        "version": env!("CARGO_PKG_VERSION"),
        "commands": picked.iter().map(|s| command_schema(s)).collect::<Vec<_>>(),
        "error_codes": ErrorCode::ALL.iter().map(|c| json!({
            "code": c.as_str(), "exit_code": c.exit_code(), "meaning": c.meaning()
        })).collect::<Vec<_>>(),
        "diagnostic_codes": DIAGNOSTIC_CODES.iter().map(|(c, m)| json!({ "code": c, "meaning": m })).collect::<Vec<_>>(),
    }))
}

/// How this particular run resolved its environment.
#[derive(Debug, Clone)]
pub struct RunReport {
    /// `agent`, `ci` or `human`.
    pub profile: &'static str,
    /// The format a data command would use right now.
    pub output_format: &'static str,
    /// Whether diagnostics are coloured.
    pub color: bool,
    /// Whether the full-screen interface may start.
    pub tui_allowed: bool,
    /// The user's protocol overrides.
    pub overrides: Overrides,
    /// The terminal as seen by this process.
    pub term: TermEnv,
}

fn protocol_state(report: &RunReport, p: Protocol) -> String {
    if let Some(reason) = report.term.gate() {
        return format!("off: {reason}");
    }
    match (p, report.overrides.get(p)) {
        (_, Some(false)) => "off: switched off by the user".to_owned(),
        (Protocol::Progress, _) => match termproto::decide_progress(&report.term, report.overrides)
        {
            termproto::State::On => "on".to_owned(),
            termproto::State::Off(why) => format!("off: {why}"),
        },
        (_, Some(true)) => "on".to_owned(),
        _ => "probed at startup".to_owned(),
    }
}

fn protocols(report: &RunReport) -> Value {
    let mut out = serde_json::Map::new();
    for p in Protocol::ALL {
        let setting = match report.overrides.get(p) {
            Some(true) => "on",
            Some(false) => "off",
            None => "auto",
        };
        let mut entry = serde_json::Map::new();
        entry.insert("implemented".into(), json!(p.not_used_because().is_none()));
        entry.insert("setting".into(), json!(setting));
        if let Some(why) = p.not_used_because() {
            entry.insert("not_used_because".into(), json!(why));
        } else {
            entry.insert("state".into(), json!(protocol_state(report, p)));
        }
        out.insert(p.name().into(), Value::Object(entry));
    }
    Value::Object(out)
}

/// Builds the output of `gantry describe`.
#[must_use]
pub fn describe(report: &RunReport) -> Value {
    json!({
        "tool": TOOL,
        "version": env!("CARGO_PKG_VERSION"),
        "description": "A themed greetd greeter for the Linux console and kiosk terminals",
        "license": "GPL-3.0-or-later",
        "homepage": WEBSITE,
        "maintainer": MAINTAINER,
        "commands": catalog().iter().map(|s| json!({
            "name": s.full(),
            "description": s.description,
            "supports_json": s.supports_json,
            "supports_dry_run": false,
            "idempotent": true,
            "destructive": false,
        })).collect::<Vec<_>>(),
        "global_flags": [
            "--json", "--format", "--fields", "--dry-run", "--verbose", "--quiet",
            "--no-color", "--color", "--help", "--version", "--absolute-time",
            "--print0", "--yes", "--force"
        ],
        "output_formats": ["json", "jsonl", "csv"],
        "mcp_available": false,
        "schema_command": "gantry schema",
        "context_files": ["CLAUDE.md", "AGENTS.md", "SKILL.md", "CONTRIBUTING.md"],
        "environment": {
            "profile": report.profile,
            "output_format": report.output_format,
            "color": report.color,
            "interactive": report.tui_allowed,
            "tui_allowed": report.tui_allowed,
        },
        "terminal_protocols": protocols(report),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_schema_lists_every_command_once() {
        let doc = schema(&[], None).unwrap();
        let names: Vec<&str> = doc["commands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["command"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "gantry",
                "gantry session list",
                "gantry user list",
                "gantry schema",
                "gantry describe"
            ]
        );
        assert_eq!(doc["$schema"], SCHEMA_DIALECT);
    }

    #[test]
    fn every_command_documents_json_example_and_exit_codes() {
        for s in catalog() {
            assert!(!s.examples.is_empty(), "{}", s.full());
            if s.supports_json {
                assert!(
                    s.examples.iter().any(|(c, _)| c.contains("--json")
                        || c.starts_with("gantry schema")
                        || c.starts_with("gantry describe")),
                    "{} needs a JSON example",
                    s.full()
                );
            }
            let doc = command_schema(&s);
            assert_eq!(doc["exit_codes"]["0"], "SUCCESS");
            assert!(doc["exit_codes"]["2"].is_string());
        }
    }

    #[test]
    fn every_parameter_has_a_type_and_description() {
        for s in catalog() {
            for (name, p) in s.parameters["properties"].as_object().unwrap() {
                assert!(p["type"].is_string(), "{} {name}: type", s.full());
                assert!(
                    p["description"].is_string(),
                    "{} {name}: description",
                    s.full()
                );
            }
        }
    }

    #[test]
    fn noun_selects_its_verbs_and_verb_selects_one() {
        let noun = schema(&["session".into()], None).unwrap();
        assert_eq!(noun["commands"].as_array().unwrap().len(), 1);
        let one = schema(&["session".into(), "list".into()], None).unwrap();
        assert_eq!(one["command"], "gantry session list");
    }

    #[test]
    fn unknown_command_is_not_found_with_a_hint() {
        let e = schema(&["nope".into()], None).unwrap_err();
        assert_eq!(e.code, ErrorCode::NotFound);
        assert_eq!(e.hint.as_deref(), Some("gantry describe"));
    }

    #[test]
    fn provider_wrappers() {
        let w = ["session".to_owned(), "list".to_owned()];
        assert!(schema(&w, Some("anthropic")).unwrap()[0]["input_schema"].is_object());
        assert!(schema(&w, Some("openai")).unwrap()[0]["function"]["parameters"].is_object());
        assert!(
            schema(&w, Some("gemini")).unwrap()[0]["function_declarations"][0]["parameters"]
                .is_object()
        );
        assert!(schema(&w, Some("mcp")).unwrap()[0]["inputSchema"].is_object());
        assert_eq!(
            schema(&w, Some("toml")).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn describe_reports_every_protocol() {
        let report = RunReport {
            profile: "agent",
            output_format: "json",
            color: false,
            tui_allowed: false,
            overrides: Overrides::default(),
            term: TermEnv::default(),
        };
        let d = describe(&report);
        for p in Protocol::ALL {
            assert!(
                d["terminal_protocols"][p.name()].is_object(),
                "{}",
                p.name()
            );
        }
        assert_eq!(
            d["terminal_protocols"]["keyboard"]["state"],
            "off: not a terminal"
        );
        assert_eq!(d["environment"]["profile"], "agent");
    }
}
