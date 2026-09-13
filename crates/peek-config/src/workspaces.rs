//! Editing the workspace list, for the connection picker's forms.
//!
//! Ported from `~/labs/peek/src/Connection/useWorkspacesMutation.ts`, with two deliberate
//! changes. The reference keys `updateConnection` and `removeConnection` on the connection's
//! **URL**, so a staging connection and a read-replica alias pointing at the same database edit
//! each other; here the name is the identity. And a duplicate name is refused rather than
//! allowed: two connections whose names differ only in case share one document file, because
//! `DocumentStore` lowercases the path.
//!
//! Names match case-insensitively throughout, which is how `connect_to` and `DocumentStore`
//! already resolve them.

use std::fmt;

use crate::{DatabaseConnection, PeekConfig, Workspace};

/// Why an edit was refused. Every variant is a condition the form can state before saving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceError {
    /// A name of nothing but whitespace.
    EmptyName,
    /// A workspace of that name already exists.
    DuplicateWorkspace,
    /// That workspace already has a connection of that name.
    DuplicateConnection,
    /// Nothing of that name to edit.
    NotFound,
}

impl fmt::Display for WorkspaceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyName => write!(formatter, "a name is required"),
            Self::DuplicateWorkspace => write!(formatter, "a workspace with that name exists"),
            Self::DuplicateConnection => {
                write!(formatter, "a connection with that name exists here")
            }
            Self::NotFound => write!(formatter, "no longer there"),
        }
    }
}

impl std::error::Error for WorkspaceError {}

impl PeekConfig {
    /// The connection at `(workspace, connection)`, for a form opening onto it.
    #[must_use]
    pub fn connection(&self, at: (&str, &str)) -> Option<&DatabaseConnection> {
        self.workspace(at.0)?
            .connections
            .iter()
            .find(|connection| connection.name.eq_ignore_ascii_case(at.1))
    }

    #[must_use]
    pub fn workspace(&self, name: &str) -> Option<&Workspace> {
        self.workspaces
            .iter()
            .find(|workspace| workspace.name.eq_ignore_ascii_case(name))
    }

    /// # Errors
    /// [`WorkspaceError::EmptyName`] or [`WorkspaceError::DuplicateWorkspace`].
    pub fn add_workspace(&mut self, name: &str) -> Result<(), WorkspaceError> {
        let name = trimmed(name)?;
        if self.workspace(&name).is_some() {
            return Err(WorkspaceError::DuplicateWorkspace);
        }
        self.workspaces.push(Workspace {
            name,
            connections: Vec::new(),
        });
        Ok(())
    }

    /// Renaming to the same name in another case is allowed: it is how the case is corrected.
    ///
    /// # Errors
    /// [`WorkspaceError::EmptyName`], [`WorkspaceError::DuplicateWorkspace`] or
    /// [`WorkspaceError::NotFound`].
    pub fn rename_workspace(&mut self, from: &str, to: &str) -> Result<(), WorkspaceError> {
        let to = trimmed(to)?;
        if !to.eq_ignore_ascii_case(from) && self.workspace(&to).is_some() {
            return Err(WorkspaceError::DuplicateWorkspace);
        }
        let found = self
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.name.eq_ignore_ascii_case(from))
            .ok_or(WorkspaceError::NotFound)?;
        found.name = to;
        Ok(())
    }

    /// Removes a workspace and every connection in it. The documents on disk are left alone.
    pub fn remove_workspace(&mut self, name: &str) {
        self.workspaces
            .retain(|workspace| !workspace.name.eq_ignore_ascii_case(name));
    }

    /// # Errors
    /// [`WorkspaceError::EmptyName`], [`WorkspaceError::DuplicateConnection`] or
    /// [`WorkspaceError::NotFound`].
    pub fn add_connection(
        &mut self,
        workspace: &str,
        connection: DatabaseConnection,
    ) -> Result<(), WorkspaceError> {
        let name = trimmed(&connection.name)?;
        if self.connection((workspace, &name)).is_some() {
            return Err(WorkspaceError::DuplicateConnection);
        }
        let found = self.workspace_mut(workspace)?;
        found
            .connections
            .push(DatabaseConnection { name, ..connection });
        Ok(())
    }

    /// Replaces the connection at `at`, which may also rename it.
    ///
    /// # Errors
    /// As [`PeekConfig::add_connection`].
    pub fn update_connection(
        &mut self,
        at: (&str, &str),
        connection: DatabaseConnection,
    ) -> Result<(), WorkspaceError> {
        let name = trimmed(&connection.name)?;
        // A rename onto a *different* existing connection would merge two canvases into one.
        if !name.eq_ignore_ascii_case(at.1) && self.connection((at.0, &name)).is_some() {
            return Err(WorkspaceError::DuplicateConnection);
        }
        let found = self.workspace_mut(at.0)?;
        let at = found
            .connections
            .iter_mut()
            .find(|candidate| candidate.name.eq_ignore_ascii_case(at.1))
            .ok_or(WorkspaceError::NotFound)?;
        *at = DatabaseConnection { name, ..connection };
        Ok(())
    }

    /// Removes a connection's entry. Its document and rows stay on disk, so re-adding the name
    /// gets the canvas back and a misclick costs nothing that cannot be undone by hand.
    pub fn remove_connection(&mut self, at: (&str, &str)) {
        let Ok(workspace) = self.workspace_mut(at.0) else {
            return;
        };
        workspace
            .connections
            .retain(|connection| !connection.name.eq_ignore_ascii_case(at.1));
    }

    fn workspace_mut(&mut self, name: &str) -> Result<&mut Workspace, WorkspaceError> {
        self.workspaces
            .iter_mut()
            .find(|workspace| workspace.name.eq_ignore_ascii_case(name))
            .ok_or(WorkspaceError::NotFound)
    }
}

