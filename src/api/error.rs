use std::time::Duration;

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use super::SignError;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// The username is empty, `.` or `..`, which would not name a user in
    /// the request's path.
    #[error("not a username")]
    InvalidUsername,
    /// /dev/null answered with an error. `message` is written for the user.
    #[error("{message}")]
    Status {
        status: StatusCode,
        code: Option<ErrorCode>,
        message: String,
        /// From `Retry-After`, on a `429`.
        retry_after: Option<Duration>,
    },
    #[error("could not reach /dev/null")]
    Transport(#[from] reqwest::Error),
    #[error("unexpected answer from /dev/null")]
    Decode(#[source] serde_json::Error),
    #[error("could not sign the request")]
    Sign(#[from] SignError),
}

impl ApiError {
    pub fn code(&self) -> Option<ErrorCode> {
        match self {
            Self::Status { code, .. } => *code,
            _ => None,
        }
    }

    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::Status { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}

/// The `code` of an API error, for the errors a client has to tell apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ErrorCode {
    InvalidGrant,
    InvalidSignature,
    DeviceNotApproved,
    UserBlocked,
    UserUnavailable,
    OwnCommit,
    CommitUnavailable,
    PasteisExhausted,
    RateLimited,
    GithubSyncFailed,
    /// A code this client does not know yet.
    #[serde(other)]
    Other,
}
