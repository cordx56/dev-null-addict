use super::*;
use crate::input::Key;
use crate::test_support::{commit, id, page};

fn app() -> App {
    let mut app = App::new(
        Some("ctrl+g".parse().unwrap()),
        LogKind::Following,
        Duration::from_secs(5),
    );
    app.set_layout(layout::OverlayLayout::FullScreen);
    app
}
fn ready(app: &mut App, now: Instant) {
    app.open(now);
    app.answer(
        Request::Me,
        Ok(Response::CurrentUser(CurrentUser {
            user: crate::api::User {
                username: "someone".into(),
                name: None,
                lane: None,
            },
            unread_notifications: 0,
        })),
        now,
    );
    app.answer(
        Request::Log {
            kind: LogKind::Following,
            cursor: None,
        },
        Ok(Response::Entries(page(&[1, 2], Some("next")))),
        now,
    );
}

// Send is skipped for short drafts, and successful sends reset the composer while moderation remains tracked.
#[test]
fn compose_focus_send_and_moderation() {
    let mut app = app();
    let now = Instant::now();
    ready(&mut app, now);
    app.input(UiInput::Key(Key::Tab), now);
    assert_eq!(app.focus, Focus::List);
    app.input(UiInput::Key(Key::Char('r')), now);
    assert_eq!(app.focus, Focus::Compose);
    assert!(app.compose.target().is_some());
    app.input(UiInput::Key(Key::Backspace), now);
    assert!(app.compose.target().is_none());
    app.input(UiInput::Paste("hello\nworld".to_owned()), now);
    app.input(UiInput::Key(Key::Tab), now);
    assert_eq!(app.focus, Focus::Send);
    let effects = app.input(UiInput::Key(Key::Enter), now);
    let Effect::Send(request @ Request::Create(_)) = effects[0].clone() else {
        panic!("create effect")
    };
    assert!(app.sending);
    assert!(app.input(UiInput::Key(Key::Enter), now).is_empty());
    let mut pending = commit(3);
    pending.status = crate::api::CommitStatus::Pending;
    app.answer(request, Ok(Response::Commit(pending)), now);
    assert!(app.compose.is_empty());
    assert_eq!(app.focus, Focus::Compose);
    assert_eq!(app.pending, vec![id(3)]);
    let effects = app.tick(now + Duration::from_secs(5));
    assert!(effects.contains(&Effect::Send(Request::Moderation(id(3)))));
    let mut rejected = commit(3);
    rejected.status = crate::api::CommitStatus::Rejected;
    app.answer(
        Request::Moderation(id(3)),
        Ok(Response::Thread(Thread {
            ancestors: Vec::new(),
            commit: rejected,
        })),
        now,
    );
    assert!(app.pending.is_empty());
    assert!(app.popup.is_none());
    app.tick(now + Popup::TOAST_TIME);
    assert!(app.popup.is_none());
}

// Esc unwinds confirmation and nested threads; the toggle always closes and preserves draft and navigation.
#[test]
fn navigation_and_delete_confirmation() {
    let mut app = app();
    let now = Instant::now();
    ready(&mut app, now);
    app.input(UiInput::Key(Key::Tab), now);
    assert_eq!(
        app.input(UiInput::Key(Key::Enter), now),
        vec![
            Effect::Send(Request::Thread(id(1))),
            Effect::Send(Request::Replies {
                id: id(1),
                cursor: None
            })
        ]
    );
    app.answer(
        Request::Thread(id(1)),
        Ok(Response::Thread(Thread {
            ancestors: Vec::new(),
            commit: commit(1),
        })),
        now,
    );
    app.input(UiInput::Key(Key::Char('d')), now);
    assert!(matches!(app.popup, Some(Popup::ConfirmDelete { .. })));
    app.input(UiInput::Key(Key::Esc), now);
    assert!(app.popup.is_none());
    assert_eq!(app.threads.len(), 1);
    app.input(UiInput::Key(Key::Char('d')), now);
    assert_eq!(
        app.input(UiInput::Key(Key::Enter), now),
        vec![Effect::Send(Request::Delete(id(1)))]
    );
    app.input(UiInput::Toggle, now);
    assert_eq!(app.threads.len(), 1);
    assert!(app.tick(now + Duration::from_secs(60)).is_empty());
    app.open(now);
    assert_eq!(app.focus, Focus::Compose);
    app.input(UiInput::Key(Key::Esc), now);
    assert!(app.threads.is_empty());
    assert_eq!(app.input(UiInput::Key(Key::Esc), now), vec![Effect::Close]);
}

