use crate::api::LogKind;

/// A tab under the composer: the three logs, the notifications, then the
/// profile. h and l move between them in [`ALL`](Self::ALL)'s order and stop at either end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Log(LogKind),
    Notifications,
    Profile,
}

impl Tab {
    pub const ALL: [Self; 5] = [
        Self::Log(LogKind::Following),
        Self::Log(LogKind::Latest),
        Self::Log(LogKind::Trending),
        Self::Notifications,
        Self::Profile,
    ];

    pub fn prev(self) -> Option<Self> {
        match self {
            Self::Log(kind) => kind.prev().map(Self::Log),
            Self::Notifications => Some(Self::Log(LogKind::Trending)),
            Self::Profile => Some(Self::Notifications),
        }
    }

    pub fn next(self) -> Option<Self> {
        match self {
            Self::Log(LogKind::Trending) => Some(Self::Notifications),
            Self::Log(kind) => kind.next().map(Self::Log),
            Self::Notifications => Some(Self::Profile),
            Self::Profile => None,
        }
    }
}
