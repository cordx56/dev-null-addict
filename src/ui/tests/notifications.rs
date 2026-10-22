use super::*;
use crate::api::{NotificationGroup, NotificationId, NotificationKind, NotificationPage};

fn notification(n: u128, kind: NotificationKind) -> NotificationGroup {
    NotificationGroup {
        id: uuid::Uuid::from_u128(n).into(),
        kind,
        reason: None,
        read_at: Some("2026-10-22T12:00:00Z".parse().unwrap()),
        created_at: jiff::Timestamp::from_second(1000 + n as i64).unwrap(),
        actor: commit(1).author,
        actors: vec![],
        actors_count: 1,
        event_count: 1,
        unread_count: 0,
        commit: (!matches!(
            kind,
            NotificationKind::Followed | NotificationKind::LeveledUp
        ))
        .then(|| commit(n)),
    }
}

fn notifications(items: Vec<NotificationGroup>, next: Option<&str>) -> NotificationPage {
    NotificationPage {
        page: Page {
            items,
            next_cursor: next.map(|s| s.to_owned().into()),
        },
        read_at: "2026-10-22T12:00:00Z".parse().unwrap(),
    }
}

fn receive(app: &mut App, items: Vec<NotificationGroup>, now: Instant) {
    app.answer(
        Request::Notifications { cursor: None },
        Ok(Response::Notifications(notifications(items, None))),
        now,
    );
}

fn me(unread_notifications: u32) -> Response {
    Response::CurrentUser(CurrentUser {
        user: commit(1).author,
        unread_notifications,
    })
}

fn visit(app: &mut App, now: Instant) {
    app.focus = Focus::List;
    for _ in 0..3 {
        app.input(UiInput::Key(Key::Char('l')), now);
    }
    assert_eq!(app.timeline.active(), Tab::Notifications);
}

fn draw(app: &App) -> ratatui::buffer::Buffer {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(
        app.size.width,
        app.size.height,
    ))
    .unwrap();
    terminal
        .draw(|frame| app.render(frame, "2026-10-22T12:00:00Z".parse().unwrap()))
        .unwrap();
    terminal.backend().buffer().clone()
}

fn row(buffer: &ratatui::buffer::Buffer, y: u16, area: ratatui::layout::Rect) -> String {
    (area.x..area.right())
        .map(|x| buffer[(x, y)].symbol())
        .collect()
}

// Refreshes poll Me without duplicating notification requests.
#[test]
fn refresh_checks_me_and_deduplicates_notifications() {
    let now = Instant::now();
    let mut app = app();
    let first = app.open(now);
    assert!(first.contains(&Effect::Send(Request::Me)));
    let request = Request::Notifications { cursor: None };
    assert_eq!(
        app.answer(Request::Me, Ok(me(2)), now),
        vec![Effect::Send(request.clone())]
    );
    assert!(app.answer(Request::Me, Ok(me(2)), now).is_empty());
    app.answer(
        Request::Log {
            kind: LogKind::Following,
            cursor: None,
        },
        Ok(Response::Entries(page(&[1], None))),
        now,
    );
    let second = app.tick(now + Duration::from_secs(5));
    assert!(second.contains(&Effect::Send(Request::Me)));
    assert!(second.contains(&Effect::Send(Request::Log {
        kind: LogKind::Following,
        cursor: None
    })));
    assert!(app.answer(Request::Me, Ok(me(1)), now).is_empty());
    visit(&mut app, now);
    assert_eq!(app.inflight.iter().filter(|r| **r == request).count(), 1);
    assert!(app.answer(Request::Me, Ok(me(2)), now).is_empty());
    receive(&mut app, vec![], now);
    assert!(!app.inflight.contains(&request));
    assert!(app.answer(Request::Me, Ok(me(2)), now).is_empty());
    assert!(!app.inflight.contains(&request));
    let third = app.tick(now + Duration::from_secs(10));
    assert!(third.contains(&Effect::Send(Request::Me)));
    assert!(third.contains(&Effect::Send(request)));
    assert!(
        !third
            .iter()
            .any(|effect| matches!(effect, Effect::Send(Request::Log { .. })))
    );
    app.input(UiInput::Toggle, now);
    assert!(app.answer(Request::Me, Ok(me(2)), now).is_empty());
    assert!(app.tick(now + Duration::from_secs(100)).is_empty());
}

