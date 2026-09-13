//! What the command palette actually lists.
//!
//! A registry [`Command`](super::Command) is `'static` and knows nothing about the document, but
//! some entries only exist at runtime — one "Go to <page>" per page. Both become a
//! [`PaletteEntry`], so the palette has a single kind of row to render and dispatch.

use gpui_kit::{Action, SharedString};
use peek_canvas::{Document, Scope};

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
