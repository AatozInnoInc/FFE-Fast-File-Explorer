#![cfg_attr(not(windows), allow(dead_code))]
//! Notice for one pane.
//!
//! A timed-out volume is marked [`PaneNotice::NotResponding`]. The caller
//! stores that notice. A busy volume stays pending, so the notice is left
//! alone. Finished clears it. Cancel, failure, and a batch leave it as it
//! was. This does not retry, schedule a timer, or read the disk.

use fx_core::{EnumError, ListingEvent};

/// Status text for one pane after a listing times out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneNotice {
    /// The volume did not answer before its deadline.
    NotResponding,
}

impl PaneNotice {
    /// Text shown for this notice.
    pub fn label(self) -> &'static str {
        match self {
            Self::NotResponding => "Not responding",
        }
    }
}

/// Change to apply to the pane notice. The caller stores the notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeUpdate {
    /// Show this notice.
    Set(PaneNotice),
    /// Remove the notice.
    Clear,
    /// Leave the notice as it was.
    Keep,
}

impl NoticeUpdate {
    /// Apply this update to the notice a pane stores. True when the stored
    /// notice changed, so the caller re-lays out the pane only when needed.
    pub fn apply(self, notice: &mut Option<PaneNotice>) -> bool {
        let next = match self {
            Self::Set(set) => Some(set),
            Self::Clear => None,
            Self::Keep => *notice,
        };
        let changed = next != *notice;
        *notice = next;
        changed
    }
}

/// Timed out sets [`PaneNotice::NotResponding`]. Every other open error,
/// including a busy volume, leaves the notice as it was.
pub fn notice_on_open_error(err: &EnumError) -> NoticeUpdate {
    match err {
        EnumError::TimedOut => NoticeUpdate::Set(PaneNotice::NotResponding),
        EnumError::VolumeBusy
        | EnumError::Cancelled
        | EnumError::WindowsOnly
        | EnumError::WorkerStopped
        | EnumError::SpawnFailed
        | EnumError::Io { .. } => NoticeUpdate::Keep,
    }
}

/// Timed out sets the notice. Finished clears it. A batch, cancel, or
/// failure leaves the notice as it was.
pub fn notice_on_listing(event: &ListingEvent) -> NoticeUpdate {
    match event {
        ListingEvent::TimedOut => NoticeUpdate::Set(PaneNotice::NotResponding),
        ListingEvent::Finished => NoticeUpdate::Clear,
        ListingEvent::Batch(_) | ListingEvent::Cancelled | ListingEvent::Failed(_) => {
            NoticeUpdate::Keep
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_reports_a_change_only_when_the_notice_moves() {
        let mut notice = None;
        assert!(!NoticeUpdate::Keep.apply(&mut notice));
        assert!(!NoticeUpdate::Clear.apply(&mut notice));
        assert_eq!(notice, None);
        assert!(NoticeUpdate::Set(PaneNotice::NotResponding).apply(&mut notice));
        assert!(!NoticeUpdate::Set(PaneNotice::NotResponding).apply(&mut notice));
        assert!(!NoticeUpdate::Keep.apply(&mut notice));
        assert_eq!(notice, Some(PaneNotice::NotResponding));
        assert!(NoticeUpdate::Clear.apply(&mut notice));
        assert_eq!(notice, None);
    }

    #[test]
    fn not_responding_label_is_the_pane_text() {
        assert_eq!(PaneNotice::NotResponding.label(), "Not responding");
    }

    #[test]
    fn timeout_open_error_marks_the_pane_not_responding() {
        assert_eq!(
            notice_on_open_error(&EnumError::TimedOut),
            NoticeUpdate::Set(PaneNotice::NotResponding)
        );
    }

    #[test]
    fn other_open_errors_leave_the_notice() {
        let errors = [
            EnumError::VolumeBusy,
            EnumError::Cancelled,
            EnumError::WindowsOnly,
            EnumError::WorkerStopped,
            EnumError::SpawnFailed,
            EnumError::Io { code: 5 },
        ];
        for err in errors {
            assert_eq!(notice_on_open_error(&err), NoticeUpdate::Keep);
        }
    }

    #[test]
    fn listing_timeout_sets_the_notice_and_finished_clears_it() {
        assert_eq!(
            notice_on_listing(&ListingEvent::TimedOut),
            NoticeUpdate::Set(PaneNotice::NotResponding)
        );
        assert_eq!(
            notice_on_listing(&ListingEvent::Finished),
            NoticeUpdate::Clear
        );
    }

    #[test]
    fn batch_cancel_and_failure_leave_the_notice() {
        assert_eq!(
            notice_on_listing(&ListingEvent::Batch(vec![])),
            NoticeUpdate::Keep
        );
        assert_eq!(
            notice_on_listing(&ListingEvent::Cancelled),
            NoticeUpdate::Keep
        );
        assert_eq!(
            notice_on_listing(&ListingEvent::Failed(EnumError::Io { code: 5 })),
            NoticeUpdate::Keep
        );
        assert_eq!(
            notice_on_listing(&ListingEvent::Failed(EnumError::TimedOut)),
            NoticeUpdate::Keep
        );
    }
}
