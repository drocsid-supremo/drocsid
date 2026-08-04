use std::{
    collections::{HashMap, VecDeque},
    net::{IpAddr, Shutdown, SocketAddr, TcpStream},
    sync::{
        Arc, Mutex,
        mpsc::{self, SyncSender, TrySendError},
    },
    time::{Duration, Instant},
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
const RATE_LIMIT_IDLE_EXPIRY: Duration = Duration::from_secs(60);

pub type ServerStateHandle = Arc<ServerState>;

pub struct ServerState {
    clients: Mutex<ClientRegistry>,
    history: Mutex<MessageHistory>,
    rate_limits: Mutex<HashMap<IpAddr, RateLimitState>>,
    replay_barrier: Mutex<()>,
}

struct ClientRegistry {
    by_addr: HashMap<SocketAddr, Vec<ClientEntry>>,
    per_ip: HashMap<IpAddr, usize>,
    total: usize,
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
            clients: Mutex::new(ClientRegistry {
                by_addr: HashMap::new(),
                per_ip: HashMap::new(),
                total: 0,
            }),
            history: Mutex::new(MessageHistory::new(MESSAGE_HISTORY_LIMIT)),
            rate_limits: Mutex::new(HashMap::new()),
            replay_barrier: Mutex::new(()),
        }
    }
}

pub fn new_shared_state() -> ServerStateHandle {
    Arc::new(ServerState::new())
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
    let address = stream.peer_addr()?;
    let mut clients = state
        .clients
        .lock()
        .map_err(|_| ServerError::ClientStatePoisoned)?;

    if clients.total >= MAX_CONNECTIONS
        || clients.per_ip.get(&address.ip()).copied().unwrap_or(0) >= MAX_CONNECTIONS_PER_IP
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

    clients
        .by_addr
        .entry(address)
        .or_default()
        .push(ClientEntry {
            addr: address,
            token: token.clone(),
            writer,
            username: None,
            ready,
            pending_messages: Vec::new(),
        });
    clients.total += 1;
    *clients.per_ip.entry(address.ip()).or_default() += 1;
    Ok(token)
}

