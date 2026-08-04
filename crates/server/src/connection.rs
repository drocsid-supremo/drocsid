use std::{
    io::{ErrorKind, Read},
    net::{SocketAddr, TcpStream},
    thread,
    time::Duration,
};

use crate::ServerError;
use chrono::Local;
use drocsid_protocol::{
    Frame, FrameType, MAX_CHAT_MESSAGE_BYTES, encode_frame, format_chat_message, is_valid_username,
    read_frame,
};
use tracing::{debug, info, warn};

use super::state::{
    ClientToken, ServerStateHandle, allow_message, broadcast, broadcast_presence, history_snapshot,
    mark_client_ready, record_message, register_pending_client, set_client_username,
};

const MAX_USERNAME_BYTES: usize = 32;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

pub struct ConnectionHandler {
    state: ServerStateHandle,
    simulated_latency: Duration,
}

impl ConnectionHandler {
    pub fn new(state: ServerStateHandle, simulated_latency: Duration) -> Self {
        Self {
            state,
            simulated_latency,
        }
    }

    pub(crate) fn authenticate(
        &self,
        stream: &mut TcpStream,
        sender_addr: SocketAddr,
    ) -> Result<(String, ClientToken), ServerError> {
        self.authenticate_with_timeout(stream, sender_addr, HANDSHAKE_TIMEOUT)
    }

    fn authenticate_with_timeout(
        &self,
        stream: &mut TcpStream,
        sender_addr: SocketAddr,
        handshake_timeout: Duration,
    ) -> Result<(String, ClientToken), ServerError> {
        stream.set_read_timeout(Some(handshake_timeout))?;
        stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
        stream.set_nodelay(true)?;
        let mut handshake_reader = FrameReader::new(stream.try_clone()?);
        let username = match self.read_handshake_username(&mut handshake_reader) {
            Ok(username) => username,
            Err(error) => {
                warn!(
                    %sender_addr,
                    error = %error,
                    error_kind = ?error_kind(&error),
                    phase = "handshake",
                    "handshake failed"
                );
                return Err(error);
            }
        };

        stream.set_read_timeout(None)?;
        let token = register_pending_client(&self.state, stream)?;
        set_client_username(&self.state, sender_addr, &token, &username)?;
        Ok((username, token))
    }

    pub(crate) fn serve_authenticated(
        &self,
        stream: TcpStream,
        sender_addr: SocketAddr,
        username: String,
        token: ClientToken,
    ) -> Result<(), ServerError> {
        let result = self.serve_authenticated_inner(stream, sender_addr, username, &token);

        if let Err(error) = &result
            && let Err(cleanup_error) =
                super::state::remove_client_if_current(&self.state, sender_addr, &token)
        {
            warn!(
                error = %cleanup_error,
                error_kind = ?error_kind(&cleanup_error),
                phase = "disconnect",
                original_error = %error,
                "failed to clean up client after session error"
            );
        }

        result
    }

    fn serve_authenticated_inner(
        &self,
        stream: TcpStream,
        sender_addr: SocketAddr,
        username: String,
        token: &ClientToken,
    ) -> Result<(), ServerError> {
        let mut reader = FrameReader::new(stream.try_clone()?);
        info!(username = ?username, phase = "handshake", "handshake completed");

        if let Err(error) = self.send_message_history(sender_addr, token) {
            super::state::remove_client_if_current(&self.state, sender_addr, token)?;
            warn!(
                error = %error,
                error_kind = ?error_kind(&error),
                phase = "history",
                "failed to send message history"
            );
            return match error {
                ServerError::Io(error) if is_disconnect_error(&error) => Ok(()),
                error => Err(error),
            };
        }
        mark_client_ready(&self.state, sender_addr, token)?;
        broadcast_presence(&self.state)?;

        let join_message = format!("@{} has entered the chat. Say hello!\n", username);
        info!(username = ?username, phase = "lifecycle", "client joined chat");
        record_message(&self.state, &join_message)?;
        let join_frame = encode_frame(FrameType::Chat, join_message.trim_end().as_bytes())
            .map_err(|_| ServerError::MessageTooLong)?;
        match broadcast(&self.state, &join_frame, None) {
            Ok(true) => broadcast_presence(&self.state)?,
            Ok(false) => {}
            Err(error) => {
                warn!(
                    error = %error,
                    error_kind = ?error_kind(&error),
                    phase = "broadcast",
                    event = "join",
                    "failed to broadcast join event"
                );
                return Err(error);
            }
        }

        let result = self.read_messages(&mut reader, sender_addr, &username);
        if let Err(error) = &result {
            warn!(
                %sender_addr,
                username = ?username,
                error = %error,
                error_kind = ?error_kind(error),
                phase = "message_read",
                "message loop ended with an error"
            );
        }

        if let Err(error) = self.disconnect_client(sender_addr, &username, token) {
            warn!(
                error = %error,
                error_kind = ?error_kind(&error),
                phase = "disconnect",
                "failed to disconnect client cleanly"
            );
            return Err(error);
        }

        result
    }

