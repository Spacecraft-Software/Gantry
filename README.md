# Gantry

Gantry is a themed, animated login greeter for [greetd](https://git.sr.ht/~kennylevinsen/greetd), written in Rust for Steelbore OS Bravais. It runs straight on the Linux console, with no compositor, GPU stack or terminal emulator. It can also run inside a fullscreen terminal (niri + Alacritty) for 24-bit colour.

Its look takes after [sysc-greet](https://github.com/Nomadcxx/sysc-greet): ASCII-art banners, animated backgrounds, a screensaver, and menus on the F-keys. All of Gantry's code, art and themes are new.

**Status:** v0, pre-release. Short code `GNT`. The design and its decisions live in the Gantry PRD.

## Features

- **Login:** greetd IPC, user picker from `/etc/passwd`, XDG session picker, a configurable default shell session, a Caps Lock warning, and remembering the last user and session.
- **Fingerprint:** shows `pam_fprintd` prompts as an animated scanner. Keys you type during a scan are buffered and sent when the password prompt arrives.
- **Themes:** all 21 registered Steelbore themes, read from the palette file. Steelbore Modern is the default. On the Linux console, Gantry loads the theme into the console's 16 palette slots, then restores the defaults on exit.
- **Effects:** starfield, DOOM-style fire, matrix rain and plasma backgrounds; typewriter and beam effects on the banner.
- **Screensaver:** a large clock with the date and cycling art after an idle timeout.
- **Accessible mode:** a line-by-line login with no art or animation, for Speakup and BRLTTY.

## Try it

```nu
cargo run -- --test                      # simulated greetd; any password but "fail" works
cargo run -- --test --test-fingerprint   # simulated reader; users named *finger* match
cargo run -- --test --accessible         # linear accessible mode
cargo run -- sessions --json             # non-interactive session list
```

## Keys

| Action | CUA | Vim-style |
| --- | --- | --- |
| Next / previous field | Tab / Shift+Tab | Ctrl+J / Ctrl+K |
| Submit | Enter | Enter |
| Back | Esc | Esc |
| Settings (theme, effects) | F1 | Alt+T |
| Session picker | F2 | Alt+S |
| Power menu | F12 | Alt+P |
| Banner variant | PgUp / PgDn | Alt+H / Alt+L |
| Accessible mode | F5 | Alt+A |
| Pick user | Up / Down | Ctrl+P / Ctrl+N |

You can remap any of these under `[keys]` in the config.

## NixOS

```nix
{
  inputs.gantry.url = "github:Spacecraft-Software/Gantry";
  outputs = { nixpkgs, gantry, ... }: {
    nixosConfigurations.host = nixpkgs.lib.nixosSystem {
      modules = [
        gantry.nixosModules.default
        {
          services.fprintd.enable = true;      # optional
          services.gantry = {
            enable = true;
            mode = "vt";                       # or "kiosk"
            settings.general.effect = "starfield";
          };
        }
      ];
    };
  };
}
```

## Configuration

`/etc/gantry/config.toml`. Every key is optional, and a malformed file falls back to the defaults with a warning. The NixOS module writes this file from `services.gantry.settings`.

```toml
[general]
greeting = "Steelbore OS Bravais"
default_session = ["nu", "--login"]
effect = "starfield"        # fire | matrix | starfield | plasma | none
text_effect = "typewriter"  # typewriter | beams | none
animations = true
remember = true
console_font = false        # true: katakana Matrix rain on the Linux console
clock = true                # local date and time on the top row
clock_format = "%a %d %b %Y  %H:%M  UTC%:z"

[theme]
name = "steelbore"

[screensaver]
idle_timeout = "PT5M"

[fingerprint]
type_ahead = true

[keys]
power_menu = ["F12", "Ctrl+q"]
```

Per-session banners go in `art_dir`, one file per session: `<session>.toml` with `description = "..."` and `variants = ["""..."""]`.

## Design notes

- **No authentication in Gantry.** greetd runs PAM as root. Gantry relays prompts and answers, and wipes passwords from memory after sending them.
- **Two threads.** One greetd reply can block for a whole fingerprint scan, so the socket lives on a worker thread and the UI never stalls.
- **No `unsafe`.** `#![forbid(unsafe_code)]`. Caps Lock is read from sysfs LEDs rather than an ioctl.
- **Seamless hand-off from Fairing.** At the greetd hand-off Fairing paints the console with its theme canvas before letting the display go, and Gantry loads its palette before its first clear, so the screen goes from the boot splash to the login box with no black console between them. Use the same theme for both (the §11.6 system theme by default).
- **Katakana on the console.** With `console_font = true`, Gantry saves the console font with `setfont -O`, loads a copy in which the 56 half-width katakana (from GNU Unifont, scaled to the font's size) replace glyphs the login screen can spare (Cyrillic, Greek, accented Latin, then code page 437's symbols such as ☺ ♪ ∞ ½, so the kernel's built-in font has room too), and loads the original back on exit. It does this just after drawing the first frame and retries for up to three seconds, because under a quiet boot the kernel hands the VT to the framebuffer console only once something is printed; until then `setfont` fails with "Function not implemented". The saved copy stays in `/var/cache/gantry` until then, so a killed greeter is put right on its next start. On a 256-glyph font the spare glyphs can include accented Latin letters, which show as katakana while Gantry runs; if a font has no room, the rain keeps to digits and symbols and the status line says why.
- **Skip key for fingerprint scans.** greetd holds a lock while PAM waits for the sensor, so a greeter cannot cancel a scan. Gantry relies on type-ahead and a short `pam_fprintd` timeout (10 s via the module) instead.

## Budgets

Steelbore Standard §22. Gantry draws at 30 frames a second while something moves and wakes twice a second when the screen is still.

| Budget | Bound | Measured |
|---|---|---|
| Frame cost, 1080p console (240×67 cells), any effect | mean ≤ 4 ms, worst ≤ 8 ms | mean 0.3–1.1 ms, worst 2.8 ms (Fire), `cargo test --release frame_cost -- --ignored --nocapture` |
| Idle wake-ups, still screen | ≤ 2 per second | 2 per second, by construction |
| Idle CPU on the reference machine (ThinkPad T490s) | no visible load | confirmed on the T490s, 2026-10-21 |

## Project Posture

Gantry is a personal hobby project of Spacecraft Software, built for the maintainer's own use. It comes with no warranty and no service commitments. See [NOTICE.md](NOTICE.md), [CONTRIBUTING.md](CONTRIBUTING.md) and [SECURITY.md](SECURITY.md).

- **Component system (§13):** not applicable. Gantry is a terminal UI built on ratatui.
- **Theme (§11):** Steelbore Modern by default; all 21 registered themes are available. It is dark-only (`light = none`) because a console exposes no colour-scheme preference.
- **Conformance (§19):** The Steelbore Standard v2.12, Category B, with the planned Rust PAM module raised to Category A; tailored, pre-release.

## License

GPL-3.0-or-later. Copyright (C) 2026 Mohamed Hammad & Spacecraft Software.

Maintainer: Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>

*— Built by Spacecraft Software —*
