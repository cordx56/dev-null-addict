mod profile;

pub use profile::Profile;

use std::fmt;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use url::Url;

use super::{CommitId, Cursor, NotificationId, ShortHash, Username};

/// A /dev/null user, as commits carry their author.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct User {
    #[serde(rename = "handle", alias = "username")]
    pub username: Username,
    pub name: Option<String>,
    #[serde(default, deserialize_with = "lane")]
    pub lane: Option<Lane>,
}

/// The color a user's name is drawn with on /dev/null.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Lane {
    /// Picked by the user, as /dev/null sends it, meant to be `#rrggbb`.
    Picked(String),
    /// One of /dev/null's own lane colors, numbered from 0, for a user who
    /// picked none.
    Automatic(u8),
}

/// Leaves out a lane of a shape /dev/null does not send yet, rather than
/// failing the whole commit.
fn lane<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Lane>, D::Error> {
    Ok(match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::String(lane) => Some(Lane::Picked(lane)),
        serde_json::Value::Number(lane) => lane
            .as_u64()
            .and_then(|lane| u8::try_from(lane).ok())
            .map(Lane::Automatic),
        _ => None,
    })
}

/// Where a commit is in moderation. Only the author sees their own commits
/// that are not approved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommitStatus {
    Pending,
    Approved,
    Rejected,
}

/// A commit, as a signed request gets it.
///
/// The counts and `starred` are only there where the API includes them.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Commit {
    pub id: CommitId,
    pub short_hash: ShortHash,
    pub body: String,
    pub parent_id: Option<CommitId>,
    pub html_url: Url,
    pub created_at: Timestamp,
    pub author: User,
    pub status: CommitStatus,
    #[serde(default)]
    pub stars_count: Option<u64>,
    #[serde(default)]
    pub replies_count: Option<u64>,
    #[serde(default)]
    pub starred: Option<bool>,
    #[serde(default)]
    pub images: Vec<Image>,
    #[serde(default)]
    pub quoted_commit: Option<CommitEntry>,
}

impl Commit {
    /// Whether everyone sees it, not just its author.
    pub fn is_approved(&self) -> bool {
        self.status == CommitStatus::Approved
    }
}

/// A commit in a list or a quote: the commit, or what is left of it once it
/// was deleted.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CommitEntry {
    Live(Box<Commit>),
    Deleted(DeletedCommit),
}

impl CommitEntry {
    pub fn id(&self) -> CommitId {
        match self {
            Self::Live(commit) => commit.id,
            Self::Deleted(commit) => commit.id,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeletedCommit {
    pub id: CommitId,
    pub parent_id: Option<CommitId>,
    pub deleted_at: Timestamp,
}

/// An image attached to a commit.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Image {
    pub url: Url,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// A page of a cursor-paginated list. `next_cursor` is `None` on the last
/// page.
///
/// It reads the API's `{"data": [...], "meta": {"next_cursor": ...}}`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "RawPage<T>", into = "RawPage<T>")]
#[serde(bound(
    serialize = "T: Clone + Serialize",
    deserialize = "T: Deserialize<'de>"
))]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<Cursor>,
}

#[derive(Clone, Serialize, Deserialize)]
struct RawPage<T> {
    data: Vec<T>,
    meta: RawPageMeta,
}

#[derive(Clone, Serialize, Deserialize)]
struct RawPageMeta {
    next_cursor: Option<Cursor>,
}

impl<T> From<RawPage<T>> for Page<T> {
    fn from(page: RawPage<T>) -> Self {
        Self {
            items: page.data,
            next_cursor: page.meta.next_cursor,
        }
    }
}

impl<T> From<Page<T>> for RawPage<T> {
    fn from(page: Page<T>) -> Self {
        Self {
            data: page.items,
            meta: RawPageMeta {
                next_cursor: page.next_cursor,
            },
        }
    }
}

/// What a notification is about, as its `type` says.
///
/// A type /dev/null adds later is [`Other`](Self::Other), so that it does not
/// fail the whole page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationKind {
    Followed,
    Replied,
    Mentioned,
    Quoted,
    Starred,
    Approved,
    Rejected,
    LeveledUp,
    #[serde(other)]
    Other,
}

