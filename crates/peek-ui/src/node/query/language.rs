//! The SQL language server behind every query editor.
//!
//! `peek-lsp` is synchronous, window-free and keyed by document URI, so one `Backend` serves
//! every query node in the process — the shape the Tauri host used, with its commands replaced
//! by direct calls.

use std::rc::Rc;
use std::str::FromStr;
use std::sync::Arc;

use gpui_kit::base::input::{CompletionProvider, Rope};
use gpui_kit::{App, Global, Task, Window};
use peek_document::NodeId;
use peek_lsp::lsp_types::{self, CompletionContext, CompletionResponse, Uri};

/// The characters that open the completion menu, from `lspProvider.ts`'s `triggerCharacters`
/// plus Monaco's `quickSuggestions`, which keeps it open while an identifier is being typed.
const TRIGGERS: [char; 5] = [' ', '.', ',', '\n', '\t'];

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
pub(crate) struct SqlCompletions {
    uri: Uri,
}

impl SqlCompletions {
    pub(crate) fn new(uri: Uri) -> Rc<Self> {
        Rc::new(Self { uri })
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
        let backend = SqlLanguage::backend(cx);
        // The document is re-sent rather than relied upon: the 30 ms sync debounce means the
        // cached tree can be a few keystrokes stale, and the cursor context changes with every
        // character. `lspProvider.ts` ships the whole text on every request for the same reason.
        backend.did_change(self.uri.clone(), text.to_string());
        let items = backend.completion_at_offset(&self.uri, offset);
        // Synchronous: a query-sized document re-parses in microseconds, so there is nothing
        // to wait for. `Backend` is `Sync`, so this can move to a background task if a
        // pathological query ever costs a frame.
        Task::ready(Ok(CompletionResponse::Array(items)))
    }

    fn is_completion_trigger(&self, _offset: usize, new_text: &str, _cx: &mut App) -> bool {
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
        for trigger in [" ", ".", ",", "\n", "\t"] {
            assert!(is_trigger(trigger), "{trigger:?} should trigger");
        }
    }

    #[test]
    fn punctuation_that_ends_a_statement_does_not_trigger() {
        assert!(!is_trigger(";"));
        assert!(!is_trigger(")"));
        assert!(!is_trigger("'"));
    }

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
