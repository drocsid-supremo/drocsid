use std::{
    collections::{HashMap, VecDeque},
    net::{IpAddr, Shutdown, SocketAddr, TcpStream},
    sync::{
        Arc, Mutex,
        mpsc::{self, SyncSender, TrySendError},
    },
    time::Instant,
};

use crate::ServerError;
use drocsid_protocol::encode_presence;
use tracing::warn;

const MESSAGE_HISTORY_LIMIT: usize = 100;
pub const MAX_CONNECTIONS: usize = 256;
pub const MAX_CONNECTIONS_PER_IP: usize = 4;
const PENDING_MESSAGES_LIMIT: usize = 128;
const OUTBOUND_QUEUE_SIZE: usize = MESSAGE_HISTORY_LIMIT + PENDING_MESSAGES_LIMIT;
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

pub(crate) type ClientWriter = SyncSender<Vec<u8>>;
#[derive(Clone, Debug)]
pub(crate) struct ClientToken(Arc<()>);

impl ClientToken {
    fn new() -> Self {
        Self(Arc::new(()))
    }

    fn matches(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

struct ClientEntry {
    addr: SocketAddr,
    token: ClientToken,
    writer: ClientWriter,
    username: Option<String>,
    ready: bool,
    pending_messages: Vec<Vec<u8>>,
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
) -> Result<ClientToken, ServerError> {
    register_client_with_status(state, stream, true)
}

pub(crate) fn register_pending_client(
    state: &ServerStateHandle,
    stream: &TcpStream,
) -> Result<ClientToken, ServerError> {
    register_client_with_status(state, stream, false)
}

fn register_client_with_status(
    state: &ServerStateHandle,
    stream: &TcpStream,
    ready: bool,
) -> Result<ClientToken, ServerError> {
    let writer_state = Arc::clone(state);
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

    let (writer, receiver) = mpsc::sync_channel(OUTBOUND_QUEUE_SIZE);
    let stream = stream.try_clone()?;
    let token = ClientToken::new();
    let writer_token = token.clone();
    std::thread::spawn(move || {
        outbound_writer(stream, receiver, address, writer_state, writer_token)
    });

    state.clients.push(ClientEntry {
        addr: address,
        token: token.clone(),
        writer,
        username: None,
        ready,
        pending_messages: Vec::new(),
    });
    Ok(token)
}

pub(crate) fn mark_client_ready(
    state: &ServerStateHandle,
    target_addr: SocketAddr,
    target_token: &ClientToken,
) -> Result<(), ServerError> {
    let mut state = state.lock().map_err(|_| ServerError::ClientStatePoisoned)?;
    let client = state
        .clients
        .iter_mut()
        .find(|client| client.addr == target_addr && client.token.matches(target_token))
        .ok_or(ServerError::UnknownClient)?;
    let writer = client.writer.clone();
    let pending_messages = std::mem::take(&mut client.pending_messages);

    let mut remaining = pending_messages.into_iter();
    for message in remaining.by_ref() {
        match writer.try_send(message) {
            Ok(()) => {}
            Err(TrySendError::Full(message)) => {
                client.pending_messages.push(message);
                client.pending_messages.extend(remaining);
                return Err(ServerError::OutboundQueueFull);
            }
            Err(TrySendError::Disconnected(message)) => {
                client.pending_messages.push(message);
                client.pending_messages.extend(remaining);
                return Err(ServerError::OutboundWriterGone);
            }
        }
    }

    client.ready = true;
    Ok(())
}

pub(crate) fn history_snapshot(
    state: &ServerStateHandle,
    target_addr: SocketAddr,
    target_token: &ClientToken,
) -> Result<(ClientWriter, Vec<String>), ServerError> {
    let state = state.lock().map_err(|_| ServerError::ClientStatePoisoned)?;

    state
        .clients
        .iter()
        .find(|client| client.addr == target_addr && client.token.matches(target_token))
        .map(|client| (client.writer.clone(), state.history.snapshot()))
        .ok_or(ServerError::UnknownClient)
}

pub(crate) fn remove_client_if_current(
    state: &ServerStateHandle,
    target_addr: SocketAddr,
    target_token: &ClientToken,
) -> Result<bool, ServerError> {
    let mut state = state.lock().map_err(|_| ServerError::ClientStatePoisoned)?;
    let before = state.clients.len();
    state
        .clients
        .retain(|client| !(client.addr == target_addr && client.token.matches(target_token)));

    if !state
        .clients
        .iter()
        .any(|client| client.addr.ip() == target_addr.ip())
    {
        state.rate_limits.remove(&target_addr.ip());
    }

    Ok(state.clients.len() != before)
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
    target_token: &ClientToken,
    username: &str,
) -> Result<(), ServerError> {
    let mut state = state.lock().map_err(|_| ServerError::ClientStatePoisoned)?;

    if let Some(client) = state
        .clients
        .iter_mut()
        .find(|client| client.addr == target_addr && client.token.matches(target_token))
    {
        client.username = Some(username.to_string());
    } else {
        return Err(ServerError::UnknownClient);
    }

    Ok(())
}

pub(crate) fn broadcast(
    state: &ServerStateHandle,
    message: &[u8],
    exclude_addr: Option<SocketAddr>,
) -> Result<bool, ServerError> {
    let mut doomed = Vec::new();
    {
        let mut state = state.lock().map_err(|_| ServerError::ClientStatePoisoned)?;

        for client in &mut state.clients {
            if exclude_addr.is_some_and(|addr| client.addr == addr) {
                continue;
            }

            if client.ready {
                match client.writer.try_send(message.to_vec()) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_)) => {
                        doomed.push((client.addr, client.token.clone(), "outbound queue is full"));
                    }
                    Err(TrySendError::Disconnected(_)) => {
                        doomed.push((client.addr, client.token.clone(), "outbound writer is gone"));
                    }
                }
            } else if client.pending_messages.len() >= PENDING_MESSAGES_LIMIT {
                doomed.push((client.addr, client.token.clone(), "pending queue is full"));
            } else {
                client.pending_messages.push(message.to_vec());
            }
        }
    }

    for (address, token, reason) in &doomed {
        warn!(%address, reason, phase = "broadcast", "dropping client");
        remove_client_if_current(state, *address, token)?;
    }

    Ok(!doomed.is_empty())
}

