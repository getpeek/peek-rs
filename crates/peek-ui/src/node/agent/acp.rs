//! Driving one node's ACP session: opening it, prompting, modes, and permission answers.

use gpui_kit::{Context, SharedString, Window};
use peek_acp::{AcpError, PermissionId, PermissionRequest};
use peek_document::AgentMessage;

use super::backend::Agents;
use super::view::{AgentView, Backend, now_ms};

/// One node's slice of the shared connection.
#[derive(Debug, Default)]
pub(super) struct Acp {
    /// This node's session, and the key every event is routed by. `None` until the open lands.
    session_id: Option<String>,
    /// `(id, name)` pairs the agent advertises. Empty when it has no modes, which hides the pill.
    modes: Vec<(String, String)>,
    current_mode: Option<String>,
    opening: bool,
    /// A prompt the user sent while the agent was still starting, replayed when it lands.
    queued: Option<String>,
    pub(super) pending: Option<(PermissionId, PermissionRequest)>,
}

impl Acp {
    pub(super) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub(super) fn modes(&self) -> &[(String, String)] {
        &self.modes
    }

    /// The mode to show: the one in force, falling back to the first the agent offers.
    pub(super) fn current_mode(&self) -> Option<&str> {
        self.current_mode
            .as_deref()
            .or_else(|| self.modes.first().map(|(id, _)| id.as_str()))
    }

    /// A mode the agent switched to on its own — a plan step finishing, say.
    pub(super) fn adopt_mode(&mut self, mode: Option<&str>) {
        if let Some(mode) = mode {
            self.current_mode = Some(mode.to_string());
        }
    }
}

/// Sends `text`, opening the session first if this is the node's first turn.
pub(super) fn ask(
    view: &mut AgentView,
    text: String,
    window: &mut Window,
    cx: &mut Context<AgentView>,
) {
    let Backend::Acp(acp) = view.backend_mut() else {
        view.stage(
            AgentMessage::new(
                "system",
                "This node's backend is not wired up yet.".to_string(),
                now_ms(),
            ),
            cx,
        );
        view.set_loading(false, cx);
        return;
    };

    let session = acp.session_id().map(str::to_string);
    let text = with_query_references(view, text, cx);
    if let Some(session) = session {
        prompt(view, session, text, window, cx);
        return;
    }
    if let Backend::Acp(acp) = view.backend_mut() {
        acp.queued = Some(text);
    }
    open(view, window, cx);
}

/// Prefixes every prompt with the queries wired into the node, and stages a chip for the ones
/// the transcript has not shown yet.
///
/// Every prompt, not only the fresh ones: the ACP session is not persisted, so an agent reopened
/// after a restart has never seen a reference the transcript says was sent. A few lines per
/// wired query is cheap next to the agent not knowing what "this node" is.
fn with_query_references(
    view: &mut AgentView,
    text: String,
    cx: &mut Context<AgentView>,
) -> String {
    let document = view.document_handle();
    let references = super::context::query_references(document.read(cx), view.node());
    if references.is_empty() {
        return text;
    }
    let seen = view.committed_messages(cx);
    for message in super::context::unseen(references.clone(), &seen) {
        view.stage(message, cx);
    }
    let preamble: Vec<String> = references
        .into_iter()
        .map(|message| message.message)
        .collect();
    format!("{}\n{text}", preamble.join("\n"))
}

/// Opens the node's session. Idempotent: a second call while one is in flight is ignored.
pub(super) fn open(view: &mut AgentView, window: &mut Window, cx: &mut Context<AgentView>) {
    let Backend::Acp(acp) = view.backend_mut() else {
        return;
    };
    if acp.opening || acp.session_id.is_some() {
        return;
    }

    let (Some(session), Some((launch, cwd))) = (Agents::session(cx), Agents::launch(cx)) else {
        view.set_warning(Some(SharedString::from(
            "No ACP agent configured. Add an `ai.acp` block to ~/peek/settings.json.",
        )));
        view.set_loading(false, cx);
        return;
    };

    acp.opening = true;
    view.set_warning(Some(SharedString::from("Starting the agent…")));

    let servers = Agents::mcp_servers(cx);
    let pending = session.open_session(launch, cwd, servers);
    let task = cx.spawn_in(window, async move |this, cx| {
        let outcome = pending.await;
        let _ = this.update_in(cx, |view, window, cx| match outcome {
            Ok(Ok(info)) => opened(view, info, window, cx),
            Ok(Err(error)) => failed(view, &error.to_string(), cx),
            // The oneshot's sender went away, which only happens if the runtime stopped.
            Err(_) => failed(view, "the agent runtime stopped", cx),
        });
    });
    view.hold_prompt(task);
}

