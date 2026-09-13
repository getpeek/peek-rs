//! The seam between the database (async, tokio) and the UI (gpui's own executor).
//!
//! `CLAUDE.md` forbids a tokio runtime in UI code, and sqlx is built on one. The runtime
//! therefore lives here, on its own threads, and every call hands back a
//! [`tokio::sync::oneshot::Receiver`] — a plain future with no reactor of its own, which gpui's
//! executor can await like any other.
//!
//! One connection behind one mutex, as the reference has it: a long query serialises the others.
//! That is a real limitation (`~/labs/peek/docs/database_drivers.md` lists it under "Known
//! gaps"), but a pool would break temp tables, `SET` and the backend pid, which the import path
//! and the Activity node depend on.

use std::fmt;
use std::sync::Arc;

use tokio::runtime::Runtime;
use tokio::sync::{Mutex, oneshot};

use crate::connection::Connection;
use crate::engine::Engine;
use crate::error::DbError;
use crate::schema::Schema;
use crate::tunnel::{HostKeyPolicy, SshTunnel, TunnelConfig};
use peek_document::ResultSet;

/// A pending database call. Awaiting it yields the result, or the [`DbError`] the call failed
/// with; the outer `Err` only happens if the runtime went away mid-flight.
pub type Pending<T> = oneshot::Receiver<Result<T, DbError>>;

#[derive(Default)]
struct Inner {
    connection: Option<Connection>,
    /// Dropped before a new one is opened, so the local port is free to rebind.
    tunnel: Option<SshTunnel>,
    /// Temp tables created by the import path, which `information_schema` will not list.
    imported: Vec<String>,
}

pub struct Session {
    /// Declared before `runtime`, and dropped before it: a tunnel left in here reaches
    /// `SshTunnel`'s `Drop`, which spawns its disconnect, and spawning after the runtime has
    /// gone panics.
    inner: Arc<Mutex<Inner>>,
    runtime: Runtime,
}

impl fmt::Debug for Session {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Session").finish_non_exhaustive()
    }
}

impl Session {
    /// Starts the runtime that owns every database call.
    ///
    /// # Errors
    /// Returns the io error if the runtime's threads cannot be started.
    pub fn new() -> std::io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("peek-db")
            .enable_all()
            .build()?;
        Ok(Self {
            inner: Arc::new(Mutex::new(Inner::default())),
            runtime,
        })
    }

    /// Opens a connection, replacing any current one, and reports the dialect it speaks.
    ///
    /// Connecting is eager: bad credentials fail here rather than at the first query, which is
    /// what lets the connection picker show an error next to the connection that caused it.
    pub fn connect(
        &self,
        url: String,
        tunnel: Option<TunnelConfig>,
        policy: HostKeyPolicy,
    ) -> Pending<Engine> {
        self.spawn(move |inner| async move {
            let mut inner = inner.lock().await;
            // Both go before anything new is opened, and the tunnel is closed rather than
            // dropped: `local_port` is a fixed number two connections routinely share, so the
            // rebind below fails unless the old listener is gone first.
            inner.connection = None;
            if let Some(tunnel) = inner.tunnel.take() {
                tunnel.close().await;
            }
            inner.imported.clear();

            let (url, opened) = match tunnel {
                Some(config) => {
                    let (url, tunnel) = open_tunnel(&url, &config, policy).await?;
                    (url, Some(tunnel))
                }
                None => (url, None),
            };

            let connection = Connection::open(&url).await?;
            let engine = connection.engine();
            inner.connection = Some(connection);
            inner.tunnel = opened;
            Ok(engine)
        })
    }

    /// Opens `url` once to see whether it answers, then throws the connection away.
    ///
    /// This is the connection form's Test button, and it must not disturb what the canvas is
    /// querying: it never locks `Inner`, so it neither replaces the open connection nor queues
    /// behind a long-running statement.
    pub fn probe(
        &self,
        url: String,
        tunnel: Option<TunnelConfig>,
        policy: HostKeyPolicy,
    ) -> Pending<Engine> {
        self.spawn(move |_| async move {
            let (url, opened) = match tunnel {
                // Port 0, whatever the config says. `local_port` is a fixed number several
                // connections routinely share, and the live tunnel is already holding it — a
                // probe that asked for the same port would fail to bind and report a false
                // negative about a database that is perfectly reachable. An OS-picked port also
                // means a tunnel dropped on the error path below cannot collide with anything.
                Some(config) => {
                    let config = TunnelConfig {
                        local_port: 0,
                        ..config
                    };
                    let (url, tunnel) = open_tunnel(&url, &config, policy).await?;
                    (url, Some(tunnel))
                }
                None => (url, None),
            };

            // The connection is dropped at the end of this block, before the tunnel it runs
            // through is closed underneath it.
            let engine = {
                let connection = Connection::open(&url).await?;
                connection.engine()
            };
            if let Some(tunnel) = opened {
                tunnel.close().await;
            }
            Ok(engine)
        })
    }

    pub fn query(&self, sql: String) -> Pending<ResultSet> {
        self.spawn(move |inner| async move {
            let mut inner = inner.lock().await;
            connected(&mut inner)?.query(&sql).await
        })
    }

    pub fn execute(&self, sql: String) -> Pending<u64> {
        self.spawn(move |inner| async move {
            let mut inner = inner.lock().await;
            connected(&mut inner)?.execute(&sql).await
        })
    }

    pub fn schema(&self) -> Pending<Schema> {
        self.spawn(|inner| async move {
            let mut inner = inner.lock().await;
            let imported = inner.imported.clone();
            connected(&mut inner)?.schema(&imported).await
        })
    }

    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.inner
            .try_lock()
            .is_ok_and(|inner| inner.connection.is_some())
    }

    fn spawn<T, F, Fut>(&self, work: F) -> Pending<T>
    where
        T: Send + 'static,
        F: FnOnce(Arc<Mutex<Inner>>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, DbError>> + Send,
    {
        let (sender, receiver) = oneshot::channel();
        let inner = Arc::clone(&self.inner);
        self.runtime.spawn(async move {
            // The receiver is dropped when the caller stops caring; that is not an error.
            let _ = sender.send(work(inner).await);
        });
        receiver
    }
}

