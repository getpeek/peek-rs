/// How to launch an ACP agent, as the user wrote it in `settings.json`.
///
/// Distinct from [`AcpSpawnConfig`], which is what this becomes once the command has been
/// resolved against the login shell's `PATH` — a step that needs a tokio reactor and therefore
/// happens inside [`crate::AgentSession::open_session`] rather than at the call site.
#[derive(Debug, Clone, Default)]
pub struct AgentLaunch {
    /// Executable that launches the ACP agent (e.g. `npx`).
    pub command: String,
    /// Arguments passed to `command` (e.g. the Claude Code adapter package).
    pub args: Vec<String>,
    /// Extra environment variables for the child process. The agent owns its own auth, so this
    /// is where credentials it expects (e.g. `ANTHROPIC_API_KEY`) are passed through.
    pub env: Vec<(String, String)>,
}

/// A launch with its command already resolved to an absolute path. Internal: callers describe
/// an agent with [`AgentLaunch`] and let [`crate::AgentSession::open_session`] resolve it.
#[derive(Debug, Clone, Default)]
pub(crate) struct AcpSpawnConfig {
    /// Executable that launches the ACP agent (e.g. `npx`).
    pub command: String,
    /// Arguments passed to `command` (e.g. the Claude Code adapter package).
    pub args: Vec<String>,
    /// Extra environment variables for the child process. The agent owns its own
    /// auth, so this is where credentials it expects (e.g. `ANTHROPIC_API_KEY`)
    /// are passed through.
    pub env: Vec<(String, String)>,
}
