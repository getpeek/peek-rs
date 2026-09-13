//! Which dialect a connection speaks, and the two things that depend on it.
//!
//! Ported from `~/labs/peek/src/Connection/engine.ts`. The engine is derived from the URL
//! **scheme alone** — deliberately, so that deciding it never requires the credentials in the
//! rest of the URL.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Engine {
    Postgres,
    MySql,
    #[default]
    Unknown,
}

impl Engine {
    /// `postgres`/`postgresql` and `mysql`/`mariadb`; anything else is [`Engine::Unknown`].
    #[must_use]
    pub fn from_url(url: &str) -> Self {
        let scheme = url
            .split_once("://")
            .map(|(scheme, _)| scheme)
            .unwrap_or_default()
            .to_ascii_lowercase();
        match scheme.as_str() {
            "postgres" | "postgresql" => Self::Postgres,
            "mysql" | "mariadb" => Self::MySql,
            _ => Self::Unknown,
        }
    }

    /// The default port for the scheme, used when an SSH tunnel has to know where to forward to
    /// and the URL left the port out.
    #[must_use]
    pub fn default_port(self) -> Option<u16> {
        match self {
            Self::Postgres => Some(5432),
            Self::MySql => Some(3306),
            Self::Unknown => None,
        }
    }

    /// Quotes an identifier for this dialect, doubling any quote already inside it.
    ///
    /// This is the only defence against a column named `id"; drop table users; --`, so every
    /// generated statement must route its table and column names through it.
    #[must_use]
    pub fn quote_identifier(self, name: &str) -> String {
        match self {
            Self::MySql => format!("`{}`", name.replace('`', "``")),
            Self::Postgres | Self::Unknown => format!("\"{}\"", name.replace('"', "\"\"")),
        }
    }

    /// The engine as `get_connection_info` reports it to an agent. A frozen wire value — the
    /// same string the reference's `engineFromUrl` returns — not the prose name.
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::Postgres => "postgresql",
            Self::MySql => "mysql",
            Self::Unknown => "unknown",
        }
    }

    /// How the dialect is named in prose, for error messages and AI prompts.
    #[must_use]
    pub fn dialect_name(self) -> &'static str {
        match self {
            Self::Postgres => "PostgreSQL",
            Self::MySql => "MySQL",
            Self::Unknown => "standard SQL",
        }
    }
}

impl fmt::Display for Engine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.dialect_name())
    }
}

#[cfg(test)]
mod tests {
    use super::Engine;

    #[test]
    fn scheme_selects_the_dialect() {
        assert_eq!(Engine::from_url("postgres://u:p@h/db"), Engine::Postgres);
        assert_eq!(Engine::from_url("postgresql://u:p@h/db"), Engine::Postgres);
        assert_eq!(Engine::from_url("mysql://u:p@h/db"), Engine::MySql);
        assert_eq!(Engine::from_url("mariadb://u:p@h/db"), Engine::MySql);
        assert_eq!(Engine::from_url("sqlite://x.db"), Engine::Unknown);
        assert_eq!(Engine::from_url("nonsense"), Engine::Unknown);
    }

    #[test]
    fn scheme_is_case_insensitive() {
        assert_eq!(Engine::from_url("POSTGRES://u@h/db"), Engine::Postgres);
    }

    /// A password containing `://` must not be mistaken for the scheme separator.
    #[test]
    fn only_the_first_separator_counts() {
        assert_eq!(
            Engine::from_url("postgres://user:pa://ss@host/db"),
            Engine::Postgres
        );
    }

    #[test]
    fn identifiers_are_quoted_per_dialect() {
        assert_eq!(Engine::Postgres.quote_identifier("users"), r#""users""#);
        assert_eq!(Engine::MySql.quote_identifier("users"), "`users`");
        assert_eq!(Engine::Unknown.quote_identifier("users"), r#""users""#);
    }

    /// The injection case the quoting exists for.
    #[test]
    fn embedded_quotes_are_doubled() {
        assert_eq!(
            Engine::Postgres.quote_identifier(r#"id"; drop table users; --"#),
            r#""id""; drop table users; --""#
        );
        assert_eq!(
            Engine::MySql.quote_identifier("id`; drop table users; --"),
            "`id``; drop table users; --`"
        );
    }

    #[test]
    fn default_ports_match_the_reference() {
        assert_eq!(Engine::Postgres.default_port(), Some(5432));
        assert_eq!(Engine::MySql.default_port(), Some(3306));
        assert_eq!(Engine::Unknown.default_port(), None);
    }
}
