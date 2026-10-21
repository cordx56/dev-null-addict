//! The wrapper: the shell on a PTY, its input and output passed through, and
//! the overlay on the toggle key.
//!
//! Before starting the input thread, startup asks once for CPR (up to 100 ms),
//! seeds the model when answered, and retains any typeahead without clearing.
//! If unanswered, the common input path consumes the first late CPR for another
//! two seconds. Its position is usable only before any shell output arrives.
//!
//! One loop on a current-thread tokio runtime waits on everything at once:
//! input (read by a thread of its own, since reading stdin cannot be
//! cancelled), the shell's output, writes to the shell ([`InputQueue`]), the
//! shell's exit, `SIGWINCH`, `SIGHUP` and `SIGTERM`, the deadlines of the
//! input handling, the output boundary and the overlay, the answers of
//! requests, and panics of their tasks. Only the loop writes to the outer
//! terminal, through [`Display`].
//!
//! Input goes through [`ToggleScanner`](crate::input::ToggleScanner) and
//! [`ToggleGate`](crate::input::ToggleGate) while the overlay is closed, and
//! through the overlay's decoder while it is open, piece by piece, so that
//! what follows the piece that opens or closes the overlay goes the other
//! way.
//!
//! The shell's output goes through an
//! [`OutputFramer`](crate::screen::OutputFramer) into the
//! [`ScreenModel`](crate::screen::ScreenModel) at all times.
//!
//! - While the overlay is closed, it also goes to the outer terminal as it
//!   is, partial sequences included.
//! - When the gate says to open while the output stops in the middle of a
//!   sequence, opening waits for a boundary, at most
//!   [`OutputFramer::BOUNDARY_TIMEOUT`](crate::screen::OutputFramer::BOUNDARY_TIMEOUT).
//!   After that it writes CAN to the outer terminal and flushes the framer.
//! - Replay close keeps incomplete output intact; redraw close waits for the
//!   same boundary timeout as opening.
//! - While the overlay is open, the framer holds partial sequences, and each
//!   frame is either held for the replay ([`Overlay::hold`]) or, for a query,
//!   answered: a local one from the model, straight to the shell, and any
//!   other by passing it to the outer terminal, whose reply comes back
//!   through the decoder as a reply for the shell. Queries are not replayed.
//!
//! The session ends in one of these ways:
//!
//! - The shell exits. Its output is still read until the PTY reports EOF or
//!   `EIO`, for at most [`DRAIN_TIMEOUT`], then the PTY is closed.
//! - `SIGHUP` or `SIGTERM` arrives, or something fails. The shell is ended
//!   with [`pty::hang_up`](crate::pty::hang_up).
//!
//! In every case the shell is reaped and the terminal restored before
//! [`run`] returns, so that what comes after, such as the fallback to a
//! plain shell, starts from a clean state.

mod display;
mod driver;
mod input_queue;
mod overlay;
mod pending;
mod requests;
mod standalone;
mod startup;

pub use standalone::{StandaloneExit, run_standalone};

use std::fmt;
use std::io;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use crate::api::{BaseUrl, LogKind};
use crate::input::ToggleKey;
use crate::shell::{ShellCommand, ShellExit};

use self::display::Display;
use self::input_queue::InputQueue;
use self::overlay::Overlay;

/// How long the shell's output is still read after it exited, for what its
/// background jobs or its last writes left in the PTY.
pub const DRAIN_TIMEOUT: Duration = Duration::from_millis(100);

/// What the timeline needs, inside the wrapper or on its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientOptions {
    /// The device key. It is read when the overlay first needs it, so that a
    /// `login` in another shell takes effect without restarting.
    pub key_path: PathBuf,
    pub base_url: BaseUrl,
    /// The tab shown first.
    pub log: LogKind,
    pub refresh_interval: RefreshInterval,
}

/// What [`run`] needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionOptions {
    pub toggle: ToggleKey,
    pub shell: ShellCommand,
    pub layout: crate::OverlayLayout,
    pub client: ClientOptions,
}

