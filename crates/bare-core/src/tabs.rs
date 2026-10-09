//! Tabs as a tree, and which tab to switch to. No UI here: the browser keeps the web views, this keeps
//! the arithmetic honest.
//!
//! A tab opened from a link in another tab (`target=_blank`, middle- or Ctrl+click, `window.open`) is
//! that tab's child; every other tab is a root. Tabs are kept depth-first, each one directly before
//! its subtree, which is the order the sidebar draws them in (like the `tree` command) and the order
//! Ctrl+Tab walks. Closing the active tab goes back to the most recently used one; closing a parent
//! moves its children up a level, where it was.

use std::collections::{HashMap, HashSet};

pub type TabId = u64;

#[derive(Debug, Default, Clone)]
pub struct TabTree {
    /// Depth-first: a tab, then its children's subtrees in the order they were opened.
    order: Vec<TabId>,
    /// Most recently used first; the active tab is `mru[0]`.
    mru: Vec<TabId>,
    /// Child → parent. Roots have no entry.
    parent: HashMap<TabId, TabId>,
    /// Tabs whose subtree is folded away in the sidebar.
    folded: HashSet<TabId>,
}

/// One line of the sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: TabId,
    /// What `tree` prints before the name: `""` for a root, then `├── `, `│   └── `, ...
    pub prefix: String,
    /// It has children, so it can be folded.
    pub has_children: bool,
    /// Folded: how many tabs are hidden under it. 0 when it isn't folded.
    pub hidden: usize,
}

impl TabTree {
    /// Add a tab as the last child of `parent`, or as the last root if there is no (known) parent.
    /// It does not become active by itself, but it is never hidden in a folded subtree.
    pub fn open(&mut self, id: TabId, parent: Option<TabId>) {
        let parent = parent.filter(|p| self.order.contains(p));
        let at = match parent {
            Some(p) => {
                self.parent.insert(id, p);
                self.subtree_end(p)
            }
            None => self.order.len(),
        };
        self.order.insert(at, id);
        self.mru.push(id);
        self.unfold_ancestors(id);
    }

    pub fn activate(&mut self, id: TabId) {
        if let Some(i) = self.mru.iter().position(|&t| t == id) {
            self.mru.remove(i);
            self.mru.insert(0, id);
            self.unfold_ancestors(id);
        }
    }

    pub fn active(&self) -> Option<TabId> {
        self.mru.first().copied()
    }

    /// Remove a tab; its children take its place, one level up. If it was the active one, returns
    /// the tab that becomes active.
    pub fn close(&mut self, id: TabId) -> Option<TabId> {
        let was_active = self.active() == Some(id);
        match self.parent.remove(&id) {
            Some(up) => self
                .parent
                .values_mut()
                .filter(|p| **p == id)
                .for_each(|p| *p = up),
            None => self.parent.retain(|_, p| *p != id),
        }
        self.folded.remove(&id);
        self.order.retain(|&t| t != id);
        self.mru.retain(|&t| t != id);
        if was_active { self.active() } else { None }
    }

    pub fn parent(&self, id: TabId) -> Option<TabId> {
        self.parent.get(&id).copied()
    }

    /// Fold or unfold the tabs under `id`. Returns whether it is folded now (a tab without children
    /// can't be).
    pub fn toggle_fold(&mut self, id: TabId) -> bool {
        if self.folded.remove(&id) || !self.has_children(id) {
            return false;
        }
        self.folded.insert(id);
        true
    }

    /// The row that stands for `id` in the sidebar: the tab itself, or the folded ancestor hiding it.
    pub fn shown_as(&self, id: TabId) -> TabId {
        let mut shown = id;
        let mut t = id;
        while let Some(p) = self.parent(t) {
            if self.folded.contains(&p) {
                shown = p;
            }
            t = p;
        }
        shown
    }

