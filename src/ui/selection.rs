use crate::api::CommitId;

/// The selected commit of a list, kept by id so that it stays on the same
/// commit while the list changes, and by position so that it lands near
/// where it was when that commit goes away.
///
/// The ids are given in display order on every call, so that one selection
/// can run over a list made of several parts, as a thread is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Selection<I = CommitId> {
    id: Option<I>,
    index: usize,
}

impl<I> Default for Selection<I> {
    fn default() -> Self {
        Self { id: None, index: 0 }
    }
}

impl<I: Copy + Eq> Selection<I> {
    pub fn id(&self) -> Option<I> {
        self.id
    }

    /// Selects the first of `ids`, or nothing when there is none.
    pub fn first(&mut self, ids: &[I]) {
        self.id = ids.first().copied();
        self.index = 0;
    }

    /// Selects `id` if it is in `ids`.
    pub fn select(&mut self, ids: &[I], id: I) {
        if let Some(index) = ids.iter().position(|candidate| *candidate == id) {
            self.id = Some(id);
            self.index = index;
        }
    }

    /// Moves to the next commit. Returns `false` when the selection is on the
    /// last one already, which is when the next page should be loaded.
    pub fn next(&mut self, ids: &[I]) -> bool {
        self.reanchor(ids);
        if self.index + 1 < ids.len() {
            self.index += 1;
            self.id = Some(ids[self.index]);
            true
        } else {
            false
        }
    }

    pub fn prev(&mut self, ids: &[I]) {
        self.reanchor(ids);
        self.index = self.index.saturating_sub(1);
        self.id = ids.get(self.index).copied();
    }

    /// After the list changed: stays on its commit if that is still in
    /// `ids`, else takes the one now at its old position or the last one,
    /// and the first one if nothing was selected.
    pub fn reanchor(&mut self, ids: &[I]) {
        self.index = self
            .id
            .and_then(|id| ids.iter().position(|candidate| *candidate == id))
            .unwrap_or_else(|| self.index.min(ids.len().saturating_sub(1)));
        let id = ids.get(self.index).copied();
        self.id = id;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::id;

    // Selection follows IDs through insertion and reordering, and falls back near a removed entry.
    #[test]
    fn selection_reanchors_and_stops_at_edges() {
        let mut selection = Selection::default();
        let ids = [id(1), id(2), id(3)];
        selection.reanchor(&ids);
        assert_eq!(selection.id(), Some(id(1)));
        selection.prev(&ids);
        assert_eq!(selection.id(), Some(id(1)));
        assert!(selection.next(&ids));
        assert_eq!(selection.id(), Some(id(2)));
        selection.reanchor(&[id(4), id(3), id(2), id(1)]);
        assert_eq!(selection.id(), Some(id(2)));
        selection.reanchor(&[id(4), id(3), id(1)]);
        assert_eq!(selection.id(), Some(id(1)));
        assert!(!selection.next(&[id(4), id(3), id(1)]));
        selection.select(&ids, id(999));
        assert_eq!(selection.id(), Some(id(1)));
        selection.reanchor(&[]);
        assert_eq!(selection.id(), None);
        selection.reanchor(&ids);
        assert_eq!(selection.id(), Some(id(1)));
    }
}
