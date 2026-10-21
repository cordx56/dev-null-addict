use std::io::{self, IsTerminal, Read};
use std::time::Instant;

use tokio::signal::unix::{SignalKind, signal};

use crate::input::{Decoded, KeyDecoder};
use crate::terminal::{self, RawTerminal, TermSize};
use crate::ui::{App, Effect, Failure};

use super::display::{Display, UI_MODES};
use super::requests::Requests;
use super::{ClientOptions, SessionError, catch_session, wait_deadline};

const CLEANUP: &[u8] = b"\x1b[?2004l\x1b[>4;0m\x1b[<u\x1b[0 q\x1b[m\x1b[?25h\x1b[?1049l";

/// How the standalone timeline ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StandaloneExit {
    Quit,
    Signal(i32),
}

/// Shows the timeline on its own over the whole screen, without a shell, until
/// Esc at the top.
///
/// SIGHUP and SIGTERM return their signal number. The terminal is restored on
/// every exit, including errors and panics, before this returns.
pub async fn run_standalone(options: ClientOptions) -> Result<StandaloneExit, SessionError> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(SessionError::NotATerminal);
    }
    let size = TermSize::current().map_err(SessionError::Terminal)?;
    let mut winch = signal(SignalKind::window_change()).map_err(SessionError::Terminal)?;
    let mut hup = signal(SignalKind::hangup()).map_err(SessionError::Terminal)?;
    let mut term = signal(SignalKind::terminate()).map_err(SessionError::Terminal)?;
    let _raw = RawTerminal::enter().map_err(SessionError::Terminal)?;
    let mut display = Display::new(size).map_err(SessionError::Terminal)?;
    let result = catch_session(async {
        terminal::set_cleanup(Some(CLEANUP.to_vec()));
        display.write(b"\x1b[?1049h").map_err(SessionError::Output)?;
        display.write(crate::screen::BASELINE).map_err(SessionError::Output)?;
        display.write(b"\x1b[>0u").map_err(SessionError::Output)?;
        display.write(UI_MODES).map_err(SessionError::Output)?;
        display.take_over().map_err(SessionError::Output)?;
        let mut app = App::new(None, options.log, options.refresh_interval.get());
        app.set_layout(crate::OverlayLayout::FullScreen);
        app.resize(size);
        let mut decoder = KeyDecoder::new(None);
        let (mut requests, mut answers) = Requests::new(options.key_path, options.base_url);
        dispatch(&mut requests, app.open(Instant::now()));
        display.draw(|frame| app.render(frame, jiff::Timestamp::now())).map_err(SessionError::Output)?;
        let (sender, mut stdin) = tokio::sync::mpsc::channel(32);
        std::thread::Builder::new()
            .name("terminal-input".to_owned())
            .spawn(move || {
                let mut stdin = io::stdin().lock();
                let mut bytes = [0u8; 8192];
                loop {
                    let result = match stdin.read(&mut bytes) {
                        Ok(0) => break,
                        Ok(n) => Ok(bytes[..n].to_vec()),
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                        Err(error) => Err(error),
                    };
                    let failed = result.is_err();
                    if sender.blocking_send(result).is_err() || failed {
                        break;
                    }
                }
            })
            .map_err(SessionError::Terminal)?;
        loop {
            let deadline = decoder.deadline().into_iter().chain(app.deadline()).min();
            tokio::select! {
                read = stdin.recv() => match read {
                    Some(Ok(bytes)) => decoder.push(&bytes, Instant::now()),
                    Some(Err(error)) => return Err(SessionError::Terminal(error)),
                    None => return Ok(StandaloneExit::Quit),
                },
                _ = winch.recv() => {
                    let size = TermSize::current().map_err(SessionError::Terminal)?;
                    app.resize(size);
                    display.resize(size, true).map_err(SessionError::Output)?;
                }
                _ = hup.recv() => return Ok(StandaloneExit::Signal(libc::SIGHUP)),
                _ = term.recv() => return Ok(StandaloneExit::Signal(libc::SIGTERM)),
                Some(answer) = answers.recv() => {
                    if matches!(&answer.result, Err(Failure::Api(error)) if error.code() == Some(crate::api::ErrorCode::DeviceNotApproved)) {
                        requests.sign_out();
                    }
                    if dispatch(&mut requests, app.answer(answer.request, answer.result, Instant::now())) {
                        return Ok(StandaloneExit::Quit);
                    }
                }
                joined = requests.join_next() => { joined.map_err(|_| SessionError::Panicked)?; }
                _ = wait_deadline(deadline) => {}
            }
            let now = Instant::now();
            while let Some(decoded) = decoder.next(now) {
                if let Decoded::Input(input) = decoded
                    && dispatch(&mut requests, app.input(input, now))
                {
                    return Ok(StandaloneExit::Quit);
                }
            }
            if dispatch(&mut requests, app.tick(now)) {
                return Ok(StandaloneExit::Quit);
            }
            display.draw(|frame| app.render(frame, jiff::Timestamp::now())).map_err(SessionError::Output)?;
        }
    }).await.unwrap_or(Err(SessionError::Panicked));
    // The panic hook already consumed cleanup before printing the panic.
    let restored = if matches!(result, Err(SessionError::Panicked)) {
        Ok(())
    } else {
        display.write(CLEANUP).map_err(SessionError::Output)
    };
    if restored.is_ok() {
        terminal::set_cleanup(None);
    }
    let exit = result?;
    restored?;
    Ok(exit)
}

fn dispatch(requests: &mut Requests, effects: Vec<Effect>) -> bool {
    for effect in effects {
        match effect {
            Effect::Close => return true,
            Effect::Send(request) => requests.send(request),
        }
    }
    false
}
