//! The Query node: `~/labs/peek/src/canvas/nodes/Query/QueryNode.tsx`.
//!
//! A SQL editor with syntax highlighting, completions and diagnostics from [`peek_lsp`], a
//! live-polling toggle in the header and a footer of actions. Running the query needs a
//! database connection and lands with peek-db; everything else works without one.

mod heading;
pub(crate) mod language;

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::TestSupportExt;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Editor, EditorState, InputEvent, RopeExt};
use gpui_kit::component::{Disableable, Selectable, Sizable, StyledExt};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, SharedString, Subscription, Task,
    Window, div, rems,
};
use peek_canvas::Document;
use peek_document::{LiveInterval, NodeData, NodeId, QueryData};
use peek_lsp::lsp_types::Uri;
use peek_theme::ActivePeekTheme;

use super::kind::NodeContext;
use super::state::NodeState;
use super::{BASE_REM, RESIZE_FOOTER_CLEARANCE};

/// `LIVE_POLL_MS` in `QueryNode.tsx`: the one interval the toggle offers.
const LIVE_POLL_MS: u64 = 10_000;
/// `attachLspDocumentSync`'s trailing debounce before re-parsing and re-diagnosing.
const SYNC_DEBOUNCE_MS: u64 = 30;

pub(crate) struct QueryState {
    editor: Entity<QueryEditor>,
}

impl std::fmt::Debug for QueryState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("QueryState").finish_non_exhaustive()
    }
}

impl QueryState {
    pub(crate) fn new(
        id: &NodeId,
        data: &QueryData,
        document: &Entity<Document>,
        window: &mut Window,
        cx: &mut App,
    ) -> Self {
        let node = id.clone();
        let document = document.clone();
        let query = data.query.clone();
        Self {
            editor: cx.new(|cx| QueryEditor::new(node, query, document, window, cx)),
        }
    }

    /// Hands focus to the SQL editor, which is what `Query::Focus` does from the canvas.
    /// `QueryEditor` stashes whatever held focus in its `InputEvent::Focus` arm, so escape
    /// gives it straight back.
    pub(crate) fn focus_editor(&self, window: &mut Window, cx: &mut App) {
        self.editor
            .update(cx, |editor, cx| editor.focus_editor(window, cx));
    }

    /// Drops the node's document from the language server when the node leaves the canvas.
    pub(crate) fn on_removed(&self, cx: &mut App) {
        self.editor.update(cx, |editor, cx| editor.close(cx));
    }
}

pub(crate) fn title(data: &QueryData) -> String {
    let description = data.description.as_deref().unwrap_or_default().trim();
    if !description.is_empty() {
        return description.to_string();
    }
    let heading = heading::heading(&data.query);
    // Trimmed, not just checked for empty: a query of only whitespace joins to a single
    // space, which is truthy in the reference's `||` chain and would leave a blank header.
    if heading.trim().is_empty() {
        return "untitled.sql".to_string();
    }
    heading
}

pub(crate) fn body(
    _id: &NodeId,
    data: &QueryData,
    context: NodeContext<'_>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let Some(NodeState::Query(state)) = context.state else {
        return div().into_any_element();
    };
    let editor = state.editor.clone();
    editor.update(cx, |editor, cx| editor.reconcile(data, window, cx));
    editor.into_any_element()
}

/// The live-polling toggle, `.header-icon-btn` with its `.live-dot`.
///
/// The 10 s tick needs something to execute and arrives with peek-db; the toggle itself writes
/// the persisted field the reference writes, so a document round-trips through both apps.
pub(crate) fn header_extras(
    id: &NodeId,
    data: &QueryData,
    context: NodeContext<'_>,
    _window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let live = matches!(data.live_interval_ms, Some(LiveInterval::EveryMs(_)));
    let theme = cx.peek_theme();
    let document = context.document.clone();
    let node = id.clone();

    let dot =
        div()
            .size(rems(0.375))
            .rounded_full()
            .bg(if live { theme.green } else { theme.fg_subtle });

    Button::new(SharedString::from(format!("{id}-live")))
        .ghost()
        .xsmall()
        .selected(live)
        .child(dot)
        .tooltip(if live {
            "Stop live polling"
        } else {
            "Poll every 10s"
        })
        .on_click(move |_, _, cx| {
            document.update(cx, |document, cx| {
                let changed = document.update_data::<QueryData>(&node, |data| {
                    data.live_interval_ms = if live {
                        None
                    } else {
                        Some(LiveInterval::EveryMs(LIVE_POLL_MS))
                    };
                });
                if changed {
                    // Each toggle is its own undo step rather than coalescing with the
                    // typing around it.
                    document.checkpoint();
                    cx.notify();
                }
            });
        })
        .into_any_element()
}

