// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>

//! Drawing. Reads [`App`] and paints one frame; never mutates state.
//!
//! Contrast: body text is `foreground` on `surface` or `background`; status
//! lines sit on the canvas because `error` on `surface` is cleared for
//! large text only (§11 rules for Steelbore Modern). `structure` is used
//! only for borders and rules on surfaces.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use crate::app::{App, Focus, Level, POWER_ITEMS, Panel, Phase, SETTINGS_ITEMS, Scan};
use crate::fx::{self, Glyphs};
use crate::theme::ColorMode;

const BOX_WIDTH: u16 = 52;

/// Paints the whole screen.
pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    let s = &app.styles;
    let glyphs = glyphs(app);
    frame.render_widget(
        Block::default().style(Style::default().bg(s.background).fg(s.foreground)),
        area,
    );

    if app.cfg.general.animations
        && let Some(effect) = &app.effect
    {
        effect.render(area, frame.buffer_mut(), &s.fx, glyphs);
    }

    if app.panel == Some(Panel::Screensaver) {
        draw_screensaver(frame, app, area);
    } else {
        draw_login(frame, app, area);
        if let Some(panel) = app.panel {
            draw_panel(frame, app, panel, area);
        }
    }

    if s.mode == ColorMode::LinuxVt {
        clamp_vt_backgrounds(frame.buffer_mut(), area);
    }
}

fn glyphs(app: &App) -> Glyphs {
    match app.styles.mode {
        ColorMode::LinuxVt => Glyphs::VtSafe,
        ColorMode::TrueColor => Glyphs::Unicode,
    }
}

/// The Linux console paints only slots 0–7 as backgrounds; a brighter slot
/// would turn on blinking instead. Decorative cells that asked for one get
/// the canvas.
fn clamp_vt_backgrounds(buf: &mut Buffer, area: Rect) {
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, y))
                && let Color::Indexed(n) = cell.bg
                && n >= 8
            {
                cell.set_bg(Color::Indexed(0));
            }
        }
    }
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

fn draw_login(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let s = &app.styles;
    let owned = vt_sanitize(&app.banner.lines(app.variant), glyphs(app));
    let art: Vec<&str> = owned.iter().map(String::as_str).collect();
    let art_h = u16::try_from(art.len()).unwrap_or(u16::MAX);
    let scanning = matches!(app.phase, Phase::Scanning(_));
    let box_h: u16 = if scanning { 13 } else { 9 };
    let total = art_h + 2 + box_h + 2;
    let show_art = total <= area.height;
    let column = centered(area, area.width, if show_art { total } else { box_h + 2 });

    let [art_area, greet_area, box_area, status_area] = Layout::vertical([
        Constraint::Length(if show_art { art_h } else { 0 }),
        Constraint::Length(2),
        Constraint::Length(box_h),
        Constraint::Length(2),
    ])
    .areas(column);

    if show_art {
        let tick = app.tick.saturating_sub(app.banner_since);
        let kind = if app.cfg.general.animations {
            app.text_fx
        } else {
            fx::TextFxKind::None
        };
        let widest = art.iter().map(|l| text_width(l)).max().unwrap_or(0);
        clear_band(frame.buffer_mut(), art_area, widest, s.background);
        fx::text::render_banner(
            &art,
            art_area,
            frame.buffer_mut(),
            s.accent,
            &s.fx,
            kind,
            tick,
        );
    }

    let greeting = Paragraph::new(Line::from(vec![
        Span::styled(
            app.cfg.general.greeting.as_str(),
            Style::default()
                .fg(s.foreground)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  ·  {}", hostname()),
            Style::default().fg(s.foreground),
        ),
    ]))
    .alignment(Alignment::Center);
    let greet_width = text_width(&app.cfg.general.greeting) + 5 + text_width(&hostname());
    clear_band(
        frame.buffer_mut(),
        Rect {
            height: 1,
            ..greet_area
        },
        greet_width,
        s.background,
    );
    frame.render_widget(greeting, greet_area);

    let boxed = centered(box_area, BOX_WIDTH, box_h);
    frame.render_widget(Clear, boxed);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(s.border).bg(s.surface))
        .title(Span::styled(
            " Login ",
            Style::default()
                .fg(s.foreground)
                .bg(s.surface)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(s.surface).fg(s.foreground));
    let inner = block.inner(boxed);
    frame.render_widget(block, boxed);
    draw_fields(frame, app, inner);
    draw_status(frame, app, status_area);
    draw_hints(frame, app, area);
}

