use crate::api::{CommitEntry, CommitId, Cursor, Page};

/// How a list is ordered, which decides how a fresh first page goes in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Order {
    /// Following and Latest: newest first.
    NewestFirst,
    /// Trending: ranked by the site.
    Ranked,
    /// Replies in a thread: oldest first.
    OldestFirst,
}

pub trait Item: Clone {
    type Id: Copy + Eq + std::fmt::Debug;
    fn id(&self) -> Self::Id;
    fn visible(&self) -> bool {
        true
    }
}

impl Item for CommitEntry {
    type Id = CommitId;
    fn id(&self) -> CommitId {
        self.id()
    }
    fn visible(&self) -> bool {
        match self {
            Self::Live(commit) => commit.is_approved(),
            Self::Deleted(_) => true,
        }
    }
}

/// A commit list hides pending and rejected entries until they are approved.
pub type CommitList = List<CommitEntry>;

/// Items loaded page by page, and the cursor of the page after the last
/// one.
/// Visibility is supplied by the item type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct List<T: Item> {
    order: Order,
    entries: Vec<T>,
    next_cursor: Option<Cursor>,
    loaded: bool,
    loading_more: bool,
    first_page: Vec<T::Id>,
    paged: bool,
    newest: Vec<T>,
}

impl<T: Item> List<T> {
    pub fn new(order: Order) -> Self {
        Self {
            order,
            entries: Vec::new(),
            next_cursor: None,
            loaded: false,
            loading_more: false,
            first_page: Vec::new(),
            paged: false,
            newest: Vec::new(),
        }
    }

    /// Puts in a fresh first page, keeping the pages loaded after it.
    ///
    /// - [`Order::NewestFirst`]: commits not there yet come in on top, the
    ///   others are updated in place.
    /// - [`Order::Ranked`]: the first page replaces what was there for it, in
    ///   its order.
    /// - [`Order::OldestFirst`]: commits already there are updated in place.
    ///   New ones are added at the end once every page is loaded, and are
    ///   left for the next pages otherwise.
    pub fn refresh(&mut self, page: Page<T>) {
        let first_page: Vec<_> = page.items.iter().map(Item::id).collect();
        let old_loaded = self.loaded;
        let all_loaded = self.loaded && self.next_cursor.is_none();
        let mut new = Vec::new();
        for entry in page.items {
            if !entry.visible() {
                self.remove(entry.id());
                continue;
            }
            if let Some(existing) = self.entries.iter_mut().find(|e| e.id() == entry.id()) {
                *existing = entry.clone();
            } else {
                new.push(entry.clone());
            }
            if self.order == Order::Ranked {
                new.retain(|e| e.id() != entry.id());
                new.push(entry);
            }
        }
        match self.order {
            Order::NewestFirst => {
                new.append(&mut self.entries);
                self.entries = new;
            }
            Order::Ranked => {
                self.entries.retain(|e| {
                    !self.first_page.contains(&e.id()) && !first_page.contains(&e.id())
                });
                new.append(&mut self.entries);
                self.entries = new;
            }
            Order::OldestFirst if !old_loaded || all_loaded => self.entries.extend(new),
            Order::OldestFirst => {}
        }
        if !self.paged {
            self.next_cursor = page.next_cursor;
        }
        self.first_page = first_page;
        self.loaded = true;
    }

    pub fn exhausted(&self) -> bool {
        self.loaded && self.next_cursor.is_none()
    }

    /// Collects newest-first pages until they overlap the loaded history, then
    /// appends the collected replies oldest first without reopening old pagination.
    pub fn refresh_newest(&mut self, page: Page<T>, first: bool) -> Option<Cursor> {
        if first {
            self.newest.clear();
        }
        let overlap = page
            .items
            .iter()
            .any(|entry| self.entries.iter().any(|old| old.id() == entry.id()));
        self.newest.extend(page.items);
        if !overlap && page.next_cursor.is_some() {
            return page.next_cursor;
        }
        for entry in std::mem::take(&mut self.newest).into_iter().rev() {
            if !entry.visible() {
                self.remove(entry.id());
            } else if let Some(old) = self.entries.iter_mut().find(|old| old.id() == entry.id()) {
                *old = entry;
            } else {
                self.entries.push(entry);
            }
        }
        None
    }

