use std::time::{Duration, Instant};

use super::{ToggleKey, UiInput};

/// Decodes the outer terminal's input for the overlay, one piece at a time.
///
/// It reads the legacy encodings, the kitty keyboard protocol and
/// modifyOtherKeys alike, since the program in the shell may have left any of
/// them on. It frames the input itself, sequence by sequence, and has
/// `terminput` parse each key sequence; modifyOtherKeys is parsed here, as
/// `terminput` does not read it.
///
/// Replies to queries the program in the shell sent to the terminal (cursor
/// position, device attributes, colors, keyboard protocol flags) are not
/// keys. They come out as [`Decoded::Reply`], to be passed to the shell.
///
/// Input is taken one piece at a time with [`next`](Self::next), so that when
/// a piece closes the overlay, the rest goes to the shell with
/// [`take_rest`](Self::take_rest), undecoded.
#[derive(Debug)]
pub struct KeyDecoder {
    toggle: Option<ToggleKey>,
    buf: Vec<u8>,
    buf_since: Option<Instant>,
    paste: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decoded {
    Input(UiInput),
    Reply(Vec<u8>),
}

impl KeyDecoder {
    /// How long a lone ESC waits for the rest of a sequence before it is the
    /// Esc key.
    pub const ESCAPE_TIMEOUT: Duration = Duration::from_millis(25);

    pub fn new(toggle: Option<ToggleKey>) -> Self {
        Self {
            toggle,
            buf: Vec::new(),
            buf_since: None,
            paste: None,
        }
    }

    pub fn push(&mut self, bytes: &[u8], now: Instant) {
        if self.buf.is_empty() {
            self.buf_since = Some(now);
        }
        self.buf.extend_from_slice(bytes);
    }

    /// The next piece of input. A partial sequence waits for more bytes until
    /// [`ESCAPE_TIMEOUT`](Self::ESCAPE_TIMEOUT) has passed at `now`, and is
    /// then read as it is. Releases of keys are skipped.
    pub fn next(&mut self, now: Instant) -> Option<Decoded> {
        loop {
            if self.paste.is_some() {
                if let Some(end) = self.buf.windows(6).position(|w| w == b"\x1b[201~") {
                    let mut paste = self.paste.take().expect("paste is active");
                    paste.extend(self.buf.drain(..end));
                    self.buf.drain(..6);
                    self.buf_since = (!self.buf.is_empty()).then_some(now);
                    return Some(Decoded::Input(UiInput::Paste(
                        String::from_utf8_lossy(&paste)
                            .replace("\r\n", "\n")
                            .replace('\r', "\n"),
                    )));
                }
                let keep = (1..=5.min(self.buf.len()))
                    .rev()
                    .find(|n| b"\x1b[201~".starts_with(&self.buf[self.buf.len() - n..]))
                    .unwrap_or(0);
                let end = self.buf.len() - keep;
                self.paste
                    .as_mut()
                    .expect("paste is active")
                    .extend(self.buf.drain(..end));
                self.buf_since = None;
                return None;
            }
            if self.buf.is_empty() {
                self.buf_since = None;
                return None;
            }
            let len = match super::protocol::sequence_len(&self.buf) {
                Some(len) => len,
                None if self.deadline().is_some_and(|d| now >= d) => 1,
                None => return None,
            };
            let raw: Vec<_> = self.buf.drain(..len).collect();
            self.buf_since = (!self.buf.is_empty()).then_some(now);
            if raw == b"\x1b[200~" {
                self.paste = Some(Vec::new());
                continue;
            }
            if super::protocol::is_reply(&raw) {
                return Some(Decoded::Reply(raw));
            }
            if let Some(event) = self
                .toggle
                .and_then(|toggle| super::protocol::toggle_event(&raw, toggle))
            {
                if event == super::ToggleEvent::Press {
                    return Some(Decoded::Input(UiInput::Toggle));
                }
                continue;
            }
            if let Some(input) = parse_key(&raw) {
                return Some(Decoded::Input(UiInput::Key(input)));
            }
        }
    }

