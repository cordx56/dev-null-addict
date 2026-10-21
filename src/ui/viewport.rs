use std::ops::Range;

use super::selection::Selection;

/// A list's first visible line, independent of its selected commit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Viewport {
    top: usize,
}

impl Viewport {
    pub fn top(&self) -> usize {
        self.top
    }

    pub fn reconcile<I: Copy + Eq>(
        &mut self,
        entries: &[(I, usize)],
        selected: Option<I>,
        height: u16,
    ) {
        if height == 0 {
            return;
        }
        let height = usize::from(height);
        let total: usize = entries.iter().map(|(_, lines)| lines).sum();
        self.top = self.top.min(total.saturating_sub(height));
        if let Some(range) = selected_range(entries, selected) {
            if range.len() > height {
                self.top = self.top.clamp(range.start, range.end - height);
            } else if range.start < self.top {
                self.top = range.start;
            } else if range.end > self.top + height {
                self.top = range.end - height;
            }
        }
    }

    // Returns true when navigation, rather than scrolling within a commit, can request another page.
    pub fn move_line<I: Copy + Eq>(
        &mut self,
        selection: &mut Selection<I>,
        entries: &[(I, usize)],
        height: u16,
        down: bool,
    ) -> bool {
        if height == 0 {
            return false;
        }
        let ids: Vec<_> = entries.iter().map(|(id, _)| *id).collect();
        selection.reanchor(&ids);
        self.reconcile(entries, selection.id(), height);
        let height_usize = usize::from(height);
        if let Some(range) = selected_range(entries, selection.id())
            && range.len() > height_usize
        {
            if down && self.top + height_usize < range.end {
                self.top += 1;
                return false;
            }
            if !down && self.top > range.start {
                self.top -= 1;
                return false;
            }
        }
        let previous = selection.id();
        if down {
            selection.next(&ids);
        } else {
            selection.prev(&ids);
        }
        if previous != selection.id()
            && let Some(range) = selected_range(entries, selection.id())
            && range.len() > height_usize
        {
            self.top = if down {
                range.start
            } else {
                range.end - height_usize
            };
        }
        self.reconcile(entries, selection.id(), height);
        true
    }
}

fn selected_range<I: Copy + Eq>(
    entries: &[(I, usize)],
    selected: Option<I>,
) -> Option<Range<usize>> {
    entries
        .iter()
        .scan(0, |start, (id, lines)| {
            let range = *start..*start + lines;
            *start = range.end;
            Some((*id, range))
        })
        .find_map(|(id, range)| (Some(id) == selected).then_some(range))
}
