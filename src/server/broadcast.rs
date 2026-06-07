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

        if exclude_addr.is_some_and(|addr| client.addr == addr) {
            index += 1;
            continue;
        }

        if let Err(error) = client.stream.write_all(msg.as_bytes()) {
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
    clients.retain(|client| client.addr != target_addr);
    Ok(())
}

pub fn set_client_username(
    clients: &Clients,
    target_addr: SocketAddr,
    username: &str,
) -> Result<(), AppError> {
    let mut clients = clients.lock().map_err(|_| AppError::ClientStatePoisoned)?;

    if let Some(client) = clients.iter_mut().find(|client| client.addr == target_addr) {
        client.username = Some(username.to_string());
    }

    Ok(())
}

pub fn usernames(clients: &Clients) -> Result<Vec<String>, AppError> {
    let clients = clients.lock().map_err(|_| AppError::ClientStatePoisoned)?;
    Ok(clients
        .iter()
        .filter_map(|client| client.username.clone())
        .collect())
}

pub fn broadcast_presence(clients: &Clients) -> Result<(), AppError> {
    let users = usernames(clients)?;
    let payload = format!("__users__:{}\n", users.join(","));
    broadcast(clients, &payload, None)?;
    Ok(())
}
