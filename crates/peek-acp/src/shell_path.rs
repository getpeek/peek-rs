//! Recovering the user's real `PATH` so the agent command resolves.
//!
//! Ported from `~/labs/peek/src-tauri/src/acp_commands.rs`. A GUI launch (Dock, Finder)
//! inherits a stripped `PATH`, so the configured command — `npx` by default — is not on it
//! and the subprocess dies before `initialize` with a bare `os error 2`.

use std::time::Duration;

use crate::AcpSpawnConfig;

/// Build the spawn config, resolving the command to an absolute path first.
///
/// macOS/Linux apps launched from the Dock/Finder inherit a stripped `PATH`
/// (`/usr/bin:/bin:…`) that omits Homebrew, nvm, Volta, etc., so the configured
/// command (`npx` by default) can't be found — the subprocess then dies before
/// `initialize` with a cryptic `os error 2`. We build a search path from the
/// login shell's `PATH` plus the well-known locations a GUI launch drops,
/// resolve the command against it, and hand the child the same `PATH` so a
/// wrapper launcher (`npx` → `node`) can find its own dependencies. Under
/// `tauri dev` the inherited `PATH` already works, so this only refines it.
///
/// # Errors
/// Returns an error if a bare command can't be found on the search path, so the
/// node surfaces an actionable message instead of a downstream `os error 2`.
pub async fn spawn_config(
    command: &str,
    args: Vec<String>,
    env: Vec<(String, String)>,
) -> Result<AcpSpawnConfig, String> {
    let mut env = env;
    let search_path = resolved_search_path().await;

    // Respect a PATH the user pinned in `ai.acp.env`; otherwise give the child
    // the recovered one so `npx` can in turn resolve `node`.
    if !env.iter().any(|(key, _)| key == "PATH") {
        env.push(("PATH".to_string(), search_path.clone()));
    }

    Ok(AcpSpawnConfig {
        command: resolve_command(command, &search_path)?,
        args,
        env,
    })
}

/// Resolve the agent command to an absolute path. A command that already
/// contains `/` is a path and used verbatim; a bare name is looked up on
/// `search_path`. Erroring for an unresolvable bare name (rather than spawning
/// it and hitting `os error 2`) lets the node tell the user what to fix.
fn resolve_command(command: &str, search_path: &str) -> Result<String, String> {
    if command.contains('/') {
        return Ok(command.to_string());
    }
    resolve_in_path(command, search_path).ok_or_else(|| {
        format!(
            "Couldn't find `{command}` on your PATH. Install it, or set `ai.acp.command` to an absolute path in settings."
        )
    })
}

