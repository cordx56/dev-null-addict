//! The overlay: the composer on top, the timeline below, and the keys down the
//! right edge when there is room.
//!
//! [`App`] holds no terminal and does no I/O. It takes input, answers and
//! ticks, and returns [`Effect`]s for the session to carry out, so it can be
//! tested on its own.

mod banner;
mod compose;
mod format;
mod help;
pub(crate) mod layout;
mod list;
mod navigation;
mod notifications;
mod popup;
mod profile;
mod render;
mod selection;
mod tab;
mod theme;
mod thread;
mod timeline;
mod viewport;

use std::time::{Duration, Instant};

use ratatui::Frame;

use crate::api::{
    ApiError, Commit, CommitEntry, CommitId, CurrentUser, Cursor, KeyError, LogKind, NewCommit,
    NotificationPage, Page, Profile, Thread, Username,
};
use crate::input::{ToggleKey, UiInput};

use self::banner::Banner;
use self::compose::Compose;
use self::notifications::Notifications;
use self::popup::{Notice, Popup};
use self::profile::{Place, Profiles};
pub use self::tab::Tab;
use self::thread::ThreadView;
use self::timeline::Timeline;

/// State of the overlay.
///
/// It lives as long as the wrapper, so the draft, the selection and open
/// threads stay when the overlay closes and opens again.
#[derive(Debug)]
pub struct App {
    toggle: Option<ToggleKey>,
    refresh_interval: Duration,
    focus: Focus,
    compose: Compose,
    timeline: Timeline,
    notifications: Notifications,
    profiles: Profiles,
    back: Vec<Place>,
    banner: Option<Banner>,
    /// Threads opened with Enter, innermost last. Esc closes the last one.
    threads: Vec<ThreadView>,
    popup: Option<Popup>,
    moderation_notices: std::collections::VecDeque<String>,
    notice: Option<Notice>,
    me: Option<Username>,
    /// Commits the user sent that are still pending review.
    pending: Vec<CommitId>,
    sending: bool,
    next_refresh: Option<Instant>,
    /// No requests but those the user asks for before this, after a `429`.
    backoff_until: Option<Instant>,
    opened: bool,
    inflight: Vec<Request>,
    size: ratatui::layout::Rect,
    layout: layout::OverlayLayout,
    background: Vec<String>,
}

/// Where keys go.
///
/// Tab moves Compose, Send, List and round again. Send is skipped while the
/// draft is not a commit /dev/null would accept.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Focus {
    #[default]
    Compose,
    Send,
    /// The timeline, or the innermost open thread.
    List,
}

/// Something for the session to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    Close,
    Send(Request),
}

/// A request to /dev/null. Its answer goes to [`App::answer`] together with
/// the request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Me,
    Profile(Username),
    UserCommits {
        username: Username,
        cursor: Option<Cursor>,
    },
    Follow(Username),
    Log {
        kind: LogKind,
        cursor: Option<Cursor>,
    },
    Notifications {
        cursor: Option<Cursor>,
    },
    Thread(CommitId),
    Replies {
        id: CommitId,
        cursor: Option<Cursor>,
    },
    NewestReplies {
        id: CommitId,
        cursor: Option<Cursor>,
    },
    Create(NewCommit),
    Delete(CommitId),
    Star {
        id: CommitId,
        starred: bool,
    },
    /// Looks at a commit the user sent, to see whether it was reviewed.
    Moderation(CommitId),
}

/// The answer to a [`Request`], by what it carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Response {
    CurrentUser(CurrentUser),
    Profile(Box<Profile>),
    Notifications(NotificationPage),
    Entries(Page<CommitEntry>),
    Thread(Thread),
    Commit(Commit),
    Done,
}