    fn read_handshake_username<R: Read>(
        &self,
        reader: &mut FrameReader<R>,
    ) -> Result<String, ServerError> {
        let frame = match reader.read_protocol_frame() {
            Ok(Some(frame)) => frame,
            Ok(None) => return Err(ServerError::EmptyHandshakeUsername),
            Err(drocsid_protocol::FrameError::TooLarge(_)) => {
                return Err(ServerError::UsernameTooLong);
            }
            Err(drocsid_protocol::FrameError::Io(kind)) => {
                return Err(std::io::Error::from(kind).into());
            }
            Err(drocsid_protocol::FrameError::UnsupportedVersion(_)) => {
                return Err(ServerError::UnsupportedProtocolVersion);
            }
            Err(drocsid_protocol::FrameError::UnknownType(_)) => {
                return Err(ServerError::UnknownProtocolFrameType);
            }
            Err(drocsid_protocol::FrameError::Truncated) => {
                return Err(ServerError::TruncatedProtocolFrame);
            }
            Err(drocsid_protocol::FrameError::InvalidUsername) => {
                return Err(ServerError::InvalidFrameType);
            }
        };
        let raw_username = match frame {
            Frame {
                kind: FrameType::Handshake,
                payload,
                ..
            } => match String::from_utf8(payload) {
                Ok(username) => username,
                Err(_) => {
                    return Err(ServerError::InvalidUtf8);
                }
            },
            _ => return Err(ServerError::InvalidFrameType),
        };

        if raw_username.len() > MAX_USERNAME_BYTES {
            return Err(ServerError::UsernameTooLong);
        }

        if raw_username.trim().is_empty() {
            return Err(ServerError::EmptyHandshakeUsername);
        }

        if !is_valid_username(&raw_username) {
            return Err(ServerError::UsernameContainsControlCharacters);
        }

        Ok(raw_username.trim().to_string())
    }

    fn read_messages(
        &self,
        reader: &mut FrameReader<TcpStream>,
        sender_addr: SocketAddr,
        username: &str,
    ) -> Result<(), ServerError> {
        loop {
            let frame = match reader.read_protocol_frame() {
                Ok(Some(frame)) => frame,
                Ok(None) => return Ok(()),
                Err(drocsid_protocol::FrameError::TooLarge(_)) => {
                    return Err(ServerError::MessageTooLong);
                }
                Err(drocsid_protocol::FrameError::Io(
                    ErrorKind::ConnectionAborted
                    | ErrorKind::ConnectionReset
                    | ErrorKind::BrokenPipe,
                )) => {
                    return Ok(());
                }
                Err(drocsid_protocol::FrameError::Io(kind)) => {
                    return Err(std::io::Error::from(kind).into());
                }
                Err(drocsid_protocol::FrameError::UnsupportedVersion(_)) => {
                    return Err(ServerError::UnsupportedProtocolVersion);
                }
                Err(drocsid_protocol::FrameError::UnknownType(_)) => {
                    return Err(ServerError::UnknownProtocolFrameType);
                }
                Err(drocsid_protocol::FrameError::Truncated) => {
                    return Err(ServerError::TruncatedProtocolFrame);
                }
                Err(drocsid_protocol::FrameError::InvalidUsername) => {
                    return Err(ServerError::InvalidFrameType);
                }
            };
            if frame.kind != FrameType::Chat {
                return Err(ServerError::InvalidFrameType);
            }
            let bytes = frame.payload;
            if bytes.len() > MAX_CHAT_MESSAGE_BYTES {
                return Err(ServerError::MessageTooLong);
            }
            debug!(
                frame_bytes = bytes.len(),
                phase = "message_read",
                "message frame received"
            );
            let content = String::from_utf8(bytes).map_err(|_| ServerError::InvalidUtf8)?;

            if content.trim().is_empty() {
                continue;
            }

            allow_message(&self.state, sender_addr)?;

            if !self.simulated_latency.is_zero() {
                thread::sleep(self.simulated_latency);
            }

            let message = Self::format_server_message(username, &content);
            record_message(&self.state, &message)?;
            let frame = encode_frame(FrameType::Chat, message.as_bytes())
                .map_err(|_| ServerError::MessageTooLong)?;
            match broadcast(&self.state, &frame, None) {
                Ok(true) => broadcast_presence(&self.state)?,
                Ok(false) => {}
                Err(error) => {
                    warn!(
                        error = %error,
                        error_kind = ?error_kind(&error),
                        phase = "broadcast",
                        "failed to broadcast chat message"
                    );
                    return Err(error);
                }
            }
        }
    }

