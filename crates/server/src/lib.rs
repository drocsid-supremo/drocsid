mod connection;
mod state;

use std::{
    net::TcpListener,
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
use state::{MAX_CONNECTIONS, new_shared_state};
use tracing::{debug, error, info, info_span, warn};

static NEXT_CONNECTION_ID: AtomicU64 = AtomicU64::new(1);
const CONNECTION_WORKER_COUNT: usize = 16;
const PENDING_CONNECTION_QUEUE_SIZE: usize = MAX_CONNECTIONS;

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("received an empty username during client handshake")]
    EmptyHandshakeUsername,
    #[error("username contains control characters")]
    UsernameContainsControlCharacters,
    #[error("username exceeds the maximum length")]
    UsernameTooLong,
    #[error("message exceeds the maximum length")]
    MessageTooLong,
    #[error("message uses a reserved protocol prefix")]
    ReservedMessagePrefix,
    #[error("received invalid UTF-8 data")]
    InvalidUtf8,
    #[error("connection limit reached")]
    ConnectionLimitReached,
    #[error("message rate limit exceeded")]
    MessageRateLimitExceeded,
    #[error("shared client state is poisoned")]
    ClientStatePoisoned,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub fn run_server(server_config: &ServerConfig) -> Result<(), ServerError> {
    let listener = TcpListener::bind(&server_config.bind_addr)?;
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

fn spawn_connection_worker(
    receiver: Arc<Mutex<Receiver<std::net::TcpStream>>>,
    state: Arc<std::sync::Mutex<state::ServerState>>,
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
            let username = match handler.authenticate(&mut stream, peer_addr) {
                Ok(username) => username,
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
                if let Err(error) = handler.serve_authenticated(stream, peer_addr, username) {
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
