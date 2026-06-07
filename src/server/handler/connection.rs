use std::{
    io::{ErrorKind, Read, Write},
    net::TcpStream,
    thread,
};

use crate::{
    config,
    error::AppError,
    server::{
        Clients,
        broadcast::{
            broadcast, broadcast_presence, message_history, record_message, remove_client,
            set_client_username,
        },
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

    set_client_username(&clients, sender_addr, &username)?;
    send_message_history(&mut stream, &clients)?;
    broadcast_presence(&clients)?;

    let join_msg = format!("@{} has entered the chat. Say hello!\n", username);
    println!("{}", join_msg.trim());

    record_message(&clients, &join_msg)?;
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

        if !simulated_latency.is_zero() {
            thread::sleep(simulated_latency);
        }

        record_message(&clients, &msg)?;
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
    record_message(clients, &leave_msg)?;
    broadcast(clients, &leave_msg, None)?;
    broadcast_presence(clients)?;

    Ok(())
}

fn send_message_history(stream: &mut TcpStream, clients: &Clients) -> Result<(), AppError> {
    let history = message_history(clients)?;

    if history.is_empty() {
        return Ok(());
    }

    for message in history {
        stream.write_all(message.as_bytes())?;
        stream.write_all(b"\n")?;
    }

    Ok(())
}
