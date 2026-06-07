mod connection;
mod state;

use std::{net::TcpListener, sync::Arc, thread};

use connection::ConnectionHandler;
use state::{new_shared_state, register_client};

use crate::{config::ServerConfig, error::AppError};

pub fn run_server(server_config: &ServerConfig) -> Result<(), AppError> {
    let listener = TcpListener::bind(&server_config.bind_addr)?;
    let state = new_shared_state();

    for stream in listener.incoming() {
        let stream = match stream {
            Ok(stream) => stream,
            Err(error) => {
                eprintln!("failed to accept incoming connection: {error}");
                continue;
            }
        };

        if let Err(error) = register_client(&state, &stream) {
            eprintln!("failed to register client: {error}");
            continue;
        }

        let state = Arc::clone(&state);
        let simulated_latency = server_config.simulated_latency;

        thread::spawn(move || {
            let handler = ConnectionHandler::new(state, simulated_latency);
            if let Err(error) = handler.serve(stream) {
                eprintln!("connection handler failed: {error}");
            }
        });
    }

    Ok(())
}
