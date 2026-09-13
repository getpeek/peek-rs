//! SSH tunnelling, so a database that only listens on a private network can be reached through
//! a bastion.
//!
//! Ported from `~/labs/peek/src-tauri/src/ssh_tunnel.rs` with **one deliberate change**: that
//! implementation's `check_server_key` is
//!
//! ```ignore
//! async fn check_server_key(&mut self, _key: &PublicKey) -> Result<bool, Self::Error> {
//!     Ok(true)
//! }
//! ```
//!
//! — every host key accepted, no `known_hosts` check, no fingerprint pinning. Database
//! credentials travel through this tunnel, so anything able to answer on the bastion's address
//! can collect them. [`HostKeyPolicy`] replaces that with a real check.

use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use russh::client::{self, Handle};
use russh::keys::known_hosts::learn_known_hosts;
use russh::keys::ssh_key::PublicKey;
use russh::keys::{Error as KeyError, PrivateKeyWithHashAlg, check_known_hosts, load_secret_key};
use russh::{ChannelMsg, Disconnect};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::error::DbError;

/// Where to forward, and how to reach the bastion. Mirrors `peek_config::SshTunnelConfig`,
/// which is the frozen `settings.json` shape; kept separate so `peek-db` does not depend on the
/// config crate.
#[derive(Debug, Clone)]
pub struct TunnelConfig {
    pub ssh_host: String,
    pub ssh_port: u16,
    pub ssh_user: String,
    pub key_path: PathBuf,
    /// `0` lets the OS pick, which is what makes reconnecting safe: a fixed port is still bound
    /// by the dying tunnel for a moment and the rebind fails.
    pub local_port: u16,
}

/// What to do about the bastion's host key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HostKeyPolicy {
    /// Accept only a key already in `~/.ssh/known_hosts`.
    Strict,
    /// Accept and record a key for a host that has none yet; still refuse a key that has
    /// **changed**, which is the case that means someone is in the middle.
    #[default]
    TrustOnFirstUse,
}

/// Why a host key was refused. Recorded by the handler, which cannot return a message of its own
/// through russh's `Result<bool, _>`.
#[derive(Debug, Clone)]
enum Rejection {
    Unknown {
        fingerprint: String,
    },
    Changed {
        fingerprint: String,
    },
    /// `known_hosts` could not be read far enough to answer. Not a changed key, and saying so
    /// would send the user hunting for an attacker that is not there.
    Unreadable {
        error: String,
    },
}

impl Rejection {
    fn message(&self, host: &str) -> String {
        match self {
            Self::Unknown { fingerprint } => format!(
                "host key for {host} is not in known_hosts (fingerprint {fingerprint}); \
                 add it with `ssh-keyscan` or connect once with `ssh` to record it"
            ),
            Self::Changed { fingerprint } => format!(
                "HOST KEY CHANGED for {host} (now {fingerprint}). The bastion may be \
                 impersonated; refusing to send database credentials through it"
            ),
            Self::Unreadable { error } => {
                format!("could not check the host key for {host} against known_hosts: {error}")
            }
        }
    }
}

struct Client {
    host: String,
    port: u16,
    policy: HostKeyPolicy,
    rejection: Arc<Mutex<Option<Rejection>>>,
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &PublicKey) -> Result<bool, Self::Error> {
        let fingerprint = key.fingerprint(russh::keys::HashAlg::Sha256).to_string();
        match check_known_hosts(&self.host, self.port, key) {
            Ok(true) => Ok(true),
            // The host has an entry under this algorithm, and it is not this key.
            Err(KeyError::KeyChanged { .. }) => {
                self.record(Rejection::Changed { fingerprint });
                Ok(false)
            }
            // No home directory, a non-UTF-8 byte in known_hosts, an unparsable matching line:
            // the file could not answer, which is not the same as an answer of "changed".
            Err(error) => {
                self.record(Rejection::Unreadable {
                    error: error.to_string(),
                });
                Ok(false)
            }
            // No entry for this host at all.
            Ok(false) => match self.policy {
                HostKeyPolicy::Strict => {
                    self.record(Rejection::Unknown { fingerprint });
                    Ok(false)
                }
                HostKeyPolicy::TrustOnFirstUse => {
                    if let Err(error) = learn_known_hosts(&self.host, self.port, key) {
                        log::warn!("could not record host key for {}: {error}", self.host);
                    }
                    Ok(true)
                }
            },
        }
    }
}