/// Why a request got no answer.
#[derive(Debug, thiserror::Error)]
pub enum Failure {
    /// There is no device key yet ([`KeyError::Missing`]).
    #[error("not signed in")]
    SignedOut,
    /// The device key is there but can't be used.
    #[error(transparent)]
    Key(KeyError),
    #[error(transparent)]
    Api(#[from] ApiError),
}

/// [`KeyError::Missing`] is [`Failure::SignedOut`].
impl From<KeyError> for Failure {
    fn from(error: KeyError) -> Self {
        match error {
            KeyError::Missing(_) => Self::SignedOut,
            other => Self::Key(other),
        }
    }
}

impl App {
    pub fn new(toggle: Option<ToggleKey>, log: LogKind, refresh_interval: Duration) -> Self {
        Self {
            toggle,
            refresh_interval,
            focus: Focus::Compose,
            compose: Compose::new(),
            timeline: Timeline::new(log),
            notifications: Notifications::new(),
            profiles: Profiles::default(),
            back: Vec::new(),
            banner: None,
            threads: Vec::new(),
            popup: None,
            moderation_notices: std::collections::VecDeque::new(),
            notice: None,
            me: None,
            pending: Vec::new(),
            sending: false,
            next_refresh: None,
            backoff_until: None,
            opened: false,
            inflight: Vec::new(),
            size: ratatui::layout::Rect::new(0, 0, 80, 24),
            layout: layout::OverlayLayout::default(),
            background: Vec::new(),
        }
    }

    /// The overlay opens: focus goes to the composer, and what is shown is
    /// refreshed.
    pub fn open(&mut self, now: Instant) -> Vec<Effect> {
        self.sync_viewport();
        self.opened = true;
        self.focus = Focus::Compose;
        self.next_refresh = Some(now);
        self.tick(now)
    }

    /// Handles a key, a paste or the toggle key.
    ///
    /// Esc goes back one step: it closes a popup, else the innermost thread,
    /// then a profile opened with p, else the overlay. The toggle key always
    /// closes the overlay.
    pub fn input(&mut self, input: UiInput, now: Instant) -> Vec<Effect> {
        let effects = self.handle_input(input, now);
        if effects.contains(&Effect::Close) && self.timeline.active() == Tab::Notifications {
            self.notifications.new.clear();
        }
        self.show_moderation_notice(now);
        self.sync_viewport();
        effects
    }

    fn handle_input(&mut self, input: UiInput, now: Instant) -> Vec<Effect> {
        use crate::input::Key;
        if input == UiInput::Toggle {
            self.opened = false;
            self.next_refresh = None;
            return vec![Effect::Close];
        }
        if input == UiInput::Key(Key::Esc) {
            if self.popup.take().is_some() {
                return Vec::new();
            }
            if self.threads.pop().is_some() {
                return Vec::new();
            }
            if let Some(place) = self.back.pop() {
                self.timeline.switch(place.tab);
                self.threads = place.threads;
                self.profiles.other = place.other;
                return self.refresh(now);
            }
            self.opened = false;
            self.next_refresh = None;
            return vec![Effect::Close];
        }
        if let Some(Popup::ConfirmDelete { id, .. }) = self.popup.as_ref() {
            if input == UiInput::Key(Key::Enter) {
                let id = *id;
                self.popup = None;
                return self.queue(vec![Request::Delete(id)]);
            }
            return Vec::new();
        }
        if input == UiInput::Key(Key::Tab) {
            self.focus = match self.focus {
                Focus::Compose if self.compose.to_new_commit().is_some() && !self.sending => {
                    Focus::Send
                }
                Focus::Compose | Focus::Send => Focus::List,
                Focus::List => Focus::Compose,
            };
            return Vec::new();
        }
        match (self.focus, input) {
            (Focus::Compose, UiInput::Key(key)) if !self.sending => self.compose.key(key),
            (Focus::Compose, UiInput::Paste(text)) if !self.sending => self.compose.paste(&text),
            (Focus::Send, UiInput::Key(Key::Enter)) if !self.sending => {
                if let Some(commit) = self.compose.to_new_commit() {
                    self.sending = true;
                    return self.queue(vec![Request::Create(commit)]);
                }
            }
            (Focus::List, UiInput::Key(key)) => return self.list_input(key, now),
            _ => {}
        }
        Vec::new()
    }

    pub fn answer(
        &mut self,
        request: Request,
        result: Result<Response, Failure>,
        now: Instant,
    ) -> Vec<Effect> {
        let effects = self.handle_answer(request, result, now);
        self.show_moderation_notice(now);
        self.sync_viewport();
        effects
    }

