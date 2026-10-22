#![cfg_attr(not(windows), allow(dead_code))]
//! Back, forward, and the address typed into this window.
//!
//! Nothing here calls Win32. [`normalize_address`] prepares a typed path,
//! [`tree_root`] decides whether the folder tree still covers it, and
//! [`History`] remembers directories this window already opened.

/// Directories this window has opened, with one current entry.
pub struct History {
    entries: Vec<String>,
    index: usize,
}

impl History {
    /// History begins at `path`. Back and forward are both unavailable.
    pub fn start(path: impl Into<String>) -> Self {
        Self {
            entries: vec![path.into()],
            index: 0,
        }
    }

    pub fn current(&self) -> &str {
        &self.entries[self.index]
    }

    pub fn can_back(&self) -> bool {
        self.index > 0
    }

    pub fn can_forward(&self) -> bool {
        self.index + 1 < self.entries.len()
    }

    /// The entry Back would open. Does not move.
    pub fn previous(&self) -> Option<&str> {
        self.index
            .checked_sub(1)
            .map(|index| self.entries[index].as_str())
    }

    /// The entry Forward would open. Does not move.
    pub fn following(&self) -> Option<&str> {
        self.entries.get(self.index + 1).map(String::as_str)
    }

    /// Move back. None if there is no previous entry.
    pub fn back(&mut self) -> Option<&str> {
        if !self.can_back() {
            return None;
        }
        self.index -= 1;
        Some(self.current())
    }

    /// Move forward. None if there is no next entry.
    pub fn forward(&mut self) -> Option<&str> {
        if !self.can_forward() {
            return None;
        }
        self.index += 1;
        Some(self.current())
    }

    /// Visit `path`. None if `path` is empty or equal to the current entry.
    /// A new path drops any forward entries.
    pub fn go(&mut self, path: &str) -> Option<&str> {
        if path.is_empty() || path == self.current() {
            return None;
        }
        self.entries.truncate(self.index + 1);
        self.entries.push(path.to_string());
        self.index += 1;
        Some(self.current())
    }
}