impl Client {
    fn record(&self, rejection: Rejection) {
        if let Ok(mut slot) = self.rejection.lock() {
            *slot = Some(rejection);
        }
    }
}

pub struct SshTunnel {
    local_port: u16,
    shutdown: Arc<Notify>,
    accept_task: Option<JoinHandle<()>>,
    session: Option<Arc<Handle<Client>>>,
}

impl fmt::Debug for SshTunnel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SshTunnel")
            .field("local_port", &self.local_port)
            .finish_non_exhaustive()
    }
}

impl SshTunnel {
    /// Opens a tunnel forwarding `127.0.0.1:local_port` to `remote_host:remote_port` through the
    /// bastion.
    ///
    /// # Errors
    /// [`DbError::Tunnel`] when the key cannot be loaded, the bastion is unreachable, the host
    /// key is refused, authentication fails, or the local port cannot be bound.
    pub async fn open(
        config: &TunnelConfig,
        remote: (&str, u16),
        policy: HostKeyPolicy,
    ) -> Result<Self, DbError> {
        let (remote_host, remote_port) = remote;
        let key = load_secret_key(&config.key_path, None).map_err(|error| {
            DbError::Tunnel(format!(
                "could not load ssh key {}: {error}",
                config.key_path.display()
            ))
        })?;

        let rejection = Arc::new(Mutex::new(None));
        let client = Client {
            host: config.ssh_host.clone(),
            port: config.ssh_port,
            policy,
            rejection: Arc::clone(&rejection),
        };

        let ssh_config = Arc::new(client::Config {
            nodelay: true,
            ..client::Config::default()
        });

        let mut session = client::connect(
            ssh_config,
            (config.ssh_host.as_str(), config.ssh_port),
            client,
        )
        .await
        .map_err(|error| {
            // A refused host key surfaces here as a generic handshake failure; the handler's
            // own reason is far more useful.
            let recorded = rejection.lock().ok().and_then(|slot| slot.clone());
            recorded.map_or_else(
                || {
                    DbError::Tunnel(format!(
                        "ssh connect to {}:{} failed: {error}",
                        config.ssh_host, config.ssh_port
                    ))
                },
                |rejection| DbError::Tunnel(rejection.message(&config.ssh_host)),
            )
        })?;

        let hash_alg = session
            .best_supported_rsa_hash()
            .await
            .map_err(|error| DbError::Tunnel(format!("ssh negotiation failed: {error}")))?
            .flatten();

        let authenticated = session
            .authenticate_publickey(
                config.ssh_user.clone(),
                PrivateKeyWithHashAlg::new(Arc::new(key), hash_alg),
            )
            .await
            .map_err(|error| DbError::Tunnel(format!("ssh auth error: {error}")))?;
        if !authenticated.success() {
            return Err(DbError::Tunnel(format!(
                "ssh publickey authentication failed for user {}",
                config.ssh_user
            )));
        }

        let listener = TcpListener::bind(("127.0.0.1", config.local_port))
            .await
            .map_err(|error| {
                DbError::Tunnel(format!(
                    "could not bind local port {}: {error}",
                    config.local_port
                ))
            })?;
        let local_port = listener
            .local_addr()
            .map_err(|error| DbError::Tunnel(format!("could not read local address: {error}")))?
            .port();

        let shutdown = Arc::new(Notify::new());
        let session = Arc::new(session);
        let accept_task = spawn_accept_loop(
            listener,
            Arc::clone(&session),
            (remote_host.to_string(), remote_port),
            Arc::clone(&shutdown),
        );

        Ok(Self {
            local_port,
            shutdown,
            accept_task: Some(accept_task),
            session: Some(session),
        })
    }

