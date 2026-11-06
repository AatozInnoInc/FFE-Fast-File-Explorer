//! Parent of a directory path.
//!
//! Nothing here reads the disk. `/` becomes `\`, and a trailing separator is
//! not another component. Drive roots and UNC share roots have no parent.
//! A `\\?\` or `\\?\UNC\` prefix stays on the parent. `..` is not invented
//! and reparse points are not followed.

/// Parent directory of `path`. None for empty, a drive root, or a UNC share root.
/// Does not touch the disk. `/` is treated as `\`.
pub fn parent_directory(path: &str) -> Option<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Same separator rules as `nav::comparable_path`: `/` becomes `\`, then
    // trailing `\` are removed so `C:\` is the drive root `C:`.
    let normalized = normalize_separators(trimmed);
    if normalized.is_empty() || !has_parent(&normalized) {
        return None;
    }
    let parent = normalized.rsplit_once('\\')?.0.trim_end_matches('\\');
    if parent.is_empty() {
        return None;
    }
    // A child of a drive root is written with the root slash (`C:\`, `\\?\C:\`).
    if is_drive_root(parent) {
        Some(format!("{parent}\\"))
    } else {
        Some(parent.to_string())
    }
}

/// `/` becomes `\`, then trailing `\` are removed.
fn normalize_separators(path: &str) -> String {
    let mut normalized = path.replace('/', "\\");
    let end = normalized.trim_end_matches('\\').len();
    normalized.truncate(end);
    normalized
}

fn has_parent(path: &str) -> bool {
    if is_drive_root(path) || is_unc_share_root(path) {
        return false;
    }
    // `\\?\foo` is one component inside the prefix, so it has no parent.
    if let Some(body) = path.strip_prefix(r"\\?\") {
        return body.contains('\\');
    }
    path.contains('\\')
}

/// `C:` and `\\?\C:` are drive roots. Letter case is left as written.
fn is_drive_root(path: &str) -> bool {
    let body = path.strip_prefix(r"\\?\").unwrap_or(path);
    let mut chars = body.chars();
    let Some(letter) = chars.next() else {
        return false;
    };
    letter.is_ascii_alphabetic() && chars.next() == Some(':') && chars.next().is_none()
}

/// Server plus share, with or without a `\\?\UNC\` prefix. Empty segments are
/// ignored, matching `unc_volume_key`.
fn is_unc_share_root(path: &str) -> bool {
    let Some(body) = unc_body(path) else {
        return false;
    };
    body.split('\\').filter(|part| !part.is_empty()).count() < 3
}

fn unc_body(path: &str) -> Option<&str> {
    if let Some(body) = path.strip_prefix(r"\\?\UNC\") {
        return Some(body);
    }
    if path.starts_with(r"\\?\") {
        return None;
    }
    path.strip_prefix(r"\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_whitespace_have_no_parent() {
        assert_eq!(parent_directory(""), None);
        assert_eq!(parent_directory("   "), None);
        assert_eq!(parent_directory("\t\n"), None);
    }

    #[test]
    fn a_drive_root_has_no_parent() {
        assert_eq!(parent_directory(r"C:\"), None);
        assert_eq!(parent_directory("C:"), None);
        assert_eq!(parent_directory("C:/"), None);
    }

    #[test]
    fn a_child_of_a_drive_root_keeps_the_root_slash() {
        assert_eq!(parent_directory(r"C:\Windows"), Some(r"C:\".to_string()));
    }

    #[test]
    fn forward_slashes_become_backslashes() {
        assert_eq!(
            parent_directory("C:/Windows/System32"),
            Some(r"C:\Windows".to_string())
        );
    }

    #[test]
    fn a_trailing_separator_is_not_a_component() {
        assert_eq!(
            parent_directory(r"C:\Windows\System32\"),
            Some(r"C:\Windows".to_string())
        );
        assert_eq!(
            parent_directory(r"C:\Windows\System32\\"),
            Some(r"C:\Windows".to_string())
        );
    }

    #[test]
    fn surrounding_whitespace_is_trimmed_and_interior_spaces_stay() {
        assert_eq!(
            parent_directory("  C:\\Program Files\\Editor  "),
            Some(r"C:\Program Files".to_string())
        );
    }

    #[test]
    fn a_unc_share_root_has_no_parent() {
        assert_eq!(parent_directory(r"\\server\share"), None);
        assert_eq!(parent_directory(r"\\server\share\"), None);
        assert_eq!(parent_directory("//server/share"), None);
    }

    #[test]
    fn a_unc_child_stops_at_the_share() {
        assert_eq!(
            parent_directory(r"\\server\share\docs"),
            Some(r"\\server\share".to_string())
        );
        assert_eq!(
            parent_directory("//server/share/docs"),
            Some(r"\\server\share".to_string())
        );
    }

    #[test]
    fn a_deeper_unc_path_drops_one_component() {
        assert_eq!(
            parent_directory(r"\\server\share\docs\a"),
            Some(r"\\server\share\docs".to_string())
        );
    }

    #[test]
    fn an_extended_drive_root_has_no_parent() {
        assert_eq!(parent_directory(r"\\?\C:\"), None);
        assert_eq!(parent_directory(r"\\?\C:"), None);
    }

    #[test]
    fn an_extended_drive_child_keeps_the_prefix_and_root_slash() {
        assert_eq!(
            parent_directory(r"\\?\C:\Windows"),
            Some(r"\\?\C:\".to_string())
        );
        assert_eq!(
            parent_directory("//?/C:/Windows"),
            Some(r"\\?\C:\".to_string())
        );
    }

    #[test]
    fn a_deeper_extended_drive_path_keeps_the_prefix() {
        assert_eq!(
            parent_directory(r"\\?\C:\Windows\System32"),
            Some(r"\\?\C:\Windows".to_string())
        );
    }

    #[test]
    fn an_extended_unc_share_root_has_no_parent() {
        assert_eq!(parent_directory(r"\\?\UNC\server\share"), None);
        assert_eq!(parent_directory(r"\\?\UNC\server\share\"), None);
    }

    #[test]
    fn an_extended_unc_child_keeps_the_prefix() {
        assert_eq!(
            parent_directory(r"\\?\UNC\server\share\docs"),
            Some(r"\\?\UNC\server\share".to_string())
        );
        assert_eq!(
            parent_directory("//?/UNC/server/share/docs"),
            Some(r"\\?\UNC\server\share".to_string())
        );
    }

    #[test]
    fn a_single_name_has_no_parent() {
        assert_eq!(parent_directory("readme.txt"), None);
        assert_eq!(parent_directory("readme.txt/"), None);
    }

    #[test]
    fn a_relative_path_drops_one_component() {
        assert_eq!(
            parent_directory(r"docs\readme.txt"),
            Some("docs".to_string())
        );
    }

    #[test]
    fn dotdot_is_not_resolved() {
        assert_eq!(
            parent_directory(r"C:\Windows\..\System32"),
            Some(r"C:\Windows\..".to_string())
        );
    }
}