struct QueryEditor {
    node: NodeId,
    document: Entity<Document>,
    editor: Entity<EditorState>,
    /// `None` when the node id somehow did not form a URI: the editor still works, it just has
    /// no language support.
    uri: Option<Uri>,
    /// What held focus when the editor took it, handed back on escape. Never a handle of the
    /// node's own — that dies with the node and leaves the window focused on nothing.
    restore_focus: Rc<RefCell<Option<FocusHandle>>>,
    /// The document revision this view last adopted text from.
    revision: u64,
    /// Generation counter for the sync debounce, so a stale batch of diagnostics cannot land
    /// on newer text.
    sync: usize,
    /// Holds the pending debounce; dropping it cancels the in-flight one.
    sync_task: Task<()>,
    /// The live-poll tick, restarted whenever `liveIntervalMs` changes. Dropping it stops the
    /// polling, which is what toggling live off does.
    live_task: Task<()>,
    /// The interval the live task is running at, so an unrelated reconcile does not restart it
    /// and reset the countdown.
    live_interval: Option<u64>,
    /// Set when a run was refused because the query is an unbounded `DELETE` or a `TRUNCATE`.
    /// The footer then offers a red "Run unbounded" and the next press goes through — the
    /// reference's two-click confirmation, which is component state there too, so editing the
    /// query clears it.
    confirming_unbounded: bool,
    _subscription: Subscription,
}

