use std::time::Instant;

use crate::api::{ApiError, CommitEntry, CommitId, CurrentUser, ErrorCode, Profile, Username};

use super::{
    App, Effect, Failure, Request, Response, Tab, ThreadView,
    list::{CommitList, Order},
    selection::Selection,
    viewport::Viewport,
};

/// One user's profile: the header, then the pinned commit and their other
/// commits, newest first. The selection runs over both, as in a
/// [`ThreadView`].
#[derive(Debug)]
pub(super) struct ProfileView {
    pub username: Username,
    /// The header. `None` until it arrives.
    pub profile: Option<Profile>,
    /// Why the profile can't be shown, such as across a block. It is shown
    /// in place of the header and the commits, and nothing more is fetched.
    pub unavailable: Option<String>,
    pinned: Option<CommitEntry>,
    pub commits: CommitList,
    pub selection: Selection,
    pub viewport: Viewport,
}

impl ProfileView {
    pub fn new(username: Username) -> Self {
        Self {
            username,
            profile: None,
            unavailable: None,
            pinned: None,
            commits: CommitList::new(Order::NewestFirst),
            selection: Selection::default(),
            viewport: Viewport::default(),
        }
    }

    pub fn entries(&self) -> impl Iterator<Item = &CommitEntry> {
        self.pinned
            .iter()
            .chain(self.commits.entries().iter().filter(|entry| {
                self.pinned
                    .as_ref()
                    .is_none_or(|pinned| pinned.id() != entry.id())
            }))
            .filter(|_| self.profile.is_some() && self.unavailable.is_none())
    }

    pub fn reanchor(&mut self) {
        self.selection
            .reanchor(&self.entries().map(CommitEntry::id).collect::<Vec<_>>());
    }

    fn set_profile(&mut self, profile: Profile) {
        self.pinned = profile
            .pinned_commit
            .as_ref()
            .filter(|commit| commit.is_approved())
            .map(|commit| CommitEntry::Live(Box::new(commit.clone())));
        self.profile = Some(profile);
        self.reanchor();
    }

    pub fn is_pinned(&self, id: CommitId) -> bool {
        self.pinned.as_ref().is_some_and(|entry| entry.id() == id)
    }

    fn update(&mut self, entry: CommitEntry) {
        if self.is_pinned(entry.id()) {
            let commit = match &entry {
                CommitEntry::Live(commit) if commit.is_approved() => Some(commit.as_ref().clone()),
                _ => None,
            };
            if let Some(profile) = &mut self.profile {
                profile.pinned_commit = commit.clone();
            }
            self.pinned = commit.map(|commit| CommitEntry::Live(Box::new(commit)));
        }
        self.commits.update(entry);
        self.reanchor();
    }

    fn remove(&mut self, id: CommitId) {
        if self.is_pinned(id) {
            self.pinned = None;
            if let Some(profile) = &mut self.profile {
                profile.pinned_commit = None;
            }
        }
        self.commits.remove(id);
        self.reanchor();
    }

    fn requests(&self) -> Vec<Request> {
        if self.unavailable.is_some() {
            return Vec::new();
        }
        vec![
            Request::Profile(self.username.clone()),
            Request::UserCommits {
                username: self.username.clone(),
                cursor: None,
            },
        ]
    }
}

/// The Profile tab.
#[derive(Debug, Default)]
pub(super) struct Profiles {
    /// The user's own, kept like the other tabs. `None` until `me` arrives.
    pub own: Option<ProfileView>,
    /// Someone else's, opened with p, until the tab is left with h or l.
    pub other: Option<ProfileView>,
}

impl Profiles {
    /// `other` if there is one, else `own`.
    pub fn shown(&self) -> Option<&ProfileView> {
        self.other.as_ref().or(self.own.as_ref())
    }

    pub fn shown_mut(&mut self) -> Option<&mut ProfileView> {
        self.other.as_mut().or(self.own.as_mut())
    }
}

/// Where p was pressed, for Esc to go back to. The logs, the notifications
/// and the own profile keep their state anyway, so only what p replaces is
/// kept.
#[derive(Debug)]
pub(super) struct Place {
    pub tab: Tab,
    pub threads: Vec<ThreadView>,
    pub other: Option<ProfileView>,
}

impl App {
    pub(super) fn showing_profile(&self) -> bool {
        self.timeline.active() == Tab::Profile && self.threads.is_empty()
    }

    pub(super) fn profile_target(&self) -> Option<Username> {
        if self.showing_notifications() {
            let tab = &self.notifications.tab;
            return tab
                .list
                .entries()
                .iter()
                .find(|group| Some(group.id) == tab.selection.id())
                .map(|group| group.actor.username.clone());
        }
        match self.selected()? {
            CommitEntry::Live(commit) => Some(commit.author.username),
            CommitEntry::Deleted(_) => None,
        }
    }