fn trimmed(name: &str) -> Result<String, WorkspaceError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(WorkspaceError::EmptyName);
    }
    Ok(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::WorkspaceError;
    use crate::{DatabaseConnection, PeekConfig};

    fn connection(name: &str) -> DatabaseConnection {
        DatabaseConnection {
            name: name.to_string(),
            color: "#5584E8".to_string(),
            url: "postgres://u:p@localhost/db".to_string(),
            ssh_tunnel: None,
        }
    }

    fn seeded() -> PeekConfig {
        let mut config = PeekConfig::default();
        config.add_workspace("Plock").unwrap();
        config.add_connection("Plock", connection("local")).unwrap();
        config
    }

    #[test]
    fn a_workspace_and_a_connection_can_be_added() {
        let config = seeded();
        assert_eq!(config.workspaces.len(), 1);
        assert!(
            config.connection(("plock", "LOCAL")).is_some(),
            "names match case-insensitively"
        );
    }

    #[test]
    fn a_name_of_whitespace_is_refused() {
        let mut config = seeded();
        assert_eq!(config.add_workspace("  "), Err(WorkspaceError::EmptyName));
        assert_eq!(
            config.add_connection("Plock", connection("   ")),
            Err(WorkspaceError::EmptyName)
        );
    }

    #[test]
    fn a_name_is_stored_trimmed() {
        let mut config = seeded();
        config
            .add_connection("Plock", connection("  staging  "))
            .unwrap();
        assert!(config.connection(("Plock", "staging")).is_some());
    }

    /// Two connections whose names differ only in case would share one document file, because
    /// `DocumentStore` lowercases the path — so this is data loss, not tidiness.
    #[test]
    fn a_duplicate_connection_name_is_refused_whatever_its_case() {
        let mut config = seeded();
        assert_eq!(
            config.add_connection("Plock", connection("Local")),
            Err(WorkspaceError::DuplicateConnection)
        );
    }

    #[test]
    fn a_duplicate_workspace_name_is_refused_whatever_its_case() {
        let mut config = seeded();
        assert_eq!(
            config.add_workspace("plock"),
            Err(WorkspaceError::DuplicateWorkspace)
        );
    }

    #[test]
    fn updating_a_connection_can_rename_it() {
        let mut config = seeded();
        config
            .update_connection(("Plock", "local"), connection("preprod"))
            .unwrap();
        assert!(config.connection(("Plock", "local")).is_none());
        assert!(config.connection(("Plock", "preprod")).is_some());
    }

    /// Renaming onto a *different* existing connection merges two canvases; renaming onto
    /// itself is how the case of a name gets corrected.
    #[test]
    fn a_rename_onto_another_connection_is_refused_but_onto_itself_is_not() {
        let mut config = seeded();
        config
            .add_connection("Plock", connection("staging"))
            .unwrap();
        assert_eq!(
            config.update_connection(("Plock", "staging"), connection("local")),
            Err(WorkspaceError::DuplicateConnection)
        );
        config
            .update_connection(("Plock", "local"), connection("Local"))
            .unwrap();
        assert_eq!(
            config
                .connection(("Plock", "local"))
                .map(|c| c.name.as_str()),
            Some("Local")
        );
    }

    #[test]
    fn renaming_a_workspace_keeps_its_connections() {
        let mut config = seeded();
        config.rename_workspace("Plock", "Orchard").unwrap();
        assert!(config.workspace("Plock").is_none());
        assert!(config.connection(("Orchard", "local")).is_some());
    }

    #[test]
    fn removing_a_workspace_takes_its_connections_with_it() {
        let mut config = seeded();
        config.remove_workspace("plock");
        assert!(config.workspaces.is_empty());
    }

    #[test]
    fn removing_a_connection_leaves_the_workspace() {
        let mut config = seeded();
        config.remove_connection(("Plock", "local"));
        assert_eq!(
            config.workspace("Plock").map(|w| w.connections.len()),
            Some(0)
        );
    }

    #[test]
    fn editing_something_that_is_gone_says_so_rather_than_creating_it() {
        let mut config = seeded();
        assert_eq!(
            config.add_connection("Nope", connection("x")),
            Err(WorkspaceError::NotFound)
        );
        assert_eq!(
            config.rename_workspace("Nope", "Other"),
            Err(WorkspaceError::NotFound)
        );
    }
}
