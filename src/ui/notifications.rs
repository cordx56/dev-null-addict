use std::collections::HashSet;

use crate::api::{NotificationGroup, NotificationId, NotificationKind, NotificationPage};

use super::{
    list::{Item, Order},
    timeline::ListTab,
};

impl Item for NotificationGroup {
    type Id = NotificationId;
    fn id(&self) -> NotificationId {
        self.id
    }
}

/// The Notifications tab.
///
/// Listing notifications marks them all read on /dev/null, so which groups
/// arrived unread is kept here: `new` until the tab has been shown and left.
#[derive(Debug)]
pub struct Notifications {
    pub tab: ListTab<NotificationGroup>,
    pub new: HashSet<NotificationId>,
    previous_fresh: HashSet<NotificationId>,
}

impl Notifications {
    pub fn new() -> Self {
        Self {
            tab: ListTab::new(Order::NewestFirst),
            new: HashSet::new(),
            previous_fresh: HashSet::new(),
        }
    }

    /// Puts `commit` into the groups that carry the commit `id`, after it was
    /// starred or deleted elsewhere. `None` drops it from them.
    pub fn change_commit(&mut self, id: crate::api::CommitId, commit: Option<&crate::api::Commit>) {
        let changed: Vec<_> = self
            .tab
            .list
            .entries()
            .iter()
            .filter(|group| group.commit.as_ref().is_some_and(|old| old.id == id))
            .map(|group| {
                let mut group = group.clone();
                group.commit = commit.cloned();
                group
            })
            .collect();
        for group in changed {
            self.tab.list.update(group);
        }
    }

    /// Takes in a page, appended for a next page or else replacing the list,
    /// and returns the banner text for the groups it newly marked read, if
    /// any. Groups the previous page already announced are not announced
    /// again, since two requests within one second share a `read_at`.
    pub fn receive(&mut self, page: NotificationPage, append: bool) -> Option<String> {
        let fresh: Vec<_> = page
            .fresh()
            .filter(|group| !self.previous_fresh.contains(&group.id))
            .collect();
        self.previous_fresh = page.fresh().map(|group| group.id).collect();
        self.new.extend(fresh.iter().map(|group| group.id));
        let banner = fresh
            .iter()
            .max_by_key(|group| (group.created_at, group.id))
            .map(|group| {
                let mut text = format!(
                    "{} {}",
                    super::format::relative_time(group.created_at, page.read_at),
                    wording(group)
                );
                if let Some(actors) = actors(group) {
                    text = format!("{actors} {text}");
                }
                if let Some(commit) = &group.commit {
                    text.push_str(": ");
                    text.push_str(commit.body.lines().next().unwrap_or_default());
                }
                if group.kind == NotificationKind::Rejected
                    && let Some(reason) = &group.reason
                {
                    text.push_str(&format!(" (reason: {reason})"));
                }
                // Reasons and names must not turn a banner into multiple lines.
                text = text.split_whitespace().collect::<Vec<_>>().join(" ");
                if fresh.len() > 1 {
                    text.push_str(&format!(" (+{} more)", fresh.len() - 1));
                }
                text
            });
        if append {
            self.tab.list.append(page.page);
        } else {
            self.tab.list.refresh(page.page);
        }
        self.tab.reanchor();
        banner
    }
}

/// Who the group is from, such as `@alice and 2 others`. `None` for what
/// /dev/null itself tells the user: moderation and level-ups.
pub fn actors(group: &NotificationGroup) -> Option<String> {
    if matches!(
        group.kind,
        NotificationKind::Approved | NotificationKind::Rejected | NotificationKind::LeveledUp
    ) {
        return None;
    }
    let mut text = format!("@{}", group.actor.username);
    if group.actors_count > 1 {
        text.push_str(&format!(" and {} others", group.actors_count - 1));
    }
    Some(text)
}

/// What happened, worded as on the web.
pub fn wording(group: &NotificationGroup) -> &'static str {
    match group.kind {
        NotificationKind::Followed => "followed you",
        NotificationKind::Replied => "replied to your commit",
        NotificationKind::Quoted => "quoted your commit",
        NotificationKind::Starred => "starred your commit",
        NotificationKind::Mentioned => "mentioned you",
        NotificationKind::Approved => "Your commit was approved",
        NotificationKind::Rejected => "Your commit was rejected",
        NotificationKind::LeveledUp => "You reached a new level",
        NotificationKind::Other => "sent you a notification",
    }
}