    /// When a partial sequence is due to be read as it is, if there is one.
    pub fn deadline(&self) -> Option<Instant> {
        self.buf_since.map(|t| t + Self::ESCAPE_TIMEOUT)
    }

    /// Takes the input not decoded yet, for the shell, and forgets any
    /// partial paste.
    pub fn take_rest(&mut self) -> Vec<u8> {
        self.buf_since = None;
        self.paste = None;
        std::mem::take(&mut self.buf)
    }
}

fn parse_key(raw: &[u8]) -> Option<super::Key> {
    use super::Key;
    use terminput::{Event, KeyCode, KeyEventKind, KeyModifiers};
    let enhanced = super::protocol::enhanced(raw);
    if let Some((code, mods, event)) = enhanced {
        if event == super::ToggleEvent::Release
            || mods & !1 != 0
            || (raw.ends_with(b"u") && matches!(code, 57354 | 57355))
        {
            return None;
        }
        if raw.ends_with(b"~") {
            return match code {
                13 => Some(Key::Enter),
                9 => Some(Key::Tab),
                8 | 127 => Some(Key::Backspace),
                27 => Some(Key::Esc),
                _ => char::from_u32(code)
                    .filter(|c| !c.is_control())
                    .map(Key::Char),
            };
        }
    }
    if raw == [8] {
        return Some(Key::Backspace);
    }
    let Event::Key(event) = Event::parse_from(raw).ok()?? else {
        return None;
    };
    if event.kind == KeyEventKind::Release
        || event.modifiers.intersects(
            KeyModifiers::CTRL
                | KeyModifiers::ALT
                | KeyModifiers::SUPER
                | KeyModifiers::HYPER
                | KeyModifiers::META,
        )
    {
        return None;
    }
    Some(match event.code {
        KeyCode::Char(c) if !c.is_control() => Key::Char(c),
        KeyCode::Enter => Key::Enter,
        KeyCode::Tab => Key::Tab,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
        KeyCode::Esc => Key::Esc,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Key;

    // Standalone input must never treat control-G as a toggle in any supported protocol.
    #[test]
    fn standalone_has_no_toggle() {
        let now = Instant::now();
        for raw in [b"\x07".as_slice(), b"\x1b[103;5u", b"\x1b[27;5;103~"] {
            for split in 0..=raw.len() {
                let mut decoder = KeyDecoder::new(None);
                decoder.push(&raw[..split], now);
                assert_eq!(decoder.next(now), None);
                decoder.push(&raw[split..], now);
                decoder.push(b"x", now);
                assert_eq!(
                    decoder.next(now),
                    Some(Decoded::Input(UiInput::Key(Key::Char('x'))))
                );
                assert_eq!(decoder.next(now + KeyDecoder::ESCAPE_TIMEOUT), None);
            }
        }
    }

    // Removed page bindings must be consumed, not leaked into text or interpreted as navigation.
    #[test]
    fn page_keys_are_inert() {
        let now = Instant::now();
        let mut decoder = KeyDecoder::new(Some("ctrl+g".parse().unwrap()));
        for raw in [
            b"\x1b[5~".as_slice(),
            b"\x1b[6~",
            b"\x1b[57354u",
            b"\x1b[57355u",
        ] {
            decoder.push(raw, now);
            assert_eq!(decoder.next(now), None);
            assert!(decoder.take_rest().is_empty());
        }
    }

    // Legacy, kitty and modifyOtherKeys must produce the same UI actions, while modifiers stay inert.
    #[test]
    fn keys_across_protocols() {
        let now = Instant::now();
        let mut decoder = KeyDecoder::new(Some("ctrl+g".parse().unwrap()));
        for (raw, expected) in [
            (&b"x"[..], Key::Char('x')),
            ("界".as_bytes(), Key::Char('界')),
            (b"\r", Key::Enter),
            (b"\t", Key::Tab),
            (b"\x08", Key::Backspace),
            (b"\x7f", Key::Backspace),
            (b"\x1b[A", Key::Up),
            (b"\x1bOB", Key::Down),
            (b"\x1b[C", Key::Right),
            (b"\x1b[D", Key::Left),
            (b"\x1b[H", Key::Home),
            (b"\x1b[F", Key::End),
            (b"\x1b[3~", Key::Delete),
            (b"\x1b[97;1u", Key::Char('a')),
            (b"\x1b[97;1:2u", Key::Char('a')),
            (b"\x1b[13u", Key::Enter),
            (b"\x1b[27;1;120~", Key::Char('x')),
            (b"\x1b[27;2;65~", Key::Char('A')),
        ] {
            for byte in raw {
                decoder.push(&[*byte], now);
            }
            assert_eq!(
                decoder.next(now),
                Some(Decoded::Input(UiInput::Key(expected))),
                "{raw:?}"
            );
        }
        decoder.push(
            b"\x1b[97;1:3u\x1b[97;5u\x1bx\x1b[103;5:2u\x1b[103;5:3u",
            now,
        );
        assert_eq!(decoder.next(now), None);
        decoder.push(b"\x1b[27;5;103~", now);
        assert_eq!(decoder.next(now), Some(Decoded::Input(UiInput::Toggle)));
    }

    // Terminal replies must never be inserted into a draft, even when fragmented.
    #[test]
    fn replies_are_separate_from_keys() {
        let now = Instant::now();
        for raw in [
            &b"\x1b[12;34R"[..],
            b"\x1b[?1;2c",
            b"\x1b[>0;123;0c",
            b"\x1b[?7u",
            b"\x1b]10;rgb:aa/bb/cc\x07",
            b"\x1bP>|terminal\x1b\\",
            b"\x1b[?25;1$y",
        ] {
            for split in 1..raw.len() {
                let mut decoder = KeyDecoder::new(Some("ctrl+g".parse().unwrap()));
                decoder.push(&raw[..split], now);
                assert_eq!(decoder.next(now), None);
                decoder.push(&raw[split..], now);
                assert_eq!(decoder.next(now), Some(Decoded::Reply(raw.to_vec())));
            }
        }
    }

    // Closing must leave the rest byte-for-byte intact; a lone ESC waits only for its deadline.
    #[test]
    fn escape_timeout_and_pull_based_rest() {
        let now = Instant::now();
        let mut decoder = KeyDecoder::new(Some("ctrl+g".parse().unwrap()));
        decoder.push(b"\x1b", now);
        assert_eq!(decoder.next(now), None);
        assert_eq!(
            decoder.next(now + KeyDecoder::ESCAPE_TIMEOUT),
            Some(Decoded::Input(UiInput::Key(Key::Esc)))
        );
        decoder.push(b"\x07echo hi\r\x1b[", now);
        assert_eq!(decoder.next(now), Some(Decoded::Input(UiInput::Toggle)));
        assert_eq!(decoder.take_rest(), b"echo hi\r\x1b[");
        assert_eq!(decoder.deadline(), None);
    }

    // A pasted toggle is text, and CRLF and CR normalize once across split paste markers.
    #[test]
    fn paste_is_literal_and_normalized() {
        let now = Instant::now();
        let mut decoder = KeyDecoder::new(Some("ctrl+g".parse().unwrap()));
        for part in [b"\x1b[200~a\r".as_slice(), b"\nb\rc\x07\x1b[20", b"1~"] {
            decoder.push(part, now);
            if part != b"1~" {
                assert_eq!(decoder.next(now), None);
            }
        }
        assert_eq!(
            decoder.next(now),
            Some(Decoded::Input(UiInput::Paste("a\nb\nc\x07".to_owned())))
        );
    }
}
