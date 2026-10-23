#![cfg_attr(not(windows), allow(dead_code))]
//! Dual-pane paths for this window.
//!
//! Nothing here calls Win32. [`Panes::start`] puts the same path in both panes
//! and activates the left one. [`Panes::toggle`] switches which pane is active
//! and does not change either path.

/// Which of the two panes is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneId {
    Left,
    Right,
}

/// Paths shown in the two panes, and which pane receives navigation.
pub struct Panes {
    left: String,
    right: String,
    active: PaneId,
}

impl Panes {
    /// Both panes show `path`. The left pane is active.
    pub fn start(path: impl Into<String>) -> Self {
        let path = path.into();
        Self {
            left: path.clone(),
            right: path,
            active: PaneId::Left,
        }
    }

    pub fn active(&self) -> PaneId {
        self.active
    }

    pub fn active_path(&self) -> &str {
        match self.active {
            PaneId::Left => &self.left,
            PaneId::Right => &self.right,
        }
    }

    /// Replace the path of the active pane. The other pane is left as it is.
    pub fn set_active_path(&mut self, path: &str) {
        self.set_path(self.active, path);
    }

    /// Replace one pane's path. The active pane does not change.
    pub fn set_path(&mut self, id: PaneId, path: &str) {
        match id {
            PaneId::Left => self.left = path.to_string(),
            PaneId::Right => self.right = path.to_string(),
        }
    }

    /// Make `id` the active pane. Both paths stay as they are.
    pub fn activate(&mut self, id: PaneId) {
        self.active = id;
    }

    /// Switch which pane is active. Both paths stay as they are.
    pub fn toggle(&mut self) {
        self.activate(match self.active {
            PaneId::Left => PaneId::Right,
            PaneId::Right => PaneId::Left,
        });
    }

    /// Path of the pane that is not active.
    pub fn other_path(&self) -> &str {
        match self.active {
            PaneId::Left => &self.right,
            PaneId::Right => &self.left,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_shows_the_same_path_in_both_panes_and_activates_left() {
        let panes = Panes::start("C:\\docs");
        assert_eq!(panes.active(), PaneId::Left);
        assert_eq!(panes.active_path(), "C:\\docs");
        assert_eq!(panes.other_path(), "C:\\docs");
    }

    #[test]
    fn set_active_path_changes_only_the_active_pane() {
        let mut panes = Panes::start("C:\\docs");
        panes.set_active_path("C:\\left");
        assert_eq!(panes.active(), PaneId::Left);
        assert_eq!(panes.active_path(), "C:\\left");
        assert_eq!(panes.other_path(), "C:\\docs");

        panes.set_active_path("C:\\left\\photos");
        assert_eq!(panes.active_path(), "C:\\left\\photos");
        assert_eq!(panes.other_path(), "C:\\docs");
    }

    #[test]
    fn toggle_switches_the_active_pane_without_changing_paths() {
        let mut panes = Panes::start("C:\\docs");
        panes.set_active_path("C:\\left");

        panes.toggle();
        assert_eq!(panes.active(), PaneId::Right);
        assert_eq!(panes.active_path(), "C:\\docs");
        assert_eq!(panes.other_path(), "C:\\left");

        panes.set_active_path("C:\\right");
        assert_eq!(panes.active(), PaneId::Right);
        assert_eq!(panes.active_path(), "C:\\right");
        assert_eq!(panes.other_path(), "C:\\left");

        panes.toggle();
        assert_eq!(panes.active(), PaneId::Left);
        assert_eq!(panes.active_path(), "C:\\left");
        assert_eq!(panes.other_path(), "C:\\right");
    }

    #[test]
    fn set_path_can_change_the_inactive_pane() {
        let mut panes = Panes::start("C:\\docs");
        panes.set_path(PaneId::Right, "C:\\right");
        assert_eq!(panes.active(), PaneId::Left);
        assert_eq!(panes.active_path(), "C:\\docs");
        assert_eq!(panes.other_path(), "C:\\right");
    }

    #[test]
    fn activate_switches_focus_without_changing_paths() {
        let mut panes = Panes::start("C:\\docs");
        panes.set_active_path("C:\\left");
        panes.activate(PaneId::Right);
        assert_eq!(panes.active(), PaneId::Right);
        assert_eq!(panes.active_path(), "C:\\docs");
        assert_eq!(panes.other_path(), "C:\\left");
        panes.activate(PaneId::Right);
        assert_eq!(panes.active(), PaneId::Right);
        assert_eq!(panes.active_path(), "C:\\docs");
    }

    #[test]
    fn toggle_twice_returns_to_the_same_pane() {
        let mut panes = Panes::start("C:\\docs");
        panes.set_active_path("C:\\left");
        panes.toggle();
        panes.set_active_path("C:\\right");
        panes.toggle();
        panes.toggle();
        assert_eq!(panes.active(), PaneId::Right);
        assert_eq!(panes.active_path(), "C:\\right");
        assert_eq!(panes.other_path(), "C:\\left");
    }
}
