use std::{
    io::Write,
    net::{SocketAddr, TcpStream},
    sync::{Arc, Mutex},
};

use crate::{error::AppError, protocol::build_users_event};

const MESSAGE_HISTORY_LIMIT: usize = 100;

pub type ServerStateHandle = Arc<Mutex<ServerState>>;

pub struct ServerState {
    clients: Vec<ClientEntry>,
    history: Vec<String>,
}

struct ClientEntry {
    addr: SocketAddr,
    stream: TcpStream,
    username: Option<String>,
}

impl ServerState {
    fn new() -> Self {
        Self {
            clients: Vec::new(),
            history: Vec::new(),
        }
    }
}

pub fn new_shared_state() -> ServerStateHandle {
    Arc::new(Mutex::new(ServerState::new()))
}

pub fn register_client(state: &ServerStateHandle, stream: &TcpStream) -> Result<(), AppError> {
    let mut state = state.lock().map_err(|_| AppError::ClientStatePoisoned)?;
    state.clients.push(ClientEntry {
        addr: stream.peer_addr()?,
        stream: stream.try_clone()?,
        username: None,
    });
    Ok(())
}

pub fn remove_client(state: &ServerStateHandle, target_addr: SocketAddr) -> Result<(), AppError> {
    let mut state = state.lock().map_err(|_| AppError::ClientStatePoisoned)?;
    state.clients.retain(|client| client.addr != target_addr);
    Ok(())
}

pub fn set_client_username(
    state: &ServerStateHandle,
    target_addr: SocketAddr,
    username: &str,
) -> Result<(), AppError> {
    let mut state = state.lock().map_err(|_| AppError::ClientStatePoisoned)?;

    if let Some(client) = state
        .clients
        .iter_mut()
        .find(|client| client.addr == target_addr)
    {
        client.username = Some(username.to_string());
    }

    Ok(())
}

pub fn broadcast(
    state: &ServerStateHandle,
    message: &str,
    exclude_addr: Option<SocketAddr>,
) -> Result<(), AppError> {
    let mut state = state.lock().map_err(|_| AppError::ClientStatePoisoned)?;
    let mut index = 0;

    while index < state.clients.len() {
        let client = &mut state.clients[index];

        if exclude_addr.is_some_and(|addr| client.addr == addr) {
            index += 1;
            continue;
        }

        if let Err(error) = client.stream.write_all(message.as_bytes()) {
            eprintln!("dropping client during broadcast: {error}");
            state.clients.remove(index);
            continue;
        }

        index += 1;
    }

    Ok(())
}

pub fn usernames(state: &ServerStateHandle) -> Result<Vec<String>, AppError> {
    let state = state.lock().map_err(|_| AppError::ClientStatePoisoned)?;
    Ok(state
        .clients
        .iter()
        .filter_map(|client| client.username.clone())
        .collect())
}

pub fn broadcast_presence(state: &ServerStateHandle) -> Result<(), AppError> {
    let users = usernames(state)?;
    broadcast(state, &build_users_event(&users), None)
}

pub fn message_history(state: &ServerStateHandle) -> Result<Vec<String>, AppError> {
    let state = state.lock().map_err(|_| AppError::ClientStatePoisoned)?;
    Ok(state.history.clone())
}

pub fn record_message(state: &ServerStateHandle, message: &str) -> Result<(), AppError> {
    let mut state = state.lock().map_err(|_| AppError::ClientStatePoisoned)?;
    state
        .history
        .push(message.trim_end_matches('\n').to_string());

    if state.history.len() > MESSAGE_HISTORY_LIMIT {
        let overflow = state.history.len() - MESSAGE_HISTORY_LIMIT;
        state.history.drain(0..overflow);
    }

    Ok(())
}