// Persistent login notices differ from transient key errors, and automatic requests obey Retry-After.
#[test]
fn failures_backoff_and_closed_answers() {
    let mut app = app();
    let now = Instant::now();
    app.open(now);
    app.answer(Request::Me, Err(Failure::SignedOut), now);
    assert_eq!(app.notice, Some(Notice::SignedOut));
    let request = Request::Log {
        kind: LogKind::Following,
        cursor: None,
    };
    app.answer(
        request.clone(),
        Err(Failure::Api(ApiError::Status {
            status: reqwest::StatusCode::TOO_MANY_REQUESTS,
            code: Some(crate::api::ErrorCode::RateLimited),
            message: "slow down".into(),
            retry_after: Some(Duration::from_secs(30)),
        })),
        now,
    );
    assert!(app.tick(now + Duration::from_secs(29)).is_empty());
    assert!(!app.tick(now + Duration::from_secs(30)).is_empty());
    app.input(UiInput::Toggle, now);
    app.pending.push(id(3));
    assert!(
        app.answer(
            Request::Moderation(id(3)),
            Ok(Response::Thread(Thread {
                ancestors: Vec::new(),
                commit: commit(3)
            })),
            now
        )
        .is_empty()
    );
    assert!(matches!(
        Failure::from(KeyError::Missing("missing".into())),
        Failure::SignedOut
    ));
    assert!(matches!(Failure::from(KeyError::NoHome), Failure::Key(_)));
}

// Fixed-size offscreen rendering verifies help, popups and the IME cursor without terminal I/O.
#[test]
fn renders_help_popups_and_cursor() {
    let mut app = app();
    let now = Instant::now();
    ready(&mut app, now);
    app.input(UiInput::Paste("hello world".into()), now);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 30)).unwrap();
    terminal
        .draw(|frame| app.render(frame, "2026-10-21T12:00:00Z".parse().unwrap()))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text.contains("Keys"));
    assert!(text.contains("hello world"));
    assert!(text.contains("someone"));
    let cursor = terminal.get_cursor_position().unwrap();
    assert!(cursor.x > 1 && cursor.x < 96 && cursor.y < 15);
    app.popup = Some(Popup::ConfirmDelete {
        id: id(1),
        short_hash: id(1).into(),
    });
    terminal
        .draw(|frame| app.render(frame, jiff::Timestamp::now()))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text.contains("Delete commit?"));
    for (cols, rows) in [(1, 1), (5, 3), (80, 24)] {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(cols, rows)).unwrap();
        terminal
            .draw(|frame| app.render(frame, jiff::Timestamp::now()))
            .unwrap();
    }
}

fn scrolling_app(full: bool, cols: u16, rows: u16) -> App {
    let mut app = app();
    app.resize(crate::terminal::TermSize { cols, rows });
    let now = Instant::now();
    ready(&mut app, now);
    let entries: Vec<_> = [1, 2]
        .into_iter()
        .map(|n| {
            let mut commit = commit(n);
            commit.images.clear();
            commit.body = (0..10)
                .map(|line| format!("line {line}"))
                .collect::<Vec<_>>()
                .join("\n");
            commit
        })
        .collect();
    if full {
        let mut view = ThreadView::new(id(1));
        view.set_thread(Thread {
            ancestors: Vec::new(),
            commit: entries[0].clone(),
        });
        view.refresh_replies(Page {
            items: vec![CommitEntry::Live(Box::new(entries[1].clone()))],
            next_cursor: Some("more".to_owned().into()),
        });
        app.threads.push(view);
    } else {
        app.timeline.tab_mut(LogKind::Following).list.refresh(Page {
            items: entries
                .into_iter()
                .map(|c| CommitEntry::Live(Box::new(c)))
                .collect(),
            next_cursor: Some("more".to_owned().into()),
        });
    }
    app.focus = Focus::List;
    app.sync_viewport();
    app
}

