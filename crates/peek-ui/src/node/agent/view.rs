//! The agent node's retained state: one entity per node, holding the conversation being drawn,
//! the turn in flight, and whichever backend the node runs on.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use gpui_kit::TestSupportExt;
use gpui_kit::component::StyledExt;
use gpui_kit::component::input::{InputEvent, TextareaState};
use gpui_kit::component::message_scroller::MessageScrollerState;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, FocusHandle, Focusable, SharedString, Subscription, Task, Window,
};
use peek_acp::{AgentEvent, Change, Transcript};
use peek_canvas::Document;
use peek_document::{AgentData, AgentMessage, AgentProvider, NodeId};
use peek_theme::ActivePeekTheme;

use super::acp::Acp;
use super::backend::Agents;
use super::rows::{self, Builder, Row};

/// Which backend a node talks to. `Unconfigured` is the state with nothing in `settings.json`.
#[derive(Debug)]
pub(crate) enum Backend {
    Acp(Acp),
    Ollama,
    Unconfigured,
}

pub(crate) struct AgentView {
    node: NodeId,
    document: Entity<Document>,
    /// The document revision this view last adopted from. Own writes push it past themselves, so
    /// committing a turn never looks like an outside edit worth rebuilding for.
    revision: u64,
    /// The conversation being drawn: what the document holds, then this turn's staged messages.
    /// `Rc` because the scroller's row renderer is `'static` and cannot borrow the view.
    messages: Rc<Vec<AgentMessage>>,
    rows: Rc<Vec<Row>>,
    builder: Builder,
    /// Where the committed messages end and the staged ones begin, so a `Transcript` index can
    /// be turned into an index into `messages`.
    staged_base: usize,
    scroller: Entity<MessageScrollerState>,
    composer: Entity<TextareaState>,
    /// Tool disclosures the user opened, keyed by tool-call id.
    ///
    /// Kept here rather than in element state because the list is virtualized: a row scrolled
    /// out of view is dropped, and with it anything the element was remembering.
    expanded: HashSet<SharedString>,
    backend: Backend,
    /// The streaming accumulator for the turn in flight. Owns every message staged but not yet
    /// committed, including the user's own turn.
    transcript: Transcript,
    /// Set when a turn starts, cleared when the prompt resolves. `stop` deliberately leaves it
    /// set: a turn ends when the agent says it has, which is what the reference does.
    loading: bool,
    /// The round trip in flight. Dropping it abandons it, which is what node removal does.
    prompt: Task<()>,
    /// A line above the transcript: a failed start, or MCP being off.
    warning: Option<SharedString>,
    /// What held focus before the composer took it, handed back on escape. Never a handle of
    /// the node's own — that dies with the node.
    restore_focus: Rc<RefCell<Option<FocusHandle>>>,
    /// The zoom the list last measured at. Rows are measured in pixels but laid out inside a rem
    /// scope that scales with the camera, so a zoom change invalidates every height.
    zoom: f64,
    _composer_events: Subscription,
}

impl std::fmt::Debug for AgentView {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentView")
            .field("node", &self.node)
            .field("messages", &self.messages.len())
            .field("loading", &self.loading)
            .field("backend", &self.backend)
            .finish_non_exhaustive()
    }
}