    fn handle_answer(
        &mut self,
        request: Request,
        result: Result<Response, Failure>,
        now: Instant,
    ) -> Vec<Effect> {
        self.inflight.retain(|r| r != &request);
        if matches!(request, Request::Create(_)) {
            self.sending = false;
        }
        if let Request::Log {
            kind,
            cursor: Some(_),
        } = &request
        {
            self.timeline.tab_mut(*kind).list.more_done();
        }
        if matches!(&request, Request::Notifications { cursor: Some(_) }) {
            self.notifications.tab.list.more_done();
        }
        if let Request::Replies {
            id,
            cursor: Some(_),
        } = &request
        {
            for view in self.kept_threads_mut().filter(|v| v.id() == *id) {
                view.replies_mut().more_done();
            }
        }
        self.profile_more_done(&request);
        let response = match result {
            Ok(response) => {
                self.notice = None;
                response
            }
            Err(error) => {
                if self.profile_error(&request, &error) {
                    return Vec::new();
                }
                match &error {
                    Failure::SignedOut => self.notice = Some(Notice::SignedOut),
                    Failure::Api(error)
                        if error.code() == Some(crate::api::ErrorCode::DeviceNotApproved) =>
                    {
                        self.notice = Some(Notice::NotApproved)
                    }
                    _ => self.popup = Some(Popup::toast(error.to_string(), now)),
                }
                if let Failure::Api(error) = &error {
                    if let Some(delay) = error.retry_after() {
                        self.backoff_until = now.checked_add(delay);
                        self.next_refresh = self.backoff_until;
                    } else if matches!(error, ApiError::Status {status, ..} if *status == reqwest::StatusCode::TOO_MANY_REQUESTS)
                    {
                        self.backoff_until = Some(now + Duration::from_secs(60));
                        self.next_refresh = self.backoff_until;
                    }
                    if (error.code() == Some(crate::api::ErrorCode::CommitUnavailable)
                        || matches!(error, ApiError::Status { status, .. } if *status == reqwest::StatusCode::NOT_FOUND))
                        && let Request::Moderation(id) = request
                    {
                        self.pending.retain(|p| *p != id);
                    }
                }
                return Vec::new();
            }
        };
        if let Some(effects) = self.profile_answer(&request, &response) {
            return effects;
        }
        match (request, response) {
            (Request::Me, Response::CurrentUser(current)) => return self.receive_me(current),
            (Request::Notifications { cursor }, Response::Notifications(page)) => {
                if let Some(text) = self.notifications.receive(page, cursor.is_some()) {
                    self.banner = Some(Banner {
                        text,
                        until: now + Banner::TIME,
                    });
                }
            }
            (Request::Log { kind, cursor }, Response::Entries(page)) => {
                let tab = self.timeline.tab_mut(kind);
                if cursor.is_some() {
                    tab.list.append(page);
                } else {
                    tab.list.refresh(page);
                }
                tab.selection.reanchor(
                    &tab.list
                        .entries()
                        .iter()
                        .map(CommitEntry::id)
                        .collect::<Vec<_>>(),
                );
            }
            (Request::Thread(id), Response::Thread(thread)) => {
                for view in self.kept_threads_mut().filter(|v| v.id() == id) {
                    view.set_thread(thread.clone());
                }
            }
            (Request::Replies { id, cursor }, Response::Entries(page)) => {
                for view in self.kept_threads_mut().filter(|v| v.id() == id) {
                    if cursor.is_some() {
                        view.replies_mut().append(page.clone());
                        view.reanchor();
                    } else {
                        view.refresh_replies(page.clone());
                    }
                }
            }
            (Request::NewestReplies { id, cursor }, Response::Entries(page)) => {
                let mut next = None;
                for view in self.kept_threads_mut().filter(|v| v.id() == id) {
                    next = view
                        .replies_mut()
                        .refresh_newest(page.clone(), cursor.is_none());
                    view.reanchor();
                }
                if let Some(cursor) = next {
                    return self.queue(vec![Request::NewestReplies {
                        id,
                        cursor: Some(cursor),
                    }]);
                }
            }
            (Request::Create(_), Response::Commit(commit)) => {
                self.compose.clear();
                self.focus = Focus::Compose;
                if commit.status == crate::api::CommitStatus::Pending {
                    self.pending.push(commit.id);
                } else if commit.status == crate::api::CommitStatus::Rejected {
                    self.moderation_notices
                        .push_back("Your commit was rejected.".into());
                } else {
                    return self.refresh(now);
                }
            }
            (Request::Moderation(id), Response::Thread(thread)) => match thread.commit.status {
                crate::api::CommitStatus::Pending => {}
                crate::api::CommitStatus::Approved => {
                    self.pending.retain(|p| *p != id);
                    return self.refresh(now);
                }
                crate::api::CommitStatus::Rejected => {
                    self.pending.retain(|p| *p != id);
                }
            },
            (Request::Star { .. }, Response::Commit(commit)) => {
                self.update_commit(CommitEntry::Live(Box::new(commit)))
            }
            (Request::Delete(id), Response::Done) => {
                for kind in LogKind::ALL {
                    let tab = self.timeline.tab_mut(kind);
                    tab.list.remove(id);
                    tab.selection.reanchor(
                        &tab.list
                            .entries()
                            .iter()
                            .map(CommitEntry::id)
                            .collect::<Vec<_>>(),
                    );
                }
                for view in self.kept_threads_mut() {
                    view.remove(id);
                }
                self.remove_from_profiles(id);
                self.notifications.change_commit(id, None);
                self.pending.retain(|p| *p != id);
            }
            _ => {}
        }
        Vec::new()
    }

