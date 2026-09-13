//! The SQL language server behind every query editor.
//!
//! `peek-lsp` is synchronous, window-free and keyed by document URI, so one `Backend` serves
//! every query node in the process — the shape the Tauri host used, with its commands replaced
//! by direct calls.

use std::cell::Cell;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::Arc;

use gpui_kit::base::input::{CompletionProvider, Rope};
use gpui_kit::{App, Global, Task, WeakEntity, Window};
use peek_canvas::Document;
use peek_document::NodeId;
use peek_lsp::lsp_types::{
    self, CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, Uri,
};

/// The characters that open the completion menu, from `lspProvider.ts`'s `triggerCharacters`,
/// the `@` of its second provider, and Monaco's `quickSuggestions`, which keeps the menu open
/// while an identifier is being typed.
const TRIGGERS: [char; 6] = [' ', '.', ',', '\n', '\t', '@'];

pub(crate) struct SqlLanguage {
    backend: Arc<peek_lsp::Backend>,
    /// Written when a connection introspects its database; read on the next keystroke. Until
    /// then it is empty, which is why completions are keywords only and `diagnose` returns
    /// nothing rather than flagging every table as unknown.
    schema: peek_lsp::SharedSchema,
}

impl std::fmt::Debug for SqlLanguage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SqlLanguage")
            .finish_non_exhaustive()
    }
}

impl Global for SqlLanguage {}

impl SqlLanguage {
    pub(crate) fn init(cx: &mut App) {
        let schema = peek_lsp::shared_schema();
        let backend = Arc::new(peek_lsp::Backend::new(Arc::clone(&schema)));
        cx.set_global(Self { backend, schema });
    }

    /// The shared schema index, filled by [`crate::database::Database`] once a connection
    /// answers. Until then it is empty, which is why completions are keyword-only.
    pub(crate) fn schema(cx: &App) -> peek_lsp::SharedSchema {
        Arc::clone(&cx.global::<Self>().schema)
    }

    fn backend(cx: &App) -> Arc<peek_lsp::Backend> {
        Arc::clone(&cx.global::<Self>().backend)
    }
}

/// A query node's document identity.
///
/// Node ids are `query_<nanoid>`, so this is always a valid URI — but it is resolved once when
/// the editor is built rather than in a render path, and a node whose id somehow did not parse
/// simply gets no language support instead of panicking.
pub(crate) fn uri_for(id: &NodeId) -> Option<Uri> {
    Uri::from_str(&format!("peek://query/{id}")).ok()
}

/// Re-parses the document and returns its diagnostics.
///
/// Callers debounce: `DocumentStore::upsert` re-parses from scratch, and the editor resets its
/// diagnostic set on every edit anyway, so there is nothing to be gained from running this per
/// keystroke.
pub(crate) fn sync(uri: &Uri, text: String, cx: &App) -> Vec<lsp_types::Diagnostic> {
    let backend = SqlLanguage::backend(cx);
    backend.did_change(uri.clone(), text);
    backend.diagnostics(uri)
}

/// Drops a node's document when its node leaves the canvas.
pub(crate) fn close(uri: &Uri, cx: &App) {
    SqlLanguage::backend(cx).did_close(uri);
}

/// Bridges `peek-lsp` to the editor's completion menu.
///
/// Nothing is converted on the way through: gpui-base and `peek-lsp` are both built on
/// `lsp-types` 0.97, so a `CompletionItem` from one is a `CompletionItem` to the other.
///
/// It answers for two of the reference's providers, not one: `lspProvider.ts` for SQL, and the
/// `@`-triggered variable provider `SqlEditor.tsx` registers beside it.
pub(crate) struct SqlCompletions {
    uri: Uri,
    node: NodeId,
    /// Whether the completion menu was open when this keystroke arrived, mirrored by
    /// [`super::QueryEditor`] — see [`SqlCompletions::is_completion_trigger`]. It cannot be read
    /// off the editor here: every provider call happens *inside* that entity's own update, and
    /// gpui refuses to hand out a second borrow of it.
    menu_open: Rc<Cell<bool>>,
    /// Where `@variable` names come from. Weak because the editor holds this provider for the
    /// node's whole life, and a strong handle here would keep a page's document alive after the
    /// canvas has moved on from it.
    document: WeakEntity<Document>,
}

impl std::fmt::Debug for SqlCompletions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SqlCompletions")
            .field("node", &self.node)
            .finish_non_exhaustive()
    }
}

impl SqlCompletions {
    /// `None` when the node id does not spell a URI, which leaves the editor without language
    /// support rather than panicking.
    pub(crate) fn new(
        node: NodeId,
        menu_open: Rc<Cell<bool>>,
        document: WeakEntity<Document>,
    ) -> Option<Rc<Self>> {
        Some(Rc::new(Self {
            uri: uri_for(&node)?,
            node,
            menu_open,
            document,
        }))
    }