// New groups replace the banner and expire at exactly three seconds.
#[test]
fn banner_freshness_replacement_and_deadline() {
    let now = Instant::now();
    let mut app = app();
    ready(&mut app, now);
    let mut newest = notification(3, NotificationKind::Starred);
    newest.actors_count = 3;
    newest.commit.as_mut().unwrap().body = "first line\nsecond line".into();
    let mut old = notification(4, NotificationKind::Mentioned);
    old.read_at = Some("2026-10-22T11:59:59Z".parse().unwrap());
    receive(
        &mut app,
        vec![
            notification(1, NotificationKind::Followed),
            newest.clone(),
            old.clone(),
        ],
        now,
    );
    let banner = app.banner.as_ref().unwrap();
    assert!(banner.text.starts_with("@someone and 2 others"));
    assert!(banner.text.contains("starred your commit: first line"));
    assert!(!banner.text.contains("second line"));
    assert!(banner.text.ends_with(" (+1 more)"));
    assert_eq!(app.notifications.new.len(), 2);
    assert_eq!(app.deadline(), Some(now + Banner::TIME));
    let replacement = now + Duration::from_secs(1);
    newest.id = uuid::Uuid::from_u128(5).into();
    receive(&mut app, vec![newest], replacement);
    assert_eq!(app.notifications.new.len(), 3);
    assert!(!app.banner.as_ref().unwrap().text.contains("more)"));
    assert_eq!(
        app.banner.as_ref().unwrap().until,
        replacement + Banner::TIME
    );
    receive(&mut app, vec![old], replacement);
    assert_eq!(
        app.banner.as_ref().unwrap().until,
        replacement + Banner::TIME
    );
    app.tick(replacement + Banner::TIME - Duration::from_nanos(1));
    assert!(app.banner.is_some());
    app.tick(replacement + Banner::TIME);
    assert!(app.banner.is_none());
}

// Banners cannot intercept Escape in a thread or at the overlay root, including standalone mode.
#[test]
fn banner_does_not_consume_escape() {
    let now = Instant::now();
    for toggle in [None, Some("ctrl+g".parse().unwrap())] {
        let mut app = App::new(toggle, LogKind::Following, Duration::from_secs(5));
        ready(&mut app, now);
        app.focus = Focus::List;
        app.input(UiInput::Key(Key::Enter), now);
        receive(
            &mut app,
            vec![notification(1, NotificationKind::Followed)],
            now,
        );
        assert!(app.banner.is_some());
        assert!(app.input(UiInput::Key(Key::Esc), now).is_empty());
        assert!(app.threads.is_empty());
        assert!(app.banner.is_some());
        assert_eq!(app.input(UiInput::Key(Key::Esc), now), vec![Effect::Close]);
    }
}

// Tab navigation stops at both ends; new markers survive a visit until leaving or closing the overlay.
#[test]
fn tab_edges_labels_and_new_markers() {
    let now = Instant::now();
    for close in [false, true] {
        let mut app = app();
        ready(&mut app, now);
        receive(
            &mut app,
            vec![
                notification(1, NotificationKind::Followed),
                notification(2, NotificationKind::Replied),
            ],
            now,
        );
        assert_eq!(
            render::tab_label(&app, Tab::Notifications),
            "Notifications (2)"
        );
        let buffer = draw(&app);
        assert!(
            row(&buffer, layout::areas(&app, app.size).tabs.y + 1, app.size)
                .contains("Notifications (2)")
        );
        app.focus = Focus::List;
        assert!(app.input(UiInput::Key(Key::Char('h')), now).is_empty());
        assert_eq!(app.timeline.active(), Tab::Log(LogKind::Following));
        visit(&mut app, now);
        assert_eq!(render::tab_label(&app, Tab::Notifications), "Notifications");
        let area = layout::areas(&app, app.size).list;
        assert!(row(&draw(&app), area.y, area).contains("new"));
        assert_eq!(app.notifications.new.len(), 2);
        app.input(
            if close {
                UiInput::Toggle
            } else {
                UiInput::Key(Key::Char('l'))
            },
            now,
        );
        assert!(app.notifications.new.is_empty());
        assert_eq!(render::tab_label(&app, Tab::Notifications), "Notifications");
        if close {
            app.open(now);
        } else {
            assert_eq!(app.timeline.active(), Tab::Profile);
            assert!(app.input(UiInput::Key(Key::Char('l')), now).is_empty());
            app.input(UiInput::Key(Key::Char('h')), now);
        }
        assert!(!row(&draw(&app), area.y, area).contains("new"));
    }
}

