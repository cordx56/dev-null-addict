use std::io;
use std::time::Instant;

use crate::input::{Decoded, GateAction, KeyDecoder, Scanned, ToggleGate, ToggleScanner};
use crate::screen::{Frame, OutputFramer, ScreenModel, Vt100Screen};
use crate::terminal::TermSize;
use crate::ui::{App, Effect, Failure};

use super::requests::{Answer, Requests};
use super::startup::LateReply;
use super::{Display, InputQueue, Overlay, SessionOptions};

pub(super) struct Driver {
    pub display: Display,
    pub queue: InputQueue,
    pub requests: Requests,
    overlay: Overlay,
    screen: Vt100Screen,
    framer: OutputFramer,
    scanner: ToggleScanner,
    gate: ToggleGate,
    opening: Option<(Instant, Vec<u8>)>,
    closing: Option<Instant>,
    startup_reply: LateReply,
    shell_output_seen: bool,
}

impl Driver {
    pub fn new(
        options: &SessionOptions,
        size: TermSize,
    ) -> io::Result<(Self, tokio::sync::mpsc::UnboundedReceiver<Answer>)> {
        let mut app = App::new(
            Some(options.toggle),
            options.client.log,
            options.client.refresh_interval.get(),
        );
        app.set_layout(options.layout);
        let (requests, answers) = Requests::new(
            options.client.key_path.clone(),
            options.client.base_url.clone(),
        );
        Ok((
            Self {
                display: Display::new(size)?,
                queue: InputQueue::default(),
                requests,
                overlay: Overlay::new(app, KeyDecoder::new(Some(options.toggle))),
                screen: Vt100Screen::new(size),
                framer: OutputFramer::new(),
                scanner: ToggleScanner::new(options.toggle),
                gate: ToggleGate::new(),
                opening: None,
                closing: None,
                startup_reply: LateReply::default(),
                shell_output_seen: false,
            },
            answers,
        ))
    }

    pub fn seed_cursor(&mut self, position: Option<(u16, u16)>) {
        if let Some((row, col)) = position {
            self.screen
                .process(format!("\x1b[{};{}H", row + 1, col + 1).as_bytes());
        }
    }

    pub fn expect_startup_reply(&mut self, now: Instant) {
        self.startup_reply.expect(now);
    }

    pub fn input(&mut self, bytes: &[u8], now: Instant) -> io::Result<()> {
        if self.startup_reply.deadline().is_none() {
            return self.route_input(bytes, now);
        }
        let filtered = self.startup_reply.filter(bytes, now);
        if !self.shell_output_seen {
            self.seed_cursor(filtered.position);
        }
        self.route_input(&filtered.held, now)
    }

    fn route_input(&mut self, bytes: &[u8], now: Instant) -> io::Result<()> {
        if self.closing.is_some() {
            self.queue.push(bytes);
        } else if let Some((_, held)) = &mut self.opening {
            held.extend_from_slice(bytes);
        } else if self.overlay.is_open() {
            self.overlay.push_input(bytes, now);
            self.decode(now)?;
        } else {
            let mut scanned = Vec::new();
            self.scanner.push(bytes, now, &mut scanned);
            self.scanned(scanned, now)?;
        }
        Ok(())
    }

    fn scanned(&mut self, pieces: Vec<Scanned>, now: Instant) -> io::Result<()> {
        let mut pieces = pieces.into_iter();
        while let Some(piece) = pieces.next() {
            if self.opening.is_some() || self.overlay.is_open() {
                let raw = match piece {
                    Scanned::Forward(raw) | Scanned::Toggle { raw, .. } => raw,
                };
                self.route_input(&raw, now)?;
            } else {
                let action = self.gate.handle(piece, now);
                if let GateAction::Open(mut raw) = action {
                    // Complete scanner pieces precede its held tail, even when this piece opens the UI.
                    for piece in pieces.by_ref() {
                        match piece {
                            Scanned::Forward(bytes) | Scanned::Toggle { raw: bytes, .. } => {
                                raw.extend(bytes)
                            }
                        }
                    }
                    self.gate_action(GateAction::Open(raw), now)?;
                } else {
                    self.gate_action(action, now)?;
                }
            }
        }
        if self.opening.is_some() || self.overlay.is_open() {
            let held = self.scanner.take_held();
            self.route_input(&held, now)?;
        }
        Ok(())
    }

    fn gate_action(&mut self, action: GateAction, now: Instant) -> io::Result<()> {
        match action {
            GateAction::Forward(raw) => self.queue.push(&raw),
            GateAction::Open(mut raw) => {
                raw.extend(self.scanner.take_held());
                self.opening = Some((now + OutputFramer::BOUNDARY_TIMEOUT, raw));
                self.try_open(now)?;
            }
            GateAction::Wait => {}
        }
        Ok(())
    }

    fn try_open(&mut self, now: Instant) -> io::Result<()> {
        let Some((deadline, _)) = self.opening.as_ref() else {
            return Ok(());
        };
        if !self.framer.at_boundary() && now < *deadline {
            return Ok(());
        }
        if !self.framer.at_boundary() {
            self.display.write(b"\x18")?;
            self.screen.process(b"\x18");
            self.framer.flush(&mut Vec::new());
        }
        let (_, raw) = self.opening.take().expect("opening is pending");
        let effects = self.overlay.open(&mut self.display, &self.screen, now)?;
        self.effects(effects, now)?;
        self.overlay.push_input(&raw, now);
        self.decode(now)
    }