fn outbound_writer(
    mut stream: TcpStream,
    receiver: mpsc::Receiver<Vec<u8>>,
    address: SocketAddr,
    state: ServerStateHandle,
    token: ClientToken,
) {
    for message in receiver {
        if let Err(error) = std::io::Write::write_all(&mut stream, &message) {
            warn!(
                %address,
                error = %error,
                error_kind = ?error.kind(),
                phase = "outbound_write",
                "outbound writer stopped"
            );
            match remove_client_if_current(&state, address, &token) {
                Ok(true) => {
                    if let Err(presence_error) = broadcast_presence(&state) {
                        warn!(
                            %address,
                            error = %presence_error,
                            phase = "presence",
                            "failed to broadcast client removal"
                        );
                    }
                }
                Ok(false) => {}
                Err(cleanup_error) => {
                    warn!(
                        %address,
                        error = %cleanup_error,
                        phase = "disconnect",
                        "failed to remove client after outbound writer stopped"
                    );
                }
            }
            break;
        }
    }

    let _ = stream.shutdown(Shutdown::Both);
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
    loop {
        let users = usernames(state)?;
        let frame = encode_presence(&users).map_err(|_| ServerError::OutboundQueueFull)?;
        if !broadcast(state, &frame, None)? {
            return Ok(());
        }
    }
}