impl QueryEditor {
    fn new(
        node: NodeId,
        query: String,
        document: Entity<Document>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let uri = language::uri_for(&node);
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("sql")
                // `lineNumbers: "off"` and `wordWrap: "on"` in `SqlEditor.tsx`. Folding and the
                // find bar are editor chrome a 350-unit card has no room for, and the find bar
                // would take cmd-f from the canvas.
                .line_number(false)
                .folding(false)
                .searchable(false)
                .soft_wrap(true)
                .placeholder("SELECT …")
                .default_value(query)
        });

        if let Some(uri) = uri.clone() {
            editor.update(cx, |state, _| {
                state.lsp_mut().completion_provider = Some(language::SqlCompletions::new(uri));
            });
        }

        let subscription = cx.subscribe_in(&editor, window, Self::on_editor_event);
        let revision = document.read(cx).revision();

        let mut this = Self {
            node,
            document,
            editor,
            uri,
            restore_focus: Rc::new(RefCell::new(None)),
            revision,
            sync: 0,
            sync_task: Task::ready(()),
            live_task: Task::ready(()),
            live_interval: None,
            confirming_unbounded: false,
            _subscription: subscription,
        };
        // The reference syncs once before subscribing, so a node opens with its diagnostics.
        this.schedule_sync(cx);
        this
    }

    fn on_editor_event(
        &mut self,
        _editor: &Entity<EditorState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                let text = self.editor.read(cx).value().to_string();
                self.write(text, cx);
                self.schedule_sync(cx);
            }
            // Seals the undo transaction a run of keystrokes opened, so one visit to the
            // editor is one undo step.
            InputEvent::Blur => self
                .document
                .update(cx, |document, _| document.checkpoint()),
            InputEvent::Focus => {
                let previous = window.focused(cx);
                // A click reaches here before the window's focus moves, so `previous` is the
                // canvas. `Query::Focus` moves it first and the event lands afterwards, so
                // `previous` is the editor itself — parking that would make escape a no-op,
                // and `focus_editor` has already stashed the right handle.
                if previous.as_ref() != Some(&self.editor.focus_handle(cx)) {
                    *self.restore_focus.borrow_mut() = previous;
                }
            }
            InputEvent::PressEnter { .. } => {}
        }
    }

    /// Takes focus on behalf of `Query::Focus`, remembering what held it first so escape can
    /// hand it straight back.
    fn focus_editor(&mut self, window: &mut Window, cx: &mut App) {
        *self.restore_focus.borrow_mut() = window.focused(cx);
        let handle = self.editor.focus_handle(cx);
        window.focus(&handle, cx);
    }

    fn write(&mut self, text: String, cx: &mut Context<Self>) {
        // Any edit withdraws the confirmation, so a `WHERE` typed after the warning is not run
        // as though it were still the unbounded statement the user was warned about.
        self.confirming_unbounded = false;
        let node = self.node.clone();
        self.document.update(cx, |document, cx| {
            if document.update_data::<QueryData>(&node, |data| data.query = text) {
                cx.notify();
            }
        });
        // The write is this view's own, so re-adopting it on the next frame would be a no-op
        // that still costs a `set_value`; move past it.
        self.revision = self.document.read(cx).revision();
    }

    /// Re-parses and re-diagnoses after a quiet moment.
    ///
    /// The editor resets its diagnostic set on every edit, so diagnostics have to be re-pushed
    /// after each change rather than only when they alter — the debounce is what keeps that
    /// from running per keystroke.
    fn schedule_sync(&mut self, cx: &mut Context<Self>) {
        let Some(uri) = self.uri.clone() else {
            return;
        };
        self.sync = self.sync.wrapping_add(1);
        let generation = self.sync;
        let editor = self.editor.clone();

        self.sync_task = cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(SYNC_DEBOUNCE_MS))
                .await;
            let Ok(current) = this.read_with(cx, |this, _| this.sync) else {
                return;
            };
            if current != generation {
                return;
            }
            cx.update(|cx| {
                let text = editor.read(cx).value().to_string();
                let diagnostics = language::sync(&uri, text, cx);
                editor.update(cx, |state, cx| {
                    let Some(set) = state.diagnostics_mut() else {
                        return;
                    };
                    set.clear();
                    set.extend(diagnostics);
                    cx.notify();
                });
            });
        });
    }

    /// Adopts text written from elsewhere — an undo, later a remote edit.
    ///
    /// Gated on the document revision rather than compared every frame: a keystroke reaches
    /// the editor before the change event that carries it to the document, and a frame can
    /// render in between. The editor is also left alone entirely while it holds focus, which
    /// is what `SqlEditor.tsx` does for the same reason.
    fn reconcile(&mut self, data: &QueryData, window: &mut Window, cx: &mut Context<Self>) {
        self.reconcile_live(data, window, cx);
        let revision = self.document.read(cx).revision();
        if revision == self.revision {
            return;
        }
        self.revision = revision;
        if self.editor.focus_handle(cx).is_focused(window) {
            return;
        }
        if self.editor.read(cx).value().as_ref() == data.query {
            return;
        }
        let query = data.query.clone();
        self.editor
            .update(cx, |state, cx| state.set_value(query, window, cx));
        self.schedule_sync(cx);
    }

    /// Starts, stops or leaves the live-poll tick alone.
    ///
    /// `liveIntervalMs` is persisted, so a document saved with live on comes back polling — the
    /// reference behaves the same way.
    fn reconcile_live(&mut self, data: &QueryData, window: &mut Window, cx: &mut Context<Self>) {
        let interval = match data.live_interval_ms {
            Some(LiveInterval::EveryMs(ms)) => Some(ms),
            Some(LiveInterval::Off) | None => None,
        };
        if interval == self.live_interval {
            return;
        }
        self.live_interval = interval;
        let Some(interval) = interval else {
            self.live_task = Task::ready(());
            return;
        };

        self.live_task = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(interval))
                    .await;
                if this.update_in(cx, QueryEditor::live_tick).is_err() {
                    return;
                }
            }
        });
    }

    /// One poll.
    ///
    /// Skipped while the editor has focus (the user is mid-edit), while a run is already in
    /// flight, and for anything that is not a plain `SELECT` — `isSelectOnly` in the reference,
    /// which is what stops a live toggle left on a `DELETE` from running it every ten seconds.
    fn live_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.focus_handle(cx).is_focused(window) {
            return;
        }
        let is_select = self
            .document
            .read(cx)
            .node(&self.node)
            .and_then(|node| QueryData::get(&node.kind))
            .is_some_and(|data| {
                data.is_running != Some(true)
                    && data
                        .query
                        .trim_start()
                        .to_ascii_lowercase()
                        .starts_with("select")
            });
        if !is_select {
            return;
        }
        // Already confirmed by construction: a SELECT is never an unbounded write.
        self.start_run(cx);
    }

    fn close(&mut self, cx: &mut App) {
        if let Some(uri) = &self.uri {
            language::close(uri, cx);
        }
    }

    /// `Query::Run`, bound on the node context so it fires while the editor has focus — which
    /// is where `cmd-enter` is pressed.
    fn run(
        &mut self,
        _: &crate::commands::actions::query::Run,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_run(cx);
        cx.stop_propagation();
    }

    fn start_run(&mut self, cx: &mut Context<Self>) {
        let confirmed = self.confirming_unbounded;
        let document = self.document.clone();
        let node = self.node.clone();
        let outcome = crate::execution::run(&document, &node, confirmed, cx);
        self.confirming_unbounded = outcome == crate::execution::Run::NeedsConfirmation;
        cx.notify();
    }

    /// `Query::Format`, the other command that fires while the editor has focus.
    ///
    /// Applied through the editor rather than the document because the editor is authoritative
    /// while focused; its `Change` event then carries the formatted text to the document.
    fn format(
        &mut self,
        _: &crate::commands::actions::query::Format,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = self.editor.read(cx).value().to_string();
        let formatted = peek_lsp::format(&current);
        if formatted != current {
            self.editor
                .update(cx, |state, cx| state.set_value(formatted, window, cx));
            let text = self.editor.read(cx).value().to_string();
            self.write(text, cx);
            self.schedule_sync(cx);
            self.document
                .update(cx, |document, _| document.checkpoint());
        }
        cx.stop_propagation();
    }

    /// Escape: hand focus back to whatever had it, so a second escape reaches the canvas and
    /// clears the selection.
    fn escape(
        &mut self,
        _: &crate::commands::actions::tool::Select,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(handle) = self.restore_focus.borrow_mut().take() else {
            return;
        };
        window.focus(&handle, cx);
        cx.stop_propagation();
    }

    /// Format and Run, `.app-node-footer`.
    ///
    /// It sits above the bottom resize band, which the canvas claims for resizing: a control
    /// painted into those last `RESIZE_FOOTER_CLEARANCE` world units never sees a press.
    fn footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.peek_theme();
        div()
            .h_flex()
            .justify_end()
            .gap(rems(0.25))
            .flex_none()
            .h(rems(1.75))
            .px(rems(0.375))
            .mb(rems(RESIZE_FOOTER_CLEARANCE / BASE_REM))
            .border_t_1()
            .border_color(theme.node_border)
            .child(
                Button::new(SharedString::from(format!("{}-format", self.node)))
                    .ghost()
                    .xsmall()
                    .label("Format")
                    .tooltip("Format query")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.format(&crate::commands::actions::query::Format, window, cx);
                    })),
            )
            .child(self.run_button(cx))
    }

    /// Run, or the red confirmation the unbounded-write gate asks for.
    fn run_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let id = SharedString::from(format!("{}-run", self.node));
        let connected = crate::database::Database::is_connected(cx);
        let running = self
            .document
            .read(cx)
            .node(&self.node)
            .and_then(|node| QueryData::get(&node.kind))
            .is_some_and(|data| data.is_running == Some(true));

        if self.confirming_unbounded {
            return Button::new(id)
                .danger()
                .xsmall()
                .label("Run unbounded")
                .disabled(running)
                .tooltip("Do you want to run this unbounded delete operation?")
                .on_click(cx.listener(|this, _, _, cx| this.start_run(cx)));
        }

        Button::new(id)
            .primary()
            .xsmall()
            .label(if running { "Running…" } else { "Run" })
            .disabled(!connected || running)
            .tooltip(if connected {
                "Run query"
            } else {
                "Running a query needs a database connection"
            })
            .on_click(cx.listener(|this, _, _, cx| this.start_run(cx)))
    }
}