fn draw_fields(frame: &mut Frame<'_>, app: &App, inner: Rect) {
    let s = &app.styles;
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Length(2),
        Constraint::Length(2),
        Constraint::Min(0),
    ])
    .split(inner.inner(ratatui::layout::Margin::new(2, 0)));
    let label = Style::default().fg(s.foreground).bg(s.surface);
    let focused = match s.focus {
        Some(c) => Style::default()
            .fg(c)
            .bg(s.surface)
            .add_modifier(Modifier::BOLD),
        None => Style::default().add_modifier(Modifier::REVERSED),
    };

    let user_focus = app.focus == Focus::User && app.phase == Phase::Idle;
    let user_line = Line::from(vec![
        Span::styled("User     ", if user_focus { focused } else { label }),
        Span::styled(app.username.as_str(), label),
    ]);
    frame.render_widget(Paragraph::new(user_line), rows[1]);
    if user_focus {
        set_cursor(frame, rows[1], 9 + text_width(&app.username));
    }

    let (prompt, enabled) = match &app.phase {
        Phase::Prompt { text, .. } => (text.trim_end_matches(':').to_owned(), true),
        Phase::Scanning(_) if app.cfg.fingerprint.type_ahead => ("Password".to_owned(), true),
        _ => ("Password".to_owned(), false),
    };
    let secret = !matches!(app.phase, Phase::Prompt { secret: false, .. });
    let shown = if secret {
        mask(app.answer.chars().count(), s.mode)
    } else {
        app.answer.to_string()
    };
    let answer_focus = app.focus == Focus::Answer && enabled;
    // Cut by characters, never bytes: PAM prompts are localized.
    let name: String = format!("{prompt:<9}").chars().take(9).collect();
    let answer_line = Line::from(vec![
        Span::styled(name, if answer_focus { focused } else { label }),
        Span::styled(
            if enabled {
                shown.clone()
            } else {
                "—".to_owned()
            },
            label,
        ),
    ]);
    frame.render_widget(Paragraph::new(answer_line), rows[2]);
    if answer_focus {
        set_cursor(frame, rows[2], 9 + text_width(&shown));
    }

    let session = app.selected_session().map_or("none", |x| x.name.as_str());
    let session_line = Line::from(vec![
        Span::styled("Session  ", label),
        Span::styled(session, Style::default().fg(s.accent).bg(s.surface)),
        Span::styled("  (F2)", label),
    ]);
    frame.render_widget(Paragraph::new(session_line), rows[3]);

    if let Phase::Scanning(scan) = app.phase {
        draw_scanner(frame, app, scan, rows[4]);
    }
}

