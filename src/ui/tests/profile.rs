use super::*;
use crate::api::{CommitStatus, ErrorCode, Lane, Profile};

fn profile(username: &str) -> Profile {
    Profile {
        user: crate::api::User {
            username: username.into(),
            name: Some("Display Name".into()),
            lane: Some(Lane::Automatic(2)),
        },
        bio: Some("bio one\nbio two\nbio three\nbio four".into()),
        company: Some("Company".into()),
        location: Some("Location".into()),
        website: Some("example.test".into()),
        created_at: "2026-10-21T00:30:00+09:00".parse().unwrap(),
        commits_count: Some(12),
        followers_count: Some(3),
        following_count: None,
        you_follow: false,
        follows_you: true,
        pinned_commit: None,
    }
}

fn latest(now: Instant) -> App {
    let mut app = app();
    ready(&mut app, now);
    app.focus = Focus::List;
    app.input(UiInput::Key(Key::Char('l')), now);
    let mut entries = page(&[3, 2, 1], None);
    entries.items.iter_mut().for_each(|entry| {
        if let CommitEntry::Live(commit) = entry {
            commit.author.username = "author".into();
            commit.images.clear();
        }
    });
    app.answer(
        Request::Log {
            kind: LogKind::Latest,
            cursor: None,
        },
        Ok(Response::Entries(entries)),
        now,
    );
    app
}

fn receive_profile(app: &mut App, profile: Profile, now: Instant) {
    app.answer(
        Request::Profile(profile.user.username.clone()),
        Ok(Response::Profile(Box::new(profile))),
        now,
    );
}

fn receive_commits(app: &mut App, username: &str, ids: &[u128], next: Option<&str>, now: Instant) {
    let mut page = page(ids, next);
    page.items.iter_mut().for_each(|entry| {
        if let CommitEntry::Live(commit) = entry {
            commit.images.clear();
        }
    });
    app.answer(
        Request::UserCommits {
            username: username.into(),
            cursor: None,
        },
        Ok(Response::Entries(page)),
        now,
    );
}

fn key(app: &mut App, key: Key, now: Instant) -> Vec<Effect> {
    app.input(UiInput::Key(key), now)
}

