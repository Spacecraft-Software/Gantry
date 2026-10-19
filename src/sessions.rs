// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>

//! Session and user discovery.
//!
//! Sessions come from XDG `.desktop` files; users from `/etc/passwd`. Both
//! readers skip anything malformed instead of failing, so one bad file can
//! never leave the machine without a login.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// Where session files live on NixOS and on FHS systems.
pub const DEFAULT_SESSION_DIRS: [(&str, Kind); 4] = [
    (
        "/run/current-system/sw/share/wayland-sessions",
        Kind::Wayland,
    ),
    ("/run/current-system/sw/share/xsessions", Kind::X11),
    ("/usr/share/wayland-sessions", Kind::Wayland),
    ("/usr/share/xsessions", Kind::X11),
];

/// What kind of session a command starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A Wayland compositor.
    Wayland,
    /// An X11 session, started through the X session wrapper.
    X11,
    /// A text-mode command on the VT (e.g. a login shell).
    Tty,
}

/// One startable session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Session {
    /// Display name.
    pub name: String,
    /// Command and arguments.
    pub exec: Vec<String>,
    /// Session kind.
    pub kind: Kind,
    /// `DesktopNames=` (for `XDG_CURRENT_DESKTOP`).
    pub desktop_names: Option<String>,
    /// Source file, if any.
    pub source: Option<PathBuf>,
}

impl Session {
    /// The configured text-mode default, shown first.
    #[must_use]
    pub fn shell(cmd: &[String]) -> Option<Self> {
        if cmd.is_empty() {
            return None;
        }
        Some(Self {
            name: "Shell".to_owned(),
            exec: cmd.to_vec(),
            kind: Kind::Tty,
            desktop_names: None,
            source: None,
        })
    }

    /// Key used to remember this session across boots.
    #[must_use]
    pub fn key(&self) -> String {
        self.source
            .as_deref()
            .and_then(Path::file_stem)
            .map_or_else(|| self.name.clone(), |s| s.to_string_lossy().into_owned())
    }

    /// Command and environment to hand to greetd.
    #[must_use]
    pub fn launch(&self, x_wrapper: &[String]) -> (Vec<String>, Vec<String>) {
        let mut env = vec![format!(
            "XDG_SESSION_TYPE={}",
            match self.kind {
                Kind::Wayland => "wayland",
                Kind::X11 => "x11",
                Kind::Tty => "tty",
            }
        )];
        if let Some(names) = &self.desktop_names {
            env.push(format!("XDG_CURRENT_DESKTOP={names}"));
            if let Some(first) = names.split(':').next() {
                env.push(format!("XDG_SESSION_DESKTOP={first}"));
            }
        }
        let cmd = match self.kind {
            Kind::X11 if !x_wrapper.is_empty() => {
                x_wrapper.iter().chain(&self.exec).cloned().collect()
            }
            _ => self.exec.clone(),
        };
        (cmd, env)
    }
}

/// Finds sessions in `extra` dirs then the defaults. Duplicate names keep the first.
#[must_use]
pub fn discover(shell: &[String], extra: &[PathBuf]) -> Vec<Session> {
    let mut out: Vec<Session> = Session::shell(shell).into_iter().collect();
    let dirs = extra.iter().map(|p| (p.clone(), Kind::Wayland)).chain(
        DEFAULT_SESSION_DIRS
            .iter()
            .map(|(p, k)| (PathBuf::from(p), *k)),
    );
    for (dir, kind) in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "desktop"))
            .collect();
        files.sort();
        for path in files {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if let Some(mut s) = parse_desktop(&text, kind) {
                if out.iter().any(|o| o.name == s.name) {
                    continue;
                }
                s.source = Some(path);
                out.push(s);
            }
        }
    }
    out
}

/// Parses the `[Desktop Entry]` group of a session file.
#[must_use]
pub fn parse_desktop(text: &str, kind: Kind) -> Option<Session> {
    let mut in_entry = false;
    let (mut name, mut exec, mut names) = (None, None, None);
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        match k.trim() {
            "Name" => name = Some(v.trim().to_owned()),
            "Exec" => exec = Some(v.trim().to_owned()),
            "DesktopNames" => names = Some(v.trim().trim_end_matches(';').replace(';', ":")),
            "Hidden" | "NoDisplay" if v.trim().eq_ignore_ascii_case("true") => return None,
            _ => {}
        }
    }
    let exec = split_exec(&exec?)?;
    Some(Session {
        name: name?,
        exec,
        kind,
        desktop_names: names,
        source: None,
    })
}

