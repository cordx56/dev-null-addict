use std::time::{Duration, Instant};

/// A one-line note of notifications that just arrived, along the bottom of the
/// window until `until`. Unlike a [`Popup`](super::popup::Popup), it takes no
/// input.
#[derive(Debug)]
pub struct Banner {
    pub text: String,
    pub until: Instant,
}

impl Banner {
    pub const TIME: Duration = Duration::from_secs(3);
}
