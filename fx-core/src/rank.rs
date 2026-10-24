//! Filename ranking for Rapid Explorer.
//!
//! A score is match quality only. Frecency, recency, and any other time-based
//! boost are not applied. Whether that signal should be the default ranking
//! is an open question and stays out of this module.

/// Case-insensitive subsequence score. A higher score is a better match.
///
/// Ordering, best first:
/// 1. Exact case-insensitive equality.
/// 2. A contiguous substring. An earlier start outranks a later one.
/// 3. A sparse subsequence. An earlier start outranks a later one.
///
/// An exact match scores above a mere substring, which scores above a sparse
/// subsequence. A contiguous match always outranks a sparse one, including
/// when the sparse match starts earlier.
///
/// Returns `None` when `query` is empty or is not a subsequence of `name`.
/// An empty query does not match every name.
pub fn score(query: &str, name: &str) -> Option<u32> {
    if query.is_empty() {
        return None;
    }
    let query = fold(query);
    let name = fold(name);
    if query.is_empty() {
        return None;
    }
    if query == name {
        return Some(EXACT_SCORE);
    }
    if let Some(start) = earliest_substring(&name, &query) {
        return Some(SUBSTRING_TIER + earlier_bonus(start));
    }
    earliest_subsequence_start(&name, &query).map(|start| SUBSEQUENCE_TIER + earlier_bonus(start))
}

/// Matching names, best score first.
///
/// Equal scores keep input order. Names that do not match are omitted.
/// An empty query returns an empty vector.
pub fn rank<'a>(query: &str, names: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    rank_by(query, names, |name| name)
}

/// Same order as [`rank`], for any item that has a name.
///
/// Equal scores keep input order. An empty query returns an empty vector.
/// The score is match quality only. Frecency is not applied.
pub fn rank_by<'a, T: ?Sized>(
    query: &str,
    items: impl IntoIterator<Item = &'a T>,
    name_of: impl Fn(&T) -> &str,
) -> Vec<&'a T> {
    let mut matched = Vec::new();
    for (index, item) in items.into_iter().enumerate() {
        if let Some(score) = score(query, name_of(item)) {
            matched.push((score, index, item));
        }
    }
    matched.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(&right.1)));
    matched.into_iter().map(|(_, _, item)| item).collect()
}

// Tiers stay disjoint so the documented order cannot collapse:
// exact > contiguous substring > sparse subsequence.
// Inside a tier, an earlier start scores higher and never reaches the next tier.
const TIER: u32 = 1 << 30;
const EXACT_SCORE: u32 = TIER * 3;
const SUBSTRING_TIER: u32 = TIER * 2;
const SUBSEQUENCE_TIER: u32 = TIER;

fn earlier_bonus(start: usize) -> u32 {
    const MAX_BONUS: u32 = TIER - 1;
    let start = u32::try_from(start).unwrap_or(u32::MAX);
    MAX_BONUS.saturating_sub(start)
}

fn fold(text: &str) -> Vec<char> {
    text.to_lowercase().chars().collect()
}

fn earliest_substring(name: &[char], query: &[char]) -> Option<usize> {
    if query.is_empty() || query.len() > name.len() {
        return None;
    }
    name.windows(query.len()).position(|window| window == query)
}

fn earliest_subsequence_start(name: &[char], query: &[char]) -> Option<usize> {
    let mut query_index = 0;
    let mut start = None;
    for (index, ch) in name.iter().enumerate() {
        if *ch == query[query_index] {
            if query_index == 0 {
                start = Some(index);
            }
            query_index += 1;
            if query_index == query.len() {
                return start;
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_outranks_substring_which_outranks_subsequence() {
        let exact = score("report", "report").unwrap();
        assert_eq!(exact, score("report", "REPORT").unwrap());

        let substring = score("report", "preREPORT").unwrap();
        let sparse = score("report", "r_e_p_o_r_t").unwrap();
        assert!(exact > substring);
        assert!(substring > sparse);
        assert!(score("RePort", "r_e_p_o_r_t").is_some());
    }

    #[test]
    fn earlier_match_outranks_a_later_one() {
        let early_substring = score("ab", "abxx").unwrap();
        let late_substring = score("ab", "xxab").unwrap();
        assert!(early_substring > late_substring);

        let early_sparse = score("ab", "a_bxx").unwrap();
        let late_sparse = score("ab", "xxa_b").unwrap();
        assert!(early_sparse > late_sparse);
        assert!(late_substring > early_sparse);
    }

    #[test]
    fn non_match_is_none() {
        assert_eq!(score("abc", "ab"), None);
        assert_eq!(score("abc", "zzz"), None);
        assert!(rank("abc", ["ab", "zzz", "cba"]).is_empty());
    }

    #[test]
    fn empty_query_matches_nothing() {
        assert_eq!(score("", "report"), None);
        assert_eq!(score("", ""), None);
        assert!(rank("", ["a", "b", "c"]).is_empty());
    }

    #[test]
    fn equal_scores_keep_input_order() {
        assert_eq!(score("ab", "xxab"), score("ab", "yyab"));
        assert_eq!(
            rank("ab", ["xxab", "nope", "yyab", "zzab"]),
            vec!["xxab", "yyab", "zzab"]
        );
    }

    #[test]
    fn sparse_match_sorts_after_a_contiguous_one() {
        assert!(score("ab", "zzab").unwrap() > score("ab", "a_b").unwrap());
        assert_eq!(rank("ab", ["a_b", "zzab", "nope"]), vec!["zzab", "a_b"]);
    }
}
