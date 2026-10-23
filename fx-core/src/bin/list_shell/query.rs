#![cfg_attr(not(windows), allow(dead_code))]
//! Search-as-you-type text for this window.
//!
//! The caller ranks matches. This module stores the typed text and does not
//! rank, touch the disk, or record frecency.

/// Text typed into search. Empty after trim means search is off.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Query {
    text: String,
}

impl Query {
    pub fn new() -> Self {
        Self::default()
    }

    /// Store `text` as typed. True when the stored text changed.
    pub fn set(&mut self, text: &str) -> bool {
        if self.text == text {
            return false;
        }
        self.text.clear();
        self.text.push_str(text);
        true
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// True when the stored text is non-empty after trim.
    pub fn is_active(&self) -> bool {
        !self.needle().is_empty()
    }

    /// Text the ranker should see. Surrounding whitespace is not part of the query.
    pub fn needle(&self) -> &str {
        self.text.trim()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn clear(&mut self) {
        self.text.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_query_is_empty_and_inactive() {
        let query = Query::new();
        assert_eq!(query.text(), "");
        assert!(!query.is_active());
        assert_eq!(Query::default().text(), "");
    }

    #[test]
    fn set_reports_whether_the_text_changed() {
        let mut query = Query::new();
        assert!(!query.set(""));
        assert!(query.set("report"));
        assert_eq!(query.text(), "report");
        assert!(query.is_active());
        assert!(!query.set("report"));
        assert!(query.set("reports"));
        assert_eq!(query.text(), "reports");
    }

    #[test]
    fn stored_text_keeps_surrounding_whitespace() {
        let mut query = Query::new();
        assert!(query.set("  report  "));
        assert_eq!(query.text(), "  report  ");
        assert!(query.is_active());
        assert!(!query.set("  report  "));
    }

    #[test]
    fn whitespace_only_is_stored_and_inactive() {
        let mut query = Query::new();
        assert!(query.set("   "));
        assert_eq!(query.text(), "   ");
        assert!(!query.is_active());
        assert!(query.set("\t\n"));
        assert_eq!(query.text(), "\t\n");
        assert!(!query.is_active());
        assert!(!query.set("\t\n"));
        assert_eq!(query.needle(), "");
    }

    #[test]
    fn needle_trims_but_text_stays() {
        let mut query = Query::new();
        query.set("  report  ");
        assert_eq!(query.text(), "  report  ");
        assert_eq!(query.needle(), "report");
    }

    #[test]
    fn clear_removes_the_text() {
        let mut query = Query::new();
        query.set("  report");
        query.clear();
        assert_eq!(query.text(), "");
        assert!(!query.is_active());
        query.clear();
        assert_eq!(query.text(), "");

        assert!(query.set("   "));
        assert!(!query.is_active());
        query.clear();
        assert_eq!(query.text(), "");
        assert!(!query.set(""));
    }
}