/// The directories to search for the agent command, and the `PATH` handed to the
/// child: the login shell's `PATH`, then Peek's own inherited `PATH`, then the
/// fallbacks a GUI launch strips — de-duplicated, first occurrence wins.
///
/// Folding in the inherited `PATH` keeps the child's environment a superset of
/// Peek's own, so essentials like `/bin` (the `sh` that `npm` shells out to) are
/// always present even when the login probe fails; the probe and fallbacks then
/// add Homebrew/nvm so `npx` itself resolves. Because it's a superset, overriding
/// the child's `PATH` is never worse than inheriting it (which is what made
/// `tauri dev` work before).
async fn resolved_search_path() -> String {
    let login = login_shell_path().await.unwrap_or_default();
    let inherited = std::env::var("PATH").unwrap_or_default();
    let mut dirs: Vec<String> = Vec::new();
    for dir in login
        .split(':')
        .chain(inherited.split(':'))
        .map(str::to_string)
        .chain(fallback_path_dirs())
    {
        if !dir.is_empty() && !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs.join(":")
}

/// Directories to search even when neither the login-shell probe nor the
/// inherited `PATH` lists them: the Homebrew/version-manager dirs a Dock/Finder
/// launch strips, plus the base system dirs so `sh`, `env`, … always resolve.
fn fallback_path_dirs() -> Vec<String> {
    let mut dirs = vec![
        "/opt/homebrew/bin".to_string(),
        "/opt/homebrew/sbin".to_string(),
        "/usr/local/bin".to_string(),
    ];
    if let Ok(home) = std::env::var("HOME") {
        for suffix in [".volta/bin", ".local/bin", ".cargo/bin"] {
            dirs.push(format!("{home}/{suffix}"));
        }
    }
    // Base system dirs last, so a wrapper the user actually uses wins, but `sh`
    // and friends still resolve if everything else somehow omits them.
    for dir in ["/usr/bin", "/bin", "/usr/sbin", "/sbin"] {
        dirs.push(dir.to_string());
    }
    dirs
}

/// The user's real `PATH`, read from their login shell. `None` when `$SHELL` is
/// unset (e.g. Windows, where GUI apps already inherit the full `PATH`) or the
/// probe fails or times out; callers then fall back to [`fallback_path_dirs`].
async fn login_shell_path() -> Option<String> {
    let shell = std::env::var("SHELL").ok()?;
    // Fence the value in sentinels so rc-file chatter (greetings, async prompt
    // plugins) printed around our line can't be mistaken for the PATH, and cap
    // the wait so a shell that blocks on init can't hang agent-node creation.
    let output = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::process::Command::new(shell)
            .args([
                "-ilc",
                "printf '__PEEK_PATH_BEGIN__%s__PEEK_PATH_END__' \"$PATH\"",
            ])
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let start = stdout.find("__PEEK_PATH_BEGIN__")? + "__PEEK_PATH_BEGIN__".len();
    let end = stdout[start..].find("__PEEK_PATH_END__")? + start;
    let path = stdout[start..end].trim();
    (!path.is_empty()).then(|| path.to_string())
}

/// Find a bare command name on `path`, returning its absolute location, or
/// `None` if nothing matches.
fn resolve_in_path(command: &str, path: &str) -> Option<String> {
    path.split(':')
        .map(|dir| std::path::Path::new(dir).join(command))
        .find(|candidate| candidate.is_file())
        .map(|candidate| candidate.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_with_a_slash_is_used_verbatim() {
        let resolved = resolve_command("/opt/homebrew/bin/npx", "/usr/bin").unwrap();
        assert_eq!(resolved, "/opt/homebrew/bin/npx");
    }

    #[test]
    fn an_unresolvable_bare_command_says_what_to_fix() {
        let error = resolve_command("definitely-not-installed", "/usr/bin:/bin").unwrap_err();
        assert!(error.contains("definitely-not-installed"));
        assert!(error.contains("ai.acp.command"));
    }

    #[test]
    fn a_bare_command_resolves_to_an_absolute_path() {
        // `sh` is on every platform this app targets.
        let resolved = resolve_command("sh", "/nonexistent:/bin:/usr/bin").unwrap();
        assert_eq!(resolved, "/bin/sh");
    }

    #[test]
    fn the_fallback_dirs_always_include_the_base_system_ones() {
        let dirs = fallback_path_dirs();
        for base in ["/usr/bin", "/bin"] {
            assert!(dirs.iter().any(|dir| dir == base), "missing {base}");
        }
        // A wrapper the user actually uses must win over the base dirs.
        let homebrew = dirs.iter().position(|dir| dir == "/opt/homebrew/bin");
        let usr_bin = dirs.iter().position(|dir| dir == "/usr/bin");
        assert!(homebrew < usr_bin);
    }

    #[tokio::test]
    async fn the_search_path_is_deduplicated_and_keeps_the_first_occurrence() {
        let path = resolved_search_path().await;
        let dirs: Vec<&str> = path.split(':').collect();
        let mut seen = std::collections::HashSet::new();
        for dir in &dirs {
            assert!(seen.insert(*dir), "duplicate {dir} in {path}");
            assert!(!dir.is_empty(), "empty entry in {path}");
        }
        assert!(dirs.contains(&"/usr/bin"));
    }
}
