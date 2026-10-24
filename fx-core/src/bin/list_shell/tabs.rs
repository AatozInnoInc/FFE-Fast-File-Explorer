#![cfg_attr(not(windows), allow(dead_code))]
//! Tabs for one pane.
//!
//! Nothing here calls Win32. [`PaneTabs::start`] opens one tab on a path.
//! [`PaneTabs::new_tab`] copies that path into a new tab and activates it.
//! Closing the only tab is refused, and a closed tab is not kept.

/// Paths shown as tabs in one pane, and which tab is active.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneTabs {
    paths: Vec<String>,
    active: usize,
}

impl PaneTabs {
    /// One tab showing `path`. That tab is active.
    pub fn start(path: impl Into<String>) -> Self {
        Self {
            paths: vec![path.into()],
            active: 0,
        }
    }

    pub fn active_path(&self) -> &str {
        &self.paths[self.active]
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn len(&self) -> usize {
        self.paths.len()
    }

    pub fn paths(&self) -> &[String] {
        &self.paths
    }

    /// Last path segment after normalizing `/` to `\` and trimming trailing `\`.
    /// A drive root (`C:\` or `C:`) stays that root. A UNC share root
    /// (`\\server\share`) stays the full share root. Empty stays empty.
    pub fn title(path: &str) -> String {
        if path.is_empty() {
            return String::new();
        }
        let normalized = path.replace('/', "\\");
        if let Some(root) = drive_root_title(&normalized) {
            return root;
        }
        let trimmed = normalized.trim_end_matches('\\');
        if trimmed.is_empty() {
            return String::new();
        }
        if let Some(share) = unc_share_root(trimmed) {
            return share.to_string();
        }
        trimmed
            .rsplit_once('\\')
            .map(|(_, name)| name)
            .unwrap_or(trimmed)
            .to_string()
    }

    /// Append a tab that copies the active path and make it active. Return that path.
    pub fn new_tab(&mut self) -> &str {
        let path = self.paths[self.active].clone();
        self.paths.push(path);
        self.active = self.paths.len() - 1;
        &self.paths[self.active]
    }

    /// Make `index` active. None if out of range or already active. Does not reorder.
    pub fn select(&mut self, index: usize) -> Option<&str> {
        if index >= self.paths.len() || index == self.active {
            return None;
        }
        self.active = index;
        Some(&self.paths[self.active])
    }

    /// Close the active tab and activate a neighbor (the next one, or the
    /// previous if closing the last). None if this is the only tab; state
    /// unchanged.
    pub fn close_active(&mut self) -> Option<&str> {
        if self.paths.len() == 1 {
            return None;
        }
        self.paths.remove(self.active);
        if self.active == self.paths.len() {
            self.active -= 1;
        }
        Some(self.active_path())
    }
}

/// `C:` after slash normalization. A trailing separator stays as `C:\`.
fn drive_root_title(normalized: &str) -> Option<String> {
    let trimmed = normalized.trim_end_matches('\\');
    let mut chars = trimmed.chars();
    let letter = chars.next()?;
    if !letter.is_ascii_alphabetic() || chars.next() != Some(':') || chars.next().is_some() {
        return None;
    }
    if normalized.len() == trimmed.len() {
        Some(trimmed.to_string())
    } else {
        Some(format!("{trimmed}\\"))
    }
}

/// `\\server\share` with nothing after the share. Trailing `\` is already trimmed.
fn unc_share_root(trimmed: &str) -> Option<&str> {
    let rest = trimmed.strip_prefix(r"\\")?;
    let (server, share) = rest.split_once('\\')?;
    if server.is_empty() || share.is_empty() || share.contains('\\') {
        None
    } else {
        Some(trimmed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_has_one_tab_at_index_zero() {
        let tabs = PaneTabs::start("C:\\docs");
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs.active_index(), 0);
        assert_eq!(tabs.active_path(), "C:\\docs");
        assert_eq!(tabs.paths(), ["C:\\docs".to_string()]);
    }

    #[test]
    fn new_tab_copies_the_active_path_and_activates_the_copy() {
        let mut tabs = PaneTabs::start("C:\\docs");
        assert_eq!(tabs.new_tab(), "C:\\docs");
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs.active_index(), 1);
        assert_eq!(tabs.active_path(), "C:\\docs");
        assert_eq!(tabs.paths()[0], "C:\\docs");
        assert_eq!(
            tabs.paths(),
            ["C:\\docs".to_string(), "C:\\docs".to_string()]
        );
    }

    #[test]
    fn select_switches_and_rejects_the_active_or_a_missing_index() {
        let mut tabs = PaneTabs::start("A");
        tabs.new_tab();
        tabs.paths[1] = "B".to_string();
        let before = tabs.paths().to_vec();

        assert_eq!(tabs.select(0), Some("A"));
        assert_eq!(tabs.active_index(), 0);
        assert_eq!(tabs.active_path(), "A");
        assert_eq!(tabs.select(0), None);
        assert_eq!(tabs.active_index(), 0);
        assert_eq!(tabs.select(2), None);
        assert_eq!(tabs.active_index(), 0);
        assert_eq!(tabs.active_path(), "A");
        assert_eq!(tabs.paths(), before.as_slice());
    }

    #[test]
    fn close_active_on_the_only_tab_returns_none() {
        let mut tabs = PaneTabs::start("C:\\only");
        assert_eq!(tabs.close_active(), None);
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs.active_index(), 0);
        assert_eq!(tabs.active_path(), "C:\\only");
        assert_eq!(tabs.paths(), ["C:\\only".to_string()]);
    }

    #[test]
    fn close_the_second_of_two_returns_the_first_path() {
        let mut tabs = PaneTabs::start("A");
        tabs.new_tab();
        tabs.paths[1] = "B".to_string();
        assert_eq!(tabs.active_index(), 1);
        assert_eq!(tabs.close_active(), Some("A"));
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs.active_index(), 0);
        assert_eq!(tabs.active_path(), "A");
        assert_eq!(tabs.paths(), ["A".to_string()]);
    }

    #[test]
    fn close_the_first_of_three_leaves_the_next_tab_active() {
        let mut tabs = PaneTabs::start("A");
        tabs.new_tab();
        tabs.new_tab();
        tabs.paths[1] = "B".to_string();
        tabs.paths[2] = "C".to_string();
        assert_eq!(tabs.select(0), Some("A"));
        assert_eq!(tabs.close_active(), Some("B"));
        assert_eq!(tabs.paths(), ["B".to_string(), "C".to_string()]);
        assert_eq!(tabs.active_index(), 0);
        assert_eq!(tabs.active_path(), "B");
    }

    #[test]
    fn title_uses_the_last_segment_and_keeps_roots() {
        assert_eq!(PaneTabs::title(r"C:\Windows\System32"), "System32");
        assert_eq!(PaneTabs::title("C:/Windows/"), "Windows");
        assert_eq!(PaneTabs::title(r"C:\"), r"C:\");
        assert_eq!(PaneTabs::title("C:"), "C:");
        assert_eq!(PaneTabs::title(r"\\server\share"), r"\\server\share");
        assert_eq!(PaneTabs::title(r"\\server\share\docs"), "docs");
        assert_eq!(PaneTabs::title(""), "");
    }
}
