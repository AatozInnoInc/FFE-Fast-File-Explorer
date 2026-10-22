#![cfg_attr(not(windows), allow(dead_code))]
//! One icon per directory, and one icon per file extension.
//!
//! Callers use [`IconCache::get_or_insert_with`] so a lookup runs once per
//! key. This module does not load icons, and the cache does not evict.

use std::collections::HashMap;

/// Key for one cached icon.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IconKey {
    Directory,
    /// Lowercase extension without the dot. `Report.TXT` and `a.txt` share `txt`.
    Extension(String),
    /// A file with no extension.
    File,
}

/// Icon key for `name`.
///
/// A directory is [`IconKey::Directory`] even when the name contains a dot.
/// The extension is the text after the last `.`, lowercased. `.gitignore`,
/// `Makefile`, an empty name, and a trailing dot are [`IconKey::File`].
pub fn icon_key(name: &str, is_directory: bool) -> IconKey {
    if is_directory {
        return IconKey::Directory;
    }
    match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() && !extension.is_empty() => {
            IconKey::Extension(extension.to_lowercase())
        }
        _ => IconKey::File,
    }
}

/// Icons stored by [`IconKey`]. Each key is inserted at most once.
pub struct IconCache<T> {
    icons: HashMap<IconKey, T>,
}

impl<T> IconCache<T> {
    pub fn new() -> Self {
        Self {
            icons: HashMap::new(),
        }
    }

    pub fn get(&self, key: &IconKey) -> Option<&T> {
        self.icons.get(key)
    }

    /// Return the stored value, calling `make` only when `key` is absent.
    pub fn get_or_insert_with(&mut self, key: IconKey, make: impl FnOnce() -> T) -> &T {
        self.icons.entry(key).or_insert_with(make)
    }

    #[allow(dead_code, clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.icons.len()
    }
}

impl<T> Default for IconCache<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directories_ignore_extensions() {
        assert_eq!(icon_key("Photos", true), IconKey::Directory);
        assert_eq!(icon_key("Photos.jpg", true), IconKey::Directory);
        assert_eq!(icon_key("archive.tar.gz", true), IconKey::Directory);
        assert_eq!(icon_key(".gitignore", true), IconKey::Directory);
        assert_eq!(icon_key("", true), IconKey::Directory);
    }

    #[test]
    fn report_txt_and_notes_txt_are_the_same_extension() {
        let report = icon_key("Report.TXT", false);
        let notes = icon_key("notes.txt", false);
        assert_eq!(report, IconKey::Extension("txt".to_string()));
        assert_eq!(notes, report);
        assert_eq!(icon_key("a.txt", false), report);
    }

    #[test]
    fn dotfiles_and_extensionless_files_are_file() {
        assert_eq!(icon_key(".gitignore", false), IconKey::File);
        assert_eq!(icon_key("Makefile", false), IconKey::File);
        assert_eq!(icon_key("", false), IconKey::File);
        assert_eq!(icon_key("foo.", false), IconKey::File);
    }

    #[test]
    fn tar_gz_uses_gz() {
        assert_eq!(
            icon_key("archive.tar.gz", false),
            IconKey::Extension("gz".to_string())
        );
    }

    #[test]
    fn cache_calls_the_factory_once_for_a_repeated_key() {
        let mut cache = IconCache::new();
        let mut calls = 0;
        let first = *cache.get_or_insert_with(IconKey::Extension("txt".to_string()), || {
            calls += 1;
            7
        });
        let second = *cache.get_or_insert_with(IconKey::Extension("txt".to_string()), || {
            calls += 1;
            9
        });
        assert_eq!(first, 7);
        assert_eq!(second, 7);
        assert_eq!(calls, 1);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get(&IconKey::Extension("txt".to_string())), Some(&7));
    }

    #[test]
    fn directory_and_extension_txt_are_different_entries() {
        let mut cache = IconCache::default();
        cache.get_or_insert_with(IconKey::Directory, || "dir");
        cache.get_or_insert_with(IconKey::Extension("txt".to_string()), || "text");
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get(&IconKey::Directory), Some(&"dir"));
        assert_eq!(
            cache.get(&IconKey::Extension("txt".to_string())),
            Some(&"text")
        );
        assert_eq!(cache.get(&IconKey::File), None);
    }
}