fn selected_offset(app: &App) -> usize {
    let area = layout::areas(app, app.size).list;
    let start: usize = app
        .list_entries()
        .iter()
        .take_while(|entry| Some(entry.id()) != app.selection().id())
        .map(|entry| render::entry_height(entry, area.width, !app.threads.is_empty()))
        .sum();
    app.viewport().top().saturating_sub(start)
}

fn drawn_list(app: &App) -> Vec<String> {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(
        app.size.width,
        app.size.height,
    ))
    .unwrap();
    let selection = *app.selection();
    let viewport = *app.viewport();
    terminal
        .draw(|frame| app.render(frame, jiff::Timestamp::UNIX_EPOCH))
        .unwrap();
    assert_eq!(
        *app.selection(),
        selection,
        "render must not change scrolling"
    );
    assert_eq!(
        *app.viewport(),
        viewport,
        "render must not change scrolling"
    );
    let area = layout::areas(app, app.size).list;
    (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| terminal.backend().buffer()[(x, y)].symbol())
                .collect()
        })
        .collect()
}

// Both list types scroll one line before changing commits; moving upward enters the previous commit at its bottom.
#[test]
fn tall_commits_scroll_before_selection_moves() {
    let now = Instant::now();
    for full in [false, true] {
        let mut app = scrolling_app(full, 80, 11);
        assert_eq!(layout::areas(&app, app.size).list.height, 4);
        let limit = if full { 9 } else { 5 };
        assert!(drawn_list(&app)[0].contains("@someone"));
        for offset in 1..=limit {
            let key = if offset % 2 == 0 {
                Key::Down
            } else {
                Key::Char('j')
            };
            assert!(app.input(UiInput::Key(key), now).is_empty());
            assert_eq!(app.selection().id(), Some(id(1)));
            assert_eq!(selected_offset(&app), offset);
        }
        let bottom = drawn_list(&app);
        assert!(
            bottom
                .iter()
                .any(|line| line.contains("1 stars  0 replies"))
        );
        assert!(bottom.iter().all(|line| line.starts_with("│ ")));
        let effects = app.input(UiInput::Key(Key::Down), now);
        assert_eq!(app.selection().id(), Some(id(2)));
        assert_eq!(selected_offset(&app), 0);
        assert_eq!(
            effects.len(),
            1,
            "navigation to the last commit requests the next page"
        );
        assert!(drawn_list(&app)[0].contains("@someone"));
        app.input(UiInput::Key(Key::Char('k')), now);
        assert_eq!(app.selection().id(), Some(id(1)));
        assert_eq!(selected_offset(&app), limit);
        assert_eq!(drawn_list(&app), bottom);
        for offset in (0..limit).rev() {
            app.input(
                UiInput::Key(if offset % 2 == 0 {
                    Key::Up
                } else {
                    Key::Char('k')
                }),
                now,
            );
            assert_eq!(app.selection().id(), Some(id(1)));
            assert_eq!(selected_offset(&app), offset);
        }
        assert!(drawn_list(&app)[0].contains("@someone"));
        app.input(UiInput::Key(Key::Up), now);
        assert_eq!(app.selection().id(), Some(id(1)));
        assert_eq!(selected_offset(&app), 0);
    }
}

