// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>

//! The greetd conversation, on its own thread.
//!
//! # Concurrency model
//!
//! greetd's IPC is strictly request/response, and one response can take as
//! long as PAM takes: while `pam_fprintd` waits for a finger, the reply to
//! our `post_auth_message_response` does not arrive for up to its timeout.
//! The UI must keep drawing and reading keys meanwhile, so the socket lives
//! on a dedicated worker thread. The UI sends [`Command`]s and polls
//! [`Event`]s over two `std::sync::mpsc` channels once per frame.
//!
//! Two threads is the whole design: there is exactly one conversation at a
//! time and nothing CPU-bound, so a pool or an async runtime would only add
//! weight. The worker owns the socket outright; nothing is shared or locked.
//!
//! `info` and `error` PAM messages need no answer, so the worker forwards
//! them as [`Event::Message`] and acknowledges them itself, leaving the UI
//! to act only on real prompts and outcomes.

use std::io::{self, Write as _};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use greetd_ipc::codec::SyncCodec;
use greetd_ipc::{AuthMessageType, ErrorType, Request, Response};
use zeroize::{Zeroize, Zeroizing};

/// Something the UI asks the worker to do.
pub enum Command {
    /// Start authenticating `username`.
    Begin {
        /// Login name.
        username: String,
    },
    /// Answer the pending prompt.
    Answer(Zeroizing<String>),
    /// Start the authenticated session.
    Start {
        /// Command line.
        cmd: Vec<String>,
        /// Extra environment, `KEY=value`.
        env: Vec<String>,
    },
    /// Abandon the current attempt.
    Cancel,
}

impl std::fmt::Debug for Command {
    // Hand-written so an answer can never reach a log or status line.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Begin { username } => {
                f.debug_struct("Begin").field("username", username).finish()
            }
            Self::Answer(_) => f.write_str("Answer(<redacted>)"),
            Self::Start { cmd, env } => f
                .debug_struct("Start")
                .field("cmd", cmd)
                .field("env", env)
                .finish(),
            Self::Cancel => f.write_str("Cancel"),
        }
    }
}

/// Kind of a PAM message that needs no answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    /// `PAM_TEXT_INFO`.
    Info,
    /// `PAM_ERROR_MSG`.
    Error,
}

/// Something the worker reports to the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// PAM wants input. `secret` means do not echo.
    Prompt {
        /// Prompt text, e.g. `Password:`.
        text: String,
        /// Hide the answer.
        secret: bool,
    },
    /// An informational or error line from PAM (already acknowledged).
    Message {
        /// Which kind.
        kind: MessageKind,
        /// Text from the PAM module.
        text: String,
    },
    /// Authentication succeeded; send [`Command::Start`].
    Authenticated,
    /// Authentication was refused; the attempt is over.
    Denied(String),
    /// The session was started; the greeter should exit.
    Started,
    /// A protocol or I/O failure; the attempt is over.
    Failed(String),
}

/// Something that answers greetd requests.
pub trait Backend: Send {
    /// Sends one request and waits for its response.
    ///
    /// # Errors
    /// I/O or protocol failure.
    fn exchange(&mut self, req: &Request) -> anyhow::Result<Response>;
}

/// The real greetd socket.
#[derive(Debug)]
pub struct Greetd {
    stream: UnixStream,
}

impl Greetd {
    /// Connects to `$GREETD_SOCK`.
    ///
    /// # Errors
    /// The variable is unset or the socket refuses the connection.
    pub fn connect() -> anyhow::Result<Self> {
        let path = std::env::var_os("GREETD_SOCK").ok_or_else(|| {
            anyhow::anyhow!("GREETD_SOCK is not set; is Gantry running under greetd? (try --test)")
        })?;
        let stream = UnixStream::connect(&path)?;
        Ok(Self { stream })
    }
}

impl Backend for Greetd {
    fn exchange(&mut self, req: &Request) -> anyhow::Result<Response> {
        // Same framing as greetd_ipc's `write_to` (native-endian u32 length,
        // then JSON), but serialized into a preallocated, zeroized buffer so
        // a password in the body is wiped instead of freed in place.
        let mut body = Zeroizing::new(Vec::with_capacity(16 * 1024));
        serde_json::to_writer(&mut *body, req)?;
        let len = u32::try_from(body.len())?;
        self.stream.write_all(&len.to_ne_bytes())?;
        self.stream.write_all(&body)?;
        Ok(Response::read_from(&mut self.stream)?)
    }
}

