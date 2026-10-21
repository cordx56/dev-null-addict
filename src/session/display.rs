use std::io::{self, Write};

use ratatui::backend::CrosstermBackend;
use ratatui::{Frame, Terminal};

use crate::terminal::TermSize;

/// Input and cursor modes shared by both timeline displays.
pub(super) const UI_MODES: &[u8] = b"\x1b[>4;0m\x1b[?1l\x1b>\x1b[?9l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1004l\x1b[?1005l\x1b[?1006l\x1b[?2004h\x1b[0 q";

/// The outer terminal's output. Everything the wrapper writes there goes
/// through here: the shell's output while the overlay is closed, and the
/// overlay's drawing.
///
/// The ratatui terminal has a fixed viewport, so it never asks the terminal
/// for its size or cursor position by itself; the answer would come on
/// stdin, which the input thread reads. Its buffers are reset with `resize`
/// when the overlay takes the screen over. It is not resized while the
/// overlay is closed, since resizing clears the screen.
pub struct Display {
    terminal: Terminal<CrosstermBackend<Output>>,
    size: TermSize,
    #[cfg(test)]
    captured: Option<std::rc::Rc<std::cell::RefCell<Vec<u8>>>>,
}

impl Display {
    pub fn new(size: TermSize) -> io::Result<Self> {
        Self::with_output(size, Output::Terminal(io::stdout()))
    }

    fn with_output(size: TermSize, output: Output) -> io::Result<Self> {
        #[cfg(test)]
        let captured = match &output {
            Output::Capture(bytes) => Some(bytes.clone()),
            _ => None,
        };
        let terminal = Terminal::with_options(
            CrosstermBackend::new(output),
            ratatui::TerminalOptions {
                viewport: ratatui::Viewport::Fixed(ratatui::layout::Rect::new(
                    0, 0, size.cols, size.rows,
                )),
            },
        )?;
        Ok(Self {
            terminal,
            size,
            #[cfg(test)]
            captured,
        })
    }

    pub(super) fn size(&self) -> TermSize {
        self.size
    }

    /// Writes bytes as they are, and flushes.
    pub fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        use std::io::Write;
        self.terminal.backend_mut().write_all(bytes)?;
        self.terminal.backend_mut().flush()
    }

    /// Takes note of a new size of the outer terminal. While the overlay is
    /// open, the ratatui terminal follows it at once.
    pub fn resize(&mut self, size: TermSize, overlay_open: bool) -> io::Result<()> {
        self.size = size;
        if overlay_open {
            self.take_over()?;
        }
        Ok(())
    }

    /// Resets ratatui's buffers to the whole screen at the current size, so
    /// that the next draw draws everything.
    pub fn take_over(&mut self) -> io::Result<()> {
        self.terminal.resize(ratatui::layout::Rect::new(
            0,
            0,
            self.size.cols,
            self.size.rows,
        ))
    }

    pub fn draw(&mut self, render: impl FnOnce(&mut Frame<'_>)) -> io::Result<()> {
        self.terminal.draw(render).map(|_| ())
    }
}

enum Output {
    Terminal(io::Stdout),
    #[cfg(test)]
    Capture(std::rc::Rc<std::cell::RefCell<Vec<u8>>>),
}

impl Write for Output {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match self {
            Self::Terminal(output) => output.write(bytes),
            #[cfg(test)]
            Self::Capture(output) => output.borrow_mut().write(bytes),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Terminal(output) => output.flush(),
            #[cfg(test)]
            Self::Capture(_) => Ok(()),
        }
    }
}

#[cfg(test)]
impl Display {
    pub(super) fn captured(size: TermSize) -> Self {
        Self::with_output(size, Output::Capture(std::rc::Rc::default())).unwrap()
    }
    pub(super) fn take_bytes(&mut self) -> Vec<u8> {
        std::mem::take(
            &mut *self
                .captured
                .as_ref()
                .expect("capture required")
                .borrow_mut(),
        )
    }
}