    /// Refreshes what is shown and checks pending commits when the interval
    /// is up, and drops popups whose time is over.
    pub fn tick(&mut self, now: Instant) -> Vec<Effect> {
        if self
            .banner
            .as_ref()
            .is_some_and(|banner| now >= banner.until)
        {
            self.banner = None;
        }
        if matches!(self.popup, Some(Popup::Toast {until, ..}) if now >= until) {
            self.popup = None;
        }
        if !self.opened {
            return Vec::new();
        }
        self.show_moderation_notice(now);
        if self.backoff_until.is_some_and(|t| t > now) {
            self.next_refresh = self.backoff_until;
            return Vec::new();
        }
        if self.next_refresh.is_some_and(|t| now >= t) {
            return self.refresh(now);
        }
        Vec::new()
    }

    /// When [`tick`](Self::tick) next has something to do.
    pub fn deadline(&self) -> Option<Instant> {
        let popup = match self.popup {
            Some(Popup::Toast { until, .. }) => Some(until),
            _ => None,
        };
        self.next_refresh
            .into_iter()
            .chain(popup)
            .chain(self.banner.as_ref().map(|banner| banner.until))
            .min()
    }

    pub fn render(&self, frame: &mut Frame<'_>, now: jiff::Timestamp) {
        render::draw(self, frame, now);
    }
}

impl App {
    fn show_moderation_notice(&mut self, now: Instant) {
        if self.opened
            && self.popup.is_none()
            && let Some(message) = self.moderation_notices.pop_front()
        {
            self.popup = Some(Popup::toast(message, now));
        }
    }

    fn queue(&mut self, requests: Vec<Request>) -> Vec<Effect> {
        if !self.opened {
            return Vec::new();
        }
        requests
            .into_iter()
            .filter_map(|request| {
                if self.inflight.iter().any(|inflight| inflight == &request || matches!((inflight, &request),
                    (Request::NewestReplies { id: active, .. }, Request::NewestReplies { id, .. }) if active == id)) {
                    None
                } else {
                    self.inflight.push(request.clone());
                    Some(Effect::Send(request))
                }
            })
            .collect()
    }