pub fn record_message(state: &ServerStateHandle, message: &str) -> Result<(), ServerError> {
    let mut state = state.lock().map_err(|_| ServerError::ClientStatePoisoned)?;
    state.history.record(message);

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead, BufReader, Read},
        net::{SocketAddr, TcpListener, TcpStream},
        sync::mpsc,
        thread,
        time::Duration,
    };

    use super::{
        ClientEntry, ClientToken, MessageHistory, OUTBOUND_QUEUE_SIZE, broadcast, history_snapshot,
        mark_client_ready, new_shared_state, record_message, register_client,
        register_pending_client, remove_client_if_current,
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
    fn server_history_snapshot_returns_trimmed_retained_messages_in_order() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let _client_stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server_stream, client_addr) = listener.accept().unwrap();
        let state = new_shared_state();

        let token = register_client(&state, &server_stream).unwrap();
        for index in 0..=100 {
            record_message(&state, &format!("message {index}\n")).unwrap();
        }

        let (_, history) = history_snapshot(&state, client_addr, &token).unwrap();

        assert_eq!(history.len(), 100);
        assert_eq!(history.first().unwrap(), "message 1");
        assert_eq!(history.last().unwrap(), "message 100");
    }

    #[test]
    fn pending_clients_do_not_receive_live_broadcasts_before_history_replay() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client_stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server_stream, client_addr) = listener.accept().unwrap();
        let state = new_shared_state();

        let token = register_pending_client(&state, &server_stream).unwrap();
        let (writer, _) = history_snapshot(&state, client_addr, &token).unwrap();
        let broadcast_state = state.clone();
        let broadcast_thread = thread::spawn(move || {
            broadcast(&broadcast_state, b"live during history\n", None).unwrap();
        });
        broadcast_thread.join().unwrap();

        client_stream
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let mut unexpected = [0; 1];
        assert!(client_stream.read(&mut unexpected).is_err());

        writer.try_send(b"history\n".to_vec()).unwrap();
        mark_client_ready(&state, client_addr, &token).unwrap();

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

    #[test]
    fn broadcast_drops_a_client_when_its_outbound_queue_is_full() {
        let state = new_shared_state();
        let address: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let (writer, receiver) = mpsc::sync_channel(OUTBOUND_QUEUE_SIZE);

        {
            let mut state_guard = state.lock().unwrap();
            state_guard.clients.push(ClientEntry {
                addr: address,
                token: ClientToken::new(),
                writer: writer.clone(),
                username: Some("slow-client".to_string()),
                ready: true,
                pending_messages: Vec::new(),
            });
        }

        for _ in 0..OUTBOUND_QUEUE_SIZE {
            writer.try_send(b"already queued\n".to_vec()).unwrap();
        }

        broadcast(&state, b"new message\n", None).unwrap();

        assert!(state.lock().unwrap().clients.is_empty());
        drop(receiver);
    }

    #[test]
    fn stale_writer_cleanup_does_not_remove_a_replacement_client() {
        let state = new_shared_state();
        let address: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let (old_writer, _old_receiver) = mpsc::sync_channel(1);
        let (new_writer, _new_receiver) = mpsc::sync_channel(1);
        let old_token = ClientToken::new();
        let new_token = ClientToken::new();

        {
            let mut state_guard = state.lock().unwrap();
            state_guard.clients.push(ClientEntry {
                addr: address,
                token: old_token.clone(),
                writer: old_writer,
                username: Some("replacement".to_string()),
                ready: true,
                pending_messages: Vec::new(),
            });
            state_guard.clients.push(ClientEntry {
                addr: address,
                token: new_token,
                writer: new_writer,
                username: Some("current".to_string()),
                ready: true,
                pending_messages: Vec::new(),
            });
        }

        remove_client_if_current(&state, address, &old_token).unwrap();

        let state_guard = state.lock().unwrap();
        assert_eq!(state_guard.clients.len(), 1);
        assert_eq!(state_guard.clients[0].username.as_deref(), Some("current"));
    }
}
