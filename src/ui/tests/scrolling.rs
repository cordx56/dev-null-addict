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

// gg resets a scrolled log or innermost thread, including ancestors, without requests.
#[test]
fn gg_returns_to_the_first_entry_of_the_shown_list() {
    let now = Instant::now();
    for full in [false, true] {
        let mut app = short_list(false);
        if full {
            app.threads = short_list(true).threads;
            let mut inner = short_list(true).threads.pop().unwrap();
            let mut ancestor = commit(0);
            ancestor.images.clear();
            inner.set_thread(Thread {
                ancestors: vec![CommitEntry::Live(Box::new(ancestor))],
                commit: commit(1),
            });
            app.threads.push(inner);
            app.sync_viewport();
        }
        for _ in 0..5 {
            assert!(app.input(UiInput::Key(Key::Char('j')), now).is_empty());
        }
        assert_eq!(app.selection().id(), Some(id(6)));
        assert!(app.viewport().top() > 0);
        let selection = *app.selection();
        let viewport = *app.viewport();
        assert!(app.input(UiInput::Key(Key::Char('g')), now).is_empty());
        assert_eq!(*app.selection(), selection);
        assert_eq!(*app.viewport(), viewport);

        // An answer between the keys must preserve the pending g.
        let mut updated = commit(6);
        updated.images.clear();
        updated.starred = Some(true);
        assert!(
            app.answer(
                Request::Star {
                    id: id(6),
                    starred: true,
                },
                Ok(Response::Commit(updated)),
                now,
            )
            .is_empty()
        );
        assert!(
            app.input(UiInput::Key(Key::Char('g')), now + GG_TIMEOUT)
                .is_empty()
        );
        assert_eq!(app.selection().id(), Some(id(if full { 0 } else { 1 })));
        assert_eq!(app.viewport().top(), 0);
        assert_eq!(app.pending_g, None);
        if full {
            assert_eq!(app.threads[0].selection().id(), Some(id(1)));
            assert_eq!(app.threads[0].viewport.top(), 0);
        }
    }
}

// Selecting an already selected tall first entry must still reset its internal scroll.
#[test]
fn gg_resets_scrolling_inside_the_first_entry() {
    let now = Instant::now();
    for full in [false, true] {
        let mut app = scrolling_app(full, 80, 11);
        for _ in 0..3 {
            app.input(UiInput::Key(Key::Char('j')), now);
        }
        assert_eq!(app.selection().id(), Some(id(1)));
        assert!(app.viewport().top() > 0);
        assert!(app.input(UiInput::Key(Key::Char('g')), now).is_empty());
        assert!(app.input(UiInput::Key(Key::Char('g')), now).is_empty());
        assert_eq!(app.selection().id(), Some(id(1)));
        assert_eq!(app.viewport().top(), 0);
    }
}

// An expired g starts a fresh pair, and the second g consumes that pair.
#[test]
fn gg_timeout_restarts_the_pending_pair() {
    let now = Instant::now();
    let mut app = short_list(false);
    for _ in 0..4 {
        app.input(UiInput::Key(Key::Char('j')), now);
    }
    let selection = *app.selection();
    let viewport = *app.viewport();
    app.input(UiInput::Key(Key::Char('g')), now);
    let later = now + GG_TIMEOUT + Duration::from_nanos(1);
    assert!(app.input(UiInput::Key(Key::Char('g')), later).is_empty());
    assert_eq!(*app.selection(), selection);
    assert_eq!(*app.viewport(), viewport);
    assert_eq!(app.pending_g, Some(later));
    assert!(app.input(UiInput::Key(Key::Char('g')), later).is_empty());
    assert_eq!(app.selection().id(), Some(id(1)));
    assert_eq!(app.viewport().top(), 0);
    assert_eq!(app.pending_g, None);
    app.input(UiInput::Key(Key::Char('g')), later);
    assert_eq!(app.pending_g, Some(later));
}

// Every other input cancels g, including early returns and ignored list pastes.
#[test]
fn gg_is_cancelled_by_intervening_input() {
    let now = Instant::now();
    for input in [
        UiInput::Key(Key::Char('j')),
        UiInput::Key(Key::Char('k')),
        UiInput::Key(Key::Char('h')),
        UiInput::Key(Key::Char('l')),
        UiInput::Key(Key::Char('x')),
        UiInput::Key(Key::Esc),
        UiInput::Key(Key::Tab),
        UiInput::Toggle,
        UiInput::Paste("g".into()),
    ] {
        let mut app = short_list(false);
        app.input(UiInput::Key(Key::Char('j')), now);
        app.input(UiInput::Key(Key::Char('g')), now);
        app.input(input, now);
        assert_eq!(app.pending_g, None);
        app.focus = Focus::List;
        let selection = *app.selection();
        let viewport = *app.viewport();
        assert!(app.input(UiInput::Key(Key::Char('g')), now).is_empty());
        assert_eq!(*app.selection(), selection);
        assert_eq!(*app.viewport(), viewport);
        assert_eq!(app.pending_g, Some(now));
    }
}

// Composer g remains text, and confirmation popups cannot start or complete gg.
#[test]
fn gg_respects_composer_and_delete_confirmation() {
    let now = Instant::now();
    let mut app = short_list(false);
    app.input(UiInput::Key(Key::Char('j')), now);
    app.input(UiInput::Key(Key::Char('g')), now);
    app.input(UiInput::Key(Key::Tab), now);
    assert_eq!(app.focus, Focus::Compose);
    for _ in 0..3 {
        assert!(app.input(UiInput::Key(Key::Char('g')), now).is_empty());
    }
    assert_eq!(app.compose.to_new_commit().unwrap().body.as_str(), "ggg");
    assert_eq!(app.pending_g, None);
    assert_eq!(app.selection().id(), Some(id(2)));

    app.focus = Focus::List;
    app.input(UiInput::Key(Key::Char('g')), now);
    app.popup = Some(Popup::ConfirmDelete {
        id: id(2),
        short_hash: commit(2).short_hash,
    });
    for _ in 0..2 {
        assert!(app.input(UiInput::Key(Key::Char('g')), now).is_empty());
        assert_eq!(app.pending_g, None);
        assert_eq!(app.selection().id(), Some(id(2)));
        assert!(matches!(app.popup, Some(Popup::ConfirmDelete { .. })));
    }
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
