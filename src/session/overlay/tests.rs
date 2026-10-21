use super::*;
use crate::api::LogKind;
use crate::screen::Vt100Screen;
use crate::terminal::TermSize;
use std::time::Duration;

fn overlay(layout: crate::OverlayLayout, now: Instant, screen: &Vt100Screen) -> Overlay {
    let toggle = "ctrl+g".parse().unwrap();
    let mut app = App::new(Some(toggle), LogKind::Following, Duration::from_secs(5));
    app.set_layout(layout);
    app.resize(TermSize {
        cols: 100,
        rows: 30,
    });
    app.set_background(screen.text_rows());
    let mut overlay = Overlay::new(app, KeyDecoder::new(Some(toggle)));
    // Model the completed takeover without writing to the test runner's terminal.
    overlay.takeover = Some(Takeover {
        shell_buffer: Buffer::Main,
        entered_alternate: true,
        shell_modes: Vec::new(),
        resized: false,
    });
    overlay.redraw.opened(now);
    overlay.redraw.drawn();
    overlay
}

fn first_row(overlay: &Overlay) -> String {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
    terminal
        .draw(|frame| overlay.app.render(frame, jiff::Timestamp::UNIX_EPOCH))
        .unwrap();
    (0..100)
        .map(|x| terminal.backend().buffer()[(x, 0)].symbol())
        .collect()
}

// The overlay deadline must refresh a dirty background even after output stops, retaining replay bytes unchanged.
#[test]
fn background_deadline_updates_the_cached_screen() {
    let now = Instant::now();
    let mut screen = Vt100Screen::new(TermSize {
        cols: 100,
        rows: 30,
    });
    screen.process(b"OLD");
    let mut overlay = overlay(crate::OverlayLayout::Float, now, &screen);
    assert_eq!(overlay.deadline(), None);
    let raw = b"\rNEW";
    screen.process(raw);
    overlay.hold(raw);
    assert_eq!(overlay.deadline(), Some(now + Duration::from_millis(33)));
    for millis in 1..33 {
        overlay.update_background(&screen, now + Duration::from_millis(millis));
        assert!(!overlay.redraw.needs_draw());
        assert!(first_row(&overlay).starts_with("OLD"));
    }
    overlay.app_mut().input(
        crate::input::UiInput::Paste("typed immediately".into()),
        now,
    );
    assert!(overlay.redraw.needs_draw());
    overlay.redraw.drawn();
    assert!(first_row(&overlay).starts_with("OLD"));
    overlay.update_background(&screen, now + Duration::from_millis(33));
    assert!(overlay.redraw.needs_draw());
    assert!(first_row(&overlay).starts_with("NEW"));
    overlay.redraw.drawn();
    assert_eq!(overlay.deadline(), None);
    assert_eq!(overlay.pending.take(), Some(raw.to_vec()));
}

// Full-screen output has no visible background and must neither schedule redraws nor discard replay bytes.
#[test]
fn full_screen_output_does_not_wake_drawing() {
    let now = Instant::now();
    let screen = Vt100Screen::new(TermSize {
        cols: 100,
        rows: 30,
    });
    let mut overlay = overlay(crate::OverlayLayout::FullScreen, now, &screen);
    overlay.hold(b"output");
    overlay.update_background(&screen, now + Duration::from_secs(1));
    assert!(!overlay.redraw.needs_draw());
    assert_eq!(overlay.deadline(), None);
    assert_eq!(overlay.pending.take(), Some(b"output".to_vec()));
}

fn round_trip(setup: &[u8], hidden: &[u8], resized: bool) -> (Vt100Screen, Vt100Screen) {
    let size = TermSize { cols: 80, rows: 24 };
    let mut expected = Vt100Screen::new(size);
    let mut outer = Vt100Screen::new(size);
    expected.process(setup);
    outer.process(setup);
    let toggle = "ctrl+g".parse().unwrap();
    let mut overlay = Overlay::new(
        App::new(Some(toggle), LogKind::Following, Duration::from_secs(5)),
        KeyDecoder::new(Some(toggle)),
    );
    let mut display = Display::captured(size);
    overlay
        .open(&mut display, &expected, Instant::now())
        .unwrap();
    outer.process(&display.take_bytes());
    expected.process(hidden);
    overlay.hold(hidden);
    if resized {
        overlay.resized();
    }
    overlay.close(&mut display, &expected).unwrap();
    outer.process(&display.take_bytes());
    (expected, outer)
}

