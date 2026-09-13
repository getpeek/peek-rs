//! The agent backend: one ACP session pool for the whole app, and the loop that routes its
//! events to the node that asked for them.
//!
//! Modelled on [`crate::database::Database`]. `peek_acp::AgentSession` owns the tokio runtime
//! every call runs on and hands back plain futures, so nothing here needs a runtime of its own.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::{App, Global, SharedString, WeakEntity};
use peek_acp::{AgentEvent, AgentLaunch, AgentSession};
use peek_config::{AiProvider, PeekConfig};
use peek_document::{AgentData, AgentProvider};
use peek_ollama::OllamaSession;

use super::AgentView;

/// The app-wide agent backend.
pub(crate) struct Agents {
    /// `None` when the runtime would not start. The node then reads as unconfigured rather
    /// than the app refusing to open, exactly as a missing database does.
    session: Option<Arc<AgentSession>>,
    settings: AiSettings,
    /// ACP session id -> the node streaming it.
    ///
    /// Weak on purpose: a node deleted mid-turn must not be kept alive by the routing table,
    /// and the entry is swept on the first event that fails to upgrade.
    routes: HashMap<String, WeakEntity<AgentView>>,
    /// Peek's own MCP address, once the server is up. `None` means the agent can chat but
    /// cannot drive the canvas, which the node says out loud rather than leaving it to puzzle.
    mcp_url: Option<String>,
    /// The local-model backend, when `ai.ollama` is configured.
    ollama: Option<Arc<OllamaSession>>,
}

/// What `settings.json` says about AI, snapshotted once at startup. Rendering must not read
/// the disk, and an agent already running would not notice a change anyway.
#[derive(Debug)]
struct AiSettings {
    /// Which backends are configured, in the order they are preferred when a node names none.
    available: Vec<AgentProvider>,
    default_provider: AgentProvider,
    ollama_model: Option<SharedString>,
    launch: Option<AgentLaunch>,
    cwd: Option<PathBuf>,
}

impl std::fmt::Debug for Agents {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Agents")
            .field("started", &self.session.is_some())
            .field("settings", &self.settings)
            .field("sessions", &self.routes.len())
            .finish_non_exhaustive()
    }
}

impl Global for Agents {}

impl Agents {
    /// Starts the backend and the one loop that drains its events.
    ///
    /// No subprocess is spawned here: `AgentSession::new` only builds the runtime, and the
    /// agent itself starts when the first node opens a session.
    pub(crate) fn init(config: &PeekConfig, cx: &mut App) {
        let started = match AgentSession::new() {
            Ok(started) => Some(started),
            Err(error) => {
                log::error!("peek: could not start the agent runtime: {error}");
                None
            }
        };
        let (session, events) = match started {
            Some((session, events)) => (Some(Arc::new(session)), Some(events)),
            None => (None, None),
        };

        let ollama = config.ai.ollama.as_ref().and_then(|ollama| {
            match OllamaSession::new(&ollama.url, &ollama.model) {
                Ok(session) => Some(Arc::new(session)),
                Err(error) => {
                    log::error!("peek: could not start the Ollama runtime: {error}");
                    None
                }
            }
        });

        cx.set_global(Self {
            session,
            settings: AiSettings::from_config(config),
            routes: HashMap::new(),
            mcp_url: None,
            ollama,
        });

        let Some(mut events) = events else {
            return;
        };
        // One receiver multiplexes every session, so fanning out by id is not an optimisation —
        // without it every node would render every other node's turn.
        cx.spawn(async move |cx| {
            // Ends when the session is dropped, which is when the app is going away.
            while let Some(event) = events.next().await {
                cx.update(|cx| Self::route(event, cx));
            }
        })
        .detach();
    }

    fn route(event: AgentEvent, cx: &mut App) {
        let session_id = match &event {
            AgentEvent::Update { session_id, .. } | AgentEvent::Permission { session_id, .. } => {
                session_id.clone()
            }
        };
        let route = cx
            .global::<Self>()
            .routes
            .get(&session_id)
            .and_then(WeakEntity::upgrade);
        let Some(view) = route else {
            // The node was deleted mid-turn. Sweep the entry rather than logging per chunk.
            cx.update_global::<Self, ()>(|agents, _| {
                agents.routes.remove(&session_id);
            });
            return;
        };
        view.update(cx, |view, cx| view.receive(event, cx));
    }

    pub(crate) fn session(cx: &App) -> Option<Arc<AgentSession>> {
        cx.global::<Self>().session.clone()
    }

