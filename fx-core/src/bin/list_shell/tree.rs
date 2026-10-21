#![cfg_attr(not(windows), allow(dead_code))]
//! Direct child directories for one expanded tree node.
//!
//! `SysTreeView32` has no owner-data mode, so each name here is a child the
//! window may insert. Nothing in this module opens a grandchild. A cache hit
//! exposes its directory names before any listing event is applied.

use fx_core::{FileEntry, ListingEvent, TryEvent};

/// How the tree control should change the children of one expanded node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChildChange {
    /// Directory names that were not inserted yet, in listing order.
    Append(Vec<String>),
    /// The full set of direct child directories, replacing whatever is shown.
    Replace(Vec<String>),
    Unchanged,
}

pub struct Expansion {
    shown: Vec<String>,
    incoming: Vec<String>,
    from_cache: bool,
    settled: bool,
}

impl Expansion {
    /// `cached` is what [`fx_core::Shell::open`] returned for this directory.
    /// This does not poll. Files are not children of the folder tree.
    pub fn start(cached: Option<&[FileEntry]>) -> (Self, ChildChange) {
        match cached {
            Some(entries) => {
                let shown = directory_names(entries);
                let change = if shown.is_empty() {
                    ChildChange::Unchanged
                } else {
                    ChildChange::Append(shown.clone())
                };
                (
                    Self {
                        shown,
                        incoming: Vec::new(),
                        from_cache: true,
                        settled: false,
                    },
                    change,
                )
            }
            None => (
                Self {
                    shown: Vec::new(),
                    incoming: Vec::new(),
                    from_cache: false,
                    settled: false,
                },
                ChildChange::Unchanged,
            ),
        }
    }

    #[cfg(test)]
    pub fn shown(&self) -> &[String] {
        &self.shown
    }

    pub fn is_settled(&self) -> bool {
        self.settled
    }

    /// Keep the directories already shown. Used when the live listing never starts.
    pub fn stop(&mut self) {
        self.settled = true;
        self.incoming.clear();
    }

    /// Drain `poll` until it would wait, or until the listing is finished.
    ///
    /// A finished listing stays ready forever. Stopping here keeps the UI thread
    /// from spinning on that event.
    pub fn apply_available<E>(
        &mut self,
        mut poll: impl FnMut() -> Result<TryEvent, E>,
    ) -> ChildChange {
        if self.settled {
            return ChildChange::Unchanged;
        }
        let mut change = ChildChange::Unchanged;
        loop {
            if self.settled {
                break;
            }
            match poll() {
                Ok(TryEvent::Pending) => break,
                Ok(TryEvent::Ready(event)) => change = combine(change, self.apply(event)),
                Err(_) => {
                    self.stop();
                    break;
                }
            }
        }
        change
    }

    pub fn apply(&mut self, event: ListingEvent) -> ChildChange {
        if self.settled {
            return ChildChange::Unchanged;
        }
        match event {
            ListingEvent::Batch(batch) => self.apply_batch(directory_names(&batch)),
            ListingEvent::Finished => self.finish(),
            ListingEvent::Cancelled | ListingEvent::TimedOut | ListingEvent::Failed(_) => {
                self.stop();
                ChildChange::Unchanged
            }
        }
    }

    fn apply_batch(&mut self, names: Vec<String>) -> ChildChange {
        if names.is_empty() {
            return ChildChange::Unchanged;
        }
        if self.from_cache {
            self.incoming.extend(names);
            ChildChange::Unchanged
        } else {
            self.shown.extend(names.iter().cloned());
            ChildChange::Append(names)
        }
    }

    fn finish(&mut self) -> ChildChange {
        self.settled = true;
        if !self.from_cache {
            return ChildChange::Unchanged;
        }
        if self.incoming == self.shown {
            self.incoming.clear();
            return ChildChange::Unchanged;
        }
        self.shown = std::mem::take(&mut self.incoming);
        ChildChange::Replace(self.shown.clone())
    }
}

/// Full path of a direct child. The parent string is not rewritten for the root
/// the user opened; only the child path uses backslashes.
pub fn child_directory(parent: &str, name: &str) -> String {
    let normalized = parent.replace('/', "\\");
    let trimmed = normalized.trim_end_matches('\\');
    format!("{trimmed}\\{name}")
}

fn directory_names(entries: &[FileEntry]) -> Vec<String> {
    entries
        .iter()
        .filter(|entry| entry.is_directory && !entry.name.is_empty())
        .map(|entry| entry.name.clone())
        .collect()
}