// Input uses current geometry before any draw, including the help column, composer growth, and the reply target.
#[test]
fn scrolling_uses_resizes_and_composer_layout_without_rendering() {
    let now = Instant::now();
    let mut app = scrolling_app(true, 100, 14);
    let mut long = commit(1);
    long.body = "x".repeat(600);
    long.images.clear();
    app.update_commit(CommitEntry::Live(Box::new(long)));
    let area = layout::areas(&app, app.size).list;
    assert_eq!((area.width, area.height), (76, 7));
    for _ in 0..5 {
        app.input(UiInput::Key(Key::Down), now);
    }
    assert_eq!(app.selection().id(), Some(id(1)));
    assert_eq!(selected_offset(&app), 5);

    app.resize(crate::terminal::TermSize { cols: 99, rows: 14 });
    assert_eq!(layout::areas(&app, app.size).list.width, 99);
    app.input(UiInput::Key(Key::Up), now);
    assert_eq!(
        selected_offset(&app),
        2,
        "resize clamps to 3 before scrolling up"
    );
    app.input(UiInput::Key(Key::Char('r')), now);
    app.input(UiInput::Paste("a\nb\nc".into()), now);
    assert_eq!(layout::areas(&app, app.size).list.height, 4);
    app.input(UiInput::Key(Key::Tab), now);
    app.input(UiInput::Key(Key::Tab), now);
    for offset in 3..=6 {
        app.input(UiInput::Key(Key::Down), now);
        assert_eq!(app.selection().id(), Some(id(1)));
        assert_eq!(selected_offset(&app), offset);
    }
    assert!(drawn_list(&app).iter().any(|line| line.contains("stars")));

    app.resize(crate::terminal::TermSize { cols: 1, rows: 1 });
    let selection = *app.selection();
    app.input(UiInput::Key(Key::Down), now);
    assert_eq!(*app.selection(), selection);
    app.resize(crate::terminal::TermSize { cols: 99, rows: 30 });
    app.input(UiInput::Key(Key::Down), now);
    assert_eq!(
        app.selection().id(),
        Some(id(2)),
        "a now-short commit moves directly"
    );
}

// Selected commits preserve foreground readability; only usernames take lane colors, only starred counts are yellow, and every line has a focus-aware bar.
#[test]
fn selection_and_counts_use_semantic_ansi_styles() {
    use ratatui::style::{Color, Modifier};
    let now = Instant::now();
    for full in [false, true] {
        for focus in [Focus::List, Focus::Compose] {
            for starred in [None, Some(false), Some(true)] {
                let mut app = scrolling_app(full, 80, 30);
                app.focus = focus;
                let mut selected = commit(1);
                selected.images.clear();
                selected.starred = starred;
                selected.body = "plain body".into();
                app.update_commit(CommitEntry::Live(Box::new(selected)));
                let mut terminal =
                    ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 30)).unwrap();
                terminal
                    .draw(|frame| app.render(frame, jiff::Timestamp::UNIX_EPOCH))
                    .unwrap();
                let area = layout::areas(&app, app.size).list;
                let buffer = terminal.backend().buffer();
                for row in area.y..area.y + 4 {
                    assert_eq!(buffer[(0, row)].symbol(), "│");
                    assert_eq!(
                        buffer[(0, row)].fg,
                        if focus == Focus::List {
                            Color::Cyan
                        } else {
                            Color::DarkGray
                        }
                    );
                    for col in 0..area.width {
                        let cell = &buffer[(col, row)];
                        assert_eq!(cell.bg, Color::Reset);
                        assert!(!cell.modifier.contains(Modifier::REVERSED));
                        // Only the username takes its lane color, the fixture's lane 4.
                        if row == area.y && (2..2 + "@someone".len() as u16).contains(&col) {
                            assert_eq!(cell.fg, Color::Rgb(0xf0, 0x65, 0x95));
                        } else {
                            assert!(!matches!(cell.fg, Color::Rgb(..) | Color::Indexed(..)));
                        }
                    }
                }
                assert!(buffer[(2, area.y)].modifier.contains(Modifier::BOLD));
                assert_eq!(buffer[(2, area.y + 1)].fg, Color::Reset);
                let counts: String = (2..area.width)
                    .map(|x| buffer[(x, area.y + 2)].symbol())
                    .collect();
                let expected = if starred == Some(true) {
                    "* 1 stars  0 replies"
                } else {
                    "1 stars  0 replies"
                };
                assert!(counts.starts_with(expected));
                let stars_end = expected.find("  ").unwrap();
                for x in 2..2 + stars_end as u16 {
                    assert_eq!(
                        buffer[(x, area.y + 2)].fg,
                        if starred == Some(true) {
                            Color::Yellow
                        } else {
                            Color::Reset
                        }
                    );
                }
                for x in 2 + stars_end as u16..2 + expected.len() as u16 {
                    assert_eq!(buffer[(x, area.y + 2)].fg, Color::Reset);
                }
                // A body that fits moves immediately, independently of prior rendering.
                app.focus = Focus::List;
                app.input(UiInput::Key(Key::Down), now);
                assert_eq!(app.selection().id(), Some(id(2)));
            }
        }
    }
}

