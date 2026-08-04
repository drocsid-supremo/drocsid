mod connection;
mod delivery;
mod state;

use std::{
    net::{IpAddr, TcpListener},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, TrySendError},
    },
    thread,
};

use drocsid_config::ServerConfig;
use thiserror::Error;

use connection::ConnectionHandler;
use state::{MAX_CONNECTIONS, ServerStateHandle, new_shared_state};
use tracing::{debug, error, info, info_span, warn};

static NEXT_CONNECTION_ID: AtomicU64 = AtomicU64::new(1);
const CONNECTION_WORKER_COUNT: usize = 16;
const PENDING_CONNECTION_QUEUE_SIZE: usize = MAX_CONNECTIONS;

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

pub fn run_server(server_config: &ServerConfig) -> Result<(), ServerError> {
    let listener = TcpListener::bind(&server_config.bind_addr)?;
    let bound_addr = listener.local_addr()?;

    if bind_is_not_loopback(bound_addr.ip()) {
        warn!(
            bind_addr = %bound_addr,
            phase = "startup",
            "server is listening beyond localhost; traffic is unauthenticated and unencrypted"
        );
    }

    let state = new_shared_state();
    info!(
        bind_addr = %server_config.bind_addr,
        simulated_latency_ms = server_config.simulated_latency.as_millis(),
        "server listening"
    );

    let (connection_sender, connection_receiver) =
        mpsc::sync_channel(PENDING_CONNECTION_QUEUE_SIZE);
    let connection_receiver = Arc::new(Mutex::new(connection_receiver));

    for _ in 0..CONNECTION_WORKER_COUNT {
        spawn_connection_worker(
            Arc::clone(&connection_receiver),
            Arc::clone(&state),
            server_config.simulated_latency,
        );
    }

    for stream in listener.incoming() {
        let stream = match stream {
            Ok(stream) => stream,
            Err(error) => {
                warn!(error = %error, error_kind = ?error.kind(), "failed to accept incoming connection");
                continue;
            }
        };

        let peer_addr = stream.peer_addr()?;
        info!(%peer_addr, "connection accepted");

        match connection_sender.try_send(stream) {
            Ok(()) => {}
            Err(TrySendError::Full(stream)) => {
                warn!(%peer_addr, "connection queue is full");
                drop(stream);
            }
            Err(TrySendError::Disconnected(stream)) => {
                error!(
                    %peer_addr,
                    "connection channel disconnected; shutting down accept loop"
                );
                drop(stream);
                return Ok(());
            }
        }
    }

    Ok(())
}

fn bind_is_not_loopback(ip: IpAddr) -> bool {
    !ip.is_loopback()
}

fn spawn_connection_worker(
    receiver: Arc<Mutex<Receiver<std::net::TcpStream>>>,
    state: ServerStateHandle,
    simulated_latency: std::time::Duration,
) {
    thread::spawn(move || {
        loop {
            let stream = match receiver.lock() {
                Ok(receiver) => receiver.recv(),
                Err(_) => return,
            };

            let stream = match stream {
                Ok(stream) => stream,
                Err(_) => return,
            };
            let peer_addr = match stream.peer_addr() {
                Ok(peer_addr) => peer_addr,
                Err(error) => {
                    warn!(error = %error, error_kind = ?error.kind(), "failed to inspect connection peer");
                    continue;
                }
            };
            let connection_id = NEXT_CONNECTION_ID.fetch_add(1, Ordering::Relaxed);
            let handler = ConnectionHandler::new(Arc::clone(&state), simulated_latency);
            let mut stream = stream;
            let (username, token) = match handler.authenticate(&mut stream, peer_addr) {
                Ok(session) => session,
                Err(error) => {
                    debug!(
                        %peer_addr,
                        error = %error,
                        error_kind = ?error_kind(&error),
                        phase = "handshake",
                        "connection handler failed"
                    );
                    continue;
                }
            };

            thread::spawn(move || {
                let _connection_span =
                    info_span!("connection", connection_id, peer_addr = %peer_addr).entered();
                if let Err(error) = handler.serve_authenticated(stream, peer_addr, username, token)
                {
                    error!(
                        error = %error,
                        error_kind = ?error_kind(&error),
                        phase = "connection",
                        "connection handler failed"
                    );
                }
            });
        }
    });
}

fn error_kind(error: &ServerError) -> Option<std::io::ErrorKind> {
    match error {
        ServerError::Io(error) => Some(error.kind()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::bind_is_not_loopback;

    #[test]
    fn recognizes_non_loopback_bind_addresses() {
        assert!(!bind_is_not_loopback("127.0.0.1".parse().unwrap()));
        assert!(!bind_is_not_loopback("::1".parse().unwrap()));
        assert!(bind_is_not_loopback("0.0.0.0".parse().unwrap()));
        assert!(bind_is_not_loopback("::".parse().unwrap()));
    }
}