    pub(crate) fn ollama(cx: &App) -> Option<Arc<OllamaSession>> {
        cx.global::<Self>().ollama.clone()
    }

    /// Which backends `settings.json` configures. Empty means the node has nothing to talk to.
    pub(crate) fn available(cx: &App) -> Vec<AgentProvider> {
        cx.global::<Self>().settings.available.clone()
    }

    /// The backend a node runs on: the one it saved, else the configured default, else whatever
    /// is available. A node whose saved provider is no longer configured falls back silently,
    /// which is what the reference does rather than showing an error.
    pub(crate) fn resolve(data: &AgentData, cx: &App) -> Option<AgentProvider> {
        let settings = &cx.global::<Self>().settings;
        let preferred = data.provider.unwrap_or(settings.default_provider);
        if settings.available.contains(&preferred) {
            return Some(preferred);
        }
        settings.available.first().copied()
    }

    /// How to launch the ACP agent, and the directory it should treat as its workspace.
    pub(crate) fn launch(cx: &App) -> Option<(AgentLaunch, Option<PathBuf>)> {
        let settings = &cx.global::<Self>().settings;
        settings
            .launch
            .clone()
            .map(|launch| (launch, settings.cwd.clone()))
    }

    /// The model name the Ollama backend runs, for the node's title and its provider pill.
    pub(crate) fn ollama_model(cx: &App) -> Option<SharedString> {
        cx.global::<Self>().settings.ollama_model.clone()
    }

    /// Records the MCP server's address, once it is listening.
    pub(crate) fn set_mcp_url(url: String, cx: &mut App) {
        cx.update_global::<Self, ()>(|agents, _| agents.mcp_url = Some(url));
    }

    /// The `(name, url)` pairs a new ACP session should forward to its agent.
    pub(crate) fn mcp_servers(cx: &App) -> Vec<(String, String)> {
        cx.global::<Self>()
            .mcp_url
            .clone()
            .map(|url| vec![("peek".to_string(), url)])
            .unwrap_or_default()
    }

    /// What to tell the user when the agent cannot reach the canvas.
    pub(crate) fn mcp_warning(cx: &App) -> Option<SharedString> {
        if cx.global::<Self>().mcp_url.is_some() {
            return None;
        }
        Some(SharedString::from(
            "Peek's MCP server is off (ai.mcp.enable). The agent can chat but can't drive the \
             canvas — enable it and restart.",
        ))
    }

    pub(crate) fn register(session_id: String, view: WeakEntity<AgentView>, cx: &mut App) {
        cx.update_global::<Self, ()>(|agents, _| {
            agents.routes.insert(session_id, view);
        });
    }

    pub(crate) fn unregister(session_id: &str, cx: &mut App) {
        cx.update_global::<Self, ()>(|agents, _| {
            agents.routes.remove(session_id);
        });
    }

    /// Declares a backend configured without one behind it, so tests reach the paths that are
    /// gated on configuration. Nothing can actually prompt: there is no session, which is what
    /// keeps a test from spawning a real agent by accident.
    #[cfg(test)]
    pub(crate) fn mark_configured_for_test(provider: AgentProvider, cx: &mut App) {
        cx.update_global::<Self, ()>(|agents, _| {
            agents.settings.available = vec![provider];
            agents.settings.default_provider = provider;
        });
    }
}

impl AiSettings {
    fn from_config(config: &PeekConfig) -> Self {
        let ai = &config.ai;
        // Ollama first, matching the reference's fallback order.
        let mut available = Vec::new();
        if ai.ollama.is_some() {
            available.push(AgentProvider::Ollama);
        }
        if ai.acp.is_some() {
            available.push(AgentProvider::Acp);
        }

        Self {
            available,
            default_provider: match ai.default_provider {
                AiProvider::Ollama => AgentProvider::Ollama,
                AiProvider::Acp => AgentProvider::Acp,
            },
            ollama_model: ai
                .ollama
                .as_ref()
                .map(|ollama| SharedString::from(ollama.model.clone())),
            launch: ai.acp.as_ref().map(|acp| AgentLaunch {
                command: acp.command.clone(),
                args: acp.args.clone(),
                env: acp.env.clone().into_iter().collect(),
            }),
            // The reference hands the agent `~/peek` when the user pins no directory: a
            // packaged app's own working directory is the bundle, which is useless to it.
            cwd: ai
                .acp
                .as_ref()
                .and_then(|acp| acp.cwd.clone())
                .map(PathBuf::from)
                .or_else(|| peek_config::config_dir().ok()),
        }
    }
}