// Notification commit actions share timeline behavior; commitless groups also hide those actions from help.
#[test]
fn notification_commit_actions_and_help() {
    let now = Instant::now();
    let mut app = app();
    app.resize(crate::terminal::TermSize {
        cols: 120,
        rows: 30,
    });
    ready(&mut app, now);
    receive(
        &mut app,
        vec![
            notification(2, NotificationKind::Starred),
            notification(1, NotificationKind::Followed),
        ],
        now,
    );
    visit(&mut app, now);
    assert_eq!(
        app.input(UiInput::Key(Key::Char('s')), now),
        vec![Effect::Send(Request::Star {
            id: id(2),
            starred: true
        })]
    );
    let mut starred = commit(2);
    starred.starred = Some(true);
    app.answer(
        Request::Star {
            id: id(2),
            starred: true,
        },
        Ok(Response::Commit(starred)),
        now,
    );
    assert_eq!(
        app.input(UiInput::Key(Key::Char('s')), now),
        vec![Effect::Send(Request::Star {
            id: id(2),
            starred: false
        })]
    );
    assert_eq!(
        app.input(UiInput::Key(Key::Enter), now),
        vec![
            Effect::Send(Request::Thread(id(2))),
            Effect::Send(Request::Replies {
                id: id(2),
                cursor: None
            })
        ]
    );
    app.input(UiInput::Key(Key::Esc), now);
    for (key, expected) in [
        ('r', crate::api::CommitTarget::Reply(id(2))),
        ('q', crate::api::CommitTarget::Quote(id(2))),
    ] {
        app.input(UiInput::Key(Key::Char(key)), now);
        assert_eq!(app.compose.target().unwrap().target, expected);
        assert_eq!(app.focus, Focus::Compose);
        app.input(UiInput::Key(Key::Backspace), now);
        app.focus = Focus::List;
    }
    app.input(UiInput::Key(Key::Char('d')), now);
    assert!(
        matches!(app.popup, Some(Popup::ConfirmDelete { id: selected, .. }) if selected == id(2))
    );
    app.input(UiInput::Key(Key::Esc), now);
    app.input(UiInput::Key(Key::Char('j')), now);
    let area = layout::areas(&app, app.size).help;
    let buffer = draw(&app);
    let help = (area.y..area.bottom())
        .map(|y| row(&buffer, y, area))
        .collect::<String>();
    assert!(help.contains("switch tab"));
    for action in [
        "open thread",
        "reply",
        "quote",
        "star / unstar",
        "delete own",
    ] {
        assert!(!help.contains(action));
    }
    for key in [
        Key::Enter,
        Key::Char('r'),
        Key::Char('q'),
        Key::Char('s'),
        Key::Char('d'),
    ] {
        assert!(app.input(UiInput::Key(key), now).is_empty());
        assert!(app.threads.is_empty());
        assert!(app.popup.is_none());
        assert!(app.compose.target().is_none());
        assert_eq!(app.focus, Focus::List);
    }
    app.answer(Request::Delete(id(2)), Ok(Response::Done), now);
    assert!(app.notifications.tab.list.entries()[0].commit.is_none());
}

// Rejected moderation polls only stop tracking, even while closed; approved polls still refresh.
#[test]
fn moderation_results_keep_only_approval_refresh() {
    let now = Instant::now();
    let mut app = app();
    ready(&mut app, now);
    for opened in [true, false] {
        app.opened = opened;
        app.pending.push(id(3));
        let mut rejected = commit(3);
        rejected.status = crate::api::CommitStatus::Rejected;
        assert!(
            app.answer(
                Request::Moderation(id(3)),
                Ok(Response::Thread(Thread {
                    ancestors: vec![],
                    commit: rejected
                })),
                now
            )
            .is_empty()
        );
        assert!(app.pending.is_empty());
        assert!(app.popup.is_none());
        assert!(app.moderation_notices.is_empty());
    }
    app.opened = true;
    app.pending.push(id(4));
    let effects = app.answer(
        Request::Moderation(id(4)),
        Ok(Response::Thread(Thread {
            ancestors: vec![],
            commit: commit(4),
        })),
        now,
    );
    assert!(app.pending.is_empty());
    assert!(effects.contains(&Effect::Send(Request::Me)));
    assert!(effects.contains(&Effect::Send(Request::Log {
        kind: LogKind::Following,
        cursor: None
    })));
}