impl AgentView {
    pub(super) fn new(
        node: NodeId,
        data: &AgentData,
        document: Entity<Document>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 8)
                .soft_wrap(true)
                .placeholder("Ask about your data, or describe what to build...")
        });
        let messages = Rc::new(data.messages.clone());
        let (builder, rows) = rows::build(&messages);
        let scroller = cx.new(|cx| MessageScrollerState::new(rows.len(), cx));
        let revision = document.read(cx).revision();

        Self {
            backend: match Agents::resolve(data, cx) {
                Some(AgentProvider::Acp) => Backend::Acp(Acp::default()),
                Some(AgentProvider::Ollama) => Backend::Ollama,
                None => Backend::Unconfigured,
            },
            node,
            document,
            revision,
            staged_base: messages.len(),
            messages,
            rows: Rc::new(rows),
            builder,
            scroller,
            _composer_events: cx.subscribe_in(&composer, window, Self::on_composer_event),
            composer,
            expanded: HashSet::new(),
            transcript: Transcript::new(),
            loading: false,
            prompt: Task::ready(()),
            warning: None,
            restore_focus: Rc::new(RefCell::new(None)),
            zoom: 0.0,
        }
    }

    pub(super) fn node(&self) -> &NodeId {
        &self.node
    }

    /// How many blocks the transcript draws, for tests that assert on folding rather than on
    /// pixels.
    #[cfg(test)]
    pub(crate) fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// The text streaming in right now.
    #[cfg(test)]
    pub(crate) fn preview(&self) -> &str {
        self.transcript.preview_text()
    }

    /// Feeds an update straight in, as the backend's drain would.
    ///
    /// The streaming paths are worth testing and spawning a real agent is not, so this is the
    /// seam: everything downstream of the channel runs for real.
    #[cfg(test)]
    pub(crate) fn pump(&mut self, update: peek_acp::AcpUpdate, cx: &mut Context<Self>) {
        let change = self.transcript.apply(update, now_ms());
        self.absorb(change, cx);
    }

    pub(super) fn is_loading(&self) -> bool {
        self.loading
    }

    pub(super) fn backend(&self) -> &Backend {
        &self.backend
    }

    pub(super) fn backend_mut(&mut self) -> &mut Backend {
        &mut self.backend
    }

    pub(super) fn set_warning(&mut self, warning: Option<SharedString>) {
        self.warning = warning;
    }

    pub(super) fn toggle_expanded(&mut self, id: &SharedString, cx: &mut Context<Self>) {
        if !self.expanded.remove(id) {
            self.expanded.insert(id.clone());
        }
        // A disclosure changes a row's height, and a virtualized list cannot infer that.
        if let Some(row) = self.row_of_tool(id) {
            self.scroller.update(cx, |state, cx| {
                state.remeasure_items(row..row + 1, cx);
            });
        }
        cx.notify();
    }

    fn row_of_tool(&self, id: &SharedString) -> Option<usize> {
        let message = self.messages.iter().position(|message| {
            message.tool_call_id.as_deref() == Some(id.as_ref())
                || (message.is("tool_call")
                    && message
                        .tool_calls
                        .as_ref()
                        .is_some_and(|calls| calls.iter().any(|call| call.id == id.as_ref())))
        })?;
        self.builder.row_of(message)
    }

    pub(super) fn composer(&self) -> &Entity<TextareaState> {
        &self.composer
    }

    // ---- the turn --------------------------------------------------------------------------

    /// An event from the backend, routed here by session id.
    pub(crate) fn receive(&mut self, event: AgentEvent, cx: &mut Context<Self>) {
        match event {
            AgentEvent::Update { update, .. } => {
                let change = self.transcript.apply(update, now_ms());
                self.absorb(change, cx);
            }
            AgentEvent::Permission { id, request, .. } => {
                if let Backend::Acp(acp) = &mut self.backend {
                    acp.pending = Some((id, request));
                }
                cx.notify();
            }
        }
    }

    /// The one place a [`Change`] becomes list bookkeeping. Nothing else touches the scroller.
    pub(super) fn absorb(&mut self, change: Change, cx: &mut Context<Self>) {
        if change == Change::default() {
            return;
        }

        if change.appended > 0 {
            let staged = self.transcript.messages();
            let first = staged.len() - change.appended;
            let mut added = 0;
            for offset in 0..change.appended {
                let index = self.staged_base + first + offset;
                Rc::make_mut(&mut self.messages).push(staged[first + offset].clone());
                if let Some(row) = self.builder.push(index, &self.messages) {
                    Rc::make_mut(&mut self.rows).push(row);
                    added += 1;
                }
            }
            if added > 0 {
                self.scroller.update(cx, |state, cx| {
                    state.append(added, cx);
                });
            }
        }

        if let Some(patched) = change.patched {
            let index = self.staged_base + patched;
            if index < self.messages.len() {
                Rc::make_mut(&mut self.messages)[index] =
                    self.transcript.messages()[patched].clone();
                if let Some(row) = self.builder.row_of(index) {
                    self.scroller.update(cx, |state, cx| {
                        state.remeasure_items(row..row + 1, cx);
                    });
                }
            }
        }

        // `streamed` and `preview_changed` move only the live preview, which is pinned below
        // the list rather than being a row in it — so a token costs one repaint and no
        // remeasure at all.
        if change.mode
            && let Backend::Acp(acp) = &mut self.backend
        {
            acp.adopt_mode(self.transcript.mode());
        }
        cx.notify();
    }

    /// Stages a message of the app's own — a user turn, or a failure worth keeping.
    pub(crate) fn stage(&mut self, message: AgentMessage, cx: &mut Context<Self>) {
        let change = self.transcript.push(message);
        self.absorb(change, cx);
    }

    pub(super) fn set_loading(&mut self, loading: bool, cx: &mut Context<Self>) {
        self.loading = loading;
        cx.notify();
    }

    /// The conversation as the document holds it, which is what a fresh turn replays to the
    /// model. Read before the user's own turn is staged.
    pub(super) fn committed_messages(&self, cx: &App) -> Vec<AgentMessage> {
        self.document
            .read(cx)
            .node(&self.node)
            .and_then(|node| peek_document::NodeData::get(&node.kind).cloned())
            .map(|data: AgentData| data.messages)
            .unwrap_or_default()
    }

    pub(super) fn document_handle(&self) -> Entity<Document> {
        self.document.clone()
    }

    /// A token from the local backend. It has no transcript of its own, so the chunk goes
    /// through the same accumulator the ACP path uses and lands in the same preview.
    pub(super) fn stream_chunk(&mut self, text: &str, cx: &mut Context<Self>) {
        let change = self.transcript.apply(
            peek_acp::AcpUpdate::MessageChunk(text.to_string()),
            now_ms(),
        );
        self.absorb(change, cx);
    }

    pub(super) fn hold_prompt(&mut self, task: Task<()>) {
        self.prompt = task;
    }

    /// Closes the turn: flushes whatever was still buffered and commits it to the document as
    /// one undo step.
    pub(crate) fn finish_turn(&mut self, cx: &mut Context<Self>) {
        let change = self.transcript.finish(now_ms());
        self.absorb(change, cx);

        let staged = self.transcript.take();
        if !staged.is_empty() {
            let node = self.node.clone();
            self.document.update(cx, |document, cx| {
                document.update_data::<AgentData>(&node, |data| data.messages.extend(staged));
                document.checkpoint();
                cx.notify();
            });
        }
        // Our own write: move past it so `reconcile` does not read it back as an outside edit.
        self.revision = self.document.read(cx).revision();
        self.staged_base = self.messages.len();
        self.loading = false;
        cx.notify();
    }

    // ---- reconcile -------------------------------------------------------------------------

    /// Adopts the document when something other than this view changed it: undo, a fork, an
    /// agent editing the node through MCP.
    pub(super) fn reconcile(&mut self, data: &AgentData, zoom: f64, cx: &mut Context<Self>) {
        if (zoom - self.zoom).abs() > f64::EPSILON {
            self.zoom = zoom;
            self.scroller.update(cx, MessageScrollerState::remeasure);
        }

        let revision = self.document.read(cx).revision();
        if revision == self.revision {
            return;
        }
        self.revision = revision;

        let mut messages = data.messages.clone();
        messages.extend(self.transcript.messages().iter().cloned());
        if messages == *self.messages {
            return;
        }

        self.staged_base = data.messages.len();
        self.messages = Rc::new(messages);
        let (builder, rows) = rows::build(&self.messages);
        self.builder = builder;
        self.rows = Rc::new(rows);
        let count = self.rows.len();
        self.scroller.update(cx, |state, cx| state.reset(count, cx));
        cx.notify();
    }

    // ---- composer --------------------------------------------------------------------------

    fn on_composer_event(
        &mut self,
        _: &Entity<TextareaState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            // Plain enter sends; shift-enter has already inserted its newline by the time this
            // arrives, and does not reach here at all.
            InputEvent::PressEnter { .. } => self.send(window, cx),
            InputEvent::Focus => {
                let focused = window.focused(cx);
                let composer = self.composer.focus_handle(cx);
                if focused.as_ref() != Some(&composer) {
                    *self.restore_focus.borrow_mut() = focused;
                }
            }
            _ => {}
        }
    }

    /// Sends whatever is in the composer.
    ///
    /// The user's turn is staged *before* the session is ensured, so a failed start leaves the
    /// question followed by the error that explains it — the reference persists the same pair,
    /// and a question that silently vanished would be worse.
    pub(super) fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        let text = self.composer.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        self.composer
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));

        self.stage(AgentMessage::new("user", text.clone(), now_ms()), cx);
        self.set_loading(true, cx);
        match self.backend {
            Backend::Ollama => super::ollama::ask(self, text, window, cx),
            _ => super::acp::ask(self, text, window, cx),
        }
    }

    pub(super) fn stop(&mut self, cx: &mut Context<Self>) {
        super::acp::stop(self, cx);
    }

    /// Hands focus back to whatever had it before the composer took it.
    pub(super) fn release_focus(&mut self, window: &mut Window, cx: &mut App) {
        if let Some(handle) = self.restore_focus.borrow_mut().take() {
            window.focus(&handle, cx);
        }
    }

    /// Last rites. A node deleted mid-turn has to stop its agent and leave the routing table,
    /// or the backend keeps streaming into a view nobody can see.
    pub(super) fn close(&mut self, cx: &mut Context<Self>) {
        let Backend::Acp(acp) = &self.backend else {
            return;
        };
        let Some(session_id) = acp.session_id().map(str::to_string) else {
            return;
        };
        if let Some(session) = Agents::session(cx) {
            drop(session.cancel(session_id.clone()));
        }
        Agents::unregister(&session_id, cx);
    }
}

