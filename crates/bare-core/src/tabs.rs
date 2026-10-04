//! Tab order and which tab to switch to. No UI here: the browser keeps the web views, this keeps
//! the arithmetic honest. Tabs are in opening order (a new tab opens right after the one that
//! opened it); closing the active tab goes back to the most recently used one.

pub type TabId = u64;

#[derive(Debug, Default, Clone)]
pub struct TabOrder {
    order: Vec<TabId>,
    /// Most recently used first; the active tab is `mru[0]`.
    mru: Vec<TabId>,
}

impl TabOrder {
    /// Add a tab right after `after` (or at the end). It does not become active by itself.
    pub fn open(&mut self, id: TabId, after: Option<TabId>) {
        let at = after
            .and_then(|a| self.order.iter().position(|&t| t == a))
            .map_or(self.order.len(), |i| i + 1);
        self.order.insert(at, id);
        self.mru.push(id);
    }

    pub fn activate(&mut self, id: TabId) {
        if let Some(i) = self.mru.iter().position(|&t| t == id) {
            self.mru.remove(i);
            self.mru.insert(0, id);
        }
    }

    pub fn active(&self) -> Option<TabId> {
        self.mru.first().copied()
    }

    /// Remove a tab. If it was the active one, returns the tab that becomes active.
    pub fn close(&mut self, id: TabId) -> Option<TabId> {
        let was_active = self.active() == Some(id);
        self.order.retain(|&t| t != id);
        self.mru.retain(|&t| t != id);
        if was_active { self.active() } else { None }
    }

    /// The next tab in opening order, wrapping around.
    pub fn next(&self, from: TabId) -> Option<TabId> {
        let i = self.order.iter().position(|&t| t == from)?;
        self.order.get((i + 1) % self.order.len()).copied()
    }

    pub fn prev(&self, from: TabId) -> Option<TabId> {
        let i = self.order.iter().position(|&t| t == from)?;
        self.order
            .get((i + self.order.len() - 1) % self.order.len())
            .copied()
    }

    /// `n` is 0-based; `Alt+1` is `nth(0)`.
    pub fn nth(&self, n: usize) -> Option<TabId> {
        self.order.get(n).copied()
    }

    pub fn last(&self) -> Option<TabId> {
        self.order.last().copied()
    }

    /// In opening order.
    pub fn ids(&self) -> &[TabId] {
        &self.order
    }

    /// Most recently used first.
    pub fn mru(&self) -> &[TabId] {
        &self.mru
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs(ids: &[TabId]) -> TabOrder {
        let mut t = TabOrder::default();
        for &i in ids {
            t.open(i, None);
            t.activate(i);
        }
        t
    }

    #[test]
    fn new_tabs_open_after_their_opener() {
        let mut t = tabs(&[1, 2, 3]);
        t.open(4, Some(1));
        assert_eq!(t.ids(), [1, 4, 2, 3]);
        t.open(5, Some(99));
        assert_eq!(t.ids(), [1, 4, 2, 3, 5], "unknown opener: at the end");
    }

    #[test]
    fn closing_the_active_tab_returns_to_the_most_recent_one() {
        let mut t = tabs(&[1, 2, 3]); // active 3, then 2, then 1
        t.activate(1);
        t.activate(3);
        // mru: 3, 1, 2
        assert_eq!(t.close(3), Some(1));
        assert_eq!(t.active(), Some(1));
        assert_eq!(t.ids(), [1, 2]);
    }

    #[test]
    fn closing_a_background_tab_keeps_the_active_one() {
        let mut t = tabs(&[1, 2, 3]);
        assert_eq!(t.close(1), None);
        assert_eq!(t.active(), Some(3));
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn closing_the_last_tab_leaves_nothing() {
        let mut t = tabs(&[7]);
        assert_eq!(t.close(7), None);
        assert!(t.is_empty() && t.active().is_none());
    }

    #[test]
    fn cycling_wraps_in_opening_order() {
        let t = tabs(&[1, 2, 3]);
        assert_eq!(t.next(3), Some(1));
        assert_eq!(t.next(1), Some(2));
        assert_eq!(t.prev(1), Some(3));
        assert_eq!(t.prev(3), Some(2));
        assert_eq!(tabs(&[5]).next(5), Some(5), "a lone tab cycles to itself");
        assert_eq!(t.next(42), None);
    }

    #[test]
    fn jumping_by_position() {
        let t = tabs(&[10, 20, 30]);
        assert_eq!(
            (t.nth(0), t.nth(2), t.nth(3), t.last()),
            (Some(10), Some(30), None, Some(30))
        );
    }

    #[test]
    fn activating_reorders_only_the_recency_list() {
        let mut t = tabs(&[1, 2, 3]);
        t.activate(1);
        assert_eq!(t.mru(), [1, 3, 2]);
        assert_eq!(t.ids(), [1, 2, 3]);
    }
}
