#![cfg_attr(not(windows), allow(dead_code))]
// The window does not own tabs yet. Tests still run with the lint on.
#![cfg_attr(all(windows, not(test)), allow(dead_code))]
//! One back and forward stack per tab.
//!
//! Nothing here calls Win32. Each tab owns its own [`History`]. Switching
//! tabs does not push a history entry.

use super::nav::History;

/// Back and forward stacks for the open tabs, with one active tab.
pub struct TabHistories {
    tabs: Vec<History>,
    active: usize,
}

impl TabHistories {
    /// One tab, started at `path`. Back and forward are both unavailable.
    pub fn start(path: impl Into<String>) -> Self {
        Self {
            tabs: vec![History::start(path)],
            active: 0,
        }
    }

    pub fn active_path(&self) -> &str {
        self.tabs[self.active].current()
    }

    /// Current path of the active tab. Does not move.
    pub fn current(&self) -> &str {
        self.active_path()
    }

    /// The path Back would open on the active tab. Does not move.
    pub fn previous(&self) -> Option<&str> {
        self.tabs[self.active].previous()
    }

    /// The path Forward would open on the active tab. Does not move.
    pub fn following(&self) -> Option<&str> {
        self.tabs[self.active].following()
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    /// Current path of every tab, index order.
    pub fn paths(&self) -> Vec<&str> {
        self.tabs.iter().map(History::current).collect()
    }

    pub fn can_back(&self) -> bool {
        self.tabs[self.active].can_back()
    }

    pub fn can_forward(&self) -> bool {
        self.tabs[self.active].can_forward()
    }

    /// [`History::back`] on the active tab only.
    pub fn back(&mut self) -> Option<&str> {
        self.tabs[self.active].back()
    }

    /// [`History::forward`] on the active tab only.
    pub fn forward(&mut self) -> Option<&str> {
        self.tabs[self.active].forward()
    }

    /// [`History::go`] on the active tab only.
    pub fn go(&mut self, path: &str) -> Option<&str> {
        self.tabs[self.active].go(path)
    }

    /// New tab copies the active path as [`History::start`] (no back). Makes it
    /// active. Returns that path.
    pub fn new_tab(&mut self) -> &str {
        let path = self.active_path().to_string();
        self.open_tab(path)
    }

    /// Append a tab started at `path` with no Back or Forward, and make it
    /// active. Returns that path.
    pub fn open_tab(&mut self, path: impl Into<String>) -> &str {
        self.tabs.push(History::start(path));
        self.active = self.tabs.len() - 1;
        self.active_path()
    }

    /// Switch active tab. Does not call go. None if out of range or already active.
    pub fn select(&mut self, index: usize) -> Option<&str> {
        if index >= self.tabs.len() || index == self.active {
            return None;
        }
        self.active = index;
        Some(self.active_path())
    }

    /// Drop the active tab. Activate the next, or the previous if it was last.
    /// None if it is the only tab.
    pub fn close_active(&mut self) -> Option<&str> {
        if self.tabs.len() == 1 {
            return None;
        }
        self.tabs.remove(self.active);
        if self.active == self.tabs.len() {
            self.active -= 1;
        }
        Some(self.active_path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_is_one_tab_without_back_or_forward() {
        let mut tabs = TabHistories::start("C:\\a");
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs.active_index(), 0);
        assert_eq!(tabs.active_path(), "C:\\a");
        assert_eq!(tabs.paths(), vec!["C:\\a"]);
        assert!(!tabs.can_back());
        assert!(!tabs.can_forward());
        assert_eq!(tabs.back(), None);
        assert_eq!(tabs.forward(), None);
        assert_eq!(tabs.active_index(), 0);
        assert_eq!(tabs.active_path(), "C:\\a");
    }

    #[test]
    fn go_back_and_forward_stay_on_that_tab() {
        let mut tabs = TabHistories::start("C:\\a");
        assert_eq!(tabs.go("C:\\b"), Some("C:\\b"));
        assert_eq!(tabs.active_index(), 0);
        assert_eq!(tabs.len(), 1);
        assert!(tabs.can_back());
        assert!(!tabs.can_forward());
        assert_eq!(tabs.back(), Some("C:\\a"));
        assert_eq!(tabs.active_index(), 0);
        assert_eq!(tabs.active_path(), "C:\\a");
        assert!(!tabs.can_back());
        assert!(tabs.can_forward());
        assert_eq!(tabs.forward(), Some("C:\\b"));
        assert_eq!(tabs.active_index(), 0);
        assert_eq!(tabs.active_path(), "C:\\b");
        assert!(tabs.can_back());
        assert!(!tabs.can_forward());
        assert_eq!(tabs.paths(), vec!["C:\\b"]);
    }

    #[test]
    fn open_tab_starts_on_its_path_with_no_back() {
        let mut tabs = TabHistories::start("C:\\a");
        let _ = tabs.go("C:\\b");
        assert!(tabs.can_back());
        assert_eq!(tabs.open_tab("D:\\closed"), "D:\\closed");
        assert_eq!(tabs.active_index(), 1);
        assert!(!tabs.can_back());
        assert!(!tabs.can_forward());
        assert_eq!(tabs.paths(), ["C:\\b", "D:\\closed"]);
    }

    #[test]
    fn new_tab_copies_the_path_and_keeps_the_first_back_stack() {
        let mut tabs = TabHistories::start("C:\\a");
        assert_eq!(tabs.go("C:\\b"), Some("C:\\b"));
        assert_eq!(tabs.new_tab(), "C:\\b");
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs.active_index(), 1);
        assert_eq!(tabs.paths(), vec!["C:\\b", "C:\\b"]);
        assert!(!tabs.can_back());
        assert!(!tabs.can_forward());
        assert_eq!(tabs.back(), None);
        assert_eq!(tabs.active_path(), "C:\\b");

        assert_eq!(tabs.select(0), Some("C:\\b"));
        assert_eq!(tabs.active_index(), 0);
        assert!(tabs.can_back());
        assert!(!tabs.can_forward());
        assert_eq!(tabs.back(), Some("C:\\a"));
        assert_eq!(tabs.active_path(), "C:\\a");
        assert!(tabs.can_forward());

        assert_eq!(tabs.select(1), Some("C:\\b"));
        assert_eq!(tabs.active_index(), 1);
        assert!(!tabs.can_back());
        assert!(!tabs.can_forward());
        assert_eq!(tabs.active_path(), "C:\\b");
        assert_eq!(tabs.paths(), vec!["C:\\a", "C:\\b"]);
    }

    #[test]
    fn select_does_not_change_either_tabs_history() {
        let mut tabs = TabHistories::start("C:\\a");
        assert_eq!(tabs.go("C:\\b"), Some("C:\\b"));
        assert_eq!(tabs.new_tab(), "C:\\b");
        assert_eq!(tabs.go("C:\\c"), Some("C:\\c"));
        assert_eq!(tabs.paths(), vec!["C:\\b", "C:\\c"]);

        assert_eq!(tabs.select(0), Some("C:\\b"));
        assert_eq!(tabs.active_index(), 0);
        assert_eq!(tabs.active_path(), "C:\\b");
        assert!(tabs.can_back());
        assert!(!tabs.can_forward());
        assert_eq!(tabs.paths(), vec!["C:\\b", "C:\\c"]);

        assert_eq!(tabs.select(1), Some("C:\\c"));
        assert_eq!(tabs.active_index(), 1);
        assert!(tabs.can_back());
        assert!(!tabs.can_forward());
        assert_eq!(tabs.back(), Some("C:\\b"));
        assert!(tabs.can_forward());
        assert_eq!(tabs.forward(), Some("C:\\c"));
        assert_eq!(tabs.paths(), vec!["C:\\b", "C:\\c"]);

        assert_eq!(tabs.select(1), None);
        assert_eq!(tabs.active_index(), 1);
        assert_eq!(tabs.active_path(), "C:\\c");
        assert_eq!(tabs.select(2), None);
        assert_eq!(tabs.active_index(), 1);
        assert_eq!(tabs.active_path(), "C:\\c");
        assert!(tabs.can_back());
        assert!(!tabs.can_forward());

        assert_eq!(tabs.select(0), Some("C:\\b"));
        assert!(tabs.can_back());
        assert!(!tabs.can_forward());
        assert_eq!(tabs.back(), Some("C:\\a"));
        assert_eq!(tabs.forward(), Some("C:\\b"));
    }

    #[test]
    fn go_on_the_second_tab_does_not_enable_back_on_the_first() {
        let mut tabs = TabHistories::start("C:\\a");
        assert_eq!(tabs.new_tab(), "C:\\a");
        assert_eq!(tabs.active_index(), 1);
        assert!(!tabs.can_back());
        assert_eq!(tabs.go("C:\\b"), Some("C:\\b"));
        assert!(tabs.can_back());
        assert_eq!(tabs.active_index(), 1);
        assert_eq!(tabs.paths(), vec!["C:\\a", "C:\\b"]);

        assert_eq!(tabs.select(0), Some("C:\\a"));
        assert_eq!(tabs.active_index(), 0);
        assert!(!tabs.can_back());
        assert!(!tabs.can_forward());
        assert_eq!(tabs.back(), None);
        assert_eq!(tabs.forward(), None);
        assert_eq!(tabs.active_path(), "C:\\a");
        assert_eq!(tabs.paths(), vec!["C:\\a", "C:\\b"]);
    }

    #[test]
    fn close_active_drops_that_history_without_merging_it() {
        let mut tabs = TabHistories::start("C:\\a");
        assert_eq!(tabs.go("C:\\b"), Some("C:\\b"));
        assert_eq!(tabs.new_tab(), "C:\\b");
        assert_eq!(tabs.go("C:\\c"), Some("C:\\c"));
        assert_eq!(tabs.go("C:\\d"), Some("C:\\d"));
        assert_eq!(tabs.back(), Some("C:\\c"));
        assert!(tabs.can_forward());

        assert_eq!(tabs.close_active(), Some("C:\\b"));
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs.active_index(), 0);
        assert_eq!(tabs.active_path(), "C:\\b");
        assert_eq!(tabs.paths(), vec!["C:\\b"]);
        assert!(tabs.can_back());
        assert!(!tabs.can_forward());
        assert_eq!(tabs.back(), Some("C:\\a"));
        assert_eq!(tabs.forward(), Some("C:\\b"));
        assert_eq!(tabs.forward(), None);
        assert_eq!(tabs.back(), Some("C:\\a"));
        assert_eq!(tabs.back(), None);
    }

