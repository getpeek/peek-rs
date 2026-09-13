//! A typed error, where the reference has `Result<_, String>` throughout.
//!
//! The distinction that matters is [`DbError::Query`] against everything else: a query error is
//! the user's SQL failing and belongs in a `query-error` node next to the query, while a
//! connection or schema error is the session being broken and belongs in the connection UI. The
//! reference cannot tell them apart, and additionally flattens every schema failure to a fixed
//! string (`"Could not get columns"`), discarding the driver's message.

use std::fmt;

#[derive(Debug)]
pub enum DbError {
    /// The URL could not be parsed, or names a scheme with no driver.
    Connect {
        url_scheme: String,
        message: String,
    },
    UnsupportedScheme(String),
    /// The SSH tunnel could not be opened.
    Tunnel(String),
    /// The statement itself failed. Carries the driver's message verbatim, because it is the
    /// only useful thing to show the user.
    Query(String),
    /// Introspection failed. `stage` names which query, so a failure is actionable.
    Schema {
        stage: &'static str,
        message: String,
    },
    Import(String),
}

impl fmt::Display for DbError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect {
                url_scheme,
                message,
            } => write!(formatter, "could not connect over {url_scheme}: {message}"),
            Self::UnsupportedScheme(scheme) => write!(
                formatter,
                "unsupported database scheme '{scheme}' (expected postgres:// or mysql://)"
            ),
            Self::Tunnel(message) => write!(formatter, "ssh tunnel failed: {message}"),
            Self::Query(message) => write!(formatter, "{message}"),
            Self::Schema { stage, message } => {
                write!(formatter, "could not read {stage}: {message}")
            }
            Self::Import(message) => write!(formatter, "import failed: {message}"),
        }
    }
}

impl std::error::Error for DbError {}
