#![cfg_attr(not(windows), allow(dead_code))]
//! Native context-menu commands for a file list.
//!
//! Nothing here calls Win32. [`menu_for`] is the commands for the current
//! selection. The window performs them. [`Command::MoreOptions`] is the full
//! shell menu, which this module does not build.

/// A command the native menu can offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Copy,
    Cut,
    Paste,
    Rename,
    Delete,
    Properties,
    /// The full shell menu. This module does not build it.
    MoreOptions,
}

/// Commands for `selected` items.
///
/// Nothing selected offers Paste only when the clipboard already holds paths.
/// One item offers Copy, Cut, Paste when the clipboard holds paths, Rename,
/// Delete, Properties, and More options. Two or more offer Copy, Cut, Paste
/// when the clipboard holds paths, Delete, and More options.
pub fn menu_for(selected: usize, clipboard_has_paths: bool) -> Vec<Command> {
    if selected == 0 {
        return if clipboard_has_paths {
            vec![Command::Paste]
        } else {
            Vec::new()
        };
    }

    let mut commands = vec![Command::Copy, Command::Cut];
    if clipboard_has_paths {
        commands.push(Command::Paste);
    }
    if selected == 1 {
        commands.push(Command::Rename);
    }
    commands.push(Command::Delete);
    if selected == 1 {
        commands.push(Command::Properties);
    }
    commands.push(Command::MoreOptions);
    commands
}

pub fn label(command: Command) -> &'static str {
    match command {
        Command::Copy => "Copy",
        Command::Cut => "Cut",
        Command::Paste => "Paste",
        Command::Rename => "Rename",
        Command::Delete => "Delete",
        Command::Properties => "Properties",
        Command::MoreOptions => "More options",
    }
}

/// A new file name in the same directory. Empty, `.`, `..`, the current name,
/// and names with reserved characters are refused. This does not touch disk.
pub fn accepted_rename(old_name: &str, new_name: &str) -> Option<String> {
    let trimmed = new_name.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." || trimmed == old_name {
        return None;
    }
    if trimmed
        .chars()
        .any(|ch| matches!(ch, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
    {
        return None;
    }
    Some(trimmed.to_string())
}

/// Double-null UTF-16 path list used by the clipboard and `SHFileOperationW`.
pub fn double_null(paths: &[impl AsRef<str>]) -> Vec<u16> {
    let mut units = Vec::new();
    for path in paths {
        units.extend(path.as_ref().encode_utf16());
        units.push(0);
    }
    units.push(0);
    units
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_selected_without_clipboard_is_empty() {
        assert!(menu_for(0, false).is_empty());
    }

    #[test]
    fn nothing_selected_with_clipboard_is_paste() {
        assert_eq!(menu_for(0, true), vec![Command::Paste]);
    }

    #[test]
    fn one_selected_without_clipboard() {
        assert_eq!(
            menu_for(1, false),
            vec![
                Command::Copy,
                Command::Cut,
                Command::Rename,
                Command::Delete,
                Command::Properties,
                Command::MoreOptions,
            ]
        );
    }

    #[test]
    fn one_selected_with_clipboard_inserts_paste() {
        assert_eq!(
            menu_for(1, true),
            vec![
                Command::Copy,
                Command::Cut,
                Command::Paste,
                Command::Rename,
                Command::Delete,
                Command::Properties,
                Command::MoreOptions,
            ]
        );
    }

    #[test]
    fn many_selected_omits_rename_and_properties() {
        let expected = vec![
            Command::Copy,
            Command::Cut,
            Command::Delete,
            Command::MoreOptions,
        ];
        assert_eq!(menu_for(2, false), expected);
        assert_eq!(menu_for(5, false), expected);
    }

    #[test]
    fn many_selected_with_clipboard_inserts_paste() {
        let expected = vec![
            Command::Copy,
            Command::Cut,
            Command::Paste,
            Command::Delete,
            Command::MoreOptions,
        ];
        assert_eq!(menu_for(2, true), expected);
        assert_eq!(menu_for(8, true), expected);
    }

    #[test]
    fn rename_refuses_empty_unchanged_and_reserved_characters() {
        assert_eq!(
            accepted_rename("a.txt", "  b.txt  ").as_deref(),
            Some("b.txt")
        );
        assert_eq!(accepted_rename("a.txt", "a.txt"), None);
        assert_eq!(accepted_rename("a.txt", "  "), None);
        assert_eq!(accepted_rename("a.txt", "."), None);
        assert_eq!(accepted_rename("a.txt", ".."), None);
        assert_eq!(accepted_rename("a.txt", "a/b"), None);
        assert_eq!(accepted_rename("a.txt", "a:b"), None);
    }

    #[test]
    fn double_null_ends_the_list_twice() {
        assert_eq!(double_null(&["a"]), vec!['a' as u16, 0, 0]);
        let two = double_null(&["ab", "c"]);
        assert_eq!(two, vec!['a' as u16, 'b' as u16, 0, 'c' as u16, 0, 0]);
    }
}
