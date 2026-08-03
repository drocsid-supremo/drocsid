use std::{
    io::{ErrorKind, Read, Write},
    net::{SocketAddr, TcpStream},
    thread,
    time::Duration,
};

use crate::ServerError;

use super::state::{
    ServerStateHandle, allow_message, broadcast, broadcast_presence, message_history,
    record_message, remove_client, set_client_username,
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

    pub fn serve(&self, mut stream: TcpStream) -> Result<(), ServerError> {
        let sender_addr = stream.peer_addr()?;
        stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
        stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
        stream.set_nodelay(true)?;

        let username = {
            let mut handshake_reader = FrameReader::new(stream.try_clone()?);
            self.read_handshake_username(&mut handshake_reader, sender_addr)?
        };
        stream.set_read_timeout(None)?;
        let mut reader = FrameReader::new(stream.try_clone()?);

        set_client_username(&self.state, sender_addr, &username)?;
        if let Err(error) = self.send_message_history(&mut stream) {
            remove_client(&self.state, sender_addr)?;
            return match error {
                ServerError::Io(error) if is_disconnect_error(&error) => Ok(()),
                error => Err(error),
            };
        }
        broadcast_presence(&self.state)?;

        let join_message = format!("@{} has entered the chat. Say hello!\n", username);
        println!("{}", join_message.trim());
        record_message(&self.state, &join_message)?;
        broadcast(&self.state, &join_message, None)?;

        let result = self.read_messages(&mut reader, sender_addr);
        self.disconnect_client(sender_addr, &username)?;
        result
    }

    fn read_handshake_username(
        &self,
        reader: &mut FrameReader<TcpStream>,
        sender_addr: SocketAddr,
    ) -> Result<String, ServerError> {
        let username = match reader.read_frame(MAX_USERNAME_BYTES) {
            Ok(Some(bytes)) => match String::from_utf8(bytes) {
                Ok(username) => username,
                Err(_) => {
                    remove_client(&self.state, sender_addr)?;
                    return Err(ServerError::InvalidUtf8);
                }
            },
            Ok(None) => {
                remove_client(&self.state, sender_addr)?;
                return Err(ServerError::EmptyHandshakeUsername);
            }
            Err(FrameError::TooLong) => {
                remove_client(&self.state, sender_addr)?;
                return Err(ServerError::UsernameTooLong);
            }
            Err(FrameError::Io(error)) => {
                let _ = remove_client(&self.state, sender_addr);
                return Err(error.into());
            }
        }
        .trim()
        .to_string();

        if username.is_empty() {
            remove_client(&self.state, sender_addr)?;
            return Err(ServerError::EmptyHandshakeUsername);
        }

        Ok(username)
    }

    fn read_messages(
        &self,
        reader: &mut FrameReader<TcpStream>,
        sender_addr: SocketAddr,
    ) -> Result<(), ServerError> {
        loop {
            let bytes = match reader.read_frame(MAX_MESSAGE_BYTES) {
                Ok(Some(bytes)) => bytes,
                Ok(None) => return Ok(()),
                Err(FrameError::TooLong) => return Err(ServerError::MessageTooLong),
                Err(FrameError::Io(error)) if is_disconnect_error(&error) => return Ok(()),
                Err(FrameError::Io(error)) => return Err(error.into()),
            };
            let message = String::from_utf8(bytes).map_err(|_| ServerError::InvalidUtf8)?;

            if message.trim().is_empty() {
                continue;
            }

            allow_message(&self.state, sender_addr)?;

            if !self.simulated_latency.is_zero() {
                thread::sleep(self.simulated_latency);
            }

            record_message(&self.state, &message)?;
            broadcast(&self.state, &format!("{message}\n"), None)?;
        }
    }

    fn disconnect_client(
        &self,
        sender_addr: SocketAddr,
        username: &str,
    ) -> Result<(), ServerError> {
        remove_client(&self.state, sender_addr)?;

        let leave_message = format!("{username} has left the chat\n");
        println!("{}", leave_message.trim());
        record_message(&self.state, &leave_message)?;
        broadcast(&self.state, &leave_message, None)?;
        broadcast_presence(&self.state)?;

        Ok(())
    }

    fn send_message_history(&self, stream: &mut TcpStream) -> Result<(), ServerError> {
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

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{FrameError, FrameReader};

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
}