fn rendered(app: &App) -> Vec<String> {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(
        app.size.width,
        app.size.height,
    ))
    .unwrap();
    terminal
        .draw(|frame| app.render(frame, jiff::Timestamp::UNIX_EPOCH))
        .unwrap();
    let buffer = terminal.backend().buffer();
    (0..app.size.height)
        .map(|y| {
            (0..app.size.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect()
}

// Opening an author from Latest keeps its selection and viewport for Esc.
#[test]
fn author_from_latest_returns_to_selected_commit() {
    let now = Instant::now();
    let mut app = latest(now);
    key(&mut app, Key::Char('j'), now);
    let selection = *app.selection();
    let viewport = *app.viewport();
    let effects = key(&mut app, Key::Char('p'), now);
    assert!(effects.contains(&Effect::Send(Request::Profile("author".into()))));
    assert!(effects.contains(&Effect::Send(Request::UserCommits {
        username: "author".into(),
        cursor: None
    })));
    assert_eq!(app.timeline.active(), Tab::Profile);
    assert_eq!(
        app.profiles.shown().unwrap().username,
        Username::from("author")
    );
    assert!(app.threads.is_empty());
    assert_eq!(app.back.len(), 1);
    key(&mut app, Key::Esc, now);
    assert_eq!(app.timeline.active(), Tab::Log(LogKind::Latest));
    assert_eq!(*app.selection(), selection);
    assert_eq!(*app.viewport(), viewport);
    assert!(app.profiles.other.is_none());
    assert!(app.back.is_empty());
}

// A profile opened inside a thread saves that thread, including late replies.
#[test]
fn author_from_thread_restores_thread() {
    let now = Instant::now();
    let mut app = latest(now);
    key(&mut app, Key::Enter, now);
    let mut root = commit(3);
    root.author.username = "author".into();
    app.answer(
        Request::Thread(id(3)),
        Ok(Response::Thread(Thread {
            ancestors: vec![],
            commit: root,
        })),
        now,
    );
    key(&mut app, Key::Char('p'), now);
    assert!(app.threads.is_empty());
    app.answer(
        Request::Replies {
            id: id(3),
            cursor: None,
        },
        Ok(Response::Entries(page(&[4], None))),
        now,
    );
    key(&mut app, Key::Esc, now);
    assert_eq!(app.threads.last().unwrap().id(), id(3));
    assert_eq!(app.selection().id(), Some(id(3)));
    assert_eq!(app.threads.last().unwrap().ids(), vec![id(3), id(4)]);
    key(&mut app, Key::Esc, now);
    assert!(app.threads.is_empty());
    assert_eq!(app.timeline.active(), Tab::Log(LogKind::Latest));
}

// Tab navigation discards p history, and a username arriving later starts both profile requests.
#[test]
fn tab_switch_uses_own_profile_and_waits_for_me() {
    let now = Instant::now();
    let mut app = latest(now);
    key(&mut app, Key::Char('p'), now);
    key(&mut app, Key::Char('h'), now);
    assert!(app.profiles.other.is_none());
    assert!(app.back.is_empty());
    let effects = key(&mut app, Key::Char('l'), now);
    assert!(effects.contains(&Effect::Send(Request::Profile("someone".into()))));
    assert_eq!(
        app.profiles.shown().unwrap().username,
        Username::from("someone")
    );

    let mut waiting = super::app();
    waiting.timeline.switch(Tab::Profile);
    assert_eq!(waiting.open(now), vec![Effect::Send(Request::Me)]);
    waiting.focus = Focus::List;
    assert!(rendered(&waiting).join("\n").contains("Loading..."));
    assert!(key(&mut waiting, Key::Char('j'), now).is_empty());
    assert!(key(&mut waiting, Key::Char('p'), now).is_empty());
    let effects = waiting.answer(
        Request::Me,
        Ok(Response::CurrentUser(CurrentUser {
            user: commit(1).author,
            unread_notifications: 1,
        })),
        now,
    );
    assert!(effects.contains(&Effect::Send(Request::Profile("someone".into()))));
    assert!(effects.contains(&Effect::Send(Request::UserCommits {
        username: "someone".into(),
        cursor: None
    })));
    assert!(effects.contains(&Effect::Send(Request::Notifications { cursor: None })));
    let mut user = commit(1).author;
    user.username = "renamed".into();
    let effects = waiting.answer(
        Request::Me,
        Ok(Response::CurrentUser(CurrentUser {
            user,
            unread_notifications: 0,
        })),
        now,
    );
    assert_eq!(
        waiting.profiles.own.as_ref().unwrap().username,
        Username::from("renamed")
    );
    assert!(effects.contains(&Effect::Send(Request::Profile("renamed".into()))));
}

// Notification p always uses its actor, even without a commit or with a different author.
#[test]
fn notification_actor_and_own_targets() {
    let now = Instant::now();
    for (actor, has_commit) in [
        ("actor", false),
        ("actor", true),
        ("someone", false),
        ("someone", true),
    ] {
        let mut app = latest(now);
        app.timeline.switch(Tab::Notifications);
        let group: crate::api::NotificationGroup = serde_json::from_value(serde_json::json!({
            "id": id(20), "type": "followed", "created_at": "2026-10-22T12:00:00Z", "read_at": "2026-10-22T12:00:00Z",
            "actor": {"username": actor}, "actors_count": 1, "event_count": 1, "unread_count": 1,
            "commit": if has_commit { Some(commit(2)) } else { None },
        })).unwrap();
        let group_id = group.id;
        app.answer(
            Request::Notifications { cursor: None },
            Ok(Response::Notifications(NotificationPage {
                page: Page {
                    items: vec![group],
                    next_cursor: None,
                },
                read_at: "2026-10-22T12:00:00Z".parse().unwrap(),
            })),
            now,
        );
        assert!(app.notifications.new.contains(&group_id));
        key(&mut app, Key::Char('p'), now);
        assert!(app.notifications.new.is_empty());
        assert_eq!(
            app.profiles.shown().unwrap().username,
            Username::from(actor)
        );
        assert_eq!(app.profiles.other.is_none(), actor == "someone");
        key(&mut app, Key::Esc, now);
        assert_eq!(app.timeline.active(), Tab::Notifications);
        assert_eq!(app.notifications.tab.selection.id(), Some(group_id));
    }
    let mut app = latest(now);
    app.timeline.switch(Tab::Log(LogKind::Following));
    key(&mut app, Key::Char('p'), now);
    assert!(app.profiles.other.is_none());
    assert_eq!(
        app.profiles.shown().unwrap().username,
        Username::from("someone")
    );
    receive_profile(&mut app, profile("someone"), now);
    receive_commits(&mut app, "someone", &[1], None, now);
    assert!(key(&mut app, Key::Char('p'), now).is_empty());
    assert_eq!(app.back.len(), 1);
    app.me = None;
    app.profiles.own = None;
    app.timeline.switch(Tab::Log(LogKind::Following));
    key(&mut app, Key::Char('p'), now);
    assert_eq!(
        app.profiles.other.as_ref().unwrap().username,
        Username::from("someone")
    );
}

// Follow is limited to a loaded, unfollowed other profile and refreshes it after success.
#[test]
fn follow_conditions_and_refresh() {
    let now = Instant::now();
    let mut app = latest(now);
    assert!(key(&mut app, Key::Char('f'), now).is_empty());
    key(&mut app, Key::Char('p'), now);
    assert!(key(&mut app, Key::Char('f'), now).is_empty());
    receive_profile(&mut app, profile("author"), now);
    app.focus = Focus::Compose;
    key(&mut app, Key::Char('f'), now);
    assert!(!app.inflight.contains(&Request::Follow("author".into())));
    app.focus = Focus::List;
    app.threads.push(ThreadView::new(id(3)));
    assert!(key(&mut app, Key::Char('f'), now).is_empty());
    app.threads.clear();
    assert_eq!(
        key(&mut app, Key::Char('f'), now),
        vec![Effect::Send(Request::Follow("author".into()))]
    );
    assert!(key(&mut app, Key::Char('f'), now).is_empty());
    assert_eq!(
        app.answer(Request::Follow("author".into()), Ok(Response::Done), now),
        vec![Effect::Send(Request::Profile("author".into()))]
    );
    let mut followed = profile("author");
    followed.you_follow = true;
    receive_profile(&mut app, followed, now);
    assert!(key(&mut app, Key::Char('f'), now).is_empty());
    key(&mut app, Key::Char('h'), now);
    key(&mut app, Key::Char('l'), now);
    receive_profile(&mut app, profile("someone"), now);
    assert!(key(&mut app, Key::Char('f'), now).is_empty());
    // A follow may finish after tab navigation discarded its profile.
    assert_eq!(
        app.answer(Request::Follow("author".into()), Ok(Response::Done), now),
        vec![Effect::Send(Request::Profile("author".into()))]
    );
}

// Permanent profile errors replace the whole list, suppress late answers and stop refresh and paging.
#[test]
fn unavailable_profiles_stop_requests_without_toasts() {
    let now = Instant::now();
    for (status, code) in [
        (403, Some(ErrorCode::UserBlocked)),
        (403, Some(ErrorCode::UserUnavailable)),
        (404, None),
    ] {
        for commits in [false, true] {
            let mut app = latest(now);
            key(&mut app, Key::Char('p'), now);
            receive_profile(&mut app, profile("author"), now);
            receive_commits(&mut app, "author", &[1], Some("next"), now);
            let request = if commits {
                Request::UserCommits {
                    username: "author".into(),
                    cursor: None,
                }
            } else {
                Request::Profile("author".into())
            };
            app.answer(
                request,
                Err(ApiError::Status {
                    status: reqwest::StatusCode::from_u16(status).unwrap(),
                    code,
                    message: "Unavailable profile".into(),
                    retry_after: None,
                }
                .into()),
                now,
            );
            assert!(app.popup.is_none());
            assert_eq!(
                app.profiles.shown().unwrap().unavailable.as_deref(),
                Some("Unavailable profile")
            );
            receive_profile(&mut app, profile("author"), now);
            receive_commits(&mut app, "author", &[2], None, now);
            assert!(app.list_entries().is_empty());
            app.inflight.clear();
            assert_eq!(app.refresh(now), vec![Effect::Send(Request::Me)]);
            assert!(key(&mut app, Key::Char('j'), now).is_empty());
            assert!(key(&mut app, Key::Char('f'), now).is_empty());
            let text = rendered(&app).join("\n");
            assert!(text.contains("Unavailable profile"));
            assert!(!text.contains("body 1"));
        }
    }
    let mut app = latest(now);
    key(&mut app, Key::Char('p'), now);
    app.answer(
        Request::Profile("author".into()),
        Err(ApiError::Status {
            status: reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            code: None,
            message: "Try again".into(),
            retry_after: None,
        }
        .into()),
        now,
    );
    assert!(app.popup.is_some());
    assert!(app.profiles.shown().unwrap().unavailable.is_none());
}

// The pin leads the combined selection; hidden moderation entries do not block cursor paging or retry.
#[test]
fn pinned_selection_and_user_commit_paging() {
    let now = Instant::now();
    let mut app = latest(now);
    key(&mut app, Key::Char('p'), now);
    let mut header = profile("author");
    let mut pinned = commit(9);
    pinned.images.clear();
    header.pinned_commit = Some(pinned);
    receive_profile(&mut app, header.clone(), now);
    let mut entries = page(&[9, 3, 2], Some("next"));
    if let CommitEntry::Live(commit) = &mut entries.items[1] {
        commit.status = CommitStatus::Pending;
    }
    if let CommitEntry::Live(commit) = &mut entries.items[2] {
        commit.status = CommitStatus::Rejected;
    }
    app.answer(
        Request::UserCommits {
            username: "author".into(),
            cursor: None,
        },
        Ok(Response::Entries(entries)),
        now,
    );
    assert_eq!(
        app.list_entries()
            .iter()
            .map(|entry| entry.id())
            .collect::<Vec<_>>(),
        vec![id(9)]
    );
    assert_eq!(app.selection().id(), Some(id(9)));
    assert!(drawn_list(&app).join("\n").contains("pinned"));
    let request = Request::UserCommits {
        username: "author".into(),
        cursor: Some("next".to_owned().into()),
    };
    assert_eq!(
        key(&mut app, Key::Char('j'), now),
        vec![Effect::Send(request.clone())]
    );
    assert!(key(&mut app, Key::Char('j'), now).is_empty());
    app.answer(
        request.clone(),
        Err(ApiError::Status {
            status: reqwest::StatusCode::BAD_GATEWAY,
            code: None,
            message: "Retry".into(),
            retry_after: None,
        }
        .into()),
        now,
    );
    key(&mut app, Key::Esc, now);
    assert_eq!(
        key(&mut app, Key::Char('j'), now),
        vec![Effect::Send(request.clone())]
    );
    app.answer(request, Ok(Response::Entries(page(&[1], None))), now);
    key(&mut app, Key::Char('j'), now);
    assert_eq!(app.selection().id(), Some(id(1)));
    key(&mut app, Key::Char('k'), now);
    assert_eq!(app.selection().id(), Some(id(9)));
    header.pinned_commit.as_mut().unwrap().status = CommitStatus::Rejected;
    receive_profile(&mut app, header, now);
    // A former pin already in a page remains governed by that page's moderation state.
    assert!(!app.profiles.shown().unwrap().is_pinned(id(9)));
}

// Header lines stay fixed while tall commits scroll within the reduced post area, including resize.
#[test]
fn fixed_header_and_post_viewport() {
    let now = Instant::now();
    let mut app = latest(now);
    app.resize(crate::terminal::TermSize {
        cols: 100,
        rows: 28,
    });
    key(&mut app, Key::Char('p'), now);
    let mut header = profile("author");
    header.you_follow = true;
    receive_profile(&mut app, header, now);
    receive_commits(&mut app, "author", &[1, 2, 3, 4, 5, 6], None, now);
    let area = layout::areas(&app, app.size).list;
    let posts = app.post_area(area);
    let text = rendered(&app);
    assert!(
        text.join("\n")
            .contains("Display Name @author  following  follows you")
    );
    assert!(
        text.join("\n")
            .contains("Company · Location · example.test")
    );
    assert!(
        text.join("\n")
            .contains("12 commits  3 followers  joined 2026-10-20")
    );
    assert!(!text.join("\n").contains("bio four"));
    assert!(posts.y > area.y);
    assert!(area.height - posts.height <= area.height / 2);
    for _ in 0..5 {
        key(&mut app, Key::Char('j'), now);
    }
    assert!(app.viewport().top() > 0);
    assert_eq!(
        &rendered(&app)[usize::from(area.y)..usize::from(posts.y)],
        &text[usize::from(area.y)..usize::from(posts.y)]
    );
    app.resize(crate::terminal::TermSize { cols: 20, rows: 12 });
    let area = layout::areas(&app, app.size).list;
    let posts = app.post_area(area);
    assert_eq!(posts.height, area.height - area.height / 2);
    let selected = app.selection().id();
    assert_eq!(selected, Some(id(6)));
    let mut tall = commit(6);
    tall.images.clear();
    tall.body = "line\n".repeat(10);
    app.answer(
        Request::Star {
            id: id(6),
            starred: true,
        },
        Ok(Response::Commit(tall)),
        now,
    );
    let before = app.viewport().top();
    key(&mut app, Key::Char('j'), now);
    assert_eq!(app.selection().id(), selected);
    assert_eq!(app.viewport().top(), before + 1);
    key(&mut app, Key::Char('k'), now);
    assert_eq!(app.selection().id(), selected);
    assert_eq!(app.viewport().top(), before);
}

// Mutation answers reach pins, lists and threads even while saved behind another profile.
#[test]
fn mutations_update_every_kept_view() {
    let now = Instant::now();
    let mut app = latest(now);
    let mut own = profile("someone");
    own.pinned_commit = Some(commit(1));
    receive_profile(&mut app, own, now);
    key(&mut app, Key::Char('p'), now);
    let mut other = profile("author");
    other.pinned_commit = Some(commit(1));
    receive_profile(&mut app, other, now);
    receive_commits(&mut app, "author", &[1, 2], None, now);
    key(&mut app, Key::Enter, now);
    app.answer(
        Request::Thread(id(1)),
        Ok(Response::Thread(Thread {
            ancestors: vec![],
            commit: commit(1),
        })),
        now,
    );
    key(&mut app, Key::Char('p'), now);
    assert!(app.profiles.other.is_none());
    assert_eq!(app.back.len(), 2);
    let mut starred = commit(1);
    starred.starred = Some(true);
    starred.stars_count = Some(2);
    app.answer(
        Request::Star {
            id: id(1),
            starred: true,
        },
        Ok(Response::Commit(starred)),
        now,
    );
    for view in app
        .profiles
        .own
        .iter()
        .chain(app.back.iter().filter_map(|place| place.other.as_ref()))
    {
        assert!(
            matches!(view.entries().next(), Some(CommitEntry::Live(commit)) if commit.starred == Some(true))
        );
    }
    assert!(
        matches!(app.back.last().unwrap().threads[0].entries().next(), Some(CommitEntry::Live(commit)) if commit.starred == Some(true))
    );
    app.answer(Request::Delete(id(1)), Ok(Response::Done), now);
    for view in app
        .profiles
        .own
        .iter()
        .chain(app.back.iter().filter_map(|place| place.other.as_ref()))
    {
        assert!(!view.entries().any(|entry| entry.id() == id(1)));
        assert!(view.profile.as_ref().unwrap().pinned_commit.is_none());
    }
    key(&mut app, Key::Esc, now);
    assert!(
        app.threads
            .last()
            .unwrap()
            .entries()
            .all(|entry| entry.id() != id(1))
    );
    key(&mut app, Key::Esc, now);
    assert_eq!(app.selection().id(), Some(id(2)));
}

// Help follows actor availability and follow eligibility without exposing commit actions for a follow notification.
#[test]
fn contextual_profile_and_follow_help() {
    let now = Instant::now();
    let mut app = latest(now);
    app.resize(crate::terminal::TermSize {
        cols: 120,
        rows: 30,
    });
    assert!(
        rendered(&app)
            .iter()
            .any(|line| line.contains("p         profile"))
    );
    assert!(
        !rendered(&app)
            .iter()
            .any(|line| line.contains("f         follow"))
    );
    key(&mut app, Key::Char('p'), now);
    assert!(
        !rendered(&app)
            .iter()
            .any(|line| line.contains("p         profile"))
    );
    receive_profile(&mut app, profile("author"), now);
    assert!(
        rendered(&app)
            .iter()
            .any(|line| line.contains("f         follow"))
    );
    app.profiles
        .other
        .as_mut()
        .unwrap()
        .profile
        .as_mut()
        .unwrap()
        .you_follow = true;
    assert!(
        !rendered(&app)
            .iter()
            .any(|line| line.contains("f         follow"))
    );
    app.timeline.switch(Tab::Notifications);
    let group: crate::api::NotificationGroup = serde_json::from_value(serde_json::json!({
        "id": id(20), "type": "followed", "created_at": "2026-10-22T12:00:00Z", "actor": {"username": "actor"},
        "actors_count": 1, "event_count": 1, "unread_count": 0,
    })).unwrap();
    app.notifications.tab.list.refresh(Page {
        items: vec![group],
        next_cursor: None,
    });
    app.notifications.tab.reanchor();
    let text = rendered(&app).join("\n");
    assert!(text.contains("p         profile"));
    assert!(!text.contains("Enter     open thread"));
    assert!(!text.contains("r         reply"));
}

// Upstream nests user fields in data and relationships outside it; extras and absent optional fields remain compatible.
#[test]
fn upstream_profile_deserializes_and_round_trips() {
    let raw = serde_json::json!({
        "data": { "id": 42, "username": "author", "name": "Display Name", "avatar_url": "https://example.test/avatar",
            "bio": "About", "company": "Company", "location": "Location", "website": "example.test", "github_url": "https://github.com/author",
            "html_url": "https://dev-null.sh/@author", "feed_url": "https://dev-null.sh/@author/feed", "created_at": "2026-10-20T12:00:00Z",
            "commits_count": 12, "followers_count": 3, "following_count": 4, "lane": "#123456" },
        "is_following": true, "follows_you": true, "is_muting": false, "pasteis_received": 7,
        "activity": {}, "streaks": {}, "achievements": [], "pinned_commit": crate::test_support::commit_json(1)
    });
    let profile: Profile = serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(profile.user.username, Username::from("author"));
    assert_eq!(profile.user.lane, Some(Lane::Picked("#123456".into())));
    assert!(profile.you_follow && profile.follows_you);
    assert_eq!(profile.following_count, Some(4));
    assert_eq!(profile.pinned_commit.as_ref().unwrap().id, id(1));
    let encoded = serde_json::to_value(&profile).unwrap();
    assert_eq!(encoded["data"]["username"], "author");
    assert_eq!(encoded["is_following"], true);
    assert_eq!(serde_json::from_value::<Profile>(encoded).unwrap(), profile);
    let mut minimal = raw;
    minimal.as_object_mut().unwrap().remove("is_following");
    minimal.as_object_mut().unwrap().remove("follows_you");
    minimal["data"]
        .as_object_mut()
        .unwrap()
        .remove("commits_count");
    minimal["data"]["following_count"] = serde_json::Value::Null;
    minimal["pinned_commit"] = serde_json::Value::Null;
    let profile: Profile = serde_json::from_value(minimal).unwrap();
    assert!(!profile.you_follow && !profile.follows_you);
    assert!(profile.commits_count.is_none() && profile.following_count.is_none());
    assert!(profile.pinned_commit.is_none());
}

// Neither response order exposes posts before the header or duplicates the loading status.
#[test]
fn loading_waits_for_header_before_exposing_commits() {
    let now = Instant::now();
    for overlay in [
        layout::OverlayLayout::FullScreen,
        layout::OverlayLayout::Float,
    ] {
        for commits_first in [false, true] {
            for pinned in [false, true] {
                let mut app = latest(now);
                app.resize(crate::terminal::TermSize {
                    cols: 120,
                    rows: 50,
                });
                app.set_layout(overlay);
                key(&mut app, Key::Char('p'), now);
                let area = layout::areas(&app, app.size).list;
                assert_eq!(app.post_area(area), area);
                assert_eq!(rendered(&app).join("\n").matches("Loading...").count(), 1);
                if commits_first {
                    receive_commits(&mut app, "author", &[3, 2], Some("next"), now);
                }
                let text = rendered(&app);
                assert_eq!(text.join("\n").matches("Loading...").count(), 1);
                assert!(
                    text[usize::from(area.y)]
                        .chars()
                        .skip(usize::from(area.x))
                        .collect::<String>()
                        .starts_with(" Loading...")
                );
                assert!(!text.join("\n").contains("body 3"));
                assert!(app.list_entries().is_empty());
                assert_eq!(app.selection().id(), None);
                assert_eq!(app.viewport().top(), 0);
                for input in [
                    Key::Char('j'),
                    Key::Char('k'),
                    Key::Enter,
                    Key::Char('p'),
                    Key::Char('s'),
                ] {
                    assert!(key(&mut app, input, now).is_empty());
                }
                assert!(app.threads.is_empty());
                assert_eq!(app.selection().id(), None);
                let mut header = profile("author");
                header.pinned_commit = pinned.then(|| commit(9));
                receive_profile(&mut app, header, now);
                let posts = app.post_area(area);
                assert!(posts.y > area.y);
                assert_eq!(
                    rendered(&app).join("\n").matches("Loading...").count(),
                    usize::from(!commits_first)
                );
                if !commits_first {
                    receive_commits(&mut app, "author", &[3, 2], Some("next"), now);
                }
                assert_eq!(app.selection().id(), Some(id(if pinned { 9 } else { 3 })));
                assert!(!rendered(&app).join("\n").contains("Loading..."));
                assert!(rendered(&app).join("\n").contains("body 3"));
            }
        }
    }
}

// Every wrapped header or unavailable line reserves its first column without losing the rightmost character.
#[test]
fn header_indent_is_included_in_wrapping_width() {
    let now = Instant::now();
    let mut app = latest(now);
    app.resize(crate::terminal::TermSize { cols: 24, rows: 50 });
    key(&mut app, Key::Char('p'), now);
    let mut header = profile("author");
    header.user.name = Some("x".repeat(24));
    header.bio = Some("b".repeat(24));
    header.company = Some("c".repeat(24));
    header.location = None;
    header.website = None;
    header.commits_count = None;
    header.followers_count = None;
    header.follows_you = false;
    receive_profile(&mut app, header, now);
    let area = layout::areas(&app, app.size).list;
    let posts = app.post_area(area);
    let text = rendered(&app);
    let actual: Vec<_> = text[usize::from(area.y)..usize::from(posts.y)]
        .iter()
        .map(|line| line.trim_end())
        .collect();
    let expected = [
        format!(" {}", "x".repeat(23)),
        " x @author".into(),
        format!(" {}", "b".repeat(23)),
        " b".into(),
        format!(" {}", "c".repeat(23)),
        " c".into(),
        " joined 2026-10-20".into(),
        String::new(),
    ];
    assert_eq!(actual, expected);
    assert!(text[usize::from(posts.y)].starts_with(" Loading..."));
    app.answer(
        Request::Profile("author".into()),
        Err(ApiError::Status {
            status: reqwest::StatusCode::NOT_FOUND,
            code: None,
            message: "u".repeat(24),
            retry_after: None,
        }
        .into()),
        now,
    );
    assert_eq!(app.post_area(area), area);
    let text = rendered(&app);
    assert_eq!(text[usize::from(area.y)], format!(" {}", "u".repeat(23)));
    assert_eq!(text[usize::from(area.y + 1)].trim_end(), " u");
    assert!(app.list_entries().is_empty());
}
