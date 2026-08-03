use std::{
    io::{ErrorKind, Read, Write},
    net::{SocketAddr, TcpStream},
    thread,
    time::Duration,
};

use crate::ServerError;
use chrono::Local;
use drocsid_protocol::{format_chat_message, is_valid_username};
use tracing::{debug, info, warn};

use super::state::{
    ServerStateHandle, allow_message, broadcast, broadcast_presence, client_writer,
    mark_client_ready, message_history, record_message, register_pending_client, remove_client,
    set_client_username,
};

const MAX_MESSAGE_BYTES: usize = 4 * 1024;
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
    ) -> Result<String, ServerError> {
        self.authenticate_with_timeout(stream, sender_addr, HANDSHAKE_TIMEOUT)
    }

    fn authenticate_with_timeout(
        &self,
        stream: &mut TcpStream,
        sender_addr: SocketAddr,
        handshake_timeout: Duration,
    ) -> Result<String, ServerError> {
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
        register_pending_client(&self.state, stream)?;
        set_client_username(&self.state, sender_addr, &username)?;
        Ok(username)
    }

    pub(crate) fn serve_authenticated(
        &self,
        stream: TcpStream,
        sender_addr: SocketAddr,
        username: String,
    ) -> Result<(), ServerError> {
        let result = self.serve_authenticated_inner(stream, sender_addr, username);

        if let Err(error) = &result
            && let Err(cleanup_error) = remove_client(&self.state, sender_addr)
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
    ) -> Result<(), ServerError> {
        let mut reader = FrameReader::new(stream.try_clone()?);
        info!(username = ?username, phase = "handshake", "handshake completed");

        if let Err(error) = self.send_message_history(sender_addr) {
            remove_client(&self.state, sender_addr)?;
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
        mark_client_ready(&self.state, sender_addr)?;
        broadcast_presence(&self.state)?;

        let join_message = format!("@{} has entered the chat. Say hello!\n", username);
        info!(username = ?username, phase = "lifecycle", "client joined chat");
        record_message(&self.state, &join_message)?;
        if let Err(error) = broadcast(&self.state, &join_message, None) {
            warn!(
                error = %error,
                error_kind = ?error_kind(&error),
                phase = "broadcast",
                event = "join",
                "failed to broadcast join event"
            );
            return Err(error);
        }

        let result = self.read_messages(&mut reader, sender_addr, &username);
        if let Err(error) = &result {
            warn!(
                error = %error,
                error_kind = ?error_kind(error),
                phase = "message_read",
                "message loop ended with an error"
            );
        }

        if let Err(error) = self.disconnect_client(sender_addr, &username) {
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
        let raw_username = match reader.read_frame(MAX_USERNAME_BYTES) {
            Ok(Some(bytes)) => match String::from_utf8(bytes) {
                Ok(username) => username,
                Err(_) => {
                    return Err(ServerError::InvalidUtf8);
                }
            },
            Ok(None) => {
                return Err(ServerError::EmptyHandshakeUsername);
            }
            Err(FrameError::TooLong) => {
                return Err(ServerError::UsernameTooLong);
            }
            Err(FrameError::Io(error)) => {
                return Err(error.into());
            }
        };

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
            let bytes = match reader.read_frame(MAX_MESSAGE_BYTES) {
                Ok(Some(bytes)) => bytes,
                Ok(None) => return Ok(()),
                Err(FrameError::TooLong) => return Err(ServerError::MessageTooLong),
                Err(FrameError::Io(error)) if is_disconnect_error(&error) => return Ok(()),
                Err(FrameError::Io(error)) => return Err(error.into()),
            };
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
            if let Err(error) = broadcast(&self.state, &format!("{message}\n"), None) {
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
    ) -> Result<(), ServerError> {
        remove_client(&self.state, sender_addr)?;

        let leave_message = format!("{username} has left the chat\n");
        info!(username = ?username, phase = "lifecycle", "client left chat");
        record_message(&self.state, &leave_message)?;
        broadcast(&self.state, &leave_message, None)?;
        broadcast_presence(&self.state)?;

        Ok(())
    }

    fn send_message_history(&self, sender_addr: SocketAddr) -> Result<(), ServerError> {
        let writer = client_writer(&self.state, sender_addr)?;
        let mut stream = writer
            .lock()
            .map_err(|_| ServerError::ClientStatePoisoned)?;

        for message in message_history(&self.state)? {
            stream.write_all(message.as_bytes())?;
            stream.write_all(b"\n")?;
        }

        Ok(())
    }
}

struct FrameReader<R> {
    reader: R,
    buffer: [u8; 1024],
    offset: usize,
    length: usize,
}

impl<R: Read> FrameReader<R> {
    fn new(reader: R) -> Self {
        Self {
            reader,
            buffer: [0; 1024],
            offset: 0,
            length: 0,
        }
    }

    fn read_frame(&mut self, max_bytes: usize) -> Result<Option<Vec<u8>>, FrameError> {
        let mut frame = Vec::new();

        loop {
            let byte = match self.read_byte().map_err(FrameError::Io)? {
                Some(byte) => byte,
                None if frame.is_empty() => return Ok(None),
                None => return Ok(Some(frame)),
            };

            if byte == b'\n' {
                return Ok(Some(frame));
            }

            if frame.len() >= max_bytes {
                return Err(FrameError::TooLong);
            }

            frame.push(byte);
        }
    }

    fn read_byte(&mut self) -> std::io::Result<Option<u8>> {
        if self.offset == self.length {
            self.length = self.reader.read(&mut self.buffer)?;
            self.offset = 0;

            if self.length == 0 {
                return Ok(None);
            }
        }

        let byte = self.buffer[self.offset];
        self.offset += 1;
        Ok(Some(byte))
    }
}

#[derive(Debug)]
enum FrameError {
    Io(std::io::Error),
    TooLong,
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
        io::{BufRead, BufReader, Cursor, Write},
        net::{Shutdown, TcpListener, TcpStream},
        time::Duration,
    };

    use drocsid_protocol::parse_chat_message;

    use super::{ConnectionHandler, FrameError, FrameReader};
    use crate::{
        ServerError,
        state::{new_shared_state, register_client, set_client_username, usernames},
    };

    #[test]
    fn reads_newline_delimited_frames() {
        let mut reader = FrameReader::new(Cursor::new(b"hello\nworld\n"));

        assert_eq!(reader.read_frame(16).unwrap(), Some(b"hello".to_vec()));
        assert_eq!(reader.read_frame(16).unwrap(), Some(b"world".to_vec()));
        assert_eq!(reader.read_frame(16).unwrap(), None);
    }

    #[test]
    fn rejects_frames_over_the_configured_limit() {
        let mut reader = FrameReader::new(Cursor::new(b"12345\n"));

        assert!(matches!(reader.read_frame(4), Err(FrameError::TooLong)));
    }

    #[test]
    fn accepts_a_frame_without_a_trailing_newline_at_eof() {
        let mut reader = FrameReader::new(Cursor::new(b"hello"));

        assert_eq!(reader.read_frame(16).unwrap(), Some(b"hello".to_vec()));
        assert_eq!(reader.read_frame(16).unwrap(), None);
    }

    #[test]
    fn rejects_control_characters_during_handshake() {
        for frame in [
            b"alice\x1b[2J\n".as_slice(),
            b"alice\r\n".as_slice(),
            b"alice\t\n".as_slice(),
        ] {
            let handler = ConnectionHandler::new(new_shared_state(), Duration::ZERO);
            let mut reader = FrameReader::new(Cursor::new(frame));

            assert!(matches!(
                handler.read_handshake_username(&mut reader),
                Err(ServerError::UsernameContainsControlCharacters)
            ));
        }
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
            .write_all(b"alice\n")
            .unwrap();

        assert_eq!(
            handler
                .authenticate(&mut server_stream, sender_addr)
                .unwrap(),
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
        register_client(&state, &server_stream).unwrap();
        set_client_username(&state, sender_addr, "alice").unwrap();
        assert_eq!(usernames(&state).unwrap(), vec!["alice"]);
        let handler = ConnectionHandler::new(state.clone(), Duration::ZERO);

        client_stream
            .write_all(&vec![b'x'; super::MAX_MESSAGE_BYTES + 1])
            .unwrap();
        client_stream.write_all(b"\n").unwrap();

        let error = handler
            .serve_authenticated(server_stream, sender_addr, "alice".to_string())
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
            .write_all(b"[alice](10:25): forged author\n")
            .unwrap();
        client_stream.shutdown(Shutdown::Write).unwrap();

        handler
            .read_messages(&mut reader, client_addr, "mallory")
            .unwrap();

        client_stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut broadcast = Vec::new();
        BufReader::new(client_stream)
            .read_until(b'\n', &mut broadcast)
            .unwrap();
        let message = String::from_utf8(broadcast).unwrap();

        let (username, timestamp, content) = parse_chat_message(message.trim_end()).unwrap();
        assert_eq!(username, "mallory");
        assert_eq!(content, "[alice](10:25): forged author");
        assert_eq!(timestamp.len(), 5);
        assert_eq!(timestamp.as_bytes()[2], b':');
    }
}