    /// The canvas variables feeding this node, as `@name` items.
    ///
    /// Filtered and ordered here rather than in `peek-lsp`, which knows nothing about canvas
    /// edges. The reference pins them above SQL suggestions with `sortText: "0_"`; here they are
    /// the only answer while an `@` is being typed, which is the same outcome.
    fn variable_items(&self, prefix: &str, cx: &App) -> Vec<CompletionItem> {
        let Some(document) = self.document.upgrade() else {
            return Vec::new();
        };
        let lowered = prefix.to_lowercase();
        document
            .read(cx)
            .variables_for(&self.node)
            .into_keys()
            .filter(|name| name.to_lowercase().starts_with(&lowered))
            .map(|name| {
                let label = format!("@{name}");
                // The menu highlights `0..filter_text.len()` of the label, so this is the `@`
                // plus what was typed. Counted in characters, never bytes.
                let matched: String = label.chars().take(1 + prefix.chars().count()).collect();
                CompletionItem {
                    label,
                    kind: Some(CompletionItemKind::VARIABLE),
                    insert_text: Some(name),
                    detail: Some("variable".to_string()),
                    filter_text: Some(matched),
                    ..Default::default()
                }
            })
            .collect()
    }
}

impl CompletionProvider for SqlCompletions {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        _trigger: CompletionContext,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<anyhow::Result<CompletionResponse>> {
        let source = text.to_string();
        // Owned so `source` can be handed to the backend below; the prefix borrows it otherwise.
        let variable_prefix = peek_lsp::variable_prefix_at(&source, offset).map(str::to_owned);

        let Some(prefix) = variable_prefix else {
            let backend = SqlLanguage::backend(cx);
            // The document is re-sent rather than relied upon: the 30 ms sync debounce means the
            // cached tree can be a few keystrokes stale, and the cursor context changes with
            // every character. `lspProvider.ts` ships the whole text on every request for the
            // same reason.
            backend.did_change(self.uri.clone(), source);
            // Synchronous: a query-sized document re-parses in microseconds, so there is nothing
            // to wait for. `Backend` is `Sync`, so this can move to a background task if a
            // pathological query ever costs a frame.
            let items = backend.completion_at_offset(&self.uri, offset);
            return Task::ready(Ok(CompletionResponse::Array(items)));
        };

        let mut items = self.variable_items(&prefix, cx);
        // `@` is not an identifier character, so the anchored range covers the name alone and
        // the `@` the user typed survives — `SqlEditor.tsx` computes the same range by hand.
        peek_lsp::anchor_to_typed_prefix(&mut items, &source, offset);
        Task::ready(Ok(CompletionResponse::Array(items)))
    }

    fn is_completion_trigger(&self, _offset: usize, new_text: &str, _cx: &mut App) -> bool {
        // A deletion. Re-query so a menu narrowed by the old prefix widens again as characters
        // come off — nothing else on the deletion path refreshes or closes it. An already-closed
        // menu stays closed, which is the rule Monaco's suggest model uses.
        if new_text.is_empty() {
            return self.menu_open.get();
        }
        is_trigger(new_text)
    }
}

/// Whether typing `new_text` should open or refresh the completion menu.
fn is_trigger(new_text: &str) -> bool {
    let Some(last) = new_text.chars().last() else {
        return false;
    };
    last.is_alphanumeric() || last == '_' || TRIGGERS.contains(&last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_keep_the_menu_open() {
        assert!(is_trigger("s"));
        assert!(is_trigger("select"));
        assert!(is_trigger("_id"));
        assert!(is_trigger("1"));
    }

    #[test]
    fn the_reference_trigger_characters_open_the_menu() {
        for trigger in [" ", ".", ",", "\n", "\t", "@"] {
            assert!(is_trigger(trigger), "{trigger:?} should trigger");
        }
    }

    #[test]
    fn punctuation_that_ends_a_statement_does_not_trigger() {
        assert!(!is_trigger(";"));
        assert!(!is_trigger(")"));
        assert!(!is_trigger("'"));
    }

    /// Typing nothing is not a trigger. A *deletion* is handled a level up, in
    /// `is_completion_trigger`, which re-queries only while a menu is already open.
    #[test]
    fn an_empty_edit_does_not_trigger() {
        assert!(!is_trigger(""));
    }

    #[test]
    fn a_node_id_becomes_a_peek_uri() {
        let id = NodeId::query();
        let uri = uri_for(&id).expect("node ids are valid uris");
        assert_eq!(uri.as_str(), format!("peek://query/{id}"));
    }
}