fn draw_scanner(frame: &mut Frame<'_>, app: &App, scan: Scan, area: Rect) {
    const PRINT: [&str; 4] = [" ▄▀▀▀▄ ", "█ ▄▀▄ █", "█ █ █ █", "▀ ▀ █ ▀"];
    let s = &app.styles;
    let phase = usize::try_from(app.tick.saturating_sub(scan.since) / 4).unwrap_or(0);
    let color = if app.cfg.general.animations {
        s.fx.ramp[3 + phase % 5]
    } else {
        s.accent
    };
    let [glyph, text] = Layout::horizontal([Constraint::Length(9), Constraint::Min(0)]).areas(area);
    let art: Vec<Line<'_>> = PRINT
        .iter()
        .map(|l| Line::styled(*l, Style::default().fg(color).bg(s.surface)))
        .collect();
    frame.render_widget(Paragraph::new(art), glyph);
    let mut lines = vec![Line::styled(
        "Fingerprint",
        Style::default()
            .fg(s.foreground)
            .bg(s.surface)
            .add_modifier(Modifier::BOLD),
    )];
    lines.push(Line::styled(
        if scan.misses == 0 {
            "Touch the reader".to_owned()
        } else {
            format!("No match ({}) — try again", scan.misses)
        },
        Style::default().fg(s.foreground).bg(s.surface),
    ));
    if scan.wants_password {
        lines.push(Line::styled(
            "Password follows the scan",
            Style::default().fg(s.foreground).bg(s.surface),
        ));
    } else if app.cfg.fingerprint.type_ahead {
        lines.push(Line::styled(
            "Or type your password",
            Style::default().fg(s.foreground).bg(s.surface),
        ));
    }
    frame.render_widget(Paragraph::new(lines), text);
}

fn draw_status(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let s = &app.styles;
    let mut lines = Vec::new();
    if let Some((level, text)) = &app.status {
        let color = match level {
            Level::Info => s.foreground,
            Level::Ok => s.success,
            Level::Warn => s.warning,
            Level::Error => s.error,
        };
        lines.push(Line::from(vec![
            Span::styled(
                level.tag(),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" {text}"), Style::default().fg(s.foreground)),
        ]));
    }
    if app.caps_lock {
        lines.push(Line::from(vec![
            Span::styled(
                "[WARN]",
                Style::default().fg(s.warning).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Caps Lock is on", Style::default().fg(s.foreground)),
        ]));
    }
    let widest = lines.iter().map(Line::width).max().unwrap_or(0);
    let rows = u16::try_from(lines.len()).unwrap_or(0);
    clear_band(
        frame.buffer_mut(),
        Rect {
            height: rows.min(area.height),
            ..area
        },
        u16::try_from(widest).unwrap_or(u16::MAX),
        s.background,
    );
    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), area);
}

/// Blanks a centered band `width` columns wide (plus two columns of padding
/// on each side, which absorbs centering rounding) so animation never shows through text.
fn clear_band(buf: &mut Buffer, area: Rect, width: u16, bg: Color) {
    if width == 0 {
        return;
    }
    let band = centered(area, width.saturating_add(4), area.height);
    for y in band.top()..band.bottom() {
        for x in band.left()..band.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.reset();
                cell.set_bg(bg);
            }
        }
    }
}

fn draw_hints(frame: &mut Frame<'_>, app: &App, area: Rect) {
    if area.height < 3 {
        return;
    }
    let s = &app.styles;
    let row = Rect::new(area.x, area.bottom() - 1, area.width, 1);
    let key = Style::default().fg(s.accent).add_modifier(Modifier::BOLD);
    let txt = Style::default().fg(s.foreground);
    let mut spans = Vec::new();
    for (k, label) in [
        ("F1", "Settings"),
        ("F2", "Session"),
        ("F12", "Power"),
        ("F5", "Accessible"),
        ("PgUp/PgDn", "Art"),
    ] {
        spans.push(Span::styled(k, key));
        spans.push(Span::styled(format!(" {label}   "), txt));
    }
    if app.test_mode {
        spans.push(Span::styled(
            "[TEST MODE]",
            Style::default().fg(s.warning).add_modifier(Modifier::BOLD),
        ));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).alignment(Alignment::Center),
        row,
    );
}

