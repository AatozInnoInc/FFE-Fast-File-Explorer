#![cfg_attr(not(windows), allow(dead_code))]
//! Places for one expanded network node.
//!
//! Nothing here calls WNet or the listing cache. [`PlaceList::start`] does not
//! browse. Batches are appended as they arrive. Timeout, cancel, and failure
//! keep the places already shown.

use fx_core::{BrowseEvent, NetworkPlace, TryBrowse};

/// Places the tree may insert under one network node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaceChange {
    /// Places that were not inserted yet, in browse order.
    Append(Vec<NetworkPlace>),
    Unchanged,
}

pub struct PlaceList {
    shown: Vec<NetworkPlace>,
    settled: bool,
}

impl PlaceList {
    /// No places yet. This does not start a browse.
    pub fn start() -> Self {
        Self {
            shown: Vec::new(),
            settled: false,
        }
    }

    pub fn is_settled(&self) -> bool {
        self.settled
    }

    #[allow(dead_code)]
    pub fn shown(&self) -> &[NetworkPlace] {
        &self.shown
    }

    /// Keep the places already shown. Used when the browse never starts.
    pub fn stop(&mut self) {
        self.settled = true;
    }

    /// Drain `try_recv` until it would wait, or until the browse is finished.
    ///
    /// A finished browse stays ready forever. Stopping here keeps the UI thread
    /// from spinning on that event.
    pub fn apply_available(&mut self, mut poll: impl FnMut() -> TryBrowse) -> PlaceChange {
        if self.settled {
            return PlaceChange::Unchanged;
        }
        let mut change = PlaceChange::Unchanged;
        loop {
            if self.settled {
                break;
            }
            match poll() {
                TryBrowse::Pending => break,
                TryBrowse::Ready(event) => change = combine(change, self.apply(event)),
            }
        }
        change
    }

    pub fn apply(&mut self, event: BrowseEvent) -> PlaceChange {
        if self.settled {
            return PlaceChange::Unchanged;
        }
        match event {
            BrowseEvent::Batch(batch) => {
                if batch.is_empty() {
                    return PlaceChange::Unchanged;
                }
                self.shown.extend(batch.iter().cloned());
                PlaceChange::Append(batch)
            }
            BrowseEvent::Finished => {
                self.settled = true;
                PlaceChange::Unchanged
            }
            BrowseEvent::Cancelled | BrowseEvent::TimedOut | BrowseEvent::Failed(_) => {
                self.stop();
                PlaceChange::Unchanged
            }
        }
    }
}

fn combine(left: PlaceChange, right: PlaceChange) -> PlaceChange {
    match (left, right) {
        (PlaceChange::Append(mut places), PlaceChange::Append(extra)) => {
            places.extend(extra);
            PlaceChange::Append(places)
        }
        (PlaceChange::Unchanged, right) => right,
        (left, PlaceChange::Unchanged) => left,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(name: &str, unc: Option<&str>, container: bool) -> NetworkPlace {
        NetworkPlace {
            name: name.to_string(),
            unc: unc.map(str::to_string),
            container,
        }
    }

    #[test]
    fn start_has_no_places_and_does_not_settle() {
        let list = PlaceList::start();
        assert!(list.shown().is_empty());
        assert!(!list.is_settled());
    }

    #[test]
    fn batches_append_until_finished() {
        let mut list = PlaceList::start();
        let server = place("server", Some(r"\\server"), true);
        let share = place("share", Some(r"\\server\share"), false);
        assert_eq!(
            list.apply(BrowseEvent::Batch(vec![server.clone()])),
            PlaceChange::Append(vec![server.clone()])
        );
        assert!(!list.is_settled());
        assert_eq!(
            list.apply(BrowseEvent::Batch(vec![share.clone()])),
            PlaceChange::Append(vec![share.clone()])
        );
        assert_eq!(list.apply(BrowseEvent::Finished), PlaceChange::Unchanged);
        assert!(list.is_settled());
        assert_eq!(list.shown(), [server, share]);
        assert_eq!(
            list.apply(BrowseEvent::Batch(vec![place("later", None, true)])),
            PlaceChange::Unchanged
        );
    }

    #[test]
    fn timeout_keeps_places_already_shown() {
        let mut list = PlaceList::start();
        let share = place("share", Some(r"\\server\share"), false);
        assert_eq!(
            list.apply(BrowseEvent::Batch(vec![share.clone()])),
            PlaceChange::Append(vec![share.clone()])
        );
        assert_eq!(list.apply(BrowseEvent::TimedOut), PlaceChange::Unchanged);
        assert!(list.is_settled());
        assert_eq!(list.shown(), [share]);
    }

    #[test]
    fn cancel_and_failure_keep_places_already_shown() {
        let mut cancelled = PlaceList::start();
        let kept = place("kept", None, true);
        cancelled.apply(BrowseEvent::Batch(vec![kept.clone()]));
        assert_eq!(
            cancelled.apply(BrowseEvent::Cancelled),
            PlaceChange::Unchanged
        );
        assert_eq!(cancelled.shown(), [kept]);

        let mut failed = PlaceList::start();
        failed.apply(BrowseEvent::Failed(fx_core::EnumError::Io { code: 5 }));
        assert!(failed.is_settled());
        assert!(failed.shown().is_empty());
    }

    #[test]
    fn apply_available_does_not_wait_on_pending() {
        let mut list = PlaceList::start();
        let started = std::time::Instant::now();
        let change = list.apply_available(|| TryBrowse::Pending);
        assert!(
            started.elapsed() < std::time::Duration::from_millis(50),
            "apply_available blocked for {:?}",
            started.elapsed()
        );
        assert_eq!(change, PlaceChange::Unchanged);
        assert!(!list.is_settled());
    }

    #[test]
    fn apply_available_stops_after_a_terminal_event() {
        let mut list = PlaceList::start();
        let share = place("share", Some(r"\\server\share"), false);
        let mut polls = 0;
        let change = list.apply_available(|| {
            polls += 1;
            match polls {
                1 => TryBrowse::Ready(BrowseEvent::Batch(vec![share.clone()])),
                2 => TryBrowse::Ready(BrowseEvent::Finished),
                _ => panic!("polled after the browse finished"),
            }
        });
        assert_eq!(polls, 2);
        assert_eq!(change, PlaceChange::Append(vec![share]));
        assert!(list.is_settled());
    }

    #[test]
    fn stop_keeps_shown_places_when_the_browse_does_not_start() {
        let mut list = PlaceList::start();
        list.apply(BrowseEvent::Batch(vec![place("workgroup", None, true)]));
        list.stop();
        assert!(list.is_settled());
        assert_eq!(list.shown().len(), 1);
        assert_eq!(
            list.apply_available(|| panic!("a settled browse must not be polled")),
            PlaceChange::Unchanged
        );
    }
}
