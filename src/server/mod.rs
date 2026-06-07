pub mod broadcast;
pub mod handler;
use crate::server::handler::connection::handle_connection;
use crate::{config, error::AppError};

use std::{
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{Arc, Mutex},
    thread,
};

pub struct ClientEntry {
    pub addr: SocketAddr,
    pub stream: TcpStream,
    pub username: Option<String>,
}

pub type Clients = Arc<Mutex<Vec<ClientEntry>>>;

pub fn run_server() -> Result<(), AppError> {
    let bind_addr = config::server_bind_addr();
    let listener = TcpListener::bind(&bind_addr)?;
    let clients: Clients = Arc::new(Mutex::new(Vec::new()));

    for stream in listener.incoming() {
        let stream = match stream {
            Ok(stream) => stream,
            Err(error) => {
                eprintln!("failed to accept incoming connection: {error}");
                continue;
            }
        };

        if let Err(error) = register_client(&clients, &stream) {
            eprintln!("failed to register client: {error}");
            continue;
        }

        let clients_clone = Arc::clone(&clients);

        thread::spawn(move || {
            if let Err(error) = handle_connection(stream, clients_clone) {
                eprintln!("connection handler failed: {error}");
            }
        });
    }

    Ok(())
}

fn register_client(clients: &Clients, stream: &TcpStream) -> Result<(), AppError> {
    let mut clients = clients.lock().map_err(|_| AppError::ClientStatePoisoned)?;
    clients.push(ClientEntry {
        addr: stream.peer_addr()?,
        stream: stream.try_clone()?,
        username: None,
    });
    Ok(())
}