/// The timestamps the transcript stamps messages with, in the units the on-disk format uses:
/// `Date.now()` milliseconds, because the TypeScript app reads the same files.
pub(super) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(0))
}

impl gpui_kit::Render for AgentView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The colour is copied out rather than held: `peek_theme()` borrows the app, and
        // everything below needs it mutably to build its children.
        let background = cx.peek_theme().node_bg;
        let unconfigured = matches!(self.backend, Backend::Unconfigured);

        let warning = self.warning.clone().map(|warning| banner(&warning, cx));
        let transcript = if unconfigured {
            super::empty::unconfigured(cx)
        } else {
            self.transcript_element(window, cx)
        };
        let preview = self.preview_element(cx);
        let thinking = self.thinking_element(cx);
        let permission = self.permission_element(cx);
        let composer = (!unconfigured).then(|| super::composer::render(self, window, cx));

        gpui_kit::div()
            .id(SharedString::from(format!("{}-body", self.node)))
            .test_support()
            .v_flex()
            .size_full()
            .min_h_0()
            .bg(background)
            // Deliberately not focusable: the composer owns the focus handle, and a handle here
            // would die with the node. This sits above it on the dispatch path instead.
            .key_context(crate::commands::AGENT_NODE)
            .on_action(cx.listener(Self::on_cycle_mode))
            .on_action(cx.listener(Self::on_stop))
            .on_action(cx.listener(Self::on_escape))
            // The canvas pans on a wheel, so a transcript that did not absorb its own scroll
            // would drag the whole page instead of moving.
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .children(warning)
            .child(transcript)
            .children(preview)
            .children(thinking)
            .children(permission)
            .children(composer)
    }
}

