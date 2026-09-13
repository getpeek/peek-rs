//! The tokio runtime every ACP call runs on, and the bridge back to the UI.
//!
//! `CLAUDE.md` forbids a tokio runtime in UI code, and `agent-client-protocol` is built on
//! one. The runtime therefore lives here, on its own threads. Request/response calls hand
//! back a [`Pending`] — a `oneshot::Receiver`, a plain future with no reactor of its own,
//! which gpui's executor can await like any other. Pushed events arrive on [`AgentEvents`],
//! whose receiver has the same property.
//!
//! This mirrors `peek_db::Session` deliberately: one runtime per backend, one shape to learn.

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::runtime::Runtime;
use tokio::sync::{Mutex as AsyncMutex, mpsc, oneshot};

use crate::config::AgentLaunch;
use crate::connection::{AcpConnection, SessionInfo};
use crate::events::{AgentEvent, AgentEvents, PermissionId, PermissionRequest};
use crate::host::AcpHost;

/// How long the agent waits for the user to answer a permission prompt before the request
/// is treated as cancelled. Matches the reference's five minutes.
const PERMISSION_TIMEOUT: Duration = Duration::from_secs(300);

/// The answer to a fallible call, awaitable on gpui's executor.
pub type Pending<T> = oneshot::Receiver<Result<T, AcpError>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcpError {
    /// The agent subprocess could not be started — a missing command, a failed handshake.
    Spawn(String),
    /// The connection is up but the call failed.
    Call(String),
    /// No agent has been started on this session yet.
    NotStarted,
}

impl fmt::Display for AcpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(message) => write!(formatter, "couldn't start the ACP agent: {message}"),
            Self::Call(message) => write!(formatter, "{message}"),
            Self::NotStarted => write!(formatter, "the ACP agent isn't running"),
        }
    }
}

impl std::error::Error for AcpError {}

/// Owns the runtime, the one agent subprocess, and every session on it.
pub struct AgentSession {
    runtime: Runtime,
    connection: Arc<AsyncMutex<Option<Arc<AcpConnection>>>>,
    host: Arc<ChannelHost>,
}

impl fmt::Debug for AgentSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgentSession")
            .finish_non_exhaustive()
    }
}

impl AgentSession {
    /// Starts the runtime. No subprocess is spawned until the first [`Self::open_session`],
    /// so constructing this is cheap and safe in a test.
    ///
    /// # Errors
    /// Returns an error if the tokio runtime cannot be built.
    pub fn new() -> std::io::Result<(Self, AgentEvents)> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("peek-acp")
            .enable_all()
            .build()?;
        let (sender, receiver) = mpsc::unbounded_channel();
        let session = Self {
            runtime,
            connection: Arc::new(AsyncMutex::new(None)),
            host: Arc::new(ChannelHost {
                events: sender,
                pending: std::sync::Mutex::new(HashMap::new()),
                counter: AtomicU64::new(0),
            }),
        };
        Ok((session, AgentEvents(receiver)))
    }

    /// Opens a session, spawning the agent subprocess on first use.
    ///
    /// `mcp_http_servers` is `(name, url)` pairs the agent should connect to — how Peek's
    /// own MCP server reaches it, and thus how the agent drives the canvas.
    ///
    /// Resolving `launch.command` against the login shell's `PATH` happens here, on the
    /// runtime, because it shells out and gpui's executor has no reactor to drive that on. It
    /// is also only paid once: the second session reuses the connection and never probes.
    pub fn open_session(
        &self,
        launch: AgentLaunch,
        cwd: Option<PathBuf>,
        mcp_http_servers: Vec<(String, String)>,
    ) -> Pending<SessionInfo> {
        let host = self.host.clone();
        self.spawn(move |slot| async move {
            let connection = {
                let mut slot = slot.lock().await;
                if let Some(connection) = slot.as_ref() {
                    connection.clone()
                } else {
                    let spawn =
                        crate::shell_path::spawn_config(&launch.command, launch.args, launch.env)
                            .await
                            .map_err(AcpError::Spawn)?;
                    let connection = Arc::new(
                        AcpConnection::spawn(spawn, host)
                            .await
                            .map_err(|error| AcpError::Spawn(error.to_string()))?,
                    );
                    *slot = Some(connection.clone());
                    connection
                }
            };
            // A packaged app's `current_dir` is `/` or the bundle, so the caller passes the
            // directory the agent should treat as its workspace; `None` falls back to the
            // process's own, which is right under `cargo run`.
            connection
                .new_session(cwd, mcp_http_servers)
                .await
                .map_err(|error| AcpError::Spawn(error.to_string()))
        })
    }

    /// Sends a prompt. Resolves with the turn's stop reason once the agent is done; every
    /// message of the answer arrives on [`AgentEvents`] in the meantime.
    pub fn prompt(&self, session_id: String, text: String) -> Pending<String> {
        self.call(move |connection| async move { connection.prompt(session_id, text).await })
    }

    pub fn set_mode(&self, session_id: String, mode_id: String) -> Pending<()> {
        self.call(move |connection| async move { connection.set_mode(session_id, mode_id).await })
    }

    /// Cancels the running turn. The prompt still resolves, with a cancelled stop reason.
    pub fn cancel(&self, session_id: String) -> Pending<()> {
        self.call(move |connection| async move { connection.cancel(session_id).await })
    }

    /// Answers an outstanding permission prompt. `None` cancels the request.
    ///
    /// Synchronous and infallible: an answer for a request that already timed out or whose
    /// agent went away is simply dropped.
    pub fn answer_permission(&self, id: PermissionId, option: Option<String>) {
        self.host.answer(id, option);
    }

    /// True once the agent subprocess is up. Used to decide whether a node needs to open a
    /// session before it can prompt.
    #[must_use]
    pub fn is_started(&self) -> bool {
        self.connection
            .try_lock()
            .is_ok_and(|connection| connection.is_some())
    }

    /// Runs `work` against the live connection, or fails with [`AcpError::NotStarted`].
    fn call<T, F, Fut>(&self, work: F) -> Pending<T>
    where
        T: Send + 'static,
        F: FnOnce(Arc<AcpConnection>) -> Fut + Send + 'static,
        Fut: Future<Output = anyhow::Result<T>> + Send,
    {
        self.spawn(move |slot| async move {
            let connection = slot.lock().await.clone().ok_or(AcpError::NotStarted)?;
            work(connection)
                .await
                .map_err(|error| AcpError::Call(error.to_string()))
        })
    }

    fn spawn<T, F, Fut>(&self, work: F) -> Pending<T>
    where
        T: Send + 'static,
        F: FnOnce(Arc<AsyncMutex<Option<Arc<AcpConnection>>>>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, AcpError>> + Send,
    {
        let (sender, receiver) = oneshot::channel();
        let connection = Arc::clone(&self.connection);
        self.runtime.spawn(async move {
            // The receiver is dropped when the caller stops caring; that is not an error.
            let _ = sender.send(work(connection).await);
        });
        receiver
    }
}

