use super::*;

fn short_list(full: bool) -> App {
    let mut app = app();
    app.resize(crate::terminal::TermSize { cols: 80, rows: 19 });
    let entries: Vec<_> = (1..=6)
        .map(|n| {
            let mut c = commit(n);
            c.images.clear();
            CommitEntry::Live(Box::new(c))
        })
        .collect();
    if full {
        let mut view = ThreadView::new(id(1));
        view.set_thread(Thread {
            ancestors: Vec::new(),
            commit: match &entries[0] {
                CommitEntry::Live(c) => *c.clone(),
                _ => unreachable!(),
            },
        });
        view.refresh_replies(Page {
            items: entries[1..].to_vec(),
            next_cursor: None,
        });
        app.threads.push(view);
    } else {
        for kind in LogKind::ALL {
            let tab = app.timeline.tab_mut(kind);
            tab.list.refresh(Page {
                items: entries.clone(),
                next_cursor: None,
            });
            tab.selection.reanchor(&(1..=6).map(id).collect::<Vec<_>>());
        }
    }
    app.focus = Focus::List;
    app.sync_viewport();
    app
}

fn unmarked(lines: Vec<String>) -> Vec<String> {
    lines
        .into_iter()
        .map(|line| line.chars().skip(2).collect())
        .collect()
}

// Regression: moving up after reaching the bottom changes selection without pinning it to the bottom row.
#[test]
fn upward_selection_keeps_the_visible_page() {
    let now = Instant::now();
    for full in [false, true] {
        let mut app = short_list(full);
        for _ in 0..5 {
            app.input(UiInput::Key(Key::Char('j')), now);
        }
        assert_eq!(app.selection().id(), Some(id(6)));
        assert_eq!(app.viewport().top(), 12);
        let page = unmarked(drawn_list(&app));
        for n in [5, 4] {
            app.input(UiInput::Key(Key::Char('k')), now);
            assert_eq!(app.selection().id(), Some(id(n)));
            assert_eq!(app.viewport().top(), 12);
            assert_eq!(unmarked(drawn_list(&app)), page);
        }
        app.input(UiInput::Key(Key::Up), now);
        assert_eq!(app.selection().id(), Some(id(3)));
        assert_eq!(app.viewport().top(), 8);
        for n in [4, 5] {
            app.input(UiInput::Key(Key::Down), now);
            assert_eq!(app.selection().id(), Some(id(n)));
            assert_eq!(app.viewport().top(), 8);
        }
        app.input(UiInput::Key(Key::Down), now);
        assert_eq!(app.viewport().top(), 12);
    }
}

// Tabs and nested threads must restore their own view, including after closing and reopening the overlay.
#[test]
fn each_list_retains_its_own_top_line() {
    let now = Instant::now();
    let mut app = short_list(false);
    for _ in 0..5 {
        app.input(UiInput::Key(Key::Down), now);
    }
    app.input(UiInput::Key(Key::Up), now);
    let following = *app.viewport();
    app.input(UiInput::Key(Key::Right), now);
    assert_eq!(app.viewport().top(), 0);
    for _ in 0..3 {
        app.input(UiInput::Key(Key::Down), now);
    }
    let latest = *app.viewport();
    assert_eq!(latest.top(), 4);
    app.input(UiInput::Key(Key::Right), now);
    assert_eq!(app.viewport().top(), 0);
    app.input(UiInput::Key(Key::Left), now);
    assert_eq!(*app.viewport(), latest);
    app.input(UiInput::Key(Key::Left), now);
    assert_eq!(*app.viewport(), following);

    app.threads = short_list(true).threads;
    for _ in 0..4 {
        app.input(UiInput::Key(Key::Down), now);
    }
    let thread = *app.viewport();
    app.threads.extend(short_list(true).threads);
    app.sync_viewport();
    assert_eq!(app.viewport().top(), 0);
    app.input(UiInput::Key(Key::Esc), now);
    assert_eq!(*app.viewport(), thread);
    app.input(UiInput::Toggle, now);
    app.open(now);
    assert_eq!(*app.viewport(), thread);
    app.input(UiInput::Key(Key::Esc), now);
    assert_eq!(*app.viewport(), following);
}

// Resizing and refreshed content reconcile before rendering, while updates that fit leave the view still.
#[test]
fn updates_and_resizes_keep_selection_visible_without_render_mutation() {
    let now = Instant::now();
    let mut app = short_list(false);
    for _ in 0..5 {
        app.input(UiInput::Key(Key::Down), now);
    }
    app.input(UiInput::Key(Key::Up), now);
    let viewport = *app.viewport();
    app.answer(
        Request::Star {
            id: id(5),
            starred: true,
        },
        Ok(Response::Commit({
            let mut c = commit(5);
            c.images.clear();
            c.starred = Some(true);
            c
        })),
        now,
    );
    assert_eq!(*app.viewport(), viewport);
    app.resize(crate::terminal::TermSize { cols: 80, rows: 15 });
    assert_eq!(app.viewport().top(), 12);
    assert!(drawn_list(&app).iter().any(|line| line.contains("body 5")));
    for n in [6, 5, 4] {
        app.answer(Request::Delete(id(n)), Ok(Response::Done), now);
    }
    assert_eq!(app.selection().id(), Some(id(3)));
    assert_eq!(app.viewport().top(), 4);
    assert!(drawn_list(&app).iter().any(|line| line.contains("body 3")));
}
