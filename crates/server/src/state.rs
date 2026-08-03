use std::{
    collections::{HashMap, VecDeque},
    io::Write,
    net::{IpAddr, SocketAddr, TcpStream},
    sync::{Arc, Mutex},
    time::Instant,
};

use crate::ServerError;
use drocsid_protocol::build_users_event;
use tracing::warn;

const MESSAGE_HISTORY_LIMIT: usize = 100;
pub const MAX_CONNECTIONS: usize = 256;
pub const MAX_CONNECTIONS_PER_IP: usize = 4;
const MESSAGE_RATE_PER_SECOND: f64 = 10.0;
const MESSAGE_BURST_SIZE: f64 = 20.0;

pub type ServerStateHandle = Arc<Mutex<ServerState>>;

pub struct ServerState {
    clients: Vec<ClientEntry>,
    history: MessageHistory,
    rate_limits: HashMap<IpAddr, RateLimitState>,
}

struct MessageHistory {
    entries: VecDeque<String>,
    limit: usize,
}

impl MessageHistory {
    fn new(limit: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(limit),
            limit,
        }
    }

    fn record(&mut self, message: &str) {
        self.entries
            .push_back(message.trim_end_matches('\n').to_string());

        if self.entries.len() > self.limit {
            self.entries.pop_front();
        }
    }

    fn snapshot(&self) -> Vec<String> {
        self.entries.iter().cloned().collect()
    }
}

struct RateLimitState {
    tokens: f64,
    last_refill: Instant,
}

pub(crate) type ClientWriter = Arc<Mutex<TcpStream>>;

struct ClientEntry {
    addr: SocketAddr,
    writer: ClientWriter,
    username: Option<String>,
    ready: bool,
    pending_messages: Vec<String>,
}

impl ServerState {
    fn new() -> Self {
        Self {
            clients: Vec::new(),
            history: MessageHistory::new(MESSAGE_HISTORY_LIMIT),
            rate_limits: HashMap::new(),
        }
    }
}

pub fn new_shared_state() -> ServerStateHandle {
    Arc::new(Mutex::new(ServerState::new()))
}

#[cfg(test)]
pub(crate) fn register_client(
    state: &ServerStateHandle,
    stream: &TcpStream,
) -> Result<(), ServerError> {
    register_client_with_status(state, stream, true)
}

pub(crate) fn register_pending_client(
    state: &ServerStateHandle,
    stream: &TcpStream,
) -> Result<(), ServerError> {
    register_client_with_status(state, stream, false)
}

fn register_client_with_status(
    state: &ServerStateHandle,
    stream: &TcpStream,
    ready: bool,
) -> Result<(), ServerError> {
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
        writer: Arc::new(Mutex::new(stream.try_clone()?)),
        username: None,
        ready,
        pending_messages: Vec::new(),
    });
    Ok(())
}

pub(crate) fn mark_client_ready(
    state: &ServerStateHandle,
    target_addr: SocketAddr,
) -> Result<(), ServerError> {
    let writer = {
        let state = state.lock().map_err(|_| ServerError::ClientStatePoisoned)?;
        state
            .clients
            .iter()
            .find(|client| client.addr == target_addr)
            .map(|client| Arc::clone(&client.writer))
            .ok_or(ServerError::ClientStatePoisoned)?
    };

    let mut stream = writer
        .lock()
        .map_err(|_| ServerError::ClientStatePoisoned)?;
    let pending_messages = {
        let mut state = state.lock().map_err(|_| ServerError::ClientStatePoisoned)?;

        let client = state
            .clients
            .iter_mut()
            .find(|client| client.addr == target_addr)
            .ok_or(ServerError::ClientStatePoisoned)?;
        client.ready = true;
        std::mem::take(&mut client.pending_messages)
    };

    for message in pending_messages {
        stream.write_all(message.as_bytes())?;
    }

    Ok(())
}

pub(crate) fn history_snapshot(
    state: &ServerStateHandle,
    target_addr: SocketAddr,
) -> Result<(ClientWriter, Vec<String>), ServerError> {
    let state = state.lock().map_err(|_| ServerError::ClientStatePoisoned)?;

    state
        .clients
        .iter()
        .find(|client| client.addr == target_addr)
        .map(|client| (Arc::clone(&client.writer), state.history.snapshot()))
        .ok_or(ServerError::ClientStatePoisoned)
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

pub(crate) fn broadcast(
    state: &ServerStateHandle,
    message: &str,
    exclude_addr: Option<SocketAddr>,
) -> Result<(), ServerError> {
    let clients = {
        let mut state = state.lock().map_err(|_| ServerError::ClientStatePoisoned)?;
        let mut clients = Vec::new();

        for client in &mut state.clients {
            if exclude_addr.is_some_and(|addr| client.addr == addr) {
                continue;
            }

            if client.ready {
                clients.push((client.addr, Arc::clone(&client.writer)));
            } else {
                client.pending_messages.push(message.to_string());
            }
        }

        clients
    };

    for (address, writer) in clients {
        let mut stream = writer
            .lock()
            .map_err(|_| ServerError::ClientStatePoisoned)?;
        if let Err(error) = stream.write_all(message.as_bytes()) {
            warn!(
                %address,
                error = %error,
                error_kind = ?error.kind(),
                "dropping client during broadcast"
            );
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

pub fn record_message(state: &ServerStateHandle, message: &str) -> Result<(), ServerError> {
    let mut state = state.lock().map_err(|_| ServerError::ClientStatePoisoned)?;
    state.history.record(message);

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::{TcpListener, TcpStream},
        thread,
        time::Duration,
    };

    use super::{
        MessageHistory, broadcast, history_snapshot, mark_client_ready, new_shared_state,
        register_pending_client,
    };

    #[test]
    fn message_history_evicts_oldest_entry_without_shifting_retained_entries() {
        let mut history = MessageHistory::new(2);

        history.record("first\n");
        history.record("second\n");
        history.record("third\n");

        assert_eq!(history.snapshot(), ["second", "third"]);
    }

    #[test]
    fn pending_clients_do_not_receive_live_broadcasts_before_history_replay() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client_stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server_stream, client_addr) = listener.accept().unwrap();
        let state = new_shared_state();

        register_pending_client(&state, &server_stream).unwrap();
        let (writer, _) = history_snapshot(&state, client_addr).unwrap();
        let replay_writer = writer.lock().unwrap();
        let broadcast_state = state.clone();
        let broadcast_thread = thread::spawn(move || {
            broadcast(&broadcast_state, "live during history\n", None).unwrap();
        });
        broadcast_thread.join().unwrap();

        client_stream
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let mut unexpected = [0; 1];
        assert!(client_stream.read(&mut unexpected).is_err());

        (&*replay_writer).write_all(b"history\n").unwrap();
        drop(replay_writer);
        mark_client_ready(&state, client_addr).unwrap();

        client_stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut reader = BufReader::new(client_stream);
        let mut lines = Vec::new();
        for _ in 0..2 {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            lines.push(line);
        }

        assert_eq!(lines, ["history\n", "live during history\n"]);
    }
}
