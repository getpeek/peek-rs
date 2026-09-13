//! The agent node: a conversation with a coding agent, on the canvas.
//!
//! Ported from `~/labs/peek/src/canvas/nodes/Agent/`. The conversation itself lives in the
//! document (`AgentData.messages`), so undo, autosave and forking all flow through it; only the
//! turn in flight is view state.

mod acp;
pub(crate) mod backend;
mod composer;
mod context;
mod empty;
mod message;
mod ollama;
mod permission;
mod pills;
mod plan_block;
mod rows;
#[cfg(test)]
mod tests;
mod tool_block;
mod tools;
mod view;

use std::collections::HashSet;
use std::rc::Rc;

use gpui_kit::component::StyledExt;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Entity, SharedString, WeakEntity, Window, div};
use peek_canvas::Document;
use peek_document::{AgentData, AgentMessage, NodeId};

use super::kind::NodeContext;
use super::state::NodeState;
use backend::Agents;
use rows::Row;
pub(crate) use view::AgentView;

/// The node's retained state: one view entity, as the query and result nodes have.
pub(crate) struct AgentState {
    view: Entity<AgentView>,
}

impl std::fmt::Debug for AgentState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("AgentState").finish_non_exhaustive()
    }
}

impl AgentState {
    pub(crate) fn new(
        id: &NodeId,
        data: &AgentData,
        document: &Entity<Document>,
        window: &mut Window,
        cx: &mut App,
    ) -> Self {
        let node = id.clone();
        let document = document.clone();
        let data = data.clone();
        Self {
            view: cx.new(|cx| AgentView::new(node, &data, document, window, cx)),
        }
    }

    /// A deleted node must not leave a turn running, nor an entry in the routing table keeping
    /// events flowing to a view nobody can see.
    pub(crate) fn on_removed(&self, cx: &mut App) {
        self.view.update(cx, AgentView::close);
    }

    pub(crate) fn view(&self) -> &Entity<AgentView> {
        &self.view
    }
}

/// The header title: which agent this node is talking to.
pub(crate) fn title(data: &AgentData, cx: &App) -> String {
    match Agents::resolve(data, cx) {
        Some(peek_document::AgentProvider::Acp) => "Claude Code".to_string(),
        Some(peek_document::AgentProvider::Ollama) => {
            Agents::ollama_model(cx).map_or_else(|| "Ollama".to_string(), |model| model.to_string())
        }
        None => "Agent".to_string(),
    }
}

pub(crate) fn body(
    _id: &NodeId,
    data: &AgentData,
    context: NodeContext<'_>,
    _window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let Some(NodeState::Agent(state)) = context.state else {
        return div().into_any_element();
    };
    let view = state.view.clone();
    let zoom = context.zoom;
    view.update(cx, |view, cx| view.reconcile(data, zoom, cx));
    view.into_any_element()
}

/// The provider and mode pills, then the fork button — which is offered only once there is a
/// conversation to fork.
pub(crate) fn header_extras(
    id: &NodeId,
    data: &AgentData,
    context: NodeContext<'_>,
    _window: &mut Window,
    cx: &mut App,
) -> Option<AnyElement> {
    let Some(NodeState::Agent(state)) = context.state else {
        return None;
    };
    let view = state.view().clone();
    let provider = pills::provider(&view, cx);
    let mode = pills::mode(&view, cx);
    let fork = (!data.messages.is_empty()).then(|| fork_button(id, context.document));

    if provider.is_none() && mode.is_none() && fork.is_none() {
        return None;
    }
    Some(
        div()
            .h_flex()
            .items_center()
            .gap(gpui_kit::rems(0.2))
            .children(provider)
            .children(mode)
            .children(fork)
            .into_any_element(),
    )
}

fn fork_button(id: &NodeId, document: &Entity<Document>) -> AnyElement {
    use gpui_kit::component::Icon;
    use gpui_kit::component::button::{Button, ButtonVariants};

    let node = id.clone();
    let document = document.clone();
    Button::new(SharedString::from(format!("{id}-fork")))
        .ghost()
        .tooltip("Fork conversation")
        .child(Icon::new(gpui_kit::assets::IconName::GitFork).size(gpui_kit::rems(0.75)))
        .on_click(move |_, window, cx| {
            // Select this node first: the handler forks the selection, so a click on a node that
            // is not selected would otherwise fork whichever one happened to be. Dispatching
            // rather than calling the handler keeps the button, the palette and the keyboard on
            // one path — `commands/mod.rs`'s rule.
            document.update(cx, |document, cx| {
                document.select_only([node.clone()]);
                cx.notify();
            });
            window.dispatch_action(Box::new(crate::commands::actions::agent::Fork), cx);
        })
        .into_any_element()
}

/// Draws one row of the transcript. Free rather than a method so the scroller's `'static` row
/// renderer can hold the data it needs without borrowing the view.
fn render_row(
    rows: &Rc<Vec<Row>>,
    messages: &Rc<Vec<AgentMessage>>,
    state: (usize, &HashSet<SharedString>, bool),
    view: WeakEntity<AgentView>,
    window: &Window,
    cx: &mut App,
) -> AnyElement {
    let (index, expanded, loading) = state;
    let Some(row) = rows.get(index) else {
        return div().into_any_element();
    };

    match row {
        Row::Message {
            message,
            context_updated,
        } => {
            let Some(message) = messages.get(*message) else {
                return div().into_any_element();
            };
            if message.is("acp_tool") {
                let id = SharedString::from(message.tool_call_id.clone().unwrap_or_default());
                let (id, element) = tool_block::acp(message, expanded.contains(&id), loading, cx);
                return disclosure(id, element, view);
            }
            message::render(
                SharedString::from(format!("agent-md-{index}")),
                message,
                *context_updated,
                window,
                cx,
            )
        }
        Row::ToolPair {
            message: index,
            blocks,
        } => {
            let Some(message) = messages.get(*index) else {
                return div().into_any_element();
            };
            let preamble = (!message.message.trim().is_empty()).then(|| {
                message::turn(
                    SharedString::from(format!("agent-tool-preamble-{index}")),
                    message,
                    window,
                    cx,
                )
            });
            div()
                .v_flex()
                .gap(gpui_kit::rems(0.6))
                .children(preamble)
                .children(blocks.iter().map(|block| {
                    let result = block.result.and_then(|result| messages.get(result));
                    let call = message
                        .tool_calls
                        .as_ref()
                        .and_then(|calls| calls.get(block.call));
                    let id =
                        SharedString::from(call.map(|call| call.id.clone()).unwrap_or_default());
                    let (id, element) =
                        tool_block::pair(message, block, result, expanded.contains(&id), cx);
                    disclosure(id, element, view.clone())
                }))
                .into_any_element()
        }
    }
}

/// Wraps a disclosure so clicking it toggles the view's own open set, which — unlike element
/// state — survives the row being scrolled out of the virtualized list.
fn disclosure(id: SharedString, element: AnyElement, view: WeakEntity<AgentView>) -> AnyElement {
    div()
        .id(SharedString::from(format!("tool-row-{id}")))
        .on_click(move |_, _, cx| {
            let id = id.clone();
            view.update(cx, |view, cx| view.toggle_expanded(&id, cx))
                .ok();
        })
        .child(element)
        .into_any_element()
}
