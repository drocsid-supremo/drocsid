use std::{
    io::{ErrorKind, Read, Write},
    net::TcpStream,
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use drocsid_config::ServerConfig;
use drocsid_protocol::parse_users_event;

const MAX_FRAME_BYTES: usize = 4 * 1024;

pub enum NetworkEvent {
    Message(String),
    UserList(Vec<String>),
    Disconnected(String),
}

pub struct ClientConnection {
    stream: TcpStream,
}

impl ClientConnection {
    pub fn connect(
        username: &str,
        server_config: &ServerConfig,
    ) -> std::io::Result<(Self, Receiver<NetworkEvent>)> {
        let mut stream = TcpStream::connect(&server_config.connect_addr)?;
        let reader_stream = stream.try_clone()?;

        stream.write_all(format!("{username}\n").as_bytes())?;

        let (tx, rx) = mpsc::channel();
        spawn_reader(reader_stream, tx);

        Ok((Self { stream }, rx))
    }

    pub fn send_message(&mut self, message: &str) -> std::io::Result<()> {
        self.stream.write_all(format!("{message}\n").as_bytes())
    }

    pub fn is_disconnect_error(error: &std::io::Error) -> bool {
        matches!(
            error.kind(),
            ErrorKind::BrokenPipe | ErrorKind::ConnectionAborted | ErrorKind::ConnectionReset
        )
    }
}

fn spawn_reader(mut reader_stream: TcpStream, tx: Sender<NetworkEvent>) {
    thread::spawn(move || {
        let mut buffer = [0; 1024];
        let mut pending = String::new();

        loop {
            let bytes = match reader_stream.read(&mut buffer) {
                Ok(bytes) => bytes,
                Err(error) => {
                    let _ = tx.send(NetworkEvent::Disconnected(format!(
                        "connection error: {error}"
                    )));
                    break;
                }
            };

            if bytes == 0 {
                if !pending.trim().is_empty() {
                    let _ = tx.send(NetworkEvent::Message(
                        pending.trim_end_matches('\n').to_string(),
                    ));
                }

                let _ = tx.send(NetworkEvent::Disconnected(
                    "server disconnected".to_string(),
                ));
                break;
            }

            pending.push_str(&String::from_utf8_lossy(&buffer[..bytes]));

            if !process_pending_frames(&mut pending, &tx) {
                break;
            }
        }
    });
}

fn process_pending_frames(pending: &mut String, tx: &Sender<NetworkEvent>) -> bool {
    while let Some(newline_index) = pending.find('\n') {
        if newline_index > MAX_FRAME_BYTES {
            let _ = tx.send(NetworkEvent::Disconnected(
                "server sent an oversized frame".to_string(),
            ));
            return false;
        }

        let line = pending[..newline_index].to_string();
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

    if pending.len() > MAX_FRAME_BYTES {
        let _ = tx.send(NetworkEvent::Disconnected(
            "server sent an oversized frame".to_string(),
        ));
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::{MAX_FRAME_BYTES, NetworkEvent, process_pending_frames};

    #[test]
    fn rejects_an_oversized_frame_without_a_newline() {
        let (tx, rx) = mpsc::channel();
        let mut pending = "x".repeat(MAX_FRAME_BYTES + 1);

        assert!(!process_pending_frames(&mut pending, &tx));
        assert!(matches!(
            rx.recv().unwrap(),
            NetworkEvent::Disconnected(reason) if reason == "server sent an oversized frame"
        ));
    }

    #[test]
    fn rejects_an_oversized_delimited_frame() {
        let (tx, rx) = mpsc::channel();
        let mut pending = format!("{}\n", "x".repeat(MAX_FRAME_BYTES + 1));

        assert!(!process_pending_frames(&mut pending, &tx));
        assert!(matches!(
            rx.recv().unwrap(),
            NetworkEvent::Disconnected(reason) if reason == "server sent an oversized frame"
        ));
    }
}