impl AgentView {
    fn transcript_element(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        if self.rows.is_empty() {
            return super::empty::waiting(cx);
        }
        let rows = Rc::clone(&self.rows);
        let messages = Rc::clone(&self.messages);
        let expanded = self.expanded.clone();
        let loading = self.loading;
        let node = self.node.clone();
        let view = cx.weak_entity();

        gpui_kit::div()
            .flex_1()
            .min_h_0()
            .child(
                gpui_kit::component::message_scroller::MessageScroller::new(
                    SharedString::from(format!("{node}-transcript")),
                    self.scroller.clone(),
                    move |index, window, cx| {
                        super::render_row(
                            &rows,
                            &messages,
                            (index, &expanded, loading),
                            view.clone(),
                            window,
                            cx,
                        )
                    },
                )
                .scrollbar(true)
                .jump_button(true)
                .with_bottom_fade(cx.peek_theme().node_bg),
            )
            .into_any_element()
    }

    /// The text streaming in right now, pinned below the list rather than being a row in it, so
    /// a chunk costs one repaint and no remeasure. It becomes a real row when the transcript
    /// flushes it at a boundary.
    fn preview_element(&self, cx: &App) -> Option<gpui_kit::AnyElement> {
        let text = self.transcript.preview_text();
        if text.is_empty() {
            return None;
        }
        let theme = cx.peek_theme();
        let thought = self.transcript.preview() == Some(peek_acp::Preview::Thought);
        Some(
            gpui_kit::div()
                .px(gpui_kit::rems(0.75))
                .py(gpui_kit::rems(0.4))
                .text_size(gpui_kit::rems(0.75))
                .when(thought, |this| this.italic().text_color(theme.fg_muted))
                .when(!thought, |this| this.text_color(theme.fg))
                .child(text.to_string())
                .into_any_element(),
        )
    }