mod floating;
mod scrolling;

// A delayed creation rejection must receive its full display interval when the overlay reopens.
#[test]
fn unseen_rejection_waits_for_reopening() {
    let mut app = app();
    let now = Instant::now();
    ready(&mut app, now);
    app.pending.push(id(7));
    app.input(UiInput::Toggle, now);
    let mut rejected = commit(7);
    rejected.status = crate::api::CommitStatus::Rejected;
    app.answer(
        Request::Create(NewCommit {
            body: crate::api::CommitBody::new("rejected body").unwrap(),
            target: None,
        }),
        Ok(Response::Commit(rejected)),
        now,
    );
    let reopened = now + Duration::from_secs(60);
    app.open(reopened);
    assert!(
        matches!(&app.popup, Some(Popup::Toast { message, until }) if message.contains("rejected") && *until == reopened + Popup::TOAST_TIME)
    );
    app.tick(reopened + Popup::TOAST_TIME);
    assert!(app.popup.is_none());
}

// Plain 404 ends moderation polling; transient server errors must retain the pending commit.
#[test]
fn moderation_plain_404_is_final_but_server_failure_retries() {
    for (status, retained) in [(404, false), (503, true), (429, true)] {
        let mut app = app();
        let now = Instant::now();
        ready(&mut app, now);
        app.pending.push(id(7));
        app.answer(
            Request::Moderation(id(7)),
            Err(Failure::Api(ApiError::Status {
                status: reqwest::StatusCode::from_u16(status).unwrap(),
                code: None,
                message: "failure".into(),
                retry_after: None,
            })),
            now,
        );
        assert_eq!(app.pending.contains(&id(7)), retained);
        let effects = app.tick(now + Duration::from_secs(61));
        assert_eq!(
            effects.contains(&Effect::Send(Request::Moderation(id(7)))),
            retained
        );
    }
}

// Exhausted replies must query the newest end, including bursts larger than a page, without duplicates or selection jumps.
#[test]
fn exhausted_replies_catch_up_from_newest_pages() {
    let mut app = app();
    let now = Instant::now();
    ready(&mut app, now);
    let mut view = ThreadView::new(id(1000));
    view.set_thread(Thread {
        ancestors: vec![],
        commit: commit(1000),
    });
    view.refresh_replies(page(&(1..=50).collect::<Vec<_>>(), Some("second")));
    view.replies_mut()
        .append(page(&(51..=60).collect::<Vec<_>>(), None));
    view.selection_mut()
        .select(&(1..=60).map(id).collect::<Vec<_>>(), id(60));
    app.threads.push(view);
    let request = Request::NewestReplies {
        id: id(1000),
        cursor: None,
    };
    assert!(
        app.tick(now + Duration::from_secs(5))
            .contains(&Effect::Send(request.clone()))
    );
    app.answer(
        request.clone(),
        Ok(Response::Entries(page(
            &(12..=61).rev().collect::<Vec<_>>(),
            Some("older"),
        ))),
        now,
    );
    let view = app.threads.last().unwrap();
    assert_eq!(
        view.replies()
            .entries()
            .iter()
            .map(CommitEntry::id)
            .collect::<Vec<_>>(),
        (1..=61).map(id).collect::<Vec<_>>()
    );
    assert_eq!(view.selection().id(), Some(id(60)));
    assert!(view.replies().exhausted());
    let next = Request::NewestReplies {
        id: id(1000),
        cursor: Some("catchup".to_owned().into()),
    };
    let effects = app.answer(
        request.clone(),
        Ok(Response::Entries(page(
            &(82..=131).rev().collect::<Vec<_>>(),
            Some("catchup"),
        ))),
        now,
    );
    assert_eq!(effects, vec![Effect::Send(next.clone())]);
    assert!(
        !app.tick(now + Duration::from_secs(10))
            .contains(&Effect::Send(request.clone()))
    );
    app.answer(
        next,
        Ok(Response::Entries(page(
            &(32..=81).rev().collect::<Vec<_>>(),
            Some("older"),
        ))),
        now,
    );
    app.answer(
        request,
        Ok(Response::Entries(page(
            &(82..=131).rev().collect::<Vec<_>>(),
            Some("older"),
        ))),
        now,
    );
    assert_eq!(
        app.threads
            .last()
            .unwrap()
            .replies()
            .entries()
            .iter()
            .map(CommitEntry::id)
            .collect::<Vec<_>>(),
        (1..=131).map(id).collect::<Vec<_>>()
    );
}

