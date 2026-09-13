//! Peek's database layer: connections, queries, schema introspection and the statements the
//! Result node's inline editing runs.
//!
//! Ported from `~/labs/peek/src-tauri/src/database/`, whose contracts are documented in
//! `~/labs/peek/docs/database_drivers.md`. Never depends on gpui.

pub mod connection;
pub mod engine;
pub mod error;
pub mod mutation;
mod mysql;
mod postgres;
pub mod schema;
pub mod session;
pub mod tunnel;

pub use connection::Connection;
pub use engine::Engine;
pub use error::DbError;
/// The result model lives in `peek-document`: the sidecar is a document file, and keeping
/// it there is what lets `peek-canvas` read rows without pulling sqlx into its test build.
pub use peek_document::{Cell, Column, ResultSet};
pub use schema::Schema;
pub use session::{Pending, Session};
pub use tunnel::{HostKeyPolicy, SshTunnel, TunnelConfig};
