//! The process's one database connection, held as a gpui global.
//!
//! `peek_db::Session` owns the tokio runtime every call runs on and hands back plain futures, so
//! nothing here needs a runtime of its own — which is what keeps `CLAUDE.md`'s "no tokio in UI
//! code" rule intact while sqlx sits underneath.
//!
//! One connection for the whole process, as the Tauri host had it: temp tables, `SET` and the
//! backend pid all live on the connection, so a pool would make an imported table vanish between
//! the query that created it and the query that reads it.

use std::sync::Arc;

use gpui_kit::{App, AsyncApp, BorrowAppContext, Global};
use peek_config::DatabaseConnection;
use peek_db::{Engine, HostKeyPolicy, Session, TunnelConfig};

use crate::node::query::language::SqlLanguage;

pub(crate) struct Database {
    session: Option<Arc<Session>>,
    engine: Engine,
    connected: bool,
    /// The last connection error, shown by the connection UI rather than thrown away.
    error: Option<String>,
}

impl std::fmt::Debug for Database {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Database")
            .field("connected", &self.connected)
            .field("engine", &self.engine)
            .finish_non_exhaustive()
    }
}

impl Global for Database {}

impl Database {
    /// Starts the runtime. A failure here leaves the app running with no database rather than
    /// refusing to open: every other node still works without one.
    pub(crate) fn init(cx: &mut App) {
        let session = match Session::new() {
            Ok(session) => Some(Arc::new(session)),
            Err(error) => {
                log::error!("peek: could not start the database runtime: {error}");
                None
            }
        };
        cx.set_global(Self {
            session,
            engine: Engine::Unknown,
            connected: false,
            error: None,
        });
    }

    pub(crate) fn is_connected(cx: &App) -> bool {
        cx.global::<Self>().connected
    }

    /// The dialect, which decides identifier quoting and the JSON cast syntax.
    #[allow(
        dead_code,
        reason = "the Result node's inline editing builds its UPDATE/DELETE with this; \
                  removing the allow is part of that milestone"
    )]
    pub(crate) fn engine(cx: &App) -> Engine {
        cx.global::<Self>().engine
    }

    pub(crate) fn session(cx: &App) -> Option<Arc<Session>> {
        cx.global::<Self>().session.clone()
    }

    /// The last connection error, which the connection picker shows.
    pub(crate) fn error(cx: &App) -> Option<String> {
        cx.global::<Self>().error.clone()
    }

    /// Opens `connection` and, once it answers, introspects its schema into the language server.
    ///
    /// Connecting is eager, so bad credentials surface here rather than at the first query.
    pub(crate) fn connect(connection: &DatabaseConnection, cx: &mut App) {
        let Some(session) = Self::session(cx) else {
            return;
        };
        let url = connection.url.clone();
        let name = connection.name.clone();
        let tunnel = connection.ssh_tunnel.as_ref().map(tunnel_config);

        // Everything the old connection left behind goes now, not when the new one answers: a
        // failed connect must not leave the previous database's tables completing, nor its
        // dialect quoting identifiers.
        peek_lsp::set_schema(&SqlLanguage::schema(cx), peek_lsp::SchemaIndex::default());
        cx.update_global::<Self, ()>(|database, _| {
            database.connected = false;
            database.engine = Engine::Unknown;
            database.error = None;
        });

        cx.spawn(async move |cx| {
            let opened = session
                .connect(url, tunnel, HostKeyPolicy::TrustOnFirstUse)
                .await;
            let engine = match opened {
                Ok(Ok(engine)) => engine,
                Ok(Err(error)) => return record_failure(&name, &error.to_string(), cx),
                Err(_) => return record_failure(&name, "the database runtime stopped", cx),
            };
            log::info!("peek: connected to {name} ({engine})");
            cx.update_global::<Self, ()>(|database, _| {
                database.connected = true;
                database.engine = engine;
                database.error = None;
            });
            load_schema(&session, cx).await;
        })
        .detach();
    }
}

fn record_failure(name: &str, message: &str, cx: &mut AsyncApp) {
    log::error!("peek: could not connect to {name}: {message}");
    let message = message.to_string();
    cx.update_global::<Database, ()>(|database, _| {
        database.connected = false;
        database.error = Some(message);
    });
}

/// Fills the language server's schema index, which lights up table and column completions and
/// turns on the diagnostics that were deliberately silent while it was empty.
async fn load_schema(session: &Arc<Session>, cx: &mut AsyncApp) {
    let schema = match session.schema().await {
        Ok(Ok(schema)) => schema,
        Ok(Err(error)) => {
            log::error!("peek: could not read the schema: {error}");
            return;
        }
        Err(_) => return,
    };
    log::info!("peek: schema has {} tables", schema.tables.len());
    // `SchemaIndex::from_raw` takes the reference's own map shapes, including the inverted
    // `"table.column"` reference keys, so the driver's output goes across unchanged.
    let index = peek_lsp::SchemaIndex::from_raw(
        schema.tables.into_iter().collect(),
        schema.references.into_iter().collect(),
        schema.primary_keys.into_iter().collect(),
    );
    cx.update(|cx| {
        peek_lsp::set_schema(&SqlLanguage::schema(cx), index);
    });
}

fn tunnel_config(config: &peek_config::SshTunnelConfig) -> TunnelConfig {
    TunnelConfig {
        ssh_host: config.ssh_host.clone(),
        ssh_port: config.ssh_port,
        ssh_user: config.ssh_user.clone(),
        key_path: config.key_path.clone(),
        local_port: config.local_port,
    }
}