    /// The sidebar's lines, top to bottom: every tab that isn't inside a folded subtree.
    pub fn rows(&self) -> Vec<Row> {
        // The last child of each parent (`None`: the last root). Anything else has a later sibling.
        let mut last: HashMap<Option<TabId>, TabId> = HashMap::new();
        for &t in &self.order {
            last.insert(self.parent(t), t);
        }
        let has_later_sibling = |t: TabId| last.get(&self.parent(t)) != Some(&t);

        let depths: Vec<usize> = self.order.iter().map(|&t| self.depth(t)).collect();
        let mut rows = Vec::with_capacity(self.order.len());
        let mut i = 0;
        while i < self.order.len() {
            let id = self.order[i];
            let end = i
                + 1
                + depths[i + 1..]
                    .iter()
                    .take_while(|&&d| d > depths[i])
                    .count();
            let folded = self.folded.contains(&id);
            // Ancestors below the root, outermost first; each draws `│` while it has siblings to come.
            let mut chain = Vec::new();
            let mut t = id;
            while let Some(p) = self.parent(t) {
                chain.push(t);
                t = p;
            }
            let mut prefix = String::new();
            for (depth, &t) in chain.iter().rev().enumerate() {
                let more = has_later_sibling(t);
                prefix.push_str(match (depth + 1 == chain.len(), more) {
                    (false, true) => "│   ",
                    (false, false) => "    ",
                    (true, true) => "├── ",
                    (true, false) => "└── ",
                });
            }
            rows.push(Row {
                id,
                prefix,
                has_children: end > i + 1,
                hidden: if folded { end - i - 1 } else { 0 },
            });
            i = if folded { end } else { i + 1 };
        }
        rows
    }

    /// The next tab down the sidebar, wrapping around. Folded-away tabs are skipped.
    pub fn next(&self, from: TabId) -> Option<TabId> {
        self.step(from, 1)
    }

    pub fn prev(&self, from: TabId) -> Option<TabId> {
        self.step(from, -1)
    }

    /// The `n`th line of the sidebar, 0-based; `Alt+1` is `nth(0)`.
    pub fn nth(&self, n: usize) -> Option<TabId> {
        self.visible().get(n).copied()
    }

    pub fn last(&self) -> Option<TabId> {
        self.visible().last().copied()
    }

    /// Every tab, depth-first.
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

    fn step(&self, from: TabId, delta: isize) -> Option<TabId> {
        if !self.order.contains(&from) {
            return None;
        }
        let visible = self.visible();
        let from = self.shown_as(from);
        let i = visible.iter().position(|&t| t == from)? as isize;
        let n = visible.len() as isize;
        visible.get((i + delta).rem_euclid(n) as usize).copied()
    }

    fn visible(&self) -> Vec<TabId> {
        self.rows().into_iter().map(|r| r.id).collect()
    }

    fn has_children(&self, id: TabId) -> bool {
        self.order
            .iter()
            .position(|&t| t == id)
            .is_some_and(|i| self.subtree_end_at(i) > i + 1)
    }

    fn unfold_ancestors(&mut self, id: TabId) {
        let mut t = id;
        while let Some(p) = self.parent(t) {
            self.folded.remove(&p);
            t = p;
        }
    }

    fn depth(&self, id: TabId) -> usize {
        let mut d = 0;
        let mut t = id;
        while let Some(p) = self.parent(t) {
            d += 1;
            t = p;
        }
        d
    }

    /// Index just past the last tab of `id`'s subtree.
    fn subtree_end(&self, id: TabId) -> usize {
        match self.order.iter().position(|&t| t == id) {
            Some(i) => self.subtree_end_at(i),
            None => self.order.len(),
        }
    }