impl Render for QueryEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id(SharedString::from(format!("{}-body", self.node)))
            .test_support()
            // Deliberately not focusable: the editor owns the focus handle and this sits above
            // it on the dispatch path, which is what makes the key context and the actions
            // below reachable while typing. A handle here would die with the node.
            .key_context("QueryNode")
            .on_action(cx.listener(Self::format))
            .on_action(cx.listener(Self::run))
            .on_action(cx.listener(Self::escape))
            .v_flex()
            .size_full()
            .overflow_hidden()
            .child(
                div()
                    .id(SharedString::from(format!("{}-editor", self.node)))
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    // A click anywhere over the code puts the cursor in the editor. The
                    // canvas resolves presses in world space and its window-level listeners
                    // run first, so the editor never sees the press itself and cannot place
                    // the caret where it landed — the same reason the Text node focuses its
                    // input by hand. The caret goes to the end of the query, which is where
                    // someone clicking into one to keep typing wants it.
                    .on_click(cx.listener(|this, _, window, cx| {
                        if this.editor.focus_handle(cx).is_focused(window) {
                            return;
                        }
                        this.editor.update(cx, |state, cx| {
                            let end = state.text().offset_to_position(state.text().len());
                            state.set_cursor_position(end, window, cx);
                        });
                    }))
                    .child(
                        Editor::new(&self.editor)
                            // Without an explicit height the editor lays out `h_auto` and
                            // renders only the rows that fit its intrinsic height, clipping
                            // the rest — visible as text cut mid-glyph once the camera zooms
                            // in and the lines grow.
                            .size_full()
                            .appearance(false)
                            .bordered(false)
                            // Rems, not pixels: the canvas lays nodes out in a rem scope of
                            // `BASE_REM * zoom`, so this is the unit that tracks the camera.
                            .text_size(rems(0.8125)),
                    ),
            )
            .child(self.footer(cx))
    }
}