/// Spawns the worker. It exits when the command channel closes.
///
/// # Errors
/// The OS refused to create a thread.
pub fn spawn(
    mut backend: Box<dyn Backend>,
    commands: Receiver<Command>,
    events: Sender<Event>,
) -> io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("gantry-auth".into())
        .spawn(move || {
            for cmd in commands {
                let outcome = handle(&mut *backend, cmd, &events);
                if let Err(e) = outcome {
                    tracing::warn!(event = "auth.failure", error = %e, "greetd exchange failed");
                    // Leave greetd clean for the next attempt; a failure here changes nothing.
                    let _ = backend.exchange(&Request::CancelSession);
                    if events.send(Event::Failed(e.to_string())).is_err() {
                        return;
                    }
                }
            }
        })
}

fn handle(backend: &mut dyn Backend, cmd: Command, events: &Sender<Event>) -> anyhow::Result<()> {
    let req = match cmd {
        Command::Begin { username } => Request::CreateSession { username },
        Command::Answer(answer) => Request::PostAuthMessageResponse {
            response: Some(answer.as_str().to_owned()),
        },
        Command::Start { cmd, env } => Request::StartSession { cmd, env },
        Command::Cancel => {
            backend.exchange(&Request::CancelSession)?;
            return Ok(());
        }
    };
    let starting = matches!(req, Request::StartSession { .. });
    let mut req = req;
    let mut resp = backend.exchange(&req);
    wipe(&mut req);
    loop {
        match resp? {
            Response::Success => {
                send(
                    events,
                    if starting {
                        Event::Started
                    } else {
                        Event::Authenticated
                    },
                )?;
                return Ok(());
            }
            Response::Error {
                error_type,
                description,
            } => {
                // greetd keeps a failed session around; cancel it before reporting.
                let _ = backend.exchange(&Request::CancelSession);
                let ev = match error_type {
                    ErrorType::AuthError => Event::Denied(description),
                    ErrorType::Error => Event::Failed(description),
                };
                send(events, ev)?;
                return Ok(());
            }
            Response::AuthMessage {
                auth_message_type,
                auth_message,
            } => match auth_message_type {
                AuthMessageType::Secret | AuthMessageType::Visible => {
                    let secret = matches!(auth_message_type, AuthMessageType::Secret);
                    send(
                        events,
                        Event::Prompt {
                            text: auth_message,
                            secret,
                        },
                    )?;
                    return Ok(());
                }
                AuthMessageType::Info | AuthMessageType::Error => {
                    let kind = if matches!(auth_message_type, AuthMessageType::Info) {
                        MessageKind::Info
                    } else {
                        MessageKind::Error
                    };
                    send(
                        events,
                        Event::Message {
                            kind,
                            text: auth_message,
                        },
                    )?;
                    // This call blocks while PAM works (e.g. a fingerprint scan).
                    resp = backend.exchange(&Request::PostAuthMessageResponse { response: None });
                }
            },
        }
    }
}

/// Overwrites a password held in a request once it has been sent.
fn wipe(req: &mut Request) {
    if let Request::PostAuthMessageResponse { response: Some(s) } = req {
        s.zeroize();
    }
}

fn send(events: &Sender<Event>, ev: Event) -> anyhow::Result<()> {
    events
        .send(ev)
        .map_err(|_closed| anyhow::anyhow!("UI is gone"))
}

/// A simulated finger on the `--test` reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Touch {
    /// An enrolled finger: the scan matches.
    Match,
    /// Any other finger: the scan fails.
    Wrong,
}

/// Tries `pam_fprintd` allows by default (`max-tries=3`).
const MOCK_TRIES: u8 = 3;