fn draw_panel(frame: &mut Frame<'_>, app: &App, panel: Panel, area: Rect) {
    let s = &app.styles;
    let (title, items, sel): (&str, Vec<String>, usize) = match panel {
        Panel::Sessions(i) => (
            "Session",
            app.sessions.iter().map(|x| x.name.clone()).collect(),
            i,
        ),
        Panel::Power(i) => (
            "Power",
            POWER_ITEMS.iter().map(|x| (*x).to_owned()).collect(),
            i,
        ),
        Panel::Settings(i) => (
            "Settings",
            vec![
                format!("{}: {}", SETTINGS_ITEMS[0], app.styles.slug),
                format!("{}: {}", SETTINGS_ITEMS[1], app.effect_kind),
                format!("{}: {}", SETTINGS_ITEMS[2], app.text_fx),
                SETTINGS_ITEMS[3].to_owned(),
            ],
            i,
        ),
        Panel::Screensaver => return,
    };
    let width = items
        .iter()
        .map(|x| text_width(x))
        .max()
        .unwrap_or(10)
        .max(24)
        + 6;
    let height = u16::try_from(items.len()).unwrap_or(10) + 4;
    let rect = centered(area, width, height);
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(s.border).bg(s.surface))
        .title(Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(s.foreground)
                .bg(s.surface)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(s.surface).fg(s.foreground));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let lines: Vec<Line<'_>> = items
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let mark = if i == sel { "► " } else { "  " };
            let style = if i == sel {
                match s.focus {
                    Some(c) => Style::default()
                        .fg(c)
                        .bg(s.surface)
                        .add_modifier(Modifier::BOLD),
                    None => Style::default().add_modifier(Modifier::REVERSED),
                }
            } else {
                Style::default().fg(s.foreground).bg(s.surface)
            };
            Line::styled(format!("{mark}{text}"), style)
        })
        .collect();
    let body = inner.inner(ratatui::layout::Margin::new(1, 1));
    frame.render_widget(Paragraph::new(lines), body);
}

fn draw_screensaver(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let s = &app.styles;
    let now = jiff::Zoned::now();
    let time = now.strftime("%H:%M:%S").to_string();
    let date = now.strftime("%Y-%m-%d  %A  UTC%:z").to_string();
    let big = fx::clock::big_text(&time);
    let art = app
        .banner
        .lines(usize::try_from(app.tick / (300 * crate::app::FPS)).unwrap_or(0));
    let art_h = u16::try_from(art.len()).unwrap_or(0);
    let h = 3 + 2 + 1 + 2 + art_h;
    let col = centered(area, area.width, h.min(area.height));
    let [clock_area, _, date_area, _, art_area] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Min(0),
    ])
    .areas(col);
    let clock: Vec<Line<'_>> = big
        .iter()
        .map(|l| Line::styled(l.as_str(), Style::default().fg(s.accent)))
        .collect();
    frame.render_widget(
        Paragraph::new(clock).alignment(Alignment::Center),
        clock_area,
    );
    frame.render_widget(
        Paragraph::new(Line::styled(date, Style::default().fg(s.foreground)))
            .alignment(Alignment::Center),
        date_area,
    );
    fx::text::render_banner(
        &art,
        art_area,
        frame.buffer_mut(),
        s.structure,
        &s.fx,
        fx::TextFxKind::None,
        0,
    );
}

/// On the Linux console, replaces glyphs its font lacks with `?` so user art
/// never prints as missing-glyph boxes.
fn vt_sanitize(lines: &[&str], glyphs: Glyphs) -> Vec<String> {
    lines
        .iter()
        .map(|l| match glyphs {
            Glyphs::VtSafe => l
                .chars()
                .map(|c| if crate::art::is_vt_safe(c) { c } else { '?' })
                .collect(),
            Glyphs::Unicode => (*l).to_owned(),
        })
        .collect()
}

fn set_cursor(frame: &mut Frame<'_>, row: Rect, offset: u16) {
    let x = row
        .x
        .saturating_add(offset)
        .min(row.right().saturating_sub(1));
    frame.set_cursor_position((x, row.y));
}

/// Display width in terminal columns (wide and combining characters counted correctly).
fn text_width(s: &str) -> u16 {
    u16::try_from(Span::raw(s).width()).unwrap_or(u16::MAX)
}

