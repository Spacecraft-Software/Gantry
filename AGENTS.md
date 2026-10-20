# Gantry — agent context

Gantry is a Rust greetd greeter (ratatui + crossterm) for Steelbore OS Bravais. It follows The Steelbore Standard v2.12. The registered name is Gantry; its short code is `GNT`.

## Build and check

- `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
- Run it with `cargo run -- --test` (mock greetd). Add `--test-fingerprint` for a simulated reader, or `--accessible` for linear mode.
- Nix: `nix build`; the NixOS module is in `nix/module.nix`.

## Layout

- `src/app.rs`: the state machine. Pure: no terminal I/O, unit-tested.
- `src/auth.rs`: the greetd worker thread and the mock backend.
- `src/ui.rs`: rendering. `src/linear.rs`: accessible mode.
- `src/theme.rs`: palette registry and §11.6 resolution. It reads `assets/steelbore.toml`, which is never edited by hand.
- `src/fx/`: background effects, banner effects and the clock font.
- `src/config.rs`, `src/keys.rs`, `src/sessions.rs`, `src/state.rs`, `src/caps.rs`, `src/fingerprint.rs`, `src/art.rs`.

## Rules

- `#![forbid(unsafe_code)]`. No `unwrap`/`expect` outside tests.
- Never log, print or keep a password. Answers travel in `Zeroizing<String>`.
- Colours come only from theme roles. Never hard-code a hex value.
- Every status line carries a text tag (`[ERROR]`, `[WARN]`, `[INFO]`, `[OK]`).
- Art and glyphs must be VT-safe: ASCII, CP437 box drawing and blocks (see `art::is_vt_safe`).
- Every file starts with the two SPDX lines (GPL-3.0-or-later).
- Commits are signed through the host's existing setup. Never push or open a PR outside `Spacecraft-Software` / `UnbreakableMJ` without the maintainer's explicit approval.
