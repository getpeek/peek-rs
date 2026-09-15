//! What the command palette actually lists.
//!
//! A registry [`Command`](super::Command) is `'static` and knows nothing about the document, but
//! some entries only exist at runtime — one "Go to <page>" per page. Both become a
//! [`PaletteEntry`], so the palette has a single kind of row to render and dispatch.
//!
//! The matching is ours, not gpui-component's: its palette filters by a case-insensitive
//! `contains` and leaves the rows in model order, which neither finds "Fit all nodes in view"
//! from `fitv` nor puts the best answer first. [`Listing`] scores with [`crate::fuzzy`] instead,
//! the same way the connection picker, the result find bar and page search already do.

use gpui_kit::{Action, SharedString};
use peek_canvas::{Document, Scope};

use crate::fuzzy::{MATCH_THRESHOLD, score};

/// How much a keyword hit is worth against a hit on the title.
///
/// Keywords are short — `"clipboard export"`, `"save download file"` — and a short haystack
/// scores high by construction, so at face value a row matched on a term nobody can see outranks
/// the row whose visible label the reader was actually typing.
const KEYWORD_WEIGHT: f64 = 0.9;

pub struct PaletteEntry {
    pub title: SharedString,
    pub keywords: SharedString,
    pub action: Box<dyn Action>,
}

impl Clone for PaletteEntry {
    fn clone(&self) -> Self {
        Self {
            title: self.title.clone(),
            keywords: self.keywords.clone(),
            action: self.action.boxed_clone(),
        }
    }
}

impl std::fmt::Debug for PaletteEntry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PaletteEntry")
            .field("title", &self.title)
            .finish_non_exhaustive()
    }
}

/// Entries generated from the document rather than the registry. Each provider appends its own;
/// registering here is all a feature has to do to put runtime rows in the palette.
static DYNAMIC: &[fn(&Document, &mut Vec<PaletteEntry>)] = &[go_to_page];

/// One "Go to <page>" per page, the active one excluded — switching to the page you are on is
/// the row that can never do anything.
fn go_to_page(document: &Document, entries: &mut Vec<PaletteEntry>) {
    let active = document.active_page_id().clone();
    entries.extend(
        document
            .pages()
            .filter(|page| page.id != active)
            .map(|page| PaletteEntry {
                title: SharedString::from(format!("Go to {}", page.name)),
                // The page's own name is already in the title; "page" is what someone types who
                // wants the list rather than one name, as the reference's `searchAgainst` has it.
                keywords: SharedString::new_static("page"),
                action: Box::new(super::actions::page::GoTo {
                    page: page.id.clone(),
                }),
            }),
    );
}

/// Everything the palette should show for this document and scope, registry rows first.
#[must_use]
pub fn entries(document: &Document, scope: &Scope) -> Vec<PaletteEntry> {
    let mut entries: Vec<PaletteEntry> = super::all()
        // Opening the palette from the palette is the one row that can never be useful.
        .filter(|command| command.id != "CommandPalette::Open")
        .filter(|command| (command.available)(scope))
        .map(|command| PaletteEntry {
            title: SharedString::new_static(command.label(scope)),
            keywords: SharedString::new_static(command.keywords),
            action: (command.build)(),
        })
        .collect();

    for provider in DYNAMIC {
        provider(document, &mut entries);
    }
    entries
}

/// One entry that survived a query, and where the query hit its title.
#[derive(Debug, Clone)]
pub(crate) struct Hit {
    pub(crate) entry: PaletteEntry,
    /// Character offsets in `entry.title`, for highlighting. Empty when the row was matched by a
    /// keyword, which is not drawn and so has nothing to mark.
    pub(crate) title_match: Vec<usize>,
}

/// What one open palette can show, and what the query typed so far ranked out of it.
///
/// Held in its own entity rather than on `WorkspaceView`: the dialog's content builder runs
/// while the workspace is rendering, and reading the entity being rendered is a double lease.
#[derive(Debug)]
pub(crate) struct Listing {
    all: Vec<PaletteEntry>,
    matched: Vec<Hit>,
}

impl Listing {
    pub(crate) fn new(all: Vec<PaletteEntry>) -> Self {
        let matched = search(&all, "");
        Self { all, matched }
    }

    pub(crate) fn refine(&mut self, query: &str) {
        self.matched = search(&self.all, query);
    }

    pub(crate) fn matched(&self) -> &[Hit] {
        &self.matched
    }
}