fn mask(n: usize, mode: ColorMode) -> String {
    let dot = if mode == ColorMode::LinuxVt {
        '*'
    } else {
        '•'
    };
    std::iter::repeat_n(dot, n.min(32)).collect()
}

fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|h| h.trim().to_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Instant;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::app::AppInit;
    use crate::config::Config;
    use crate::keys::{KeyConfig, Keymap};
    use crate::state::State;
    use crate::theme::{Registry, ThemeInputs};

    fn render(mode: ColorMode, w: u16, h: u16, prep: impl FnOnce(&mut App)) -> Buffer {
        let (tx, _rx) = mpsc::channel();
        let cfg = Config::default();
        let (keymap, _) = Keymap::new(&KeyConfig::default());
        let mut app = App::new(AppInit {
            sessions: crate::sessions::Session::shell(&cfg.general.default_session)
                .into_iter()
                .collect(),
            cfg,
            keymap,
            registry: Registry::embedded().expect("palette"),
            theme_inputs: ThemeInputs::default(),
            mode,
            users: vec![],
            state: State::default(),
            commands: tx,
            test_mode: true,
            screensaver: false,
            now: Instant::now(),
        });
        prep(&mut app);
        for _ in 0..90 {
            app.on_tick(Instant::now(), Rect::new(0, 0, w, h));
        }
        let mut term = Terminal::new(TestBackend::new(w, h)).expect("test terminal");
        term.draw(|f| draw(f, &app)).expect("draw");
        term.backend().buffer().clone()
    }

    fn text(buf: &Buffer) -> String {
        let a = buf.area;
        (a.top()..a.bottom())
            .map(|y| {
                (a.left()..a.right())
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn login_screen_shows_fields_and_hints() {
        let t = text(&render(ColorMode::TrueColor, 100, 32, |_| {}));
        assert!(t.contains("Login"));
        assert!(t.contains("User"));
        assert!(t.contains("Session"));
        assert!(t.contains("Shell"));
        assert!(t.contains("F12"));
    }

    #[test]
    fn tiny_terminals_do_not_panic() {
        for (w, h) in [(1, 1), (10, 3), (40, 10), (300, 100)] {
            let _ = render(ColorMode::LinuxVt, w, h, |_| {});
        }
    }

    #[test]
    fn vt_mode_never_uses_bright_backgrounds() {
        let buf = render(ColorMode::LinuxVt, 80, 30, |_| {});
        for cell in &buf.content {
            if let Color::Indexed(n) = cell.bg {
                assert!(n < 8, "background slot {n}");
            }
        }
    }

    #[test]
    fn errors_carry_a_text_tag() {
        let t = text(&render(ColorMode::TrueColor, 100, 32, |a| {
            a.status = Some((Level::Error, "Login failed".into()));
        }));
        assert!(t.contains("[ERROR] Login failed"));
    }

    #[test]
    fn scanner_appears_while_scanning() {
        let t = text(&render(ColorMode::TrueColor, 100, 36, |a| {
            a.phase = Phase::Scanning(Scan {
                misses: 1,
                since: 0,
                wants_password: false,
            });
        }));
        assert!(t.contains("Fingerprint"));
        assert!(t.contains("No match (1)"));
    }

    #[test]
    fn localized_prompts_do_not_panic() {
        for prompt in ["Contraseña:", "Пароль: ", "パスワード:", "Mot de passe :"] {
            let t = text(&render(ColorMode::TrueColor, 100, 32, |a| {
                a.phase = Phase::Prompt {
                    text: prompt.into(),
                    secret: true,
                };
            }));
            assert!(t.contains("Login"));
        }
    }

    #[test]
    fn screensaver_draws_the_clock() {
        let t = text(&render(ColorMode::TrueColor, 100, 32, |a| {
            a.panel = Some(Panel::Screensaver);
        }));
        assert!(t.contains("UTC"));
        assert!(!t.contains("Login"));
    }
}
