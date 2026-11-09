#![cfg_attr(not(windows), allow(dead_code))]
//! Visible rows for one directory.
//!
//! The list control reads [`DirectoryRows::shown`] when it asks for a row.
//! A cache hit keeps those entries until a finished revalidation differs.
//! A miss appends batches as they arrive. Timeout, cancel, and failure leave
//! the rows already shown.

use fx_core::{FileEntry, ListingEvent, TryEvent};

use std::collections::HashSet;

use crate::column_sort::{sort_entries, RowSort};

/// What the owner-data control needs after one or more listing events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowChange {
    /// `shown().len()` is the new item count.
    Count,
    /// The count is unchanged and visible text must be requested again.
    Redraw,
    /// Nothing visible changed.
    Unchanged,
}

pub struct DirectoryRows {
    shown: Vec<FileEntry>,
    incoming: Vec<FileEntry>,
    from_cache: bool,
    settled: bool,
    /// Last `Finished` or `TimedOut` the pane notice has not read.
    terminal: Option<ListingEvent>,
    /// Order for `shown`. Batches and a finished revalidation are sorted
    /// before they are shown or compared, so a cache hit that did not change
    /// still reads as unchanged.
    sort: Option<RowSort>,
}

impl DirectoryRows {
    /// `cached` is what [`fx_core::Shell::open`] returned. This does not poll.
    pub fn from_cached(cached: Option<Vec<FileEntry>>) -> Self {
        match cached {
            Some(shown) => Self {
                shown,
                incoming: Vec::new(),
                from_cache: true,
                settled: false,
                terminal: None,
                sort: None,
            },
            None => Self {
                shown: Vec::new(),
                incoming: Vec::new(),
                from_cache: false,
                settled: false,
                terminal: None,
                sort: None,
            },
        }
    }

    pub fn shown(&self) -> &[FileEntry] {
        &self.shown
    }

    /// Keep `shown` in `sort` order from now on, and sort what is shown now.
    pub fn set_sort(&mut self, sort: RowSort) {
        self.sort = Some(sort);
        sort_entries(&mut self.shown, sort);
    }

    /// True while a sort is set and the listing can still reorder `shown`.
    pub fn may_reorder(&self) -> bool {
        self.sort.is_some() && !self.settled
    }

    /// Names at `indexes` in `shown`. Out-of-range indexes are skipped.
    pub fn names_at(&self, indexes: &[usize]) -> Vec<String> {
        indexes
            .iter()
            .filter_map(|&index| self.shown.get(index))
            .map(|entry| entry.name.clone())
            .collect()
    }

    /// Indexes in `shown` of the entries named in `names`, ascending. Names
    /// that are no longer listed are skipped. Used to keep a selection on the
    /// same files after the rows reorder.
    pub fn indexes_of(&self, names: &[String]) -> Vec<usize> {
        let wanted: HashSet<&str> = names.iter().map(String::as_str).collect();
        self.shown
            .iter()
            .enumerate()
            .filter(|(_, entry)| wanted.contains(entry.name.as_str()))
            .map(|(index, _)| index)
            .collect()
    }

    fn sort_in_place(sort: Option<RowSort>, entries: &mut [FileEntry]) {
        if let Some(sort) = sort {
            sort_entries(entries, sort);
        }
    }

    pub fn is_settled(&self) -> bool {
        self.settled
    }

    /// `Finished` or `TimedOut` since the last read. Other events are not stored.
    pub fn take_terminal(&mut self) -> Option<ListingEvent> {
        self.terminal.take()
    }

    /// `poll` returned [`fx_core::EnumError::TimedOut`] instead of a listing event.
    pub fn note_timeout(&mut self) {
        self.terminal = Some(ListingEvent::TimedOut);
    }

    /// Keep the rows already shown. Used when the live listing never starts.
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
    ) -> RowChange {
        if self.settled {
            return RowChange::Unchanged;
        }
        let mut change = RowChange::Unchanged;
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

    pub fn apply(&mut self, event: ListingEvent) -> RowChange {
        if self.settled {
            return RowChange::Unchanged;
        }
        match event {
            ListingEvent::Batch(batch) => {
                if self.from_cache {
                    self.incoming.extend(batch);
                    RowChange::Unchanged
                } else {
                    self.shown.extend(batch);
                    Self::sort_in_place(self.sort, &mut self.shown);
                    RowChange::Count
                }
            }
            ListingEvent::Finished => {
                self.terminal = Some(ListingEvent::Finished);
                self.settled = true;
                if !self.from_cache {
                    return RowChange::Unchanged;
                }
                Self::sort_in_place(self.sort, &mut self.incoming);
                let change = if self.incoming == self.shown {
                    RowChange::Unchanged
                } else if self.incoming.len() == self.shown.len() {
                    RowChange::Redraw
                } else {
                    RowChange::Count
                };
                if change == RowChange::Unchanged {
                    self.incoming.clear();
                } else {
                    self.shown = std::mem::take(&mut self.incoming);
                }
                change
            }
            ListingEvent::TimedOut => {
                self.terminal = Some(ListingEvent::TimedOut);
                self.stop();
                RowChange::Unchanged
            }
            ListingEvent::Cancelled | ListingEvent::Failed(_) => {
                self.stop();
                RowChange::Unchanged
            }
        }
    }
}