pub(crate) fn mark_client_ready(
    state: &ServerStateHandle,
    target_addr: SocketAddr,
    target_token: &ClientToken,
) -> Result<(), ServerError> {
    let mut clients = state
        .clients
        .lock()
        .map_err(|_| ServerError::ClientStatePoisoned)?;
    let client = clients
        .by_addr
        .get_mut(&target_addr)
        .and_then(|entries| {
            entries
                .iter_mut()
                .find(|client| client.token.matches(target_token))
        })
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

#[cfg(test)]
pub(crate) fn history_snapshot(
    state: &ServerStateHandle,
    target_addr: SocketAddr,
    target_token: &ClientToken,
) -> Result<(ClientWriter, Vec<String>), ServerError> {
    let _replay_barrier = state
        .replay_barrier
        .lock()
        .map_err(|_| ServerError::ClientStatePoisoned)?;
    history_snapshot_unlocked(state, target_addr, target_token)
}

fn history_snapshot_unlocked(
    state: &ServerStateHandle,
    target_addr: SocketAddr,
    target_token: &ClientToken,
) -> Result<(ClientWriter, Vec<String>), ServerError> {
    let clients = state
        .clients
        .lock()
        .map_err(|_| ServerError::ClientStatePoisoned)?;

    let writer = clients
        .by_addr
        .get(&target_addr)
        .and_then(|entries| {
            entries
                .iter()
                .find(|client| client.token.matches(target_token))
        })
        .map(|client| client.writer.clone())
        .ok_or(ServerError::UnknownClient)?;
    drop(clients);
    let history = state
        .history
        .lock()
        .map_err(|_| ServerError::ClientStatePoisoned)?
        .snapshot();
    Ok((writer, history))
}

pub(crate) fn with_history_replay<F>(
    state: &ServerStateHandle,
    target_addr: SocketAddr,
    target_token: &ClientToken,
    replay: F,
) -> Result<(), ServerError>
where
    F: FnOnce(ClientWriter, Vec<String>) -> Result<(), ServerError>,
{
    let _replay_barrier = state
        .replay_barrier
        .lock()
        .map_err(|_| ServerError::ClientStatePoisoned)?;
    let (writer, history) = history_snapshot_unlocked(state, target_addr, target_token)?;
    replay(writer, history)
}

pub(crate) fn remove_client_if_current(
    state: &ServerStateHandle,
    target_addr: SocketAddr,
    target_token: &ClientToken,
) -> Result<bool, ServerError> {
    let removed = {
        let mut clients = state
            .clients
            .lock()
            .map_err(|_| ServerError::ClientStatePoisoned)?;
        let (removed, empty) = {
            let Some(entries) = clients.by_addr.get_mut(&target_addr) else {
                return Ok(false);
            };
            let before = entries.len();
            entries.retain(|client| !client.token.matches(target_token));
            (entries.len() != before, entries.is_empty())
        };
        if removed {
            clients.total -= 1;
            let ip_count = clients.per_ip.get_mut(&target_addr.ip()).unwrap();
            *ip_count -= 1;
            if *ip_count == 0 {
                clients.per_ip.remove(&target_addr.ip());
            }
        }
        if empty {
            clients.by_addr.remove(&target_addr);
        }
        removed
    };

    Ok(removed)
}

pub fn allow_message(
    state: &ServerStateHandle,
    sender_addr: SocketAddr,
) -> Result<(), ServerError> {
    let mut rate_limits = state
        .rate_limits
        .lock()
        .map_err(|_| ServerError::ClientStatePoisoned)?;
    let now = Instant::now();
    rate_limits
        .retain(|_, limiter| now.duration_since(limiter.last_refill) < RATE_LIMIT_IDLE_EXPIRY);
    let limiter = rate_limits
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
    let mut clients = state
        .clients
        .lock()
        .map_err(|_| ServerError::ClientStatePoisoned)?;

    if let Some(client) = clients.by_addr.get_mut(&target_addr).and_then(|entries| {
        entries
            .iter_mut()
            .find(|client| client.token.matches(target_token))
    }) {
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
        let mut clients = state
            .clients
            .lock()
            .map_err(|_| ServerError::ClientStatePoisoned)?;

        for client in clients.by_addr.values_mut().flatten() {
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

pub(crate) fn record_and_broadcast(
    state: &ServerStateHandle,
    message: &str,
    frame: &[u8],
    exclude_addr: Option<SocketAddr>,
) -> Result<bool, ServerError> {
    let _replay_barrier = state
        .replay_barrier
        .lock()
        .map_err(|_| ServerError::ClientStatePoisoned)?;
    state
        .history
        .lock()
        .map_err(|_| ServerError::ClientStatePoisoned)?
        .record(message);
    broadcast(state, frame, exclude_addr)
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
    let clients = state
        .clients
        .lock()
        .map_err(|_| ServerError::ClientStatePoisoned)?;
    Ok(clients
        .by_addr
        .values()
        .flatten()
        .filter_map(|client| client.username.clone())
        .collect())
}

pub fn broadcast_presence(state: &ServerStateHandle) -> Result<(), ServerError> {
    loop {
        let users = usernames(state)?;
        let frame = match encode_presence(&users) {
            Ok(frame) => frame,
            Err(error) => {
                warn!(
                    error = ?error,
                    error_kind = "presence_encoding",
                    phase = "presence",
                    "failed to encode presence frame"
                );
                return Err(ServerError::PresenceEncodingFailed);
            }
        };
        if !broadcast(state, &frame, None)? {
            return Ok(());
        }
    }
}

#[cfg(test)]
pub(crate) fn record_message(state: &ServerStateHandle, message: &str) -> Result<(), ServerError> {
    let _replay_barrier = state
        .replay_barrier
        .lock()
        .map_err(|_| ServerError::ClientStatePoisoned)?;
    state
        .history
        .lock()
        .map_err(|_| ServerError::ClientStatePoisoned)?
        .record(message);

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead, BufReader, Read},
        net::{IpAddr, SocketAddr, TcpListener, TcpStream},
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };

    use super::{
        ClientEntry, ClientToken, MAX_CONNECTIONS, MAX_CONNECTIONS_PER_IP, MessageHistory,
        OUTBOUND_QUEUE_SIZE, RATE_LIMIT_IDLE_EXPIRY, RateLimitState, allow_message, broadcast,
        history_snapshot, mark_client_ready, new_shared_state, record_and_broadcast,
        record_message, register_client, register_pending_client, remove_client_if_current,
        with_history_replay,
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
            let mut clients = state.clients.lock().unwrap();
            clients
                .by_addr
                .entry(address)
                .or_default()
                .push(ClientEntry {
                    addr: address,
                    token: ClientToken::new(),
                    writer: writer.clone(),
                    username: Some("slow-client".to_string()),
                    ready: true,
                    pending_messages: Vec::new(),
                });
            clients.total += 1;
            *clients.per_ip.entry(address.ip()).or_default() += 1;
        }

        for _ in 0..OUTBOUND_QUEUE_SIZE {
            writer.try_send(b"already queued\n".to_vec()).unwrap();
        }

        broadcast(&state, b"new message\n", None).unwrap();

        assert!(state.clients.lock().unwrap().by_addr.is_empty());
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
            let mut clients = state.clients.lock().unwrap();
            clients
                .by_addr
                .entry(address)
                .or_default()
                .push(ClientEntry {
                    addr: address,
                    token: old_token.clone(),
                    writer: old_writer,
                    username: Some("replacement".to_string()),
                    ready: true,
                    pending_messages: Vec::new(),
                });
            clients
                .by_addr
                .entry(address)
                .or_default()
                .push(ClientEntry {
                    addr: address,
                    token: new_token,
                    writer: new_writer,
                    username: Some("current".to_string()),
                    ready: true,
                    pending_messages: Vec::new(),
                });
            clients.total = 2;
            clients.per_ip.insert(address.ip(), 2);
        }

        remove_client_if_current(&state, address, &old_token).unwrap();

        let clients = state.clients.lock().unwrap();
        let entries = clients.by_addr.get(&address).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].username.as_deref(), Some("current"));
    }

    #[test]
    fn per_ip_limit_is_released_when_a_client_is_removed() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let state = new_shared_state();
        let mut client_streams = Vec::new();
        let mut server_streams = Vec::new();
        let mut addresses = Vec::new();
        let mut tokens = Vec::new();

        for _ in 0..MAX_CONNECTIONS_PER_IP {
            let client_stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
            let (server_stream, address) = listener.accept().unwrap();
            let token = register_client(&state, &server_stream).unwrap();
            client_streams.push(client_stream);
            server_streams.push(server_stream);
            addresses.push(address);
            tokens.push(token);
        }

        let rejected_client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (rejected_server, _) = listener.accept().unwrap();
        assert_eq!(
            register_client(&state, &rejected_server)
                .unwrap_err()
                .to_string(),
            "connection limit reached"
        );

        remove_client_if_current(&state, addresses[0], &tokens[0]).unwrap();

        let replacement_client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (replacement_server, _) = listener.accept().unwrap();
        register_client(&state, &replacement_server).unwrap();

        drop(replacement_client);
        drop(rejected_client);
        drop(client_streams);
        drop(server_streams);
        drop(replacement_server);
    }

    #[test]
    fn removing_the_last_client_keeps_its_rate_limit_for_reconnects() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client_stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server_stream, client_addr) = listener.accept().unwrap();
        let state = new_shared_state();
        let token = register_client(&state, &server_stream).unwrap();

        let mut accepted_messages = 0;
        loop {
            if allow_message(&state, client_addr).is_err() {
                break;
            }
            accepted_messages += 1;
            assert!(accepted_messages <= 1_000);
        }
        assert!(accepted_messages >= 20);

        remove_client_if_current(&state, client_addr, &token).unwrap();

        let reconnect_client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (reconnect_server, reconnect_addr) = listener.accept().unwrap();
        let reconnect_token = register_client(&state, &reconnect_server).unwrap();
        assert!(allow_message(&state, reconnect_addr).is_err());

        remove_client_if_current(&state, reconnect_addr, &reconnect_token).unwrap();
        drop(reconnect_client);
        drop(reconnect_server);
        drop(client_stream);
        drop(server_stream);
    }

    #[test]
    fn rate_limit_cleanup_reclaims_stale_entries_from_other_ips() {
        let state = new_shared_state();
        let stale_ips: [IpAddr; 2] = ["192.0.2.1".parse().unwrap(), "192.0.2.2".parse().unwrap()];
        let active_ip: IpAddr = "192.0.2.3".parse().unwrap();
        let stale_time = Instant::now() - RATE_LIMIT_IDLE_EXPIRY - Duration::from_secs(1);

        {
            let mut rate_limits = state.rate_limits.lock().unwrap();
            for ip in stale_ips {
                rate_limits.insert(
                    ip,
                    RateLimitState {
                        tokens: 0.0,
                        last_refill: stale_time,
                    },
                );
            }
            rate_limits.insert(
                active_ip,
                RateLimitState {
                    tokens: 1.0,
                    last_refill: Instant::now(),
                },
            );
        }

        allow_message(&state, SocketAddr::new(active_ip, 1234)).unwrap();

        let rate_limits = state.rate_limits.lock().unwrap();
        assert!(!rate_limits.contains_key(&stale_ips[0]));
        assert!(!rate_limits.contains_key(&stale_ips[1]));
        assert!(rate_limits.contains_key(&active_ip));
    }

    #[test]
    fn history_replay_completes_before_live_broadcasts_are_delivered() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client_stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server_stream, client_addr) = listener.accept().unwrap();
        let state = new_shared_state();
        let token = register_pending_client(&state, &server_stream).unwrap();

        with_history_replay(&state, client_addr, &token, |_writer, history| {
            assert!(history.is_empty());
            mark_client_ready(&state, client_addr, &token)
        })
        .unwrap();
        record_and_broadcast(&state, "live\n", b"live\n", None).unwrap();

        client_stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut reader = BufReader::new(client_stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line, "live\n");
    }

    #[test]
    fn global_connection_limit_uses_the_indexed_total() {
        let state = new_shared_state();
        let (writer, _receiver) = mpsc::sync_channel(OUTBOUND_QUEUE_SIZE);

        {
            let mut clients = state.clients.lock().unwrap();
            for port in 0..MAX_CONNECTIONS {
                let address: SocketAddr = format!("127.0.0.1:{}", 10_000 + port).parse().unwrap();
                clients
                    .by_addr
                    .entry(address)
                    .or_default()
                    .push(ClientEntry {
                        addr: address,
                        token: ClientToken::new(),
                        writer: writer.clone(),
                        username: None,
                        ready: true,
                        pending_messages: Vec::new(),
                    });
                clients.total += 1;
                *clients.per_ip.entry(address.ip()).or_default() += 1;
            }

            assert_eq!(clients.total, MAX_CONNECTIONS);
        }

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client_stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server_stream, _) = listener.accept().unwrap();

        assert_eq!(
            register_client(&state, &server_stream)
                .unwrap_err()
                .to_string(),
            "connection limit reached"
        );
        drop(client_stream);
        drop(server_stream);
    }
}
