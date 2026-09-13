//! One live database connection.
//!
//! An enum rather than `Box<dyn Database>`: there are exactly two drivers, so dynamic dispatch
//! buys nothing and `async_trait`'s boxing on every query is a cost with no benefit.
//!
//! Three properties of the reference are deliberately kept (`~/labs/peek/docs/database_drivers.md`
//! states them as rules):
//!
//! - **Connect eagerly.** Bad credentials must fail here, not at the first query.
//! - **One connection for the session.** Temp tables, `SET` and the backend pid all live on the
//!   connection, so a pool would make imported tables vanish between queries.
//! - **No timeout and no cancellation.** The reference has neither; adding one silently changes
//!   which long queries survive. The Activity node's `pg_terminate_backend` is the only stop.

use std::fmt;

use sqlx::{Connection as _, MySqlConnection, PgConnection};

use crate::engine::Engine;
use crate::error::DbError;
use crate::schema::Schema;
use peek_document::ResultSet;

pub enum Connection {
    Postgres(Box<PgConnection>),
    MySql(Box<MySqlConnection>),
}

impl fmt::Debug for Connection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Connection")
            .field("engine", &self.engine())
            .finish_non_exhaustive()
    }
}

impl Connection {
    /// Opens a connection, choosing the driver from the URL scheme.
    ///
    /// # Errors
    /// [`DbError::UnsupportedScheme`] for anything but postgres/mysql, and [`DbError::Connect`]
    /// when the server refuses.
    pub async fn open(url: &str) -> Result<Self, DbError> {
        let engine = Engine::from_url(url);
        match engine {
            Engine::Postgres => PgConnection::connect(url)
                .await
                .map(|connection| Self::Postgres(Box::new(connection)))
                .map_err(|error| DbError::Connect {
                    url_scheme: "postgres".to_string(),
                    message: error.to_string(),
                }),
            Engine::MySql => MySqlConnection::connect(url)
                .await
                .map(|connection| Self::MySql(Box::new(connection)))
                .map_err(|error| DbError::Connect {
                    url_scheme: "mysql".to_string(),
                    message: error.to_string(),
                }),
            Engine::Unknown => Err(DbError::UnsupportedScheme(
                url.split_once("://")
                    .map(|(scheme, _)| scheme.to_string())
                    .unwrap_or_default(),
            )),
        }
    }

    #[must_use]
    pub fn engine(&self) -> Engine {
        match self {
            Self::Postgres(_) => Engine::Postgres,
            Self::MySql(_) => Engine::MySql,
        }
    }

    /// Runs a statement that returns rows.
    ///
    /// # Errors
    /// [`DbError::Query`] carrying the driver's own message, which is what the `query-error`
    /// node shows.
    pub async fn query(&mut self, sql: &str) -> Result<ResultSet, DbError> {
        match self {
            Self::Postgres(connection) => crate::postgres::query(connection, sql).await,
            Self::MySql(connection) => crate::mysql::query(connection, sql).await,
        }
    }

    /// Runs a statement that returns no rows, yielding the number of rows it changed.
    ///
    /// # Errors
    /// [`DbError::Query`] carrying the driver's message.
    pub async fn execute(&mut self, sql: &str) -> Result<u64, DbError> {
        match self {
            Self::Postgres(connection) => crate::postgres::execute(connection, sql).await,
            Self::MySql(connection) => crate::mysql::execute(connection, sql).await,
        }
    }

    /// Introspects tables, foreign keys and primary keys.
    ///
    /// `imported` names temp tables created by the import path, which `information_schema` does
    /// not list on `MySQL`; Postgres finds them through its `pg_class` union and ignores the hint.
    ///
    /// # Errors
    /// [`DbError::Schema`] naming which of the three queries failed.
    pub async fn schema(&mut self, imported: &[String]) -> Result<Schema, DbError> {
        match self {
            Self::Postgres(connection) => crate::postgres::schema(connection).await,
            Self::MySql(connection) => crate::mysql::schema(connection, imported).await,
        }
    }
}
