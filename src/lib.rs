//! A client for [/dev/null](https://dev-null.sh) that runs your shell on a PTY
//! and opens the timeline over it when a key is pressed.
//!
//! [`run`] is the wrapper itself. [`api`] and [`auth`] work on their own.

pub mod api;
pub mod auth;
pub mod init;
mod input;
mod pty;
mod screen;
mod session;
pub mod shell;
mod terminal;
mod ui;

pub use input::{ToggleKey, ToggleKeyError};
pub use session::{
    ClientOptions, RefreshInterval, RefreshIntervalError, SessionError, SessionOptions,
    StandaloneExit, run, run_standalone,
};
pub use ui::layout::OverlayLayout;

/// Set to a non-empty value, it keeps the wrapper from starting.
///
/// The wrapper sets it for the shell it runs, so a startup file that checks it
/// before `exec`ing the wrapper skips that line inside the wrapper.
pub const DISABLE_ENV: &str = "NO_DEV_NULL_ADDICT";

#[cfg(test)]
mod test_support;