/// Splits an `Exec=` value per the Desktop Entry spec: double quotes group,
/// backslash escapes inside quotes, `%x` field codes are dropped (`%%` is `%`).
fn split_exec(exec: &str) -> Option<Vec<String>> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut has_token = false;
    let mut quoted = false;
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                has_token = true;
            }
            '\\' if quoted => cur.push(chars.next()?),
            '%' => {
                if chars.next() == Some('%') {
                    cur.push('%');
                }
            }
            c if c.is_whitespace() && !quoted => {
                if has_token || !cur.is_empty() {
                    args.push(std::mem::take(&mut cur));
                    has_token = false;
                }
            }
            c => {
                cur.push(c);
                has_token = true;
            }
        }
    }
    if quoted {
        return None;
    }
    if has_token || !cur.is_empty() {
        args.push(cur);
    }
    (!args.is_empty()).then_some(args)
}

/// A local account that may log in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct User {
    /// Login name.
    pub name: String,
    /// GECOS full name, when set.
    pub full_name: Option<String>,
    /// Numeric UID.
    pub uid: u32,
}

/// Human users from a passwd file: UID 1000..=59999 with a real shell.
#[must_use]
pub fn users_from_passwd(text: &str) -> Vec<User> {
    let mut out: Vec<User> = text
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            let [name, _pw, uid, _gid, gecos, _home, shell] = f.as_slice() else {
                return None;
            };
            let uid: u32 = uid.parse().ok()?;
            let real_shell =
                !(shell.ends_with("nologin") || shell.ends_with("false") || shell.is_empty());
            ((1000..60000).contains(&uid) && real_shell).then(|| User {
                name: (*name).to_owned(),
                full_name: gecos
                    .split(',')
                    .next()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned),
                uid,
            })
        })
        .collect();
    out.sort_by_key(|u| u.uid);
    out
}

/// Reads `/etc/passwd`; an unreadable file yields no users (typing still works).
#[must_use]
pub fn users() -> Vec<User> {
    std::fs::read_to_string("/etc/passwd")
        .map(|t| users_from_passwd(&t))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_wayland_session() {
        let s = parse_desktop(
            "[Desktop Entry]\nName=niri\nExec=niri-session\nDesktopNames=niri;\nType=Application\n",
            Kind::Wayland,
        )
        .expect("valid session");
        assert_eq!(s.name, "niri");
        assert_eq!(s.exec, vec!["niri-session"]);
        assert_eq!(s.desktop_names.as_deref(), Some("niri"));
    }

    #[test]
    fn hidden_sessions_are_skipped() {
        assert!(
            parse_desktop("[Desktop Entry]\nName=x\nExec=x\nHidden=true\n", Kind::X11).is_none()
        );
    }

    #[test]
    fn other_groups_are_ignored() {
        let s = parse_desktop(
            "[Desktop Entry]\nName=A\nExec=a\n[Desktop Action b]\nName=B\nExec=b\n",
            Kind::Wayland,
        )
        .expect("valid");
        assert_eq!(s.name, "A");
    }

    #[test]
    fn exec_quotes_and_field_codes() {
        assert_eq!(
            split_exec(r#"sh -c "echo \"hi\" there" %U 100%%"#),
            Some(vec![
                "sh".into(),
                "-c".into(),
                "echo \"hi\" there".into(),
                "100%".into()
            ])
        );
        assert_eq!(split_exec("\"unterminated"), None);
        assert_eq!(split_exec("   "), None);
        assert_eq!(
            split_exec(r#"run """#),
            Some(vec!["run".into(), String::new()])
        );
    }

    #[test]
    fn x11_session_goes_through_the_wrapper() {
        let s = Session {
            name: "i3".into(),
            exec: vec!["i3".into()],
            kind: Kind::X11,
            desktop_names: None,
            source: None,
        };
        let (cmd, env) = s.launch(&["startx".into(), "/usr/bin/env".into()]);
        assert_eq!(cmd, vec!["startx", "/usr/bin/env", "i3"]);
        assert_eq!(env, vec!["XDG_SESSION_TYPE=x11"]);
    }

    #[test]
    fn passwd_filters_system_accounts() {
        let users = users_from_passwd(
            "root:x:0:0:root:/root:/run/current-system/sw/bin/nu\n\
             greeter:x:999:999::/var/empty:/run/current-system/sw/bin/nologin\n\
             mj:x:1000:100:Mohamed Hammad,,,:/home/mj:/run/current-system/sw/bin/nu\n\
             svc:x:1001:100::/var/empty:/run/current-system/sw/bin/nologin\n\
             broken line\n",
        );
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].name, "mj");
        assert_eq!(users[0].full_name.as_deref(), Some("Mohamed Hammad"));
    }
}
