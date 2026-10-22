use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use super::{Commit, User};

/// A user's profile, as `GET /api/v1/users/{username}` shows it to the user,
/// with how the two follow each other.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "RawProfile", into = "RawProfile")]
pub struct Profile {
    pub user: User,
    pub bio: Option<String>,
    pub company: Option<String>,
    pub location: Option<String>,
    /// As the user typed it, not always a URL.
    pub website: Option<String>,
    /// When they joined.
    pub created_at: Timestamp,
    pub commits_count: Option<u64>,
    pub followers_count: Option<u64>,
    pub following_count: Option<u64>,
    /// Whether the user follows them.
    pub you_follow: bool,
    pub follows_you: bool,
    /// Shown above their other commits, which
    /// [`Client::user_commits`](crate::api::Client::user_commits) leaves it out of.
    pub pinned_commit: Option<Commit>,
}

#[derive(Serialize, Deserialize)]
struct RawProfile {
    data: RawUser,
    #[serde(default)]
    is_following: bool,
    #[serde(default)]
    follows_you: bool,
    pinned_commit: Option<Commit>,
}

#[derive(Serialize, Deserialize)]
struct RawUser {
    #[serde(flatten)]
    user: User,
    bio: Option<String>,
    company: Option<String>,
    location: Option<String>,
    website: Option<String>,
    created_at: Timestamp,
    commits_count: Option<u64>,
    followers_count: Option<u64>,
    following_count: Option<u64>,
}

impl From<RawProfile> for Profile {
    fn from(raw: RawProfile) -> Self {
        Self {
            user: raw.data.user,
            bio: raw.data.bio,
            company: raw.data.company,
            location: raw.data.location,
            website: raw.data.website,
            created_at: raw.data.created_at,
            commits_count: raw.data.commits_count,
            followers_count: raw.data.followers_count,
            following_count: raw.data.following_count,
            you_follow: raw.is_following,
            follows_you: raw.follows_you,
            pinned_commit: raw.pinned_commit,
        }
    }
}

impl From<Profile> for RawProfile {
    fn from(profile: Profile) -> Self {
        Self {
            data: RawUser {
                user: profile.user,
                bio: profile.bio,
                company: profile.company,
                location: profile.location,
                website: profile.website,
                created_at: profile.created_at,
                commits_count: profile.commits_count,
                followers_count: profile.followers_count,
                following_count: profile.following_count,
            },
            is_following: profile.you_follow,
            follows_you: profile.follows_you,
            pinned_commit: profile.pinned_commit,
        }
    }
}