    /// Adds the next page at the end.
    pub fn append(&mut self, page: Page<T>) {
        for entry in page.items {
            if let Some(existing) = self.entries.iter_mut().find(|e| e.id() == entry.id()) {
                *existing = entry;
            } else {
                self.entries.push(entry);
            }
        }
        self.entries.retain(Item::visible);
        self.next_cursor = page.next_cursor;
        self.loaded = true;
        self.paged = true;
        self.more_done();
    }

    /// Replaces a commit, after it was starred or unstarred.
    pub fn update(&mut self, entry: T) {
        if !entry.visible() {
            self.remove(entry.id());
        } else if let Some(existing) = self.entries.iter_mut().find(|e| e.id() == entry.id()) {
            *existing = entry;
        }
    }

    pub fn remove(&mut self, id: T::Id) {
        self.entries.retain(|e| e.id() != id);
    }

    pub fn entries(&self) -> &[T] {
        &self.entries
    }

    /// Whether a first page has arrived.
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// The cursor of the next page, and marks it as loading. `None` while a
    /// page is loading or after the last one.
    pub fn begin_more(&mut self) -> Option<Cursor> {
        if self.loading_more {
            return None;
        }
        let cursor = self.next_cursor.clone()?;
        self.loading_more = true;
        Some(cursor)
    }

    /// The next page arrived or failed.
    pub fn more_done(&mut self) {
        self.loading_more = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{commit, id, page};
    fn ids(list: &CommitList) -> Vec<CommitId> {
        list.entries().iter().map(Item::id).collect()
    }

    // A chronological refresh prepends only new entries and preserves later pages and their cursor.
    #[test]
    fn newest_refresh_preserves_pages_and_updates() {
        let mut list = CommitList::new(Order::NewestFirst);
        list.refresh(page(&[3, 2], Some("second")));
        assert_eq!(list.begin_more().unwrap().as_str(), "second");
        assert_eq!(list.begin_more(), None);
        list.append(page(&[2, 1], Some("third")));
        let mut fresh = page(&[4, 3], Some("new-second"));
        if let CommitEntry::Live(c) = &mut fresh.items[1] {
            c.stars_count = Some(7);
        }
        list.refresh(fresh);
        assert_eq!(ids(&list), vec![id(4), id(3), id(2), id(1)]);
        assert!(matches!(&list.entries()[1], CommitEntry::Live(c) if c.stars_count == Some(7)));
        assert_eq!(list.begin_more().unwrap().as_str(), "third");
        list.more_done();
        assert_eq!(list.begin_more().unwrap().as_str(), "third");
    }

    // Ranked refreshes replace the previous first page in server order while keeping unrelated later pages.
    #[test]
    fn ranked_refresh_replaces_first_page() {
        let mut list = CommitList::new(Order::Ranked);
        list.refresh(page(&[1, 2], Some("next")));
        list.append(page(&[3, 4], None));
        list.refresh(page(&[3, 5], Some("new-next")));
        assert_eq!(ids(&list), vec![id(3), id(5), id(4)]);
        assert_eq!(list.begin_more(), None);
    }

    // Oldest-first lists defer new first-page IDs while later pages remain, avoiding replies out of order.
    #[test]
    fn oldest_refresh_respects_unloaded_pages() {
        let mut list = CommitList::new(Order::OldestFirst);
        list.refresh(page(&[1, 2], Some("next")));
        list.refresh(page(&[1, 2, 3], Some("next")));
        assert_eq!(ids(&list), vec![id(1), id(2)]);
        list.append(page(&[3, 4], None));
        list.refresh(page(&[1, 5], None));
        assert_eq!(ids(&list), vec![id(1), id(2), id(3), id(4), id(5)]);
    }

    // Moderation must not leak pending or rejected bodies through any update path.
    #[test]
    fn filters_moderated_entries_and_deduplicates_pages() {
        let mut list = CommitList::new(Order::NewestFirst);
        list.refresh(page(&[1, 2], None));
        let mut pending = commit(1);
        pending.status = crate::api::CommitStatus::Pending;
        list.update(CommitEntry::Live(Box::new(pending)));
        assert_eq!(ids(&list), vec![id(2)]);
        let mut rejected = commit(3);
        rejected.status = crate::api::CommitStatus::Rejected;
        let mut page = page(&[2, 4], None);
        page.items.push(CommitEntry::Live(Box::new(rejected)));
        list.append(page);
        assert_eq!(ids(&list), vec![id(2), id(4)]);
    }
}
