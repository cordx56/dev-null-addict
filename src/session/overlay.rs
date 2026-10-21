mod redraw;

use std::io;
use std::time::Instant;

use crate::input::{Decoded, KeyDecoder};
use crate::screen::{Buffer, ScreenModel};
use crate::ui::{App, Effect};

use super::display::Display;
use super::pending::PendingOutput;

/// The overlay on the outer terminal, open or closed.
///
/// Opening takes the screen over. If the program in the shell is on the main
/// screen, the overlay draws on the alternate screen, so the main screen and
/// its scrollback stay as they are. If the program is on the alternate screen
/// already, the overlay draws over it. Either way the overlay sets
/// [`BASELINE`](crate::screen::BASELINE) and its own modes first, and gives
/// [`terminal::set_cleanup`](crate::terminal::set_cleanup) the bytes that
/// undo them.
///
/// Closing gives the screen back as the shell left it.
///
/// - The output is replayed when the overlay entered the alternate screen,
///   nothing was dropped ([`PendingOutput::LIMIT`]) and the terminal was not
///   resized meanwhile. The overlay leaves the alternate screen, sets the
///   shell's modes as they were when it opened, and replays the output,
///   which brings the terminal to where the shell is now.
/// - Otherwise both buffers, their saved cursors and the active buffer are
///   reconstructed with [`ScreenModel::redraw_all`]. The driver waits for an
///   output boundary before this path.
pub struct Overlay {
    app: App,
    decoder: KeyDecoder,
    pending: PendingOutput,
    takeover: Option<Takeover>,
    redraw: redraw::Redraw,
}

/// What opening the overlay changed on the outer terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Takeover {
    /// The buffer the shell was on, and the outer terminal showed, when the
    /// overlay opened.
    shell_buffer: Buffer,
    entered_alternate: bool,
    /// Buffer and mode restoration captured on opening, preceding replay.
    shell_modes: Vec<u8>,
    /// Set by a resize while open, which rules out a replay.
    resized: bool,
}

impl Overlay {
    pub fn new(app: App, decoder: KeyDecoder) -> Self {
        Self {
            app,
            decoder,
            pending: PendingOutput::default(),
            takeover: None,
            redraw: redraw::Redraw::default(),
        }
    }

    pub fn is_open(&self) -> bool {
        self.takeover.is_some()
    }

    /// Takes the screen over and draws everything. Only to be called when
    /// the output that reached the outer terminal ends at a boundary (see
    /// [`OutputFramer`](crate::screen::OutputFramer)).
    pub fn open(
        &mut self,
        display: &mut Display,
        screen: &impl ScreenModel,
        now: Instant,
    ) -> io::Result<Vec<Effect>> {
        if self.is_open() {
            return Ok(Vec::new());
        }
        let shell_buffer = screen.buffer();
        let entered_alternate = shell_buffer == Buffer::Main;
        let preserve_saved_cursor = screen.has_saved_cursor();
        let mut shell_modes = Vec::new();
        if entered_alternate {
            shell_modes.extend_from_slice(b"\x1b[?47l");
            shell_modes.extend(if preserve_saved_cursor {
                screen.restore()
            } else {
                screen.restore_modes()
            });
            if !preserve_saved_cursor {
                // Restore modes before DECRC, which knows the real cursor even without startup CPR.
                shell_modes.extend_from_slice(b"\x1b[?47h\x1b[?1049l");
            }
        }
        let cleanup = if entered_alternate {
            shell_modes.clone()
        } else {
            screen.redraw_all()
        };
        crate::terminal::set_cleanup(Some(cleanup));
        self.takeover = Some(Takeover {
            shell_buffer,
            entered_alternate,
            shell_modes,
            resized: false,
        });
        if entered_alternate {
            // Mode 47 does not overwrite an application's DECSC slot.
            display.write(if preserve_saved_cursor {
                b"\x1b[?47h"
            } else {
                b"\x1b[?1049h"
            })?;
        }
        display.write(crate::screen::BASELINE)?;
        display.write(b"\x1b[=0u")?;
        display.write(super::display::UI_MODES)?;
        display.take_over()?;
        self.app.resize(display.size());
        if self.app.floating() {
            self.app.set_background(screen.text_rows());
        }
        self.redraw.opened(now);
        let effects = self.app.open(now);
        self.draw(display)?;
        Ok(effects)
    }

    /// Gives the screen back, and returns the input not decoded yet, for the
    /// shell.
    pub fn close(
        &mut self,
        display: &mut Display,
        screen: &impl ScreenModel,
    ) -> io::Result<Vec<u8>> {
        let Some(takeover) = self.takeover.as_ref() else {
            return Ok(self.decoder.take_rest());
        };
        let replay = self.pending.take();
        if takeover.entered_alternate && !takeover.resized && replay.is_some() {
            display.write(&takeover.shell_modes)?;
            display.write(replay.as_deref().unwrap_or_default())?;
        } else {
            display.write(&screen.redraw_all())?;
        }
        self.takeover = None;
        self.redraw = redraw::Redraw::default();
        let _ = self
            .app
            .input(crate::input::UiInput::Toggle, Instant::now());
        crate::terminal::set_cleanup(None);
        Ok(self.decoder.take_rest())
    }

    pub fn can_replay(&self) -> bool {
        self.takeover.as_ref().is_some_and(|takeover| {
            takeover.entered_alternate && !takeover.resized && self.pending.replayable()
        })
    }

    /// Input while open, to be taken with [`next_input`](Self::next_input).
    pub fn push_input(&mut self, bytes: &[u8], now: Instant) {
        self.decoder.push(bytes, now);
    }

    pub fn take_input(&mut self) -> Vec<u8> {
        self.decoder.take_rest()
    }

    pub fn next_input(&mut self, now: Instant) -> Option<Decoded> {
        self.decoder.next(now)
    }

    /// Keeps output of the shell, but not its queries, for the replay.
    pub fn hold(&mut self, bytes: &[u8]) {
        self.pending.push(bytes);
        if self.app.floating() {
            self.redraw.background_changed();
        }
    }

    /// The outer terminal was resized while open.
    pub fn resized(&mut self) {
        self.redraw.ui_changed();
        self.redraw.background_changed();
        if let Some(takeover) = &mut self.takeover {
            takeover.resized = true;
        }
    }

    pub fn app_mut(&mut self) -> &mut App {
        self.redraw.ui_changed();
        &mut self.app
    }

    /// The earliest deadline of the decoder and the app.
    pub fn deadline(&self) -> Option<Instant> {
        self.decoder
            .deadline()
            .into_iter()
            .chain(self.app.deadline())
            .chain(
                self.app
                    .floating()
                    .then(|| self.redraw.deadline())
                    .flatten(),
            )
            .min()
    }

    pub(super) fn update_background(&mut self, screen: &impl ScreenModel, now: Instant) {
        if self.is_open() && self.app.floating() && self.redraw.background_due(now) {
            self.app.set_background(screen.text_rows());
            self.redraw.background_refreshed(now);
        }
    }

    pub(super) fn tick(&mut self, now: Instant) -> Vec<Effect> {
        if self.app.deadline().is_some_and(|deadline| now >= deadline) {
            self.redraw.ui_changed();
        }
        self.app.tick(now)
    }

    pub fn draw(&mut self, display: &mut Display) -> io::Result<()> {
        if self.is_open() && self.redraw.needs_draw() {
            display.draw(|frame| self.app.render(frame, jiff::Timestamp::now()))?;
            self.redraw.drawn();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
