#![cfg_attr(not(windows), allow(dead_code))]
//! Whether a tree child would repeat a directory already on its own branch.
//!
//! This only compares directory strings with `nav::same_directory`. It does
//! not read the disk and does not follow reparse points, so a junction that
//! points back at an ancestor under a different name (`C:\a\loop` targeting
//! `C:\a`) is not caught here. That needs the target's file identity, which
//! is an open design question.

use crate::nav::same_directory;

/// True when `child` names the same directory as any path in `lineage`.
///
/// `lineage` is the branch from the root down to the node being expanded,
/// in any order. `C:\` equals `C:` and slashes are normalized. An empty
/// child is not a repeat.
pub fn repeats_lineage(lineage: &[impl AsRef<str>], child: &str) -> bool {
    if child.is_empty() {
        return false;
    }
    lineage
        .iter()
        .any(|directory| same_directory(directory.as_ref(), child))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: &[&str] = &[];

    #[test]
    fn child_equal_to_the_node_repeats() {
        assert!(repeats_lineage(&[r"C:\a"], r"C:\a"));
    }

    #[test]
    fn child_equal_to_an_ancestor_repeats() {
        let lineage = [r"C:\", r"C:\projects", r"C:\projects\ffe"];
        assert!(repeats_lineage(&lineage, r"C:\projects"));
    }

    #[test]
    fn a_descendant_is_not_the_same_directory() {
        assert!(!repeats_lineage(&[r"C:\a"], r"C:\a\b"));
    }

    #[test]
    fn comparison_matches_same_directory_case_sensitivity() {
        assert!(!same_directory(r"C:\A", r"C:\a"));
        assert!(!repeats_lineage(&[r"C:\a", r"C:\other"], r"C:\A"));
        assert!(!repeats_lineage(&[r"C:\a", r"C:\other"], "C:/A"));
        assert!(repeats_lineage(&[r"C:\a"], "C:/a/"));
    }

    #[test]
    fn drive_root_with_and_without_a_slash_repeats() {
        assert!(repeats_lineage(&[r"C:\"], "C:"));
        assert!(repeats_lineage(&["C:", r"C:\Windows"], r"C:\"));
    }

    #[test]
    fn an_empty_child_is_not_a_repeat() {
        assert!(!repeats_lineage(&[r"C:\a"], ""));
        assert!(!repeats_lineage(NONE, ""));
    }

    #[test]
    fn an_empty_lineage_never_repeats() {
        assert!(!repeats_lineage(NONE, r"C:\a"));
    }

    #[test]
    fn an_unrelated_path_does_not_repeat() {
        assert!(!repeats_lineage(&[r"C:\a", r"C:\a\b"], r"D:\other"));
    }

    #[test]
    fn a_junction_is_only_a_path_compare() {
        let lineage = [r"C:\real", r"C:\junction", r"C:\real\node"];
        assert!(repeats_lineage(&lineage, r"C:\junction"));
        assert!(!repeats_lineage(&lineage, r"C:\junction\target"));
    }
}