fn combine(left: ChildChange, right: ChildChange) -> ChildChange {
    match (right, left) {
        (ChildChange::Replace(names), _) => ChildChange::Replace(names),
        (ChildChange::Append(extra), ChildChange::Append(mut names)) => {
            names.extend(extra);
            ChildChange::Append(names)
        }
        (ChildChange::Append(names), ChildChange::Unchanged) => ChildChange::Append(names),
        (ChildChange::Unchanged, left) => left,
        (ChildChange::Append(_), ChildChange::Replace(names)) => ChildChange::Replace(names),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn entry(name: &str, is_directory: bool) -> FileEntry {
        FileEntry {
            name: name.to_string(),
            attributes: 0,
            size: 0,
            created: SystemTime::UNIX_EPOCH,
            last_write: SystemTime::UNIX_EPOCH,
            is_directory,
            is_reparse_point: false,
        }
    }

    #[test]
    fn cache_hit_lists_directories_before_any_event() {
        let cached = vec![
            entry("readme.txt", false),
            entry("src", true),
            entry("docs", true),
        ];
        let (expansion, change) = Expansion::start(Some(&cached));
        assert_eq!(
            change,
            ChildChange::Append(vec!["src".to_string(), "docs".to_string()])
        );
        assert_eq!(expansion.shown(), ["src", "docs"]);
        assert!(!expansion.is_settled());
    }

    #[test]
    fn cache_hit_keeps_children_until_a_different_listing_finishes() {
        let (mut expansion, _) = Expansion::start(Some(&[entry("src", true)]));
        assert_eq!(
            expansion.apply(ListingEvent::Batch(vec![
                entry("notes.txt", false),
                entry("src", true),
                entry("tests", true),
            ])),
            ChildChange::Unchanged
        );
        assert_eq!(expansion.shown(), ["src"]);
        assert_eq!(
            expansion.apply(ListingEvent::Finished),
            ChildChange::Replace(vec!["src".to_string(), "tests".to_string()])
        );
        assert_eq!(expansion.shown(), ["src", "tests"]);
        assert!(expansion.is_settled());
        assert_eq!(
            expansion.apply(ListingEvent::Finished),
            ChildChange::Unchanged
        );
    }

    #[test]
    fn cache_hit_unchanged_when_the_directories_match() {
        let (mut expansion, _) = Expansion::start(Some(&[entry("src", true)]));
        expansion.apply(ListingEvent::Batch(vec![
            entry("file", false),
            entry("src", true),
        ]));
        assert_eq!(
            expansion.apply(ListingEvent::Finished),
            ChildChange::Unchanged
        );
        assert_eq!(expansion.shown(), ["src"]);
    }

    #[test]
    fn cache_hit_timeout_keeps_the_cached_directories() {
        let (mut expansion, _) = Expansion::start(Some(&[entry("src", true)]));
        expansion.apply(ListingEvent::Batch(vec![entry("other", true)]));
        assert_eq!(
            expansion.apply(ListingEvent::TimedOut),
            ChildChange::Unchanged
        );
        assert_eq!(expansion.shown(), ["src"]);
        assert!(expansion.is_settled());
    }

    #[test]
    fn miss_appends_directories_as_batches_arrive() {
        let (mut expansion, change) = Expansion::start(None);
        assert_eq!(change, ChildChange::Unchanged);
        assert!(expansion.shown().is_empty());
        assert_eq!(
            expansion.apply(ListingEvent::Batch(vec![
                entry("a.txt", false),
                entry("src", true),
            ])),
            ChildChange::Append(vec!["src".to_string()])
        );
        assert_eq!(
            expansion.apply(ListingEvent::Batch(vec![entry("docs", true)])),
            ChildChange::Append(vec!["docs".to_string()])
        );
        assert_eq!(expansion.shown(), ["src", "docs"]);
        assert_eq!(
            expansion.apply(ListingEvent::Cancelled),
            ChildChange::Unchanged
        );
        assert_eq!(expansion.shown(), ["src", "docs"]);
    }

    #[test]
    fn apply_available_stops_after_a_terminal_event() {
        let (mut expansion, _) = Expansion::start(None);
        let mut polls = 0;
        let change = expansion.apply_available(|| {
            polls += 1;
            if polls == 1 {
                Ok::<_, ()>(TryEvent::Ready(ListingEvent::Batch(vec![entry(
                    "src", true,
                )])))
            } else if polls == 2 {
                Ok(TryEvent::Ready(ListingEvent::Finished))
            } else {
                panic!("polled after the listing finished");
            }
        });
        assert_eq!(change, ChildChange::Append(vec!["src".to_string()]));
        assert_eq!(polls, 2);
        assert!(expansion.is_settled());
    }

    #[test]
    fn apply_available_does_not_wait_and_keeps_a_cache_hit_when_poll_fails() {
        let (mut expansion, _) = Expansion::start(Some(&[entry("src", true)]));
        let mut polls = 0;
        let change = expansion.apply_available(|| {
            polls += 1;
            Err::<TryEvent, _>(())
        });
        assert_eq!(change, ChildChange::Unchanged);
        assert_eq!(expansion.shown(), ["src"]);
        assert!(expansion.is_settled());
        assert_eq!(polls, 1);
    }

    #[test]
    fn child_paths_stay_under_the_opened_directory() {
        assert_eq!(
            child_directory("C:\\Windows", "System32"),
            "C:\\Windows\\System32"
        );
        assert_eq!(
            child_directory("C:\\Windows\\", "System32"),
            "C:\\Windows\\System32"
        );
        assert_eq!(
            child_directory("C:/Windows", "System32"),
            "C:\\Windows\\System32"
        );
        assert_eq!(child_directory("C:\\", "Windows"), "C:\\Windows");
        assert_eq!(
            child_directory("\\\\server\\share\\docs", "specs"),
            "\\\\server\\share\\docs\\specs"
        );
    }
}