#[cfg(test)]
mod tests {
    use super::title;
    use peek_document::QueryData;

    fn data(query: &str, description: Option<&str>) -> QueryData {
        QueryData {
            query: query.to_string(),
            description: description.map(ToString::to_string),
            ..QueryData::default()
        }
    }

    #[test]
    fn a_description_wins_over_the_query() {
        assert_eq!(
            title(&data("select 1", Some("Active users"))),
            "Active users"
        );
    }

    #[test]
    fn a_blank_description_falls_through_to_the_query() {
        assert_eq!(title(&data("select 1", Some("   "))), "select 1");
    }

    #[test]
    fn the_query_becomes_the_title_when_there_is_no_description() {
        assert_eq!(
            title(&data("select *\nfrom users", None)),
            "select * from users"
        );
    }

    #[test]
    fn a_leading_comment_titles_the_node() {
        assert_eq!(
            title(&data("-- Active users\nselect 1", None)),
            "Active users select 1"
        );
    }

    #[test]
    fn an_empty_query_reads_as_untitled() {
        assert_eq!(title(&data("", None)), "untitled.sql");
        assert_eq!(title(&data("   \n ", None)), "untitled.sql");
    }
}

#[cfg(test)]
mod layout_tests {
    use super::QueryEditor;
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, TestAppContext, px, size};
    use peek_canvas::Document;
    use peek_document::{CanvasDocument, NodeId};

    const THREE_LINES: &str = "select 1\nselect 2\nselect 3\nselect 4\nselect 5";

    const DOCUMENT: &str = r#"{
      "version": 1,
      "activePageId": "page_test0001",
      "pageOrder": ["page_test0001"],
      "pages": {
        "page_test0001": {
          "id": "page_test0001",
          "name": "Page 1",
          "nodes": [],
          "edges": [],
          "viewport": { "x": 0, "y": 0, "zoom": 1 }
        }
      }
    }"#;

    /// The editor must fill the height it is given rather than shrink to its text.
    ///
    /// `Editor` lays out `h_auto` unless told otherwise, rendering only the rows that fit its
    /// intrinsic height and clipping the rest — invisible on a one-line query at zoom 1, and
    /// obvious as text cut mid-glyph once the camera zooms in.
    ///
    /// Asserted on `visible_row_range` rather than on bounds: the `Editor` element registers
    /// no id of its own (it appears in the tree only as a path segment, with an unstable
    /// integer leaf below it), and the wrapper around it fills its parent whether or not the
    /// bug is present — so a bounds assertion one level out passes either way.
    #[gpui_kit::test]
    fn the_editor_renders_every_row_of_a_tall_query(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let config = peek_config::PeekConfig::default();
            crate::init(&config, cx);
        });

        let mut editor = None;
        let handle = cx.open_window(size(px(600.0), px(400.0)), |window, cx| {
            let document = cx.new(|_| {
                Document::load(CanvasDocument::from_json(DOCUMENT).expect("fixture parses"))
            });
            let view = cx.new(|cx| {
                QueryEditor::new(
                    NodeId::from("query_aaaaaaaa"),
                    THREE_LINES.to_string(),
                    document,
                    window,
                    cx,
                )
            });
            editor = Some(view.clone());
            Root::new(view, window, cx)
        });
        cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
            .unwrap();

        let visible = cx
            .update(|cx| {
                editor
                    .as_ref()
                    .expect("editor built")
                    .read(cx)
                    .editor
                    .read(cx)
                    .visible_row_range()
            })
            .expect("the editor laid out");

        assert!(
            visible.end >= 5,
            "all five rows should be laid out, got {visible:?}"
        );
    }
}