    fn format_server_message(username: &str, content: &str) -> String {
        let timestamp = Local::now().format("%H:%M").to_string();
        Self::format_server_message_at(username, &timestamp, content)
    }

    fn format_server_message_at(username: &str, timestamp: &str, content: &str) -> String {
        format_chat_message(username, timestamp, content)
    }

    fn disconnect_client(
        &self,
        sender_addr: SocketAddr,
        username: &str,
        token: &ClientToken,
    ) -> Result<(), ServerError> {
        super::state::remove_client_if_current(&self.state, sender_addr, token)?;

        let leave_message = format!("{username} has left the chat\n");
        info!(username = ?username, phase = "lifecycle", "client left chat");
        record_message(&self.state, &leave_message)?;
        let leave_frame = encode_frame(FrameType::Chat, leave_message.trim_end().as_bytes())
            .map_err(|_| ServerError::MessageTooLong)?;
        broadcast(&self.state, &leave_frame, None)?;
        broadcast_presence(&self.state)?;

        Ok(())
    }

    fn send_message_history(
        &self,
        sender_addr: SocketAddr,
        token: &ClientToken,
    ) -> Result<(), ServerError> {
        let (writer, history) = history_snapshot(&self.state, sender_addr, token)?;

        for message in history {
            let frame = encode_frame(FrameType::Chat, message.as_bytes())
                .map_err(|_| ServerError::MessageTooLong)?;
            writer.try_send(frame).map_err(|error| match error {
                std::sync::mpsc::TrySendError::Full(_) => ServerError::OutboundQueueFull,
                std::sync::mpsc::TrySendError::Disconnected(_) => ServerError::OutboundWriterGone,
            })?;
        }

        Ok(())
    }
}

struct FrameReader<R> {
    reader: R,
}

impl<R: Read> FrameReader<R> {
    fn new(reader: R) -> Self {
        Self { reader }
    }

    fn read_protocol_frame(&mut self) -> Result<Option<Frame>, drocsid_protocol::FrameError> {
        read_frame(&mut self.reader)
    }
}

fn is_disconnect_error(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::BrokenPipe
            | ErrorKind::ConnectionAborted
            | ErrorKind::ConnectionReset
            | ErrorKind::TimedOut
            | ErrorKind::WouldBlock
    )
}