    #[test]
    fn close_of_the_last_tab_activates_the_previous() {
        let mut tabs = TabHistories::start("C:\\a");
        assert_eq!(tabs.go("C:\\left"), Some("C:\\left"));
        assert_eq!(tabs.new_tab(), "C:\\left");
        assert_eq!(tabs.go("C:\\right"), Some("C:\\right"));
        assert_eq!(tabs.close_active(), Some("C:\\left"));
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs.active_index(), 0);
        assert_eq!(tabs.active_path(), "C:\\left");
        assert!(tabs.can_back());
        assert!(!tabs.can_forward());
        assert_eq!(tabs.back(), Some("C:\\a"));
        assert_eq!(tabs.forward(), Some("C:\\left"));
        assert_eq!(tabs.forward(), None);
    }

    #[test]
    fn close_of_a_middle_tab_activates_the_next() {
        let mut tabs = TabHistories::start("C:\\a");
        assert_eq!(tabs.go("C:\\b"), Some("C:\\b"));
        assert_eq!(tabs.new_tab(), "C:\\b");
        assert_eq!(tabs.go("C:\\mid"), Some("C:\\mid"));
        assert_eq!(tabs.new_tab(), "C:\\mid");
        assert_eq!(tabs.go("C:\\c"), Some("C:\\c"));
        assert_eq!(tabs.select(1), Some("C:\\mid"));

        assert_eq!(tabs.close_active(), Some("C:\\c"));
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs.active_index(), 1);
        assert_eq!(tabs.paths(), vec!["C:\\b", "C:\\c"]);
        assert!(tabs.can_back());
        assert_eq!(tabs.back(), Some("C:\\mid"));
        assert!(!tabs.can_back());

        assert_eq!(tabs.select(0), Some("C:\\b"));
        assert!(tabs.can_back());
        assert!(!tabs.can_forward());
        assert_eq!(tabs.back(), Some("C:\\a"));
        assert_eq!(tabs.forward(), Some("C:\\b"));
        assert_eq!(tabs.forward(), None);
    }

    #[test]
    fn close_of_the_only_tab_returns_none() {
        let mut tabs = TabHistories::start("C:\\a");
        assert_eq!(tabs.go("C:\\b"), Some("C:\\b"));
        assert_eq!(tabs.close_active(), None);
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs.active_index(), 0);
        assert_eq!(tabs.active_path(), "C:\\b");
        assert_eq!(tabs.paths(), vec!["C:\\b"]);
        assert!(tabs.can_back());
        assert!(!tabs.can_forward());
        assert_eq!(tabs.back(), Some("C:\\a"));
    }

    #[test]
    fn empty_go_and_same_path_go_return_none() {
        let mut tabs = TabHistories::start("C:\\a");
        assert_eq!(tabs.go(""), None);
        assert_eq!(tabs.active_path(), "C:\\a");
        assert_eq!(tabs.active_index(), 0);
        assert!(!tabs.can_back());
        assert!(!tabs.can_forward());

        assert_eq!(tabs.go("C:\\a"), None);
        assert_eq!(tabs.active_path(), "C:\\a");
        assert!(!tabs.can_back());
        assert!(!tabs.can_forward());

        assert_eq!(tabs.go("C:\\b"), Some("C:\\b"));
        assert_eq!(tabs.back(), Some("C:\\a"));
        assert_eq!(tabs.go("C:\\a"), None);
        assert_eq!(tabs.active_path(), "C:\\a");
        assert!(tabs.can_forward());
        assert_eq!(tabs.forward(), Some("C:\\b"));

        assert_eq!(tabs.back(), Some("C:\\a"));
        assert_eq!(tabs.go(""), None);
        assert_eq!(tabs.active_path(), "C:\\a");
        assert!(tabs.can_forward());
        assert_eq!(tabs.forward(), Some("C:\\b"));
    }
}
