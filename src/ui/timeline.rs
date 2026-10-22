use crate::api::LogKind;

use super::Tab;
use super::list::{Item, List, Order};
use super::selection::Selection;
use crate::api::CommitEntry;

/// The logs and the active tab. h and l switch tabs and stop at either end.
#[derive(Debug)]
pub struct Timeline {
    active: Tab,
    following: ListTab,
    latest: ListTab,
    trending: ListTab,
}

/// A paginated list with its own selection and viewport.
#[derive(Debug)]
pub struct ListTab<T: Item = CommitEntry> {
    pub list: List<T>,
    pub selection: Selection<T::Id>,
    pub(super) viewport: super::viewport::Viewport,
}

impl Timeline {
    pub fn new(active: LogKind) -> Self {
        Self {
            active: Tab::Log(active),
            following: ListTab::new(Order::NewestFirst),
            latest: ListTab::new(Order::NewestFirst),
            trending: ListTab::new(Order::Ranked),
        }
    }

    pub fn active_log(&self) -> Option<LogKind> {
        match self.active {
            Tab::Log(kind) => Some(kind),
            Tab::Notifications | Tab::Profile => None,
        }
    }

    pub fn active(&self) -> Tab {
        self.active
    }

    /// Switches to `kind`. Returns whether the tab changed.
    pub fn switch(&mut self, kind: Tab) -> bool {
        let changed = self.active != kind;
        self.active = kind;
        changed
    }

    pub fn tab(&self, kind: LogKind) -> &ListTab {
        match kind {
            LogKind::Following => &self.following,
            LogKind::Latest => &self.latest,
            LogKind::Trending => &self.trending,
        }
    }

    pub fn tab_mut(&mut self, kind: LogKind) -> &mut ListTab {
        match kind {
            LogKind::Following => &mut self.following,
            LogKind::Latest => &mut self.latest,
            LogKind::Trending => &mut self.trending,
        }
    }
}

impl<T: Item> ListTab<T> {
    pub fn new(order: Order) -> Self {
        Self {
            list: List::new(order),
            selection: Selection::default(),
            viewport: super::viewport::Viewport::default(),
        }
    }

    pub fn reanchor(&mut self) {
        self.selection
            .reanchor(&self.list.entries().iter().map(Item::id).collect::<Vec<_>>());
    }

    pub fn navigate(
        &mut self,
        entries: &[(T::Id, usize)],
        height: u16,
        down: bool,
    ) -> Option<crate::api::Cursor> {
        let navigate = self
            .viewport
            .move_line(&mut self.selection, entries, height, down);
        if navigate && down && self.selection.id() == entries.last().map(|(id, _)| *id) {
            self.list.begin_more()
        } else {
            None
        }
    }
}
