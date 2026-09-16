//! The page list the picker's cursor and search box walk.
//!
//! Split out of the views for the reason `picker/entry.rs` is: the filtering is the part with
//! rules worth stating and testing, and both the tab strip and the panel build their rows the
//! same way.

use gpui_kit::SharedString;
use peek_canvas::Document;
use peek_document::PageId;

use crate::fuzzy::{MATCH_THRESHOLD, score};

/// One frame's view of a page, snapshotted so the document borrow ends before the listeners
/// that need `&mut Context` are built.
#[derive(Debug, Clone)]
pub(super) struct PageRow {
    pub(super) id: PageId,
    pub(super) name: SharedString,
    pub(super) active: bool,
    pub(super) closable: bool,
    /// Character offsets the query matched, for highlighting. Empty when not searching.
    pub(super) matched: Vec<usize>,
}

/// Every page, in document order.
pub(super) fn rows(document: &Document) -> Vec<PageRow> {
    let active = document.active_page_id().clone();
    // The `×` needs somewhere to go: the last page cannot be deleted.
    let closable = document.page_count() > 1;
    document
        .pages()
        .map(|page| PageRow {
            id: page.id.clone(),
            name: SharedString::from(page.name.clone()),
            active: page.id == active,
            closable: closable && page.id == active,
            matched: Vec::new(),
        })
        .collect()
}

/// Narrows and re-orders `rows` by `query`, and records where it matched.
///
/// A page has one searchable key, its name, so unlike the connection picker there is nothing to
/// rank across. An empty query is not a search: every page comes back in document order,
/// unhighlighted, which is what keeps the list stable while the box is empty.
pub(super) fn filter(rows: Vec<PageRow>, query: &str) -> Vec<PageRow> {
    let query = query.trim();
    if query.is_empty() {
        return rows;
    }

    let mut scored: Vec<(f64, PageRow)> = rows
        .into_iter()
        .filter_map(|mut row| {
            let found = score(&row.name, query).filter(|found| found.score >= MATCH_THRESHOLD)?;
            row.matched = found.indices;
            Some((found.score, row))
        })
        .collect();
    // Descending, and stable, so equally good pages keep their document order.
    scored.sort_by(|left, right| {
        right
            .0
            .partial_cmp(&left.0)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.into_iter().map(|(_, row)| row).collect()
}

#[cfg(test)]
mod tests {
    use super::{PageRow, filter};
    use gpui_kit::SharedString;
    use peek_document::PageId;

    fn rows(names: &[&str]) -> Vec<PageRow> {
        names
            .iter()
            .map(|name| PageRow {
                id: PageId::from(*name),
                name: SharedString::from((*name).to_string()),
                active: false,
                closable: false,
                matched: Vec::new(),
            })
            .collect()
    }

    fn names(rows: &[PageRow]) -> Vec<String> {
        rows.iter().map(|row| row.name.to_string()).collect()
    }

    #[test]
    fn an_empty_query_keeps_every_page_in_document_order() {
        let pages = rows(&["Overview", "Metrics", "Billing"]);
        assert_eq!(
            names(&filter(pages, "  ")),
            ["Overview", "Metrics", "Billing"]
        );
    }

    #[test]
    fn a_query_keeps_only_the_pages_it_matches() {
        let pages = rows(&["Overview", "Metrics", "Billing"]);
        assert_eq!(names(&filter(pages, "metr")), ["Metrics"]);
    }

    /// The load-bearing one: the row Enter takes is the first row on screen, so the best match
    /// has to be at the top rather than wherever the document happens to put it.
    #[test]
    fn the_best_match_comes_first() {
        let pages = rows(&["Weekly billing report", "Billing"]);
        assert_eq!(names(&filter(pages, "billing"))[0], "Billing");
    }

    #[test]
    fn matching_ignores_case_and_records_where_it_hit() {
        let found = filter(rows(&["Billing"]), "BILL");
        assert_eq!(found[0].matched, [0, 1, 2, 3]);
    }

    #[test]
    fn a_query_that_matches_nothing_leaves_no_rows() {
        assert!(filter(rows(&["Overview", "Metrics"]), "zzzz").is_empty());
    }
}
