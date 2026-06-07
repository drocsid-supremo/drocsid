use std::{io, net::AddrParseError};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("missing mode argument, expected `mode=server`, `mode=client`, or `mode=tui`")]
    MissingMode,
    #[error("missing username argument, expected `username=<name>`")]
    MissingUsername,
    #[error("invalid mode `{0}`, expected `server`, `client`, or `tui`")]
    InvalidMode(String),
    #[error("received an empty username during client handshake")]
    EmptyHandshakeUsername,
    #[error("shared client state is poisoned")]
    ClientStatePoisoned,
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("address parse error: {0}")]
    AddrParse(#[from] AddrParseError),
}
