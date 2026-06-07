use std::{io::Write, net::SocketAddr};

use crate::{error::AppError, server::Clients};

const MESSAGE_HISTORY_LIMIT: usize = 100;

pub fn broadcast(
    clients: &Clients,
    msg: &str,
    exclude_addr: Option<SocketAddr>,
) -> Result<(), AppError> {
    let mut state = clients.lock().map_err(|_| AppError::ClientStatePoisoned)?;
    let mut index = 0;

    while index < state.clients.len() {
        let client = &mut state.clients[index];

        if exclude_addr.is_some_and(|addr| client.addr == addr) {
            index += 1;
            continue;
        }

        if let Err(error) = client.stream.write_all(msg.as_bytes()) {
            eprintln!("dropping client during broadcast: {error}");
            state.clients.remove(index);
            continue;
        }

        index += 1;
    }

    Ok(())
}

pub fn remove_client(clients: &Clients, target_addr: SocketAddr) -> Result<(), AppError> {
    let mut state = clients.lock().map_err(|_| AppError::ClientStatePoisoned)?;
    state.clients.retain(|client| client.addr != target_addr);
    Ok(())
}

pub fn set_client_username(
    clients: &Clients,
    target_addr: SocketAddr,
    username: &str,
) -> Result<(), AppError> {
    let mut state = clients.lock().map_err(|_| AppError::ClientStatePoisoned)?;

    if let Some(client) = state
        .clients
        .iter_mut()
        .find(|client| client.addr == target_addr)
    {
        client.username = Some(username.to_string());
    }

    Ok(())
}

pub fn usernames(clients: &Clients) -> Result<Vec<String>, AppError> {
    let state = clients.lock().map_err(|_| AppError::ClientStatePoisoned)?;
    Ok(state
        .clients
        .iter()
        .filter_map(|client| client.username.clone())
        .collect())
}

pub fn message_history(clients: &Clients) -> Result<Vec<String>, AppError> {
    let state = clients.lock().map_err(|_| AppError::ClientStatePoisoned)?;
    Ok(state.history.clone())
}

pub fn record_message(clients: &Clients, msg: &str) -> Result<(), AppError> {
    let mut state = clients.lock().map_err(|_| AppError::ClientStatePoisoned)?;
    state.history.push(msg.trim_end_matches('\n').to_string());

    if state.history.len() > MESSAGE_HISTORY_LIMIT {
        let overflow = state.history.len() - MESSAGE_HISTORY_LIMIT;
        state.history.drain(0..overflow);
    }

    Ok(())
}

pub fn broadcast_presence(clients: &Clients) -> Result<(), AppError> {
    let users = usernames(clients)?;
    let payload = format!("__users__:{}\n", users.join(","));
    broadcast(clients, &payload, None)?;
    Ok(())
}
