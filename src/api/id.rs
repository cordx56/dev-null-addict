use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A commit's id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CommitId(Uuid);

impl From<Uuid> for CommitId {
    fn from(id: Uuid) -> Self {
        Self(id)
    }
}

impl From<CommitId> for Uuid {
    fn from(id: CommitId) -> Self {
        id.0
    }
}

/// Parses a UUID, hyphenated or not.
impl FromStr for CommitId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s).map(Self)
    }
}

/// Writes the hyphenated UUID, as the API does.
impl fmt::Display for CommitId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A notification group's leading event id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NotificationId(Uuid);

impl From<Uuid> for NotificationId {
    fn from(id: Uuid) -> Self {
        Self(id)
    }
}

/// The last seven hex digits of a commit's id, as /dev/null shows it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ShortHash(String);

impl ShortHash {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<CommitId> for ShortHash {
    fn from(id: CommitId) -> Self {
        let hex = id.0.simple().to_string();
        Self(hex[hex.len() - 7..].to_owned())
    }
}

impl fmt::Display for ShortHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A /dev/null username, the same as on GitHub.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Username(String);

impl Username {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for Username {
    fn from(name: String) -> Self {
        Self(name)
    }
}

impl From<&str> for Username {
    fn from(name: &str) -> Self {
        Self(name.to_owned())
    }
}

impl fmt::Display for Username {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where the next page of a list starts, as the API hands it out.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Cursor(String);

impl Cursor {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for Cursor {
    fn from(cursor: String) -> Self {
        Self(cursor)
    }
}

/// One of the three logs of /dev/null.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogKind {
    #[default]
    Following,
    Latest,
    Trending,
}

impl LogKind {
    /// In the order of the tabs.
    pub const ALL: [Self; 3] = [Self::Following, Self::Latest, Self::Trending];

    /// The name in the API path, also accepted by [`FromStr`].
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Following => "following",
            Self::Latest => "latest",
            Self::Trending => "trending",
        }
    }

    /// The tab to the left, if any.
    pub fn prev(self) -> Option<Self> {
        match self {
            Self::Following => None,
            Self::Latest => Some(Self::Following),
            Self::Trending => Some(Self::Latest),
        }
    }

    /// The tab to the right, if any.
    pub fn next(self) -> Option<Self> {
        match self {
            Self::Following => Some(Self::Latest),
            Self::Latest => Some(Self::Trending),
            Self::Trending => None,
        }
    }
}

impl fmt::Display for LogKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for LogKind {
    type Err = LogKindError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str() == s)
            .ok_or_else(|| LogKindError(s.to_owned()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("expected following, latest or trending, not {0:?}")]
pub struct LogKindError(String);
