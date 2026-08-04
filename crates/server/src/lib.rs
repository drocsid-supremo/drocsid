mod connection;
mod delivery;
mod state;

use drocsid_config::ServerConfig;
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
};
use thiserror::Error;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tracing::{error, info, warn};

use connection::ConnectionHandler;
use state::{MAX_CONNECTIONS, MAX_CONNECTIONS_PER_IP, cleanup_rate_limits, new_shared_state};

pub(crate) struct ConnectionPermit {
    _global: OwnedSemaphorePermit,
    ip: IpAddr,
    per_ip: Arc<Mutex<HashMap<IpAddr, usize>>>,
}

impl Drop for ConnectionPermit {
    fn drop(&mut self) {
        if let Ok(mut connections) = self.per_ip.lock()
            && let Some(count) = connections.get_mut(&self.ip)
        {
            *count -= 1;
            if *count == 0 {
                connections.remove(&self.ip);
            }
        }
    }
}

struct ConnectionAdmission {
    global: Arc<Semaphore>,
    per_ip: Arc<Mutex<HashMap<IpAddr, usize>>>,
}

impl ConnectionAdmission {
    fn new() -> Self {
        Self {
            global: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
            per_ip: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn try_acquire(&self, address: SocketAddr) -> Option<ConnectionPermit> {
        let global = self.global.clone().try_acquire_owned().ok()?;
        let mut connections = self.per_ip.lock().ok()?;
        let count = connections.entry(address.ip()).or_default();
        if *count >= MAX_CONNECTIONS_PER_IP {
            drop(global);
            return None;
        }
        *count += 1;
        Some(ConnectionPermit {
            _global: global,
            ip: address.ip(),
            per_ip: self.per_ip.clone(),
        })
    }
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ServerError {
    #[error("received an empty username during client handshake")]
    EmptyHandshakeUsername,
    #[error("username contains control characters")]
    UsernameContainsControlCharacters,
    #[error("username exceeds the maximum length")]
    UsernameTooLong,
    #[error("message exceeds the maximum length")]
    MessageTooLong,
    #[error("received an unexpected protocol frame type")]
    InvalidFrameType,
    #[error("unsupported protocol version")]
    UnsupportedProtocolVersion,
    #[error("unknown protocol frame type")]
    UnknownProtocolFrameType,
    #[error("truncated protocol frame")]
    TruncatedProtocolFrame,
    #[error("failed to encode a presence frame")]
    PresenceEncodingFailed,
    #[error("received invalid UTF-8 data")]
    InvalidUtf8,
    #[error("connection limit reached")]
    ConnectionLimitReached,
    #[error("message rate limit exceeded")]
    MessageRateLimitExceeded,
    #[error("client outbound queue is full")]
    OutboundQueueFull,
    #[error("client outbound writer is gone")]
    OutboundWriterGone,
    #[error("client is unknown or no longer active")]
    UnknownClient,
    #[error("shared client state is poisoned")]
    ClientStatePoisoned,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub fn run_server(config: &ServerConfig) -> Result<(), ServerError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run_server_async(config))
}

async fn run_server_async(config: &ServerConfig) -> Result<(), ServerError> {
    let listener = tokio::net::TcpListener::bind(&config.bind_addr).await?;
    let address = listener.local_addr()?;
    if bind_is_not_loopback(address.ip()) {
        warn!(bind_addr = %address, phase = "startup", "server is listening beyond localhost; traffic is unauthenticated and unencrypted");
    }
    let state = new_shared_state();
    let admission = Arc::new(ConnectionAdmission::new());
    let cleanup_state = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            if let Err(error) = cleanup_rate_limits(&cleanup_state) {
                warn!(error = %error, phase = "rate_limit_cleanup", "failed to clean up idle rate limits");
            }
        }
    });
    info!(bind_addr = %address, max_connections = MAX_CONNECTIONS, max_connections_per_ip = MAX_CONNECTIONS_PER_IP, simulated_latency_ms = config.simulated_latency.as_millis(), "server listening");
    loop {
        let (stream, peer_addr) = listener.accept().await?;
        info!(%peer_addr, "connection accepted");
        let Some(permit) = admission.try_acquire(peer_addr) else {
            warn!(%peer_addr, phase = "admission", "connection limit reached");
            drop(stream);
            continue;
        };
        let handler = ConnectionHandler::new(state.clone(), config.simulated_latency);
        tokio::spawn(async move {
            if let Err(error) = handler.serve(stream, peer_addr, permit).await {
                error!(%peer_addr, error = %error, phase = "connection", "connection closed with error");
            }
        });
    }
}

fn bind_is_not_loopback(ip: IpAddr) -> bool {
    !ip.is_loopback()
}

#[cfg(test)]
mod tests {
    use super::{ConnectionAdmission, MAX_CONNECTIONS_PER_IP, bind_is_not_loopback};
    #[test]
    fn recognizes_non_loopback_bind_addresses() {
        assert!(!bind_is_not_loopback("127.0.0.1".parse().unwrap()));
        assert!(!bind_is_not_loopback("::1".parse().unwrap()));
        assert!(bind_is_not_loopback("0.0.0.0".parse().unwrap()));
        assert!(bind_is_not_loopback("::".parse().unwrap()));
    }

    #[test]
    fn admission_limits_pending_connections_per_ip() {
        let admission = ConnectionAdmission::new();
        let address = "127.0.0.1:7878".parse().unwrap();
        let permits = (0..MAX_CONNECTIONS_PER_IP)
            .map(|_| admission.try_acquire(address).unwrap())
            .collect::<Vec<_>>();

        assert!(admission.try_acquire(address).is_none());
        drop(permits);
        assert!(admission.try_acquire(address).is_some());
    }
}
