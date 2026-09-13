//! Finding rows inside a result.
//!
//! Ported from `useResultSearchMatches.ts`, which uses `fuzzysort`. There is no fuzzy matcher in
//! gpui-component — its command palette is a plain case-insensitive `contains` — so the scoring
//! here is **ours**, not fuzzysort's numbers. It keeps the reference's shape: a subsequence
//! match, a score in `0.0..=1.0`, and the same `MATCH_THRESHOLD` of 0.5, below which a row
//! neither shows nor highlights.
//!
//! What it rewards, in the order that matters for a database grid, where people mostly type a
//! literal fragment of a value: a contiguous run beats a scattered one, a match at the start of
//! the value beats one in the middle, and a shorter haystack beats a longer one holding the same
//! match.

use std::collections::HashMap;

use peek_document::ResultSet;

/// `MATCH_THRESHOLD` in the reference: 1 is perfect, 0.5 is a good match, 0 is none.
const MATCH_THRESHOLD: f64 = 0.5;

/// Where a query matched inside one value.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Match {
    pub(super) score: f64,
    /// Character (not byte) offsets of the matched characters, for highlighting.
    pub(super) indices: Vec<usize>,
}

/// Scores `needle` against `haystack`, case-insensitively. `None` when it is not a subsequence.
///
/// An empty needle matches nothing: an empty search box shows everything by not searching at
/// all, rather than by matching every row with a score of zero.
pub(super) fn score(haystack: &str, needle: &str) -> Option<Match> {
    if needle.is_empty() || haystack.is_empty() {
        return None;
    }
    let hay: Vec<char> = haystack.chars().flat_map(char::to_lowercase).collect();
    let pin: Vec<char> = needle.chars().flat_map(char::to_lowercase).collect();
    if pin.len() > hay.len() {
        return None;
    }

    let mut indices = Vec::with_capacity(pin.len());
    let mut cursor = 0;
    for wanted in &pin {
        let found = hay[cursor..]
            .iter()
            .position(|candidate| candidate == wanted)?;
        indices.push(cursor + found);
        cursor += found + 1;
    }

    Some(Match {
        score: rate(&hay, &indices),
        indices,
    })
}

/// Turns a set of matched positions into `0.0..=1.0`.
fn rate(hay: &[char], indices: &[usize]) -> f64 {
    let matched = indices.len();
    debug_assert!(matched > 0, "an empty match is not produced");

    // How much of the match is one unbroken run. A literal substring scores 1 here, which is the
    // case that has to feel exact.
    let contiguous = indices
        .windows(2)
        .filter(|pair| pair[1] == pair[0] + 1)
        .count();
    #[allow(
        clippy::cast_precision_loss,
        reason = "match lengths are far below 2^53"
    )]
    let run = if matched > 1 {
        contiguous as f64 / (matched - 1) as f64
    } else {
        1.0
    };

    // Matching at the start of the value, or at a word boundary inside it, beats the middle.
    let first = indices[0];
    let at_start = if first == 0 {
        1.0
    } else if hay
        .get(first - 1)
        .is_some_and(|before| !before.is_alphanumeric())
    {
        0.8
    } else {
        0.0
    };

    // A short value holding the match is a better hit than a long one that merely contains it.
    #[allow(
        clippy::cast_precision_loss,
        reason = "cell lengths are far below 2^53"
    )]
    let density = matched as f64 / hay.len() as f64;

    (run * 0.6) + (at_start * 0.25) + (density * 0.15)
}

/// Which rows a search leaves visible, and where it matched inside them.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Matches {
    /// Display order: position on screen → index into the result. Re-sorted by score, which is
    /// exactly why selection positions are display positions and are dropped when this changes.
    visible: Vec<usize>,
    cells: HashMap<(usize, usize), Match>,
}

impl Matches {
    /// Every row, unsearched and in their own order.
    pub(super) fn unfiltered(row_count: usize) -> Self {
        Self {
            visible: (0..row_count).collect(),
            cells: HashMap::new(),
        }
    }

    pub(super) fn visible(&self) -> &[usize] {
        &self.visible
    }

    pub(crate) fn len(&self) -> usize {
        self.visible.len()
    }

    /// The data row at a display position.
    pub(super) fn row_at(&self, position: usize) -> Option<usize> {
        self.visible.get(position).copied()
    }

    /// Where the query matched in one cell, by **data** row index.
    pub(super) fn cell(&self, row: usize, column: usize) -> Option<&Match> {
        self.cells.get(&(row, column))
    }

    pub(super) fn is_searching(&self) -> bool {
        !self.cells.is_empty() || self.visible.is_empty()
    }
}

