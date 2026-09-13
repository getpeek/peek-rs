//! Finding rows inside a result.
//!
//! Ported from `useResultSearchMatches.ts`. The scoring lives in [`crate::fuzzy`], which the
//! connection picker's search shares; what belongs here is the part that is about a *result*:
//! searching every cell, keeping the rows whose best cell clears the threshold, and ordering
//! them by that score.

use std::collections::HashMap;

use peek_document::ResultSet;

use crate::fuzzy::{MATCH_THRESHOLD, Match, score};

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

    use super::search;

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
}