    /// Depth-first, a subtree is the run of deeper tabs right after its root.
    fn subtree_end_at(&self, i: usize) -> usize {
        let depth = self.depth(self.order[i]);
        let mut j = i + 1;
        while j < self.order.len() && self.depth(self.order[j]) > depth {
            j += 1;
        }
        j
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs(ids: &[TabId]) -> TabTree {
        let mut t = TabTree::default();
        for &i in ids {
            t.open(i, None);
            t.activate(i);
        }
        t
    }

    /// ```text
    /// 1
    /// ├── 2
    /// │   ├── 4
    /// │   └── 5
    /// └── 3
    ///     └── 6
    /// 7
    /// ```
    fn sample() -> TabTree {
        let mut t = TabTree::default();
        for (id, parent) in [
            (1, None),
            (2, Some(1)),
            (3, Some(1)),
            (4, Some(2)),
            (5, Some(2)),
            (6, Some(3)),
            (7, None),
        ] {
            t.open(id, parent);
        }
        t.activate(1);
        t
    }

    /// The sidebar as `tree` would print it, with tab ids for names; folded rows end in `+n`.
    fn drawn(t: &TabTree) -> String {
        t.rows()
            .iter()
            .map(|r| match r.hidden {
                0 => format!("{}{}\n", r.prefix, r.id),
                n => format!("{}{} +{n}\n", r.prefix, r.id),
            })
            .collect()
    }

    #[test]
    fn children_open_after_their_parents_subtree() {
        let mut t = tabs(&[1, 2, 3]);
        t.open(4, Some(1));
        assert_eq!(t.ids(), [1, 4, 2, 3]);
        t.open(5, Some(1));
        assert_eq!(t.ids(), [1, 4, 5, 2, 3], "siblings in opening order");
        t.open(6, Some(4));
        assert_eq!(
            t.ids(),
            [1, 4, 6, 5, 2, 3],
            "a grandchild goes under its own parent"
        );
        t.open(7, Some(99));
        assert_eq!(
            t.ids(),
            [1, 4, 6, 5, 2, 3, 7],
            "unknown parent: a root at the end"
        );
        assert_eq!(
            (t.parent(6), t.parent(4), t.parent(1)),
            (Some(4), Some(1), None)
        );
    }

    #[test]
    fn rows_are_drawn_like_the_tree_command() {
        let t = sample();
        assert_eq!(
            drawn(&t),
            "1\n├── 2\n│   ├── 4\n│   └── 5\n└── 3\n    └── 6\n7\n"
        );
        let rows = t.rows();
        assert!(rows[0].has_children && rows[1].has_children && !rows[2].has_children);
    }

    #[test]
    fn closing_a_parent_moves_its_children_up_in_place() {
        let mut t = sample();
        t.close(2);
        assert_eq!(drawn(&t), "1\n├── 4\n├── 5\n└── 3\n    └── 6\n7\n");
        t.close(1);
        assert_eq!(
            drawn(&t),
            "4\n5\n3\n└── 6\n7\n",
            "a root's children become roots"
        );
        assert_eq!(t.parent(4), None);
        assert_eq!(t.parent(6), Some(3));
    }

    #[test]
    fn folding_hides_a_subtree_and_counts_it() {
        let mut t = sample();
        assert!(t.toggle_fold(2));
        assert_eq!(drawn(&t), "1\n├── 2 +2\n└── 3\n    └── 6\n7\n");
        assert!(t.toggle_fold(1));
        assert_eq!(drawn(&t), "1 +5\n7\n");
        assert!(!t.toggle_fold(1), "toggling again unfolds");
        assert_eq!(
            drawn(&t),
            "1\n├── 2 +2\n└── 3\n    └── 6\n7\n",
            "2 stays folded"
        );
        assert!(!t.toggle_fold(7), "a tab without children can't fold");
        assert_eq!(t.shown_as(4), 2);
        assert_eq!(t.shown_as(6), 6);
    }

    #[test]
    fn activating_or_opening_inside_a_folded_subtree_unfolds_it() {
        let mut t = sample();
        t.toggle_fold(1);
        t.activate(4);
        assert_eq!(drawn(&t).lines().count(), 7);
        t.toggle_fold(3);
        t.open(8, Some(6));
        assert_eq!(t.shown_as(8), 8, "a new tab is never born hidden");
    }

    #[test]
    fn closing_a_folded_parent_shows_its_children() {
        let mut t = sample();
        t.toggle_fold(2);
        t.close(2);
        assert_eq!(drawn(&t), "1\n├── 4\n├── 5\n└── 3\n    └── 6\n7\n");
    }

    #[test]
    fn cycling_and_jumping_follow_the_visible_rows() {
        let mut t = sample();
        assert_eq!(t.next(5), Some(3));
        assert_eq!(t.prev(3), Some(5));
        assert_eq!(t.next(7), Some(1));
        t.toggle_fold(2);
        assert_eq!(t.next(2), Some(3), "skips the folded 4 and 5");
        assert_eq!(
            t.next(4),
            Some(3),
            "from inside a folded subtree: from its row"
        );
        assert_eq!((t.nth(2), t.last()), (Some(3), Some(7)));
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
    fn cycling_wraps_in_order() {
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