/// Searches every cell, keeping rows whose best cell clears the threshold and ordering them by
/// that score.
pub(super) fn search(rows: &ResultSet, query: &str) -> Matches {
    let query = query.trim();
    if query.is_empty() {
        return Matches::unfiltered(rows.row_count());
    }

    let mut cells = HashMap::new();
    let mut scored: Vec<(usize, f64)> = Vec::new();

    for (row, values) in rows.rows().iter().enumerate() {
        let mut best = 0.0_f64;
        for (column, value) in values.iter().enumerate() {
            let Some(found) = score(&value.to_display_string(), query) else {
                continue;
            };
            if found.score < MATCH_THRESHOLD {
                continue;
            }
            best = best.max(found.score);
            cells.insert((row, column), found);
        }
        if best > 0.0 {
            scored.push((row, best));
        }
    }

    // Descending by score, and stable, so equally good rows keep the order they came back in.
    scored.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Matches {
        visible: scored.into_iter().map(|(row, _)| row).collect(),
        cells,
    }
}

#[cfg(test)]
mod tests {
    use peek_document::{Cell, Column, ResultSet};

    use super::{MATCH_THRESHOLD, score, search};

    fn rows(values: &[&str]) -> ResultSet {
        ResultSet::new(
            vec![Column::new("name", "TEXT")],
            values
                .iter()
                .map(|value| vec![Cell::Text((*value).to_string())])
                .collect(),
        )
    }

    #[test]
    fn a_literal_substring_scores_well_above_the_threshold() {
        let found = score("cta.hero", "hero").expect("matches");
        assert!(found.score > MATCH_THRESHOLD, "{}", found.score);
        assert_eq!(found.indices, [4, 5, 6, 7]);
    }

    #[test]
    fn matching_ignores_case() {
        assert!(score("CTA.Hero", "hero").is_some());
        assert!(score("cta.hero", "HERO").is_some());
    }

    #[test]
    fn a_value_that_does_not_contain_the_characters_does_not_match() {
        assert!(score("cta.hero", "zzz").is_none());
        assert!(
            score("abc", "abcd").is_none(),
            "needle longer than haystack"
        );
    }

    /// A subsequence still matches, but scattered characters must not rank with a real substring
    /// — that is what the threshold is for.
    #[test]
    fn a_scattered_subsequence_scores_below_a_contiguous_one() {
        let scattered = score("a_b_c_d_e", "abc").expect("matches");
        let contiguous = score("xxabcxx", "abc").expect("matches");
        assert!(scattered.score < contiguous.score);
        assert!(scattered.score < MATCH_THRESHOLD, "{}", scattered.score);
    }

    #[test]
    fn a_match_at_the_start_beats_the_same_match_in_the_middle() {
        let start = score("heroic", "hero").expect("matches");
        let middle = score("the heroic", "hero").expect("matches");
        assert!(start.score > middle.score);
    }

    /// A short value holding the match is a better hit than a long one merely containing it.
    #[test]
    fn a_shorter_value_outranks_a_longer_one() {
        let short = score("hero", "hero").expect("matches");
        let long = score("hero among a great many other words", "hero").expect("matches");
        assert!(short.score > long.score);
    }

    #[test]
    fn an_empty_query_matches_nothing_on_its_own() {
        assert!(score("anything", "").is_none());
    }

    #[test]
    fn an_empty_search_shows_every_row_in_its_own_order() {
        let matches = search(&rows(&["b", "a", "c"]), "   ");
        assert_eq!(matches.visible(), [0, 1, 2]);
        assert!(!matches.is_searching());
    }

    #[test]
    fn searching_keeps_only_matching_rows() {
        let matches = search(&rows(&["alpha", "beta", "alphabet"]), "alpha");
        assert_eq!(matches.len(), 2);
        assert!(matches.visible().contains(&0));
        assert!(matches.visible().contains(&2));
        assert!(!matches.visible().contains(&1));
    }

    /// The best match comes first, which is what makes typing a few characters useful.
    #[test]
    fn rows_are_ordered_by_their_best_cell() {
        let matches = search(
            &rows(&["a very long line that mentions hero late", "hero"]),
            "hero",
        );
        assert_eq!(matches.row_at(0), Some(1), "the exact, short match leads");
    }

    #[test]
    fn a_search_that_matches_nothing_hides_every_row() {
        let matches = search(&rows(&["alpha", "beta"]), "zzzz");
        assert_eq!(matches.len(), 0);
        assert!(matches.is_searching(), "so the empty state can say why");
    }

    #[test]
    fn matched_cells_are_recorded_for_highlighting() {
        let result = rows(&["alpha"]);
        let matches = search(&result, "lph");
        let found = matches.cell(0, 0).expect("the cell matched");
        assert_eq!(found.indices, [1, 2, 3]);
    }

    /// Character offsets, not byte offsets, or highlighting would slice a multi-byte value in
    /// the middle of a character.
    #[test]
    fn indices_are_character_offsets() {
        let found = score("café_id", "id").expect("matches");
        assert_eq!(found.indices, [5, 6]);
    }
}