/// A group of notifications as the web shows it, led by its newest.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NotificationGroup {
    pub id: NotificationId,
    #[serde(rename = "type")]
    pub kind: NotificationKind,
    pub reason: Option<String>,
    pub read_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub actor: User,
    #[serde(default)]
    pub actors: Vec<User>,
    pub actors_count: u32,
    pub event_count: u32,
    pub unread_count: u32,
    pub commit: Option<Commit>,
}

/// A page of `GET /api/v1/notifications`, which marked every notification
/// read at [`read_at`](Self::read_at).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NotificationPage {
    #[serde(flatten)]
    pub page: Page<NotificationGroup>,
    /// When the request marked everything read.
    pub read_at: Timestamp,
}

impl NotificationPage {
    /// The groups that were unread until this request.
    pub fn fresh(&self) -> impl Iterator<Item = &NotificationGroup> {
        self.page
            .items
            .iter()
            .filter(|group| group.read_at.is_some_and(|at| at >= self.read_at))
    }
}

/// The signed-in user, as `GET /api/v1/me` gives it, with the unread count
/// the web shows on Notifications. Reading it marks nothing read.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CurrentUser {
    #[serde(rename = "data")]
    pub user: User,
    #[serde(rename = "unread_notifications_count")]
    pub unread_notifications: u32,
}

/// A commit with the commits it replies to, oldest first, as
/// `GET /api/v1/commits/{id}` gives it to a signed request.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Thread {
    pub ancestors: Vec<CommitEntry>,
    #[serde(rename = "data")]
    pub commit: Commit,
}

/// The body of `POST /api/v1/devices`, which turns a one-time code from the
/// approval page into an approved device.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeviceApproval {
    pub code: String,
    pub code_verifier: String,
}

/// Leaves out the code and the verifier.
impl fmt::Debug for DeviceApproval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceApproval").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{commit_json, id};

    // The server may still send the old key during or after the rollout.
    #[test]
    fn user_deserializes_legacy_username() {
        let user: User =
            serde_json::from_value(serde_json::json!({"username": "someone"})).unwrap();
        assert_eq!(user.username.as_str(), "someone");
    }

    // Signed resources omit optional counts, include extra fields and may contain deleted quoted commits; lanes are picked colors or numbered ones.
    #[test]
    fn upstream_resources_deserialize() {
        let deleted = serde_json::json!({"id": id(2), "parent_id": id(1), "deleted_at": "2026-10-21T00:00:00Z", "replies_count": 3});
        let mut live = commit_json(1);
        live["quoted_commit"] = deleted.clone();
        live.as_object_mut().unwrap().remove("stars_count");
        let value = serde_json::json!({"data": [live.clone(), deleted.clone()], "meta": {"next_cursor": "opaque+/=", "path": "ignored"}, "links": {"next": "ignored"}});
        let page: Page<CommitEntry> = serde_json::from_value(value).unwrap();
        assert_eq!(page.next_cursor.unwrap().as_str(), "opaque+/=");
        assert_eq!(page.items.len(), 2);
        let CommitEntry::Live(commit) = &page.items[0] else {
            panic!("expected live commit")
        };
        assert_eq!(commit.stars_count, None);
        assert!(matches!(
            commit.quoted_commit,
            Some(CommitEntry::Deleted(_))
        ));
        let thread: Thread =
            serde_json::from_value(serde_json::json!({"data": live, "ancestors": [deleted]}))
                .unwrap();
        assert_eq!(thread.commit.id, id(1));
        for (lane, expected) in [
            (
                serde_json::json!("#719681"),
                Some(Lane::Picked("#719681".into())),
            ),
            (serde_json::json!(5), Some(Lane::Automatic(5))),
            (serde_json::json!(-1), None),
            (serde_json::json!(1.5), None),
            (serde_json::Value::Null, None),
        ] {
            let user: User = serde_json::from_value(
                serde_json::json!({"handle": "someone", "name": null, "lane": lane}),
            )
            .unwrap();
            assert_eq!(user.lane, expected);
        }
        assert_eq!(thread.ancestors[0].id(), id(2));
        let round_trip: Thread =
            serde_json::from_value(serde_json::to_value(&thread).unwrap()).unwrap();
        assert_eq!(thread, round_trip);
    }
}

#[cfg(test)]
mod notification_tests;
