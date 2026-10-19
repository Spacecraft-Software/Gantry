# Contributing to Gantry

Contributions are welcome. Whether they are accepted is at the maintainer's discretion: scope, naming, architecture and roadmap are decided by the maintainer, and a rejection reflects fit, not quality.

- **Scope:** greetd greeting on the Linux console and in kiosk terminals. Lock screens, X11-specific features and non-greetd display managers are out of scope.
- **Before a large change**, open an issue so we can agree on direction.
- **Checks:** `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` must pass. Do not add `unsafe` code.
- **Commits** must be signed and carry a `Signed-off-by:` line (Developer Certificate of Origin).
- **Licensing:** contributions are accepted under GPL-3.0-or-later. Every new file needs the two SPDX header lines.
- **Security issues:** do not open a public issue. Follow [SECURITY.md](SECURITY.md).
