#![cfg_attr(not(windows), allow(dead_code))]
//! Paths of tabs the user closed, so the most recent one can be reopened.
//!
//! Nothing here calls Win32. [`ClosedTabs`] stores each path it is given and
//! does not read the disk or the USN journal. The most recent path is last.
//! The stack is unbounded. Eviction is TODO: define.

/// Closed paths. The most recent path is last.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosedTabs {
    paths: Vec<String>,
}

impl ClosedTabs {
    pub fn new() -> Self {
        Self { paths: Vec::new() }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.paths.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    /// Remember `path`. Empty or whitespace-only is ignored. Does not
    /// normalize. Does not cap the stack.
    pub fn remember(&mut self, path: &str) {
        if path.trim().is_empty() {
            return;
        }
        self.paths.push(path.to_string());
    }

    /// Pop the most recent path.
    pub fn reopen(&mut self) -> Option<String> {
        self.paths.pop()
    }
}

impl Default for ClosedTabs {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_empty() {
        let tabs = ClosedTabs::new();
        assert!(tabs.is_empty());
        assert_eq!(tabs.len(), 0);
        assert_eq!(tabs, ClosedTabs::default());
        assert_eq!(tabs.clone(), ClosedTabs::new());
    }

    #[test]
    fn remember_then_reopen_returns_that_path_and_leaves_empty() {
        let mut tabs = ClosedTabs::new();
        tabs.remember("C:/Windows");
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs.reopen(), Some("C:/Windows".to_string()));
        assert!(tabs.is_empty());
        assert_eq!(tabs.len(), 0);
    }

    #[test]
    fn two_remembers_reopen_in_lifo_order() {
        let mut tabs = ClosedTabs::new();
        tabs.remember("C:\\a");
        tabs.remember("C:\\b");
        assert_eq!(tabs.reopen(), Some("C:\\b".to_string()));
        assert_eq!(tabs.reopen(), Some("C:\\a".to_string()));
        assert!(tabs.is_empty());
    }

    #[test]
    fn remember_empty_and_whitespace_does_not_grow_len() {
        let mut tabs = ClosedTabs::new();
        tabs.remember("");
        tabs.remember("   ");
        assert_eq!(tabs.len(), 0);
        assert!(tabs.is_empty());
    }

    #[test]
    fn reopen_on_empty_is_none() {
        let mut tabs = ClosedTabs::new();
        assert_eq!(tabs.reopen(), None);
        assert!(tabs.is_empty());
    }

    #[test]
    fn a_third_remember_after_reopen_keeps_the_earlier_path() {
        let mut tabs = ClosedTabs::new();
        tabs.remember("C:\\a");
        tabs.remember("C:\\b");
        assert_eq!(tabs.reopen(), Some("C:\\b".to_string()));
        tabs.remember("C:\\c");
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs.reopen(), Some("C:\\c".to_string()));
        assert_eq!(tabs.reopen(), Some("C:\\a".to_string()));
        assert_eq!(tabs.reopen(), None);
    }

    #[test]
    fn a_thousand_remembers_all_reopen() {
        let mut tabs = ClosedTabs::new();
        for i in 0..1000 {
            tabs.remember(&format!("C:\\dir\\{i}"));
        }
        assert_eq!(tabs.len(), 1000);
        for i in (0..1000).rev() {
            assert_eq!(tabs.reopen(), Some(format!("C:\\dir\\{i}")));
        }
        assert!(tabs.is_empty());
    }
}