    /// The port to point the database URL at.
    #[must_use]
    pub fn local_port(&self) -> u16 {
        self.local_port
    }

    /// Shuts the tunnel down and waits for the local listener to be released.
    ///
    /// [`Drop`] cannot do this: `abort()` only schedules the accept task's cancellation, so the
    /// `TcpListener` it owns is still bound for a moment afterwards. `local_port` is a fixed
    /// number in `settings.json` and two connections routinely share it, so the next tunnel binds
    /// it microseconds later and fails with `Address already in use`. Awaiting the task here is
    /// what makes reconnecting to a second tunnelled connection work.
    pub async fn close(mut self) {
        // `notify_one` stores a permit; `notify_waiters` would be lost in the window between the
        // accept loop's iterations, where no `notified()` future exists yet.
        self.shutdown.notify_one();
        if let Some(handle) = self.accept_task.take() {
            let _ = handle.await;
        }
        if let Some(session) = self.session.take() {
            let _ = session
                .disconnect(Disconnect::ByApplication, "", "en")
                .await;
        }
    }
}

/// The ungraceful path, for a tunnel dropped without [`SshTunnel::close`]. Both fields are
/// already taken after a `close`, so this then does nothing.
impl Drop for SshTunnel {
    fn drop(&mut self) {
        self.shutdown.notify_waiters();
        if let Some(handle) = self.accept_task.take() {
            handle.abort();
        }
        if let Some(session) = self.session.take() {
            tokio::spawn(async move {
                let _ = session
                    .disconnect(Disconnect::ByApplication, "", "en")
                    .await;
            });
        }
    }
}

fn spawn_accept_loop(
    listener: TcpListener,
    session: Arc<Handle<Client>>,
    remote: (String, u16),
    shutdown: Arc<Notify>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let (remote_host, remote_port) = remote;
        loop {
            tokio::select! {
                () = shutdown.notified() => break,
                accepted = listener.accept() => {
                    let Ok((socket, originator)) = accepted else {
                        log::error!("ssh tunnel accept failed; closing the tunnel");
                        break;
                    };
                    let session = Arc::clone(&session);
                    let remote_host = remote_host.clone();
                    tokio::spawn(async move {
                        let origin = (originator.ip().to_string(), u32::from(originator.port()));
                        if let Err(error) =
                            forward(socket, origin, session, (remote_host, u32::from(remote_port)))
                                .await
                        {
                            log::error!("ssh tunnel forward error: {error}");
                        }
                    });
                }
            }
        }
    })
}

/// Pumps one accepted socket through a `direct-tcpip` channel until either side closes.
async fn forward(
    mut stream: TcpStream,
    origin: (String, u32),
    session: Arc<Handle<Client>>,
    remote: (String, u32),
) -> Result<(), String> {
    let (originator_address, originator_port) = origin;
    let (remote_host, remote_port) = remote;
    let mut channel = session
        .channel_open_direct_tcpip(
            remote_host,
            remote_port,
            originator_address,
            originator_port,
        )
        .await
        .map_err(|error| format!("channel_open_direct_tcpip: {error}"))?;

    let mut buffer = vec![0u8; 65536];
    let mut stream_closed = false;

    loop {
        tokio::select! {
            read = stream.read(&mut buffer), if !stream_closed => match read {
                Ok(0) => {
                    stream_closed = true;
                    channel.eof().await.map_err(|error| format!("channel eof: {error}"))?;
                }
                Ok(count) => channel
                    .data(&buffer[..count])
                    .await
                    .map_err(|error| format!("channel data: {error}"))?,
                Err(error) => return Err(format!("local read: {error}")),
            },
            message = channel.wait() => {
                let Some(message) = message else { break };
                match message {
                    ChannelMsg::Data { ref data } => stream
                        .write_all(data)
                        .await
                        .map_err(|error| format!("local write: {error}"))?,
                    ChannelMsg::Eof => {
                        if !stream_closed {
                            channel.eof().await.map_err(|e| format!("channel eof: {e}"))?;
                        }
                        break;
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}
