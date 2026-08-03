use std::{
    collections::HashMap,
    io::Write,
    net::{IpAddr, SocketAddr, TcpStream},
    sync::{Arc, Mutex},
    time::Instant,
};

use crate::ServerError;
use drocsid_protocol::build_users_event;

const MESSAGE_HISTORY_LIMIT: usize = 100;
pub const MAX_CONNECTIONS: usize = 256;
pub const MAX_CONNECTIONS_PER_IP: usize = 4;
const MESSAGE_RATE_PER_SECOND: f64 = 10.0;
const MESSAGE_BURST_SIZE: f64 = 20.0;

pub type ServerStateHandle = Arc<Mutex<ServerState>>;

pub struct ServerState {
    clients: Vec<ClientEntry>,
    history: Vec<String>,
    rate_limits: HashMap<IpAddr, RateLimitState>,
}

struct RateLimitState {
    tokens: f64,
    last_refill: Instant,
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
            rate_limits: HashMap::new(),
        }
    }
}

pub fn new_shared_state() -> ServerStateHandle {
    Arc::new(Mutex::new(ServerState::new()))
}

pub fn register_client(state: &ServerStateHandle, stream: &TcpStream) -> Result<(), ServerError> {
    let mut state = state.lock().map_err(|_| ServerError::ClientStatePoisoned)?;
    let address = stream.peer_addr()?;

    if state.clients.len() >= MAX_CONNECTIONS
        || state
            .clients
            .iter()
            .filter(|client| client.addr.ip() == address.ip())
            .count()
            >= MAX_CONNECTIONS_PER_IP
    {
        return Err(ServerError::ConnectionLimitReached);
    }

    state.clients.push(ClientEntry {
        addr: address,
        stream: stream.try_clone()?,
        username: None,
    });
    Ok(())
}

pub fn remove_client(
    state: &ServerStateHandle,
    target_addr: SocketAddr,
) -> Result<(), ServerError> {
    let mut state = state.lock().map_err(|_| ServerError::ClientStatePoisoned)?;
    state.clients.retain(|client| client.addr != target_addr);

    if !state
        .clients
        .iter()
        .any(|client| client.addr.ip() == target_addr.ip())
    {
        state.rate_limits.remove(&target_addr.ip());
    }

    Ok(())
}

pub fn allow_message(
    state: &ServerStateHandle,
    sender_addr: SocketAddr,
) -> Result<(), ServerError> {
    let mut state = state.lock().map_err(|_| ServerError::ClientStatePoisoned)?;
    let now = Instant::now();
    let limiter = state
        .rate_limits
        .entry(sender_addr.ip())
        .or_insert_with(|| RateLimitState {
            tokens: MESSAGE_BURST_SIZE,
            last_refill: now,
        });
    let elapsed = now.duration_since(limiter.last_refill).as_secs_f64();

    limiter.tokens = (limiter.tokens + elapsed * MESSAGE_RATE_PER_SECOND).min(MESSAGE_BURST_SIZE);
    limiter.last_refill = now;

    if limiter.tokens < 1.0 {
        return Err(ServerError::MessageRateLimitExceeded);
    }

    limiter.tokens -= 1.0;
    Ok(())
}

pub fn set_client_username(
    state: &ServerStateHandle,
    target_addr: SocketAddr,
    username: &str,
) -> Result<(), ServerError> {
    let mut state = state.lock().map_err(|_| ServerError::ClientStatePoisoned)?;

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
) -> Result<(), ServerError> {
    let clients = {
        let state = state.lock().map_err(|_| ServerError::ClientStatePoisoned)?;
        state
            .clients
            .iter()
            .filter(|client| exclude_addr.is_none_or(|addr| client.addr != addr))
            .map(|client| Ok((client.addr, client.stream.try_clone()?)))
            .collect::<Result<Vec<_>, std::io::Error>>()?
    };

    for (address, mut stream) in clients {
        if let Err(error) = stream.write_all(message.as_bytes()) {
            eprintln!("dropping client during broadcast: {error}");
            remove_client(state, address)?;
        }
    }

    Ok(())
}

pub fn usernames(state: &ServerStateHandle) -> Result<Vec<String>, ServerError> {
    let state = state.lock().map_err(|_| ServerError::ClientStatePoisoned)?;
    Ok(state
        .clients
        .iter()
        .filter_map(|client| client.username.clone())
        .collect())
}

pub fn broadcast_presence(state: &ServerStateHandle) -> Result<(), ServerError> {
    let users = usernames(state)?;
    broadcast(state, &build_users_event(&users), None)
}

pub fn message_history(state: &ServerStateHandle) -> Result<Vec<String>, ServerError> {
    let state = state.lock().map_err(|_| ServerError::ClientStatePoisoned)?;
    Ok(state.history.clone())
}

pub fn record_message(state: &ServerStateHandle, message: &str) -> Result<(), ServerError> {
    let mut state = state.lock().map_err(|_| ServerError::ClientStatePoisoned)?;
    state
        .history
        .push(message.trim_end_matches('\n').to_string());

    if state.history.len() > MESSAGE_HISTORY_LIMIT {
        let overflow = state.history.len() - MESSAGE_HISTORY_LIMIT;
        state.history.drain(0..overflow);
    }

    Ok(())
}
