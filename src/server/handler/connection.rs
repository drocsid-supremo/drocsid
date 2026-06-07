use std::{
    io::{ErrorKind, Read},
    net::TcpStream,
    thread,
};

use crate::{
    config,
    error::AppError,
    server::{
        Clients,
        broadcast::{broadcast, remove_client},
    },
};

pub fn handle_connection(mut stream: TcpStream, clients: Clients) -> Result<(), AppError> {
    let mut buffer = [0; 1024];
    let sender_addr = stream.peer_addr()?;
    let simulated_latency = config::server_simulated_latency();

    let bytes = stream.read(&mut buffer)?;

    if bytes == 0 {
        remove_client(&clients, sender_addr)?;
        return Ok(());
    }

    let username = String::from_utf8_lossy(&buffer[..bytes]).trim().to_string();
    if username.is_empty() {
        remove_client(&clients, sender_addr)?;
        return Err(AppError::EmptyHandshakeUsername);
    }

    let join_msg = format!("{} has entered the chat. Say hello!\n", username);
    println!("{}", join_msg.trim());

    broadcast(&clients, &join_msg, None)?;

    loop {
        let bytes = match stream.read(&mut buffer) {
            Ok(bytes) => bytes,
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::BrokenPipe
                        | ErrorKind::ConnectionAborted
                        | ErrorKind::ConnectionReset
                ) =>
            {
                disconnect_client(&clients, sender_addr, &username)?;
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        };

        if bytes == 0 {
            disconnect_client(&clients, sender_addr, &username)?;
            return Ok(());
        }

        let msg = String::from_utf8_lossy(&buffer[..bytes]);
        print!("{}", msg);

        if !simulated_latency.is_zero() {
            thread::sleep(simulated_latency);
        }

        broadcast(&clients, &msg, None)?;
    }
}

fn disconnect_client(
    clients: &Clients,
    sender_addr: std::net::SocketAddr,
    username: &str,
) -> Result<(), AppError> {
    remove_client(clients, sender_addr)?;

    let leave_msg = format!("{} has left the chat\n", username);
    println!("{}", leave_msg.trim());
    broadcast(clients, &leave_msg, None)?;

    Ok(())
}
