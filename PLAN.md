# Gantry — Plan

MVP: M0–M3 — Gantry v1.0

## M0 — v0 scaffold

- [x] P-001 greetd IPC worker thread with mock backend
- [x] P-002 Session discovery (XDG) and user picker (passwd)
- [x] P-003 Login state machine with remember-last
- [x] P-004 Theme registry and §11.6 two-stage resolution
- [x] P-005 Linux VT palette programming and restore
- [x] P-006 Banner art with variants and VT-safe glyph check
- [x] P-007 Background effects: starfield, fire, matrix, plasma
- [x] P-008 Banner effects: typewriter, beams
- [x] P-009 Idle screensaver with block clock
- [x] P-010 Fingerprint prompt classification and scanner widget
- [x] P-011 Type-ahead password during a scan
- [x] P-012 Remappable CUA and Vim key bindings
- [x] P-013 Accessible linear mode and JSON subcommands
- [x] P-014 NixOS module (VT and kiosk modes, fprintd PAM args)

## M1 — Validation on Bravais

- [ ] P-015 Run on a real VT: palette, glyphs, Caps Lock LED
- [ ] P-016 Fingerprint flow on the T490s sensor
- [ ] P-017 Kiosk mode under niri + Alacritty
- [ ] P-018 Speakup read-through of accessible mode (§18.4)
- [ ] P-019 Measure frame cost and idle CPU; set §22 budgets

## M2 — Polish

- [x] P-020 Settle open questions in the PRD
- [ ] P-021 Fairing hand-off (no blank flash between splash and login)
- [ ] P-022 Texinfo manual `doc/gantry.texi` (§8)
- [ ] ~~P-023 Upstream greetd patch to cancel during a scan~~ (dropped)
- [ ] P-024 Concurrent fingerprint-or-password Rust PAM module
- [ ] P-029 Nickel config and themes evaluated at runtime, with contracts
- [ ] P-030 `gnt` command alias and registry entry

## M3 — Release

- [ ] P-025 `packaging/guix.scm` and `packaging/PKGBUILD` (§5.5)
- [ ] P-026 CI: fmt, clippy, tests, `cargo audit`, `cargo deny`, `reuse lint`
- [ ] P-027 SBOM and release manifest (§24, §25)
- [ ] P-028 `CHANGELOG.md` and tagged signed release
