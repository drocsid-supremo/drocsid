use std::{
    io::{self, ErrorKind},
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use chrono::Local;
use drocsid_protocol::{
    CURRENT_PROTOCOL_VERSION, Frame, FrameType, MAX_CHAT_MESSAGE_BYTES, encode_frame,
    format_chat_message, is_valid_username,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::tcp::{OwnedReadHalf, OwnedWriteHalf},
    sync::mpsc,
    time::timeout,
};
use tracing::{info, warn};

use crate::{
    ConnectionPermit, ServerError,
    state::{
        ClientToken, OUTBOUND_QUEUE_SIZE, ServerStateHandle, allow_message, broadcast_presence,
        mark_client_ready, record_and_broadcast, register_pending_client, remove_client_if_current,
        set_client_username, with_history_replay,
    },
};

const MAX_USERNAME_BYTES: usize = 32;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

pub struct ConnectionHandler {
    state: ServerStateHandle,
    simulated_latency: Duration,
    joined: Arc<AtomicBool>,
}

impl ConnectionHandler {
    pub fn new(state: ServerStateHandle, simulated_latency: Duration) -> Self {
        Self {
            state,
            simulated_latency,
            joined: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) async fn serve(
        self,
        stream: tokio::net::TcpStream,
        sender_addr: SocketAddr,
        _permit: ConnectionPermit,
    ) -> Result<(), ServerError> {
        let _ = stream.set_nodelay(true);
        let (mut reader, writer) = stream.into_split();
        let username = timeout(HANDSHAKE_TIMEOUT, read_handshake_username(&mut reader))
            .await
            .map_err(|_| io::Error::new(ErrorKind::TimedOut, "handshake timed out"))??;
        let (sender, receiver) = mpsc::channel(OUTBOUND_QUEUE_SIZE);
        let (disconnect, mut disconnect_rx) = tokio::sync::watch::channel(false);
        let token = register_pending_client(&self.state, sender_addr, sender, disconnect)?;
        if let Err(error) = set_client_username(&self.state, sender_addr, &token, &username) {
            if let Err(cleanup_error) = remove_client_if_current(&self.state, sender_addr, &token) {
                warn!(
                    %sender_addr,
                    error = %cleanup_error,
                    phase = "disconnect",
                    original_error = %error,
                    "failed to clean up client after registration error"
                );
            }
            return Err(error);
        }
        let writer_state = self.state.clone();
        let writer_token = token.clone();
        let writer_username = username.clone();
        let writer_joined_state = self.joined.clone();
        tokio::spawn(async move {
            outbound_writer(
                writer,
                receiver,
                sender_addr,
                writer_state,
                writer_token,
                writer_username,
                writer_joined_state,
            )
            .await
        });

        let cleanup_username = username.clone();
        let mut joined = false;
        let result = self
            .serve_authenticated(
                &mut reader,
                &mut disconnect_rx,
                sender_addr,
                username,
                &token,
                &mut joined,
            )
            .await;
        if let Err(error) = &result {
            if let Err(cleanup_error) =
                self.finish_session(sender_addr, &cleanup_username, &token, joined)
            {
                warn!(
                    %sender_addr,
                    error = %cleanup_error,
                    phase = "disconnect",
                    original_error = %error,
                    "failed to clean up client after session error"
                );
            }
            warn!(%sender_addr, error = %error, phase = "connection", "connection handler failed");
        }
        result
    }

    async fn serve_authenticated(
        &self,
        reader: &mut OwnedReadHalf,
        disconnect: &mut tokio::sync::watch::Receiver<bool>,
        sender_addr: SocketAddr,
        username: String,
        token: &ClientToken,
        joined: &mut bool,
    ) -> Result<(), ServerError> {
        info!(%sender_addr, username = ?username, phase = "handshake", "handshake completed");
        self.send_message_history(sender_addr, token)?;
        broadcast_presence(&self.state)?;

        let join_message = format!("@{username} has entered the chat. Say hello!\n");
        let join_frame = encode_frame(FrameType::Chat, join_message.trim_end().as_bytes())
            .map_err(|_| ServerError::MessageTooLong)?;
        if record_and_broadcast(&self.state, &join_message, &join_frame, None)? {
            broadcast_presence(&self.state)?;
        }
        *joined = true;
        self.joined.store(true, Ordering::Release);

        loop {
            let frame = tokio::select! {
                _ = disconnect.changed() => break,
                result = read_async_frame(reader) => result?,
            };
            let Some(frame) = frame else { break };
            if frame.kind != FrameType::Chat {
                return Err(ServerError::InvalidFrameType);
            }
            if frame.payload.len() > MAX_CHAT_MESSAGE_BYTES {
                return Err(ServerError::MessageTooLong);
            }
            let content = String::from_utf8(frame.payload).map_err(|_| ServerError::InvalidUtf8)?;
            if content.trim().is_empty() {
                continue;
            }
            allow_message(&self.state, sender_addr)?;
            if !self.simulated_latency.is_zero() {
                tokio::time::sleep(self.simulated_latency).await;
            }
            let message = format_chat_message(
                &username,
                &Local::now().format("%H:%M").to_string(),
                &content,
            );
            let frame = encode_frame(FrameType::Chat, message.as_bytes())
                .map_err(|_| ServerError::MessageTooLong)?;
            if record_and_broadcast(&self.state, &message, &frame, None)? {
                broadcast_presence(&self.state)?;
            }
        }

        self.finish_session(sender_addr, &username, token, *joined)?;
        Ok(())
    }

    fn finish_session(
        &self,
        sender_addr: SocketAddr,
        username: &str,
        token: &ClientToken,
        announce_leave: bool,
    ) -> Result<(), ServerError> {
        if !remove_client_if_current(&self.state, sender_addr, token)? {
            return Ok(());
        }
        if !announce_leave {
            return Ok(());
        }
        let leave = format!("{username} has left the chat\n");
        let frame = encode_frame(FrameType::Chat, leave.trim_end().as_bytes())
            .map_err(|_| ServerError::MessageTooLong)?;
        record_and_broadcast(&self.state, &leave, &frame, None)?;
        broadcast_presence(&self.state)?;
        Ok(())
    }

    fn send_message_history(
        &self,
        address: SocketAddr,
        token: &ClientToken,
    ) -> Result<(), ServerError> {
        with_history_replay(&self.state, address, token, |writer, history| {
            for message in history {
                let frame = encode_frame(FrameType::Chat, message.as_bytes())
                    .map_err(|_| ServerError::MessageTooLong)?;
                writer
                    .try_send(Arc::from(frame))
                    .map_err(|error| match error {
                        tokio::sync::mpsc::error::TrySendError::Full(_) => {
                            ServerError::OutboundQueueFull
                        }
                        tokio::sync::mpsc::error::TrySendError::Closed(_) => {
                            ServerError::OutboundWriterGone
                        }
                    })?;
            }
            mark_client_ready(&self.state, address, token)
        })
    }
}

async fn outbound_writer(
    mut writer: OwnedWriteHalf,
    mut receiver: mpsc::Receiver<crate::delivery::OutboundMessage>,
    address: SocketAddr,
    state: ServerStateHandle,
    token: ClientToken,
    username: String,
    joined: Arc<AtomicBool>,
) {
    while let Some(message) = receiver.recv().await {
        let write_result = timeout(WRITE_TIMEOUT, writer.write_all(&message)).await;
        if let Err(error) = match write_result {
            Ok(result) => result,
            Err(_) => Err(io::Error::new(
                ErrorKind::TimedOut,
                "outbound write timed out",
            )),
        } {
            warn!(%address, error = %error, phase = "outbound_write", "outbound writer stopped");
            match remove_client_if_current(&state, address, &token) {
                Ok(true) => {
                    if joined.load(Ordering::Acquire) {
                        let leave = format!("{username} has left the chat\n");
                        if let Ok(frame) =
                            encode_frame(FrameType::Chat, leave.trim_end().as_bytes())
                        {
                            let _ = record_and_broadcast(&state, &leave, &frame, None);
                        }
                    }
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
}

async fn read_handshake_username<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> Result<String, ServerError> {
    let frame = match read_async_frame(reader).await {
        Ok(Some(frame)) => frame,
        Ok(None) => return Err(ServerError::EmptyHandshakeUsername),
        Err(ServerError::MessageTooLong) => return Err(ServerError::UsernameTooLong),
        Err(error) => return Err(error),
    };
    let Frame {
        kind: FrameType::Handshake,
        payload,
        ..
    } = frame
    else {
        return Err(ServerError::InvalidFrameType);
    };
    let username = String::from_utf8(payload).map_err(|_| ServerError::InvalidUtf8)?;
    if username.len() > MAX_USERNAME_BYTES {
        return Err(ServerError::UsernameTooLong);
    }
    if username.trim().is_empty() {
        return Err(ServerError::EmptyHandshakeUsername);
    }
    if !is_valid_username(&username) {
        return Err(ServerError::UsernameContainsControlCharacters);
    }
    Ok(username.trim().to_string())
}

async fn read_async_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> Result<Option<Frame>, ServerError> {
    let mut bytes = [0u8; drocsid_protocol::FRAME_HEADER_BYTES];
    let first = reader.read(&mut bytes[..1]).await?;
    if first == 0 {
        return Ok(None);
    }
    reader.read_exact(&mut bytes[1..]).await.map_err(|error| {
        if error.kind() == ErrorKind::UnexpectedEof {
            ServerError::TruncatedProtocolFrame
        } else {
            error.into()
        }
    })?;
    let payload_len = u32::from_be_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]) as usize;
    if payload_len > drocsid_protocol::MAX_FRAME_PAYLOAD_BYTES {
        return Err(ServerError::MessageTooLong);
    }

    let version = bytes[0];
    if version != CURRENT_PROTOCOL_VERSION {
        return Err(ServerError::UnsupportedProtocolVersion);
    }
    let kind = FrameType::try_from(bytes[1]).map_err(|_| ServerError::UnknownProtocolFrameType)?;
    let mut payload = vec![0; payload_len];
    reader.read_exact(&mut payload).await.map_err(|error| {
        if error.kind() == ErrorKind::UnexpectedEof {
            ServerError::TruncatedProtocolFrame
        } else {
            error.into()
        }
    })?;
    Ok(Some(Frame {
        version,
        kind,
        payload,
    }))
}

#[cfg(test)]
mod tests {
    use super::{ConnectionHandler, read_async_frame, read_handshake_username};
    use crate::ServerError;
    use crate::{
        ConnectionAdmission,
        state::{new_shared_state, register_client, set_client_username, usernames},
    };
    use drocsid_protocol::{FrameType, encode_frame};
    use std::{
        net::{IpAddr, SocketAddr},
        time::Duration,
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt, duplex},
        net::{TcpListener, TcpStream},
    };

    #[tokio::test]
    async fn accepts_a_valid_handshake_without_allocating_a_session_thread() {
        let (mut client, mut server) = duplex(128);
        client
            .write_all(&encode_frame(FrameType::Handshake, b"alice").unwrap())
            .await
            .unwrap();

        assert_eq!(read_handshake_username(&mut server).await.unwrap(), "alice");
    }

    #[tokio::test]
    async fn treats_clean_eof_as_a_disconnect() {
        let (client, mut server) = duplex(128);
        drop(client);
        assert!(read_async_frame(&mut server).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn rejects_an_oversized_handshake_as_a_username_error() {
        let (mut client, mut server) = duplex(64 * 1024);
        client
            .write_all(&encode_frame(FrameType::Handshake, &[b'x'; 33]).unwrap())
            .await
            .unwrap();

        assert!(matches!(
            read_handshake_username(&mut server).await,
            Err(ServerError::UsernameTooLong)
        ));
    }

    #[test]
    fn session_cleanup_is_idempotent_and_updates_presence() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let client = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, address) = listener.accept().unwrap();
        let state = new_shared_state();
        let token = register_client(&state, &server).unwrap();
        set_client_username(&state, address, &token, "alice").unwrap();
        let handler = ConnectionHandler::new(state.clone(), Duration::ZERO);

        handler
            .finish_session(address, "alice", &token, true)
            .unwrap();
        handler
            .finish_session(address, "alice", &token, true)
            .unwrap();

        assert!(usernames(&state).unwrap().is_empty());
        drop(client);
    }

    #[tokio::test]
    async fn authenticated_tcp_sessions_use_the_async_path() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let state = new_shared_state();
        let client_task = tokio::spawn(async move {
            let mut client = TcpStream::connect(address).await.unwrap();
            client
                .write_all(&encode_frame(FrameType::Handshake, b"alice").unwrap())
                .await
                .unwrap();
            let mut frame = [0; 6];
            client.read_exact(&mut frame).await.unwrap();
            client.shutdown().await.unwrap();
        });
        let (server, peer) = listener.accept().await.unwrap();
        let permit = ConnectionAdmission::new().try_acquire(peer).unwrap();
        let handler = ConnectionHandler::new(state, Duration::ZERO);
        handler.serve(server, peer, permit).await.unwrap();
        client_task.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn maintains_more_persistent_sessions_than_runtime_workers() {
        const SESSION_COUNT: usize = 32;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let state = new_shared_state();
        let admission = ConnectionAdmission::new();
        let mut clients = Vec::with_capacity(SESSION_COUNT);
        let mut handlers = Vec::with_capacity(SESSION_COUNT);

        for index in 0..SESSION_COUNT {
            let mut client = TcpStream::connect(address).await.unwrap();
            client
                .write_all(
                    &encode_frame(FrameType::Handshake, format!("user-{index}").as_bytes())
                        .unwrap(),
                )
                .await
                .unwrap();

            let (server, _) = listener.accept().await.unwrap();
            let peer = SocketAddr::new(
                IpAddr::from([192, 0, 2, (index + 1) as u8]),
                10_000 + index as u16,
            );
            let permit = admission.try_acquire(peer).unwrap();
            let handler = ConnectionHandler::new(state.clone(), Duration::ZERO);
            handlers.push(tokio::spawn(handler.serve(server, peer, permit)));
            clients.push(client);
        }

        for client in &mut clients {
            let mut saw_presence = false;
            for _ in 0..SESSION_COUNT {
                let frame = read_async_frame(client).await.unwrap().unwrap();
                if frame.kind == FrameType::Presence {
                    saw_presence = true;
                    break;
                }
            }
            assert!(saw_presence, "session did not receive a presence frame");
        }

        clients[0]
            .write_all(&encode_frame(FrameType::Chat, b"load-test-message").unwrap())
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let frame = read_async_frame(&mut clients[1]).await.unwrap().unwrap();
                if frame.kind == FrameType::Chat
                    && String::from_utf8(frame.payload)
                        .unwrap()
                        .contains("load-test-message")
                {
                    break;
                }
            }
        })
        .await
        .expect("established clients were starved by the concurrent session load");

        for client in &mut clients {
            client.shutdown().await.unwrap();
        }
        for handler in handlers {
            handler.await.unwrap().unwrap();
        }
    }
}