fn error_kind(error: &ServerError) -> Option<ErrorKind> {
    match error {
        ServerError::Io(error) => Some(error.kind()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Cursor, Read, Write},
        net::{Shutdown, TcpListener, TcpStream},
        time::Duration,
    };

    use drocsid_protocol::{FrameType, encode_frame, parse_chat_message, read_frame};

    use super::{ConnectionHandler, FrameReader};
    use crate::{
        ServerError,
        state::{new_shared_state, register_client, set_client_username, usernames},
    };

    #[test]
    fn rejects_control_characters_during_handshake() {
        for payload in [
            b"alice\x1b[2J".as_slice(),
            b"alice\r".as_slice(),
            b"alice\t".as_slice(),
        ] {
            let handler = ConnectionHandler::new(new_shared_state(), Duration::ZERO);
            let frame = encode_frame(FrameType::Handshake, payload).unwrap();
            let mut reader = FrameReader::new(Cursor::new(frame));

            assert!(matches!(
                handler.read_handshake_username(&mut reader),
                Err(ServerError::UsernameContainsControlCharacters)
            ));
        }
    }

    #[test]
    fn rejects_non_handshake_frames_during_handshake() {
        let handler = ConnectionHandler::new(new_shared_state(), Duration::ZERO);
        let frame = encode_frame(FrameType::Chat, b"alice").unwrap();
        let mut reader = FrameReader::new(Cursor::new(frame));

        assert!(matches!(
            handler.read_handshake_username(&mut reader),
            Err(ServerError::InvalidFrameType)
        ));
    }

    #[test]
    fn registers_a_client_only_after_a_successful_handshake() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client_stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut server_stream, sender_addr) = listener.accept().unwrap();
        let state = new_shared_state();
        let handler = ConnectionHandler::new(state.clone(), Duration::ZERO);

        client_stream
            .try_clone()
            .unwrap()
            .write_all(&encode_frame(FrameType::Handshake, b"alice").unwrap())
            .unwrap();

        assert_eq!(
            handler
                .authenticate(&mut server_stream, sender_addr)
                .unwrap()
                .0,
            "alice"
        );
        assert_eq!(usernames(&state).unwrap(), vec!["alice"]);
    }

    #[test]
    fn times_out_an_incomplete_handshake_without_registering_a_client() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let _client_stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut server_stream, sender_addr) = listener.accept().unwrap();
        let state = new_shared_state();
        let handler = ConnectionHandler::new(state.clone(), Duration::ZERO);

        let error = handler
            .authenticate_with_timeout(&mut server_stream, sender_addr, Duration::from_millis(20))
            .unwrap_err();

        assert!(matches!(
            error,
            ServerError::Io(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                )
        ));
        assert!(usernames(&state).unwrap().is_empty());
    }

    #[test]
    fn removes_a_client_when_authenticated_session_returns_an_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client_stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server_stream, sender_addr) = listener.accept().unwrap();
        let state = new_shared_state();
        let token = register_client(&state, &server_stream).unwrap();
        set_client_username(&state, sender_addr, &token, "alice").unwrap();
        assert_eq!(usernames(&state).unwrap(), vec!["alice"]);
        let handler = ConnectionHandler::new(state.clone(), Duration::ZERO);

        client_stream
            .write_all(
                &encode_frame(
                    FrameType::Chat,
                    &vec![b'x'; super::MAX_CHAT_MESSAGE_BYTES + 1],
                )
                .unwrap(),
            )
            .unwrap();

        let error = handler
            .serve_authenticated(server_stream, sender_addr, "alice".to_string(), token)
            .unwrap_err();

        assert!(matches!(error, ServerError::MessageTooLong));
        assert!(usernames(&state).unwrap().is_empty());
    }

    #[test]
    fn formats_messages_with_authenticated_username() {
        let message = ConnectionHandler::format_server_message_at(
            "mallory",
            "12:34",
            "[alice](10:25): forged author",
        );

        assert_eq!(
            parse_chat_message(&message),
            Some(("mallory", "12:34", "[alice](10:25): forged author"))
        );
    }

    #[test]
    fn broadcasts_messages_with_authenticated_username() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let server_addr = listener.local_addr().unwrap();
        let mut client_stream = TcpStream::connect(server_addr).unwrap();
        let (server_stream, client_addr) = listener.accept().unwrap();
        let state = new_shared_state();
        register_client(&state, &server_stream).unwrap();

        let handler = ConnectionHandler::new(state, Duration::ZERO);
        let mut reader = FrameReader::new(server_stream.try_clone().unwrap());
        client_stream
            .write_all(&encode_frame(FrameType::Chat, b"[alice](10:25): forged author").unwrap())
            .unwrap();
        client_stream.shutdown(Shutdown::Write).unwrap();

        handler
            .read_messages(&mut reader, client_addr, "mallory")
            .unwrap();

        client_stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let frame = read_frame(&mut client_stream).unwrap().unwrap();
        assert_eq!(frame.kind, FrameType::Chat);
        let message = String::from_utf8(frame.payload).unwrap();

        let (username, timestamp, content) = parse_chat_message(&message).unwrap();
        assert_eq!(username, "mallory");
        assert_eq!(content, "[alice](10:25): forged author");
        assert_eq!(timestamp.len(), 5);
        assert_eq!(timestamp.as_bytes()[2], b':');
    }

    #[test]
    fn rejects_non_chat_messages_from_authenticated_clients() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client_stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server_stream, client_addr) = listener.accept().unwrap();
        let state = new_shared_state();
        let token = register_client(&state, &server_stream).unwrap();

        let handler = ConnectionHandler::new(state.clone(), Duration::ZERO);
        let mut reader = FrameReader::new(server_stream.try_clone().unwrap());
        client_stream
            .write_all(&encode_frame(FrameType::Presence, b"admin").unwrap())
            .unwrap();

        let error = handler
            .read_messages(&mut reader, client_addr, "attacker")
            .unwrap_err();

        assert!(matches!(error, ServerError::InvalidFrameType));

        let (_, history) = super::history_snapshot(&state, client_addr, &token).unwrap();
        assert!(history.is_empty());

        client_stream
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let mut broadcast = [0; 1];
        let read_result = client_stream.read(&mut broadcast);
        assert!(matches!(
            read_result,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                )
        ));
    }
}