    fn decode(&mut self, now: Instant) -> io::Result<()> {
        while self.overlay.is_open() && self.closing.is_none() {
            let Some(decoded) = self.overlay.next_input(now) else {
                break;
            };
            match decoded {
                Decoded::Reply(raw) => self.queue.push(&raw),
                Decoded::Input(input) => {
                    let effects = self.overlay.app_mut().input(input, now);
                    self.effects(effects, now)?;
                }
            }
        }
        Ok(())
    }

    pub fn output(&mut self, bytes: &[u8], now: Instant) -> io::Result<()> {
        // Even a partial hidden frame makes the startup position too old to apply.
        self.shell_output_seen |= !bytes.is_empty();
        let mut frames = Vec::new();
        self.framer.push(bytes, self.overlay.is_open(), &mut frames);
        self.frames(frames)?;
        self.try_open(now)?;
        self.try_close(now)
    }

    fn frames(&mut self, frames: Vec<Frame>) -> io::Result<()> {
        for frame in frames {
            let raw = match &frame {
                Frame::Bytes(raw) | Frame::Query { raw, .. } => raw,
            };
            self.screen.process(raw);
            if !self.overlay.is_open() {
                self.display.write(raw)?;
            } else {
                match frame {
                    Frame::Bytes(raw) => self.overlay.hold(&raw),
                    Frame::Query { kind, raw } => {
                        if kind.is_local() {
                            if let Some(answer) = self.screen.answer(kind, &raw) {
                                self.queue.push(&answer);
                            }
                        } else {
                            self.display.write(&raw)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn effects(&mut self, effects: Vec<Effect>, now: Instant) -> io::Result<()> {
        for effect in effects {
            match effect {
                Effect::Close => self.request_close(now)?,
                Effect::Send(request) => self.requests.send(request),
            }
        }
        Ok(())
    }

    pub fn close(&mut self) -> io::Result<()> {
        let now = Instant::now();
        self.request_close(now)?;
        self.try_close(now + OutputFramer::BOUNDARY_TIMEOUT)
    }

    fn request_close(&mut self, now: Instant) -> io::Result<()> {
        if self.overlay.is_open() {
            self.closing
                .get_or_insert(now + OutputFramer::BOUNDARY_TIMEOUT);
        }
        self.try_close(now)?;
        if self.closing.is_some() {
            let rest = self.overlay.take_input();
            self.queue.push(&rest);
        }
        Ok(())
    }

    fn try_close(&mut self, now: Instant) -> io::Result<()> {
        let Some(deadline) = self.closing else {
            return Ok(());
        };
        if !self.overlay.can_replay() && !self.framer.at_boundary() {
            if now < deadline {
                return Ok(());
            }
            let mut frames = Vec::new();
            self.framer.flush(&mut frames);
            self.frames(frames)?;
            self.screen.process(b"\x18");
        }
        let rest = self.overlay.close(&mut self.display, &self.screen)?;
        self.closing = None;
        // Deliver a replay tail only after returning the screen, retaining the parser state.
        let mut frames = Vec::new();
        self.framer.push(&[], false, &mut frames);
        self.frames(frames)?;
        self.route_input(&rest, now)
    }

    pub fn resize(&mut self, size: TermSize) -> io::Result<()> {
        self.screen.resize(size);
        self.overlay.resized();
        self.overlay.app_mut().resize(size);
        self.display.resize(size, self.overlay.is_open())
    }

    pub fn answer(&mut self, answer: Answer, now: Instant) -> io::Result<()> {
        if matches!(&answer.result, Err(Failure::Api(error)) if error.code() == Some(crate::api::ErrorCode::DeviceNotApproved))
        {
            self.requests.sign_out();
        }
        let effects = self
            .overlay
            .app_mut()
            .answer(answer.request, answer.result, now);
        self.effects(effects, now)
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.scanner
            .deadline()
            .into_iter()
            .chain(self.gate.deadline())
            .chain(self.startup_reply.deadline())
            .chain(self.closing)
            .chain(self.opening.as_ref().map(|(at, _)| *at))
            .chain(
                self.overlay
                    .is_open()
                    .then(|| self.overlay.deadline())
                    .flatten(),
            )
            .min()
    }

    pub fn tick(&mut self, now: Instant) -> io::Result<()> {
        if self
            .startup_reply
            .deadline()
            .is_some_and(|deadline| now >= deadline)
        {
            self.input(&[], now)?;
        }
        let mut pieces = Vec::new();
        self.scanner.expire(now, &mut pieces);
        self.scanned(pieces, now)?;
        let action = self.gate.expire(now);
        self.gate_action(action, now)?;
        self.try_open(now)?;
        self.try_close(now)?;
        if self.overlay.is_open() {
            self.decode(now)?;
            let effects = self.overlay.tick(now);
            self.effects(effects, now)?;
        }
        Ok(())
    }

    pub fn draw(&mut self) -> io::Result<()> {
        if self.overlay.is_open() {
            self.overlay.update_background(&self.screen, Instant::now());
            self.overlay.draw(&mut self.display)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
