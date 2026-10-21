use super::*;

// The border's size controls both fallback and help, including exact thresholds and odd terminal dimensions.
#[test]
fn float_geometry_and_help_thresholds() {
    let mut app = App::new(
        Some("ctrl+g".parse().unwrap()),
        LogKind::Following,
        Duration::from_secs(5),
    );
    for (cols, rows, floating, expected) in [
        (75, 20, true, (7, 2, 60, 16)),
        (74, 20, false, (0, 0, 74, 20)),
        (75, 19, false, (0, 0, 75, 19)),
        (101, 31, true, (10, 3, 80, 24)),
        (1, 1, false, (0, 0, 1, 1)),
    ] {
        app.resize(crate::terminal::TermSize { cols, rows });
        let areas = layout::areas(&app, app.size);
        assert_eq!(areas.window.floating, floating);
        assert_eq!(
            areas.window.outer,
            ratatui::layout::Rect::new(expected.0, expected.1, expected.2, expected.3)
        );
    }
    for (cols, help) in [(124, 0), (125, 24), (150, 24)] {
        app.resize(crate::terminal::TermSize { cols, rows: 30 });
        let areas = layout::areas(&app, app.size);
        assert_eq!(areas.help.width, help);
        assert!(areas.window.inner.contains(areas.list.as_position()));
    }
    app.set_layout(layout::OverlayLayout::FullScreen);
    app.resize(crate::terminal::TermSize {
        cols: 120,
        rows: 30,
    });
    let areas = layout::areas(&app, app.size);
    assert!(!areas.window.floating);
    assert_eq!(areas.window.inner, app.size);
    assert_eq!(areas.help.width, 24);
}

// The window must erase shell text and styles, while its grey background preserves wide-character columns.
#[test]
fn shell_background_is_plain_and_window_is_opaque() {
    use crate::screen::{ScreenModel, Vt100Screen};
    use ratatui::style::{Color, Modifier};
    let mut app = App::new(
        Some("ctrl+g".parse().unwrap()),
        LogKind::Following,
        Duration::from_secs(5),
    );
    let size = crate::terminal::TermSize {
        cols: 100,
        rows: 30,
    };
    app.resize(size);
    let mut screen = Vt100Screen::new(size);
    screen.process(
        "\x1b[31;44;1;4mA界B e\u{301}\x1b[4;10H界\x1b[4;90H界\x1b[15;20HSHELL_SECRET".as_bytes(),
    );
    app.set_background(screen.text_rows());
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(size.cols, size.rows)).unwrap();
    terminal
        .draw(|frame| app.render(frame, jiff::Timestamp::UNIX_EPOCH))
        .unwrap();
    let buffer = terminal.backend().buffer();
    for (x, symbol) in [(0, "A"), (1, "界"), (3, "B"), (5, "e\u{301}")] {
        assert_eq!(buffer[(x, 0)].symbol(), symbol);
        assert_eq!(buffer[(x, 0)].fg, Color::DarkGray);
        assert_eq!(buffer[(x, 0)].bg, Color::Reset);
        assert_eq!(buffer[(x, 0)].modifier, Modifier::empty());
    }
    let window = layout::areas(&app, app.size).window;
    assert_eq!(buffer[(window.outer.x, window.outer.y)].symbol(), "┌");
    assert_eq!(
        buffer[(9, 3)].symbol(),
        " ",
        "no wide glyph may overrun the left border"
    );
    assert_eq!(
        buffer[(90, 3)].symbol(),
        " ",
        "no continuation may escape the right border"
    );
    assert_eq!(buffer[(19, 14)].symbol(), " ");
    assert_eq!(buffer[(19, 14)].fg, Color::Reset);
    let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
    assert!(!text.contains("SHELL_SECRET"));

    app.popup = Some(Popup::ConfirmDelete {
        id: id(1),
        short_hash: id(1).into(),
    });
    terminal
        .draw(|frame| app.render(frame, jiff::Timestamp::UNIX_EPOCH))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Delete commit?"));
    assert_eq!(terminal.backend().buffer()[(15, 13)].symbol(), "┌");
    assert_eq!(terminal.backend().buffer()[(0, 0)].symbol(), "A");
}

// Navigation must use the float's smaller list, not the terminal height, and rendering cannot change its offset.
#[test]
fn floating_list_scrolls_using_its_inner_area() {
    let now = Instant::now();
    let mut app = scrolling_app(true, 100, 30);
    app.set_layout(layout::OverlayLayout::Float);
    app.resize(crate::terminal::TermSize {
        cols: 100,
        rows: 20,
    });
    let area = layout::areas(&app, app.size).list;
    assert_eq!(area.height, 9);
    for offset in 1..=4 {
        app.input(UiInput::Key(Key::Down), now);
        assert_eq!(app.selection().id(), Some(id(1)));
        assert_eq!(app.viewport().top(), offset);
    }
    drawn_list(&app);
    app.input(UiInput::Key(Key::Down), now);
    assert_eq!(app.selection().id(), Some(id(2)));
    assert_eq!(app.viewport().top(), 13);
}
