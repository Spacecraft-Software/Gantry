# Gantry — agent context

Gantry is a Rust greetd greeter (ratatui + crossterm) for Steelbore OS Bravais. It follows The Steelbore Standard v2.12. The registered name is Gantry; its short code is `GNT`. Assurance: Category B, with the planned Rust PAM module raised to Category A (§19). The fprintd D-Bus probe stays off by default.

## Build and check

- `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
- Run it with `cargo run -- --test` (mock greetd). Add `--test-fingerprint` for a simulated reader, or `--accessible` for linear mode.
- Nix: `nix build`; the NixOS module is in `nix/module.nix`.

## Layout

- `src/app.rs`: the state machine. Pure: no terminal I/O, unit-tested.
- `src/auth.rs`: the greetd worker thread and the mock backend.
- `src/ui.rs`: rendering. `src/linear.rs`: accessible mode.
- `src/theme.rs`: palette registry and §11.6 resolution. It reads `assets/steelbore.toml`, which is never edited by hand.
- `src/art.rs`: banners. On Steelbore OS (`[general] brand`, `auto` reads `ID=` from os-release) the built-in banner is `assets/steelbore-logo.txt`, with `steelbore-wordmark.txt` for short screens. Regenerate both with `tools/logo2art.py`; never edit them by hand.
- `src/fx/`: background effects, banner effects and the clock font.
- `src/output.rs`: the dual-mode CLI surface (agent detection, format cascade, JSON envelope, severity-tagged diagnostics, structured errors and exit codes). `src/introspect.rs`: `schema` and `describe`.
- `src/termproto.rs`: terminal protocols (kitty keyboard, OSC 9;4 progress), their gate, probe and restore.
- `src/config.rs`, `src/keys.rs`, `src/sessions.rs`, `src/state.rs`, `src/caps.rs`, `src/fingerprint.rs`.

## Rules

- `#![forbid(unsafe_code)]`. No `unwrap`/`expect` outside tests.
- Never log, print or keep a password. Answers travel in `Zeroizing<String>`.
- Colours come only from theme roles. Never hard-code a hex value.
- Every status line carries a text tag (`[ERROR]`, `[WARN]`, `[INFO]`, `[OK]`).
- Art and glyphs must be VT-safe: ASCII, CP437 box drawing and blocks (see `art::is_vt_safe`).
- Every file starts with the two SPDX lines (GPL-3.0-or-later).
- Commits are signed through the host's existing setup. Never push or open a PR outside `Spacecraft-Software` / `UnbreakableMJ` without the maintainer's explicit approval.

## CLI surface (CLI Standard v1.1.0)

- Commands are noun-verb and singular: `session list`, `user list`, `schema`, `describe`. `sessions` and `users` are hidden legacy spellings. Add new commands to the catalog in `src/introspect.rs` too, or `schema` and `describe` go stale (a test checks the list).
- Output cascade: `--json`/`--format` wins, then an agent variable (`AI_AGENT` or `AGENT` set to any non-empty value, or truthy `CI`), then TTY, then JSON for a pipe. Detection is presence-based, never `== "1"`. Tests must inject the environment through a closure, because developers often run tests under an agent harness that exports `AI_AGENT`.
- stdout is data only. Everything else goes through `Reporter`: one JSON line per diagnostic in machine mode, tagged text otherwise. Severity floor: `--quiet` error, agent warn, default ok, `--verbose` info.
- Every failure is an `AppError` with a code, one lowercase sentence, and a runnable `hint`. Exit codes: 1 internal, 2 usage, 3 not found, 6 `GREETD_UNAVAILABLE`, 7 `NOT_A_TTY`; 126-255 are reserved. Document new codes in `ErrorCode::ALL`.
- `--dry-run`, `--yes`, `--force`, `--absolute-time` are accepted no-ops: nothing here writes, deletes or prints relative times. `--format yaml|explore` is refused on purpose.
- With an agent driving, bare `gantry` never starts the TUI: it warns `TUI_FALLBACK` and runs linear mode.

## Terminal protocols

- Implemented: kitty keyboard (flag 1 only) and OSC 9;4 indeterminate progress during Waiting/Checking/Starting. Not used, by design: kitty graphics and animation (the banner is VT-safe text), OSC 52 and OSC 5522 clipboard (nothing is copied), OSC 22 pointer (no mouse regions). OSC 1337 clipboard is ruled out.
- Gate first: nothing is sent unless stdout and stdin are TTYs, no agent is running, and `TERM` is not unset, `dumb` or `linux`. The Linux VT therefore gets no protocol traffic at all.
- The keyboard is probed once, before the alternate screen, with `CSI ? u` fenced by DA1, under `[terminal] probe_timeout` (default `PT0.2S`). Late bytes are drained. Probing uses `rustix` poll on stdin, so there is still no `unsafe`.
- OSC 9;4 is allow-listed by terminal identity (see `TermEnv::progress_allowed`); kitty is not on the list until its first supporting version is recorded. Unknown terminals get nothing, because OSC 9 elsewhere means "desktop notification".
- `--term-protocols=-graphics,+progress` and `SPACECRAFT_TERM_PROTOCOLS` override detection; the flag is applied after the variable. Forcing on never bypasses the gate.
- Restore order is the reverse of setup: pop keyboard flags, clear progress, reset VT palette, leave the alternate screen and raw mode. It runs from `Drop`, the panic hook, and the SIGTERM/SIGHUP/SIGINT path (exit 128+N). Any new protocol mode must register in `termproto::restore`.

## Accessibility notes

- Accessible mode is `--accessible`, `SPACECRAFT_A11Y`, `[accessibility] enabled`, then hints (`TERM=dumb`, `GTK_MODULES` naming both `gail` and `atk`). `NO_COLOR` is deliberately not a hint here: it selects the `steelbore-mono` theme and leaves the layout alone.
- Key chords with Ctrl+Alt cannot be bound (VoiceOver), and `Insert`/`CapsLock` have no chord name (NVDA, Orca).