    fn refresh(&mut self, now: Instant) -> Vec<Effect> {
        self.next_refresh = self
            .opened
            .then(|| now.checked_add(self.refresh_interval))
            .flatten();
        if self.backoff_until.is_some_and(|t| t > now) {
            return Vec::new();
        }
        let mut requests = vec![Request::Me];
        match self.timeline.active() {
            Tab::Log(kind) => requests.push(Request::Log { kind, cursor: None }),
            Tab::Notifications => requests.push(Request::Notifications { cursor: None }),
            Tab::Profile => requests.extend(self.profile_requests()),
        }
        if let Some(view) = self.threads.last() {
            requests.push(Request::Thread(view.id()));
            requests.push(if view.replies().exhausted() {
                Request::NewestReplies {
                    id: view.id(),
                    cursor: None,
                }
            } else {
                Request::Replies {
                    id: view.id(),
                    cursor: None,
                }
            });
        }
        requests.extend(self.pending.iter().copied().map(Request::Moderation));
        self.queue(requests)
    }

    fn selected(&self) -> Option<CommitEntry> {
        if self.showing_profile() && self.profiles.shown().is_none() {
            return None;
        }
        if self.showing_notifications() {
            let tab = &self.notifications.tab;
            tab.list
                .entries()
                .iter()
                .find(|group| Some(group.id) == tab.selection.id())
                .and_then(|group| group.commit.as_ref())
                .map(|commit| CommitEntry::Live(Box::new(commit.clone())))
        } else {
            let selected = self.selection().id();
            self.list_entries()
                .into_iter()
                .find(|entry| Some(entry.id()) == selected)
                .cloned()
        }
    }

    fn update_commit(&mut self, entry: CommitEntry) {
        self.update_profiles(&entry);
        self.notifications.change_commit(
            entry.id(),
            match &entry {
                CommitEntry::Live(commit) => Some(commit.as_ref()),
                CommitEntry::Deleted(_) => None,
            },
        );
        for kind in LogKind::ALL {
            self.timeline.tab_mut(kind).list.update(entry.clone());
        }
        for view in self.kept_threads_mut() {
            view.update(entry.clone());
        }
    }

    fn list_input(&mut self, key: crate::input::Key, now: Instant) -> Vec<Effect> {
        use crate::input::Key;
        let down = matches!(key, Key::Down | Key::Char('j'));
        let up = matches!(key, Key::Up | Key::Char('k'));
        if down || up {
            return self.move_selection(down);
        }
        if self.threads.is_empty() {
            let kind = match key {
                Key::Left | Key::Char('h') => self.timeline.active().prev(),
                Key::Right | Key::Char('l') => self.timeline.active().next(),
                _ => None,
            };
            if let Some(kind) = kind {
                if self.timeline.active() == Tab::Notifications {
                    self.notifications.new.clear();
                }
                self.back.clear();
                self.profiles.other = None;
                if self.timeline.switch(kind) {
                    return self.refresh(now);
                }
            }
        }
        if key == Key::Char('p') {
            return self.open_profile(now);
        }
        if key == Key::Char('f') {
            return self.follow_profile();
        }
        let Some(entry) = self.selected() else {
            return Vec::new();
        };
        if key == Key::Enter {
            let id = entry.id();
            self.threads.push(ThreadView::new(id));
            return self.queue(vec![
                Request::Thread(id),
                Request::Replies { id, cursor: None },
            ]);
        }
        let CommitEntry::Live(commit) = entry else {
            return Vec::new();
        };
        match key {
            Key::Char('r' | 'q') => {
                self.compose.set_target(compose::Target {
                    target: if key == Key::Char('r') {
                        crate::api::CommitTarget::Reply(commit.id)
                    } else {
                        crate::api::CommitTarget::Quote(commit.id)
                    },
                    short_hash: commit.short_hash,
                    author: commit.author.username,
                });
                self.focus = Focus::Compose;
            }
            Key::Char('s') => {
                return self.queue(vec![Request::Star {
                    id: commit.id,
                    starred: !commit.starred.unwrap_or(false),
                }]);
            }
            Key::Char('d') if self.me.as_ref() == Some(&commit.author.username) => {
                self.popup = Some(Popup::ConfirmDelete {
                    id: commit.id,
                    short_hash: commit.short_hash,
                })
            }
            _ => {}
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests;
