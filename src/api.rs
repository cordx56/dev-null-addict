//! The signed /dev/null API under `/api/v1`, as the mobile app uses it.
//!
//! Every request is signed with a [`DeviceKey`] that the user approved once as
//! a device (see [`auth`](crate::auth)).

mod body;
mod client;
mod error;
mod id;
mod model;
mod signing;

pub use body::{CommitBody, CommitBodyError, CommitTarget, NewCommit};
pub use client::{BaseUrl, BaseUrlError, Client};
pub use error::{ApiError, ErrorCode};
pub use id::{CommitId, Cursor, LogKind, LogKindError, ShortHash, Username};
pub use model::{
    Commit, CommitEntry, CommitStatus, DeletedCommit, DeviceApproval, Image, Lane, Page, Thread,
    User,
};
pub use signing::{DeviceKey, Fingerprint, KeyError, SignError, SignedHeaders};