fn opened(
    view: &mut AgentView,
    info: peek_acp::SessionInfo,
    window: &mut Window,
    cx: &mut Context<AgentView>,
) {
    let queued = {
        let Backend::Acp(acp) = view.backend_mut() else {
            return;
        };
        acp.opening = false;
        acp.session_id = Some(info.session_id.clone());
        acp.modes = info.available_modes;
        acp.current_mode = info.current_mode;
        acp.queued.take()
    };
    Agents::register(info.session_id.clone(), cx.weak_entity(), cx);
    view.set_warning(Agents::mcp_warning(cx));

    match queued {
        Some(text) => prompt(view, info.session_id, text, window, cx),
        None => view.set_loading(false, cx),
    }
}

/// A failed start is staged as a `system` message rather than only logged: it follows the user's
/// question in the transcript, so the pair explains itself when they come back to it.
fn failed(view: &mut AgentView, message: &str, cx: &mut Context<AgentView>) {
    if let Backend::Acp(acp) = view.backend_mut() {
        acp.opening = false;
        acp.queued = None;
    }
    view.set_warning(None);
    view.stage(
        AgentMessage::new(
            "system",
            format!("Couldn't start the agent: {message}"),
            now_ms(),
        ),
        cx,
    );
    view.finish_turn(cx);
}

fn prompt(
    view: &mut AgentView,
    session_id: String,
    text: String,
    window: &mut Window,
    cx: &mut Context<AgentView>,
) {
    let Some(session) = Agents::session(cx) else {
        failed(view, "the agent runtime stopped", cx);
        return;
    };
    let pending = session.prompt(session_id, text);
    let task = cx.spawn_in(window, async move |this, cx| {
        let outcome = pending.await;
        let _ = this.update_in(cx, |view, _, cx| {
            match outcome {
                Ok(Ok(_stop_reason)) => {}
                Ok(Err(AcpError::NotStarted)) => view.stage(
                    AgentMessage::new("system", "The agent is not running.".to_string(), now_ms()),
                    cx,
                ),
                Ok(Err(error)) => view.stage(
                    AgentMessage::new(
                        "system",
                        format!("The agent's turn failed: {error}"),
                        now_ms(),
                    ),
                    cx,
                ),
                Err(_) => view.stage(
                    AgentMessage::new("system", "The agent runtime stopped.".to_string(), now_ms()),
                    cx,
                ),
            }
            view.finish_turn(cx);
        });
    });
    view.hold_prompt(task);
}

/// Asks the agent to stop.
///
/// Deliberately leaves `loading` set: the turn is over when the agent says it is, and its
/// `prompt` still resolves — with a cancelled stop reason — which is what clears the flag.
pub(super) fn stop(view: &mut AgentView, cx: &mut Context<AgentView>) {
    let Backend::Acp(acp) = view.backend() else {
        return;
    };
    let (Some(session_id), Some(session)) = (acp.session_id(), Agents::session(cx)) else {
        return;
    };
    // Fire and forget: a cancel that does not land leaves the turn running, which the stop
    // button already communicates by staying a stop button.
    drop(session.cancel(session_id.to_string()));
}

/// Switches mode optimistically; the agent's own `ModeChanged` is the authority and folds back
/// in through the transcript.
pub(super) fn set_mode(view: &mut AgentView, mode: String, cx: &mut Context<AgentView>) {
    let Backend::Acp(acp) = view.backend_mut() else {
        return;
    };
    let Some(session_id) = acp.session_id().map(str::to_string) else {
        return;
    };
    acp.current_mode = Some(mode.clone());
    if let Some(session) = Agents::session(cx) {
        drop(session.set_mode(session_id, mode));
    }
    cx.notify();
}

/// Advances to the next mode, wrapping. With no mode in force this lands on the first, which is
/// what the reference's `index === -1` fallthrough does.
pub(super) fn cycle_mode(view: &mut AgentView, cx: &mut Context<AgentView>) {
    let Backend::Acp(acp) = view.backend() else {
        return;
    };
    if acp.modes.is_empty() {
        return;
    }
    let current = acp.current_mode();
    let index = acp
        .modes
        .iter()
        .position(|(id, _)| Some(id.as_str()) == current);
    let next = index.map_or(0, |index| (index + 1) % acp.modes.len());
    let mode = acp.modes[next].0.clone();
    set_mode(view, mode, cx);
}

/// Answers an outstanding permission prompt. `None` cancels it.
pub(super) fn answer_permission(
    view: &mut AgentView,
    option: Option<String>,
    cx: &mut Context<AgentView>,
) {
    let Backend::Acp(acp) = view.backend_mut() else {
        return;
    };
    let Some((id, _)) = acp.pending.take() else {
        return;
    };
    if let Some(session) = Agents::session(cx) {
        session.answer_permission(id, option);
    }
    cx.notify();
}
