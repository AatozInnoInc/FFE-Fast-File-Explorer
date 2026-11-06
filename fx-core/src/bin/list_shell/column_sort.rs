//! Sort order for rows the caller already has.
//!
//! Directories and files are one sequence. Equal keys keep their existing
//! order. This module does not read the filesystem, an index, or frecency.

use fx_core::FileEntry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortColumn {
    Name,
    Size,
    Modified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDir {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowSort {
    pub column: SortColumn,
    pub dir: SortDir,
}

impl RowSort {
    /// First click: Name ascending. Same column toggles direction. A different column starts ascending.
    pub fn header_click(current: Option<RowSort>, column: SortColumn) -> RowSort {
        match current {
            Some(current) if current.column == column => RowSort {
                column,
                dir: match current.dir {
                    SortDir::Asc => SortDir::Desc,
                    SortDir::Desc => SortDir::Asc,
                },
            },
            _ => RowSort {
                column,
                dir: SortDir::Asc,
            },
        }
    }
}

/// Case-insensitive name: compare `name.to_lowercase()`. Size compares `size` as u64. Modified compares `last_write`.
pub fn sort_entries(entries: &mut [FileEntry], sort: RowSort) {
    entries.sort_by(|left, right| {
        let ordering = match sort.column {
            SortColumn::Name => left.name.to_lowercase().cmp(&right.name.to_lowercase()),
            SortColumn::Size => left.size.cmp(&right.size),
            SortColumn::Modified => left.last_write.cmp(&right.last_write),
        };
        match sort.dir {
            SortDir::Asc => ordering,
            SortDir::Desc => ordering.reverse(),
        }
    });
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::*;

    fn entry(name: &str, size: u64, directory: bool, modified: SystemTime) -> FileEntry {
        FileEntry {
            name: name.to_string(),
            attributes: 0,
            size,
            created: modified,
            last_write: modified,
            is_directory: directory,
            is_reparse_point: false,
        }
    }

    fn names(entries: &[FileEntry]) -> Vec<&str> {
        entries.iter().map(|entry| entry.name.as_str()).collect()
    }

    fn by(column: SortColumn, dir: SortDir) -> RowSort {
        RowSort { column, dir }
    }

    #[test]
    fn header_click_starts_on_name_then_toggles_and_switches_column() {
        let name = RowSort::header_click(None, SortColumn::Name);
        assert_eq!(name, by(SortColumn::Name, SortDir::Asc));
        let name_desc = RowSort::header_click(Some(name), SortColumn::Name);
        assert_eq!(name_desc, by(SortColumn::Name, SortDir::Desc));
        assert_eq!(
            RowSort::header_click(Some(name_desc), SortColumn::Size),
            by(SortColumn::Size, SortDir::Asc)
        );
    }

    #[test]
    fn name_sort_is_case_insensitive() {
        let epoch = SystemTime::UNIX_EPOCH;
        let mut entries = vec![entry("b", 1, false, epoch), entry("A", 2, false, epoch)];
        sort_entries(&mut entries, by(SortColumn::Name, SortDir::Asc));
        assert_eq!(names(&entries), ["A", "b"]);
    }

    #[test]
    fn equal_names_keep_input_order() {
        let epoch = SystemTime::UNIX_EPOCH;
        let mut entries = vec![
            entry("a", 2, false, epoch),
            entry("a", 9, false, epoch),
            entry("b", 1, false, epoch),
        ];
        sort_entries(&mut entries, by(SortColumn::Name, SortDir::Asc));
        assert_eq!(names(&entries), ["a", "a", "b"]);
        assert_eq!(entries[0].size, 2);
        assert_eq!(entries[1].size, 9);
    }

    #[test]
    fn size_sort_is_numeric() {
        let epoch = SystemTime::UNIX_EPOCH;
        let mut entries = vec![
            entry("ten", 10, false, epoch),
            entry("two", 2, false, epoch),
            entry("hundred", 100, false, epoch),
        ];
        sort_entries(&mut entries, by(SortColumn::Size, SortDir::Asc));
        assert_eq!(names(&entries), ["two", "ten", "hundred"]);
    }

    #[test]
    fn descending_size_reverses() {
        let epoch = SystemTime::UNIX_EPOCH;
        let mut entries = vec![
            entry("mid-a", 5, false, epoch),
            entry("low", 1, false, epoch),
            entry("mid-b", 5, false, epoch),
            entry("high", 9, false, epoch),
        ];
        sort_entries(&mut entries, by(SortColumn::Size, SortDir::Desc));
        assert_eq!(names(&entries), ["high", "mid-a", "mid-b", "low"]);
    }

    #[test]
    fn modified_uses_system_time_order() {
        let minute = SystemTime::UNIX_EPOCH + Duration::from_secs(1_609_459_200);
        let later_same_minute = minute + Duration::from_secs(30);
        let mut entries = vec![
            entry("later", 1, true, later_same_minute),
            entry("earlier", 50, false, minute),
        ];
        sort_entries(&mut entries, by(SortColumn::Modified, SortDir::Asc));
        assert_eq!(names(&entries), ["earlier", "later"]);
        sort_entries(&mut entries, by(SortColumn::Modified, SortDir::Desc));
        assert_eq!(names(&entries), ["later", "earlier"]);
    }

    #[test]
    fn directory_with_size_zero_sorts_by_that_size() {
        let epoch = SystemTime::UNIX_EPOCH;
        let mut entries = vec![
            entry("dir-big", 100, true, epoch),
            entry("file-zero", 0, false, epoch),
            entry("dir-zero", 0, true, epoch),
        ];
        sort_entries(&mut entries, by(SortColumn::Size, SortDir::Asc));
        assert_eq!(names(&entries), ["file-zero", "dir-zero", "dir-big"]);
        assert!(entries[1].is_directory);
        assert_eq!(entries[1].size, 0);
    }

    #[test]
    fn sort_does_not_read_the_filesystem() {
        let epoch = SystemTime::UNIX_EPOCH;
        let mut entries = vec![entry("b", 2, true, epoch), entry("A", 1, false, epoch)];
        sort_entries(&mut entries, by(SortColumn::Name, SortDir::Asc));
        assert_eq!(names(&entries), ["A", "b"]);
        assert!(entries[1].is_directory);
    }
}