// Notification paging preserves ID selection and viewport position, and tall reasons scroll before changing items.
#[test]
fn notification_paging_and_tall_item_navigation() {
    let now = Instant::now();
    let mut app = app();
    app.resize(crate::terminal::TermSize { cols: 80, rows: 12 });
    ready(&mut app, now);
    visit(&mut app, now);
    let mut tall = notification(3, NotificationKind::Rejected);
    tall.reason = Some(
        (0..12)
            .map(|n| format!("reason {n}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    let first = notifications(
        vec![tall, notification(2, NotificationKind::Followed)],
        Some("more"),
    );
    app.answer(
        Request::Notifications { cursor: None },
        Ok(Response::Notifications(first.clone())),
        now,
    );
    app.tick(now + Banner::TIME);
    let area = layout::areas(&app, app.size).list;
    let height = render::notification_height(&first.page.items[0], area.width);
    let bottom = height - usize::from(area.height);
    for top in 1..=bottom {
        assert!(app.input(UiInput::Key(Key::Char('j')), now).is_empty());
        assert_eq!(
            app.notifications.tab.selection.id(),
            Some(first.page.items[0].id)
        );
        assert_eq!(app.viewport().top(), top);
    }
    let request = Request::Notifications {
        cursor: Some("more".to_owned().into()),
    };
    assert_eq!(
        app.input(UiInput::Key(Key::Down), now),
        vec![Effect::Send(request.clone())]
    );
    assert_eq!(
        app.notifications.tab.selection.id(),
        Some(first.page.items[1].id)
    );
    app.input(UiInput::Key(Key::Up), now);
    assert_eq!(app.viewport().top(), bottom);
    assert_eq!(
        app.notifications.tab.selection.id(),
        Some(first.page.items[0].id)
    );
    app.input(UiInput::Key(Key::Down), now);
    app.answer(
        request,
        Ok(Response::Notifications(notifications(
            vec![
                notification(2, NotificationKind::Followed),
                notification(1, NotificationKind::Followed),
            ],
            None,
        ))),
        now,
    );
    assert_eq!(app.notifications.tab.list.entries().len(), 3);
    assert_eq!(
        app.notifications.tab.selection.id(),
        Some(first.page.items[1].id)
    );
    receive(
        &mut app,
        vec![notification(4, NotificationKind::Followed)],
        now,
    );
    let ids: Vec<_> = app
        .notifications
        .tab
        .list
        .entries()
        .iter()
        .map(|g| g.id)
        .collect();
    assert_eq!(
        ids,
        [4, 3, 2, 1].map(|n| NotificationId::from(uuid::Uuid::from_u128(n)))
    );
    assert_eq!(
        app.notifications.tab.selection.id(),
        Some(first.page.items[1].id)
    );
    app.resize(crate::terminal::TermSize { cols: 80, rows: 14 });
    app.input(UiInput::Key(Key::Down), now);
    let top = app.viewport().top();
    app.input(UiInput::Key(Key::Up), now);
    assert_eq!(
        app.viewport().top(),
        top,
        "visible selections must not move the viewport"
    );
    let before = *app.viewport();
    draw(&app);
    assert_eq!(*app.viewport(), before);
}

// Banners occupy the inner bottom three rows in float, fullscreen and standalone, with popups painted last.
#[test]
fn banner_geometry_and_popup_layering() {
    let now = Instant::now();
    for (mode, toggle) in [
        (
            layout::OverlayLayout::Float,
            Some("ctrl+g".parse().unwrap()),
        ),
        (
            layout::OverlayLayout::FullScreen,
            Some("ctrl+g".parse().unwrap()),
        ),
        (layout::OverlayLayout::FullScreen, None),
    ] {
        let mut app = App::new(toggle, LogKind::Following, Duration::from_secs(5));
        app.set_layout(mode);
        app.resize(crate::terminal::TermSize {
            cols: 100,
            rows: 24,
        });
        app.banner = Some(Banner {
            text: "banner text".into(),
            until: now + Banner::TIME,
        });
        let area = layout::areas(&app, app.size).window.inner;
        let buffer = draw(&app);
        assert!(row(&buffer, area.bottom() - 3, area).contains(" Notification "));
        assert!(row(&buffer, area.bottom() - 2, area).contains("banner text"));
        assert_eq!(buffer[(area.x, area.bottom() - 1)].symbol(), "└");
        let popup = (0..area.height)
            .map(|_| "popup foreground")
            .collect::<Vec<_>>()
            .join("\n");
        app.popup = Some(Popup::toast(popup, now));
        let buffer = draw(&app);
        let line = row(&buffer, area.bottom() - 2, area);
        assert!(line.contains("popup foreground"));
        let popup_x = area.x + (area.width - 70) / 2;
        assert_eq!(buffer[(popup_x, area.bottom() - 3)].symbol(), "│");
        assert_eq!(buffer[(popup_x, area.bottom() - 1)].symbol(), "└");
    }
}

// Repeated fresh IDs neither restart banners nor inflate their counts.
#[test]
fn repeated_notification_answers_are_not_announced_twice() {
    let now = Instant::now();
    for append in [false, true] {
        let mut app = app();
        ready(&mut app, now);
        let groups = vec![
            notification(9, NotificationKind::Starred),
            notification(1, NotificationKind::Followed),
        ];
        receive(&mut app, groups.clone(), now);
        let original = app.banner.as_ref().unwrap().text.clone();
        let repeat = |app: &mut App, at| {
            app.answer(
                Request::Notifications {
                    cursor: append.then(|| "next".to_owned().into()),
                },
                Ok(Response::Notifications(notifications(groups.clone(), None))),
                at,
            );
        };
        repeat(&mut app, now + Duration::from_millis(100));
        assert_eq!(app.banner.as_ref().unwrap().text, original);
        assert_eq!(app.banner.as_ref().unwrap().until, now + Banner::TIME);
        assert_eq!(app.notifications.new.len(), 2);
        app.tick(now + Banner::TIME);
        repeat(&mut app, now + Banner::TIME);
        assert!(app.banner.is_none());
        let later = now + Banner::TIME;
        let mixed = vec![
            groups[0].clone(),
            notification(3, NotificationKind::Replied),
            notification(2, NotificationKind::Followed),
        ];
        receive(&mut app, mixed.clone(), later);
        let text = app.banner.as_ref().unwrap().text.clone();
        assert!(text.contains("replied to your commit: body 3"));
        assert!(text.ends_with(" (+1 more)"));
        assert_eq!(app.notifications.new.len(), 4);
        receive(&mut app, mixed, later + Duration::from_millis(100));
        assert_eq!(app.banner.as_ref().unwrap().text, text);
        assert_eq!(app.banner.as_ref().unwrap().until, later + Banner::TIME);
        receive(&mut app, vec![], later);
        receive(&mut app, groups, later + Duration::from_millis(200));
        assert_eq!(app.banner.as_ref().unwrap().text, original);
        assert_eq!(
            app.banner.as_ref().unwrap().until,
            later + Duration::from_millis(200) + Banner::TIME
        );
    }
}

// Self-addressed notices omit actors and separate reasons from excerpts.
#[test]
fn self_addressed_notification_layout_and_banner() {
    let now = Instant::now();
    for (kind, phrase) in [
        (NotificationKind::Approved, "Your commit was approved"),
        (NotificationKind::Rejected, "Your commit was rejected"),
        (NotificationKind::LeveledUp, "You reached a new level"),
    ] {
        for reason in [None, Some("Looks like spam.")] {
            let mut app = app();
            app.resize(crate::terminal::TermSize {
                cols: 120,
                rows: 30,
            });
            ready(&mut app, now);
            let mut group = notification(2, kind);
            group.created_at = "2026-10-22T12:00:00Z".parse().unwrap();
            group.actors_count = 3;
            if kind == NotificationKind::Rejected {
                group.reason = reason.map(str::to_owned);
            }
            if let Some(commit) = &mut group.commit {
                commit.body = "buy cheap stuff\nsecond line".into();
            }
            assert_eq!(super::super::notifications::wording(&group), phrase);
            receive(
                &mut app,
                vec![group, notification(1, NotificationKind::Followed)],
                now,
            );
            let excerpt = if kind == NotificationKind::LeveledUp {
                ""
            } else {
                ": buy cheap stuff"
            };
            let explanation = if kind == NotificationKind::Rejected && reason.is_some() {
                " (reason: Looks like spam.)"
            } else {
                ""
            };
            assert_eq!(
                app.banner.as_ref().unwrap().text,
                format!("now {phrase}{excerpt}{explanation} (+1 more)")
            );
            visit(&mut app, now);
            let area = layout::areas(&app, app.size).list;
            let buffer = draw(&app);
            assert_eq!(row(&buffer, area.y, area).trim(), "│ now  new");
            assert_eq!(row(&buffer, area.y + 1, area).trim(), format!("│ {phrase}"));
            let mut body_row = area.y + 2;
            if kind == NotificationKind::Rejected && reason.is_some() {
                assert_eq!(
                    row(&buffer, body_row, area).trim(),
                    "│ Reason: Looks like spam."
                );
                body_row += 1;
            }
            if kind != NotificationKind::LeveledUp {
                assert_eq!(row(&buffer, body_row, area).trim(), "│ buy cheap stuff");
                assert_eq!(row(&buffer, body_row + 1, area).trim(), "│ second line");
            } else {
                assert_eq!(row(&buffer, body_row, area).trim(), "│");
            }
        }
    }
}