fn assert_screen(expected: &Vt100Screen, outer: &Vt100Screen) {
    assert_eq!(outer.buffer(), expected.buffer());
    assert_eq!(outer.text_rows(), expected.text_rows());
    assert_eq!(
        outer.answer(crate::screen::QueryKind::CursorPosition, b"\x1b[6n"),
        expected.answer(crate::screen::QueryKind::CursorPosition, b"\x1b[6n")
    );
}

// With no startup CPR, replay must retain the terminal's real cursor and preexisting output.
#[test]
fn replay_preserves_real_unsynchronized_cursor() {
    let size = TermSize { cols: 80, rows: 24 };
    let mut outer = Vt100Screen::new(size);
    outer.process(b"EXISTING\x1b[10;7H");
    let model = Vt100Screen::new(size);
    let toggle = "ctrl+g".parse().unwrap();
    let mut overlay = Overlay::new(
        App::new(Some(toggle), LogKind::Following, Duration::from_secs(5)),
        KeyDecoder::new(Some(toggle)),
    );
    let mut display = Display::captured(size);
    overlay.open(&mut display, &model, Instant::now()).unwrap();
    outer.process(&display.take_bytes());
    overlay.close(&mut display, &model).unwrap();
    outer.process(&display.take_bytes());
    outer.process(b"NEXT");
    assert_eq!(outer.text_rows()[0], "EXISTING");
    assert_eq!(outer.text_rows()[9], "      NEXT");
}

// A later DECRC must recover the application's saved position and SGR after either close path.
#[test]
fn application_saved_cursor_survives_overlay() {
    for resized in [false, true] {
        let (mut expected, mut outer) = round_trip(
            b"\x1b[5;6H\x1b[31;1m\x1b7\x1b[0m\x1b[10;1H",
            b"hidden",
            resized,
        );
        expected.process(b"\x1b8X");
        outer.process(b"\x1b8X");
        assert_screen(&expected, &outer);
        assert_eq!(outer.restore(), expected.restore());
    }
}

// Hidden buffer transitions must survive redraw, including a later return to the updated main screen.
#[test]
fn redraw_restores_both_buffers_and_return_cursor() {
    for resized in [false, true] {
        let (mut expected, mut outer) = round_trip(
            b"MAIN\x1b[?1049hALT",
            b"\x1b[?1049l\rUPDATED\x1b[8;9H\x1b[32m\x1b[?1049hNEW ALT",
            resized,
        );
        assert_screen(&expected, &outer);
        expected.process(b"\x1b[?1049lRETURN");
        outer.process(b"\x1b[?1049lRETURN");
        assert_screen(&expected, &outer);
        assert_eq!(outer.restore(), expected.restore());
    }
}

// Opening over a 1047 application must redraw its alternate buffer and keep the main buffer intact.
#[test]
fn overlay_over_1047_application() {
    let (mut expected, mut outer) = round_trip(b"MAIN\x1b[?1047hALT", b" hidden", false);
    assert_screen(&expected, &outer);
    expected.process(b"\x1b[?1047l");
    outer.process(b"\x1b[?1047l");
    assert_screen(&expected, &outer);
}

// An unsynchronized model can have delayed wrap on a different row; replay must not redraw that cell into old output.
#[test]
fn replay_does_not_repaint_unsynchronized_wrap_cursor() {
    let size = TermSize { cols: 80, rows: 24 };
    let mut outer = Vt100Screen::new(size);
    outer.process(b"EXISTING\x1b[10;7H");
    let mut model = Vt100Screen::new(size);
    let output = vec![b'X'; 80];
    outer.process(&output);
    model.process(&output);
    let before = outer.text_rows();
    let cursor = outer.answer(crate::screen::QueryKind::CursorPosition, b"\x1b[6n");
    let toggle = "ctrl+g".parse().unwrap();
    let mut overlay = Overlay::new(
        App::new(Some(toggle), LogKind::Following, Duration::from_secs(5)),
        KeyDecoder::new(Some(toggle)),
    );
    let mut display = Display::captured(size);
    overlay.open(&mut display, &model, Instant::now()).unwrap();
    outer.process(&display.take_bytes());
    overlay.close(&mut display, &model).unwrap();
    outer.process(&display.take_bytes());
    assert_eq!(outer.text_rows(), before);
    assert_eq!(
        outer.answer(crate::screen::QueryKind::CursorPosition, b"\x1b[6n"),
        cursor
    );
}