// Multiple unseen creation rejections must survive unrelated errors and be shown one at a time after reopening.
#[test]
fn unseen_rejections_are_queued_behind_other_popups() {
    let mut app = app();
    let now = Instant::now();
    for n in [7, 8] {
        let mut rejected = commit(n);
        rejected.status = crate::api::CommitStatus::Rejected;
        app.answer(
            Request::Create(NewCommit {
                body: crate::api::CommitBody::new("rejected body").unwrap(),
                target: None,
            }),
            Ok(Response::Commit(rejected)),
            now,
        );
    }
    app.popup = Some(Popup::toast("other error", now));
    let reopened = now + Duration::from_secs(60);
    app.open(reopened);
    assert!(
        matches!(&app.popup, Some(Popup::Toast {message, until}) if message.contains("rejected") && *until == reopened + Popup::TOAST_TIME)
    );
    app.input(UiInput::Key(Key::Esc), reopened);
    assert!(
        matches!(&app.popup, Some(Popup::Toast {message, until}) if message.contains("rejected") && *until == reopened + Popup::TOAST_TIME)
    );
}

// Standalone Esc closes only at the top, and every help context explains quitting without a toggle.
#[test]
fn standalone_escape_and_help() {
    let now = Instant::now();
    let mut app = App::new(None, LogKind::Following, Duration::from_secs(5));
    ready(&mut app, now);
    app.popup = Some(Popup::Toast {
        message: "notice".to_owned(),
        until: now + Duration::from_secs(5),
    });
    assert!(app.input(UiInput::Key(Key::Esc), now).is_empty());
    assert!(app.opened);
    assert_eq!(app.input(UiInput::Key(Key::Esc), now), vec![Effect::Close]);
    assert!(!app.opened);
    for context in [
        help::HelpContext::Compose,
        help::HelpContext::Send,
        help::HelpContext::Timeline,
        help::HelpContext::Thread,
        help::HelpContext::ConfirmDelete,
    ] {
        let entries = help::entries(context, app.toggle);
        // List help places gg directly after vertical navigation.
        if matches!(
            context,
            help::HelpContext::Timeline | help::HelpContext::Thread
        ) {
            let navigation = entries.iter().position(|(key, _)| key == "j / k").unwrap();
            assert_eq!(entries[navigation + 1], ("gg".to_owned(), "top"));
        } else {
            assert!(!entries.iter().any(|(key, _)| key == "gg"));
        }
        assert!(entries.contains(&("Esc".to_owned(), "back / quit")));
        assert_eq!(entries.iter().filter(|(key, _)| key == "Esc").count(), 1);
        assert!(
            !entries
                .iter()
                .any(|(key, action)| key.starts_with("ctrl+") || *action == "close")
        );
    }
}

mod notifications;

mod profile;