/// Narrows and re-orders `entries` by `query`, and records where it hit each title.
///
/// An empty query is not a search: everything comes back in registry order, unhighlighted.
fn search(entries: &[PaletteEntry], query: &str) -> Vec<Hit> {
    let query = query.trim();
    if query.is_empty() {
        return entries
            .iter()
            .map(|entry| Hit {
                entry: entry.clone(),
                title_match: Vec::new(),
            })
            .collect();
    }

    let mut scored: Vec<(f64, Hit)> = entries
        .iter()
        .filter_map(|entry| rank(entry, query))
        .collect();
    // Descending, and stable, so equally good rows keep their registry order.
    scored.sort_by(|left, right| {
        right
            .0
            .partial_cmp(&left.0)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.into_iter().map(|(_, hit)| hit).collect()
}

/// Scores one entry against its title and each keyword separately, or drops it.
///
/// Each keyword is its own haystack. Scoring the joined string instead would let a subsequence
/// wander across two unrelated terms, and would sink every short keyword's density besides.
fn rank(entry: &PaletteEntry, query: &str) -> Option<(f64, Hit)> {
    let title = score(&entry.title, query).filter(|found| found.score >= MATCH_THRESHOLD);
    let keyword = entry
        .keywords
        .split_whitespace()
        .filter_map(|keyword| score(keyword, query))
        .map(|found| found.score * KEYWORD_WEIGHT)
        .fold(0.0_f64, f64::max);

    let best = title.as_ref().map_or(0.0, |found| found.score).max(keyword);
    if best < MATCH_THRESHOLD {
        return None;
    }

    Some((
        best,
        Hit {
            entry: entry.clone(),
            title_match: title.map(|found| found.indices).unwrap_or_default(),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::{Hit, PaletteEntry, search};
    use crate::commands::actions;
    use gpui_kit::SharedString;

    /// The action is irrelevant to ranking; every row carries the same one.
    fn entry(title: &'static str, keywords: &'static str) -> PaletteEntry {
        PaletteEntry {
            title: SharedString::new_static(title),
            keywords: SharedString::new_static(keywords),
            action: Box::new(actions::zoom::In),
        }
    }

    fn titles(hits: &[Hit]) -> Vec<&str> {
        hits.iter().map(|hit| hit.entry.title.as_ref()).collect()
    }

    /// The whole point: `fitv` is not a substring of anything, which is why the palette used to
    /// come up empty on it.
    #[test]
    fn a_query_that_is_not_a_substring_still_finds_its_command() {
        let entries = [entry("Fit all nodes in view", ""), entry("Undo", "")];
        assert_eq!(titles(&search(&entries, "fitv")), ["Fit all nodes in view"]);
    }

    #[test]
    fn the_best_match_leads() {
        let entries = [
            entry("Add a copy", ""),
            entry("Copy cell value", ""),
            entry("Copy", ""),
        ];
        assert_eq!(
            titles(&search(&entries, "copy")),
            ["Copy", "Copy cell value", "Add a copy"]
        );
    }

    /// A keyword is a way in, not something to draw: nothing on the row holds those characters.
    #[test]
    fn a_row_matched_only_by_a_keyword_surfaces_unhighlighted() {
        let hits = search(&[entry("Zoom in", "bigger closer")], "bigger");
        assert_eq!(titles(&hits), ["Zoom in"]);
        assert!(hits[0].title_match.is_empty());
    }

    /// What [`KEYWORD_WEIGHT`](super::KEYWORD_WEIGHT) is for. Undiscounted, the perfect score a
    /// six-letter keyword earns would bury the row whose visible title the reader typed.
    #[test]
    fn a_title_hit_outranks_a_keyword_hit() {
        let entries = [
            entry("Copy as CSV", "clipboard export"),
            entry("Export as CSV", ""),
        ];
        assert_eq!(
            titles(&search(&entries, "export")),
            ["Export as CSV", "Copy as CSV"]
        );
    }

    #[test]
    fn an_empty_query_returns_every_entry_in_registry_order() {
        let entries = [entry("Zoom in", ""), entry("Undo", "")];
        let hits = search(&entries, "");
        assert_eq!(titles(&hits), ["Zoom in", "Undo"]);
        assert!(hits.iter().all(|hit| hit.title_match.is_empty()));
    }

    #[test]
    fn a_query_that_matches_nothing_returns_nothing() {
        assert!(search(&[entry("Zoom in", "bigger")], "zzzz").is_empty());
    }

    /// Character offsets, not byte offsets, or highlighting would split a multi-byte character.
    #[test]
    fn title_match_holds_character_offsets() {
        let hits = search(&[entry("Café view", "")], "view");
        assert_eq!(hits[0].title_match, [5, 6, 7, 8]);
    }
}
