use std::{
    io::{ErrorKind, Write},
    net::TcpStream,
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
};

use drocsid_config::ServerConfig;
use drocsid_protocol::{Frame, FrameType, encode_frame, parse_presence, read_frame};
use tracing::{debug, info, warn};

const NETWORK_EVENT_CAPACITY: usize = 256;

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

        let handshake = encode_frame(FrameType::Handshake, username.as_bytes())
            .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "username is too large"))?;
        if let Err(error) = stream.write_all(&handshake) {
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

        let (tx, rx) = mpsc::sync_channel(NETWORK_EVENT_CAPACITY);
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
        let frame = encode_frame(FrameType::Chat, message.as_bytes())
            .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "message is too large"))?;
        if let Err(error) = self.stream.write_all(&frame) {
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
    tx: SyncSender<NetworkEvent>,
    server_addr: String,
    username: String,
) {
    thread::spawn(move || {
        loop {
            let frame = match read_frame(&mut reader_stream) {
                Ok(Some(frame)) => frame,
                Ok(None) => {
                    let _ = tx.send(NetworkEvent::Disconnected(
                        "server disconnected".to_string(),
                    ));
                    break;
                }
                Err(error) => {
                    warn!(
                        server_addr = %server_addr,
                        username = %username,
                        error = ?error,
                        phase = "message_read",
                        "client frame read failed"
                    );
                    let _ = tx.send(NetworkEvent::Disconnected(format!(
                        "connection error: {error:?}"
                    )));
                    break;
                }
            };
            if !process_frame(frame, &tx) {
                break;
            }
        }
    });
}

fn process_frame(frame: Frame, tx: &SyncSender<NetworkEvent>) -> bool {
    match frame.kind {
        FrameType::Chat => {
            let message = match String::from_utf8(frame.payload) {
                Ok(message) => message,
                Err(_) => {
                    let _ = tx.send(NetworkEvent::Disconnected(
                        "server sent invalid UTF-8 chat payload".to_string(),
                    ));
                    return false;
                }
            };
            if !message.trim().is_empty() {
                let _ = tx.send(NetworkEvent::Message(message));
            }
        }
        FrameType::Presence => {
            let users = match parse_presence(&frame.payload) {
                Some(users) => users,
                None => {
                    let _ = tx.send(NetworkEvent::Disconnected(
                        "server sent an invalid presence payload".to_string(),
                    ));
                    return false;
                }
            };
            let _ = tx.send(NetworkEvent::UserList(users));
        }
        FrameType::Handshake => {
            let _ = tx.send(NetworkEvent::Disconnected(
                "server sent an unexpected handshake frame".to_string(),
            ));
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use drocsid_protocol::Frame;

    use super::{NETWORK_EVENT_CAPACITY, NetworkEvent, process_frame};

    #[test]
    fn event_channel_is_bounded() {
        let (tx, rx) = mpsc::sync_channel(NETWORK_EVENT_CAPACITY);

        for _ in 0..NETWORK_EVENT_CAPACITY {
            tx.try_send(NetworkEvent::Message("message".to_string()))
                .unwrap();
        }

        assert!(matches!(
            tx.try_send(NetworkEvent::Message("overflow".to_string())),
            Err(mpsc::TrySendError::Full(_))
        ));
        drop(rx);
    }

    #[test]
    fn disconnects_on_invalid_chat_utf8() {
        let (tx, rx) = mpsc::sync_channel(NETWORK_EVENT_CAPACITY);
        assert!(!process_frame(
            Frame {
                version: 1,
                kind: super::FrameType::Chat,
                payload: vec![0xff],
            },
            &tx,
        ));
        assert!(matches!(
            rx.recv().unwrap(),
            NetworkEvent::Disconnected(reason) if reason == "server sent invalid UTF-8 chat payload"
        ));
    }

    #[test]
    fn disconnects_on_malformed_presence_payload() {
        let (tx, rx) = mpsc::sync_channel(NETWORK_EVENT_CAPACITY);
        assert!(!process_frame(
            Frame {
                version: 1,
                kind: super::FrameType::Presence,
                payload: vec![0, 1, 0],
            },
            &tx,
        ));
        assert!(matches!(
            rx.recv().unwrap(),
            NetworkEvent::Disconnected(reason) if reason == "server sent an invalid presence payload"
        ));
    }
}