fn connected(inner: &mut Inner) -> Result<&mut Connection, DbError> {
    inner
        .connection
        .as_mut()
        .ok_or_else(|| DbError::Query("not connected".to_string()))
}

/// Opens the tunnel and rewrites the URL to point at its local end.
async fn open_tunnel(
    url: &str,
    config: &TunnelConfig,
    policy: HostKeyPolicy,
) -> Result<(String, SshTunnel), DbError> {
    let mut parsed = url::Url::parse(url)
        .map_err(|error| DbError::Tunnel(format!("invalid connection url: {error}")))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| DbError::Tunnel("connection url has no host".to_string()))?
        .to_string();
    let port = parsed
        .port()
        .or_else(|| Engine::from_url(url).default_port())
        .ok_or_else(|| DbError::Tunnel(format!("cannot infer a port for {}", parsed.scheme())))?;

    let tunnel = SshTunnel::open(config, (&host, port), policy).await?;

    parsed
        .set_host(Some("127.0.0.1"))
        .map_err(|error| DbError::Tunnel(format!("could not rewrite url host: {error}")))?;
    parsed
        .set_port(Some(tunnel.local_port()))
        .map_err(|()| DbError::Tunnel("could not rewrite url port".to_string()))?;
    Ok((parsed.to_string(), tunnel))
}

#[cfg(test)]
mod tests {
    use super::Session;

    /// A call with no connection must fail rather than hang, so the UI can show the error
    /// instead of spinning forever.
    #[test]
    fn a_query_without_a_connection_reports_it() {
        let session = Session::new().unwrap();
        let pending = session.query("select 1".to_string());
        let result = session.runtime.block_on(pending).unwrap();
        assert!(result.is_err());
    }

    #[test]
    fn a_fresh_session_is_not_connected() {
        assert!(!Session::new().unwrap().is_connected());
    }

    /// The Test button must never cost the canvas its connection. A probe that fails has to
    /// leave the session exactly as it found it — here, still unconnected rather than holding a
    /// broken connection the next query would trip over.
    #[test]
    fn a_failed_probe_leaves_the_session_alone() {
        let session = Session::new().unwrap();
        let pending = session.probe(
            "postgres://nobody@127.0.0.1:1/none".to_string(),
            None,
            crate::tunnel::HostKeyPolicy::TrustOnFirstUse,
        );

        let result = session.runtime.block_on(pending).unwrap();

        assert!(result.is_err(), "nothing is listening on port 1");
        assert!(!session.is_connected());
    }
}