    pub(super) fn open_profile(&mut self, now: Instant) -> Vec<Effect> {
        let Some(username) = self.profile_target() else {
            return Vec::new();
        };
        if self.showing_profile()
            && self
                .profiles
                .shown()
                .is_some_and(|view| view.username == username)
        {
            return Vec::new();
        }
        self.back.push(Place {
            tab: self.timeline.active(),
            threads: std::mem::take(&mut self.threads),
            other: self.profiles.other.take(),
        });
        if self.timeline.active() == Tab::Notifications {
            self.notifications.new.clear();
        }
        self.timeline.switch(Tab::Profile);
        if self.me.as_ref() != Some(&username) {
            self.profiles.other = Some(ProfileView::new(username));
        }
        self.refresh(now)
    }

    pub(super) fn follow_target(&self) -> Option<Username> {
        if !self.showing_profile() {
            return None;
        }
        let view = self.profiles.other.as_ref()?;
        (view.unavailable.is_none()
            && self.me.as_ref() != Some(&view.username)
            && view
                .profile
                .as_ref()
                .is_some_and(|profile| !profile.you_follow))
        .then(|| view.username.clone())
    }

    pub(super) fn follow_profile(&mut self) -> Vec<Effect> {
        self.queue(
            self.follow_target()
                .map(Request::Follow)
                .into_iter()
                .collect(),
        )
    }

    pub(super) fn profile_requests(&self) -> Vec<Request> {
        self.profiles
            .shown()
            .map_or_else(Vec::new, ProfileView::requests)
    }

    pub(super) fn receive_me(&mut self, current: CurrentUser) -> Vec<Effect> {
        let changed = self
            .profiles
            .own
            .as_ref()
            .is_none_or(|view| view.username != current.user.username);
        self.me = Some(current.user.username.clone());
        if changed {
            self.profiles.own = Some(ProfileView::new(current.user.username));
        }
        let mut requests =
            if changed && self.timeline.active() == Tab::Profile && self.profiles.other.is_none() {
                self.profile_requests()
            } else {
                Vec::new()
            };
        if current.unread_notifications > 0 && self.timeline.active() != Tab::Notifications {
            requests.push(Request::Notifications { cursor: None });
        }
        self.queue(requests)
    }

    fn profiles_mut(&mut self) -> impl Iterator<Item = &mut ProfileView> {
        self.profiles
            .own
            .iter_mut()
            .chain(self.profiles.other.iter_mut())
            .chain(
                self.back
                    .iter_mut()
                    .filter_map(|place| place.other.as_mut()),
            )
    }

    pub(super) fn kept_threads_mut(&mut self) -> impl Iterator<Item = &mut ThreadView> {
        self.threads.iter_mut().chain(
            self.back
                .iter_mut()
                .flat_map(|place| place.threads.iter_mut()),
        )
    }

    pub(super) fn profile_more_done(&mut self, request: &Request) {
        if let Request::UserCommits {
            username,
            cursor: Some(_),
        } = request
        {
            self.profiles_mut()
                .filter(|view| &view.username == username)
                .for_each(|view| view.commits.more_done());
        }
    }

    pub(super) fn profile_error(&mut self, request: &Request, error: &Failure) -> bool {
        let (Request::Profile(username) | Request::UserCommits { username, .. }) = request else {
            return false;
        };
        let Failure::Api(ApiError::Status {
            status,
            code,
            message,
            ..
        }) = error
        else {
            return false;
        };
        if *status != reqwest::StatusCode::NOT_FOUND
            && !(*status == reqwest::StatusCode::FORBIDDEN
                && matches!(
                    code,
                    Some(ErrorCode::UserBlocked | ErrorCode::UserUnavailable)
                ))
        {
            return false;
        }
        self.profiles_mut()
            .filter(|view| &view.username == username)
            .for_each(|view| {
                view.unavailable = Some(message.clone());
                view.reanchor();
            });
        true
    }

    pub(super) fn profile_answer(
        &mut self,
        request: &Request,
        response: &Response,
    ) -> Option<Vec<Effect>> {
        match (request, response) {
            (Request::Profile(username), Response::Profile(profile)) => {
                self.profiles_mut()
                    .filter(|view| &view.username == username && view.unavailable.is_none())
                    .for_each(|view| view.set_profile(profile.as_ref().clone()));
            }
            (Request::UserCommits { username, cursor }, Response::Entries(page)) => {
                self.profiles_mut()
                    .filter(|view| &view.username == username && view.unavailable.is_none())
                    .for_each(|view| {
                        if cursor.is_some() {
                            view.commits.append(page.clone());
                        } else {
                            view.commits.refresh(page.clone());
                        }
                        view.reanchor();
                    });
            }
            (Request::Follow(username), Response::Done) => {
                let unavailable = self
                    .profiles_mut()
                    .any(|view| &view.username == username && view.unavailable.is_some());
                return Some(self.queue(if unavailable {
                    Vec::new()
                } else {
                    vec![Request::Profile(username.clone())]
                }));
            }
            _ => return None,
        }
        Some(Vec::new())
    }

    pub(super) fn update_profiles(&mut self, entry: &CommitEntry) {
        self.profiles_mut()
            .for_each(|view| view.update(entry.clone()));
    }

    pub(super) fn remove_from_profiles(&mut self, id: CommitId) {
        self.profiles_mut().for_each(|view| view.remove(id));
    }
}
