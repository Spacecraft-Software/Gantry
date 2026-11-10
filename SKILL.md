---
name: gantry
description: >
  Inspect and test Gantry, the Steelbore OS greetd greeter. Use to list the
  sessions and users the greeter offers (`gantry session list`, `gantry user
  list`), read its JSON Schema or capability manifest (`gantry schema`,
  `gantry describe`), or run the greeter against a simulated greetd (`gantry
  --test`). Every data command speaks JSON with a stable envelope, structured
  errors with runnable hints, and documented exit codes. The login screen
  itself is interactive; agents get accessible linear mode automatically.
license: GPL-3.0-or-later
---

<!--
SPDX-License-Identifier: GPL-3.0-or-later
SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
-->

# Gantry

Gantry is a themed greetd greeter for Steelbore OS Bravais. This file is the
capability surface of its command line. Run `gantry describe` for the live
manifest and `gantry schema` for JSON Schema (Draft 2020-12).

## Commands

| Command | Returns |
|---------|---------|
| `gantry session list` | Startable sessions: `name`, `exec`, `kind`, `desktop_names`, `source` |
| `gantry user list` | Local users that may log in: `name`, `full_name`, `uid` |
| `gantry schema [noun [verb]]` | JSON Schema; `--format anthropic\|openai\|gemini\|mcp` wraps it as tool definitions |
| `gantry describe` | Capability manifest, plus how this run resolved agent mode, colour and terminal protocols |
| `gantry --test [--accessible]` | The interactive greeter against a simulated greetd |

All commands are read-only and idempotent.

## Output

- Under an agent (`AI_AGENT` or `AGENT` set to any non-empty value, or a
  truthy `CI`) or when stdout is piped, data commands print one compact JSON
  envelope: `{"metadata": {...}, "data": [...]}`. On a terminal they print
  `field: value` text.
- `--format json|jsonl|csv`, `--fields a,b`, `--print0`. `yaml` and `explore`
  are not offered and return `INVALID_ARGUMENT`.
- Stdout carries data only. Diagnostics go to stderr, one JSON line each in
  machine mode, `[ERROR]`/`[WARN]`/`[OK]`/`[INFO]` lines otherwise.

## Errors and exit codes

Machine-mode errors are one line on stderr:
`{"error": {"code", "exit_code", "message", "hint", "timestamp", "command"}}`.
`hint` is a command you can run.

| Exit | Code | Meaning |
|------|------|---------|
| 0 | | success |
| 1 | `INTERNAL_ERROR` | unexpected failure |
| 2 | `INVALID_ARGUMENT` | usage error; do not retry unchanged |
| 3 | `NOT_FOUND` | no such command |
| 6 | `GREETD_UNAVAILABLE` | `GREETD_SOCK` unset or refused |
| 7 | `NOT_A_TTY` | the greeter needs a terminal on stdin |
| 128+N | | killed by signal N, terminal restored first |

## The greeter under an agent

`gantry` with no subcommand never starts the full-screen interface while an
agent is driving: it emits a `TUI_FALLBACK` warning and runs accessible linear
mode, which needs a terminal on stdin. To read data, use the commands above.
