use std::{
    io::{ErrorKind, Read, Write},
    net::TcpStream,
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use drocsid_config::ServerConfig;
use drocsid_protocol::{USERS_EVENT_PREFIX, parse_users_event};
use tracing::{debug, info, warn};

const MAX_CHAT_FRAME_BYTES: usize = 4 * 1024;
const MAX_PRESENCE_FRAME_BYTES: usize = 16 * 1024;

pub enum NetworkEvent {
    Message(String),
    UserList(Vec<String>),
    Disconnected(String),
}

pub struct ClientConnection {
    stream: TcpStream,
    server_addr: String,
    username: String,
}

impl ClientConnection {
    pub fn connect(
        username: &str,
        server_config: &ServerConfig,
    ) -> std::io::Result<(Self, Receiver<NetworkEvent>)> {
        info!(server_addr = %server_config.connect_addr, username = %username, "connecting to server");
        let mut stream = match TcpStream::connect(&server_config.connect_addr) {
            Ok(stream) => stream,
            Err(error) => {
                warn!(
                    server_addr = %server_config.connect_addr,
                    username = %username,
                    error = %error,
                    error_kind = ?error.kind(),
                    phase = "connect",
                    "failed to connect to server"
                );
                return Err(error);
            }
        };
        let reader_stream = stream.try_clone()?;

        if let Err(error) = stream.write_all(format!("{username}\n").as_bytes()) {
            warn!(
                server_addr = %server_config.connect_addr,
                error = %error,
                error_kind = ?error.kind(),
                phase = "handshake",
                "failed to send client handshake"
            );
            return Err(error);
        }
        debug!(
            server_addr = %server_config.connect_addr,
            username = %username,
            phase = "handshake",
            "client handshake sent"
        );

        let (tx, rx) = mpsc::channel();
        spawn_reader(
            reader_stream,
            tx,
            server_config.connect_addr.clone(),
            username.to_string(),
        );

        Ok((
            Self {
                stream,
                server_addr: server_config.connect_addr.clone(),
                username: username.to_string(),
            },
            rx,
        ))
    }

    pub fn send_message(&mut self, message: &str) -> std::io::Result<()> {
        if let Err(error) = self.stream.write_all(format!("{message}\n").as_bytes()) {
            warn!(
                server_addr = %self.server_addr,
                username = %self.username,
                error = %error,
                error_kind = ?error.kind(),
                phase = "message_send",
                "failed to send chat message"
            );
            return Err(error);
        }

        Ok(())
    }

    pub fn is_disconnect_error(error: &std::io::Error) -> bool {
        matches!(
            error.kind(),
            ErrorKind::BrokenPipe | ErrorKind::ConnectionAborted | ErrorKind::ConnectionReset
        )
    }
}

fn spawn_reader(
    mut reader_stream: TcpStream,
    tx: Sender<NetworkEvent>,
    server_addr: String,
    username: String,
) {
    thread::spawn(move || {
        let mut buffer = [0; 1024];
        let mut pending = Vec::new();

        loop {
            let bytes = match reader_stream.read(&mut buffer) {
                Ok(bytes) => bytes,
                Err(error) => {
                    warn!(
                        server_addr = %server_addr,
                        username = %username,
                        error = %error,
                        error_kind = ?error.kind(),
                        phase = "message_read",
                        "client read failed"
                    );
                    let _ = tx.send(NetworkEvent::Disconnected(format!(
                        "connection error: {error}"
                    )));
                    break;
                }
            };

            if bytes == 0 {
                debug!(
                    server_addr = %server_addr,
                    username = %username,
                    phase = "disconnect",
                    "server closed client connection"
                );
                if !pending.is_empty() {
                    let _ = tx.send(NetworkEvent::Message(
                        String::from_utf8_lossy(&pending)
                            .trim_end_matches('\n')
                            .to_string(),
                    ));
                }

                let _ = tx.send(NetworkEvent::Disconnected(
                    "server disconnected".to_string(),
                ));
                break;
            }

            pending.extend_from_slice(&buffer[..bytes]);

            if !process_pending_frames(&mut pending, &tx) {
                break;
            }
        }
    });
}

fn process_pending_frames(pending: &mut Vec<u8>, tx: &Sender<NetworkEvent>) -> bool {
    while let Some(newline_index) = pending.iter().position(|byte| *byte == b'\n') {
        if newline_index > max_frame_bytes(&pending[..newline_index]) {
            let _ = tx.send(NetworkEvent::Disconnected(
                "server sent an oversized frame".to_string(),
            ));
            return false;
        }

        let line = String::from_utf8_lossy(&pending[..newline_index]).to_string();
        pending.drain(..=newline_index);

        if line.trim().is_empty() {
            continue;
        }

        if let Some(users) = parse_users_event(&line) {
            let _ = tx.send(NetworkEvent::UserList(users));
        } else {
            let _ = tx.send(NetworkEvent::Message(line));
        }
    }

    if pending.len() > max_frame_bytes(pending) {
        let _ = tx.send(NetworkEvent::Disconnected(
            "server sent an oversized frame".to_string(),
        ));
        return false;
    }

    true
}

fn max_frame_bytes(frame: &[u8]) -> usize {
    if frame.starts_with(USERS_EVENT_PREFIX.as_bytes()) {
        MAX_PRESENCE_FRAME_BYTES
    } else {
        MAX_CHAT_FRAME_BYTES
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::{MAX_CHAT_FRAME_BYTES, NetworkEvent, USERS_EVENT_PREFIX, process_pending_frames};

    #[test]
    fn rejects_an_oversized_frame_without_a_newline() {
        let (tx, rx) = mpsc::channel();
        let mut pending = vec![b'x'; MAX_CHAT_FRAME_BYTES + 1];

        assert!(!process_pending_frames(&mut pending, &tx));
        assert!(matches!(
            rx.recv().unwrap(),
            NetworkEvent::Disconnected(reason) if reason == "server sent an oversized frame"
        ));
    }

    #[test]
    fn rejects_an_oversized_delimited_frame() {
        let (tx, rx) = mpsc::channel();
        let mut pending = format!("{}\n", "x".repeat(MAX_CHAT_FRAME_BYTES + 1)).into_bytes();

        assert!(!process_pending_frames(&mut pending, &tx));
        assert!(matches!(
            rx.recv().unwrap(),
            NetworkEvent::Disconnected(reason) if reason == "server sent an oversized frame"
        ));
    }

    #[test]
    fn accepts_a_presence_frame_larger_than_the_chat_limit() {
        let (tx, rx) = mpsc::channel();
        let mut pending =
            format!("{USERS_EVENT_PREFIX}{}\n", "x".repeat(MAX_CHAT_FRAME_BYTES)).into_bytes();

        assert!(process_pending_frames(&mut pending, &tx));
        assert!(matches!(rx.recv().unwrap(), NetworkEvent::UserList(_)));
    }

    #[test]
    fn counts_frame_bytes_before_decoding_split_utf8() {
        let (tx, rx) = mpsc::channel();
        let mut frame = vec![b'x'; MAX_CHAT_FRAME_BYTES - 3];
        frame.extend_from_slice("€".as_bytes());
        frame.push(b'\n');

        let mut pending = frame[..MAX_CHAT_FRAME_BYTES - 2].to_vec();
        assert!(process_pending_frames(&mut pending, &tx));

        pending.extend_from_slice(&frame[MAX_CHAT_FRAME_BYTES - 2..]);
        assert!(process_pending_frames(&mut pending, &tx));
        assert!(matches!(
            rx.recv().unwrap(),
            NetworkEvent::Message(message) if message.len() == MAX_CHAT_FRAME_BYTES
        ));
    }
}
