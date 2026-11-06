//! Whether expanding a tree node would repeat an ancestor path.
//!
//! This only compares directory strings. It does not read the disk and does not
//! follow reparse points.

/// True when `child` names the same directory as `node` or any path in `ancestors`.
/// Uses `nav::same_directory` (`C:\` equals `C:`, slashes are normalized).
/// An empty child is not a loop. This does not read the disk and does not follow links.
pub fn repeats_ancestor(ancestors: &[impl AsRef<str>], node: &str, child: &str) -> bool {
    if child.is_empty() {
        return false;
    }
    if super::nav::same_directory(node, child) {
        return true;
    }
    ancestors
        .iter()
        .any(|ancestor| super::nav::same_directory(ancestor.as_ref(), child))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_equal_to_the_node_repeats() {
        assert!(repeats_ancestor(&[] as &[&str], r"C:\a", r"C:\a"));
    }

    #[test]
    fn child_equal_to_an_ancestor_repeats() {
        let ancestors = [r"C:\", r"C:\projects"];
        assert!(repeats_ancestor(
            &ancestors,
            r"C:\projects\ffe",
            r"C:\projects"
        ));
    }

    #[test]
    fn a_descendant_is_not_the_same_directory() {
        assert!(!repeats_ancestor(&[r"C:\a"], r"C:\a", r"C:\a\b"));
    }

    #[test]
    fn comparison_matches_same_directory_case_sensitivity() {
        assert!(!super::super::nav::same_directory(r"C:\A", r"C:\a"));
        assert!(!repeats_ancestor(&[r"C:\a"], r"C:\other", r"C:\A"));
        assert!(!repeats_ancestor(&[r"C:\a"], r"C:\other", "C:/A"));
        assert!(repeats_ancestor(&[] as &[&str], r"C:\a", "C:/a/"));
    }

    #[test]
    fn drive_root_with_and_without_a_slash_repeats() {
        assert!(repeats_ancestor(&[] as &[&str], r"C:\", "C:"));
        assert!(repeats_ancestor(&["C:"], r"C:\Windows", r"C:\"));
    }

    #[test]
    fn an_empty_child_is_not_a_loop() {
        assert!(!repeats_ancestor(&[r"C:\a"], r"C:\a", ""));
        assert!(!repeats_ancestor(&[] as &[&str], "", ""));
    }

    #[test]
    fn an_unrelated_path_does_not_repeat() {
        assert!(!repeats_ancestor(
            &[r"C:\a", r"C:\a\b"],
            r"C:\a\b",
            r"D:\other"
        ));
    }

    #[test]
    fn a_junction_looking_name_is_only_a_path_compare() {
        let ancestors = [r"C:\real", r"C:\junction"];
        assert!(repeats_ancestor(
            &ancestors,
            r"C:\real\node",
            r"C:\junction"
        ));
        assert!(!repeats_ancestor(
            &ancestors,
            r"C:\real\node",
            r"C:\junction\target"
        ));
    }
}