/// Trim whitespace and turn `/` into `\`. None when nothing remains.
pub fn normalize_address(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.replace('/', "\\"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeRoot {
    /// `target` is `root` or a descendant. Do not rebuild the tree.
    Keep,
    /// `target` is outside `root`. Replace the tree root with `target` and leave it collapsed.
    Replace,
}

/// Whether the folder tree rooted at `root` still covers `target`.
///
/// Both paths are compared after `/` is turned into `\` and trailing `\` are
/// removed. `C:\` and `C:` stay the same drive root. A descendant must begin
/// with that path plus `\`, so a longer name that only shares a prefix does not.
pub fn tree_root(root: &str, target: &str) -> TreeRoot {
    let root = comparable_path(root);
    let target = comparable_path(target);
    if target == root || target.starts_with(&format!("{root}\\")) {
        TreeRoot::Keep
    } else {
        TreeRoot::Replace
    }
}

/// `/` becomes `\`, then trailing `\` are removed. `C:\` therefore compares as
/// `C:`, which is still that drive root.
fn comparable_path(path: &str) -> String {
    let mut normalized = path.replace('/', "\\");
    let end = normalized.trim_end_matches('\\').len();
    normalized.truncate(end);
    normalized
}

/// Both paths name the same directory after slash normalization.
///
/// `C:\` and `C:` compare equal, matching [`tree_root`]. The strings passed to
/// `Shell::open` stay as the caller wrote them; this is only identity.
pub fn same_directory(left: &str, right: &str) -> bool {
    comparable_path(left) == comparable_path(right)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_is_the_current_path_without_history() {
        let mut history = History::start("C:\\a");
        assert_eq!(history.current(), "C:\\a");
        assert!(!history.can_back());
        assert!(!history.can_forward());
        assert_eq!(history.back(), None);
        assert_eq!(history.forward(), None);
        assert_eq!(history.current(), "C:\\a");
    }

    #[test]
    fn go_to_a_different_path_enables_back() {
        let mut history = History::start("C:\\a");
        assert_eq!(history.go("C:\\b"), Some("C:\\b"));
        assert_eq!(history.current(), "C:\\b");
        assert!(history.can_back());
        assert!(!history.can_forward());
    }

    #[test]
    fn go_to_the_same_path_returns_none() {
        let mut history = History::start("C:\\a");
        assert_eq!(history.go("C:\\a"), None);
        assert_eq!(history.current(), "C:\\a");
        assert!(!history.can_back());
        assert!(!history.can_forward());

        assert_eq!(history.go("C:\\b"), Some("C:\\b"));
        assert_eq!(history.back(), Some("C:\\a"));
        assert_eq!(history.go("C:\\a"), None);
        assert_eq!(history.current(), "C:\\a");
        assert!(history.can_forward());
        assert_eq!(history.forward(), Some("C:\\b"));
    }

    #[test]
    fn go_empty_returns_none() {
        let mut history = History::start("C:\\a");
        assert_eq!(history.go(""), None);
        assert_eq!(history.current(), "C:\\a");
        assert!(!history.can_back());
        assert!(!history.can_forward());

        assert_eq!(history.go("C:\\b"), Some("C:\\b"));
        assert_eq!(history.back(), Some("C:\\a"));
        assert_eq!(history.go(""), None);
        assert_eq!(history.current(), "C:\\a");
        assert!(history.can_forward());
        assert_eq!(history.forward(), Some("C:\\b"));
    }

    #[test]
    fn back_then_forward_round_trips() {
        let mut history = History::start("C:\\a");
        assert_eq!(history.go("C:\\b"), Some("C:\\b"));
        assert_eq!(history.back(), Some("C:\\a"));
        assert_eq!(history.current(), "C:\\a");
        assert!(!history.can_back());
        assert!(history.can_forward());
        assert_eq!(history.forward(), Some("C:\\b"));
        assert_eq!(history.current(), "C:\\b");
        assert!(history.can_back());
        assert!(!history.can_forward());
        assert_eq!(history.forward(), None);
        assert_eq!(history.previous(), Some("C:\\a"));
        assert_eq!(history.following(), None);
        assert_eq!(history.back(), Some("C:\\a"));
        assert_eq!(history.previous(), None);
        assert_eq!(history.following(), Some("C:\\b"));
    }

    #[test]
    fn go_after_back_drops_the_abandoned_forward_entry() {
        let mut history = History::start("C:\\a");
        assert_eq!(history.go("C:\\b"), Some("C:\\b"));
        assert_eq!(history.go("C:\\c"), Some("C:\\c"));
        assert_eq!(history.back(), Some("C:\\b"));
        assert!(history.can_forward());
        assert_eq!(history.go("C:\\d"), Some("C:\\d"));
        assert_eq!(history.current(), "C:\\d");
        assert!(!history.can_forward());
        assert_eq!(history.back(), Some("C:\\b"));
        assert_eq!(history.back(), Some("C:\\a"));
        assert_eq!(history.back(), None);
        assert_eq!(history.forward(), Some("C:\\b"));
        assert_eq!(history.forward(), Some("C:\\d"));
        assert_eq!(history.forward(), None);
    }

    #[test]
    fn normalize_address_trims_whitespace_and_uses_backslashes() {
        assert_eq!(
            normalize_address("  C:/Windows/System32  "),
            Some("C:\\Windows\\System32".to_string())
        );
        assert_eq!(
            normalize_address("\t//server/share/docs\n"),
            Some("\\\\server\\share\\docs".to_string())
        );
        assert_eq!(
            normalize_address("C:\\Program Files"),
            Some("C:\\Program Files".to_string())
        );
        assert_eq!(
            normalize_address("C:/Windows/"),
            Some("C:\\Windows\\".to_string())
        );
        assert_eq!(normalize_address(" / "), Some("\\".to_string()));
    }

    #[test]
    fn normalize_address_is_none_when_nothing_remains() {
        assert_eq!(normalize_address(""), None);
        assert_eq!(normalize_address("   "), None);
        assert_eq!(normalize_address("\t\n"), None);
    }

    #[test]
    fn equal_paths_keep_the_tree_root() {
        assert_eq!(tree_root("C:\\Windows", "C:\\Windows"), TreeRoot::Keep);
        assert_eq!(tree_root("C:\\Windows\\", "C:\\Windows"), TreeRoot::Keep);
        assert_eq!(tree_root("C:/Windows", "C:\\Windows"), TreeRoot::Keep);
        assert_eq!(tree_root("C:\\Windows/", "C:/Windows\\"), TreeRoot::Keep);
        assert_eq!(
            tree_root("\\\\server\\share\\", "\\\\server\\share"),
            TreeRoot::Keep
        );
    }

    #[test]
    fn drive_roots_with_and_without_a_trailing_slash_match() {
        assert_eq!(tree_root("C:\\", "C:"), TreeRoot::Keep);
        assert_eq!(tree_root("C:", "C:\\"), TreeRoot::Keep);
        assert_eq!(tree_root("C:/", "C:"), TreeRoot::Keep);
        assert_eq!(tree_root("C:\\", "C:\\"), TreeRoot::Keep);
        assert_eq!(tree_root("C:\\", "C:\\Windows"), TreeRoot::Keep);
        assert_eq!(tree_root("C:", "C:/Windows/System32"), TreeRoot::Keep);
    }

    #[test]
    fn a_descendant_keeps_the_tree_root() {
        assert_eq!(
            tree_root("C:\\Windows", "C:\\Windows\\System32"),
            TreeRoot::Keep
        );
        assert_eq!(
            tree_root("C:\\Windows\\", "C:/Windows/System32/"),
            TreeRoot::Keep
        );
        assert_eq!(
            tree_root("C:\\Windows\\System32", "C:\\Windows\\System32\\drivers"),
            TreeRoot::Keep
        );
    }

    #[test]
    fn a_prefix_that_is_not_a_parent_replaces_the_tree_root() {
        assert_eq!(
            tree_root("C:\\Windows", "C:\\WindowsOld"),
            TreeRoot::Replace
        );
        assert_eq!(
            tree_root("C:\\WindowsOld", "C:\\Windows"),
            TreeRoot::Replace
        );
    }

    #[test]
    fn a_unc_descendant_keeps_the_tree_root() {
        assert_eq!(
            tree_root("\\\\server\\share", "\\\\server\\share\\docs"),
            TreeRoot::Keep
        );
        assert_eq!(
            tree_root("//server/share/", "\\\\server\\share\\docs"),
            TreeRoot::Keep
        );
        assert_eq!(
            tree_root("\\\\server\\share", "\\\\server\\share"),
            TreeRoot::Keep
        );
        assert_eq!(
            tree_root("\\\\server\\share\\docs", "\\\\server\\share"),
            TreeRoot::Replace
        );
        assert_eq!(
            tree_root("\\\\server\\share", "\\\\server\\share2"),
            TreeRoot::Replace
        );
    }

    #[test]
    fn a_path_outside_the_root_replaces_the_tree() {
        assert_eq!(
            tree_root("C:\\Windows\\System32", "C:\\Windows"),
            TreeRoot::Replace
        );
        assert_eq!(tree_root("C:\\Windows", "D:\\Windows"), TreeRoot::Replace);
    }

    #[test]
    fn same_directory_ignores_slash_form() {
        assert!(same_directory("C:\\Windows", "C:/Windows/"));
        assert!(same_directory("C:\\", "C:"));
        assert!(same_directory("\\\\server\\share\\", "//server/share"));
        assert!(!same_directory("C:\\Windows", "C:\\WindowsOld"));
        assert!(!same_directory("C:\\Windows", "c:\\windows"));
    }

    #[test]
    fn tree_root_comparison_is_case_sensitive() {
        assert_eq!(tree_root("C:\\Windows", "c:\\windows"), TreeRoot::Replace);
        assert_eq!(
            tree_root("C:\\Windows", "C:\\windows\\System32"),
            TreeRoot::Replace
        );
        assert_eq!(
            tree_root("\\\\Server\\Share", "\\\\server\\share\\docs"),
            TreeRoot::Replace
        );
    }
}