    /// Only before the first token of a turn: once text is arriving, the text is the progress.
    fn thinking_element(&self, cx: &App) -> Option<gpui_kit::AnyElement> {
        if !self.loading || !self.transcript.preview_text().is_empty() {
            return None;
        }
        Some(
            gpui_kit::component::marker::Marker::new()
                .id(SharedString::from(format!("{}-thinking", self.node)))
                .role(gpui_kit::Role::Status)
                .loading(true)
                .with_loading_style(gpui_kit::component::marker::MarkerLoadingStyle::Shimmer)
                .child(
                    gpui_kit::div()
                        .px(gpui_kit::rems(0.75))
                        .text_size(gpui_kit::rems(0.72))
                        .text_color(cx.peek_theme().fg_muted)
                        .child("Thinking"),
                )
                .into_any_element(),
        )
    }

    fn permission_element(&self, cx: &mut Context<Self>) -> Option<gpui_kit::AnyElement> {
        let Backend::Acp(acp) = &self.backend else {
            return None;
        };
        let request = acp.pending.as_ref().map(|(_, request)| request.clone())?;
        Some(super::permission::render(&request, cx))
    }

    fn on_cycle_mode(
        &mut self,
        _: &crate::commands::actions::agent::CycleMode,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        super::acp::cycle_mode(self, cx);
    }

    fn on_stop(
        &mut self,
        _: &crate::commands::actions::agent::Stop,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.stop(cx);
    }

    /// Escape hands focus back and stops there, so a second press reaches the canvas and clears
    /// the selection — the query editor behaves the same way.
    fn on_escape(
        &mut self,
        _: &crate::commands::actions::tool::Select,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.composer.focus_handle(cx).is_focused(window) {
            self.release_focus(window, cx);
            cx.stop_propagation();
        }
    }
}

fn banner(warning: &SharedString, cx: &App) -> gpui_kit::AnyElement {
    let theme = cx.peek_theme();
    gpui_kit::div()
        .h_flex()
        .items_center()
        .gap(gpui_kit::rems(0.4))
        .px(gpui_kit::rems(0.75))
        .py(gpui_kit::rems(0.4))
        .border_b_1()
        .border_color(theme.node_border)
        .child(
            gpui_kit::component::Icon::new(gpui_kit::assets::IconName::TriangleAlert)
                .size(gpui_kit::rems(0.875))
                .text_color(theme.yellow),
        )
        .child(
            gpui_kit::div()
                .text_size(gpui_kit::rems(0.68))
                .text_color(theme.fg_muted)
                .child(warning.clone()),
        )
        .into_any_element()
}