/// A stand-in for greetd used by `--test`.
///
/// Any password except `fail` succeeds. With a simulated reader it mirrors
/// `pam_fprintd` stacked before `pam_unix`: each try waits up to the scan
/// window for a [`Touch`]; a wrong finger or a timeout uses up one try and
/// the scan restarts; after the last try the password prompt follows.
#[derive(Debug)]
pub struct Mock {
    fingerprint: bool,
    window: Duration,
    touches: Option<Receiver<Touch>>,
    state: MockState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MockState {
    Idle,
    /// "Place your finger" was sent; the next ack starts waiting for a touch.
    Scanning {
        tries_left: u8,
    },
    /// A failure or timeout notice was sent; the next ack restarts or gives up.
    Missed {
        tries_left: u8,
    },
    Password,
    Authenticated,
}

impl Mock {
    /// Creates the mock. `fingerprint` simulates a reader whose scans wait up
    /// to `window` for a touch from `touches`.
    #[must_use]
    pub fn new(fingerprint: bool, window: Duration, touches: Option<Receiver<Touch>>) -> Self {
        Self {
            fingerprint,
            window,
            touches,
            state: MockState::Idle,
        }
    }

    /// Waits for a touch; `None` means the window closed.
    fn wait_for_touch(&self) -> Option<Touch> {
        if let Some(rx) = &self.touches {
            rx.recv_timeout(self.window).ok()
        } else {
            std::thread::sleep(self.window);
            None
        }
    }
}

impl Backend for Mock {
    fn exchange(&mut self, req: &Request) -> anyhow::Result<Response> {
        let msg = |t: AuthMessageType, s: &str| Response::AuthMessage {
            auth_message_type: t,
            auth_message: s.to_owned(),
        };
        let place = || {
            msg(
                AuthMessageType::Info,
                "Place your right index finger on the fingerprint reader",
            )
        };
        let password = || msg(AuthMessageType::Secret, "Password:");
        let resp = match (req, std::mem::replace(&mut self.state, MockState::Idle)) {
            (Request::CreateSession { .. }, _) => {
                if self.fingerprint {
                    self.state = MockState::Scanning {
                        tries_left: MOCK_TRIES,
                    };
                    place()
                } else {
                    self.state = MockState::Password;
                    password()
                }
            }
            (
                Request::PostAuthMessageResponse { response: None },
                MockState::Scanning { tries_left },
            ) => {
                let left = tries_left.saturating_sub(1);
                match self.wait_for_touch() {
                    Some(Touch::Match) => {
                        self.state = MockState::Authenticated;
                        Response::Success
                    }
                    Some(Touch::Wrong) => {
                        self.state = MockState::Missed { tries_left: left };
                        msg(AuthMessageType::Error, "Failed to match fingerprint")
                    }
                    None => {
                        self.state = MockState::Missed { tries_left: left };
                        msg(AuthMessageType::Info, "Verification timed out")
                    }
                }
            }
            (
                Request::PostAuthMessageResponse { response: None },
                MockState::Missed { tries_left },
            ) => {
                if tries_left > 0 {
                    self.state = MockState::Scanning { tries_left };
                    place()
                } else {
                    self.state = MockState::Password;
                    password()
                }
            }
            (Request::PostAuthMessageResponse { response: Some(pw) }, MockState::Password) => {
                if pw == "fail" {
                    Response::Error {
                        error_type: ErrorType::AuthError,
                        description: "Authentication failure".into(),
                    }
                } else {
                    self.state = MockState::Authenticated;
                    Response::Success
                }
            }
            (Request::StartSession { .. }, MockState::Authenticated)
            | (Request::CancelSession, _) => Response::Success,
            (other, state) => {
                self.state = state;
                Response::Error {
                    error_type: ErrorType::Error,
                    description: format!("mock: unexpected {}", request_name(other)),
                }
            }
        };
        Ok(resp)
    }
}

/// Names a request without its contents (answers may hold passwords).
fn request_name(req: &Request) -> &'static str {
    match req {
        Request::CreateSession { .. } => "create_session",
        Request::PostAuthMessageResponse { .. } => "post_auth_message_response",
        Request::StartSession { .. } => "start_session",
        Request::CancelSession => "cancel_session",
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    fn run(fingerprint: bool, user: &str, password: Option<&str>) -> Vec<Event> {
        run_with(fingerprint, &[], user, password)
    }

    fn run_with(
        fingerprint: bool,
        touches: &[Touch],
        user: &str,
        password: Option<&str>,
    ) -> Vec<Event> {
        let (ttx, trx) = mpsc::channel();
        for t in touches {
            ttx.send(*t).expect("queue touch");
        }
        let (ctx, crx) = mpsc::channel();
        let (etx, erx) = mpsc::channel();
        let handle = spawn(
            Box::new(Mock::new(fingerprint, Duration::ZERO, Some(trx))),
            crx,
            etx,
        )
        .expect("spawn");
        ctx.send(Command::Begin {
            username: user.into(),
        })
        .expect("send");
        let mut seen = Vec::new();
        loop {
            let ev = erx
                .recv_timeout(Duration::from_secs(5))
                .expect("event arrives");
            let done = !matches!(ev, Event::Message { .. });
            seen.push(ev.clone());
            if !done {
                continue;
            }
            match ev {
                Event::Prompt { .. } => {
                    let pw = password.unwrap_or("secret");
                    ctx.send(Command::Answer(Zeroizing::new(pw.into())))
                        .expect("send");
                }
                Event::Authenticated => {
                    ctx.send(Command::Start {
                        cmd: vec!["nu".into()],
                        env: vec![],
                    })
                    .expect("send");
                }
                _ => break,
            }
        }
        drop(ctx);
        handle.join().expect("worker exits cleanly");
        seen
    }

    #[test]
    fn password_login_starts_a_session() {
        let ev = run(false, "mj", None);
        assert_eq!(
            ev.first(),
            Some(&Event::Prompt {
                text: "Password:".into(),
                secret: true
            })
        );
        assert_eq!(ev.last(), Some(&Event::Started));
    }

    #[test]
    fn wrong_password_is_denied() {
        let ev = run(false, "mj", Some("fail"));
        assert!(matches!(ev.last(), Some(Event::Denied(_))));
    }

    #[test]
    fn fingerprint_match_skips_the_password() {
        let ev = run_with(true, &[Touch::Match], "mj", None);
        assert!(ev.iter().all(|e| !matches!(e, Event::Prompt { .. })));
        assert_eq!(ev.last(), Some(&Event::Started));
    }

    #[test]
    fn failed_scans_fall_back_to_password() {
        // One wrong finger, then two empty windows: three tries, as pam_fprintd.
        let ev = run_with(true, &[Touch::Wrong], "mj", None);
        let texts: Vec<&str> = ev
            .iter()
            .filter_map(|e| match e {
                Event::Message { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            texts.len(),
            6,
            "place/failed, place/timed out, place/timed out: {texts:?}"
        );
        assert_eq!(texts.get(1), Some(&"Failed to match fingerprint"));
        assert_eq!(texts.get(5), Some(&"Verification timed out"));
        assert!(
            ev.iter()
                .any(|e| matches!(e, Event::Prompt { secret: true, .. }))
        );
        assert_eq!(ev.last(), Some(&Event::Started));
    }

    #[test]
    fn answers_never_print() {
        let cmd = Command::Answer(Zeroizing::new("hunter2".into()));
        assert!(!format!("{cmd:?}").contains("hunter2"));
        let mut mock = Mock::new(false, Duration::ZERO, None);
        let resp = mock
            .exchange(&Request::PostAuthMessageResponse {
                response: Some("hunter2".into()),
            })
            .expect("mock answers");
        assert!(!format!("{resp:?}").contains("hunter2"));
    }

    #[test]
    fn framing_matches_greetd_ipc() {
        let req = Request::CreateSession {
            username: "mj".into(),
        };
        let mut ours = Vec::new();
        let body = serde_json::to_vec(&req).expect("serialize");
        ours.extend_from_slice(&u32::try_from(body.len()).expect("fits").to_ne_bytes());
        ours.extend_from_slice(&body);
        let mut theirs = Vec::new();
        req.write_to(&mut theirs).expect("greetd_ipc writes");
        assert_eq!(ours, theirs);
    }

    #[test]
    fn wipe_clears_the_password() {
        let mut req = Request::PostAuthMessageResponse {
            response: Some("hunter2".into()),
        };
        wipe(&mut req);
        assert!(
            matches!(req, Request::PostAuthMessageResponse { response: Some(s) } if s.is_empty())
        );
    }
}