fn combine(left: RowChange, right: RowChange) -> RowChange {
    match (left, right) {
        (RowChange::Count, _) | (_, RowChange::Count) => RowChange::Count,
        (RowChange::Redraw, _) | (_, RowChange::Redraw) => RowChange::Redraw,
        _ => RowChange::Unchanged,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn entry(name: &str) -> FileEntry {
        FileEntry {
            name: name.to_string(),
            attributes: 0,
            size: 0,
            created: SystemTime::UNIX_EPOCH,
            last_write: SystemTime::UNIX_EPOCH,
            is_directory: false,
            is_reparse_point: false,
        }
    }

    fn names(rows: &DirectoryRows) -> Vec<&str> {
        rows.shown()
            .iter()
            .map(|entry| entry.name.as_str())
            .collect()
    }

    #[test]
    fn cache_hit_is_visible_before_any_event() {
        let rows = DirectoryRows::from_cached(Some(vec![entry("cached")]));
        assert_eq!(names(&rows), ["cached"]);
        assert!(!rows.is_settled());
    }

    #[test]
    fn cache_hit_keeps_shown_rows_until_a_different_listing_finishes() {
        let mut rows = DirectoryRows::from_cached(Some(vec![entry("cached")]));
        assert_eq!(
            rows.apply(ListingEvent::Batch(vec![entry("new"), entry("newer")])),
            RowChange::Unchanged
        );
        assert_eq!(names(&rows), ["cached"]);
        assert_eq!(rows.apply(ListingEvent::Finished), RowChange::Count);
        assert_eq!(names(&rows), ["new", "newer"]);
        assert!(rows.is_settled());
        assert_eq!(rows.apply(ListingEvent::Finished), RowChange::Unchanged);
        assert_eq!(names(&rows), ["new", "newer"]);
    }

    #[test]
    fn cache_hit_redraws_when_the_finished_listing_has_the_same_length() {
        let mut rows = DirectoryRows::from_cached(Some(vec![entry("cached")]));
        rows.apply(ListingEvent::Batch(vec![entry("renamed")]));
        assert_eq!(rows.apply(ListingEvent::Finished), RowChange::Redraw);
        assert_eq!(names(&rows), ["renamed"]);
    }

    #[test]
    fn cache_hit_unchanged_when_the_finished_listing_matches() {
        let mut rows = DirectoryRows::from_cached(Some(vec![entry("cached")]));
        rows.apply(ListingEvent::Batch(vec![entry("cached")]));
        assert_eq!(rows.apply(ListingEvent::Finished), RowChange::Unchanged);
        assert_eq!(names(&rows), ["cached"]);
    }

    #[test]
    fn cache_hit_timeout_keeps_the_cached_rows() {
        let mut rows = DirectoryRows::from_cached(Some(vec![entry("cached")]));
        rows.apply(ListingEvent::Batch(vec![entry("partial")]));
        assert_eq!(rows.apply(ListingEvent::TimedOut), RowChange::Unchanged);
        assert_eq!(names(&rows), ["cached"]);
        assert!(rows.is_settled());
    }

    #[test]
    fn miss_appends_batches_and_keeps_a_partial_cancel() {
        let mut rows = DirectoryRows::from_cached(None);
        assert!(names(&rows).is_empty());
        assert_eq!(
            rows.apply(ListingEvent::Batch(vec![entry("a")])),
            RowChange::Count
        );
        assert_eq!(
            rows.apply(ListingEvent::Batch(vec![entry("b")])),
            RowChange::Count
        );
        assert_eq!(names(&rows), ["a", "b"]);
        assert_eq!(rows.apply(ListingEvent::Cancelled), RowChange::Unchanged);
        assert_eq!(names(&rows), ["a", "b"]);
    }

    #[test]
    fn miss_finished_does_not_request_another_paint() {
        let mut rows = DirectoryRows::from_cached(None);
        rows.apply(ListingEvent::Batch(vec![entry("a")]));
        assert_eq!(rows.apply(ListingEvent::Finished), RowChange::Unchanged);
        assert_eq!(names(&rows), ["a"]);
    }

    #[test]
    fn apply_available_stops_after_a_terminal_event() {
        let mut rows = DirectoryRows::from_cached(None);
        let mut polls = 0;
        let change = rows.apply_available(|| {
            polls += 1;
            if polls == 1 {
                Ok::<_, ()>(TryEvent::Ready(ListingEvent::Batch(vec![entry("a")])))
            } else if polls == 2 {
                Ok(TryEvent::Ready(ListingEvent::Finished))
            } else {
                panic!("polled after the listing finished");
            }
        });
        assert_eq!(change, RowChange::Count);
        assert_eq!(names(&rows), ["a"]);
        assert_eq!(polls, 2);
    }

    #[test]
    fn apply_available_does_not_wait_on_pending() {
        let mut rows = DirectoryRows::from_cached(Some(vec![entry("cached")]));
        let mut polls = 0;
        let change = rows.apply_available(|| {
            polls += 1;
            if polls == 1 {
                Ok::<_, ()>(TryEvent::Ready(ListingEvent::Batch(vec![entry("new")])))
            } else {
                Ok(TryEvent::Pending)
            }
        });
        assert_eq!(change, RowChange::Unchanged);
        assert_eq!(names(&rows), ["cached"]);
        assert_eq!(polls, 2);
        assert!(!rows.is_settled());
    }

    #[test]
    fn apply_available_keeps_cached_rows_when_poll_fails() {
        let mut rows = DirectoryRows::from_cached(Some(vec![entry("cached")]));
        let change = rows.apply_available(|| Err::<TryEvent, _>(()));
        assert_eq!(change, RowChange::Unchanged);
        assert_eq!(names(&rows), ["cached"]);
        assert!(rows.is_settled());
    }

    #[test]
    fn stop_keeps_shown_rows_when_the_listing_does_not_start() {
        let mut rows = DirectoryRows::from_cached(Some(vec![entry("cached")]));
        rows.stop();
        assert_eq!(names(&rows), ["cached"]);
        assert_eq!(
            rows.apply(ListingEvent::Batch(vec![entry("late")])),
            RowChange::Unchanged
        );
        assert_eq!(names(&rows), ["cached"]);
    }

    fn by_name() -> RowSort {
        RowSort {
            column: crate::column_sort::SortColumn::Name,
            dir: crate::column_sort::SortDir::Asc,
        }
    }

    #[test]
    fn a_sorted_miss_keeps_every_batch_in_order() {
        let mut rows = DirectoryRows::from_cached(None);
        rows.set_sort(by_name());
        rows.apply(ListingEvent::Batch(vec![entry("c"), entry("a")]));
        assert_eq!(names(&rows), ["a", "c"]);
        rows.apply(ListingEvent::Batch(vec![entry("b")]));
        assert_eq!(names(&rows), ["a", "b", "c"]);
        assert!(rows.may_reorder());
        rows.apply(ListingEvent::Finished);
        assert!(!rows.may_reorder());
    }

    #[test]
    fn a_sorted_cache_hit_that_did_not_change_stays_unchanged() {
        let mut rows = DirectoryRows::from_cached(Some(vec![entry("b"), entry("a")]));
        rows.set_sort(by_name());
        assert_eq!(names(&rows), ["a", "b"]);
        rows.apply(ListingEvent::Batch(vec![entry("b"), entry("a")]));
        assert_eq!(rows.apply(ListingEvent::Finished), RowChange::Unchanged);
        assert_eq!(names(&rows), ["a", "b"]);
    }

    #[test]
    fn a_sorted_cache_hit_that_changed_is_shown_sorted() {
        let mut rows = DirectoryRows::from_cached(Some(vec![entry("b"), entry("a")]));
        rows.set_sort(by_name());
        rows.apply(ListingEvent::Batch(vec![
            entry("c"),
            entry("a"),
            entry("b"),
        ]));
        assert_eq!(rows.apply(ListingEvent::Finished), RowChange::Count);
        assert_eq!(names(&rows), ["a", "b", "c"]);
    }

    #[test]
    fn without_a_sort_rows_never_reorder() {
        let rows = DirectoryRows::from_cached(None);
        assert!(!rows.may_reorder());
    }

    #[test]
    fn a_selection_follows_its_names_across_a_reorder() {
        let mut rows = DirectoryRows::from_cached(Some(vec![entry("c"), entry("a"), entry("b")]));
        let selected = rows.names_at(&[0, 2, 99]);
        assert_eq!(selected, ["c", "b"]);
        rows.set_sort(by_name());
        assert_eq!(rows.indexes_of(&selected), [1, 2]);
        assert_eq!(rows.indexes_of(&["gone".to_string()]), Vec::<usize>::new());
    }
}
