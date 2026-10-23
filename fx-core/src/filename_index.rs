//! In-memory filename index for Rapid Explorer.
//!
//! [`FilenameIndex`] is the store. The Win32 USN reader is a later wiring.
//! Building a record or applying a journal does not read the disk. This file
//! contains no Windows USN code.

use std::collections::HashMap;
use std::time::SystemTime;

/// One indexed file or directory.
///
/// The caller supplies every field. Constructing a record does not read the disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexRecord {
    pub path: String,
    pub name: String,
    pub volume_key: String,
    pub is_directory: bool,
    pub size: u64,
    pub modified: SystemTime,
}

/// One change to apply to a [`FilenameIndex`].
///
/// These values are data. Applying them does not read the disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JournalEvent {
    Upsert(IndexRecord),
    Delete { path: String },
}

/// Filename records keyed by exact path.
///
/// Eviction: TODO — define. This index has no eviction cap.
#[derive(Debug, Clone, Default)]
pub struct FilenameIndex {
    records: HashMap<String, IndexRecord>,
}

impl FilenameIndex {
    pub fn new() -> Self {
        Self {
            records: HashMap::new(),
        }
    }

    /// Insert or replace the record stored at `record.path`. The path is matched exactly.
    pub fn upsert(&mut self, record: IndexRecord) {
        self.records.insert(record.path.clone(), record);
    }

    /// Remove the record stored at `path`. Returns whether a record was present.
    pub fn remove(&mut self, path: &str) -> bool {
        self.records.remove(path).is_some()
    }

    /// The record stored at `path`, if any.
    pub fn get(&self, path: &str) -> Option<&IndexRecord> {
        self.records.get(path)
    }

    /// Every stored record. Iteration order is not significant.
    pub fn iter(&self) -> impl Iterator<Item = &IndexRecord> {
        self.records.values()
    }

    /// Apply journal events in iterator order. A later event wins for the same path.
    ///
    /// This does not read the disk.
    /// Records whose names match `query`, best match first.
    ///
    /// Ties keep path order. An empty query matches nothing. The score is
    /// match quality only. Frecency is not applied. This does not read the disk
    /// and does not search outside the records already stored.
    pub fn search(&self, query: &str) -> Vec<&IndexRecord> {
        let mut records: Vec<&IndexRecord> = self.records.values().collect();
        records.sort_by(|left, right| left.path.cmp(&right.path));
        crate::rank::rank_by(query, records, |record| record.name.as_str())
    }

    pub fn apply_journal(&mut self, records: impl IntoIterator<Item = JournalEvent>) {
        for event in records {
            match event {
                JournalEvent::Upsert(record) => self.upsert(record),
                JournalEvent::Delete { path } => {
                    self.remove(&path);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(path: &str, name: &str, size: u64) -> IndexRecord {
        IndexRecord {
            path: path.to_string(),
            name: name.to_string(),
            volume_key: "C:".to_string(),
            is_directory: false,
            size,
            modified: SystemTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn upsert_replaces_the_same_path() {
        let mut index = FilenameIndex::new();
        index.upsert(record("C:\\data\\a.txt", "a.txt", 1));

        let mut replaced = record("C:\\data\\a.txt", "a-renamed.txt", 9);
        replaced.volume_key = "D:".to_string();
        replaced.is_directory = true;
        replaced.modified = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(5);
        index.upsert(replaced.clone());

        assert_eq!(index.get("C:\\data\\a.txt"), Some(&replaced));
        assert_eq!(index.iter().count(), 1);
    }

    #[test]
    fn remove_deletes_the_path() {
        let mut index = FilenameIndex::new();
        index.upsert(record("C:\\data\\a.txt", "a.txt", 1));
        index.upsert(record("C:\\data\\b.txt", "b.txt", 2));

        assert!(index.remove("C:\\data\\a.txt"));
        assert!(index.get("C:\\data\\a.txt").is_none());
        assert_eq!(
            index
                .get("C:\\data\\b.txt")
                .map(|record| record.name.as_str()),
            Some("b.txt")
        );
        assert!(!index.remove("C:\\data\\a.txt"));
    }

    #[test]
    fn iter_returns_stored_records_without_asserting_order() {
        let mut index = FilenameIndex::default();
        index.upsert(record("C:\\data\\b.txt", "same.txt", 2));
        index.upsert(record("C:\\data\\a.txt", "same.txt", 1));

        let mut paths: Vec<String> = index.iter().map(|record| record.path.clone()).collect();
        paths.sort();
        assert_eq!(
            paths,
            vec!["C:\\data\\a.txt".to_string(), "C:\\data\\b.txt".to_string()]
        );
    }

    #[test]
    fn journal_applies_in_order_so_a_delete_after_an_upsert_wins() {
        let mut index = FilenameIndex::new();
        index.apply_journal([
            JournalEvent::Upsert(record("C:\\data\\gone.txt", "first", 1)),
            JournalEvent::Upsert(record("C:\\data\\gone.txt", "second", 2)),
            JournalEvent::Delete {
                path: "C:\\data\\gone.txt".to_string(),
            },
            JournalEvent::Delete {
                path: "C:\\data\\revived.txt".to_string(),
            },
            JournalEvent::Upsert(record("C:\\data\\revived.txt", "back", 3)),
            JournalEvent::Delete {
                path: "C:\\missing.txt".to_string(),
            },
        ]);

        assert!(index.get("C:\\data\\gone.txt").is_none());
        assert_eq!(
            index.get("C:\\data\\revived.txt").map(|record| record.size),
            Some(3)
        );
        assert!(index.get("C:\\missing.txt").is_none());
    }

    #[test]
    fn search_ranks_match_quality_and_skips_an_empty_query() {
        let mut index = FilenameIndex::new();
        index.upsert(record("C:\\data\\zzreport.txt", "zzreport.txt", 1));
        index.upsert(record("C:\\data\\report.txt", "report.txt", 2));
        index.upsert(record("C:\\data\\r_e_port.txt", "r_e_port.txt", 3));
        index.upsert(record("C:\\other\\notes.txt", "notes.txt", 4));

        assert!(index.search("").is_empty());
        let found: Vec<&str> = index
            .search("report")
            .into_iter()
            .map(|record| record.path.as_str())
            .collect();
        assert_eq!(
            found,
            vec![
                "C:\\data\\report.txt",
                "C:\\data\\zzreport.txt",
                "C:\\data\\r_e_port.txt",
            ]
        );
    }

    #[test]
    fn equal_names_keep_path_order() {
        let mut index = FilenameIndex::new();
        index.upsert(record("C:\\b\\notes.txt", "notes.txt", 1));
        index.upsert(record("C:\\a\\notes.txt", "notes.txt", 1));
        let found: Vec<&str> = index
            .search("notes")
            .into_iter()
            .map(|record| record.path.as_str())
            .collect();
        assert_eq!(found, vec!["C:\\a\\notes.txt", "C:\\b\\notes.txt"]);
    }
}
