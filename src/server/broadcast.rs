use std::{io::Write, net::SocketAddr};

use crate::{error::AppError, server::Clients};

pub fn broadcast(
    clients: &Clients,
    msg: &str,
    exclude_addr: Option<SocketAddr>,
) -> Result<(), AppError> {
    let mut clients = clients.lock().map_err(|_| AppError::ClientStatePoisoned)?;
    let mut index = 0;

    while index < clients.len() {
        let client = &mut clients[index];

        if exclude_addr.is_some_and(|addr| client.peer_addr().ok() == Some(addr)) {
            index += 1;
            continue;
        }

        if let Err(error) = client.write_all(msg.as_bytes()) {
            eprintln!("dropping client during broadcast: {error}");
            clients.remove(index);
            continue;
        }

        index += 1;
    }

    Ok(())
}

pub fn remove_client(clients: &Clients, target_addr: SocketAddr) -> Result<(), AppError> {
    let mut clients = clients.lock().map_err(|_| AppError::ClientStatePoisoned)?;
    clients.retain(|client| client.peer_addr().ok() != Some(target_addr));
    Ok(())
}