/// How often the open overlay refreshes. Nothing is fetched while it is
/// closed.
///
/// At least [`MIN`](Self::MIN), so that a few terminals at once stay under the
/// API's limit of 120 signed requests a minute.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RefreshInterval(Duration);

impl RefreshInterval {
    pub const MIN: Self = Self(Duration::from_secs(2));
    pub const DEFAULT: Self = Self(Duration::from_secs(5));

    pub fn new(interval: Duration) -> Result<Self, RefreshIntervalError> {
        if interval < Self::MIN.0 {
            Err(RefreshIntervalError::TooShort)
        } else {
            Ok(Self(interval))
        }
    }

    pub fn get(self) -> Duration {
        self.0
    }
}

impl Default for RefreshInterval {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Whole seconds.
impl FromStr for RefreshInterval {
    type Err = RefreshIntervalError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(Duration::from_secs(
            s.parse().map_err(|_| RefreshIntervalError::Syntax)?,
        ))
    }
}

/// Whole seconds.
impl fmt::Display for RefreshInterval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.as_secs().fmt(f)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RefreshIntervalError {
    #[error("expected a whole number of seconds")]
    Syntax,
    #[error("the refresh interval has to be at least {} seconds", RefreshInterval::MIN.0.as_secs())]
    TooShort,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("standard input and output have to be a terminal")]
    NotATerminal,
    #[error("could not set up the terminal")]
    Terminal(#[source] io::Error),
    #[error("could not start {}", program.display())]
    Spawn {
        program: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("lost the connection to the shell")]
    Pty(#[source] io::Error),
    #[error("could not write to the terminal")]
    Output(#[source] io::Error),
    #[error("a task of the wrapper panicked")]
    Panicked,
}

/// Runs the shell until it exits and returns how it exited.
///
/// On `SIGHUP` or `SIGTERM` the shell is hung up first, and its exit is
/// returned. On an error the shell is hung up too. The shell is reaped and
/// the outer terminal restored before this returns, whether it fails or not.
pub async fn run(options: SessionOptions) -> Result<ShellExit, SessionError> {
    use crate::pty::{self, ShellProcess};
    use crate::terminal::{RawTerminal, TermSize};
    use std::io::{IsTerminal, Read};
    use std::time::Instant;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::signal::unix::{SignalKind, signal};

    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(SessionError::NotATerminal);
    }
    let size = TermSize::current().map_err(SessionError::Terminal)?;
    let mut winch = signal(SignalKind::window_change()).map_err(SessionError::Terminal)?;
    let mut hup = signal(SignalKind::hangup()).map_err(SessionError::Terminal)?;
    let mut term = signal(SignalKind::terminate()).map_err(SessionError::Terminal)?;
    let _raw = RawTerminal::enter().map_err(SessionError::Terminal)?;
    let (mut driver, mut answers) =
        driver::Driver::new(&options, size).map_err(SessionError::Terminal)?;
    let startup::StartupInput {
        position,
        held: typeahead,
    } = startup::cursor(&mut driver.display).map_err(SessionError::Terminal)?;
    let now = Instant::now();
    driver.seed_cursor(position);
    if position.is_none() {
        driver.expect_startup_reply(now);
    }
    driver
        .input(&typeahead, now)
        .map_err(SessionError::Terminal)?;
    let ShellProcess {
        mut output,
        mut input,
        mut child,
    } = ShellProcess::spawn(&options.shell, size).map_err(|source| SessionError::Spawn {
        program: options.shell.program().to_owned(),
        source,
    })?;
    let (sender, mut stdin) = tokio::sync::mpsc::channel(32);
    let reader = std::thread::Builder::new()
        .name("terminal-input".to_owned())
        .spawn(move || {
            let mut stdin = io::stdin().lock();
            let mut bytes = [0u8; 8192];
            loop {
                let result = match stdin.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(n) => Ok(bytes[..n].to_vec()),
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => Err(e),
                };
                let failed = result.is_err();
                if sender.blocking_send(result).is_err() || failed {
                    break;
                }
            }
        });
    if let Err(error) = reader {
        let _ = pty::hang_up(output, input, child).await;
        return Err(SessionError::Terminal(error));
    }
    let mut bytes = [0u8; 32768];
    let mut exited = None;
    let mut drain_until = None;
    let mut eof = false;
    let result: Result<Option<ShellExit>, SessionError> = catch_session(async {
        loop {
            if eof && exited.is_some() { return Ok(exited); }
            let deadline = driver.deadline().into_iter().chain(drain_until).min();
            tokio::select! {
                read = stdin.recv(), if exited.is_none() => {
                    match read {Some(Ok(bytes)) => driver.input(&bytes, Instant::now()).map_err(SessionError::Output)?, Some(Err(error)) => return Err(SessionError::Terminal(error)), None => return Ok(None)}
                }
                read = output.read(&mut bytes), if !eof => {
                    match read {
                        Ok(0) => eof = true,
                        Ok(n) => driver.output(&bytes[..n], Instant::now()).map_err(SessionError::Output)?,
                        Err(error) if error.raw_os_error() == Some(5) => eof = true,
                        Err(error) => return Err(SessionError::Pty(error)),
                    }
                }
                written = input.write(driver.queue.front()), if !driver.queue.is_empty() && !eof => {
                    let written = written.map_err(SessionError::Pty)?;
                    if written == 0 { return Err(SessionError::Pty(io::ErrorKind::WriteZero.into())); }
                    driver.queue.consume(written);
                }
                status = child.wait(), if exited.is_none() => {
                    exited = Some(status.map_err(SessionError::Pty)?.into()); drain_until = Some(Instant::now() + DRAIN_TIMEOUT);
                }
                _ = winch.recv() => {
                    let size = TermSize::current().map_err(SessionError::Terminal)?;
                    input.resize(size.into()).map_err(|e| SessionError::Pty(io::Error::other(e)))?;
                    driver.resize(size).map_err(SessionError::Output)?;
                }
                _ = hup.recv() => return Ok(None),
                _ = term.recv() => return Ok(None),
                Some(answer) = answers.recv() => driver.answer(answer, Instant::now()).map_err(SessionError::Output)?,
                joined = driver.requests.join_next() => { joined.map_err(|_| SessionError::Panicked)?; }
                _ = wait_deadline(deadline) => {
                    if drain_until.is_some_and(|at| Instant::now() >= at) { return Ok(exited); }
                    driver.tick(Instant::now()).map_err(SessionError::Output)?;
                }
            }
            driver.draw().map_err(SessionError::Output)?;
        }
    }).await.unwrap_or(Err(SessionError::Panicked));
    let closed = if matches!(result, Err(SessionError::Panicked)) {
        Ok(())
    } else {
        driver.close().map_err(SessionError::Output)
    };
    let status = if let Some(status) = exited {
        drop(output);
        drop(input);
        Ok(status)
    } else {
        pty::hang_up(output, input, child)
            .await
            .map_err(SessionError::Pty)
    };
    match result {
        Err(error) => Err(error),
        Ok(_) => {
            closed?;
            status
        }
    }
}

async fn wait_deadline(deadline: Option<std::time::Instant>) {
    if let Some(deadline) = deadline {
        tokio::time::sleep_until(deadline.into()).await;
    } else {
        std::future::pending::<()>().await;
    }
}

// Keep the child outside the unwind boundary so it can still be hung up and reaped.
async fn catch_session<F: std::future::Future>(future: F) -> Result<F::Output, ()> {
    let mut future = std::pin::pin!(future);
    std::future::poll_fn(|cx| {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| future.as_mut().poll(cx))) {
            Ok(std::task::Poll::Ready(output)) => std::task::Poll::Ready(Ok(output)),
            Ok(std::task::Poll::Pending) => std::task::Poll::Pending,
            Err(_) => std::task::Poll::Ready(Err(())),
        }
    })
    .await
}
