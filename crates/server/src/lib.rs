mod connection;
mod state;

use std::{
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread,
};

use drocsid_config::ServerConfig;
use thiserror::Error;

use connection::ConnectionHandler;
use state::{new_shared_state, register_client};
use tracing::{error, info, info_span, warn};

static NEXT_CONNECTION_ID: AtomicU64 = AtomicU64::new(1);

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

        if let Err(error) = register_client(&state, &stream) {
            warn!(%peer_addr, error = %error, "failed to register client");
            continue;
        }

        let state = Arc::clone(&state);
        let simulated_latency = server_config.simulated_latency;
        let connection_id = NEXT_CONNECTION_ID.fetch_add(1, Ordering::Relaxed);

        thread::spawn(move || {
            let _connection_span =
                info_span!("connection", connection_id, peer_addr = %peer_addr).entered();
            let handler = ConnectionHandler::new(state, simulated_latency);
            if let Err(error) = handler.serve(stream) {
                error!(
                    error = %error,
                    error_kind = ?error_kind(&error),
                    phase = "connection",
                    "connection handler failed"
                );
            }
        });
    }

    Ok(())
}

fn error_kind(error: &ServerError) -> Option<std::io::ErrorKind> {
    match error {
        ServerError::Io(error) => Some(error.kind()),
        _ => None,
    }
}