/// The [`AcpHost`] the connection calls from its own threads. Both methods must return
/// promptly — blocking here stalls the ACP dispatch loop for every session.
struct ChannelHost {
    events: mpsc::UnboundedSender<AgentEvent>,
    pending: std::sync::Mutex<HashMap<PermissionId, oneshot::Sender<Option<String>>>>,
    counter: AtomicU64,
}

impl fmt::Debug for ChannelHost {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChannelHost")
            .finish_non_exhaustive()
    }
}

impl ChannelHost {
    fn answer(&self, id: PermissionId, option: Option<String>) {
        let sender = self
            .pending
            .lock()
            .expect("the permission map is never poisoned")
            .remove(&id);
        if let Some(sender) = sender {
            let _ = sender.send(option);
        }
    }
}

#[async_trait::async_trait]
impl AcpHost for ChannelHost {
    async fn on_update(&self, session_id: String, update: serde_json::Value) {
        // Parsing here keeps `serde_json` out of the UI crate; an update Peek ignores never
        // reaches the channel at all.
        let Some(update) = crate::update::AcpUpdate::from_value(&update) else {
            return;
        };
        // Unbounded, so the send is synchronous and cannot block the dispatch loop.
        let _ = self.events.send(AgentEvent::Update { session_id, update });
    }

    async fn request_permission(
        &self,
        session_id: String,
        request: serde_json::Value,
    ) -> Option<String> {
        let id = PermissionId(self.counter.fetch_add(1, Ordering::Relaxed));
        let (sender, receiver) = oneshot::channel();
        self.pending
            .lock()
            .expect("the permission map is never poisoned")
            .insert(id, sender);

        let event = AgentEvent::Permission {
            session_id,
            id,
            request: PermissionRequest::from_value(&request),
        };
        if self.events.send(event).is_err() {
            self.forget(id);
            return None;
        }

        // A prompt nobody answers is a cancel, not a wedged agent.
        match tokio::time::timeout(PERMISSION_TIMEOUT, receiver).await {
            Ok(Ok(choice)) => choice,
            Ok(Err(_)) => None,
            Err(_) => {
                self.forget(id);
                None
            }
        }
    }
}

impl ChannelHost {
    fn forget(&self, id: PermissionId) {
        self.pending
            .lock()
            .expect("the permission map is never poisoned")
            .remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_session_starts_no_subprocess() {
        let (session, _events) = AgentSession::new().expect("runtime builds");
        assert!(!session.is_started());
    }

    #[test]
    fn calling_before_the_agent_starts_reports_not_started() {
        let (session, _events) = AgentSession::new().expect("runtime builds");
        let pending = session.prompt("s1".into(), "hello".into());
        let outcome = futures_lite_block_on(pending);
        assert_eq!(outcome, Err(AcpError::NotStarted));
    }

    #[test]
    fn an_answer_for_an_unknown_prompt_is_dropped_rather_than_panicking() {
        let (session, _events) = AgentSession::new().expect("runtime builds");
        session.answer_permission(PermissionId(42), Some("allow".into()));
    }

    #[test]
    fn the_errors_read_as_sentences() {
        assert_eq!(
            AcpError::Spawn("no npx".into()).to_string(),
            "couldn't start the ACP agent: no npx"
        );
        assert_eq!(
            AcpError::NotStarted.to_string(),
            "the ACP agent isn't running"
        );
    }

    /// The production caller awaits a `Pending` on gpui's executor; a test has none, so it
    /// blocks on the runtime the session already owns.
    fn futures_lite_block_on<T: Send + 'static>(pending: Pending<T>) -> Result<T, AcpError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime builds");
        runtime
            .block_on(pending)
            .expect("the session runtime stays alive")
    }
}
